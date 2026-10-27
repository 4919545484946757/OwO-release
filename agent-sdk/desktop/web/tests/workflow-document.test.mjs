import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const win={};new Function('window',readFileSync(new URL('../core/workflow-document.js',import.meta.url),'utf8'))(win);
const D=win.OwoWorkflowDocument;
function fixture(){return D.normalize({template_id:'pipeline',name:'Pipeline',extra:{keep:true},roles:[{role:'Input',assignee:'human',depends_on:[]},{role:'Draw',assignee:'agent',depends_on:['Input'],tools:['generate_image']},{role:'Review',assignee:'human',depends_on:['Draw']}]}).document;}
const id=(doc,name)=>doc.roles.find(node=>node.role===name).node_id;

test('legacy normalization assigns repeatable IDs and JSON round trip preserves metadata and unknown fields',()=>{
 const a=fixture(),b=fixture();assert.deepEqual(a.editor,b.editor);
 assert.deepEqual(D.normalize(JSON.parse(JSON.stringify(a))).document,a);
 assert.equal(a.extra.keep,true);assert.equal(new Set(a.editor.nodes.map(n=>n.id)).size,3);
});
test('rename keeps node identity, position and both inbound and outbound dependencies',()=>{
 const before=fixture(),draw=id(before,'Draw');
 const moved=D.moveNode(before,draw,{x:123.45,y:234.56}).document;
 const after=D.updateNode(moved,draw,{role:'Illustrate'}).document;
 assert.equal(id(after,'Illustrate'),draw);assert.deepEqual(after.editor.nodes.find(n=>n.id===draw).position,{x:123.5,y:234.6});
 assert.deepEqual(after.roles[1].depends_on,['Input']);assert.deepEqual(after.roles[2].depends_on,['Illustrate']);
 assert.equal(before.roles[1].role,'Draw');assert.equal(after.extra.keep,true);
});
test('adding and deleting a node keeps identities unique and removes only its edges',()=>{
 const before=fixture(),after=D.addNode(before,{role:'Voice',tools:['text_to_speech'],write_paths:['audio/'],model:'voice-model'}).document;
 assert.equal(after.roles.length,4);assert.deepEqual(after.roles[3].tools,['text_to_speech']);
 const linked=D.connect(after,id(after,'Voice'),id(after,'Review')).document;
 const removed=D.removeNode(linked,id(linked,'Draw')).document;
 assert.deepEqual(removed.roles.find(r=>r.role==='Review').depends_on,['Voice']);
 assert.equal(removed.editor.nodes.some(n=>n.id===id(before,'Draw')),false);
 assert.equal(id(before,'Review'),id(removed,'Review'));
});
test('connection edits and JSON refer to the same dependencies; cycles fail without mutation',()=>{
 const before=fixture(),frozen=JSON.stringify(before);
 const cycle=D.connect(before,id(before,'Review'),id(before,'Input'));
 assert.equal(cycle.ok,false);assert.match(cycle.error,/依赖环/);assert.equal(JSON.stringify(before),frozen);
 assert.equal(D.connect(before,id(before,'Draw'),id(before,'Draw')).ok,false);
 assert.equal(D.connect(before,'unknown',id(before,'Draw')).ok,false);
 const disconnected=D.disconnect(before,id(before,'Draw'),id(before,'Review')).document;
 assert.deepEqual(disconnected.roles[2].depends_on,[]);
 const reconnected=D.connect(disconnected,id(before,'Input'),id(before,'Review')).document;
 assert.deepEqual(reconnected.roles[2].depends_on,['Input']);
 const projected=D.graphTemplate(reconnected);assert.deepEqual(projected.template.roles[2].depends_on,[id(before,'Input')]);
});
test('node moves change layout only and reject non-finite or out of range positions',()=>{
 const before=fixture(),moved=D.moveNode(before,id(before,'Draw'),{x:900,y:10}).document;
 assert.deepEqual(moved.roles,before.roles);assert.equal(before.editor.nodes[1].position,undefined);
 for(const pos of [{x:NaN,y:0},{x:Infinity,y:0},{x:-1,y:0},{x:100001,y:0},{x:'1',y:0}]) assert.equal(D.moveNode(before,id(before,'Draw'),pos).ok,false);
});
test('node forms reject invalid lists and protected ID fields and preserve capability semantics',()=>{
 const before=fixture(),draw=id(before,'Draw');
 for(const patch of [{tools:'read_file'},{tools:[7]},{id:'replacement'},{depends_on:['Review']},{assignee:'root'},{role:'Input'}]) assert.equal(D.updateNode(before,draw,patch).ok,false);
 const changed=D.updateNode(before,draw,{tools:['write_file','write_file',' read_file '],write_paths:['images/'],model:' model-x ',verify:'non_empty',handoff_contract:'Asset refs',capabilities:['custom']}).document;
 assert.deepEqual(changed.roles[1].tools,['write_file','read_file']);assert.equal(changed.roles[1].model,'model-x');
 assert.equal(id(changed,'Draw'),draw);assert.deepEqual(changed.roles[1].depends_on,['Input']);
});
test('missing dependencies and cycles in imported JSON remain visible instead of being silently discarded',()=>{
 const before=fixture();before.roles[0].depends_on=['Review','missing'];
 const result=D.normalize(before);assert.equal(result.ok,true);assert.match(result.problems.join('\n'),/不存在.*missing/);assert.match(result.problems.join('\n'),/存在环/);
 assert.deepEqual(result.document.roles[0].depends_on,['Review','missing']);
});
test('reserved JavaScript property names remain ordinary role labels',()=>{
 const before=D.normalize({roles:[{role:'__proto__'},{role:'constructor',depends_on:['__proto__']}]}).document;
 const renamed=D.updateNode(before,id(before,'__proto__'),{role:'toString'});assert.equal(renamed.ok,true);
 assert.deepEqual(renamed.document.roles[1].depends_on,['toString']);assert.deepEqual(D.validate(renamed.document),[]);
});
test('duplicate mappings, unsupported editor versions and invalid metadata fail explicitly',()=>{
 const before=fixture();before.roles[1].node_id=before.roles[0].node_id;assert.equal(D.normalize(before).ok,false);
 assert.equal(D.normalize({roles:[{role:'A'}],editor:{version:2,nodes:[]}}).ok,false);
 assert.equal(D.normalize({roles:[{role:'A'}],editor:{version:1,nodes:[{id:'node-a',role:'missing'}]}}).ok,false);
 assert.equal(D.normalize({roles:[{role:' A '}]}).ok,false);
});

