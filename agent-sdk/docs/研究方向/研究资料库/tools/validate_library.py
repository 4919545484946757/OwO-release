import hashlib,json,os,re,zipfile
from pathlib import Path
from urllib.parse import unquote
from pypdf import PdfReader
DOCS=Path.cwd()/'agent-sdk/docs';LIB=DOCS/'研究方向/研究资料库'
papers=json.loads((LIB/'papers/manifest.json').read_text(encoding='utf-8'));repos=json.loads((LIB/'project-snapshots/catalog.json').read_text(encoding='utf-8'))
errors=[];pdfs=[];archives=[];selected=[]
for r in papers:
    p=LIB/r['file'];b=p.read_bytes();reader=PdfReader(str(p));ok=b.startswith(b'%PDF') and len(reader.pages)==r['pages'] and hashlib.sha256(b).hexdigest()==r['sha256']
    pdfs.append({'file':r['file'],'pages':len(reader.pages),'valid':ok})
    if not ok:errors.append('PDF/hash '+r['file'])
for r in repos:
    p=LIB/r['file'];b=p.read_bytes()
    with zipfile.ZipFile(p) as z:bad=z.testzip()
    ok=bad is None and hashlib.sha256(b).hexdigest()==r['sha256'];archives.append({'repository':r['repository'],'valid':ok})
    if not ok:errors.append('ZIP/hash '+r['file'])
    note=LIB/r['notes_dir'];a=json.loads((note/'selected-source-manifest.json').read_text(encoding='utf-8'))
    for x in a:
        p=note/'selected-source'/x['path'];ok=p.is_file() and hashlib.sha256(p.read_bytes()).hexdigest()==x['sha256'];selected.append(ok)
        if not ok:errors.append('selected source '+str(p))
docs=list(DOCS.glob('*.md'))
for folder in ['architecture','guides','plans','reports','adr','internal']:docs.extend((DOCS/folder).rglob('*.md'))
for folder in ['qa','review']:docs.extend((DOCS/folder).glob('*.md'))
docs.extend((DOCS/'研究方向').glob('*.md'));docs.extend((DOCS/'研究方向/专题').glob('*.md'));docs.extend((LIB/'paper-notes').glob('*.md'))
docs.append(LIB/'README.md');docs.extend((LIB/'project-notes').glob('*/README.md'));docs+=[DOCS/'qa/evidence/README.md',DOCS/'qa/logs/README.md']
# Original reference draft is preserved, not rewritten or included in newly managed link checks.
docs=sorted(set(p for p in docs if p.exists() and p.name!='参考项目.md'))
links=0;broken=[];utf8=[]
for p in docs:
    try:text=p.read_text(encoding='utf-8-sig');utf8.append(str(p.relative_to(DOCS)))
    except UnicodeError as e:errors.append('UTF8 '+str(p));continue
    for m in re.finditer(r'(?<!!)\[[^\]\n]+\]\(([^)\n]+)\)',text):
        target=m.group(1)
        if target.startswith('<'):target=target[1:target.index('>')]
        target=re.sub(r'\s+["\x27].*$','',target)
        if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:',target) or target.startswith(('#','/')):continue
        target=unquote(target.split('#')[0])
        if not target:continue
        links+=1;resolved=(p.parent/target).resolve()
        if not resolved.exists():broken.append({'document':str(p.relative_to(DOCS)),'target':target})
errors.extend(['broken link '+str(x) for x in broken])
manifestlines=(LIB/'SHA256SUMS.txt').read_text(encoding='utf-8').splitlines();sumok=True
for line in manifestlines:
    want,file=line.split('  ',1)
    if hashlib.sha256((LIB/file).read_bytes()).hexdigest()!=want:sumok=False;errors.append('SUM '+file)
report={'checked_at':'2026-10-13','scope':'maintained docs and new research cards; excludes untouched reference draft, third-party READMEs, raw evidence snapshots, downloaded source documentation','pdf_count':len(pdfs),'zip_count':len(archives),'checksum_entries':len(manifestlines),'selected_source_files':len(selected),'markdown_files_checked':len(docs),'local_links_checked':links,'broken_links':broken,'utf8_valid_count':len(utf8),'checksum_match':sumok,'pdf_results':pdfs,'zip_results':archives,'errors':errors,'status':'passed' if not errors else 'failed','not_run':['Rust build/tests','downloaded project installation/execution','paper experiment reproduction','cloud model evaluation']}
(LIB/'validation.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps({k:report[k] for k in ['status','pdf_count','zip_count','checksum_entries','selected_source_files','markdown_files_checked','local_links_checked','broken_links','errors']},ensure_ascii=False),flush=True)
