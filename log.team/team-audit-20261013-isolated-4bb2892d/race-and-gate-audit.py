from pathlib import Path
import json,threading,time,hashlib
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=5
try:
 start()
 t='team-cc598140';s,g=call('GET','/teams/'+t+'/change-sets');s,h=call('GET','/artifacts/'+t+':code:v3/history');emit('pending_code_approved_head',approval_blocked=g.get('approval_blocked'),approved_head=h.get('approved_head',{}).get('artifact_id'))
 t=create([role('researcher')]);poll(t,{'succeeded','failed'})
 names=['race-files/f'+str(i).zfill(4)+'.txt' for i in range(1200)];before=b'baseline synthetic data';after=b'candidate synthetic data';bh=hashlib.sha256(before).hexdigest();rh=hashlib.sha256(after).hexdigest();records=[]
 for trial,delay in enumerate([.01,.03,.07]):
  for rel in names:(ws/rel).write_bytes(after)
  cs={'change_set_id':'cs-race-'+t+'-'+str(trial),'team_id':t,'step_id':'s-researcher','role':'researcher','base_hashes':[{'path':v,'sha256':bh,'content_available':True} for v in names],'result_hashes':[{'path':v,'sha256':rh,'content_available':False} for v in names],'changed_files':names,'status':'pending_review','created_at':'2026-10-13T00:00:00Z','conflicts':[]};records.append(cs)
  file=data/'workswarm/runs'/(t+'-change-sets.json');file.write_text(json.dumps(records),encoding='utf-8')
  rejected=[];thread=threading.Thread(target=lambda:rejected.append(call('POST','/change-sets/'+cs['change_set_id']+'/reject',{'idempotency_key':'reject'})));thread.start();time.sleep(delay)
  s,x=call('POST','/change-sets/'+cs['change_set_id']+'/accept',{'idempotency_key':'accept'});thread.join(timeout=10);status,stored=call('GET','/change-sets/'+cs['change_set_id']);count=sum((ws/v).read_bytes()==before for v in names)
  emit('changeset_race_'+str(trial),accept_http=s,reject_http=rejected[0][0] if rejected else None,stored_status=stored.get('change_set',{}).get('status'),baseline_file_count=count,total_files=len(names));records=json.loads(file.read_text())
  if s==200 and count>0:break
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'race-and-gate-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')