import concurrent.futures,json
import collect_sources as c
p=c.ROOT/'metadata/papers-expanded.json';old=json.loads(p.read_text(encoding='utf-8'));rejected=[x for x in old if x['status']=='failed']
(c.ROOT/'metadata/rejected-seeds.json').write_text(json.dumps(rejected,ensure_ascii=False,indent=2),encoding='utf-8')
old=[x for x in old if x['status']=='downloaded']
config=json.loads((c.ROOT/'tools/paper-fixes.json').read_text(encoding='utf-8'))
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
    for r in pool.map(c.paper,config):
        old=[x for x in old if x['id']!=r['id']];old.append(r);p.write_text(json.dumps(old,ensure_ascii=False,indent=2),encoding='utf-8')
print('PAPERS total',sum(x['status']=='downloaded' for x in old),len(old),flush=True)
