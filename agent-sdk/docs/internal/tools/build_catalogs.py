import collections,json,os,re
from pathlib import Path
from urllib.parse import quote
DOCS=Path.cwd()/'agent-sdk/docs';BASE=DOCS/'研究方向';LIB=BASE/'研究资料库'
def q(p):return quote(str(p).replace('\\','/'),safe='/._-')
def write(p,s):p.parent.mkdir(parents=True,exist_ok=True);p.write_text(s.rstrip()+'\n',encoding='utf-8')
papers=json.loads((LIB/'papers/manifest.json').read_text(encoding='utf-8'));projects=json.loads((LIB/'project-snapshots/catalog.json').read_text(encoding='utf-8'))
ptopics=collections.defaultdict(list);rtopics=collections.defaultdict(list)
for r in papers:ptopics[r['topic']].append(r)
for r in projects:rtopics[r['topic']].append(r)
for r in papers:
    lines=['# '+r['title'],'','- 编号：'+r['id'],'- 主题：'+r['topic'],'- 作者：'+'；'.join(r.get('authors',[])),'- 本地 PDF 版本：'+r['pdf_version'],'- 页数：'+str(r['pages']),
      '- [原始来源]('+r['source_url']+')','- [本地 PDF](../'+q(r['file'])+')','- [前三页文本](../paper-text/'+q(r['id'])+'.txt)','- SHA-256：'+r['sha256'],'',
      '## 可借鉴机制','',r['lesson'],'','## 适用边界','',r['limits'],'',
      '本卡片基于原始元数据与前三页核对，并未完成独立复现实验。应用于 OwO 时需跟踪当前调用链、定义对应验收，再评估实际效果。']
    write(LIB/'paper-notes'/(r['id']+'.md'),'\n'.join(lines))
lines=['# Agent 论文与 GitHub 项目调研','',
  '整理日期：2026-10-13。本地收录 '+str(len(papers))+' 篇论文 PDF 和 '+str(len(projects))+' 个项目快照，其中 40 个含源码、1 个为 Letta 迁移说明。前一批为 9 篇论文和 4 个源码包；本轮新增 39 篇论文及 36 个源码包，并保留迁移说明。','',
  '本轮完成主题检索、原始元数据与 PDF 结构核对、前三页提取和部分源码定位；未逐篇全文精读、未运行下载项目或复现实验。[检索与限制](检索方法与来源记录.md)、[本地资料库](研究资料库/README.md)。','',
  '## 按主题阅读','','[框架](专题/01-Agent框架与分层架构.md) · [持久化工作流](专题/02-工作流与持久化执行.md) · [并行开发](专题/03-多Agent并行开发与协作.md) · [自动优化](专题/04-自动化工作流与自优化.md) · [记忆](专题/05-记忆与共享上下文.md) · [评测](专题/06-评测与交付验收.md) · [安全与协议](专题/07-权限注入防御与协议.md) · [发展方向](专题/08-发展方向与研究问题.md)','']
for topic,rs in ptopics.items():
    lines+=['## 论文：'+topic,'','| 论文 / 编号 | 可借鉴机制 | 适用边界 | 本地与来源 |','| --- | --- | --- | --- |']
    for r in rs:
        lines+=['| '+r['title'].replace('|','/')+' / '+r['id']+' | '+r['lesson']+' | '+r['limits']+' | [PDF](研究资料库/'+q(r['file'])+') · [阅读卡片](研究资料库/paper-notes/'+q(r['id'])+'.md) · [原文]('+r['source_url']+') |']
    lines+=['']
for topic,rs in rtopics.items():
    lines+=['## 项目：'+topic,'','| 项目 | 阅读目标 | 版本与本地入口 |','| --- | --- | --- |']
    for r in rs:
        focus=str(r.get('focus') or '')[:160].replace('|','/').replace('\n',' ')
        lines+=['| ['+r['repository']+']('+r['source_url']+') | '+focus+' | '+r['commit'][:12]+' · [阅读卡片](研究资料库/'+q(r['notes_dir'])+'/README.md) · [ZIP](研究资料库/'+q(r['file'])+') |']
    lines+=['']
lines+=['## 如何用于开发','','先读专题结论，再选 2–3 个项目追踪实际源码；用[项目开发映射](项目开发映射与研究验证计划.md)形成契约、实验和验收任务。不得跨论文直接比较成功率或将外部结果当作本项目证据。','',
 '源码包保留固定提交、校验与筛选范围。CrewAI 为核心模块选读包，新收录其他项目为筛选文本源码，缺少依赖/大数据不能直接复现实验。[全部元数据](研究资料库/project-snapshots/catalog.json)和[许可证原文](研究资料库/README.md)用于进一步核对。']
