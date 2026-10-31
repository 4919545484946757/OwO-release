#pragma once
#include <Windows.h>
#include <msctf.h>
#include <string_view>
namespace owo::tsf {
// Invoke only inside the guarded production read/write edit session.
// Success denotes SetText, independently of later caret/selection refresh.
HRESULT commit_scheme_selection(ITfContext*,TfEditCookie,std::wstring_view,ITfRange** refreshed_selection);
}
