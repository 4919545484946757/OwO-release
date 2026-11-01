#pragma once
#include "owo/compatibility/input_context.h"
#include <ole2.h>
#include <UIAutomation.h>
#include <wrl/client.h>
#include <condition_variable>
#include <mutex>
#include <thread>

namespace owo::compatibility {
struct UiaCaretProof {Resolution access{};HRESULT result{E_ACCESSDENIED};};
[[nodiscard]] inline UiaCaretProof inspect_uia_text_caret(IUIAutomationElement* element,DWORD expected_pid,bool cached=false){
    UiaCaretProof proof;
    if(!element)return proof;
    int pid{};BOOL password=TRUE,enabled=FALSE;
    if(FAILED(cached?element->get_CachedProcessId(&pid):element->get_CurrentProcessId(&pid))||pid!=static_cast<int>(expected_pid)||
       FAILED(cached?element->get_CachedIsPassword(&password):element->get_CurrentIsPassword(&password))||
       FAILED(cached?element->get_CachedIsEnabled(&enabled):element->get_CurrentIsEnabled(&enabled)))return proof;
    if(password){proof.access={Decision::denied,Reason::password};return proof;}
    if(!enabled){proof.access={Decision::denied,Reason::disabled};return proof;}
    Microsoft::WRL::ComPtr<IUIAutomationTextPattern2> pattern;
    Microsoft::WRL::ComPtr<IUIAutomationTextRange> caret;
    proof.result=element->GetCurrentPatternAs(UIA_TextPattern2Id,IID_PPV_ARGS(&pattern));
    if(FAILED(proof.result)||!pattern){if(SUCCEEDED(proof.result))proof.result=E_NOINTERFACE;return proof;}
    BOOL active=FALSE;
    proof.result=pattern->GetCaretRange(&active,&caret);
    if(FAILED(proof.result)||!caret||!active){if(SUCCEEDED(proof.result))proof.result=E_ACCESSDENIED;return proof;}
    VARIANT value{};
    proof.result=caret->GetAttributeValue(UIA_IsReadOnlyAttributeId,&value);
    // GetCaretRange::isActive explicitly denotes keyboard focus of the text
    // control. A host's focused automation wrapper can be a surrounding Pane.
    proof.access=evaluate_edit({true,active!=FALSE,true,proof.result==S_OK&&value.vt==VT_BOOL,
        enabled!=FALSE,password!=FALSE,value.vt==VT_BOOL&&value.boolVal!=VARIANT_FALSE});
    VariantClear(&value);
    return proof;
}
// One bounded metadata worker per active TextService. No TSF interface or text
// crosses apartments. The GUI thread never waits for its own UIA provider.
class UiaTextVerifier final {
public:
    struct Proof {
        HWND focus{}; DWORD pid{}, thread{};
        ULONGLONG captured{}, revision{};
        Resolution access{}; HRESULT result{E_PENDING};
    };
    UiaTextVerifier():worker_([this]{run();}){}
    ~UiaTextVerifier(){ {std::lock_guard lock(mutex_);stopping_=true;}ready_.notify_one();worker_.join(); }
    UiaTextVerifier(const UiaTextVerifier&)=delete;
    UiaTextVerifier& operator=(const UiaTextVerifier&)=delete;
    [[nodiscard]] static bool current(const Proof& proof,HWND focus,DWORD pid,DWORD thread,ULONGLONG revision,ULONGLONG now){
        return proof.focus==focus&&proof.pid==pid&&proof.thread==thread&&revision&&proof.revision==revision&&
            proof.captured&&now>=proof.captured&&now-proof.captured<=150;
    }
    void request(HWND focus,DWORD pid,DWORD thread){
        std::lock_guard lock(mutex_);
        if(wanted_.focus!=focus || wanted_.pid!=pid || wanted_.thread!=thread){
            wanted_={focus,pid,thread,0,++revision_};proof_={};ready_.notify_one();
        }
    }
    void invalidate(){std::lock_guard lock(mutex_);wanted_={};proof_={};++revision_;ready_.notify_one();}
    [[nodiscard]] Proof snapshot(HWND focus,DWORD pid,DWORD thread){
        request(focus,pid,thread);
        std::lock_guard lock(mutex_);
        const auto now=GetTickCount64();
        if(!current(proof_,focus,pid,thread,revision_,now))return {};
        return proof_;
    }
private:
    static Proof query(IUIAutomation2* automation,IUIAutomationCacheRequest* cache,Proof target){
        target.access={};target.result=E_ACCESSDENIED;
        DWORD owner{};GUITHREADINFO info{sizeof(info)};
        if(!target.focus || GetWindowThreadProcessId(target.focus,&owner)!=target.thread || owner!=target.pid ||
           !GetGUIThreadInfo(target.thread,&info) || info.hwndFocus!=target.focus)return target;
        Microsoft::WRL::ComPtr<IUIAutomationElement> element;
        target.result=automation->GetFocusedElementBuildCache(cache,&element);
        if(FAILED(target.result)||!element)return target;
        auto verified=inspect_uia_text_caret(element.Get(),target.pid,true);
        if(verified.access.decision==Decision::unknown || verified.result==E_NOINTERFACE){
            Microsoft::WRL::ComPtr<IUIAutomationElement> native;
            if(SUCCEEDED(automation->ElementFromHandleBuildCache(target.focus,cache,&native))&&native){
                const auto alternative=inspect_uia_text_caret(native.Get(),target.pid,true);
                if(alternative.access.decision==Decision::allowed){element=std::move(native);verified=alternative;}
            }
        }
        target.result=verified.result;target.access=verified.access;
        if(!GetGUIThreadInfo(target.thread,&info)||info.hwndFocus!=target.focus)target.access={};
        // Age includes query time; a slow provider never creates a fresh proof.
        return target;
    }
    void run(){
        const auto initialized=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
        if(FAILED(initialized))return;
        {
            Microsoft::WRL::ComPtr<IUIAutomation2> automation;
            const auto created=CoCreateInstance(__uuidof(CUIAutomation8),nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&automation));
            if(SUCCEEDED(created)&&SUCCEEDED(automation->put_ConnectionTimeout(100))&&SUCCEEDED(automation->put_TransactionTimeout(100))){
                Microsoft::WRL::ComPtr<IUIAutomationCacheRequest> cache;
                auto cache_result=automation->CreateCacheRequest(&cache);
                for(auto property:{UIA_ProcessIdPropertyId,UIA_IsPasswordPropertyId,UIA_IsEnabledPropertyId})
                    if(SUCCEEDED(cache_result))cache_result=cache->AddProperty(property);
                if(FAILED(cache_result)){cache.Reset();automation.Reset();CoUninitialize();return;}
                std::unique_lock lock(mutex_);
                while(!stopping_){
                    if(!wanted_.focus){ready_.wait(lock,[&]{return stopping_||wanted_.focus;});continue;}
                    auto target=wanted_;target.captured=GetTickCount64();
                    lock.unlock();auto value=query(automation.Get(),cache.Get(),target);lock.lock();
                    if(value.revision==revision_ && !stopping_)proof_=value;
                    ready_.wait_for(lock,std::chrono::milliseconds(25),[&]{return stopping_||wanted_.revision!=target.revision;});
                }
            }
        }
        CoUninitialize();
    }
    std::mutex mutex_;std::condition_variable ready_;bool stopping_{};
    Proof wanted_{},proof_{};ULONGLONG revision_{};std::thread worker_;
};
// Trivial TLS pointer: ownership belongs to TextService and is cleared before
// worker shutdown / COM server release. No DLL-unload thread-local destructor.
inline thread_local UiaTextVerifier* current_text_verifier{};
}
