(function(root){
 "use strict";
 function annotate(host){
  for(const control of host.querySelectorAll("input,select,textarea,button")){
   if(control.type==="hidden" || control.hasAttribute("aria-label") || control.hasAttribute("aria-labelledby"))continue;
   if(control.labels?.length || control.closest("label"))continue;
   const row=control.closest(".settings-row");
   const title=row?.querySelector(".settings-row-label strong")?.textContent?.trim();
   const hint=control.getAttribute("title") || control.getAttribute("placeholder");
   const label=title || hint;
   if(label && (control.tagName!=="BUTTON" || !control.textContent.trim()))control.setAttribute("aria-label",label);
  }
 }
 function watch(host){
  annotate(host);let pending=false,stopped=false;
  const observer=new MutationObserver(()=>{
   if(pending)return;pending=true;
   queueMicrotask(()=>{pending=false;if(!stopped)annotate(host);});
  });
  observer.observe(host,{childList:true,subtree:true});
  return ()=>{stopped=true;observer.disconnect();};
 }
 root.OwoAccessibility=Object.freeze({annotate,watch});
})(typeof window!=="undefined"?window:globalThis);
