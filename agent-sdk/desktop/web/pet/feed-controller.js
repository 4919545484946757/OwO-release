/* Cuttle feed: real TeamRun, shared API, distinct role mascots. */
(function(g) {
  'use strict';
  const norm = s => String(s || '').toLowerCase().replace(/[_\s-]/g, '');
  function validate(text) {
    if (typeof text !== 'string' || !text.trim()) throw Error('先复制或拖入一段需求');
    if (text.length > 8000) throw Error('需求超过 8000 字，请缩短后投喂');
    return text.trim();
  }
  function label(snapshot) {
    if (snapshot.interrupted) return '任务已中断，请到工作台恢复';
    const team = snapshot.team || snapshot;
    const status = norm(team.status);
    if (status === 'succeeded') return '任务完成，查看成果';
    if (status === 'failed') return '任务失败，查看原因';
    if (status === 'cancelled') return '已停止，已有成果保留';
    if (status === 'awaitinghuman') return '需要你处理，请查看工作台';
    const active = (snapshot.tasks || []).filter(t => norm(t.status) === 'running');
    return active.length ? active.map(t => ['lead','planner'].includes(t.role)?'正在明确需求与安排任务':/reviewer|critic/.test(t.role||'')?'正在评审成果':'正在执行任务').join(' · ') : '正在准备任务';
  }
  function commanderRoles(configured, defaults, team) {
    const input=(configured||[]).map(role=>({...role,assignee:role.assignee||'agent'}));
    if(input.some(role=>role.assignee!=='agent'))throw Error('这个队伍包含人工节点，请在队伍工作台发起；桌宠指挥模式使用 Agent 成员');
    if(input.some(role=>role.worker&&role.worker!=='agent'))throw Error('桌宠指挥模式需要模型 Agent；其他执行器请先在工作台配置适配器');
    const isReview=role=>role.capabilities?.includes('review')||/reviewer|critic|judge/.test(role.role);
    const lead=input.find(role=>['lead','planner'].includes(role.role))||{};
    const review=input.find(isReview)||{};
    let workers=input.filter(role=>!isReview(role)&&!['lead','planner','leader'].includes(role.role));
    if(!workers.length)workers=Array.from({length:Math.max(1,Math.min(6,Number(team.parallel)||1))},()=>({}));
    if(workers.length>6)throw Error('桌宠队伍最多 6 名执行 Agent，请到工作台使用更大队伍');
    const reserved=new Set(workers.filter(source=>/^w[1-9][0-9]*$/.test(source.role||'')).map(source=>source.role));let slot=1;
    const slots=workers.map(source=>{
      while(reserved.has('w'+slot))slot++;
      const role=/^w[1-9][0-9]*$/.test(source.role||'')?source.role:'w'+slot++;reserved.add(role);
      const paths=source.write_paths?.length?source.write_paths:['outputs'];
      if(paths.some(path=>typeof path!=='string'||!(path==='outputs'||path.startsWith('outputs/'))))throw Error('桌宠指挥模式成果限于 outputs，其他写范围请使用队伍工作台');
      return {...source,...defaults[1],role,model:source.model,write_paths:[...paths],handoff_contract:defaults[1].handoff_contract+(source.handoff_contract?' 该成员原有职责与约束：'+source.handoff_contract:''),extra_input:{...(source.extra_input||{}),specialty:source.role||'执行者'}};
    });
    if(new Set(slots.map(role=>role.role)).size!==slots.length)throw Error('执行成员标识重复，请检查队伍配置');
    const roster=slots.map(role=>({worker:role.role,specialty:role.extra_input.specialty,contract:role.handoff_contract,write_paths:role.write_paths,model:role.model||team.model||'默认模型'}));
    const direction='你是指挥 Agent，使用真实队伍模式。先在 brief 中明确 goal、deliverables、constraints、assumptions；再生成任务 DAG。task 字段就是下发给负责 Agent 的完整执行提示词，写清输入、制作步骤、交付物相对路径和验收方式。只能选择下列执行成员：'+JSON.stringify(roster)+'。不能把指挥或评审员当执行者；不要虚构 API、文件或已经完成的结果。关键条件缺失时返回 blocked 并在 open_issues 指明需要用户补充什么，停止派发。用户要求保存到磁盘的文档、数据或其他非代码成果，也必须指定具体 write_paths，并使用 VerificationPlanV1 的 workspace-file-non-empty-v1，validator_version="1"，scope.kind=workspace_paths，relative_paths 包含每个交付文件，arguments={}，required=true，resources={cpu_slots:1,memory_mb:64,exclusive_workspace:false,timeout_ms:1000}；不得用 non_empty 代替文件落盘验证。只交付聊天文案且用户未要求文件时才用 non_empty。写文件和执行命令须等待宿主原有权限。';
    const contract=defaults[0].handoff_contract.replace('worker=w1','worker 从执行成员列表选择');
    return [
      {...defaults[0],model:lead.model,handoff_contract:direction+contract},
      ...slots,
      {...defaults[2],model:review.model,handoff_contract:defaults[2].handoff_contract+(review.handoff_contract?' 原评审职责：'+review.handoff_contract:''),depends_on:slots.map(role=>role.role)},
    ];
  }
  function buildTeamRequest(text, workspace, settings = {}, options = {}) {
    const team=settings.team || {};
    const leadContract='只读理解用户目标，输出合法 TaskGraphV1 JSON：version=1，tasks 为 1..128 个任务。每项包含 task_id、worker=w1、task、acceptance、depends_on、requirement_quotes、write_paths、read_refs、contract_refs、required_capabilities、estimated_effort、verification、risk、priority。requirement_quotes 必须逐字引用用户目标，并逐字出现在本任务 task 或 acceptance 中。所有用户要求至少被一个任务覆盖。estimated_effort 必须为 1..1000000 的 JSON 整数，禁止 small 等字符串；risk 只能是 low|normal|high|critical；priority 必须为 0..100 的 JSON 整数。depends_on、requirement_quotes、write_paths、read_refs、contract_refs、required_capabilities 均为字符串数组，无需内容时传 []。写路径只能在 outputs 内。代码任务的 verification 必须是 VerificationPlanV1：plan_id=verify-task-id，requirements 每项带 requirement_id、validator_id=workspace-command-success-v1、validator_version="1"、scope={kind:workspace_paths,relative_paths:[所有被改源文件与测试文件]}、arguments={command:node --test outputs/page.test.cjs}、required=true、resources={cpu_slots:1,memory_mb:64,exclusive_workspace:false,timeout_ms:30000}，并请求 read_file、write_file、run_command 能力。仅聊天文本的 verification 必须直接写 JSON 字符串 "non_empty"，即 "verification":"non_empty"；禁止写成对象或 validator_id=non_empty，也不允许空 workspace_paths。只申请实际需要的能力，不输出预算字段。依赖引用 task_id，图无环。不要为了分工拆出无关任务。TaskGraph JSON 放入 WorkerOutputV1.artifact.content，artifact.format=json，status=done。';
    const defaults=[
      {role:'lead',assignee:'agent',verify:'non_empty',handoff_contract:leadContract+' 代码任务 verification 示例（路径替换为真实改动的所有源码与测试文件）：'+JSON.stringify({"plan_id": "verify-page", "requirements": [{"requirement_id": "page-behavior", "validator_id": "workspace-command-success-v1", "validator_version": "1", "scope": {"kind": "workspace_paths", "relative_paths": ["outputs/page.html", "outputs/page.test.cjs"]}, "arguments": {"command": "node --test outputs/page.test.cjs"}, "required": true, "resources": {"cpu_slots": 1, "memory_mb": 64, "exclusive_workspace": false, "timeout_ms": 30000}}]})},
      {role:'w1',assignee:'agent',depends_on:['lead'],verify:'non_empty',write_paths:['outputs'],handoff_contract:'按宿主 assigned_task/assigned_acceptance/assigned_verification 执行。只写 assigned_write_paths。可运行的 HTML/代码必须在授权范围内落盘，并运行登记的真实行为测试。Node 场景使用 node --test outputs/page.test.cjs，测试必须实际执行被测交互，不能仅字符串非空或固定通过；检查后不得修改源码。提供完整产物和宿主证据，不发布公网、不安装依赖。'},
      {role:'reviewer',assignee:'agent',capabilities:['review'],depends_on:['w1'],verify:'non_empty',handoff_contract:'只读独立评审最终产物及实际行为证据；输出宿主要求的结构化 review_result，缺陷定位到具体 task/artifact。不得给自己授权或修改被审文件。'}
    ];
    const roles=options.commander ? commanderRoles(Array.isArray(team.roles)?JSON.parse(JSON.stringify(team.roles)):[],defaults,team) : Array.isArray(team.roles)&&team.roles.length ? JSON.parse(JSON.stringify(team.roles)).map(role=>({...role,assignee:role.assignee||'agent'})) : defaults;
    if(roles.some(x=>typeof x.role!=='string'||!x.role.trim()||x.role.length>96||['__proto__','constructor','prototype'].includes(x.role)))throw Error('成员标识无效，请检查团队配置');
    if(roles.filter(x=>x.assignee==='agent').length>8)throw Error('桌宠模式最多显示 8 名 Agent，请使用团队工作台运行更大团队');
    const body={objective:text,mode:'team',strategy:'team',roles,
      parallel:options.commander || (!Array.isArray(team.roles)||!team.roles.length ? true : !!team.parallel || roles.some(x=>x.role==='lead')),
      max_agent_members:Math.max(roles.filter(x=>x.assignee==='agent').length,3),
      budget:{max_steps:32,max_retries_per_step:2,max_total_retries:6,max_model_calls:48,max_wall_secs:600,max_duration_secs:600,max_parallel:Math.min(2,Number(team.max_parallel)||2)},
      workspace:{root:workspace,read_only:false,write_allowed_paths:['outputs']}};
    const model=team.model || settings.runtime?.model || settings.model;
    if(typeof model==='string'&&model.trim())body.model=model.trim();
    return body;
  }
  function createController(deps) {
    let busy=false, ref=null, timer=null, disposed=false, pending=null, generation=0, status='', interrupted=false;
    const emit=(text,data)=>{if(!disposed)deps.update(text,{ref,busy,...data});};
    function stopTimer(){if(timer!==null)deps.clear(timer);timer=null;}
    async function save(value){try{await deps.save(value);}catch{emit('观察正常，但任务绑定未保存，请查看工作台',{error:true});}}
    async function bind(value){stopTimer();const epoch=++generation;ref=value;busy=true;status='';interrupted=false;await save(value);return epoch;}
    async function reconcile(){
      if(!ref?.pending || disposed)return;
      const current=ref, epoch=generation;
      try{
        const result=await deps.request('/teams');
        if(disposed||epoch!==generation)return;
        const found=(result.teams||[]).find(x=>x.goal_id===current.goalId);
        if(found){await bind({id:found.team_id,projectId:found.project_space_id,members:found.members||[]});return observe();}
        emit('启动结果待确认：右键查看团队记录，确认后可解除绑定',{uncertain:true});
      }catch{emit('无法核对启动结果，请到工作台确认',{error:true,uncertain:true});}
    }
    async function observe(){
      if(!ref||disposed)return;
      if(ref.pending)return reconcile();
      const id=ref.id,epoch=generation;
      stopTimer();
      try{
        const snapshot=await deps.request('/teams/'+encodeURIComponent(id));
        if(disposed||epoch!==generation)return;
        status=norm((snapshot.team||snapshot).status);
        interrupted=!!snapshot.interrupted;
        busy=!interrupted&&!['succeeded','failed','cancelled'].includes(status);
        if(snapshot.team?.members){
          const signature=members=>JSON.stringify((members||[]).map(m=>[m.role,m.member_id,m.runtime_binding]));
          const changed=signature(ref.members)!==signature(snapshot.team.members);
          ref={...ref,members:snapshot.team.members};
          if(changed){await save(ref);if(disposed||epoch!==generation)return;}
        }
        emit(label(snapshot),{snapshot,status});
      }catch(e){
        if(disposed||epoch!==generation)return;
        if(e.status===404){ref=null;busy=false;status='';interrupted=false;generation++;await save(null);emit('原任务已不存在，可以重新投喂',{error:true});return;}
        busy=true;emit(e.status===401||e.status===403?'连接未授权，请重新连接工作台':'连接中断，正在重新核对任务状态',{error:true});
      }
      // Failed/Cancelled runs can be explicitly resumed from the workbench.
      if(!disposed&&epoch===generation&&ref)timer=deps.schedule(observe,busy?1200:5000);
    }
    return {
      async feed(raw){
        if(disposed)return;
        if(pending||busy){emit('已有任务，请先查看或停止');return;}
        let text;try{text=validate(raw);}catch(e){emit(e.message,{error:true});return;}
        const submission={};pending=submission;
        let sent=false,feedEpoch=generation;
        const alive=()=>!disposed&&feedEpoch===generation;
        try{
          if(ref){await observe();if(!alive())return;if(busy){emit('原任务正在恢复，请先查看');return;}}
          const workspace=await deps.workspace();if(!alive())return;
          if(!workspace||typeof workspace!=='string')throw Error('请先在工作台选择项目目录');
          const settings=deps.settings?await deps.settings():{};if(!alive())return;
          const preview=buildTeamRequest(text,workspace,settings,{commander:true});
          emit('收到需求，等你确认后再执行',{confirming:true});
          await deps.eat(text);if(!alive())return;
          if(!deps.confirm)throw Error('任务确认窗口未连接，请重新打开桌宠');
          const names={lead:'指挥员',reviewer:'验收员'};
          const answer=await deps.confirm({text,workspace,previousInterrupted:interrupted,roles:preview.roles.map(role=>({name:names[role.role]||role.extra_input?.specialty||'执行者',model:role.model||preview.model||'默认模型'}))});
          if(!alive())return;
          if(!answer){emit('已取消，任务没有执行');return;}
          text=validate(typeof answer==='string'?answer:answer.text);
          const currentWorkspace=await deps.workspace();if(!alive())return;
          if(currentWorkspace!==workspace)throw Error('项目已改变，请重新确认任务');
          if(ref){await observe();if(!alive())return;if(busy){emit('原任务正在执行，请先查看或停止');return;}}
          const body=buildTeamRequest(text,workspace,settings,{commander:true});
          const goalId=deps.id?deps.id():'feed-'+g.crypto.randomUUID();
          body.goal_id=goalId;
          feedEpoch=await bind({pending:true,goalId});if(!alive())return;
          emit('已确认，指挥员开始明确需求并安排任务',{model:body.model});
          sent=true;
          const result=await deps.request('/teams',{method:'POST',json:body});
          if(!alive())return;
          if(!result.team_id)throw Error('启动响应缺少任务标识');
          feedEpoch=await bind({id:result.team_id,projectId:result.project_space_id,members:result.members||[],model:body.model,roleModels:Object.fromEntries(body.roles.map(role=>[role.role,role.model||body.model||'默认模型']))});if(!alive())return;
          await observe();
        }catch(e){
          if(!alive())return;
          const rejected=[400,401,403,404,409,422,429].includes(e.status);
          if(!sent||rejected){stopTimer();generation++;feedEpoch=generation;ref=null;busy=false;status='';interrupted=false;await save(null);emit(e.message||'任务未启动，请修改后重试',{error:true});}
          else{busy=true;emit('启动结果待确认，正在查找已创建任务',{error:true,uncertain:true});await reconcile();}
        }finally{if(pending===submission)pending=null;}
      },
      async cancel(){
        if(!ref?.id){emit('请先查找或确认启动中的任务',{error:true});return;}
        if(!busy&&!interrupted){emit('任务已结束，可到工作台继续或重试');return;}
        emit('正在请求停止');
        try{await deps.request('/teams/'+encodeURIComponent(ref.id)+'/steer',{method:'POST',json:{command:'cancel'}});await observe();}
        catch(e){emit('停止未确认，请查看工作台',{error:true});}
      },
      async forget(){stopTimer();generation++;ref=null;busy=false;pending=null;status='';interrupted=false;await save(null);emit('已解除观察绑定，可以重新投喂');},
      reconcile,refresh:observe,
      open(){return deps.open(ref?.id?ref:null);},
      restore(value){stopTimer();generation++;ref=value;busy=!!value;status='';interrupted=false;if(value)observe();else emit('等待任务');},
      dispose(){disposed=true;generation++;stopTimer();},
      get ref(){return ref;},get busy(){return busy;},get status(){return status;},get interrupted(){return interrupted;}
    };
  }
  const exported={validate,label,buildTeamRequest,createController};
  if(typeof module !== 'undefined' && module.exports) {module.exports=exported;return;}
  g.OwoFeed=exported;
  const pet=document.getElementById('pet'), bubble=document.getElementById('bubble');
  const food=document.getElementById('feed-food');
  const params=new URLSearchParams(g.location.search);
  const coordinator=params.get('member')!=='1';
  let role=params.get('role') || 'planner';
  const names={planner:'指挥员',lead:'指挥员',implementer:'执行者',reviewer:'验收员'};
  let currentText='', currentState='idle', visualTimer=null;
  function update(text,data) {
    if(coordinator&&data.ref?.members?.length){const agents=data.ref.members.filter(m=>m.runtime_binding?.kind==='agent');role=agents.find(m=>['lead','planner'].includes(m.role))?.role || agents[0]?.role || 'planner';}
    const tasks=data.snapshot?.tasks || [];
    if(coordinator&&data.snapshot&&data.ref?.id){
      const activeRoles=[...new Set(tasks.filter(t=>norm(t.status)==='running').map(t=>t.role||String(t.worker||'').replace(/^m-/,'')))];
      g.petShell?.presentFeed?.({id:data.ref.id,status:norm(data.snapshot.team?.status),activeRoles}).catch(()=>{});
    }
    const ownTasks=tasks.filter(t=>(t.role || String(t.worker||'').replace(/^m-/,''))===role);
    const task=ownTasks.find(t=>['running','claimed'].includes(norm(t.status))) || ownTasks.at(-1);
    const member=data.ref?.members?.find(m=>m.role===role);
    const reviewDuty=member?.capabilities?.includes('review')||/reviewer|critic|judge/.test(role);
    const status=norm(task?.status);
    const teamStatus=norm(data.snapshot?.team?.status);
    currentState=data.error || data.snapshot?.interrupted || teamStatus==='awaitinghuman' ? 'alert' : status==='running' || status==='claimed' ? 'thinking' : 'idle';
    if (coordinator) currentText=text;
    else if(data.error || teamStatus==='failed' || teamStatus==='cancelled' || teamStatus==='succeeded') currentText=text;
    else currentText=status==='running' ? (reviewDuty ? '我来检查成果' : '正在制作，交给我吧') : status==='succeeded' ? '我的任务完成了' : '等队友交接给我';
    currentText=(names[role] || ({lead:'规划师',w1:'执行者'}[role]) || (/^w[1-9][0-9]*$/.test(role)?'执行员 '+role.slice(1):role))+' · '+currentText;
    if(data.ref?.model||data.ref?.roleModels?.[role])pet.title='当前角色模型：'+(data.ref.roleModels?.[role]||data.ref.model)+'；右键查看任务与操作';
    pet.classList.toggle('feed-working',currentState==='thinking');
    pet.classList.toggle('feed-complete',teamStatus==='succeeded');
    bubble.textContent=currentText;
    g.OwoPetActor?.render(currentState);
  }
  const request=(path,options)=>g.OwoApi.request(path,{...options,signal:AbortSignal.timeout(20000)});
  const controller=createController({request,
    workspace:()=>g.owo?.getWorkspace(),settings:()=>request('/settings'),update,
    eat(text) {clearTimeout(visualTimer);food.textContent=text.slice(0,16);pet.classList.remove('feed-eating');void pet.offsetWidth;pet.classList.add('feed-eating');return new Promise(resolve=>{visualTimer=setTimeout(()=>{pet.classList.remove('feed-eating');resolve();},950);});},
    confirm:input=>g.petShell?.confirmTask ? g.petShell.confirmTask(input) : Promise.resolve(g.confirm('确认将此任务交给队伍执行？\n项目：'+input.workspace+'\n\n'+input.text)?{text:input.text,workspace:input.workspace}:null),
    schedule:(fn,ms)=>setTimeout(fn,ms),clear:id=>clearTimeout(id),
    save:ref=>coordinator?g.petShell?.saveFeed(ref):Promise.resolve(),open:ref=>g.petShell?.openFeed(ref)
  });
  g.OwoFeedUI={controller,coordinator,text:()=>currentText,state:()=>currentText ? currentState : null,role,
    paste:async()=>receive(await g.petShell.readFeedText())};
  async function receive(text) {
    text=validate(text);
    if(!coordinator) {
      food.textContent=String(text).slice(0,16);pet.classList.add('feed-eating');setTimeout(()=>pet.classList.remove('feed-eating'),950);
      return g.petShell.forwardFeed(text);
    }
    return controller.feed(text);
  }
  let depth=0;
  document.addEventListener('dragenter',e=>{e.preventDefault();depth++;pet.classList.add('feed-hover');});
  document.addEventListener('dragover',e=>{e.preventDefault(); if(e.dataTransfer)e.dataTransfer.dropEffect='copy';});
  document.addEventListener('dragleave',()=>{if(--depth<=0){depth=0;pet.classList.remove('feed-hover');}});
  document.addEventListener('drop',e=>{e.preventDefault();depth=0;pet.classList.remove('feed-hover');
    if(e.dataTransfer?.files?.length) {update('请拖入选中文字，暂不接收文件',{error:true});return;}
    receive(e.dataTransfer?.getData('text/plain') || '').catch(e=>update(e.message,{error:true}));
  });
  const unsubscribe=g.petShell?.onFeed(text=>receive(text).catch(e=>update(e.message,{error:true})));
  const unobserve=g.petShell?.onFeedRun(ref=>controller.restore(ref));
  g.petShell?.loadFeed().then(ref=>controller.restore(ref)).catch(()=>{});
  g.addEventListener('beforeunload',()=>{controller.dispose();clearTimeout(visualTimer);unsubscribe?.();unobserve?.();});
})(globalThis);
