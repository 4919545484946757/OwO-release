// Upload planning only: the daemon remains the authority for ownership, bytes and model inputs.
(function(root){
  "use strict";
  // Current /session/{id}/attachments JSON body gate and turn image gate.
  var REQUEST_BYTES=32*1024*1024, IMAGE_BYTES=5*1024*1024;
  function plan(file){
    if(!file||typeof file.name!=="string"||!file.name||!Number.isSafeInteger(file.size)||file.size<0)
      throw new Error("附件名称或大小无效");
    var mime=typeof file.type==="string"&&file.type?file.type:"application/octet-stream";
    var image=/\.(png|jpe?g|webp|gif)$/i.test(file.name);
    if(/^image\//i.test(mime)&&!image)throw new Error("当前图片输入仅支持 PNG、JPEG、WebP、GIF；请先转换图片格式");
    if(image&&(file.size===0||file.size>IMAGE_BYTES))throw new Error("图片必须非空且单张不超过 5 MiB；尚未读取或上传文件");
    var encoded=4*Math.ceil(file.size/3);
    var overhead=new TextEncoder().encode(JSON.stringify({name:file.name,mime:mime,data_b64:""})).length;
    if(!Number.isSafeInteger(encoded)||encoded+overhead>REQUEST_BYTES)
      throw new Error("附件超过当前上传上限（编码后请求最多 32 MiB，原文件约 24 MiB）；尚未读取文件");
    return {image:image,mime:mime,requestBytes:encoded+overhead,encodedBytes:encoded};
  }
  function read(file){return new Promise(function(resolve,reject){
    var reader=new root.FileReader();reader.onload=function(){resolve(reader.result);};reader.onerror=function(){reject(reader.error||new Error("读取附件失败"));};reader.onabort=function(){reject(new Error("附件读取已取消"));};reader.readAsDataURL(file);
  });}
  async function upload(files,options){
    var session=String(options.sessionId||"");if(!session)throw new Error("请先选择会话");
    var current=options.isCurrent||function(){return true;};
    function stopped(started){if(options.onStopped)options.onStopped({requestStarted:started});}
    for(var file of Array.from(files||[])){
      if(!current()){stopped(false);break;}
      var started=false;
      try{
        var limits=plan(file); // Fail before FileReader allocates a full base64 string.
        var data=String(await (options.read||read)(file));
        if(!current()){stopped(false);break;}
        var comma=data.indexOf(",");
        if(comma<0||!/^data:[^,]*;base64$/i.test(data.slice(0,comma)))throw new Error("附件读取结果不是 base64 数据");
        var encoded=data.slice(comma+1);
        if(encoded.length!==limits.encodedBytes)throw new Error("读取后的附件大小与选择时不一致；未上传");
        started=true;
        var uploaded=await options.request('/session/'+encodeURIComponent(session)+'/attachments',{
          method:'POST',body:JSON.stringify({name:file.name,mime:limits.mime,data_b64:encoded})
        });
        if(!current()){stopped(true);break;}
        if(!uploaded||typeof uploaded.id!=="string"||!uploaded.id)throw new Error("服务端未返回有效的附件引用");
        if(options.onUploaded)options.onUploaded(uploaded,file);
      }catch(error){
        if(!current()){stopped(started);break;}
        if(options.onError)options.onError(file,error);
      }
    }
  }
  function turnInput(uploads){
    var result={attachments:[],content_parts:[]},seen=new Set();
    for(var upload of uploads||[]){
      if(!upload||typeof upload.id!=="string"||!upload.id)throw new Error("附件引用无效，请重新上传");
      if(seen.has(upload.id))continue;
      seen.add(upload.id);
      var part=upload.content_part;
      if(!part){result.attachments.push(upload.id);continue;}
      if(part.type==="text"){if(typeof part.text!=="string"||new TextEncoder().encode(part.text).length>32768)throw new Error("附加文本格式或大小无效");result.content_parts.push({type:"text",text:part.text});continue;}
      if(part.type==="audio"||part.type==="video")throw new Error("当前尚未配置音频/视频输入适配器；请先转写为文本或移除该附件");
      if(!["image","file"].includes(part.type)||!part.asset||part.asset.artifact_id!==upload.id||
         !/^input-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(upload.id)||
         !/^[0-9a-f]{64}$/.test(part.asset.sha256))throw new Error("服务端返回的附件资产引用无效，请重新上传");
      result.content_parts.push({type:part.type,asset:{artifact_id:upload.id,sha256:part.asset.sha256}});
    }
    if(result.attachments.length+result.content_parts.length>32)throw new Error("每次最多提交 32 个输入内容块");
    if(result.content_parts.filter(function(part){return part.type==="image";}).length>8)throw new Error("每次最多提交 8 张图片");
    return result;
  }
  var api={plan:plan,upload:upload,turnInput:turnInput,requestBytes:REQUEST_BYTES,imageBytes:IMAGE_BYTES};root.OwoAttachmentUpload=api;
  if(typeof module!=="undefined"&&module.exports)module.exports=api;
})(typeof window!=="undefined"?window:globalThis);
