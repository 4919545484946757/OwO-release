param(
    [Parameter(Mandatory=$true)][string[]]$ReportDirectories,
    [Parameter(Mandatory=$true)][string]$OutputFile,
    [ValidateRange(1,1000)][int]$MaximumPerReport=100
)
$ErrorActionPreference='Stop'
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a dataset.' }
$rows=[Collections.Generic.List[string]]::new()
$sources=[Collections.Generic.List[object]]::new()
$seen=[Collections.Generic.HashSet[string]]::new()
foreach ($directory in $ReportDirectories) {
    $report=[IO.Path]::GetFullPath($directory)
    $count=0
    foreach ($line in [IO.File]::ReadLines((Join-Path $report 'cases.jsonl'),[Text.Encoding]::UTF8)) {
        $record=$line|ConvertFrom-Json
        if ($record.record -ne 'case' -or $record.runtime_error -or -not $record.expected.Count -or $null -ne $record.final_first_page_rank) { continue }
        $test=$record.test
        $key=$test|ConvertTo-Json -Depth 12 -Compress
        if (-not $seen.Add($key)) { continue }
        $test.tags=@($test.tags)+@('deep_scan_retest')
        $rows.Add(($test|ConvertTo-Json -Depth 12 -Compress))
        $sources.Add([ordered]@{id=$test.id;input=$test.input;report=$report;base_rank=$record.base_rank;base_scan_truncated=$record.base_scan_truncated;final_rank=$record.final_first_page_rank})
        $count++
        if ($count -ge $MaximumPerReport) { break }
    }
}
if (-not $rows.Count) { throw 'No eligible misses.' }
$utf8=[Text.UTF8Encoding]::new($false)
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,$utf8)
[IO.File]::WriteAllText(($destination+'.sources.json'),($sources|ConvertTo-Json -Depth 12),$utf8)
Write-Output ('Generated deep-scan retest: '+$rows.Count+' cases')
