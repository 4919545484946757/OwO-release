from pathlib import Path
import json,subprocess
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=9
try:
 start();t=create([role('researcher')]);d=poll(t,{'succeeded','failed'});arts=call('GET','/projects/proj-'+t+'/artifacts')[1]['artifacts'];aid=arts[0]['artifact_id'];s,x=call('GET','/artifacts/'+aid+'/content');emit('artifact_content',http=s,hash_valid=__import__('hashlib').sha256(x['content'].encode()).hexdigest()==x['sha256'])
 for endpoint in ['/teams/'+t+'/events?format=json','/teams/'+t+'/metrics','/teams/'+t+'/diagnostic','/projects/proj-'+t+'/delivery-manifest']:
  s,x=call('GET',endpoint);emit('read_surface',endpoint=endpoint,http=s)
 s,c=call('GET','/teams/'+t+'/context');rev=c.get('revision',c.get('context',{}).get('revision',0));draft={'expected_revision':rev,'key':'local-check','value':'synthetic bounded fact','producer':'user','source_refs':[]}
 s,x=call('POST','/teams/'+t+'/context',draft);emit('context_publish',http=s,status=x.get('fact',{}).get('status'));s,x=call('POST','/teams/'+t+'/context',draft);emit('context_stale_revision',http=s)
 s,x=call('GET','/teams/templates/proposals');proposals=x.get('proposals',[])
 if proposals:
  q=next((v for v in proposals if v.get('status')=='proposed'),proposals[0]);s,x=call('POST','/teams/templates/proposals/'+q['proposal_id']+'/adopt',{});emit('template_adopt',http=s)
 skipws=base/'auto-reviewer-workspace';skipws.mkdir();subprocess.run(['git','-c','init.templateDir=','init','-q',str(skipws)],check=True,capture_output=True)
 reviewer=role('independent_reviewer');reviewer['depends_on']=['builder'];reviewer['extra_input']={'_workswarm_auto_independent_reviewer':True}
 s,c=call('POST','/teams',{'objective':'no-change reviewer contract check','mode':'team','strategy':'team','roles':[role('builder'),reviewer],'budget':{'max_steps':20},'workspace':{'root':str(skipws),'read_only':True}})
 if s==202:
  tid=c['team_id'];d=poll(tid,{'succeeded','failed'});state=json.loads((data/'workswarm/runs'/(tid+'.json')).read_text(encoding='utf-8'));emit('auto_reviewer_skip_gate',status=d['team']['status'],error=state['goal'].get('error'),reviewer_status=next(a for a in d['tasks'] if a['task_id']=='s-independent_reviewer')['status'])
 else:emit('auto_reviewer_create',http=s,reply=c)
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'feature-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')