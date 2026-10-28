param([string]$SessionRoot='', [string[]]$Inputs=@('zj','baocun','gongzuo','xuexi','wang','wsm','nihao','shurufa','mingan','keyi','yiyixiawenben'))
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SessionRoot) { $SessionRoot=Join-Path $repo 'build/quality-pagination-repros-20261029' }
$SessionRoot=[IO.Path]::GetFullPath($SessionRoot)
$cli=Join-Path $repo 'build/windows-developer/Release/owo_dev_cli.exe'
$lexicon=Join-Path $repo 'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'
$utf8=[Text.UTF8Encoding]::new($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8
if (-not (Test-Path -LiteralPath (Join-Path $SessionRoot '.owo-dev-session'))) { & $cli --dev --root $SessionRoot init; if($LASTEXITCODE){throw 'Cannot initialize isolated repro session.'} }
$directory=Join-Path $SessionRoot ('pagination-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($directory)
$results=[Collections.Generic.List[string]]::new()
foreach ($inputCode in $Inputs) {
    if ($inputCode -notmatch "^[a-zA-Z']{1,256}$") { throw 'Unsafe replay input.' }
    $scriptPath=Join-Path $directory ($inputCode.Replace("'",'-')+'.owotest')
    $operations=@('query '+$inputCode)+@(1..7 | ForEach-Object {'page '+$_})+@('page 0','expand','page 0')
    [IO.File]::WriteAllLines($scriptPath,$operations,$utf8)
    $raw=@(& $cli --dev --root $SessionRoot input replay --file $scriptPath --lexicon $lexicon)
    $exit=$LASTEXITCODE
    [IO.File]::WriteAllLines(($scriptPath+'.jsonl'),$raw,$utf8)
    if ($exit) { throw ('Replay failed for '+$inputCode) }
    $events=@($raw | ForEach-Object {$_|ConvertFrom-Json} | Where-Object {$_.step})
    if ($events.Count -ne 11) { throw 'Unexpected replay event count.' }
    $fresh=@($events[0].candidates | Select-Object -First 7)
    $revisit=@($events[8].candidates | Select-Object -First 7)
    $expanded=@($events[9].candidates | Select-Object -First 7)
    $after=@($events[10].candidates | Select-Object -First 7)
    $row=[ordered]@{input=$inputCode;fresh_page0=$fresh;after_paging_page0=$revisit;expanded_first_page=$expanded;after_expansion_page0=$after;paging_changes_first_page=(($fresh|ConvertTo-Json -Compress)-cne($revisit|ConvertTo-Json -Compress));expansion_changes_first_page=(($revisit|ConvertTo-Json -Compress)-cne($expanded|ConvertTo-Json -Compress));raw_replay=($scriptPath+'.jsonl')}
    $encoded=$row|ConvertTo-Json -Depth 8 -Compress; $results.Add($encoded); Write-Output $encoded
}
[IO.File]::WriteAllLines((Join-Path $directory 'summary.jsonl'),$results,$utf8)
Write-Output ('REPRO_DIRECTORY='+$directory)
