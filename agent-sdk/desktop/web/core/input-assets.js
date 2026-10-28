// Explicit task inputs. Asset bytes, ownership and capability remain host decisions.
(function(root){
  "use strict";
  var MAX_ROWS=256;
  function esc(s){return String(s==null?"":s).replace(/[&<>"']/g,function(c){return {"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c];});}
  function sourceContext(snapshot){
    snapshot=snapshot||{};
    var session=(Array.isArray(snapshot.sessions)?snapshot.sessions:[]).find(function(s){return s&&s.id===snapshot.sessionId&&s.workspace===snapshot.workspace;});
    return session&&snapshot.workspace?{sessionId:String(session.id),workspace:String(snapshot.workspace)}:null;
  }
  function key(context){return context?context.sessionId+"\n"+context.workspace:"";}
  function create(options){
    options=options||{};
    var container=null,disposed=false,epoch=0,request=0;
    var state={context:null,rows:[],selected:new Set(),loading:false,busy:false,ready:false,error:"",notice:""};
    function current(){return sourceContext(options.snapshot?options.snapshot():{});}
    function rowsSelected(){return state.rows.filter(function(row){return state.selected.has(row.id);});}
    function selectedParts(){
      if(disposed)throw new Error("任务附件页面已关闭，请重新打开");
      if(state.busy)throw new Error("任务附件正在上传，请等待上传完成");
      if(!state.selected.size)return [];
      if(key(current())!==key(state.context))throw new Error("对话或工作区已切换，请刷新任务附件后重选");
      if(!state.ready||state.loading||state.busy)throw new Error("任务附件尚未就绪，请等待上传或刷新完成");
      var rows=rowsSelected();
      if(rows.length!==state.selected.size)throw new Error("选中的附件已不在当前列表，请重新选择");
      if(rows.some(function(row){return row.unavailable;}))throw new Error("选中的附件当前不可作为输入");
      var input=root.OwoAttachmentUpload.turnInput(rows);
      if(input.attachments.length)throw new Error("旧附件需要重新上传后才能加入任务");
      if(rows.reduce(function(sum,row){return sum+(row.content_part.type==="image"?row.size:0);},0)>20*1024*1024)throw new Error("每次图片输入合计不超过 20 MiB");
      return input.content_parts;
    }
    function normalize(row){
      var result={id:String(row&&row.id||""),name:String(row&&row.name||"未命名附件"),mime:String(row&&row.mime||"application/octet-stream"),size:row&&row.size,content_part:row&&row.content_part};
      if(!Number.isSafeInteger(result.size)||result.size<0){result.unavailable="附件大小无效，请重新上传";return result;}
      try{var input=root.OwoAttachmentUpload.turnInput([result]);if(input.attachments.length)result.unavailable="旧附件需重新上传";}
      catch(error){result.unavailable=error.message||"附件引用无效";}
      return result;
    }
    function view(){return {context:state.context,rows:state.rows.map(function(row){return Object.assign({},row,{selected:state.selected.has(row.id)});}),loading:state.loading,busy:state.busy,ready:state.ready,error:state.error,notice:state.notice,count:state.selected.size};}
    function html(){
      var disabled=!state.context||!state.ready||state.loading||state.busy;
      var rows=state.rows.map(function(row){
        var unavailable=!!row.unavailable;
        return '<label style="display:flex;gap:8px;align-items:flex-start;min-width:0;overflow-wrap:anywhere"><input type="checkbox" data-input-id="'+esc(row.id)+'"'+(state.selected.has(row.id)?' checked':'')+(disabled||unavailable?' disabled':'')+'><span style="min-width:0">'+esc(row.name)+' <span class="hint">'+esc(Math.ceil(row.size/1024)||0)+' KiB · '+esc(row.mime)+(row.content_part&&row.content_part.asset&&typeof row.content_part.asset.sha256==="string"&&/^[0-9a-f]{64}$/.test(row.content_part.asset.sha256)?' · 版本 '+esc(row.content_part.asset.sha256.slice(0,12)):'')+'</span>'+(unavailable?'<br><span class="hint">'+esc(row.unavailable)+'</span>':'')+'</span></label>';
      }).join('');
      var selected=rowsSelected().filter(function(row){return !row.unavailable&&row.content_part.type==="image";});
      var previews=selected.map(function(row){return '<div data-media-artifact="'+esc(JSON.stringify({tool:"generate_image",outputs:[{artifact_id:row.id,kind:"image",mime_type:row.mime,bytes:row.size,sha256:row.content_part.asset.sha256}]}))+'"></div>';}).join('');
      return '<fieldset style="min-width:0"><legend>任务附件（可选）</legend><p class="hint">仅勾选的附件会加入本次运行。图片将发给该运行的 Agent 角色；音频和视频输入尚未配置。</p>'+
        (!state.context?'<p class="hint">请先在当前工作区打开或新建对话，再添加任务附件。</p>':'')+
        '<div class="owo-ws-inline"><button type="button" data-input-refresh'+(state.loading||state.busy?' disabled':'')+'>刷新附件</button><button type="button" data-input-clear'+(!state.selected.size||state.busy?' disabled':'')+'>移除选中附件</button><label>添加附件 <input type="file" data-input-upload multiple accept="image/png,image/jpeg,image/gif,image/webp,.txt,.md,.pdf,.json,.csv"'+(disabled?' disabled':'')+' style="max-width:100%"></label></div>'+
        '<p role="status" aria-live="polite" style="overflow-wrap:anywhere">'+esc(state.error||state.notice||(state.loading?'正在读取附件…':state.context?(state.rows.length?'已选择 '+state.selected.size+' 个附件':'当前对话尚无附件'):'未选择来源对话'))+'</p>'+rows+previews+'</fieldset>';
    }
    function paint(){
      if(disposed)return;
      if(options.onView)options.onView(view());
      if(!container)return;
      var doc=container.ownerDocument,active=doc&&doc.activeElement;
      var focus=active&&container.contains(active)&&active.getAttribute?active.getAttribute("data-input-id"):null;
      if(root.OwoMediaResult&&root.OwoMediaResult.renderInto)root.OwoMediaResult.renderInto(container,html());else container.innerHTML=html();
      if(focus&&/^input-[0-9a-f-]+$/.test(focus)){var replacement=container.querySelector('[data-input-id="'+focus+'"]');if(replacement)replacement.focus({preventScroll:true});}
    }
    async function refresh(){
      if(disposed)return;
      var context=current(),changed=key(context)!==key(state.context);
      if(state.busy&&!changed)return;
      if(changed){epoch++;state.selected.clear();state.rows=[];state.busy=false;}
      state.context=context;state.ready=false;state.error="";state.notice="";
      if(!context){state.loading=false;paint();return;}
      var token=++request,scope=key(context);state.loading=true;paint();
      function valid(){return !disposed&&token===request&&scope===key(current())&&scope===key(state.context);}
      try{
        var capabilities=await options.get('/capabilities');if(!valid())return;
        if(!capabilities||!capabilities.constraints||capabilities.constraints.structured_input_parts!==true)throw new Error("当前核心不支持任务附件，请更新核心后刷新；不会仅发送文字");
        var rows=await options.get('/session/'+encodeURIComponent(context.sessionId)+'/attachments');if(!valid())return;
        if(!Array.isArray(rows)||rows.length>MAX_ROWS||new TextEncoder().encode(JSON.stringify(rows)).length>512*1024)throw new Error("附件列表超过当前显示范围，请先整理该对话的附件");
        var ids=new Set(),normalized=rows.map(normalize);
        if(normalized.some(function(row){if(!row.id||ids.has(row.id))return true;ids.add(row.id);return false;}))throw new Error("附件列表包含重复或无效引用，请刷新后重试");
        var old=new Map(state.rows.map(function(row){return [row.id,row.content_part&&JSON.stringify(row.content_part)];}));
        state.rows=normalized;state.selected=new Set(normalized.filter(function(row){return !row.unavailable&&state.selected.has(row.id)&&old.get(row.id)===JSON.stringify(row.content_part);}).map(function(row){return row.id;}));state.ready=true;
      }catch(error){if(valid()){state.error=error.message||"读取任务附件失败";state.ready=false;}}
      finally{if(valid()){state.loading=false;paint();}}
    }
    function select(id,checked){
      if(disposed)return false;
      if(key(current())!==key(state.context)){refresh();return false;}
      if(!state.ready||state.loading||state.busy)return false;
      var row=state.rows.find(function(row){return row.id===id;});if(!row||row.unavailable)return false;
      var before=new Set(state.selected);if(checked)state.selected.add(id);else state.selected.delete(id);
      try{selectedParts();state.error="";state.notice="";}catch(error){state.selected=before;state.error=error.message;paint();return false;}
      paint();return true;
    }
    async function uploadFiles(files){
      files=Array.from(files||[]);if(!files.length||disposed)return;
      if(key(current())!==key(state.context)){await refresh();throw new Error("对话已切换，请在新的来源对话重新选择附件");}
      if(!state.context||!state.ready||state.loading||state.busy)throw new Error("请先选择来源对话并等待附件就绪");
      var generation=epoch,scope=key(state.context),sessionId=state.context.sessionId;state.busy=true;state.error="";paint();
      function valid(){return !disposed&&generation===epoch&&scope===key(current())&&scope===key(state.context);}
      try{
        await root.OwoAttachmentUpload.upload(files,{sessionId:sessionId,isCurrent:valid,read:options.read,
          request:function(path,request){return options.post(path,JSON.parse(request.body));},
          onUploaded:function(uploaded){
            if(!valid())return;var row=normalize(uploaded),existing=state.rows.find(function(r){return r.id===row.id;});
            if(existing){if(JSON.stringify(existing.content_part)!==JSON.stringify(row.content_part)){state.error="上传返回了冲突引用，请刷新后重试";return;}}
            else if(state.rows.length>=MAX_ROWS){state.error="附件已上传，但当前列表已满；未自动选中";return;}
            else state.rows.push(row);
            if(row.unavailable)state.error=row.unavailable;
            else {var before=new Set(state.selected);state.selected.add(row.id);state.busy=false;try{selectedParts();}catch(error){state.selected=before;state.error=error.message;}state.busy=true;}
            paint();
          },onError:function(file,error){if(valid()){state.error='上传失败 '+file.name+'：'+(error.message||error);paint();}},
          onStopped:function(){if(!disposed)state.notice="来源对话已切换；原上传未加入当前任务";}
        });
      }finally{if(valid()){state.busy=false;paint();}else if(!disposed){await refresh();}}
    }
    function change(event){var target=event.target;if(target&&target.hasAttribute('data-input-id'))select(target.getAttribute('data-input-id'),target.checked);else if(target&&target.hasAttribute('data-input-upload')){var files=Array.from(target.files||[]);target.value='';uploadFiles(files).catch(function(error){state.error=error.message;paint();});}}
    function clearSelection(){if(disposed||state.busy)return false;state.selected.clear();if(state.ready)state.error="";state.notice="已移除选中附件";paint();return true;}
    function click(event){var target=event.target;if(target&&target.closest){if(target.closest('[data-input-refresh]'))refresh();else if(target.closest('[data-input-clear]'))clearSelection();}}
    function mount(node){
      if(container&&container!==node){container.removeEventListener('change',change);container.removeEventListener('click',click);}
      if(container!==node&&node){node.addEventListener('change',change);node.addEventListener('click',click);}
      container=node;if(!node||disposed)return;
      if(!state.context||key(current())!==key(state.context))return refresh();paint();
    }
    function applyTo(body){var parts=selectedParts();if(parts.length){if(body.workspace&&body.workspace.root!==state.context.workspace)throw new Error("任务附件与运行工作区不一致，请重新选择");if(body.parent_session_id&&body.parent_session_id!==state.context.sessionId)throw new Error("任务附件与来源对话不一致，请重新选择");body.parent_session_id=state.context.sessionId;body.content_parts=parts;}return body;}
    function dispose(){disposed=true;epoch++;request++;if(container){container.removeEventListener('change',change);container.removeEventListener('click',click);if(root.OwoMediaResult&&root.OwoMediaResult.renderInto)root.OwoMediaResult.renderInto(container,'');else container.innerHTML='';}container=null;state.selected.clear();}
    return {mount:mount,refresh:refresh,select:select,clearSelection:clearSelection,uploadFiles:uploadFiles,applyTo:applyTo,view:view,dispose:dispose};
  }
  function mountPanel(controller,container,helpers){if(!container)return controller;if(!controller)controller=create({snapshot:helpers.snapshot,get:helpers.get,post:helpers.post});controller.mount(container);return controller;}
  var api={create:create,sourceContext:sourceContext,mountPanel:mountPanel};root.OwoInputAssets=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
