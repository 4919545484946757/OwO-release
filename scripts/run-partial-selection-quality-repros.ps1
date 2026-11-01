param([string]$Cases='', [switch]$HostSeparatorCleanup)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Cases) {$Cases=Join-Path $repo 'tests/data/quality-review/partial-selection-cases.json'}
$cli=Join-Path $repo 'build/windows-developer/Release/owo_dev_cli.exe'
$lexicon=Join-Path $repo 'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'
$base=Join-Path $repo ('build/quality-partial-selection-20261029/run-'+[Guid]::NewGuid().ToString('N'))
$utf8=[Text.UTF8Encoding]::new($false);[Console]::OutputEncoding=$utf8;$OutputEncoding=$utf8
[void][IO.Directory]::CreateDirectory($base)
$results=[Collections.Generic.List[string]]::new()
foreach ($test in (Get-Content -LiteralPath $Cases -Raw -Encoding UTF8|ConvertFrom-Json)) {
    if ($test.id -notmatch '^[a-z-]+$' -or $test.input -notmatch "^[a-zA-Z']{1,256}$") {throw 'Invalid fixture.'}
    $root=Join-Path $base $test.id
    $null=& $cli --dev --root $root init
    if ($LASTEXITCODE) {throw 'Cannot initialize private session.'}
    $operations=[Collections.Generic.List[string]]::new();$operations.Add('query '+$test.input)
    $current=$test.input;$committed='';$evidence=[Collections.Generic.List[object]]::new();$pass=$true;$reason=''
    foreach ($step in $test.steps) {
        $found=$null
        for ($page=0;$page -lt 8;$page++) {
            $raw=@(& $cli --dev --root $root input query --text $current --page $page --lexicon $lexicon)
            if ($LASTEXITCODE) {throw 'Candidate query failed.'}
            $answer=$raw[-1]|ConvertFrom-Json
            for ($index=0;$index -lt $answer.candidates.Count;$index++) {
                if ($answer.candidates[$index] -ceq $step.select) {$found=[pscustomobject]@{page=$page;index=($index+1);consumed=$answer.candidate_consumed[$index]};break}
            }
            if ($found -or -not $answer.has_more) {break}
        }
        if (-not $found) {$pass=$false;$reason='requested_partial_candidate_not_visible';break}
        $operations.Add('page '+$found.page);$operations.Add('select '+$found.index)
        $committed+=$step.select
        $operations.Add('expect-commit '+$committed)
        $rawRemaining=$current.Substring($found.consumed)
        $actualRemaining=if ($HostSeparatorCleanup) {$rawRemaining.TrimStart([char]39)} else {$rawRemaining}
        if ($actualRemaining -cne $rawRemaining) { $operations.Add('query '+$actualRemaining) }
        $operations.Add('expect-input '+$step.remaining)
        $evidence.Add([ordered]@{input=$current;selected=$step.select;page=$found.page;index=$found.index;consumed=$found.consumed;expected_remaining=$step.remaining;raw_remaining=$rawRemaining;actual_remaining=$actualRemaining})
        if ($actualRemaining -cne $step.remaining) {$pass=$false;$reason='consumed_range_mismatch';break}
        $current=$actualRemaining
    }
    $replay=Join-Path $root 'selection.owotest'
    [IO.File]::WriteAllLines($replay,$operations,$utf8)
    $raw=@(& $cli --dev --root $root input replay --file $replay --lexicon $lexicon)
    $exit=$LASTEXITCODE
    [IO.File]::WriteAllLines(($replay+'.jsonl'),$raw,$utf8)
    if ($exit) {$pass=$false;if (-not $reason) {$reason='replay_selection_or_assertion_failed'}}
    $row=[ordered]@{id=$test.id;input=$test.input;host_separator_cleanup=[bool]$HostSeparatorCleanup;pass=$pass;reason=$reason;replay_exit=$exit;evidence=$evidence.ToArray();replay=($replay+'.jsonl');note='CLI/Core consumption and next-query test, not TSF keyboard/mouse or renderer coverage. Optional separator cleanup simulates the inspected TSF adapter.'}
    $encoded=$row|ConvertTo-Json -Depth 12 -Compress;$results.Add($encoded);Write-Output $encoded
}
[IO.File]::WriteAllLines((Join-Path $base 'summary.jsonl'),$results,$utf8)
Write-Output ('PARTIAL_REPRO_DIRECTORY='+$base)
