import argparse, concurrent.futures, datetime, hashlib, html, io, json, re, time, zipfile
from pathlib import Path
import requests
from pypdf import PdfReader
ROOT=Path(__file__).resolve().parents[1]
def fetch(url,limit=180*1024*1024):
    for attempt in range(3):
        try:
            r=requests.get(url,headers={'User-Agent':'OwO-research-library/1.0'},timeout=(15,90),stream=True);r.raise_for_status();out=bytearray()
            for c in r.iter_content(256*1024):
                out.extend(c)
                if len(out)>limit: raise ValueError('download exceeds limit')
            return bytes(out),r.url
        except Exception:
            if attempt==2: raise
            time.sleep(2*(attempt+1))
def tags(page,name):
    return [html.unescape(x) for x in re.findall(r'<meta\s+name=["\x27]'+re.escape(name)+r'["\x27]\s+content=["\x27](.*?)["\x27]',page,re.I)]
def digest(data): return hashlib.sha256(data).hexdigest()
def paper(seed):
    ident,expected,topic,lesson,limit=seed
    record={'id':ident,'topic':topic,'lesson':lesson,'limits':limit,'source_url':'https://arxiv.org/abs/'+ident,'collected_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    try:
        data,url=fetch(record['source_url'],4*1024*1024);page=data.decode('utf-8');title=tags(page,'citation_title')
        if not title: raise ValueError('citation title missing')
        record.update(title=title[0],authors=tags(page,'citation_author'),submitted=tags(page,'citation_date'),abstract_url=url)
        if expected.lower() not in title[0].lower(): raise ValueError('seed/title mismatch: '+title[0])
        desc=re.search(r'<blockquote[^>]*class=["\x27]abstract[^>]*>(.*?)</blockquote>',page,re.S)
        record['abstract']=html.unescape(re.sub('<[^>]+>',' ',desc.group(1))).strip() if desc else ''
        pdfurls=tags(page,'citation_pdf_url');record['pdf_url']=pdfurls[0] if pdfurls else 'https://arxiv.org/pdf/'+ident
        raw,pdfurl=fetch(record['pdf_url'],40*1024*1024)
        if not raw.startswith(b'%PDF'): raise ValueError('invalid PDF header')
        reader=PdfReader(io.BytesIO(raw));record.update(pages=len(reader.pages),resolved_pdf_url=pdfurl,sha256=digest(raw),bytes=len(raw),file='papers/'+ident+'.pdf',status='downloaded')
        (ROOT/record['file']).write_bytes(raw)
        texts=ROOT/'paper-text';texts.mkdir(exist_ok=True)
        (texts/(ident+'.txt')).write_text('\n\n'.join((p.extract_text() or '') for p in reader.pages[:3]),encoding='utf-8')
        (ROOT/'metadata'/('arxiv-'+ident+'.html')).write_bytes(data)
        print('PAPER OK '+ident+' '+record['title'],flush=True)
    except Exception as e: record.update(status='failed',error=str(e));print('PAPER FAIL '+ident+' '+str(e),flush=True)
    return record
TEXT_EXT={'.py','.rs','.ts','.tsx','.js','.jsx','.mjs','.cjs','.md','.rst','.toml','.yml','.yaml','.json','.txt','.proto','.sql','.go','.java','.cs','.sh','.ps1','.ini','.cfg','.xml','.html','.css','.graphql'}
SKIP={'node_modules','.git','target','dist','build','vendor','venv','.venv','__pycache__','site-packages','FINAL_results','datasets','dataset','fixtures','test_data','testdata','screenshots','videos','images','assets','static'}
LOCK={'package-lock.json','yarn.lock','pnpm-lock.yaml','uv.lock','poetry.lock','Cargo.lock'}
def project(seed):
    repo,topic,focus=seed
    record={'repository':repo,'topic':topic,'focus':focus,'source_url':'https://github.com/'+repo,'collected_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'snapshot_kind':'curated text source snapshot'}
    try:
        raw,_=fetch('https://api.github.com/repos/'+repo,3*1024*1024);meta=json.loads(raw);actual=meta['full_name'];branch=meta['default_branch']
        rawc,_=fetch('https://api.github.com/repos/'+actual+'/commits/'+branch,3*1024*1024);commit=json.loads(rawc)['sha']
        record.update(repository=actual,branch=branch,commit=commit,license_spdx=(meta.get('license') or {}).get('spdx_id'),description=meta.get('description'),archived=meta.get('archived'),pushed_at=meta.get('pushed_at'))
        data,archiveurl=fetch('https://codeload.github.com/'+actual+'/zip/'+commit);z=zipfile.ZipFile(io.BytesIO(data));bad=z.testzip()
        if bad: raise ValueError('invalid upstream archive '+bad)
        kept=[];omitted=[];dest=io.BytesIO()
        with zipfile.ZipFile(dest,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as out:
            for info in z.infolist():
                if info.is_dir(): continue
                rel=Path(*Path(info.filename).parts[1:]);name=rel.name;licensefile=any(x in name.upper() for x in ['LICENSE','COPYING','NOTICE'])
                if ((set(rel.parts)&SKIP) or name in LOCK or info.file_size>1024*1024 or (rel.suffix.lower() not in TEXT_EXT and not licensefile and name not in {'Dockerfile','Makefile','Justfile'})):
                    omitted.append({'path':rel.as_posix(),'bytes':info.file_size});continue
                body=z.read(info.filename);out.writestr(info.filename,body);kept.append({'path':rel.as_posix(),'bytes':len(body)})
        curated=dest.getvalue();filename=actual.replace('/','--')+'-'+commit[:12]+'.zip';(ROOT/'project-snapshots'/filename).write_bytes(curated)
        reference=ROOT/'project-notes'/actual.replace('/','--');reference.mkdir(parents=True,exist_ok=True)
        readmes=sorted([i for i in z.infolist() if len(Path(i.filename).parts)==2 and Path(i.filename).name.lower().startswith('readme')],key=lambda i:len(i.filename))
        if readmes: (reference/'UPSTREAM_README.md').write_bytes(z.read(readmes[0].filename))
        licenses=[i for i in z.infolist() if len(Path(i.filename).parts)==2 and any(t in Path(i.filename).name.upper() for t in ['LICENSE','COPYING','NOTICE'])]
        for info in licenses: (reference/Path(info.filename).name).write_bytes(z.read(info.filename))
        (reference/'source-map.json').write_text(json.dumps({'kept':kept,'omitted':omitted},ensure_ascii=False,indent=2),encoding='utf-8')
        (reference/'repository.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2),encoding='utf-8')
        record.update(file='project-snapshots/'+filename,bytes=len(curated),sha256=digest(curated),status='downloaded',upstream_archive_url=archiveurl,upstream_archive_bytes=len(data),upstream_archive_sha256=digest(data),kept_files=len(kept),omitted_files=len(omitted),license_files=[Path(i.filename).name for i in licenses],notes_dir='project-notes/'+actual.replace('/','--'))
        print('PROJECT OK '+actual+' '+str(len(kept))+' files '+str(len(curated))+' bytes',flush=True)
    except Exception as e: record.update(status='failed',error=str(e));print('PROJECT FAIL '+repo+' '+str(e),flush=True)
    return record
def run(mode):
    for d in ['metadata','papers','project-snapshots','project-notes']: (ROOT/d).mkdir(exist_ok=True)
    config=json.loads((ROOT/'tools'/'research-seeds.json').read_text(encoding='utf-8'));key='papers' if mode=='papers' else 'projects';func=paper if mode=='papers' else project;results=[]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures=[pool.submit(func,s) for s in config[key]]
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())
            (ROOT/'metadata'/(mode+'-expanded.json')).write_text(json.dumps(sorted(results,key=lambda x:x.get('id',x.get('repository',''))),ensure_ascii=False,indent=2),encoding='utf-8')
    print('DONE '+mode+' '+str(sum(x['status']=='downloaded' for x in results))+'/'+str(len(results)),flush=True)
if __name__=='__main__':
    a=argparse.ArgumentParser();a.add_argument('mode',choices=['papers','projects']);run(a.parse_args().mode)
