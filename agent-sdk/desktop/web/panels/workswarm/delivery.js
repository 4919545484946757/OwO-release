(function(g){
  "use strict";
  function esc(value){return String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));}
  function size(n){return n<1024?n+' B':n<1024*1024?(n/1024).toFixed(1)+' KB':(n/1024/1024).toFixed(1)+' MB';}
  function html(data){
    const files=data.files||[],available=files.filter(f=>f.available);
    return '<div class="team-result-heading"><div><p class="team-result-eyebrow">你的任务</p><h2>'+esc(data.title)+'</h2><p class="hint">'+(available.length?(data.status==='succeeded'?'文件已保存，可直接打开。':'任务关联文件可打开，执行中内容可能继续更新。'):data.status==='succeeded'?'这个任务没有可打开的文件。文字结果和执行说明见下方记录。':'文件保存后会出现在这里。')+'</p></div><button type="button" class="owo-ws-mini" data-team-file-action="refresh">刷新</button></div>'+
      '<div class="team-file-heading"><h3>生成的文件 <span>'+available.length+'</span></h3><button type="button" class="owo-ws-mini" data-team-file-action="folder">打开任务文件夹</button></div>'+
      (files.length?'<div class="team-file-list">'+files.map((file,i)=>'<article class="team-file-row"><div class="team-file-icon" aria-hidden="true">'+esc(file.name.split('.').pop().toUpperCase().slice(0,5))+'</div><div class="team-file-info"><strong>'+esc(file.name)+'</strong><p>'+esc(file.relativePath)+'<span> · '+(file.available?size(file.size):'尚未生成或已移走')+'</span></p></div><div class="team-file-actions">'+(file.available?(file.canOpen?'<button type="button" class="primary" data-team-file-action="open" data-team-file-index="'+i+'">打开文件</button>':'')+'<button type="button" class="owo-ws-mini" data-team-file-action="reveal" data-team-file-index="'+i+'">所在文件夹</button>':'')+'</div></article>').join('')+'</div>':'<p class="team-file-empty">暂无交付文件</p>')+
      (data.notice?'<p class="hint">'+esc(data.notice)+'</p>':'')+'<p class="team-file-feedback" role="status" aria-live="polite"></p>';
  }
  function create({box,bridge}) {
    let disposed=false,seq=0,key='',data=null,pending=null,currentTeamId='',currentStatus='';
    async function load(teamId,status,force=false) {
      const next=teamId+':'+status;if(!force&&next===key)return pending;
      if(currentTeamId!==teamId)data=null;
      currentTeamId=teamId;currentStatus=status;key=next;const ticket=++seq;
      if(!data)box.innerHTML='<p class="hint">正在查找本次交付文件…</p>';
      pending=(async()=>{
        try {
          if(!bridge?.list)throw Error('请在桌面客户端查看真实文件；执行说明仍可在下方下载。');
          const result=await bridge.list({teamId});if(disposed||ticket!==seq)return;
          data=result;box.innerHTML=html(data);
        }catch(e){if(!disposed&&ticket===seq){key='';box.innerHTML='<h2>任务成果</h2><p role="alert">'+esc(e.message||e)+'</p><button type="button" class="owo-ws-mini" data-team-file-action="refresh">重新加载</button>';}}
      })();return pending;
    }
    const click=async event=>{
      const button=event.target.closest('[data-team-file-action]');if(!button||!box.contains(button))return;
      const action=button.dataset.teamFileAction;
      const teamId=currentTeamId;
      if(action==='refresh')return load(teamId,currentStatus,true);
      const file=data?.files[Number(button.dataset.teamFileIndex)];
      const feedback=box.querySelector('.team-file-feedback');button.disabled=true;
      try {await bridge.act({teamId,action,relativePath:file?.relativePath});if(feedback)feedback.textContent=action==='open'?'文件已打开。':'文件夹已打开。';}
      catch(e){if(feedback)feedback.textContent='打开失败：'+(e.message||e);}
      finally {button.disabled=false;}
    };
    box.addEventListener('click',click);
    return {load,dispose(){disposed=true;seq++;box.removeEventListener('click',click);}};
  }
  const api={create,html};g.OwoTeamDelivery=api;
  if(typeof module!=='undefined')module.exports=api;
})(typeof window!=='undefined'?window:globalThis);
