param(
    [Parameter(Mandatory=$true)][string]$Source,
    [Parameter(Mandatory=$true)][string]$OutputFile,
    [ValidateSet('near','distant')][string]$Variant='near',
    [ValidateSet('suffix','preserve')][string]$PreviousPolicy='suffix'
)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a dataset.' }
$prefixFile=Join-Path $repo 'tests/data/quality-review/context-window-prefix.json'
$prefixDefinition=Get-Content -LiteralPath $prefixFile -Raw -Encoding UTF8|ConvertFrom-Json
$prefix=$prefixDefinition.text
$rows=[Collections.Generic.List[string]]::new();$utf8=[Text.UTF8Encoding]::new($false)
foreach ($line in [IO.File]::ReadLines([IO.Path]::GetFullPath($Source),[Text.Encoding]::UTF8)) {
    $test=$line|ConvertFrom-Json
    if (-not $test.context -or -not $test.expected.Count) { throw 'Expected labeled context cases.' }
    $test.context=if ($Variant -eq 'near') { $prefix+$test.context } else { $test.context+$prefix }
    if ($Variant -eq 'distant' -and $PreviousPolicy -eq 'suffix') { $test.previous=$prefixDefinition.suffix_previous }
    if ($utf8.GetByteCount($test.context) -gt 256) { throw 'Context exceeds batch UTF-8 limit.' }
    $test.tags=@($test.tags)+@('context_window',('context_'+$Variant),'context_distance_exploratory')
    $rows.Add(($test|ConvertTo-Json -Depth 12 -Compress))
}
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,$utf8)
$meta=[ordered]@{source=[IO.Path]::GetFullPath($Source);source_sha256=(Get-FileHash -LiteralPath $Source).Hash.ToLowerInvariant();prefix_sha256=(Get-FileHash -LiteralPath $prefixFile).Hash.ToLowerInvariant();variant=$Variant;previous_policy=$PreviousPolicy;cases=$rows.Count;note='Signal-distance diagnostic; not a natural distribution or unique-intent assertion.'}
[IO.File]::WriteAllText(($destination+'.meta.json'),($meta|ConvertTo-Json -Depth 8),$utf8)
Write-Output ('Generated context-window cases: '+$rows.Count+' '+$Variant)
