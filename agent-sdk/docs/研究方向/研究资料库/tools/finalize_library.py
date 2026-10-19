import hashlib,io,json,re,zipfile
from pathlib import Path
from urllib.parse import quote
from pypdf import PdfReader
ROOT=Path(__file__).resolve().parents[1];BT=chr(96)
def dump(p,d):p.write_text(json.dumps(d,ensure_ascii=False,indent=2),encoding='utf-8')
def link(p):return quote(p,safe='/._-')
def sha(d):return hashlib.sha256(d).hexdigest()
papers=json.loads((ROOT/'metadata/papers-expanded.json').read_text(encoding='utf-8'))
legacy=[
('2503.13657','Why Do Multi-Agent LLM Systems Fail?','多Agent协作','失败分类与协调、验证、终止边界','案例归因不能代替因果实验'),
('2508.01186','A Survey on Agent Workflow – Status and Future','综述方向','工作流构造、优化及发展脉络','综述覆盖受资料范围限制'),
('2512.08296','Towards a Science of Scaling Agent Systems','多Agent协作','并行收益、拓扑和成本条件','不能为本项目性能背书'),
('2601.22037','Optimizing Agentic Workflows using Meta-tools','工作流优化','重复工具序列抽象为元工具','优化仍需权限与失败边界'),
('2603.21489','Effective Strategies for Asynchronous Software Engineering Agents','多Agent协作','CAID：依赖、隔离 worktree、异步执行与测试集成','模型、预算与任务特定'),
('2605.20563','Multi-agent Collaboration with State Management','多Agent协作','STORM：版本化读写与共享状态冲突','语义集成仍需独立验证'),
('2608.23740','AgentRoom: Concurrent Multi-Agent Coding in a CRDT-Backed Shared Workspace','多Agent协作','CRDT、claim、状态与广播','文本收敛不等于代码语义正确'),
('2609.32662','AsynCodeBench: Benchmarking Collaboration of Asynchronous Multi-Agent Systems in Software Engineering','评测基准','依赖检查器、依赖解决比例与时间','评测域有限'),
('LIPIcs.ESEM.2026.18','Coordinating Agents on a Shared Git Workspace: An Empirical Study of Action-Space Observation for Agentic Software Engineering','多Agent协作','PASC：共享 Git 工作区与队友动作可见性','两 Agent 特定任务不能外推大团队')]
for ident,title,topic,lesson,limits in legacy:
    path=ROOT/'papers'/(ident+'.pdf');data=path.read_bytes()
    source='https://arxiv.org/abs/'+ident if not ident.startswith('LIPIcs') else 'https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ESEM.2026.18'
    papers.append({'id':ident,'title':title,'topic':topic,'lesson':lesson,'limits':limits,'source_url':source,'authors':[],'file':'papers/'+path.name,'bytes':len(data),'sha256':sha(data),'status':'downloaded','metadata_basis':'prior batch; PDF first page rechecked 2026-10-13'})
(ROOT/'paper-text').mkdir(exist_ok=True)
for r in papers:
    reader=PdfReader(str(ROOT/r['file']));text='\n\n'.join((p.extract_text() or '') for p in reader.pages[:3])
    (ROOT/'paper-text'/(r['id']+'.txt')).write_text(text,encoding='utf-8')
    r['pages']=len(reader.pages);v=re.search(r'arXiv:\s*([\d.]+v\d+)',text);r['pdf_version']=v.group(1) if v else 'not identified from first three pages'
