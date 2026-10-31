#pragma once
#include <Windows.h>
#include <cstdio>
namespace owo::tsf {
// Called outside DllMain only. No typed text is recorded.
inline void activation_trace(const char* event, HRESULT result = S_OK, const void* context = nullptr) noexcept {
 wchar_t root[MAX_PATH]{}; GetEnvironmentVariableW(L"LOCALAPPDATA",root,MAX_PATH);
 wchar_t path[MAX_PATH]{}; swprintf_s(path,L"%s\\OwO-tsf-activation.log",root);
 char line[1024]{}; HMODULE module{};wchar_t image[MAX_PATH]{};
 GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,reinterpret_cast<LPCWSTR>(&activation_trace),&module);
 GetModuleFileNameW(module,image,MAX_PATH);
 // A DLL hosted by a packaged app can have a different AppData view from Core.
 // For a writable development deployment, record beside the loaded module so
 // diagnostics identify that exact image independently of package redirection.
 wchar_t deployment_log[MAX_PATH]{};
 if(swprintf_s(deployment_log,L"%s.%lu.trace.log",image,GetCurrentProcessId())>=0)
  wcscpy_s(path,deployment_log);
 int n=sprintf_s(line,"pid=%lu tid=%lu event=%s hr=%08lx context=%p dll=%ls\r\n",GetCurrentProcessId(),GetCurrentThreadId(),event,static_cast<unsigned long>(result),context,image);
 if(n<=0)return;OutputDebugStringA(line);
 HANDLE file=CreateFileW(path,FILE_APPEND_DATA,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);
 if(file==INVALID_HANDLE_VALUE){
  // Packaged/low-integrity hosts may not write to the desktop LocalAppData root.
  // Use the host's own writable temp location; never change directory ACLs.
  wchar_t temp[MAX_PATH]{};
  const auto length=GetTempPathW(MAX_PATH,temp);
  if(!length||length>=MAX_PATH||swprintf_s(path,L"%sOwO-tsf-activation-%lu.log",temp,GetCurrentProcessId())<0)return;
  file=CreateFileW(path,FILE_APPEND_DATA,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);
 }
 if(file!=INVALID_HANDLE_VALUE){LARGE_INTEGER size{};if(!GetFileSizeEx(file,&size)||size.QuadPart>=4*1024*1024){CloseHandle(file);return;}DWORD written{};WriteFile(file,line,static_cast<DWORD>(n),&written,nullptr);CloseHandle(file);}
}
}

