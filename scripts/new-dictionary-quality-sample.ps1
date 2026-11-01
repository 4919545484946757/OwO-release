param([string]$Source='', [Parameter(Mandatory=$true)][string]$OutputFile, [ValidateRange(1,100)][int]$PerCell=30)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $Source) { $Source=Join-Path $repo 'build/upstream/rime-ice-2026.06.30/cn_dicts/base.dict.yaml' }
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a sample.' }
$buckets=@{}
foreach ($line in [IO.File]::ReadLines([IO.Path]::GetFullPath($Source),[Text.Encoding]::UTF8)) {
    if ($line.StartsWith('#')) { continue }
    $parts=$line.Split("`t")
    if ($parts.Count -lt 3 -or $parts[0] -notmatch '^[\u4e00-\u9fff]{2,6}$' -or $parts[1] -notmatch '^[a-z]+( [a-z]+)+$') { continue }
    $frequency=0L
    if (-not [long]::TryParse($parts[2],[ref]$frequency) -or $frequency -lt 1000) { continue }
    $length=$parts[0].Length
    $syllables=@($parts[1].Split(' '))
    if ($length -ne $syllables.Count) { continue }
    $band=if ($frequency -ge 1000000) { 'high' } elseif ($frequency -ge 10000) { 'medium' } else { 'lower' }
    $key="$length-$band"
    if (-not $buckets.ContainsKey($key)) { $buckets[$key]=[Collections.Generic.List[object]]::new() }
    $buckets[$key].Add([pscustomobject]@{text=$parts[0];input=($syllables -join '');frequency=$frequency;length=$length;band=$band})
}
$rows=[Collections.Generic.List[string]]::new(); $number=0
foreach ($key in @($buckets.Keys | Sort-Object)) {
    foreach ($word in @($buckets[$key] | Sort-Object @{Expression='frequency';Descending=$true},text | Select-Object -First $PerCell)) {
        $number++
        $rows.Add(([ordered]@{id=('r04-dict-'+$number.ToString('D4'));input=$word.input;expected=@($word.text);tags=@('dictionary_self_consistency',('chars_'+$word.length),('freq_'+$word.band),'ambiguous')} | ConvertTo-Json -Depth 8 -Compress))
    }
}
if (-not $rows.Count) { throw 'No source entries qualified.' }
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
$utf8=[Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllLines($destination,$rows,$utf8)
$metadata=[ordered]@{source=[IO.Path]::GetFullPath($Source);source_sha256=(Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash.ToLowerInvariant();kind='dictionary_self_consistency_not_independent_accuracy';per_length_frequency_cell=$PerCell;cases=$rows.Count;attribution='Existing Rime Ice source fixture; repository/upstream licensing applies.'}
[IO.File]::WriteAllText(($destination+'.meta.json'),($metadata | ConvertTo-Json -Depth 8),$utf8)
Write-Output ('Generated dictionary self-consistency sample: '+$rows.Count+' cases')
