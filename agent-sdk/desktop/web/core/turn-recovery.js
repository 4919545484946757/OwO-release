(function (root) {
 "use strict";
 function observe({sessionId,turnId,getPage,onPage,onEnd,onError,visible,interval=1500,requestTimeout=10000}) {
  let timer,cursor=0,stopped=false,request=null;
  function schedule(ms){if(!stopped)timer=setTimeout(tick,ms);}
  async function tick(){
   if(stopped)return;
   if(!visible()){schedule(interval);return;}
   request=new AbortController(); const deadline=setTimeout(()=>request?.abort(),requestTimeout);
   try{
    const page=await getPage(root.OwoTurnSse.replayPath(sessionId,turnId,cursor,128),{signal:request.signal});
    if(stopped)return;
    const records=root.OwoTurnSse.eventsAfterCursor(page,turnId,cursor);
    if(records.length){cursor=records[records.length-1].seq;onPage(records,page);}
    if(!page.active){stopped=true;Promise.resolve(onEnd(page)).catch(onError);return;}
    schedule(records.length===128?0:interval);
   }catch(error){if(!stopped){onError(error);schedule(5000);}}
   finally{clearTimeout(deadline);request=null;}
  }
  schedule(0);
  return {stop(){stopped=true;clearTimeout(timer);request?.abort();}};
 }
 root.OwoTurnRecovery=Object.freeze({observe});
})(typeof window!=="undefined"?window:globalThis);
