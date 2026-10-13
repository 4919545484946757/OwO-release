from pathlib import Path
import json,os,subprocess,time,urllib.request,urllib.error
root=Path(r'D:\src\OwO');base=Path('D:\\src\\OwO\\env\\team-audit-20261013-isolated-4bb2892d');ws=base/'workspace';data=base/'data'
env=os.environ.copy()
for k in list(env):
 if k.startswith(('OWO_','OPENAI_','ANTHROPIC_','DASHSCOPE_','MCP_')):env.pop(k,None)
env.update({'OWO_AGENT_DATA':str(data),'OWO_CLOUD_ENABLED':'false','OPENAI_BASE_URL':'http://127.0.0.1:1/v1','TEMP':str(base/'tmp'),'TMP':str(base/'tmp'),'LOCALAPPDATA':str(base/'localappdata'),'APPDATA':str(base/'appdata')})
exe=root/'agent-sdk/target/debug/owo-agent.exe';p=None;n=0;port=None;token=None;results=[]
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
def start():
 global p,n,port,token
 n+=1;out=base/f'server-{n}.out.log';err=base/f'server-{n}.err.log'
 with out.open('xb') as o,err.open('xb') as e:p=subprocess.Popen([str(exe),'serve','--port','0','--workspace',str(ws)],cwd=ws,env=env,stdout=o,stderr=e,creationflags=subprocess.CREATE_NO_WINDOW)
 end=time.time()+25
 while time.time()<end:
  for line in out.read_text(encoding='utf-8',errors='replace').splitlines():
   try:x=json.loads(line)
   except ValueError:continue
   if x.get('event')=='core_ready':port=x['port'];token=(data/'auth/token').read_text().strip();return
  if p.poll() is not None:raise RuntimeError('server exit '+str(p.returncode))
  time.sleep(.1)
 raise RuntimeError('ready timeout')
def call(method,path,body=None):
 req=urllib.request.Request(f'http://127.0.0.1:{port}'+path,data=json.dumps(body).encode() if body is not None else None,method=method,headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
 try:
  with opener.open(req,timeout=8) as r:return r.status,json.load(r)
 except urllib.error.HTTPError as e:
  raw=e.read().decode(errors='replace')
  try:return e.code,json.loads(raw)
  except ValueError:return e.code,{'error':raw}
def stop():
 global p
 if p is None or p.poll() is not None:return
 try:call('POST','/server/shutdown',{'confirm':True});p.wait(timeout=5)
 except Exception:p.terminate();p.wait(timeout=5)
 p=None
def emit(case,**fields):
 x={'case':case,**fields};results.append(x);print(json.dumps(x,ensure_ascii=True),flush=True)
def role(name,kind='echo',deps=(),ms=None):
 r={'role':name,'assignee':'human' if kind=='human' else 'agent','worker':'tester' if kind=='human' else kind,'depends_on':list(deps),'verify':'non_empty'}
 if ms is not None:r['extra_input']={'ms':ms}
 return r
def create(roles):
 s,x=call('POST','/teams',{'objective':'isolated collaboration contract check','mode':'team','strategy':'team','roles':roles,'budget':{'max_steps':30}})
 if s!=202:raise RuntimeError('create '+str(s)+' '+str(x))
 return x['team_id']
def detail(tid):
 s,x=call('GET','/teams/'+tid)
 if s!=200:raise RuntimeError('detail '+str(s)+' '+str(x))
 return x
def poll(tid,wanted,seconds=8):
 end=time.time()+seconds
 while time.time()<end:
  x=detail(tid)
  if x['team']['status'].lower() in wanted:return x
  time.sleep(.1)
 return x
try:
 start();emit('health',health=call('GET','/health')[1])
 t=create([role('writer'),role('summarizer',deps=['writer'])]);d=poll(t,{'succeeded','failed'});emit('normal_echo_relay',status=d['team']['status'],tasks=d['tasks'])
 t=create([role('planner','sleep',ms=1800),role('human','human',deps=['planner']),role('summarizer',deps=['human'])]);time.sleep(.25)
 s,x=call('POST','/tasks/s-human/human-result',{'team_id':t,'result':'premature human result'});d=detail(t);emit('human_before_dependency',http=s,upstream=next(a for a in d['tasks'] if a['task_id']=='s-planner')['status'],human=next(a for a in d['tasks'] if a['task_id']=='s-human')['status']);poll(t,{'succeeded','failed'})
 t=create([role('writer'),role('human','human',deps=['writer']),role('summarizer',deps=['human'])]);poll(t,{'awaiting_human'})
 s,x=call('POST','/teams/'+t+'/steer',{'command':'replace','role':'human','new_worker':'echo','note':'isolated human to worker'});d=poll(t,{'succeeded','failed'},4);emit('replace_human_echo',http=s,status=d['team']['status'],tasks=d['tasks'])
 t=create([role('writer'),role('human','human',deps=['writer']),role('summarizer',deps=['human'])]);poll(t,{'awaiting_human'})
 stop();start();d=detail(t);emit('restart_human_state',status=d['team']['status'],interrupted=d.get('interrupted'))
 s,x=call('POST','/teams/'+t+'/steer',{'command':'continue'});emit('restart_human_continue',http=s,reply=x)
 s,x=call('POST','/tasks/s-human/human-result',{'team_id':t,'result':'human after restart'});time.sleep(1);d=detail(t);emit('restart_human_submit',http=s,status=d['team']['status'],tasks=d['tasks']);call('POST','/teams/'+t+'/steer',{'command':'cancel'})
 t=create([role('writer','sleep',ms=4000)]);time.sleep(.2);call('POST','/teams/'+t+'/steer',{'command':'cancel'});d=poll(t,{'cancelled'});emit('cancel_sleep_worker',status=d['team']['status'],tasks=d['tasks'])
 t=create([role('human','human'),role('writer',deps=['human'])]);poll(t,{'awaiting_human'});s,x=call('POST','/tasks/s-human/human-result',{'team_id':t,'result':'normal handoff'});d=poll(t,{'succeeded','failed'});emit('normal_human_handoff',http=s,status=d['team']['status'])
except Exception as e:emit('harness_error',error=str(e))
finally:
 stop();(base/'runtime-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')