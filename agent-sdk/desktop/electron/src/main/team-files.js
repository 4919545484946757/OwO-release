"use strict";
const fs=require('node:fs/promises'),path=require('node:path');
const OPENABLE=new Set(['.txt','.md','.pdf','.doc','.docx','.xls','.xlsx','.ppt','.pptx','.html','.htm','.csv','.json','.png','.jpg','.jpeg','.webp','.gif','.svg','.mp3','.mp4','.wav']);
function relativePath(value) {
  if(typeof value!=='string'||!value||value.length>2000||/[\0:]/.test(value)||/^[\\/]/.test(value))throw Error('文件路径无效');
  const parts=value.replace(/\\/g,'/').split('/');
  if(parts.some(p=>!p||p==='.'||p==='..'))throw Error('文件路径无效');
  return parts.join('/');
}
function inside(root,target) {const rel=path.relative(root,target);return !path.isAbsolute(rel)&&rel!=='..'&&!rel.startsWith('..'+path.sep);}
function planFiles(plan) {
  const found=[];
  for(const task of plan?.tasks||[]) {
    found.push(...(Array.isArray(task.write_paths)?task.write_paths:[]));
    for(const req of task.verification?.requirements||[])if(req.scope?.kind==='workspace_paths')found.push(...(req.scope.relative_paths||[]));
  }
  return found;
}
function createTeamFiles({get,open,reveal}) {
  async function describe(teamId) {
    if(typeof teamId!=='string'||!/^team-[a-zA-Z0-9_-]{1,100}$/.test(teamId))throw Error('任务标识无效');
    const snapshot=await get('/teams/'+encodeURIComponent(teamId));
    const team=snapshot.team;
    if(!team||team.team_id!==teamId||!team.project_space_id)throw Error('找不到任务');
    const base='/projects/'+encodeURIComponent(team.project_space_id);
    const [binding,artifacts,changes]=await Promise.all([get(base+'/workspace'),get(base+'/artifacts'),get(base+'/workspace/changes').catch(()=>({changed_files:[]}))]);
    const workspace=binding.workspace||binding;
    if(!workspace?.root)throw Error('这个任务没有绑定文件目录');
    const root=await fs.realpath(workspace.root);
    const allowed=(workspace.write_allowed_paths||[]).map(value=>{try{return value==='.'?root:path.resolve(root,relativePath(value));}catch{return null;}}).filter(Boolean);
    const leads=(artifacts.artifacts||[]).filter(a=>['lead','planner'].includes(a.kind)&&String(a.artifact_id).startsWith(teamId+':')).sort((a,b)=>b.version-a.version);
    let plan=null,planError='';
    if(leads.length)try {const content=await get('/artifacts/'+encodeURIComponent(leads[0].artifact_id)+'/content');plan=JSON.parse(content.content);}catch {planError='任务计划暂时无法读取，已显示可核对的变更文件。';}
    const candidates=[...planFiles(plan),...(changes.changed_files||[]),...(changes.records||[]).flatMap(r=>r.changed_files||[])];
    const names=new Set();for(const value of candidates.slice(0,1000))try {names.add(relativePath(value));}catch {}
    const files=[];
    for(const relative of [...names].slice(0,200)) {
      const target=path.resolve(root,relative);
      if(!inside(root,target)||!allowed.some(scope=>inside(scope,target)))continue;
      try {
        const canonical=await fs.realpath(target);
        if(!inside(root,canonical)||!allowed.some(scope=>inside(scope,canonical)))continue;
        const stat=await fs.stat(canonical);if(!stat.isFile())continue;
        files.push({relativePath:relative,name:path.basename(canonical),size:stat.size,modifiedAt:stat.mtime.toISOString(),available:true,canOpen:OPENABLE.has(path.extname(canonical).toLowerCase())});
      }catch(error) {if(error.code==='ENOENT')files.push({relativePath:relative,name:path.basename(target),available:false,canOpen:false});else throw error;}
    }
    const title=plan?.brief?.goal||(plan?.tasks||[]).flatMap(t=>t.requirement_quotes||[])[0]||'任务成果';
    return {teamId,status:team.status,title:String(title).slice(0,240),root,allowedPaths:allowed,files,notice:planError};
  }
  async function act({teamId,relativePath:relative,action}) {
    if(!['open','reveal','folder'].includes(action))throw Error('文件操作无效');
    const current=await describe(teamId);
    if(action==='folder') {const error=await open(current.root);if(error)throw Error(error);return {ok:true};}
    const requested=relativePath(relative);
    const file=current.files.find(f=>f.relativePath===requested&&f.available);
    if(!file)throw Error('文件已不存在或不属于这个任务');
    const canonical=await fs.realpath(path.resolve(current.root,requested));
    if(!inside(current.root,canonical)||!current.allowedPaths.some(scope=>inside(scope,canonical)))throw Error('文件已移出任务目录');
    const stat=await fs.stat(canonical);if(!stat.isFile())throw Error('文件已不存在');
    if(action==='reveal') {reveal(canonical);return {ok:true};}
    if(!file.canOpen||!OPENABLE.has(path.extname(canonical).toLowerCase()))throw Error('这类文件请在文件夹中查看');
    const error=await open(canonical);if(error)throw Error(error);
    return {ok:true};
  }
  return {describe,act};
}
module.exports={createTeamFiles,relativePath,planFiles,inside};
