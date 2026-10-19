import io,json,re,subprocess,zipfile,datetime,hashlib,concurrent.futures
from pathlib import Path
import collect_sources as c
ROOT=c.ROOT
original_fetch=c.fetch
refcache={}
def refs(repo):
    if repo not in refcache:
        p=subprocess.run(['git','-c','http.sslBackend=openssl','ls-remote','--symref','https://github.com/'+repo+'.git','HEAD'],capture_output=True,text=True,timeout=60)
        if p.returncode: raise RuntimeError(p.stderr.strip())
        branch=re.search(r'ref: refs/heads/([^\s]+)',p.stdout).group(1);sha=re.search(r'(?m)^([a-f0-9]{40})\s+HEAD',p.stdout).group(1)
        refcache[repo]={'full_name':repo,'default_branch':branch,'sha':sha,'license':None,'description':None,'archived':None,'pushed_at':None,'identity_source':'git ls-remote public HEAD'}
    return refcache[repo]
def fetch(url,limit=180*1024*1024):
    if url.startswith('https://api.github.com/repos/'):
        value=url.removeprefix('https://api.github.com/repos/');repo='/'.join(value.split('/')[:2]);m=refs(repo)
        return json.dumps({'sha':m['sha']} if '/commits/' in value else m).encode(),url
    return original_fetch(url,limit)
c.fetch=fetch
def crew(seed):
    repo,topic,focus=seed;meta=refs(repo);sha=meta['sha'];prefix=repo.split('/')[-1]+'-'+sha
    r={'repository':repo,'topic':topic,'focus':focus,'source_url':'https://github.com/'+repo,'commit':sha,'branch':meta['default_branch'],'license_spdx':None,'identity_source':'git ls-remote public HEAD','snapshot_kind':'selected core modules only','collected_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    paths=['README.md','LICENSE','LICENSE.md','LICENSE.txt','pyproject.toml','lib/crewai/pyproject.toml','lib/crewai/src/crewai/__init__.py','lib/crewai/src/crewai/crew.py','lib/crewai/src/crewai/task.py','lib/crewai/src/crewai/agent/core.py','lib/crewai/src/crewai/process.py','lib/crewai/src/crewai/agents/agent_builder/base_agent.py','lib/crewai/src/crewai/agents/crew_agent_executor.py','lib/crewai/src/crewai/flow/flow.py','lib/crewai/src/crewai/flow/persistence/base.py','lib/crewai/src/crewai/flow/persistence/sqlite.py','lib/crewai/src/crewai/state/event_record.py','lib/crewai/src/crewai/state/crew_state.py','lib/crewai/src/crewai/memory/types.py','lib/crewai/src/crewai/tools/base_tool.py','lib/crewai/src/crewai/tools/agent_tools/delegate_work_tool.py','lib/crewai/src/crewai/tools/agent_tools/ask_question_tool.py']
    output=io.BytesIO();kept=[];omitted=[];note=ROOT/'project-notes'/repo.replace('/','--');note.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED) as z:
        for path in paths:
            url='https://raw.githubusercontent.com/'+repo+'/'+sha+'/'+path
            try: body,_=original_fetch(url,4*1024*1024)
            except Exception as e: omitted.append({'path':path,'reason':str(e)});continue
            z.writestr(prefix+'/'+path,body);kept.append({'path':path,'bytes':len(body),'url':url})
            if path=='README.md':(note/'UPSTREAM_README.md').write_bytes(body)
            if Path(path).name.upper().startswith('LICENSE'):(note/Path(path).name).write_bytes(body)
    if len(kept)<8:raise ValueError('too few core files')
    data=output.getvalue();filename=repo.replace('/','--')+'-'+sha[:12]+'-core.zip';(ROOT/'project-snapshots'/filename).write_bytes(data)
    (note/'source-map.json').write_text(json.dumps({'kept':kept,'omitted':omitted,'scope':'selected paths; all other files omitted'},ensure_ascii=False,indent=2),encoding='utf-8')
    (note/'repository.json').write_text(json.dumps(meta,indent=2),encoding='utf-8')
    r.update(status='downloaded',file='project-snapshots/'+filename,bytes=len(data),sha256=hashlib.sha256(data).hexdigest(),kept_files=len(kept),omitted_files=len(omitted),notes_dir='project-notes/'+repo.replace('/','--'),license_files=[p.name for p in note.glob('LICENSE*')])
    print('PROJECT OK '+repo+' SELECTED '+str(len(kept))+' files',flush=True);return r
def main():
    records=json.loads((ROOT/'metadata/projects-expanded.json').read_text(encoding='utf-8'));config=json.loads((ROOT/'tools/research-seeds.json').read_text(encoding='utf-8'))
    seeds=[s for s in config['projects'] if any(r['repository']==s[0] and r['status']=='failed' for r in records)]
    seeds.extend([['modelcontextprotocol/rust-sdk','协议工具','Rust MCP Client/Server、transport 与取消'],['a2aproject/a2a-python','协作协议','Agent card、task lifecycle 与流式事件'],['stanfordnlp/dspy','工作流优化','类型化模块、优化器、指标和轨迹']])
    def worker(seed):
        try:
            if seed[0]=='crewAIInc/crewAI':return crew(seed)
            value=c.project(seed);value['identity_source']='git ls-remote public HEAD';return value
        except Exception as e:print('PROJECT FAIL '+seed[0]+' '+str(e),flush=True);return {'repository':seed[0],'status':'failed','error':str(e),'topic':seed[1],'focus':seed[2]}
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for result in pool.map(worker,seeds):
            records=[r for r in records if r['repository']!=result['repository']];records.append(result)
            (ROOT/'metadata/projects-expanded.json').write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')
    print('DONE projects total '+str(sum(r['status']=='downloaded' for r in records))+'/'+str(len(records)),flush=True)
if __name__=='__main__':main()
