param([Parameter(Mandatory=$true)][string]$CasesFile,[Parameter(Mandatory=$true)][string]$OutputFile,[ValidateRange(1,100)][int]$Maximum=30,[ValidateSet('inherit','on','off')][string]$Correction='inherit')
$ErrorActionPreference='Stop'
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) {throw 'Refusing to overwrite a dataset.'}
$cases=@([IO.File]::ReadLines([IO.Path]::GetFullPath($CasesFile),[Text.Encoding]::UTF8)|ForEach-Object {$_|ConvertFrom-Json}|Where-Object {$_.record -eq 'case' -and -not $_.runtime_error -and $_.timing_ms.base_request -gt 50}|Sort-Object @{Expression={$_.timing_ms.base_request};Descending=$true}|Select-Object -First $Maximum)
if (-not $cases.Count) {throw 'No latency hotspots found.'}
$rows=@($cases|ForEach-Object {$test=$_.test;$test.tags=@($test.tags)+@('latency_hotspot_retest');if ($Correction -ne 'inherit') {$test.correction=($Correction -eq 'on');$test.tags+=('correction_forced_'+$Correction)};$test|ConvertTo-Json -Depth 12 -Compress})
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,[Text.UTF8Encoding]::new($false))
Write-Output ('Generated hotspot retest: '+$rows.Count)
