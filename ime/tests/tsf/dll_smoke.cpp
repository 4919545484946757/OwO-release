#include <Windows.h>
#include <msctf.h>

#include <iostream>

namespace {
constexpr CLSID kTextServiceClsid{
    0x6d31c9b1, 0x8978, 0x4f49, {0x89, 0xb4, 0x66, 0xeb, 0x1e, 0x74, 0x15, 0x91}};
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    const HMODULE module = LoadLibraryW(argv[1]);
    if (module == nullptr) {
        std::cerr << "LoadLibraryW failed: " << GetLastError() << '\n';
        return 3;
    }
    using GetClassObject = HRESULT(__stdcall*)(REFCLSID, REFIID, void**);
    const auto get_class_object = reinterpret_cast<GetClassObject>(
        GetProcAddress(module, "DllGetClassObject"));
    using CanUnloadNow = HRESULT(__stdcall*)();
    const auto can_unload_now = reinterpret_cast<CanUnloadNow>(
        GetProcAddress(module, "DllCanUnloadNow"));
    if (get_class_object == nullptr || can_unload_now == nullptr) {
        FreeLibrary(module);
        return 4;
    }
    IClassFactory* factory = nullptr;
    HRESULT result = get_class_object(kTextServiceClsid, IID_PPV_ARGS(&factory));
    if (FAILED(result)) {
        FreeLibrary(module);
        return 5;
    }
    ITfTextInputProcessorEx* service = nullptr;
    result = factory->CreateInstance(nullptr, IID_PPV_ARGS(&service));
    factory->Release();
    if (FAILED(result)) {
        FreeLibrary(module);
        return 6;
    }
    ITfKeyEventSink* key_sink = nullptr;
    result = service->QueryInterface(IID_PPV_ARGS(&key_sink));
    if (SUCCEEDED(result)) key_sink->Release();
    ITfThreadMgrEventSink* manager_sink = nullptr;
    const HRESULT manager_result = service->QueryInterface(IID_PPV_ARGS(&manager_sink));
    if (SUCCEEDED(manager_result)) manager_sink->Release();
    ITfThreadFocusSink* focus_sink = nullptr;
    const HRESULT focus_result = service->QueryInterface(IID_PPV_ARGS(&focus_sink));
    if (SUCCEEDED(focus_result)) focus_sink->Release();
    service->Release();
    const HRESULT unload_result = can_unload_now();
    FreeLibrary(module);
    return SUCCEEDED(result) && SUCCEEDED(manager_result) && SUCCEEDED(focus_result) &&
                   unload_result == S_OK
               ? 0
               : 7;
}
