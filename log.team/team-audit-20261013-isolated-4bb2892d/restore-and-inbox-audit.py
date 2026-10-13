from pathlib import Path
import json,hashlib,subprocess,urllib.parse
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=6
try:
 start();t=create([role('researcher')]);poll(t,{'succeeded','failed'})
 other=base/('outside-bound-workspace-'+t);other.mkdir();victim=other/'victim.txt';after=b'candidate synthetic data';before=b'baseline synthetic data';victim.write_bytes(after)
 link=ws/('junction-'+t)
 subprocess.run(['node','-e',"require('node:fs').symlinkSync(process.argv[1],process.argv[2],'junction')",str(other),str(link)],check=True,capture_output=True)
 rel=link.name+'/victim.txt';bh=hashlib.sha256(before).hexdigest();rh=hashlib.sha256(after).hexdigest();(data/'workswarm/cas'/bh).write_bytes(before)
 cs={'change_set_id':'cs-junction-'+t,'team_id':t,'step_id':'s-researcher','role':'researcher','base_hashes':[{'path':rel,'sha256':bh,'content_available':True}],'result_hashes':[{'path':rel,'sha256':rh,'content_available':False}],'changed_files':[rel],'status':'pending_review','created_at':'2026-10-13T00:00:00Z','conflicts':[]}
 (data/'workswarm/runs'/(t+'-change-sets.json')).write_text(json.dumps([cs]),encoding='utf-8')
 s,x=call('POST','/change-sets/'+cs['change_set_id']+'/reject',{'idempotency_key':'junction-reject'});emit('restore_junction_outside_workspace',http=s,outside_workspace_file_changed=victim.read_bytes()==before,outside_path_still_within_audit_env=True)
 t=create([role('human','human'),role('researcher',deps=['human'])]);poll(t,{'awaiting_human'});s,x=call('GET','/human/inbox?team_id='+t);item=next(a for a in x.get('items',[]) if a['kind']=='human_result');item_id=item['item_id'];stop()
 file=data/'workswarm/human-inbox.json';snapshot=json.loads(file.read_text());record=snapshot['items'][item_id];record['status']='resolving';record['idempotency_key']='interrupted-resolve';file.write_text(json.dumps(snapshot),encoding='utf-8')
 start();s,x=call('POST','/human/inbox/'+urllib.parse.quote(item_id,safe='')+'/resolve',{'user':'alice','result':'after restart','resolution_id':'interrupted-resolve'});emit('inbox_crashed_resolve_restart',http=s,error=x.get('error'));call('POST','/teams/'+t+'/steer',{'command':'cancel'})
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'restore-and-inbox-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')