test('JSON rename preserves authoritative IDs and repairs only dangling references to the same renamed node',()=>{
 const before=fixture(),json=JSON.parse(JSON.stringify(before));json.roles[1].role='Illustrate';
 const changed=D.reconcile(before,json);assert.equal(changed.ok,true);
 assert.equal(id(changed.document,'Illustrate'),id(before,'Draw'));assert.deepEqual(changed.document.roles[2].depends_on,['Illustrate']);
 assert.deepEqual(changed.problems,[]);assert.equal(before.roles[1].role,'Draw');
 const reused=JSON.parse(JSON.stringify(json));reused.roles.push({node_id:'node-new',role:'Draw'});
 assert.deepEqual(D.reconcile(before,reused).document.roles[2].depends_on,['Draw']);
});
test('layout metadata refers only to stable IDs; invalid editor values fail and legacy label metadata migrates',()=>{
 for(const editor of [false,0,''])assert.equal(D.normalize({roles:[{role:'A'}],editor}).ok,false);
 const migrated=D.normalize({roles:[{role:'A'}],editor:{version:1,nodes:[{id:'node-existing',role:'A',position:{x:10,y:20}}]}});
 assert.equal(migrated.ok,true);assert.equal(migrated.document.roles[0].node_id,'node-existing');
 assert.deepEqual(migrated.document.editor.nodes,[{id:'node-existing',position:{x:10,y:20}}]);
 const tooMany={roles:Array.from({length:13},(_,i)=>({role:'A'+i}))};assert.equal(D.normalize(tooMany).ok,false);
});

test('JSON deletion prunes only layout entries of removed known nodes, leaving unknown IDs as errors',()=>{
 const before=fixture(),json=JSON.parse(JSON.stringify(before));json.roles.splice(1,1);
 const changed=D.reconcile(before,json);assert.equal(changed.ok,true);
 assert.equal(changed.document.editor.nodes.some(n=>n.id===id(before,'Draw')),false);
 assert.match(changed.problems.join(' '),/不存在.*Draw/);
 json.editor.nodes.push({id:'node-unknown'});assert.equal(D.reconcile(before,json).ok,false);
 assert.equal(D.reconcile(before,undefined).ok,false);
});
