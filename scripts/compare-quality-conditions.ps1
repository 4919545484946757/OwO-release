param(
    [Parameter(Mandatory=$true)][string]$Baseline,
    [Parameter(Mandatory=$true)][string]$Candidate,
    [Parameter(Mandatory=$true)][string]$OutputFile
)
# Exploratory paired comparison for changed input spelling / evaluation conditions.
# Unlike compare-cli-reports.ps1, this is NOT a strict same-policy regression gate.
$ErrorActionPreference='Stop'
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a comparison.' }
function Read-Cases($Directory) {
    $path=[IO.Path]::GetFullPath($Directory)
    $summary=Get-Content -LiteralPath (Join-Path $path 'summary.json') -Raw -Encoding UTF8|ConvertFrom-Json
    if (-not $summary.complete -or $summary.metrics.errors) { throw 'Incomplete/failed report.' }
    $manifest=$null; $cases=@{}
    foreach ($line in [IO.File]::ReadLines((Join-Path $path 'cases.jsonl'),[Text.Encoding]::UTF8)) {
        $row=$line|ConvertFrom-Json
        if ($row.record -eq 'manifest') { $manifest=$row }
        if ($row.record -ne 'case') { continue }
        if (-not $row.expected.Count) { throw 'Paired comparison requires gold labels.' }
        if ($null -ne $row.test.consumed -and $row.test.consumed -ne $row.input.Length) { throw 'Partial-consumption cases require separate mapping.' }
        $key=[ordered]@{expected=@($row.expected);context=$row.test.context;previous=$row.test.previous;valid=$row.test.valid}|ConvertTo-Json -Depth 10 -Compress
        if ($cases.ContainsKey($key)) { throw 'Ambiguous pairing: repeated gold/context definition.' }
        $cases[$key]=$row
    }
    if (-not $manifest -or $cases.Count -ne $summary.metrics.cases) { throw 'Manifest/count mismatch.' }
    return @{manifest=$manifest;cases=$cases;path=$path}
}
$left=Read-Cases $Baseline; $right=Read-Cases $Candidate
if ($left.cases.Count -ne $right.cases.Count) { throw 'Different case counts.' }
foreach ($field in @('cli_asset','lexicon','model_asset','bridge')) {
    if ($left.manifest.$field.sha256 -cne $right.manifest.$field.sha256) { throw ('Different/unhashed asset: '+$field) }
    if ($left.manifest.$field -and -not $left.manifest.$field.sha256) { throw 'Assets must be hashed.' }
}
if ($left.manifest.model -cne $right.manifest.model -or $left.manifest.learning -cne $right.manifest.learning) { throw 'Backend/learning differs.' }
$changes=[Collections.Generic.List[object]]::new()
$baseLost=0;$baseGained=0;$finalLost=0;$finalGained=0;$firstChanges=0;$baseFirstChanges=0
foreach ($key in $left.cases.Keys) {
    if (-not $right.cases.ContainsKey($key)) { throw 'Missing paired gold/context definition.' }
    $a=$left.cases[$key];$b=$right.cases[$key]
    $baseChange=($a.base_candidates[0].text -cne $b.base_candidates[0].text)
    $finalChange=($a.final_first_page[0].text -cne $b.final_first_page[0].text)
    if ($baseChange) {$baseFirstChanges++};if ($finalChange) {$firstChanges++}
    if ($null -ne $a.base_rank -and $null -eq $b.base_rank) {$baseLost++}
    if ($null -eq $a.base_rank -and $null -ne $b.base_rank) {$baseGained++}
    if ($null -ne $a.final_first_page_rank -and $null -eq $b.final_first_page_rank) {$finalLost++}
    if ($null -eq $a.final_first_page_rank -and $null -ne $b.final_first_page_rank) {$finalGained++}
    if ($baseChange -or $finalChange -or $a.base_rank -ne $b.base_rank -or $a.final_first_page_rank -ne $b.final_first_page_rank) {
        $changes.Add([pscustomobject][ordered]@{expected=@($a.expected);before_id=$a.id;after_id=$b.id;before_input=$a.input;after_input=$b.input;before_preview=$a.preview;after_preview=$b.preview;before_base_rank=$a.base_rank;after_base_rank=$b.base_rank;before_final_rank=$a.final_first_page_rank;after_final_rank=$b.final_first_page_rank;before_base_first=$a.base_candidates[0].text;after_base_first=$b.base_candidates[0].text;before_final_first=$a.final_first_page[0].text;after_final_first=$b.final_first_page[0].text})
    }
}
$result=[ordered]@{schema_version=1;kind='exploratory_condition_comparison_not_regression_gate';baseline=$left.path;candidate=$right.path;cases=$left.cases.Count;base_coverage_lost=$baseLost;base_coverage_gained=$baseGained;final_first_page_lost=$finalLost;final_first_page_gained=$finalGained;base_first_changes=$baseFirstChanges;final_first_changes=$firstChanges;baseline_policy=$left.manifest;candidate_policy=$right.manifest;changes=@($changes|Sort-Object before_input)}
$utf8=[Text.UTF8Encoding]::new($false)
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllText($destination,($result|ConvertTo-Json -Depth 25),$utf8)
[ordered]@{cases=$result.cases;base_lost=$baseLost;base_gained=$baseGained;final_lost=$finalLost;final_gained=$finalGained;base_first_changes=$baseFirstChanges;final_first_changes=$firstChanges;output=$destination}|ConvertTo-Json -Compress
