from pathlib import Path
import json
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d')
source=(base/'runtime-audit.py').read_text(encoding='utf-8')
exec(source.split('try:\n start();')[0])
n=2
try:
 start()
 a=create([role('researcher')]);poll(a,{'succeeded','failed'})
 b=create([role('researcher')]);poll(b,{'succeeded','failed'})
 arts=call('GET','/projects/proj-'+a+'/artifacts')[1]['artifacts'];art=arts[0];aid=art['artifact_id'];owner=art['producer']
 for case,team,reviewer,key in [('review_self_baseline',a,owner,'baseline'),('review_self_whitespace',a,' '+owner+' ','spaces'),('review_cross_team',b,'user','cross-team')]:
  s,x=call('POST','/artifacts/'+aid+'/review',{'team_id':team,'decision':'approve','reviewer':reviewer,'expected_version':art['version'],'idempotency_key':key})
  emit(case,http=s,review_team=x.get('review',{}).get('team_id'),stored_reviewer=x.get('review',{}).get('reviewer'),error=x.get('error'))
 # Unknown executor accepted by replace, then registry cannot initialize.
 t=create([role('human','human'),role('researcher',deps=['human'])]);poll(t,{'awaiting_human'})
 s,x=call('POST','/teams/'+t+'/steer',{'command':'replace','role':'human','new_worker':'does-not-exist'})
 emit('replace_unknown_worker',http=s,reply=x)
 call('POST','/teams/'+t+'/steer',{'command':'cancel'})
except Exception as e:emit('harness_error',error=str(e))
finally:
 stop();(base/'review-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')