param(
    [string]$SessionRoot='',
    [string]$OutputDirectory=''
)
$ErrorActionPreference='Stop'
$utf8=[Text.UTF8Encoding]::new($false)
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SessionRoot) { $SessionRoot=Join-Path $repo 'build/quality-review-20261029' }
if (-not $OutputDirectory) { $OutputDirectory=Join-Path $repo 'docs/quality-review/2026-10-29' }
$SessionRoot=[IO.Path]::GetFullPath($SessionRoot); $OutputDirectory=[IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath (Join-Path $SessionRoot '.owo-dev-session'))) { throw 'Missing isolated test-session marker.' }
$runs=@([IO.File]::ReadLines((Join-Path $SessionRoot 'rounds.jsonl'),[Text.Encoding]::UTF8) | ForEach-Object { $_ | ConvertFrom-Json })
$issues=@{}; $observations=0; $testKeys=[Collections.Generic.HashSet[string]]::new()
function Fingerprint([string]$Text) {
    $digest=[Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($digest.ComputeHash($utf8.GetBytes($Text)))).Replace('-','').ToLowerInvariant().Substring(0,12) } finally { $digest.Dispose() }
}
function Definition($Test) {
    $consumed=if ($null -ne $Test.consumed) { [int]$Test.consumed } else { $Test.input.Length }
    return ([ordered]@{input=$Test.input;expected=@($Test.expected);segmentation=$Test.segmentation;context=$Test.context;previous=$Test.previous;correction=$Test.correction;valid=$Test.valid;consumed=$consumed} | ConvertTo-Json -Compress -Depth 10)
}
function Add-Issue($Case,[string]$Kind,[string]$Priority,$Run) {
    $definition=Definition $Case.test
    $key=Fingerprint ($definition+'|'+$Kind)
    if (-not $issues.ContainsKey($key)) {
        $issues[$key]=[ordered]@{id=('Q-'+$key);priority=$Priority;kind=$Kind;status='待批阅';notes='';input=$Case.input;expected=@($Case.expected);test=$Case.test;tags=@($Case.tags);observations=[Collections.Generic.List[object]]::new()}
    }
    $issues[$key].tags=@(@($issues[$key].tags)+@($Case.tags) | Sort-Object -Unique)
    $issues[$key].observations.Add([ordered]@{round=$Run.round;backend=$Run.backend;case_id=$Case.id;report=$Run.report;page_size=$Run.page_size;partition=$Run.partition;typing=$Run.typing;correction=$Run.correction;base_rank=$Case.base_rank;final_rank=$Case.final_first_page_rank;first_base=$Case.base_candidates[0].text;first_final=$Case.final_first_page[0].text;preview=$Case.preview;scan_truncated=$Case.base_scan_truncated;base_candidates=@($Case.base_candidates | Select-Object -First 7);final_candidates=@($Case.final_first_page);expansion_evidence=$Case.expansion_evidence;runtime_error=$Case.error;model_failed=$Case.model_failed;base_ms=$Case.timing_ms.base_request;settled_ms=$Case.timing_ms.settled_request})
}
foreach ($run in $runs) {
    $path=Join-Path $run.report 'cases.jsonl'
    if (-not $run.summary.complete -or -not (Test-Path -LiteralPath $path)) { throw 'Incomplete run; refusing to present a completed review.' }
    foreach ($line in [IO.File]::ReadLines($path,[Text.Encoding]::UTF8)) {
        $case=$line | ConvertFrom-Json
        if ($case.record -ne 'case') { continue }
        $observations++; [void]$testKeys.Add((Definition $case.test))
        if ($case.runtime_error) { Add-Issue $case '服务或协议错误' 'P0' $run }
        if ($case.model_failed) { Add-Issue $case '模型细化失败' 'P1' $run }
        if ($case.expansion_checked -and -not $case.expansion_pass) { Add-Issue $case '分页与展开结果不一致' 'P1' $run }
        if (-not $case.parse_validity_pass) { Add-Issue $case '可解析性与标注不一致' 'P1' $run }
        if ($case.expected_segmentation -and $case.preview -cne $case.expected_segmentation) { Add-Issue $case '拼音预览与期望分割不同' 'P2' $run }
        if (@($case.expected).Count) {
            if ($null -eq $case.base_rank -and $null -eq $case.final_first_page_rank) {
                if ($case.base_scan_truncated) { Add-Issue $case '扫描范围内未命中（需加深复核）' 'P2' $run }
                else { Add-Issue $case '可见候选池未命中（不等于词典缺词）' 'P1' $run }
            } elseif ($null -eq $case.base_rank) { Add-Issue $case '基础候选未命中、模型首页已补救' 'P2' $run }
            elseif ($case.base_rank -gt $run.page_size -and $null -eq $case.final_first_page_rank) { Add-Issue $case '目标候选在首屏之外' 'P2' $run }
            if ($case.model_applied -and $case.base_rank -eq 1 -and $case.final_first_page_rank -ne 1) { Add-Issue $case '模型改变了原本命中的首选（歧义待评）' 'P2' $run }
            elseif (($case.base_rank -gt 1) -or ($case.final_first_page_rank -gt 1)) { Add-Issue $case '首候选与目标不同（同音歧义待评）' 'P3' $run }
        }
        if ($case.timing_ms.base_request -gt 50) { Add-Issue $case '基础请求超过50毫秒（需重复测量）' 'P2' $run }
    }
}
$all=@($issues.Values | ForEach-Object { [pscustomobject]$_ } | Sort-Object priority,kind,input,id)
[void][IO.Directory]::CreateDirectory($OutputDirectory)
$jsonLines=@($all | ForEach-Object { $_ | ConvertTo-Json -Depth 20 -Compress })
[IO.File]::WriteAllLines((Join-Path $OutputDirectory 'issues.jsonl'),$jsonLines,$utf8)
$summary=[ordered]@{generated_utc=[DateTime]::UtcNow.ToString('o');runs=$runs.Count;case_observations=$observations;distinct_test_definitions=$testKeys.Count;issue_count=$all.Count;priorities=@($all | Group-Object priority | ForEach-Object { @{priority=$_.Name;count=$_.Count} });classes=@($all | Group-Object kind | ForEach-Object { @{kind=$_.Name;count=$_.Count} });rounds=@($runs | ForEach-Object { [ordered]@{round=$_.round;backend=$_.backend;page_size=$_.page_size;partition=$_.partition;typing=$_.typing;correction=$_.correction;report=$_.report;metrics=$_.summary.metrics;timing=$_.summary.timing_ms} })}
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'summary.json'),($summary | ConvertTo-Json -Depth 20),$utf8)
function Markdown([object]$Value) { return ([string]$Value).Replace('|','\|').Replace("`r",' ').Replace("`n",' ') }
$markdown=[Collections.Generic.List[string]]::new()
$markdown.Add('# 拼音候选问题清单（待人工批阅）')
$markdown.Add('')
$markdown.Add(('已完成 '+$runs.Count+' 个后端/配置轮次，'+$observations+' 条用例观测，'+$testKeys.Count+' 个独立测试定义；自动汇总 '+$all.Count+' 个待批阅条目。'))
$markdown.Add('')
$markdown.Add('自动筛出不等于确认缺陷。未命中仅指本轮可见候选与扫描预算；同音歧义、标注合理性、人工可接受性需要逐项判断。词典抽样自一致性不能当作整体输入准确率。')
$markdown.Add('')
$markdown.Add('建议在 [交互批阅页](issues.html) 筛选、填写判定及备注，并下载批阅 JSON。生成文件会随后续轮次更新；不要直接在这些生成文件里保存唯一一份人工意见。')
$markdown.Add('')
$markdown.Add('## 轮次结果')
$markdown.Add('')
$markdown.Add('|轮次|后端|用例|基础首选|基础首屏|基础Top-K|最终首选|最终首屏|展开差异|运行错误|')
$markdown.Add('|---|---|---:|---:|---:|---:|---:|---:|---:|---:|')
foreach ($run in $runs) { $m=$run.summary.metrics; $markdown.Add('|'+$run.round+'|'+$run.backend+'|'+$m.cases+'|'+$m.base_top1+'|'+$m.base_first_page+'|'+$m.base_topk+'|'+$m.final_top1+'|'+$m.final_first_page+'|'+$m.expansion_failed+'|'+$m.errors+'|') }
$markdown.Add(''); $markdown.Add('比例为0–1；这里只列每轮结果，不把不同来源、变体、配置重复观测混算为代表性总体准确率。')
$markdown.Add(''); $markdown.Add('## 优先批阅条目（前150项，完整清单见HTML/JSONL）'); $markdown.Add('')
$markdown.Add('|建议优先度|问题ID|目标文字|输入|实际首选示例|类型|观测数|人工判定|')
$markdown.Add('|---|---|---|---|---|---|---:|---|')
foreach ($issue in @($all | Select-Object -First 150)) { $first=$issue.observations[0]; $markdown.Add('|'+$issue.priority+'|'+$issue.id+'|'+(Markdown ($issue.expected -join ' / '))+'|'+(Markdown $issue.input)+'|'+(Markdown $first.first_final)+'|'+$issue.kind+'|'+$issue.observations.Count+'|待批阅|') }
$markdown.Add(''); $markdown.Add('P0/P1/P2/P3仅表示建议复核顺序，不代表已确认严重程度。消费范围、分区编号和分页证据请展开HTML中的详细观测；旧轮次没有差异证据时需看后续复核轮次。')
[IO.File]::WriteAllLines((Join-Path $OutputDirectory 'issues.md'),$markdown,$utf8)
$payload=(@{summary=$summary;issues=$all} | ConvertTo-Json -Depth 25 -Compress).Replace('<','\u003c').Replace('>','\u003e').Replace('&','\u0026')
$html=@'
<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>OwO 拼音候选人工批阅</title>
<style>:root{color-scheme:light dark;font:14px/1.6 system-ui,"Microsoft YaHei",sans-serif}body{margin:24px;background:light-dark(#f4f5f7,#171a20);color:light-dark(#20242a,#e6e8ed)}h1{font-size:25px;font-weight:500}.note{max-width:1100px;color:light-dark(#515967,#b9c1cd)}.tools{display:flex;gap:8px;flex-wrap:wrap;margin:16px 0}input,select,button,textarea{font:inherit;border:1px solid #8893a055;border-radius:6px;padding:7px;background:light-dark(white,#242a34);color:inherit}input{min-width:230px}button{cursor:pointer}.table{overflow:auto;border-radius:10px;border:1px solid #8893a044;background:light-dark(white,#20252e)}table{border-collapse:collapse;width:100%;min-width:1050px}th,td{padding:10px;vertical-align:top;text-align:left;border-bottom:1px solid #8893a033}th{background:light-dark(#e9edf3,#2c3440);font-weight:500}code{overflow-wrap:anywhere;white-space:pre-wrap}.target{min-width:130px;max-width:250px}.details{min-width:230px;max-width:430px}textarea{display:block;width:210px;min-height:45px;resize:vertical}.small{font-size:12px;opacity:.7}.priority{white-space:nowrap}.P0,.P1{color:light-dark(#a62a22,#ff9b8e)}.P2{color:light-dark(#885900,#e5c477)}details{margin-top:6px}pre{max-width:600px;max-height:480px;overflow:auto;font:12px/1.5 ui-monospace,monospace;background:light-dark(#f2f4f7,#12161d);padding:10px}.pager{display:flex;gap:12px;align-items:center;margin:12px 0}.badge{display:inline-block;margin-right:5px;padding:0 5px;background:#8893a020;border-radius:4px}</style>
<h1>OwO 拼音候选人工批阅</h1><p id="stats"></p><p class="note">这里是自动筛出的待复核项目，不是已确认缺陷。目标不是首选可能只是合法同音歧义；候选未命中不等于词典缺词。请结合上下文、消费范围和分页证据判定。数据均为合成语料或已有词典抽样。</p>
<div class="tools"><input id="search" placeholder="搜索拼音、目标、实际候选或标签"><select id="priority"><option value="">所有优先度</option></select><select id="kind"><option value="">所有类型</option></select><select id="status"><option value="">所有人工状态</option></select><button id="save">下载人工批阅JSON</button><button id="load">导入批阅JSON</button><input id="file" type="file" accept=".json" hidden><button id="reset">重置筛选</button></div>
<p class="small">本页不运行输入法、不联网。浏览器本地保存可能受限，离开前请下载JSON备份；刷新后的新问题可导入旧批阅结果。源JSONL报告路径见每条详细观测。</p>
<div class="pager"><button id="prev">上一页</button><span id="page"></span><button id="next">下一页</button><select id="size"><option>25</option><option selected>50</option><option>100</option></select></div>
<div class="table"><table><thead><tr><th>优先度 / 类型</th><th>目标 / 输入</th><th>实际观测</th><th>人工批阅</th></tr></thead><tbody id="rows"></tbody></table></div>
<script type="application/json" id="data">__DATA__</script><script>
'use strict';const data=JSON.parse(document.getElementById('data').textContent);const statuses=['待批阅','确认问题','可接受歧义','标注需修正','需要复现','不复现'];let reviews={},page=0;const storage='owo-quality-review-20261029';try{reviews=JSON.parse(localStorage.getItem(storage)||'{}')}catch{};
const $=id=>document.getElementById(id);function node(tag,text,cls){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;if(cls)e.className=cls;return e}function persist(){try{localStorage.setItem(storage,JSON.stringify(reviews))}catch{}}
function state(issue){return reviews[issue.id]||{status:'待批阅',notes:''}};function options(id,values){for(const value of values){const e=node('option',value);e.value=value;$(id).append(e)}};options('priority',[...new Set(data.issues.map(i=>i.priority))]);options('kind',[...new Set(data.issues.map(i=>i.kind))]);options('status',statuses);
$('stats').textContent=`${data.summary.runs} 个后端/配置轮次 · ${data.summary.case_observations} 条观测 · ${data.summary.distinct_test_definitions} 个独立测试定义 · ${data.summary.issue_count} 个去重待批阅项`;
function filtered(){const q=$('search').value.trim().toLowerCase();return data.issues.filter(i=>(!$('priority').value||i.priority===$('priority').value)&&(!$('kind').value||i.kind===$('kind').value)&&(!$('status').value||state(i).status===$('status').value)&&(!q||[i.input,i.expected.join(' '),i.tags.join(' '),i.kind,...i.observations.map(o=>o.first_final||'')].join(' ').toLowerCase().includes(q)))}
function render(){const all=filtered(),size=+$('size').value;page=Math.max(0,Math.min(page,Math.ceil(all.length/size)-1));$('rows').replaceChildren();$('page').textContent=`筛选 ${all.length} 项 · 第 ${page+1} / ${Math.max(1,Math.ceil(all.length/size))} 页`;$('prev').disabled=page===0;$('next').disabled=(page+1)*size>=all.length;for(const issue of all.slice(page*size,(page+1)*size)){const tr=node('tr'),a=node('td',undefined,'priority '+issue.priority);a.append(node('div',issue.priority),node('div',issue.kind),node('div',issue.id,'small'));const b=node('td',undefined,'target');b.append(node('div',issue.expected.join(' / ')),node('code',issue.input));for(const tag of issue.tags)b.append(node('span',tag,'small badge'));const c=node('td',undefined,'details'),o=issue.observations[0];c.append(node('div','实际首选：'+(o.first_final||'（无）')),node('div',`基础排名 ${o.base_rank??'未命中'} / 最终首页 ${o.final_rank??'未命中'} · ${issue.observations.length} 条观测`,'small'));const d=node('details');d.append(node('summary','查看配置、候选与差异证据'));d.append(node('pre',JSON.stringify({test:issue.test,observations:issue.observations},null,2)));c.append(d);const e=node('td'),select=node('select');for(const status of statuses){const option=node('option',status);option.value=status;select.append(option)}select.value=state(issue).status;const notes=node('textarea');notes.placeholder='人工判断依据 / 复现备注';notes.value=state(issue).notes;select.onchange=()=>{reviews[issue.id]={status:select.value,notes:notes.value};persist()};notes.oninput=()=>{reviews[issue.id]={status:select.value,notes:notes.value};persist()};e.append(select,notes);tr.append(a,b,c,e);$('rows').append(tr)}}
for(const id of ['search','priority','kind','status','size'])$(id).addEventListener(id==='search'?'input':'change',()=>{page=0;render()});$('prev').onclick=()=>{page--;render()};$('next').onclick=()=>{page++;render()};$('reset').onclick=()=>{for(const id of ['search','priority','kind','status'])$(id).value='';page=0;render()};$('save').onclick=()=>{const result={schema_version:1,report:'OwO quality review 2026-10-29',saved_at:new Date().toISOString(),reviews:data.issues.map(i=>({id:i.id,input:i.input,expected:i.expected,...state(i)}))};const url=URL.createObjectURL(new Blob([JSON.stringify(result,null,2)],{type:'application/json;charset=utf-8'}));const link=node('a');link.href=url;link.download='owo-quality-human-review.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000)};$('load').onclick=()=>$('file').click();$('file').onchange=async()=>{try{const file=$('file').files[0];if(!file||file.size>16*1024*1024)throw Error('文件过大或未选择');const input=JSON.parse(await file.text());if(input.schema_version!==1||!Array.isArray(input.reviews))throw Error('不是批阅JSON');const ids=new Set(data.issues.map(i=>i.id));let count=0;for(const item of input.reviews)if(ids.has(item.id)&&statuses.includes(item.status)&&typeof item.notes==='string'&&item.notes.length<=5000){reviews[item.id]={status:item.status,notes:item.notes};count++}persist();render();alert(`已导入 ${count} 项；未匹配的旧问题没有覆盖新问题`)}catch(e){alert('导入失败：'+e.message)}};render();
</script></html>
'@
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'issues.html'),$html.Replace('__DATA__',$payload),$utf8)
Write-Output ([ordered]@{runs=$runs.Count;observations=$observations;distinct_tests=$testKeys.Count;issues=$all.Count;output=$OutputDirectory} | ConvertTo-Json -Compress)
