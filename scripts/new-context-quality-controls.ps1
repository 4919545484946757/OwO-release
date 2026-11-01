param([string]$OutputDirectory='')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $OutputDirectory) { $OutputDirectory=Join-Path $repo 'build/quality-datasets-20261029' }
$OutputDirectory=[IO.Path]::GetFullPath($OutputDirectory)
$source=Join-Path $repo 'tests/data/quality-review/context-cases.jsonl'
$previous=Get-Content -LiteralPath (Join-Path $repo 'tests/data/quality-review/context-previous.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$sets=@{corrected=[Collections.Generic.List[string]]::new();with_previous=[Collections.Generic.List[string]]::new();without_context=[Collections.Generic.List[string]]::new()}
foreach ($line in [IO.File]::ReadLines($source,[Text.Encoding]::UTF8)) {
    $row=$line | ConvertFrom-Json
    # Annotation correction only: the company reading is gong si, not gong shi.
    # Original v1 data and its complete reports remain unchanged for audit.
    if ($row.id -eq 'ctx26-a') { $row.input='gongsi' }
    $sets.corrected.Add(($row | ConvertTo-Json -Depth 8 -Compress))
    if (-not $previous.PSObject.Properties[$row.id]) { throw ('Missing previous-code label: '+$row.id) }
    $with=$row | ConvertTo-Json -Depth 8 -Compress | ConvertFrom-Json
    $with | Add-Member -NotePropertyName previous -NotePropertyValue $previous.PSObject.Properties[$row.id].Value -Force
    $with.tags += @('previous_code_present')
    $sets.with_previous.Add(($with | ConvertTo-Json -Depth 8 -Compress))
    $without=$row | ConvertTo-Json -Depth 8 -Compress | ConvertFrom-Json
    $without.context=''; $without.tags += @('context_removed_control')
    $sets.without_context.Add(($without | ConvertTo-Json -Depth 8 -Compress))
}
foreach ($name in $sets.Keys) { if (Test-Path -LiteralPath (Join-Path $OutputDirectory ('context-'+$name+'.jsonl'))) { throw 'Context output already exists; refusing overwrite.' } }
[void][IO.Directory]::CreateDirectory($OutputDirectory)
foreach ($name in $sets.Keys) { [IO.File]::WriteAllLines((Join-Path $OutputDirectory ('context-'+$name+'.jsonl')),$sets[$name],[Text.UTF8Encoding]::new($false)) }
Write-Output ('Created corrected, previous-code, and no-context controls: '+$sets.corrected.Count+' cases each')
