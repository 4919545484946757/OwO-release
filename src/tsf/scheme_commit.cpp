#include "owo/tsf/scheme_commit.h"
#include <limits>
namespace owo::tsf {
HRESULT commit_scheme_selection(ITfContext* context,TfEditCookie cookie,std::wstring_view text,ITfRange** refreshed, ITfComposition* preview) {
 if(!context||!refreshed||text.empty()||text.size()>static_cast<std::size_t>((std::numeric_limits<LONG>::max)()))return E_INVALIDARG;
 TF_SELECTION selection{};ULONG count{};
 auto written=context->GetSelection(cookie,TF_DEFAULT_SELECTION,1,&selection,&count);
 if(FAILED(written)||count!=1||!selection.range){if(selection.range)selection.range->Release();return FAILED(written)?written:E_FAIL;}
 if(preview) {
  selection.range->Release();selection.range=nullptr;
  written=preview->GetRange(&selection.range);
  if(FAILED(written)||!selection.range){if(selection.range)selection.range->Release();return FAILED(written)?written:E_FAIL;}
 }
 written=selection.range->SetText(cookie,0,text.data(),static_cast<LONG>(text.size()));
 if(SUCCEEDED(written)) {
  (void)selection.range->Collapse(cookie,TF_ANCHOR_END);(void)context->SetSelection(cookie,1,&selection);
  if(preview)(void)preview->EndComposition(cookie);
  if(*refreshed)(*refreshed)->Release();*refreshed=nullptr;
  TF_SELECTION fresh{};ULONG fetched{};
  if(SUCCEEDED(context->GetSelection(cookie,TF_DEFAULT_SELECTION,1,&fresh,&fetched))&&fetched==1)*refreshed=fresh.range;
  else if(fresh.range)fresh.range->Release();
 }
 selection.range->Release();return written;
}
}
