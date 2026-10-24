import fs from 'node:fs';
import vm from 'node:vm';
import path from 'node:path';
const root=process.cwd();
const ctx={};vm.createContext(ctx);vm.runInContext(fs.readFileSync(path.join(root,'agent-sdk/desktop/web/core/media-result.js'),'utf8'),ctx);
const boundary=fs.readFileSync(path.join(root,'agent-sdk/crates/owo-agent-core/src/agent/mod.rs'),'utf8').match(/const TOOL_PREVIEW_CHARS: usize = (\d+)/);
const cap=Number(boundary[1]);
for(const size of [32,1200,4096,512*1024+1]){
 const data={path:'outputs/audit.png',bytes:size,model:'audit-fixture'};
 if(size<=512*1024)data.data_url='data:image/png;base64,'+Buffer.alloc(size,1).toString('base64');
 const raw=JSON.stringify(data); const truncated=raw.length>cap?raw.slice(0,cap)+'\n[工具输出已截断]':raw;
 const full=ctx.OwoMediaResult.mediaFromToolResult({tool:'generate_image',ok:true,preview:raw});
 const actual=ctx.OwoMediaResult.mediaFromToolResult({tool:'generate_image',ok:true,preview:truncated});
 console.log(JSON.stringify({probe:'media-preview-boundary',bytes:size,cap,serialized:raw.length,fullParsed:!!full,truncatedParsed:!!actual,pathVisible:actual?.path||null}));
}
const src=fs.readFileSync(path.join(root,'agent-sdk/crates/owo-agent-server/src/workflow_draft_api.rs'),'utf8');
const resolver=src.slice(src.indexOf('fn resolve_draft_tools('),src.indexOf('#[derive(Debug, Default, Deserialize)]'));
console.log(JSON.stringify({probe:'draft-default-media-vocabulary',image:resolver.includes('"generate_image"'),speech:resolver.includes('"text_to_speech"'),note:'source membership check, not Rust execution'}));

const {createRequire}=await import('node:module');
const req=createRequire(import.meta.url);
const renderer=req(path.join(root,'agent-sdk/desktop/web/panels/workswarm/render.js'));
const dag=renderer.dagSvg([{task_id:'a',role:'a',worker:'agent',status:'Succeeded',depends_on:[]},{task_id:'b',role:'b',worker:'agent',status:'Pending',depends_on:['a']}],'Running');
console.log(JSON.stringify({probe:'team-dag-edge',x1:dag.match(/<line x1="([^"]+)/)?.[1],expectedX1:222,svgWidth:dag.match(/width="(\d+)"/)?.[1]}));
