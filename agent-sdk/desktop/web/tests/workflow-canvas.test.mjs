import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
function setup(){
 const listeners=new Map(),win={addEventListener:(key,fn)=>listeners.set(key,fn),removeEventListener:(key,fn)=>{if(listeners.get(key)===fn)listeners.delete(key);}};
 for(const file of ['workflow-document','workflow-editor','workflow-canvas'])new Function('window',readFileSync(new URL('../core/'+file+'.js',import.meta.url),'utf8'))(win);
 const fields={tools:{value:'unfinished tool'},catalog:{innerHTML:''}},root={innerHTML:'',ownerDocument:{elementFromPoint:()=>root.hit},contains:target=>target.owner===root,querySelector:selector=>selector==='svg'?svg:selector==='[name="tools"]'?fields.tools:selector==='.owo-wfc-add-tool'?fields.catalog:null,querySelectorAll:()=>[]};
 const svg={getBoundingClientRect:()=>({left:0,top:0,width:342,height:96})};
 const changes=[],messages=[],controller=win.OwoWorkflowCanvas.create({root,helper:win.OwoWorkflowEditor,onChange:(doc,message)=>changes.push({doc,message}),onMessage:message=>messages.push(message)});
 function target(attrs={},kind=''){
  const t={owner:root,attrs,getAttribute:key=>attrs[key]??null,setAttribute:(key,value)=>attrs[key]=value,closest:selector=>selector.split(',').some(part=>{part=part.trim();return part===kind||part.startsWith('[')&&Object.hasOwn(attrs,part.slice(1,-1));})?t:null};return t;
 }
 const click=t=>root.onclick({target:t});
 const original={template_id:'graph',name:'Graph',applicability:'fixture',roles:[{node_id:'node-a',role:'Input',assignee:'human'},{node_id:'node-b',role:'Draw',assignee:'agent',tools:['generate_image'],depends_on:[]}]};
 controller.setDocument(original);
 return {win,root,controller,original,listeners,fields,changes,messages,target,click};
}
test('port clicks, keyboard connection, edge deletion and cycles operate on actual JSON dependencies',()=>{
 const s=setup(),{controller:c,root:r,target:t,click}=s;
 r.onkeydown({key:'Enter',target:t({'data-output-port':'node-a'}),preventDefault(){}});
 r.onkeydown({key:'Enter',target:t({'data-input-port':'node-b'}),preventDefault(){}});
 assert.deepEqual(c.getDocument().roles[1].depends_on,['Input']);
 const frozen=JSON.stringify(c.getDocument());
 click(t({'data-output-port':'node-b'}));click(t({'data-input-port':'node-a'}));
 assert.equal(JSON.stringify(c.getDocument()),frozen);assert.match(s.messages.at(-1),/依赖环/);assert.match(r.innerHTML,/role="alert"/);
 click(t({'data-edge-from':'node-a','data-edge-to':'node-b'}));click(t({'data-wfc':'disconnect'}));
 assert.deepEqual(c.getDocument().roles[1].depends_on,[]);
});
test('node forms rename dependencies and preserve identity, lists and portable position',()=>{
 const s=setup(),{controller:c,root:r,target:t,click}=s;
 let doc=c.getDocument();doc.roles[1].depends_on=['Input'];doc.editor.nodes[0].position={x:20,y:120};c.setDocument(doc);
 click(t({'data-node-id':'node-a'}));
 const lines=values=>values.join(String.fromCharCode(10));
 const fields=Object.entries({role:'Brief',assignee:'human',worker:'',model:'',tools:lines(['read_file','write_file']),write_paths:lines(['images/','audio/']),capabilities:'custom',verify:'non_empty',handoff_contract:'Asset refs'}).map(([name,value])=>({name,value}));
 const form=t({},'.owo-wfc-form');form.querySelectorAll=()=>fields;r.onsubmit({target:form,preventDefault(){}});
 doc=c.getDocument();assert.equal(doc.roles[0].node_id,'node-a');assert.equal(doc.roles[0].role,'Brief');assert.deepEqual(doc.roles[1].depends_on,['Brief']);
 assert.deepEqual(doc.roles[0].tools,['read_file','write_file']);assert.deepEqual(doc.roles[0].write_paths,['images/','audio/']);assert.deepEqual(doc.editor.nodes[0].position,{x:20,y:120});assert.equal(s.original.roles[0].role,'Input');
});
test('add/remove and undo/redo restore graph and matching JSON without crossing workflow IDs',()=>{
 const {controller:c,target:t,click}=setup();click(t({'data-wfc':'add'}));
 const added=c.getDocument().roles.at(-1);assert.equal(c.getDocument().roles.length,3);assert.ok(added.node_id);
 click(t({'data-wfc':'remove'}));assert.equal(c.getDocument().roles.length,2);
 click(t({'data-wfc':'undo'}));assert.equal(c.getDocument().roles.at(-1).node_id,added.node_id);
 click(t({'data-wfc':'redo'}));assert.equal(c.getDocument().roles.length,2);
 c.setDocument({template_id:'different',roles:[{role:'Other'}]});click(t({'data-wfc':'undo'}));assert.equal(c.getDocument().template_id,'different');
});
test('pointer drag connects ports; node movement changes layout without changing execution fields',()=>{
 const {controller:c,root:r,target:t,listeners:l}=setup();r.hit=t({'data-input-port':'node-b'});
 r.onpointerdown({button:0,target:t({'data-output-port':'node-a'}),clientX:142,clientY:42,preventDefault(){}});
 l.get('pointermove')({clientX:200,clientY:42});l.get('pointerup')({clientX:200,clientY:42});assert.deepEqual(c.getDocument().roles[1].depends_on,['Input']);
 const roles=structuredClone(c.getDocument().roles);
 r.onpointerdown({button:0,target:t({'data-node-id':'node-a'}),clientX:20,clientY:20,preventDefault(){}});
 l.get('pointermove')({clientX:70,clientY:80});l.get('pointerup')({clientX:70,clientY:80});
 assert.deepEqual(c.getDocument().roles,roles);assert.ok(c.getDocument().editor.nodes.find(n=>n.id==='node-a').position.y>0);
});
test('invalid JSON disables mutations; disposal releases global pointer handlers',()=>{
 const s=setup();s.controller.invalidate('bad JSON');s.click(s.target({'data-wfc':'add'}));assert.equal(s.controller.getDocument(),null);assert.match(s.root.innerHTML,/编辑已停用/);
 assert.equal(s.controller.setDocument(s.original).ok,true);assert.equal(s.controller.getDocument().roles.length,2);s.controller.dispose();assert.equal(s.listeners.size,0);assert.equal(s.root.onclick,null);
});
test('catalog refresh preserves unsaved input and advertises only registered Worker tools',()=>{
 const s=setup();s.click(s.target({'data-node-id':'node-b'}));s.controller.setCatalog([{name:'read_file',worker_supported:true,status:'registered'},{name:'generate_image',worker_supported:true,status:'not_configured'},{name:'main_only',worker_supported:false,status:'unsupported'}]);
 assert.equal(s.fields.tools.value,'unfinished tool');assert.match(s.fields.catalog.innerHTML,/read_file/);assert.doesNotMatch(s.fields.catalog.innerHTML,/generate_image|main_only/);
});
test('imported cycles and missing dependencies remain visible and editable; labels are escaped',()=>{
 const s=setup(),doc={roles:[{node_id:'node-a',role:'<img onerror=bad>',depends_on:['B']},{node_id:'node-b',role:'B',depends_on:['<img onerror=bad>','missing']}]};
 const rendered=s.win.OwoWorkflowCanvas.renderGraphSvg(doc,s.win.OwoWorkflowEditor);
 assert.match(rendered,/存在环/);assert.match(rendered,/不存在.*missing/);assert.equal((rendered.match(/class="owo-wf-node owo-wfc-node/g)||[]).length,2);assert.doesNotMatch(rendered,/<img/);assert.match(rendered,/&lt;img/);
});

test('Escape outside a pending gesture leaves unsaved node form fields intact',()=>{
 const s=setup();s.click(s.target({'data-node-id':'node-b'}));
 s.root.innerHTML='editing unsaved form';s.root.onkeydown({key:'Escape',target:s.target({},'input')});
 assert.equal(s.root.innerHTML,'editing unsaved form');
 s.click(s.target({'data-output-port':'node-a'}));s.root.onkeydown({key:'Escape',target:s.target({},'svg')});
 assert.doesNotMatch(s.root.innerHTML,/选择目标节点的左侧端口/);assert.equal(s.controller.getDocument().roles.length,2);
});
