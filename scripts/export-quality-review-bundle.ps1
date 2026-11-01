param([string]$OutputFile='')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $OutputFile) {$OutputFile=Join-Path $repo 'artifacts/quality-review/2026-10-29/OwO-quality-review-20261029.zip'}
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) {throw 'Refusing to overwrite a review bundle.'}
$session=Join-Path $repo 'build/quality-review-20261029'
$review=Join-Path $repo 'docs/quality-review/2026-10-29'
$runs=Get-Content -LiteralPath (Join-Path $session 'rounds.jsonl') -Encoding UTF8|ForEach-Object {$_|ConvertFrom-Json}
$files=[Collections.Generic.List[object]]::new();$names=[Collections.Generic.HashSet[string]]::new()
function Add-File([string]$File,[string]$Name) {
    $absolute=[IO.Path]::GetFullPath($File)
    if (-not $absolute.StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {throw 'Bundle source escaped repository.'}
    if ($Name -match '(^/|(^|/)\.\.(/|$)|:)') {throw 'Unsafe archive entry.'}
    $item=Get-Item -LiteralPath $absolute
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {throw 'Refusing reparse-point source.'}
    if (-not $names.Add($Name)) {return}
    $files.Add([pscustomobject]@{path=$absolute;entry=$Name;bytes=$item.Length;sha256=(Get-FileHash -LiteralPath $absolute -Algorithm SHA256).Hash.ToLowerInvariant()})
}
foreach ($file in (Get-ChildItem -LiteralPath $review -File)) {Add-File $file.FullName ('review/'+$file.Name)}
foreach ($file in (Get-ChildItem -LiteralPath (Join-Path $repo 'tests/data/quality-review') -File)) {Add-File $file.FullName ('tests/data/quality-review/'+$file.Name)}
foreach ($file in (Get-ChildItem -LiteralPath $PSScriptRoot -File|Where-Object {$_.Name -match '(quality|^new-context-|^run-pagination-repros|^compare-cli-reports)'})) {Add-File $file.FullName ('scripts/'+$file.Name)}
foreach ($name in @('cli-user.md','cli-ai.md')) {Add-File (Join-Path $repo ('docs/'+$name)) ('docs/'+$name)}
Add-File (Join-Path $session 'rounds.jsonl') 'raw/rounds.jsonl'
foreach ($run in $runs) {
    $directory=[IO.Path]::GetFullPath($run.report)
    if (-not $directory.StartsWith((Join-Path $session 'reports')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or -not $run.summary.complete) {throw 'Unexpected/incomplete native report.'}
    foreach ($file in (Get-ChildItem -LiteralPath $directory -File|Where-Object {$_.Extension -in @('.jsonl','.json','.md','.conf')})) {Add-File $file.FullName ('raw/reports/'+(Split-Path $directory -Leaf)+'/'+$file.Name)}
}
foreach ($file in (Get-ChildItem -LiteralPath $session -Filter '*comparison.json' -File)) {Add-File $file.FullName ('raw/comparisons/'+$file.Name)}
foreach ($file in (Get-ChildItem -LiteralPath (Join-Path $repo 'build/quality-datasets-20261029') -File|Where-Object {$_.Name -match '\.(meta|sources)\.json$'})) {Add-File $file.FullName ('raw/dataset-metadata/'+$file.Name)}
foreach ($name in @('quality-learning-20261029','quality-pagination-repros-20261029','quality-partial-selection-20261029')) {
    $directory=Join-Path $repo ('build/'+$name)
    if (-not (Test-Path -LiteralPath $directory)) {continue}
    foreach ($file in (Get-ChildItem -LiteralPath $directory -Recurse -File|Where-Object {$_.Name -eq 'summary.jsonl' -or $_.Name -match '\.owotest(\.jsonl)?$'})) {
        Add-File $file.FullName ('raw/supplemental/'+$name+'/'+$file.FullName.Substring($directory.Length+1).Replace('\','/'))
    }
}
$revision=(& git -C $repo rev-parse HEAD).Trim()
if ($LASTEXITCODE) {throw 'Cannot record source revision.'}
$metadata=[ordered]@{schema_version=1;created_utc=[DateTime]::UtcNow.ToString('o');source_revision=$revision;purpose='Offline human review of synthetic pinyin quality tests; no installer, production runtime, model, or personal user lexicon included.';runs=@($runs).Count;files=@($files|Select-Object entry,bytes,sha256)}
$manifest=$metadata|ConvertTo-Json -Depth 12
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
$archive=[IO.Compression.ZipFile]::Open($destination,[IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($file in $files) {[void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive,$file.path,$file.entry,[IO.Compression.CompressionLevel]::Optimal)}
    foreach ($extra in @(@('bundle-manifest.json',$manifest),@('START-HERE.md',"# OwO candidate quality review`n`nOpen review/issues.html in a browser. Read review/README.md and review/priority-review.md first.`nThe raw/reports folders contain dataset snapshots, case evidence and effective configurations.`nAll text is synthetic or dictionary sampling. No personal lexicon, binary lexicon or model is included.`nThe scripts require the repository development CLI/assets to rerun; the HTML review works offline without them.`n"))) {
        $entry=$archive.CreateEntry($extra[0]);$writer=[IO.StreamWriter]::new($entry.Open(),[Text.UTF8Encoding]::new($false))
        try {$writer.Write($extra[1])} finally {$writer.Dispose()}
    }
} finally {$archive.Dispose()}
$check=[IO.Compression.ZipFile]::OpenRead($destination)
try {if ($check.Entries.Count -ne $files.Count+2) {throw 'Archive entry count mismatch.'}} finally {$check.Dispose()}
[IO.File]::WriteAllText(($destination+'.manifest.json'),$manifest,[Text.UTF8Encoding]::new($false))
[ordered]@{bundle=$destination;files=$files.Count+2;bytes=(Get-Item -LiteralPath $destination).Length;sha256=(Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()}|ConvertTo-Json -Compress
