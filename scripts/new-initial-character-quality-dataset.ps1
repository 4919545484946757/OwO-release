param([Parameter(Mandatory=$true)][string]$OutputFile)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) {throw 'Refusing to overwrite a dataset.'}
$targets=Get-Content -LiteralPath (Join-Path $repo 'tests/data/quality-review/initial-character-targets.json') -Raw -Encoding UTF8|ConvertFrom-Json
$rows=[Collections.Generic.List[string]]::new()
foreach ($letter in $targets.PSObject.Properties) {
    $number=0
    foreach ($target in $letter.Value) {
        $number++
        $row=[ordered]@{id=('initial-character-'+$letter.Name+'-'+$number);input=$letter.Name;expected=@($target);consumed=1;tags=@('human_seed','initial_character','ambiguous','common_character_coverage')}
        $rows.Add(($row|ConvertTo-Json -Depth 8 -Compress))
    }
}
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,[Text.UTF8Encoding]::new($false))
Write-Output ('Generated initial-character targets: '+$rows.Count)
