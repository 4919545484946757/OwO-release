param(
    [Parameter(Mandatory=$true)][string]$Dataset,
    [Parameter(Mandatory=$true)][string]$Round,
    [string]$SessionRoot='',
    [ValidateSet('base','libime','both')][string]$Backend='both',
    [ValidateSet('whole','incremental')][string]$Typing='whole',
    [ValidateRange(4,9)][int]$PageSize=7,
    [ValidateSet('on','off')][string]$Partition='on',
    [ValidateSet('on','off')][string]$Correction='on',
    [ValidateRange(1,101)][int]$MaxPages=8,
    [ValidateRange(1,256)][int]$TopK=21
)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cli = Join-Path $repo 'build/windows-developer/Release/owo_dev_cli.exe'
if (-not $SessionRoot) { $SessionRoot=Join-Path $repo 'build/quality-review-20261029' }
$SessionRoot = [IO.Path]::GetFullPath($SessionRoot)
if (-not (Test-Path -LiteralPath (Join-Path $SessionRoot '.owo-dev-session'))) {
    & $cli --dev --root $SessionRoot init
    if ($LASTEXITCODE) { throw 'Private session initialization failed.' }
}
$datasetPath=[IO.Path]::GetFullPath($Dataset)
$backends = if ($Backend -eq 'both') { @('base','libime') } else { @($Backend) }
foreach ($kind in $backends) {
    $report=$Round+'-'+$kind+'-p'+$PageSize+'-'+$Partition+'-'+$Typing+'-c'+$Correction
    if (Test-Path -LiteralPath (Join-Path $SessionRoot ('reports/'+$report))) { throw "Report already exists; use a new round: $report" }
    $arguments=@('--dev','--root',$SessionRoot,'input','batch','--file',$datasetPath,'--report',$report,
        '--lexicon',(Join-Path $repo 'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'),
        '--model-backend',$kind,'--page-size',"$PageSize",'--partition',$Partition,'--typing',$Typing,
        '--correction',$Correction,'--top-k',"$TopK",'--max-pages',"$MaxPages",'--repeat','3',
        '--check-expanded','--hash-assets','--allow-misses')
    if ($kind -eq 'libime') { $arguments += @('--run-model','--bridge',(Join-Path $repo 'build/dependencies/libime-1.1.15/runtime-p1-x86_64/owo_libime_bridge.dll'),'--model',(Join-Path $repo 'build/dependencies/libime-1.1.15/model-20260804/lib/libime/zh_CN.lm'),'--model-timeout-ms','100') }
    & $cli @arguments | ForEach-Object { if ($_ -match '"record":"summary"') { $null = $_ } elseif ($_ -match '"ok":false' -and $_ -notmatch '"record":') { Write-Warning $_ } }
    $exit=$LASTEXITCODE
    $directory=Join-Path $SessionRoot ('reports/'+$report)
    $summaryPath=Join-Path $directory 'summary.json'
    if (-not (Test-Path -LiteralPath $summaryPath)) { throw "Round did not complete: $report exit=$exit" }
    $summary=Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $entry=[ordered]@{round=$Round;backend=$kind;report=$directory;dataset=$datasetPath;page_size=$PageSize;partition=$Partition;typing=$Typing;correction=$Correction;exit_code=$exit;summary=$summary}
    [IO.File]::AppendAllText((Join-Path $SessionRoot 'rounds.jsonl'),(($entry | ConvertTo-Json -Depth 20 -Compress)+"`n"),$utf8)
    [ordered]@{round=$report;exit=$exit;cases=$summary.metrics.cases;failures=$summary.metrics.failures;errors=$summary.metrics.errors;base_top1=$summary.metrics.base_top1;base_first_page=$summary.metrics.base_first_page;base_topk=$summary.metrics.base_topk;final_top1=$summary.metrics.final_top1;final_first_page=$summary.metrics.final_first_page;model_applied=$summary.metrics.model_applied;model_failed=$summary.metrics.model_failed;expansion_failed=$summary.metrics.expansion_failed;p95_ms=$summary.timing_ms.base_request.p95} | ConvertTo-Json -Compress
    if (-not $summary.complete) { throw 'Incomplete round.' }
}
