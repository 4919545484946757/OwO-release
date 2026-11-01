param(
    [string]$Targets='',
    [ValidateRange(1,10)][int]$Selections=10,
    [ValidateRange(1,10)][int[]]$Sensitivities=@(1,5,10),
    [ValidateSet('on','off')][string]$SingleCharacterFirst='on',
    [string]$InputFilter=''
)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Targets) { $Targets=Join-Path $repo 'tests/data/quality-review/learning-targets.json' }
$cli=Join-Path $repo 'build/windows-developer/Release/owo_dev_cli.exe'
$lexicon=Join-Path $repo 'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'
$base=Join-Path $repo ('build/quality-learning-20261029/run-'+[Guid]::NewGuid().ToString('N'))
$utf8=[Text.UTF8Encoding]::new($false); [Console]::OutputEncoding=$utf8; $OutputEncoding=$utf8
[void][IO.Directory]::CreateDirectory($base)
$results=[Collections.Generic.List[string]]::new()
function Locate($Root,$InputCode,$Target) {
    for ($page=0;$page -lt 8;$page++) {
        $raw=@(& $cli --dev --root $Root input query --text $InputCode --page $page --lexicon $lexicon)
        if ($LASTEXITCODE) { throw 'Isolated query failed.' }
        $response=$raw[-1]|ConvertFrom-Json
        for ($index=0;$index -lt $response.candidates.Count;$index++) {
            if ($response.candidates[$index] -ceq $Target -and $response.candidate_consumed[$index] -eq $InputCode.Length) {
                return [pscustomobject]@{page=$page;index=($index+1);rank=($page*$response.page_size+$index+1)}
            }
        }
        if (-not $response.has_more) { break }
    }
    return $null
}
foreach ($sensitivity in $Sensitivities) {
    foreach ($test in (Get-Content -LiteralPath $Targets -Raw -Encoding UTF8|ConvertFrom-Json)) {
        if ($InputFilter -and $test.input -cne $InputFilter) { continue }
        if ($test.input -notmatch '^[a-z]{1,256}$' -or $test.target -match '[\r\n]') { throw 'Invalid learning fixture.' }
        $root=Join-Path $base ('s'+$sensitivity+'-'+$test.input)
        $null=& $cli --dev --root $root init
        if ($LASTEXITCODE) { throw 'Cannot initialize isolated learning root.' }
        $configPath=Join-Path $root 'owo.conf'
        $config=[IO.File]::ReadAllText($configPath,[Text.Encoding]::UTF8).Replace('user_learning_sensitivity=10',('user_learning_sensitivity='+$sensitivity))
        if ($SingleCharacterFirst -eq 'off') { $config=$config.Replace('candidate_single_character_first_enabled=true','candidate_single_character_first_enabled=false') }
        [IO.File]::WriteAllText($configPath,$config,$utf8)
        $before=Locate $root $test.input $test.target
        $history=[Collections.Generic.List[object]]::new()
        if ($null -ne $before) {
            for ($count=1;$count -le $Selections;$count++) {
                $position=Locate $root $test.input $test.target
                if ($null -eq $position) { throw 'A previously selectable target disappeared.' }
                $replay=Join-Path $root ('select-'+$count+'.owotest')
                $operations=@(('query '+$test.input),('page '+$position.page),('select '+$position.index),('expect-commit '+$test.target),'expect-input ')
                [IO.File]::WriteAllLines($replay,$operations,$utf8)
                $raw=@(& $cli --dev --root $root input replay --file $replay --lexicon $lexicon --learn)
                [IO.File]::WriteAllLines(($replay+'.jsonl'),$raw,$utf8)
                if ($LASTEXITCODE) { throw 'Learning replay/commit assertion failed.' }
                $after=Locate $root $test.input $test.target
                $history.Add([ordered]@{selections=$count;rank=$after.rank})
            }
        }
        $row=[ordered]@{input=$test.input;target=$test.target;sensitivity=$sensitivity;single_character_first=$SingleCharacterFirst;selectable=($null -ne $before);initial_rank=$before.rank;history=$history.ToArray();isolated_root=$root;note='Every query/replay uses a fresh CLI/Core process; persistence is exercised, not wall-clock decay.'}
        $encoded=$row|ConvertTo-Json -Depth 10 -Compress
        $results.Add($encoded); Write-Output $encoded
    }
}
if (-not $results.Count) { throw 'No learning fixtures matched.' }
[IO.File]::WriteAllLines((Join-Path $base 'summary.jsonl'),$results,$utf8)
Write-Output ('LEARNING_REPRO_DIRECTORY='+$base)
