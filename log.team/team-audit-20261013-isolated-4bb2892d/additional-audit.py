from pathlib import Path
import json,hashlib,threading,time,urllib.parse
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d')
exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=3
try:
 start()
 t=create([role('human','human'),role('researcher',deps=['human'])]);poll(t,{'awaiting_human'})
 s,x=call('GET','/human/inbox?team_id='+t)
 items=x.get('items',[]);item=next(a for a in items if a['kind']=='human_result');iid=urllib.parse.quote(item['item_id'],safe='')
 s,x=call('POST','/human/inbox/'+iid+'/claim',{'user':'alice'});emit('inbox_claim_alice',http=s)
 s,x=call('POST','/human/inbox/'+iid+'/resolve',{'user':'bob','result':'submitted by non-owner','resolution_id':'bob-result'});emit('inbox_resolve_bob',http=s,resolved=x.get('resolved'),error=x.get('error'));poll(t,{'succeeded','failed'})
 t=create([role('code')]);d=poll(t,{'succeeded','failed'});arts=call('GET','/projects/proj-'+t+'/artifacts')[1]['artifacts'];art=arts[0]
 run_dir=data/'workswarm/runs'
 cs={'change_set_id':'cs-pending-'+t,'team_id':t,'step_id':'s-code','role':'code','attempt_id':d['tasks'][0].get('attempt_id'),'base_hashes':[],'result_hashes':[],'changed_files':[],'status':'pending_review','created_at':'2026-10-13T00:00:00Z','conflicts':[]}
 (run_dir/(t+'-change-sets.json')).write_text(json.dumps([cs]),encoding='utf-8')
 status,gate=call('GET','/teams/'+t+'/change-sets')
 s,x=call('POST','/artifacts/'+art['artifact_id']+'/review',{'team_id':t,'decision':'approve','reviewer':'user','expected_version':art['version'],'idempotency_key':'approve-pending'})
 emit('direct_review_pending_changeset',approval_blocked=gate.get('approval_blocked'),http=s,artifact_kind=art['kind'],head=x.get('approved_head',{}).get('artifact_id'))
 # Restore/accept race: all synthetic files, hashes and state belong to this audit.
 t=create([role('researcher')]);poll(t,{'succeeded','failed'})
 fixture=ws/'race-files';fixture.mkdir();before=b'baseline synthetic data';after=b'candidate synthetic data';bh=hashlib.sha256(before).hexdigest();rh=hashlib.sha256(after).hexdigest();(data/'workswarm/cas'/bh).write_bytes(before)
 names=[]
 for i in range(1200):
  rel='race-files/f'+str(i).zfill(4)+'.txt';(ws/rel).write_bytes(after);names.append(rel)
 cs={'change_set_id':'cs-race-'+t,'team_id':t,'step_id':'s-researcher','role':'researcher','base_hashes':[{'path':v,'sha256':bh,'content_available':True} for v in names],'result_hashes':[{'path':v,'sha256':rh,'content_available':False} for v in names],'changed_files':names,'status':'pending_review','created_at':'2026-10-13T00:00:00Z','conflicts':[]}
 (run_dir/(t+'-change-sets.json')).write_text(json.dumps([cs]),encoding='utf-8')
 rejected=[]
 thread=threading.Thread(target=lambda:rejected.append(call('POST','/change-sets/'+cs['change_set_id']+'/reject',{'idempotency_key':'reject-race'})));thread.start();deadline=time.time()+5
 while time.time()<deadline and (ws/names[0]).read_bytes()!=before and thread.is_alive():time.sleep(.001)
 s,x=call('POST','/change-sets/'+cs['change_set_id']+'/accept',{'idempotency_key':'accept-race'});thread.join(timeout=10)
 status,stored=call('GET','/change-sets/'+cs['change_set_id']);emit('changeset_accept_reject_race',accept_http=s,reject_http=rejected[0][0] if rejected else None,stored_status=stored.get('change_set',{}).get('status'),baseline_file_count=sum((ws/v).read_bytes()==before for v in names),total_files=len(names))
except Exception as e:emit('harness_error',error=str(e))
finally:
 stop();(base/'additional-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')