write(BASE/'论文与GitHub项目调研.md','\n'.join(lines))
lines=['# Agent 研究资料库','',
  '采集与整理日期：2026-10-13。当前有 48 篇论文 PDF、40 个源码包和 1 个项目迁移说明包，约 '+str(round(sum(r['bytes'] for r in papers+projects)/1024/1024,1))+' MiB（不含选读副本、元数据及工具）。','',
 '## 入口','','- [论文与项目总表](../论文与GitHub项目调研.md)','- [八个主题综述](../README.md)','- [论文元数据及 PDF 哈希](papers/manifest.json)','- [项目提交、范围与源码哈希](project-snapshots/catalog.json)','- [统一 SHA-256](SHA256SUMS.txt)','- [BibTeX](references.bib)','- [文件与链接校验结果](validation.json)','',
 '## 离线资料结构','','| 目录 | 内容 |','| --- | --- |','| papers/ | 48 篇原始 PDF；作者、来源、版本和页数见 manifest |','| paper-notes/ | 每篇论文的阅读卡片、机制与限制 |','| paper-text/ | PDF 前三页文本，用于搜索和初步阅读 |','| project-snapshots/ | 固定提交 ZIP；manifest.json 保留前一批，catalog.json 为全部清单 |','| project-notes/ | 41 份阅读卡片、上游 README、许可与文件范围 |','| project-notes/*/selected-source/ | 合计 477 个可直接打开的选读源码文件 |','| metadata/ | 原始出版页 HTML、下载记录、候选错误记录 |','| tools/ | 采集、筛选、整理工具及输入种子 |','',
 '## 快照范围与许可','','新源码包保留可阅读文本及许可，过滤安装依赖、二进制、生成目录、实验运行输出和录制响应。逐文件范围在各项目 source-map.json 中。CrewAI 只下载选定核心模块，不能据此认为已取得完整仓库。所有包均未安装或执行。','',
 'Letta 主仓库当前是迁移说明，当前源码在 letta-ai/letta-code；Strands 旧地址已重定向到 strands-agents/harness-sdk。','',
 'n8n 保留 Sustainable Use 与 Enterprise 许可文件；Dify 保留带附加条件的许可；AutoGen 有 LICENSE 和 LICENSE-CODE。许可可能区分文档、源码和组件，GitHub SPDX 字段不充分，采用具体文件前核对原文。未确认许可的资料只记录事实，不推断可自由复制。','',
 '## 下载论文','','| 主题 | 数量 |','| --- | --- |',*['| '+k+' | '+str(len(v))+' |' for k,v in ptopics.items()],'',
 '## 源码阅读卡片','']
for r in projects:lines+=['- ['+r['repository']+']('+q(r['notes_dir'])+'/README.md)：'+r['commit'][:12]+'，'+str(r['selected_source_files'])+' 个选读文件。']
lines+=['','论文核验仅涵盖元数据、结构和前三页；完整实验设置、附录及可靠性结论仍需进一步精读。PDF 或源码存在不构成 OwO 的功能验收。']
write(LIB/'README.md','\n'.join(lines))
# Add updated sources to the development direction without discarding earlier reasoning.
p=BASE/'Agent发展方向与设计建议.md';s=p.read_text(encoding='utf-8')
s+='\n\n## 扩展调研后的阅读路线（2026-10-13）\n\n本地资料已扩展到 48 篇论文、40 个源码包及一个迁移说明。框架、工作流、记忆、安全和评测分别成章，见 [专题入口](README.md) 和 [总表](论文与GitHub项目调研.md)。\n\n设计应同时回答：任务如何定义、状态由谁维护、执行怎样受权、依赖怎样更新、失败怎样恢复、结果如何独立验收。源码选读包含 LangGraph 的状态与 checkpoint、Temporal 的 workflow instance、Rig 的 builder/run、CrewAI 的 Flow persistence，以及 n8n 的工作流执行入口。\n\n这些资料支持机制对照，具体实施仍先核查现有 Daemon、ToolHost、WorkSwarm 与持久化模块。[开发映射与研究计划](项目开发映射与研究验证计划.md) 给出切口和可评审交付物；[发展方向专题](专题/08-发展方向与研究问题.md) 列出可检验假设。\n'
write(p,s)
# Update metadata statement now that all author fields have been supplemented.
p=BASE/'检索方法与来源记录.md';s=p.read_text(encoding='utf-8')
s=s.replace('原 9 篇论文的作者字段尚未补齐到统一元数据，BibTeX 对这些条目保留缺失，不编造作者；正式论文引用前应回原始来源补全。','48 篇论文均已补齐作者字段；前一批的作者从原始出版页或 PDF 首页核对。正式引用仍应核对论文版本、会议/期刊信息与 BibTeX 格式。')
write(p,s)
# QA navigations preserve evidence locations.
e=DOCS/'qa/evidence';groups=collections.defaultdict(list)
for p in e.glob('*.json'):
    name=p.name;kind='Team / WorkSwarm' if name.startswith('team-') else 'Goal / Workflow / Automation' if name.startswith(('goal-','workflow-','automation-')) else '其他专项'
    groups[kind].append(p)
