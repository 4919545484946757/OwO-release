from pathlib import Path
import json,urllib.parse
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=7
try:
 start();t=create([role('human','human'),role('researcher',deps=['human'])]);poll(t,{'awaiting_human'});s,x=call('GET','/human/inbox?team_id='+t);item=next(a for a in x.get('items',[]) if a['kind']=='human_result');item_id=item['item_id'];stop()
 file=data/'workswarm/human-inbox.json';snapshot=json.loads(file.read_text(encoding='utf-8'));record=snapshot['items'][item_id];record['status']='resolving';record['idempotency_key']='interrupted-resolve';file.write_text(json.dumps(snapshot),encoding='utf-8')
 start();s,x=call('POST','/human/inbox/'+urllib.parse.quote(item_id,safe='')+'/resolve',{'user':'alice','result':'after restart','resolution_id':'interrupted-resolve'});emit('inbox_crashed_resolve_restart',http=s,error=x.get('error'));call('POST','/teams/'+t+'/steer',{'command':'cancel'})
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'inbox-crash-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')