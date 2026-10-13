from pathlib import Path
import json,urllib.parse
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=10
try:
 start()
 graph={'subtasks':[{'task_id':'a','worker':'w1','task':'produce A','acceptance':'nonempty A','write_paths':[]},{'task_id':'b','worker':'w1','task':'produce B','acceptance':'nonempty B','write_paths':[],'depends_on':['a']}]}
 lead=role('lead');lead['extra_input']={'text':json.dumps(graph)}
 s,c=call('POST','/teams',{'objective':'produce A and B','mode':'team','strategy':'team','parallel':True,'roles':[lead,role('w1',deps=['lead']),role('leader',deps=['w1'])],'budget':{'max_steps':30}})
 emit('dynamic_graph_create',http=s)
 if s==202:
  t=c['team_id'];d=poll(t,{'succeeded','failed'},12);emit('dynamic_graph_run',status=d['team']['status'],tasks=d['tasks'])
  arts=call('GET','/projects/proj-'+t+'/artifacts')[1]['artifacts'];owned=[a for a in arts if a['producer']=='m-w1'];emit('dynamic_worker_artifacts',artifacts=[{'id':a['artifact_id'],'version':a['version']} for a in owned])
  if len(owned)>=2:
   art=max(owned,key=lambda a:a['version']);s,m=call('GET','/artifacts/'+art['artifact_id']+'/metadata');expected_task=m.get('task_id')
   s,x=call('POST','/artifacts/'+art['artifact_id']+'/review',{'team_id':t,'decision':'request_changes','reviewer':'user','expected_version':art['version'],'idempotency_key':'dynamic-review-b','comment':'correct B only'})
   review_id=x.get('review',{}).get('review_id');emit('dynamic_review_binding',http=s,review_step=x.get('review',{}).get('step_id'),artifact_metadata_task=expected_task)
   if review_id:
    s,x=call('POST','/artifacts/'+art['artifact_id']+'/rework',{'team_id':t,'review_id':review_id,'instruction':'correct B only','idempotency_key':'dynamic-rework-b'});emit('dynamic_rework_target',http=s,target_step=x.get('rework',{}).get('step_id'),reply=x if s>=400 else None)
    call('POST','/teams/'+t+'/steer',{'command':'cancel'})
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'dynamic-rework-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')