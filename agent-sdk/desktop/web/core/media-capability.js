// Configuration and service verification are separate evidence layers.
(function(root){
 "use strict";
 const KEYS={image:["image_generation","media_image_generation"],audio:["text_to_speech","media_speech"]};
 function describe(capabilities,kind){
  const keys=KEYS[kind];if(!keys)throw new Error("Unknown media capability");
  const view=capabilities?.media_connections?.[keys[0]],legacy=capabilities?.constraints?.[keys[1]];
  if(view&&typeof view.configured==="boolean"){
   const configured=view.configured&&view.status!=="invalid_configuration";
   const invalid=view.status==="invalid_configuration";
   const label=invalid?"配置无效":configured?"已配置 · 未验证":"未配置";
   const model=typeof view.model==="string"?view.model:"";
   const reason=typeof view.reason==="string"?view.reason:"";
   const source=view.configuration_source==="legacy_image_environment"&&kind==="audio"?"沿用旧媒体连接；":"";
   return {configured,status:invalid?"invalid_configuration":configured?"configured":"not_configured",label,
    detail:configured?source+(model?"模型："+model+"；":"")+"连接未测试，服务未验证":reason||"请配置该能力的独立端点与凭据引用",model,legacy:false};
  }
  const known=typeof legacy==="boolean";
  return {configured:legacy===true,status:known?(legacy?"configured":"not_configured"):"unknown",
   label:known?(legacy?"已配置 · 未验证":"未配置"):"状态未知",
   detail:known?(legacy?"旧核心仅报告配置，连接与服务均未验证":"请配置该能力的端点与凭据"):"当前核心没有提供此能力状态",model:"",legacy:true};
 }
 const api={describe};root.OwoMediaCapability=api;if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
