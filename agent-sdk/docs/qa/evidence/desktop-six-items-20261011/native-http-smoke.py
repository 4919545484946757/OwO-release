import pathlib,os,subprocess,threading,json,time,uuid,hashlib,urllib.request,urllib.error,http.server,queue,sys
ROOT=pathlib.Path(__file__).resolve().parents[4]
OUT=pathlib.Path(__file__).resolve().parent
WS=OUT/'fixture-workspace';WS.mkdir(exist_ok=True)
OPEN=urllib.request.build_opener(urllib.request.ProxyHandler({}))
results=[];calls=[];entered=threading.Event()
class Provider(http.server.BaseHTTPRequestHandler):
 def log_message(self,*_):pass
 def do_GET(self):
  data=json.dumps({'data':[{'id':'native-fixture'}]}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def do_POST(self):
  data=json.loads(self.rfile.read(int(self.headers.get('Content-Length','0'))));calls.append({'model':data.get('model'),'authorized':bool(self.headers.get('Authorization'))})
  text='本地脚本模型完成；用于接口与桌面验收。'
  wait=any('wait-for-cancel' in str(m.get('content','')) for m in data.get('messages',[]) if m.get('role')=='user')
  if wait:entered.set();time.sleep(3)
  try:
   if data.get('stream'):
    frames=[{'id':'fixture-request','model':data.get('model'),'choices':[{'delta':{'content':text},'finish_reason':None}]},{'id':'fixture-request','model':data.get('model'),'choices':[{'delta':{},'finish_reason':'stop'}],'usage':{'prompt_tokens':3,'completion_tokens':2,'total_tokens':5}}]
    body=(''.join('data: '+json.dumps(f,ensure_ascii=False)+'\n\n' for f in frames)+'data: [DONE]\n\n').encode()
    self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
   else:
    body=json.dumps({'model':data.get('model'),'choices':[{'message':{'role':'assistant','content':text},'finish_reason':'stop'}],'usage':{'prompt_tokens':3,'completion_tokens':2,'total_tokens':5}},ensure_ascii=False).encode()
    self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  except (BrokenPipeError,ConnectionResetError,ConnectionAbortedError):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Provider)
