// Host-owned media configuration. Drafts remain in memory; this module never stores credentials.
(function(root){
 "use strict";
 const SERVICES=["image_generation","text_to_speech"], LABELS={image_generation:"图片生成",text_to_speech:"语音合成"};
 const clone=v=>JSON.parse(JSON.stringify(v));
 const bytes=s=>new TextEncoder().encode(s).length;
 function validate(c){
  if(!c||c.schema_version!==1||!Number.isSafeInteger(c.revision)||c.revision<0||!Array.isArray(c.profiles)||c.profiles.length>64||!c.assignments||typeof c.assignments!=="object")throw Error("媒体配置版本或结构无效");
  const only=(o,names)=>{if(!o||Object.keys(o).some(k=>!names.includes(k)))throw Error("媒体配置含不支持的字段；请使用匹配版本的核心");};
  only(c,["schema_version","revision","profiles","assignments"]);only(c.assignments,SERVICES);
  const ids=new Set();
  for(const p of c.profiles){
   only(p,["id","name","base_url","model","credential_env","services"]);
   if(typeof p.id!=="string"||!/^[A-Za-z0-9_-]{1,64}$/.test(p.id)||ids.has(p.id))throw Error("连接 ID 无效或重复");ids.add(p.id);
   for(const [key,limit] of [["name",128],["model",256],["base_url",2048]])if(typeof p[key]!=="string"||!p[key].trim()||bytes(p[key])>limit||/[\x00-\x1f\x7f]/.test(p[key]))throw Error("请填写有效的连接名称、端点和模型");
   let url;try{url=new URL(p.base_url.trim());}catch{throw Error("端点必须是完整的 HTTP(S) URL");}
   if(!["http:","https:"].includes(url.protocol)||url.username||url.password||url.search||url.hash||p.base_url.includes("?")||p.base_url.includes("#"))throw Error("端点不能含凭据、查询或片段");
   if(p.credential_env!=null&&(typeof p.credential_env!=="string"||!/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(p.credential_env)||/^OWO_CUSTOM_MODEL_SECRET_/i.test(p.credential_env)))throw Error("凭据只能填写环境变量名；桌面模型保险库尚未接入媒体连接");
   if(!Array.isArray(p.services)||!p.services.length||p.services.length>2||new Set(p.services).size!==p.services.length||p.services.some(s=>!SERVICES.includes(s)))throw Error("请选择该连接的服务类型");
  }
  for(const [service,id] of Object.entries(c.assignments))if(!c.profiles.some(p=>p.id===id&&p.services.includes(service)))throw Error("服务选择指向不存在或不兼容的连接，请重新选择");
  if(bytes(JSON.stringify(c))>256*1024)throw Error("配置超过 256 KiB");
  return c;
 }
 function removeProfile(c,id){if(Object.values(c.assignments).includes(id))throw Error("请先切换该连接承担的服务，再删除连接");return {...clone(c),profiles:c.profiles.filter(p=>p.id!==id).map(clone)};}
 function connectionText(check){
  if(!check||check.kind!=="tcp"||typeof check.connected!=="boolean"||!Number.isSafeInteger(check.checked_at_ms)||check.checked_at_ms<=0)return "连接未测试；服务未验证";
  return (check.connected?"上次 TCP 可达":"上次 TCP 未连通")+" · "+new Date(check.checked_at_ms).toLocaleString()+"；服务未验证";
 }
 function mount(host,{api,onSaved=()=>{}}){
  if(!host)return null;
  const doc=host.ownerDocument;let catalog=null,checks=[],selected="",dirty=false,busy=false,destroyed=false,generation=0,discard=false;
  const controllers=new Set();
  const el=(tag,text,cls)=>{const e=doc.createElement(tag);if(text)e.textContent=text;if(cls)e.className=cls;return e;};
  const heading=el("div",null,"settings-block-head");heading.append(el("strong","媒体连接"));
  const refreshBtn=el("button","重新读取");refreshBtn.type="button";heading.append(refreshBtn);
  const note=el("p","分别选择图片与语音的端点、模型和凭据来源。保存不测试连接。","sub");
  const status=el("p","打开配置页后读取媒体连接。","sub");status.setAttribute("role","status");status.setAttribute("aria-live","polite");
  const discardBtn=el("button","放弃草稿并重新读取");discardBtn.type="button";discardBtn.hidden=true;
  const area=el("div",null,"media-profiles-body");host.replaceChildren(heading,note,status,discardBtn,area);
  const message=(text,error=false)=>{status.textContent=text;status.dataset.error=String(error);};
  const mutate=()=>{dirty=true;discard=false;discardBtn.hidden=true;message("有未保存的修改；保存后可测试连接。");updateControls();};
  const makeLabel=(title,control)=>{const label=el("label",null,"field-label");control.setAttribute("aria-label",title);label.append(el("span",title),control);return label;};
  const option=(select,value,text)=>{const o=el("option",text);o.value=value;select.append(o);};
  function updateControls(){for(const input of area.querySelectorAll("input,select,button"))input.disabled=busy;const save=area.querySelector('[data-action="save"]');if(save)save.disabled=busy||!dirty;const check=area.querySelector('[data-action="check"]');if(check)check.disabled=busy||dirty||!catalog?.profiles.some(p=>p.id===selected&&p.services.length);refreshBtn.disabled=busy;discardBtn.disabled=busy;host.setAttribute("aria-busy",String(busy));}
  function render(){
   area.replaceChildren();if(!catalog)return;
   const assignments=el("div",null,"media-profiles-grid");
   for(const service of SERVICES){const select=el("select");select.dataset.service=service;option(select,"","使用现有环境配置");for(const p of catalog.profiles)if(p.services.includes(service))option(select,p.id,p.name);const id=catalog.assignments[service]||"";if(id&&!Array.from(select.options).some(o=>o.value===id))option(select,id,"连接不兼容，请重新选择");select.value=id;select.addEventListener("change",()=>{if(select.value)catalog.assignments[service]=select.value;else delete catalog.assignments[service];mutate();});assignments.append(makeLabel(LABELS[service]+"连接",select));}
   area.append(assignments);
   const row=el("div",null,"media-profiles-actions"),picker=el("select");picker.setAttribute("aria-label","编辑媒体连接");option(picker,"","选择要编辑的连接");for(const p of catalog.profiles)option(picker,p.id,p.name);picker.value=selected;picker.addEventListener("change",()=>{selected=picker.value;render();});
   const add=el("button","新增连接");add.type="button";add.addEventListener("click",()=>{if(catalog.profiles.length>=64){message("最多保存 64 个连接",true);return;}selected="media-"+(root.crypto?.randomUUID?.()||Date.now().toString(36)+Math.random().toString(36).slice(2));catalog.profiles.push({id:selected,name:"新媒体连接",base_url:"",model:"",credential_env:null,services:["image_generation"]});mutate();render();area.querySelector('[data-field="name"]')?.focus();});row.append(picker,add);area.append(row);
   const p=catalog.profiles.find(p=>p.id===selected);
   if(p){
    const fields=el("div",null,"media-profiles-grid");
    for(const [key,title,placeholder]of [["name","连接名称","例如 本地图片"],["base_url","服务地址（Base URL）","https://api.example.com/v1"],["model","默认模型","服务支持的模型 ID"],["credential_env","凭据环境变量名","例如 MY_MEDIA_KEY，本机免密钥可留空"]]){const input=el("input");input.value=p[key]||"";input.placeholder=placeholder;input.autocomplete="off";input.dataset.field=key;input.addEventListener("input",()=>{p[key]=key==="credential_env"?(input.value||null):input.value;mutate();if(key==="name"){for(const o of host.querySelectorAll("option"))if(o.value===p.id)o.textContent=p.name;}});fields.append(makeLabel(title,input));}
    area.append(fields);
    const services=el("div",null,"media-profiles-actions");for(const service of SERVICES){const input=el("input");input.type="checkbox";input.checked=p.services.includes(service);input.dataset.profileService=service;input.addEventListener("change",()=>{p.services=input.checked?[...p.services,service]:p.services.filter(s=>s!==service);mutate();render();area.querySelector('[data-profile-service="'+service+'"]')?.focus();});services.append(makeLabel(LABELS[service],input));}area.append(services);
    const actions=el("div",null,"media-profiles-actions"),checkService=el("select");checkService.setAttribute("aria-label","测试服务类型");for(const service of p.services)option(checkService,service,LABELS[service]);
    const checkBtn=el("button","测试 TCP 连通");checkBtn.type="button";checkBtn.dataset.action="check";checkBtn.addEventListener("click",()=>testConnection(p.id,checkService.value));
    const remove=el("button","删除连接");remove.type="button";remove.addEventListener("click",()=>{try{catalog=removeProfile(catalog,p.id);selected="";mutate();render();}catch(error){message(error.message,true);}});actions.append(checkService,checkBtn,remove);area.append(actions);
    const check=checks.find(c=>c.profile_id===p.id&&c.service===checkService.value&&c.catalog_revision===catalog.revision),result=el("p",connectionText(check),"sub");result.dataset.checkResult="true";checkService.addEventListener("change",()=>{result.textContent=connectionText(checks.find(c=>c.profile_id===p.id&&c.service===checkService.value&&c.catalog_revision===catalog.revision));});area.append(result);
   }else area.append(el("p","新增连接后填写端点和模型，再为图片或语音选择该连接。","sub"));
   const save=el("button","保存媒体配置","primary");save.type="button";save.dataset.action="save";save.addEventListener("click",saveCatalog);area.append(save);updateControls();
  }
  async function request(path,options={}){const controller=new AbortController();controllers.add(controller);try{return await api(path,{...options,signal:controller.signal});}finally{controllers.delete(controller);}}
  function accept(data){validate(data?.catalog);catalog=clone(data.catalog);checks=Array.isArray(data.checks)?data.checks:[];if(!catalog.profiles.some(p=>p.id===selected))selected=catalog.profiles[0]?.id||"";dirty=false;discard=false;discardBtn.hidden=true;render();}
  async function refresh(force=false){
   if(destroyed||busy)return;
   if(dirty&&!force){discard=true;discardBtn.hidden=false;message("草稿尚未保存。可继续编辑，或明确放弃草稿后重新读取。",true);return;}
   const token=++generation;busy=true;updateControls();message("正在读取媒体连接…");
   try{const data=await request("/settings/media-connections");if(destroyed||token!==generation)return;accept(data);message("媒体配置已读取；测试仅由按钮触发。");}
   catch(error){if(!destroyed&&token===generation)message(error.status===404||error.status===405?"当前核心不支持媒体连接档，请更新核心。":String(error.message||error),true);}
   finally{if(!destroyed&&token===generation){busy=false;updateControls();}}
  }
  async function saveCatalog(){
   if(destroyed||busy||!dirty)return;
   let proposal;try{proposal=clone(validate(catalog));}catch(error){message(error.message,true);return;}
   const token=++generation;busy=true;updateControls();message("正在保存媒体配置…");
   try{const data=await request("/settings/media-connections",{method:"PUT",body:JSON.stringify(proposal)});if(destroyed||token!==generation)return;accept(data);message("已保存媒体配置；连接与服务尚未验证。");Promise.resolve(onSaved(data)).catch(()=>{});}
   catch(error){if(!destroyed&&token===generation)message(error.status===409?"配置已被其他窗口修改。当前草稿保留，请重新读取后合并。":"保存未确认，草稿保留。请重新读取核对："+String(error.message||error),true);}
   finally{if(!destroyed&&token===generation){busy=false;updateControls();}}
  }
  async function testConnection(id,service){
   if(destroyed||busy||dirty||!SERVICES.includes(service))return;
   const token=++generation;busy=true;updateControls();message("正在测试 TCP 连通，最长 3 秒；不会调用生成模型。");
   try{const check=await request("/settings/media-connections/check",{method:"POST",body:JSON.stringify({expected_revision:catalog.revision,profile_id:id,service})});if(destroyed||token!==generation)return;if(check.catalog_revision!==catalog.revision||check.profile_id!==id||check.service!==service)throw Error("连接测试结果与当前配置不匹配");checks=checks.filter(c=>!(c.profile_id===id&&c.service===service));checks.push(check);render();message(connectionText(check));Promise.resolve(onSaved()).catch(()=>{});}
   catch(error){if(!destroyed&&token===generation)message(error.status===409?"测试期间配置已变化，请重新读取后测试。":String(error.message||error),true);}
   finally{if(!destroyed&&token===generation){busy=false;updateControls();}}
  }
  refreshBtn.addEventListener("click",()=>refresh());discardBtn.addEventListener("click",()=>{if(discard)refresh(true);});
  return {refresh,destroy(){destroyed=true;++generation;for(const controller of controllers)controller.abort();controllers.clear();host.replaceChildren();},snapshot(){return {catalog:catalog&&clone(catalog),dirty,busy};}};
 }
 const api={validate,removeProfile,connectionText,mount};root.OwoMediaProfilesSettings=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
