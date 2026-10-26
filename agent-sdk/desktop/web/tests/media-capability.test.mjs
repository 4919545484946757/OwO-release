import test from "node:test";
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
const require=createRequire(import.meta.url),M=require("../core/media-capability.js");
const configured=(extra={})=>({status:"configured",configured:true,connected:null,verified:null,last_verified_at:null,model:"model-a",...extra});
test("image and speech configuration remain independent",()=>{
 const caps={media_connections:{image_generation:configured(),text_to_speech:{status:"not_configured",configured:false,reason:"missing speech key"}}};
 assert.equal(M.describe(caps,"image").label,"已配置 · 未验证");assert.equal(M.describe(caps,"audio").configured,false);assert.equal(M.describe(caps,"audio").detail,"missing speech key");
});
test("configuration never promotes connected or verified state",()=>{
 for(const attrs of [{},{verified:true},{connected:true},{last_verified_at:"2026-10-26T00:00:00Z"}]){
  const info=M.describe({media_connections:{image_generation:configured(attrs)}},"image");assert.equal(info.label,"已配置 · 未验证");assert.match(info.detail,/连接未测试，服务未验证/);
 }
});
test("legacy booleans indicate only configuration and missing data remains unknown",()=>{
 assert.equal(M.describe({constraints:{media_image_generation:true}},"image").label,"已配置 · 未验证");
 assert.equal(M.describe({constraints:{media_speech:false}},"audio").label,"未配置");
 for(const caps of [null,{}, {constraints:{media_speech:"true"}}])assert.equal(M.describe(caps,"audio").status,"unknown");
});
test("invalid endpoint state cannot be mislabeled as configured",()=>{
 const info=M.describe({media_connections:{image_generation:configured({status:"invalid_configuration",reason:"invalid URL"})}},"image");assert.equal(info.configured,false);assert.equal(info.label,"配置无效");assert.equal(info.detail,"invalid URL");
});
test("legacy speech fallback is explicit without claiming service availability",()=>{
 const info=M.describe({media_connections:{text_to_speech:configured({configuration_source:"legacy_image_environment"})}},"audio");assert.match(info.detail,/沿用旧媒体连接/);assert.doesNotMatch(info.label,/可用|已验证/);
});
const app=readFileSync(new URL("../app.js",import.meta.url),"utf8");
const start=app.indexOf("function mediaConnectionInfo("),end=app.indexOf("// 媒体入口：",start);
const escape=text=>String(text).replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
test("actual model menu hint renders separate states, not a shared ready flag",()=>{
 const context={state:{capabilities:{constraints:{media_image_generation:true,media_speech:false}}},window:{OwoMediaCapability:M},esc:escape};runInNewContext(app.slice(start,end)+"\nthis.hint=mediaHintHtml;",context);
 assert.match(context.hint(),/文生图：已配置 · 未验证/);assert.match(context.hint(),/语音：未配置/);assert.doesNotMatch(context.hint(),/可用/);
});
test("actual media menu escapes model and reason metadata, and inserts instructions without starting a request",()=>{
 const source=app.slice(app.indexOf("function openMediaMenu()"),app.indexOf("\nfunction ",app.indexOf("function openMediaMenu()")+15));let html; const attrs={};
 const context={state:{capabilities:{media_connections:{image_generation:configured({model:'<img src=x onerror=alert(1)>'}),text_to_speech:{configured:false,status:"not_configured",reason:'<script>bad</script>'}}}},window:{OwoMediaCapability:M},esc:escape,currentSessionMode:()=>"agent",$:()=>({setAttribute:(key,value)=>{attrs[key]=value;}}),openComposerMenu:(_button,markup)=>{html=markup;}};
 runInNewContext(app.slice(start,end)+source+"\nthis.menu=openMediaMenu;",context);context.menu();assert(!html.includes("<img"));assert(!html.includes("<script>"));assert.match(html,/&lt;img/);assert.match(html,/&lt;script&gt;/);assert.match(html,/发送后仍按当前权限档位审批/);assert.equal(attrs["aria-expanded"],"true");
});
