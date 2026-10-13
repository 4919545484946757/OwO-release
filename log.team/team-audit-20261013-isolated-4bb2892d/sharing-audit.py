from pathlib import Path
import json,io,zipfile,base64,hashlib
base=Path(r'D:\src\OwO\env\team-audit-20261013-isolated-4bb2892d');exec((base/'runtime-audit.py').read_text(encoding='utf-8').split('try:\n start();')[0]);n=11

def package(ident,name,body,version='1.0.0',permission='ui:operate'):
 manifest={'id':ident,'name':name,'version':version,'min_app_version':'0.1.0','target_apps':['notepad'],'permissions':[permission],'variables':[],'sensitivity':'low'}
 graph={'version':1,'start':'step','nodes':[{'id':'step','action_type':'click','anchor':{'name':'audit-button'}}],'edges':[]}
 memory=io.BytesIO()
 with zipfile.ZipFile(memory,'w',compression=zipfile.ZIP_DEFLATED) as z:
  z.writestr('SKILL.md','---\nname: '+name+'\n---\n'+body);z.writestr('manifest.json',json.dumps(manifest));z.writestr('graph.json',json.dumps(graph));z.writestr('versions.json','{}')
 return base64.b64encode(memory.getvalue()).decode()
try:
 start()
 for case,ident,name,body,perm in [('share_clean','audit-shared-flow','audit_shared_flow','plain procedure','ui:operate'),('share_unknown_permission','audit-unknown','audit_unknown','plain procedure','not-a-permission'),('share_uppercase_danger','audit-danger','audit_danger',"subprocess.Popen(['synthetic-example'])",'ui:operate'),('share_id_escape','../escaped-audit-version','audit_escape','plain procedure','ui:operate')]:
  s,x=call('POST','/team/import',{'package_b64':package(ident,name,body,permission=perm)})
  emit(case,http=s,blocked=x.get('blocked'),findings=x.get('findings'),error=x.get('error'),escaped_file_exists=(data/'escaped-audit-version/versions.json').exists() if case=='share_id_escape' else None)
 for body in ['revision A','revision B']:
  s,x=call('POST','/team/import',{'package_b64':package('audit-same-version','audit_same_version',body)})
  emit('share_same_version_'+body[-1],http=s,blocked=x.get('blocked'),versions=x.get('versions'))
 s,x=call('GET','/team/versions?id=audit-same-version');emit('share_same_version_history',http=s,count=x.get('count'))
except Exception as e:emit('harness_error',error=str(e))
finally:stop();(base/'sharing-results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')