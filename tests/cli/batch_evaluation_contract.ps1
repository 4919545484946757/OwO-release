param([string]$Cli,[string]$Parent,[string]$Repo)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$utf8 = [Text.UTF8Encoding]::new($false)
$hello = ([string][char]0x4F60) + [char]0x597D
$fixture = Join-Path ([IO.Path]::GetFullPath($Parent)) ('cli-batch-' + [Guid]::NewGuid().ToString('N'))
$session = Join-Path $fixture 'session'
function Run([string[]]$Tail,[int]$Expected=0) {
    $raw = @(& $Cli --dev --root $session @Tail); $code = $LASTEXITCODE
    if ($code -ne $Expected) { throw "Unexpected exit $code for $($Tail -join ' '): $($raw -join ' ')" }
    return @($raw | ForEach-Object { $_ | ConvertFrom-Json })[-1]
}
function Assert($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
[void](New-Item -ItemType Directory -Path $fixture)
Run @('init') | Out-Null
$smoke = Join-Path $Repo 'examples/cli/smoke.jsonl'
$result = Run @('input','batch','--file',$smoke,'--report','smoke','--check-expanded','--repeat','2','--hash-assets')
Assert ($result.complete -and $result.metrics.labeled -eq 4 -and $result.metrics.base_topk -eq 1 -and $result.metrics.errors -eq 0) 'Smoke batch metrics are invalid.'
$report = Join-Path $session 'reports/smoke'
Assert ((Get-Content -LiteralPath (Join-Path $report 'failures.jsonl') -Raw).Length -eq 0) 'Smoke batch unexpectedly failed.'
Assert (-not (Test-Path -LiteralPath (Join-Path $session 'user-frequency.owuf'))) 'Batch persisted learning.'
$manifest = Get-Content -LiteralPath (Join-Path $report 'cases.jsonl') -Encoding UTF8 -TotalCount 1 | ConvertFrom-Json
Assert ($manifest.dataset.sha256.Length -eq 64 -and $manifest.cli_asset.sha256.Length -eq 64) 'Missing reproducible asset hashes.'
Run @('input','batch','--file',$smoke,'--report','smoke') 3 | Out-Null
$bad = Join-Path $fixture 'bad.jsonl'
[IO.File]::WriteAllText($bad,'{"input":"nihao","expected":["missing"]}',$utf8)
$failed = Run @('input','batch','--file',$bad,'--report','miss') 1
Assert ($failed.metrics.failures -eq 1 -and $failed.metrics.base_top1 -eq 0) 'Quality miss was not reported.'
Run @('input','batch','--file',(Join-Path $session 'reports/miss/failures.jsonl'),'--report','retry','--allow-misses') | Out-Null
Run @('input','batch','--file',$bad,'--report','gate','--allow-misses','--min-top1','1') 1 | Out-Null
$tsv = Join-Path $fixture 'words.tsv'
[IO.File]::WriteAllText($tsv,"id`tinput`texpected`ttags`nhello`tnihao`t[`"$hello`"]`t[`"tsv`"]`n",$utf8)
$tab = Run @('input','batch','--file',$tsv,'--report','tsv','--typing','incremental')
Assert ($tab.metrics.labeled -eq 1 -and $tab.timing_ms.incremental_prefix_request.samples -eq 4) 'TSV or incremental typing failed.'
$mock = Run @('input','batch','--file',$smoke,'--report','mock','--model-backend','mock','--allow-misses')
Assert ($mock.metrics.model_requested -gt 0 -and $mock.metrics.model_applied -eq $mock.metrics.model_requested -and $mock.metrics.model_failed -eq 0) 'Private Core/ModelHost pipeline did not apply model results.'
$externalDictionary = Join-Path $fixture 'external-dictionary'
$snapshotId = 'a' * 32
[void](New-Item -ItemType Directory -Path (Join-Path $externalDictionary ('snapshots/' + $snapshotId)))
[IO.File]::WriteAllText((Join-Path $externalDictionary 'current'),$snapshotId,$utf8)
[IO.File]::WriteAllText((Join-Path $externalDictionary ('snapshots/' + $snapshotId + '/active.tsv')),"OWO_DICTIONARIES_V1`nnihao`tWRONG`t128`n",$utf8)
$previousDictionaryPath = $env:OWO_DICTIONARY_STORE_PATH
try {
    $env:OWO_DICTIONARY_STORE_PATH = $externalDictionary
    $isolated = Run @('input','batch','--file',$smoke,'--report','isolated')
    Assert ($isolated.metrics.base_top1 -eq $result.metrics.base_top1 -and $isolated.metrics.base_topk -eq 1) 'Inherited dictionary environment leaked into batch.'
    $isolatedCases = Get-Content -LiteralPath (Join-Path $session 'reports/isolated/cases.jsonl') -Encoding UTF8 | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object record -eq 'case'
    Assert (-not @($isolatedCases.base_candidates | Where-Object text -eq 'WRONG').Count) 'External dictionary candidate leaked into isolated batch.'
} finally { $env:OWO_DICTIONARY_STORE_PATH = $previousDictionaryPath }
$rows = @(@{input='nihao';segmentation="ni'hao"},@{input='123';valid=$false}) | ForEach-Object { $_ | ConvertTo-Json -Compress }
[IO.File]::WriteAllText($bad,($rows -join "`n"),$utf8)
Run @('input','batch','--file',$bad,'--report','parse','--mode','parse') | Out-Null
foreach ($invalid in @('{"input":"nihao","input":"x"}','{"input":"nihao","unknown":1}','{"input":"nihao","expected":"hello"}','{"input":"nihao","consumed":99}',('{"input":"nihao","id":"same"}'+"`n"+'{"input":"nihao","id":"same"}'),'{"input":"nihao","tags":["\uD800"]}')) {
    [IO.File]::WriteAllText($bad,$invalid,$utf8)
    Run @('input','batch','--file',$bad,'--report','bad') 2 | Out-Null
    Assert (-not (Test-Path -LiteralPath (Join-Path $session 'reports/bad'))) 'Invalid dataset wrote a report.'
}
Run @('input','batch','--file',$smoke,'--report','../escape') 2 | Out-Null
Run @('input','batch','--file',$smoke,'--report','CON') 2 | Out-Null
Run @('input','batch','--file',$smoke,'--unknown','x') 2 | Out-Null
Run @('input','batch','--file',$smoke,'--model-backend','libime') 3 | Out-Null
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Repo 'scripts/compare-cli-reports.ps1') -Baseline $report -Candidate $report -FailOnRegression | Out-Null
Assert ($LASTEXITCODE -eq 0) 'Identical reports failed comparison.'
Write-Output 'PASS: isolated CLI batch formats, metrics, gates, failures replay, incremental input, model pipeline, hashes, invalid data and report comparison.'
# Intentionally preserve this bounded private fixture for reproducibility.
