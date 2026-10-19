import json
import retry_projects as r
p=r.ROOT/'metadata/projects-expanded.json';records=json.loads(p.read_text(encoding='utf-8'))
seeds=[['letta-ai/letta-code','有状态Agent','当前 Letta Agent harness、server 与记忆集成'],['xlang-ai/OSWorld','桌面自动化评测','任务环境、桌面动作与最终状态检查'],['THUDM/AgentBench','通用Agent评测','交互环境、控制器与评估'],['uiuc-kang-lab/InjecAgent','安全评测','工具输出注入、攻击样本与评估脚本']]
for seed in seeds:
    value=r.c.project(seed);value['identity_source']='git ls-remote public HEAD'
    records=[x for x in records if x['repository']!=value['repository']];records.append(value)
    p.write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')
print('PROJECTS total',sum(x['status']=='downloaded' for x in records),len(records),flush=True)
