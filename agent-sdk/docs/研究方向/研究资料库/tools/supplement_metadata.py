import concurrent.futures,json
import collect_sources as c
p=c.ROOT/'papers/manifest.json';records=json.loads(p.read_text(encoding='utf-8'))
def work(r):
    if not r.get('authors') and not r['id'].startswith('LIPIcs'):
        try:
            data,url=c.fetch(r['source_url'],4*1024*1024);page=data.decode('utf-8');r['authors']=c.tags(page,'citation_author')
            r['submitted']=c.tags(page,'citation_date');r['citation_metadata_url']=url;r['metadata_basis']='publisher metadata retrieved 2026-10-13; local PDF version in pdf_version'
            (c.ROOT/'metadata'/('arxiv-'+r['id']+'.html')).write_bytes(data)
            print('METADATA OK '+r['id'],flush=True)
        except Exception as e:r['metadata_error']=str(e);print('METADATA FAIL '+r['id']+' '+str(e),flush=True)
    if r['id'].startswith('LIPIcs'):
        r['authors']=['Hasen Özaytürk','Feza Buzluca'];r['metadata_basis']='authors and title verified on local PDF first page';r['doi']='10.4230/LIPIcs.ESEM.2026.18'
    return r
with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:records=list(pool.map(work,records))
p.write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')
bib=[]
for r in records:
    fields=['  title={'+r['title']+'}','  year={'+('20'+r['id'][:2] if r['id'][0].isdigit() else '2026')+'}','  url={'+r['source_url']+'}']
    if r.get('authors'):fields.insert(1,'  author={'+' and '.join(r['authors'])+'}')
    if r['id'][0].isdigit():fields+=['  eprint={'+r['id']+'}','  archivePrefix={arXiv}']
    if r.get('doi'):fields+=['  doi={'+r['doi']+'}']
    bib.append('@misc{agent_'+r['id'].replace('.','_')+',\n'+',\n'.join(fields)+'\n}')
(c.ROOT/'references.bib').write_text('\n\n'.join(bib)+'\n',encoding='utf-8')
print('AUTHOR METADATA',sum(bool(r.get('authors')) for r in records),len(records),flush=True)