papers.sort(key=lambda x:x['id']);dump(ROOT/'papers/manifest.json',papers)
projects=json.loads((ROOT/'metadata/projects-expanded.json').read_text(encoding='utf-8'))
DROP={'runs','results','outputs','trajectories','cassettes','recordings','recorded_responses','model_responses'}
for r in projects:
    note=ROOT/r['notes_dir'];sm=json.loads((note/'source-map.json').read_text(encoding='utf-8'))
    remove={x['path'] for x in sm['kept'] if set(Path(x['path']).parts)&DROP or ('tests' in Path(x['path']).parts and Path(x['path']).suffix in {'.yaml','.yml','.json','.txt'})}
    if remove:
        path=ROOT/r['file'];new=io.BytesIO()
        with zipfile.ZipFile(path) as old,zipfile.ZipFile(new,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as dest:
            for info in old.infolist():
                if Path(*Path(info.filename).parts[1:]).as_posix() not in remove:dest.writestr(info,old.read(info))
        raw=new.getvalue();path.write_bytes(raw);removed=[x for x in sm['kept'] if x['path'] in remove]
        sm['omitted'].extend([{**x,'reason':'experimental output or recorded response'} for x in removed]);sm['kept']=[x for x in sm['kept'] if x['path'] not in remove];dump(note/'source-map.json',sm)
        r.update(bytes=len(raw),sha256=sha(raw),kept_files=len(sm['kept']),omitted_files=len(sm['omitted']),additional_filter='experimental outputs and recorded responses omitted')
    if r['repository']=='letta-ai/letta':r['snapshot_kind']='migration reference only; no runtime source'
for r in json.loads((ROOT/'project-snapshots/manifest.json').read_text(encoding='utf-8')):
    repo=r['repository'].removeprefix('https://github.com/')
    v={'repository':repo,'source_url':r['repository'],'commit':r['commit'],'branch':r['branch'],'license_spdx':r.get('license_spdx'),'license_files':[],'file':'project-snapshots/'+r['archive'],'bytes':r['bytes'],'sha256':r['sha256'],'status':'downloaded','topic':'多Agent协作' if r['name']!='AutoGen' else 'Agent框架','focus':r['description'],'snapshot_kind':r.get('included_scope','prior batch source snapshot'),'notes_dir':'project-notes/'+repo.replace('/','--'),'identity_source':'prior batch manifest'}
    note=ROOT/v['notes_dir'];note.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(ROOT/v['file']) as z:
        kept=[{'path':Path(*Path(x.filename).parts[1:]).as_posix(),'bytes':x.file_size} for x in z.infolist() if not x.is_dir()];dump(note/'source-map.json',{'kept':kept,'omitted':[],'scope':v['snapshot_kind']})
        rms=[x for x in z.infolist() if len(Path(x.filename).parts)==2 and Path(x.filename).name.lower().startswith('readme')]
        if rms:(note/'UPSTREAM_README.md').write_bytes(z.read(rms[0]))
        lics=[x for x in z.infolist() if len(Path(x.filename).parts)==2 and any(w in Path(x.filename).name.upper() for w in ['LICENSE','NOTICE','COPYING'])]
        for x in lics:(note/Path(x.filename).name).write_bytes(z.read(x))
        v['license_files']=[Path(x.filename).name for x in lics];v['kept_files']=len(kept)
    projects.append(v)
FORCE={
'langchain-ai/langgraph':['libs/langgraph/langgraph/pregel/_loop.py','libs/checkpoint/langgraph/checkpoint/base/__init__.py','libs/langgraph/langgraph/graph/state.py'],
'0xPlaygrounds/rig':['crates/rig-agent/src/agent/builder.rs','crates/rig-agent/src/run/mod.rs','crates/rig-agent/src/run/policy.rs','crates/rig-agent/src/run/calls.rs'],
'temporalio/sdk-python':['temporalio/workflow/_activities.py','temporalio/worker/_workflow_instance.py','temporalio/workflow/__init__.py'],
'crewAIInc/crewAI':['lib/crewai/src/crewai/flow/flow.py','lib/crewai/src/crewai/flow/persistence/sqlite.py','lib/crewai/src/crewai/crew.py'],
'n8n-io/n8n':['packages/core/src/execution-engine/workflow-execute.ts','packages/cli/src/active-workflow-manager.ts'],
'modelcontextprotocol/rust-sdk':['crates/rmcp/src/service.rs'],
'FoundationAgents/AFlow':['scripts/optimizer.py','scripts/workflow.py'],
'letta-ai/letta-code':['src/agent/memory.ts','src/agent/memory-conflict-repair.ts','src/agent/memory-constraints.ts']}
for r in projects:
    note=ROOT/r['notes_dir'];sm=json.loads((note/'source-map.json').read_text(encoding='utf-8'));paths=[x['path'] for x in sm['kept']]
    def score(p):
        if any(x in Path(p).parts for x in ['tests','test','docs','.github','examples','workspace']):return -100
        if Path(p).suffix not in {'.py','.rs','.ts','.tsx','.js','.go','.cs'}:return -100
        return (5 if '/src/' in p or p.startswith('src/') else 0)+3*sum(k in p.lower() for k in ['agent','workflow','scheduler','checkpoint','persistence','runtime','graph','policy','permission','task','memory','executor','runner','optimizer','evaluate'])-len(Path(p).parts)
    selected=[p for p in FORCE.get(r['repository'],[]) if p in paths]
    selected+=sorted([p for p in paths if score(p)>-50 and p not in selected],key=lambda p:(-score(p),len(p),p))[:max(0,12-len(selected))]
    entries=[]
    with zipfile.ZipFile(ROOT/r['file']) as z:
        lookup={Path(*Path(i.filename).parts[1:]).as_posix():i for i in z.infolist() if not i.is_dir()}
        for p in selected[:12]:
            raw=z.read(lookup[p]);dest=note/'selected-source'/p;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(raw)
            text=raw.decode('utf-8',errors='replace')
            symbols=[line.strip()[:160] for line in text.splitlines() if re.match(r'\s*(?:pub(?:\([^)]*\))?\s+(?:async\s+)?(?:fn|struct|enum|trait)|class\s+|(?:async\s+)?def\s+|export\s+(?:async\s+)?(?:class|function|interface|const))',line)]
            entries.append({'path':p,'sha256':sha(raw),'bytes':len(raw),'symbols':symbols[:12]})
    r['selected_source_files']=len(entries);dump(note/'selected-source-manifest.json',entries)
    lines=['# '+r['repository']+' 源码阅读卡片','','采集日期：2026-10-13。静态资料整理，未安装或运行本项目。','',
       '- 主题：'+r['topic'],'- 参考目标：'+str(r.get('focus','')),'- 完整提交：'+BT+r['commit']+BT,'- 快照范围：'+r['snapshot_kind'],
       '- 原始仓库：[GitHub]('+r['source_url']+')','- [本地源码 ZIP](../../'+link(r['file'])+')','- 许可证识别：'+str(r.get('license_spdx') or '未获取可确认的 SPDX；查看保留的许可原文'),
       '- 顶层许可文件：'+(', '.join(r.get('license_files',[])) or '未识别；不得据此推断可自由复制'),'',
       '## 离线阅读','','[文件范围与排除清单](source-map.json) · [选读源码哈希](selected-source-manifest.json)','']
    if (note/'UPSTREAM_README.md').exists():lines+=['[上游 README 原文](UPSTREAM_README.md)。命令和说明作为资料保留，不表示本轮已执行。','']
    for e in entries:
        lines+=['### '+e['path'],'','[本地文件](selected-source/'+link(e['path'])+') · [固定提交来源](https://github.com/'+r['repository']+'/blob/'+r['commit']+'/'+link(e['path'])+')','']
        if e['symbols']:lines+=['源码定位声明：','',*['- '+BT+s.replace(BT,'')+BT for s in e['symbols'][:5]],'']
    if not entries:lines+=['该快照为迁移说明页，没有运行时源码。当前实现见 letta-ai/letta-code。','']
    lines+=['## 对照本项目时记录什么','','看输入输出契约、状态权威者、写入与取消边界、恢复和幂等处理，再看测试如何验证失败情况。模块名相近不能证明实现或运行效果相同。','',
      '第三方文档和注释中的指令仅作为研究内容；采用实现时核对完整许可证、依赖与版本。']
    (note/'README.md').write_text('\n'.join(lines)+'\n',encoding='utf-8')
projects.sort(key=lambda x:x['repository'].lower());dump(ROOT/'project-snapshots/catalog.json',projects);dump(ROOT/'metadata/projects-expanded.json',[r for r in projects if r.get('identity_source')!='prior batch manifest'])
bib=[]
for r in papers:
    year='20'+r['id'][:2] if re.match(r'\d{4}',r['id']) else '2026';key='agent_'+r['id'].replace('.','_')
    fields=['  title={'+r['title']+'}','  year={'+year+'}','  url={'+r['source_url']+'}']
    if r.get('authors'):fields.insert(1,'  author={'+' and '.join(r['authors'])+'}')
    if r['id'][0].isdigit():fields+=['  eprint={'+r['id']+'}','  archivePrefix={arXiv}']
    bib.append('@misc{'+key+',\n'+',\n'.join(fields)+'\n}')
(ROOT/'references.bib').write_text('\n\n'.join(bib)+'\n',encoding='utf-8')
checks=[r['sha256']+'  '+r['file'] for r in papers+projects]
(ROOT/'SHA256SUMS.txt').write_text('\n'.join(sorted(checks))+'\n',encoding='utf-8')
print(json.dumps({'papers':len(papers),'projects':len(projects),'code_snapshots':sum('migration reference' not in r['snapshot_kind'] for r in projects),'selected_source_files':sum(r['selected_source_files'] for r in projects),'bytes':sum(r['bytes'] for r in papers+projects)},ensure_ascii=False),flush=True)