lines=['# 原始证据索引','','只索引本目录顶层 JSON，嵌套材料和快照保留原位置。文件的日期、源码身份、验证方法与退出码以正文为准；本次没有重新执行。','']
for kind,fs in groups.items():
    lines+=['## '+kind,'',*['- ['+p.name+']('+q(p.name)+')' for p in sorted(fs)],'']
write(e/'README.md','\n'.join(lines))
logs=DOCS/'qa/logs';count=sum(1 for p in logs.rglob('*') if p.is_file())
lines=['# 历史日志索引','','本目录共 '+str(count)+' 个日志与摘要文件，保持原路径。先按日期和任务找到 summary/evidence，再读取逐行日志；不能以文件名中的 passed 代替查看内容。','',
 '## 批次目录','',*['- ['+p.name+']('+q(p.name)+'/)：'+str(sum(1 for x in p.rglob('*') if x.is_file()))+' 个文件。' for p in sorted(logs.iterdir()) if p.is_dir()],'',
 '顶层日志按原文件名检索，包含 mk 微内核、core/server/cli、clippy、格式及专项验证记录。']
write(logs/'README.md','\n'.join(lines))
# Automatically index small subdirectories.
for name in ['plans/team','plans/product','reports/team','reports/reliability']:
    p=DOCS/name;write(p/'README.md','# '+name+' 文档\n\n'+'\n'.join('- ['+x.stem+']('+q(x.name)+')' for x in sorted(p.glob('*.md')) if x.name!='README.md')+'\n\n返回 [文档入口]('+q(os.path.relpath(DOCS/'README.md',p))+')。')
report=json.loads((DOCS/'internal/document-organization-20261013.json').read_text(encoding='utf-8'));arch=sum('/archive/' in x['to'] for x in report['moved'])
lines=['# 文档整理记录（2026-10-13）','',
 '本轮实际移动 26 份文档，其中 '+str(arch)+' 份进入设计/报告归档。原内容保留，相关 Markdown 引用和移动后可定位链接同步调整。源码中引用旧微内核路径的注释通过顶层跳转文档兼容。','',
 '## 迁移明细','','| 原路径 | 新路径 | 原因 |','| --- | --- | --- |']
for m in report['moved']:lines+=['| '+m['from']+' | [当前文件](../'+q(m['to'])+') | '+m['reason']+' |']
lines+=['','## 原有证据缺失','',
 '可靠性审查提到的 qa/evidence/task-execution-audit-20261010/ 在当前工作树没有对应证据；非桌面审查提到的 project-wide-audit-20261010/ 中 README.md、inspection-results.json、source-manifest.json、typescript-syntax.json 同样缺失。正文已记录缺失，并取消无效链接，不用其他日期证据替代。','',
 '## 保留的材料','','QA 证据、日志、安装快照和复核 ZIP 保持原路径。既有参考项目.md 草稿保持内容。工作区原有 .coord6/.coord7/.coord8 删除保持。目录调整不改变历史验证方法或结论。','',
 '外部 Markdown 中两处 builGoal 导航随迁移更新，产品源码未修改。未提交、未推送、未执行 Rust 或下载项目。','',
 '完整可机器读取记录见 [JSON](document-organization-20261013.json)。当次文件、编码、链接、PDF、ZIP 和哈希检查见 [资料库校验](../研究方向/研究资料库/validation.json)。']
write(DOCS/'internal/文档整理记录-2026-10-13.md','\n'.join(lines))
# Inventory maintained docs, omitting downloaded upstream source and evidence snapshots.
docs=list(DOCS.glob('*.md'))
for dirname in ['architecture','guides','plans','reports','adr','internal']:docs.extend((DOCS/dirname).rglob('*.md'))
for dirname in ['qa','review']:docs.extend((DOCS/dirname).glob('*.md'))
docs.extend(BASE.glob('*.md'));docs.extend((BASE/'专题').glob('*.md'));docs.append(LIB/'README.md')
lines=['# 文档目录清单','',
 '本表索引维护文档与研究综述，不逐项列入下载源码、论文阅读卡片、原始日志和证据快照。后者分别由资料库、QA 和 Review 入口索引。','',
 '| 文档 | 分类 | 状态口径 |','| --- | --- | --- |']
for p in sorted(set(docs)):
    if p.name=='文档目录清单.md':continue
    path=p.relative_to(DOCS).as_posix()
    status='历史记录/设计参考，需按版本复核' if '/archive/' in path else '源码导航/设计/操作说明，按正文证据范围读取'
    if path.startswith('研究方向'):status='研究资料与设计建议，未复现实验'
    lines+=['| ['+path+']('+q(path)+') | '+path.split('/')[0]+' | '+status+' |']
write(DOCS/'文档目录清单.md','\n'.join(lines))
print('Generated catalogs, reading cards, navigation and inventory',len(papers),len(projects),'archived',arch,flush=True)