threading.Thread(target=server.serve_forever,daemon=True).start()
endpoint='http://127.0.0.1:'+str(server.server_port)+'/v1'
def start():
 env=os.environ.copy();env.update(OWO_AGENT_DATA=str(OUT/'http-fixture-data'),OWO_CLOUD_ENABLED='false',OPENAI_BASE_URL=endpoint,OPENAI_MODEL='native-fixture',OWO_DESKTOP_PAIRING_SECRET=uuid.uuid4().hex,OWO_DESKTOP_INSTANCE_ID=uuid.uuid4().hex,OWO_WEB_UI_DIR=str(ROOT/'desktop/web'))
 env.pop('OPENAI_API_KEY',None);env.pop('ANTHROPIC_API_KEY',None)
 proc=subprocess.Popen([str(ROOT/'target/debug/owo-agent.exe'),'serve','--port','0','--workspace',str(WS)],cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
 q=queue.Queue()
 def lines():
  for raw in proc.stdout:
   line=raw.decode('utf-8',errors='replace')
   try:
    value=json.loads(line)
    if value.get('event')=='core_ready':q.put(value)
   except ValueError:pass
 threading.Thread(target=lines,daemon=True).start()
 try:ready=q.get(timeout=20)
 except queue.Empty:proc.terminate();raise RuntimeError('Core ready deadline')
 headers={'x-owo-desktop-pairing':env['OWO_DESKTOP_PAIRING_SECRET'],'x-owo-desktop-instance':env['OWO_DESKTOP_INSTANCE_ID'],'x-owo-client':'shell'}
 base='http://127.0.0.1:'+str(ready['port'])
 def request(path,data=None,raw=False,headers_override=None):
  h=dict(headers if headers_override is None else headers_override)
  if data is not None:h['Content-Type']='application/json'
  req=urllib.request.Request(base+path,data=json.dumps(data).encode() if data is not None else None,headers=h)
  try:
   with OPEN.open(req,timeout=25) as response:body=response.read()
   return body.decode('utf-8') if raw else json.loads(body)
  except urllib.error.HTTPError as e:return {'http_error':e.code,'error_body':e.read().decode('utf-8',errors='replace')}
 token=request('/auth/token')['token'];headers['Authorization']='Bearer '+token
 return proc,request,ready
def stop(proc,request):
 try:request('/server/shutdown',{})
 except Exception:pass
 try:proc.wait(timeout=10)
 except subprocess.TimeoutExpired:proc.terminate();proc.wait(timeout=5)
def note(name,value=True):
 assert value,name
 results.append({'check':name,'passed':True})
def events(text):
 if not isinstance(text,str):raise RuntimeError('turn transport rejected: '+str(text))
 out=[]
 for line in text.splitlines():
  if line.startswith('data: '):
   try:out.append(json.loads(line[6:]))
   except ValueError:pass
 return out
proc=None
try:
 proc,request,identity=start()
 caps=request('/capabilities');note('secure reference capability',caps['constraints']['custom_model_env_credentials'])
 spec=request('/openapi.json');snapshot=json.loads((ROOT/'clients/ts/openapi.json').read_text(encoding='utf-8'));(OUT/'runtime-openapi.json').write_text(json.dumps(spec,ensure_ascii=False,sort_keys=True,separators=(',',':')),encoding='utf-8')
 note('runtime OpenAPI equals checked snapshot',spec==snapshot)
 sid=request('/session',{'workspace':str(WS)})['id']
 ref=uuid.uuid4().hex;fake=uuid.uuid4().hex
 body={'credential_ref':ref,'model':'native-fixture','base_url':endpoint,'api_format':'openai','api_key':fake}
 bare=request('/desktop/model-credentials',body,headers_override={'Authorization':request.__name__}) # expected rejected without valid auth
 note('unpaired registration is rejected',bare.get('http_error') in (401,403))
 note('paired registration succeeds',request('/desktop/model-credentials',body).get('configured') is True)
 conn={'model':'native-fixture','base_url':endpoint,'api_format':'openai','api_key_env':'OWO_CUSTOM_MODEL_SECRET_'+ref}
 terminal=events(request('/session/'+sid+'/turn',{'prompt':'complete local fixture','turn_id':str(uuid.uuid4()),'model_connection':conn},raw=True))
 stats=next(e for e in reversed(terminal)if e.get('type')=='turn_stats')
 note('turn telemetry names actual model',stats['model_calls'][-1]['model']=='native-fixture')
 detail=request('/session/'+sid);note('history restores exact provider telemetry',detail['turn_history']['last_stats']['payload']['model_calls']==stats['model_calls'])
 note('usage is measured by scripted fixture',stats['model_calls'][-1]['usage']['total_tokens']==5)
 (OUT/'http-session-id.json').write_text(json.dumps({'session_id':sid}),encoding='utf-8')
 stop(proc,request);proc=None
 proc,request,new_identity=start();detail=request('/session/'+sid)
 note('daemon restart preserves completed facts',all(detail['turn_history']['last_stats']['payload'].get(k)==stats.get(k) for k in ['steps','duration_ms','model_calls','completion_status','completion_record','total_tokens']))
 note('new ephemeral port',identity['port']!=new_identity['port'])
 note('credential can be registered again by its desktop owner',request('/desktop/model-credentials',body).get('configured')is True)
 tid=str(uuid.uuid4());stream=[]
 thread=threading.Thread(target=lambda:stream.append(request('/session/'+sid+'/turn',{'prompt':'wait-for-cancel','turn_id':tid,'model_connection':conn},raw=True)))
 thread.start();note('model request began',entered.wait(timeout=8))
 cancel=request('/session/'+sid+'/abort',{'turn_id':tid});note('scoped cancellation accepted',cancel.get('state') in ('cancellation_requested','cancellation_queued'))
 thread.join(timeout=12);note('cancelled stream closes',not thread.is_alive())
 tail=events(stream[0]);note('cancelled host terminal persisted',any(e.get('completion_status')=='aborted' for e in tail))
 detail=request('/session/'+sid);note('history reports the cancelled turn',detail['turn_history']['last_turn_id']==tid and detail['turn_history']['last_terminal']['payload']['completion_status']=='aborted')
 note('canonical diff endpoint remains available',isinstance(request('/session/'+sid+'/diff'),list))
 body.pop('api_key');request('/desktop/model-credentials',body)
 for _ in range(30):
  if not request('/session/'+sid).get('active_turn_id'):break
  time.sleep(.1)
 before_calls=len(calls)
 reply=request('/session/'+sid+'/turn',{'prompt':'revoked credential','turn_id':str(uuid.uuid4()),'model_connection':conn},raw=True)
 reason=reply if isinstance(reply,str) else reply.get('error_body','')
 note('revoked credential never falls back to a default provider',('not_configured' in reason or 'unavailable' in reason) and len(calls)==before_calls)
 note('synthetic credential absent from persisted facts',fake not in json.dumps(detail))
 note('model calls stayed local',all(item['model']=='native-fixture' for item in calls))
 results.append({'identity':new_identity,'source':'current debug CLI; current frontend files','model':'local scripted provider; no cloud'})
 (OUT/'native-http-smoke.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')
 print('HTTP_SMOKE PASS',sum(r.get('passed',False)for r in results),'checks')
finally:
 if proc:stop(proc,request)
 server.shutdown()
