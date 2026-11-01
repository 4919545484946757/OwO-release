#include <Windows.h>
namespace {
LRESULT CALLBACK procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if(message==WM_CREATE){
        const wchar_t* labels[]={L"Edit",L"RichEdit",L"Password (must reject)",L"Readonly (must reject)",L"Disabled (must reject)"};
        for(int i=0;i<5;++i){
            CreateWindowW(L"STATIC",labels[i],WS_CHILD|WS_VISIBLE,16,16+i*76,370,22,window,nullptr,nullptr,nullptr);
            const DWORD special=i==2?ES_PASSWORD:i==3?ES_READONLY:i==4?WS_DISABLED:0;
            CreateWindowExW(WS_EX_CLIENTEDGE,i==1?L"RICHEDIT50W":L"EDIT",L"",WS_CHILD|WS_VISIBLE|WS_TABSTOP|ES_AUTOHSCROLL|special,
                            16,40+i*76,550,30,window,reinterpret_cast<HMENU>(static_cast<INT_PTR>(100+i)),nullptr,nullptr);
        }
        SetFocus(GetDlgItem(window,100));return 0;
    }
    if(message==WM_DESTROY){PostQuitMessage(0);return 0;}
    return DefWindowProcW(window,message,wparam,lparam);
}
}
int WINAPI wWinMain(HINSTANCE instance,HINSTANCE,PWSTR,int show){
    const auto rich=LoadLibraryExW(L"Msftedit.dll",nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32);
    if(!rich)return 1;
    WNDCLASSW cls{};cls.hInstance=instance;cls.lpfnWndProc=procedure;cls.lpszClassName=L"OwOCompatibilityHost";
    cls.hCursor=LoadCursorW(nullptr,IDC_IBEAM);cls.hbrBackground=reinterpret_cast<HBRUSH>(COLOR_WINDOW+1);
    if(!RegisterClassW(&cls))return 2;
    const auto title=sizeof(void*)==8?L"OwO compatibility host x64":L"OwO compatibility host x86";
    auto window=CreateWindowW(cls.lpszClassName,title,WS_OVERLAPPEDWINDOW,CW_USEDEFAULT,CW_USEDEFAULT,610,470,nullptr,nullptr,instance,nullptr);
    if(!window)return 3;ShowWindow(window,show);
    MSG message{};while(GetMessageW(&message,nullptr,0,0)>0){if(!IsDialogMessageW(window,&message)){TranslateMessage(&message);DispatchMessageW(&message);}}
    FreeLibrary(rich);return 0;
}
