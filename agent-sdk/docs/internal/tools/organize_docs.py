import json,os,re,subprocess,zipfile
from pathlib import Path
from urllib.parse import quote,unquote
ROOT=Path.cwd().resolve();DOCS=(ROOT/'agent-sdk/docs').resolve()
moves=json.loads((DOCS/'internal/tools/moves-20261013.json').read_text(encoding='utf-8'))
mapping={(DOCS/a).resolve():(DOCS/b).resolve() for a,b,_ in moves}
def authored():
    fs=list(DOCS.glob('*.md'))
    for name in ['architecture','guides','plans','reports','adr','internal']:
        fs.extend((DOCS/name).rglob('*.md'))
    fs.extend((DOCS/'qa').glob('*.md'));fs.extend((DOCS/'review').glob('*.md'));fs.extend((DOCS/'研究方向').glob('*.md'))
    fs.append(DOCS/'研究方向/研究资料库/README.md')
    return sorted(set(f.resolve() for f in fs if f.exists() and f.name!='参考项目.md' and 'tools' not in f.parts))
original={f:f.read_text(encoding='utf-8-sig') for f in authored()}
for a,b in mapping.items():
    if not a.is_relative_to(DOCS) or not b.is_relative_to(DOCS): raise ValueError('move outside docs')
    if not a.exists() or b.exists(): raise ValueError('move source missing or target exists: '+str(a))
backup=ROOT/'.tmp/docs-organize-20261013';backup.mkdir(parents=True,exist_ok=True)
with zipfile.ZipFile(backup/'before.zip','w',zipfile.ZIP_DEFLATED) as z:
    for p in original:z.writestr(p.relative_to(ROOT).as_posix(),p.read_bytes())
def rel(target,parent):return Path(os.path.relpath(target,parent)).as_posix()
rewritten=[];repairs=[];previous_missing=[]
pattern=re.compile(r'(?<!!)\[([^\]\n]*)\]\(([^)\n]+)\)')
def rewrite(text,old,new):
    def link(m):
        label,target=m.groups(); angle=target.startswith('<') and '>' in target
        if angle: dest=target[1:target.index('>')];tail=target[target.index('>')+1:]
        else:
            spl=re.match(r'^(.*?)(\s+["\x27].*)?$',target);dest=spl.group(1);tail=spl.group(2) or ''
        if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:',dest) or dest.startswith(('#','/')):return m.group(0)
        local,sep,anchor=dest.partition('#')
        if not local:return m.group(0)
        resolved=(old.parent/unquote(local)).resolve()
        if not resolved.exists() and resolved not in mapping:
            for base in [ROOT,ROOT/'agent-sdk',DOCS]:
                candidate=(base/unquote(local)).resolve()
                if candidate.exists() or candidate in mapping:
                    resolved=candidate;repairs.append({'document':old.relative_to(DOCS).as_posix(),'target':dest});break
            else:
                previous_missing.append({'document':old.relative_to(DOCS).as_posix(),'target':dest});return m.group(0)
        updated=rel(mapping.get(resolved,resolved),new.parent)
        updated=quote(updated,safe='/._-')+(sep+anchor if sep else '')
        if angle:updated='<'+updated+'>'+tail
        else:updated+=tail
        return '['+label+']('+updated+')'
    text=pattern.sub(link,text)
    for a,b in mapping.items():
        oldsdk='docs/'+a.relative_to(DOCS).as_posix();newsdk='docs/'+b.relative_to(DOCS).as_posix()
        text=text.replace(oldsdk,newsdk)
    return text
for old,content in original.items():
    new=mapping.get(old,old);updated=rewrite(content,old,new)
    if new!=old:
        new.parent.mkdir(parents=True,exist_ok=True);old.rename(new)
    if updated!=content:
        new.write_text(updated,encoding='utf-8');rewritten.append(new.relative_to(DOCS).as_posix())
# Keep a tiny compatibility entry for source comments that cite the old architecture path.
(DOCS/'ARCH-MICROKERNEL.md').write_text('# 微内核文档已迁移\n\n完整设计与拆分台账见 [architecture/ARCH-MICROKERNEL.md](architecture/ARCH-MICROKERNEL.md)。\n\n此页用于兼容源码注释中的旧路径；文档导航见 [README](README.md)。\n',encoding='utf-8')
external=[]
for p in (ROOT/'builGoal').glob('*.md'):
    text=p.read_text(encoding='utf-8-sig');new=text
    for a,b in mapping.items():new=new.replace('docs/'+a.relative_to(DOCS).as_posix(),'docs/'+b.relative_to(DOCS).as_posix())
    if new!=text:p.write_text(new,encoding='utf-8');external.append(str(p.relative_to(ROOT)))
report={'date':'2026-10-13','moved':[{'from':a,'to':b,'reason':c} for a,b,c in moves],'rewritten_documents':rewritten,'updated_external_markdown':external,'resolved_existing_relative_links':repairs,'unresolved_preexisting_links':previous_missing}
(DOCS/'internal/document-organization-20261013.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps({'moved':len(moves),'rewritten':len(rewritten),'external':external,'repaired':len(repairs),'preexisting_missing':len(previous_missing)},ensure_ascii=False))
