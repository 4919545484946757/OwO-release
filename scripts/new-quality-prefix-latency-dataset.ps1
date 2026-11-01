param([Parameter(Mandatory=$true)][string]$Source,[Parameter(Mandatory=$true)][string]$OutputFile)
$ErrorActionPreference='Stop'
$destination=[IO.Path]::GetFullPath($OutputFile)
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a dataset.' }
$rows=[Collections.Generic.List[string]]::new();$seen=[Collections.Generic.HashSet[string]]::new();$number=0
foreach ($line in [IO.File]::ReadLines([IO.Path]::GetFullPath($Source),[Text.Encoding]::UTF8)) {
    $sourceCase=$line|ConvertFrom-Json
    if ($sourceCase.input -notmatch '^[a-z]{1,256}$') { throw 'Expected plain full-pinyin source.' }
    for ($length=4;$length -lt $sourceCase.input.Length;$length++) {
        $prefix=$sourceCase.input.Substring(0,$length)
        if (-not $seen.Add($prefix)) {continue}
        $number++
        if ($number -gt 10000) {throw 'Too many prefixes.'}
        $row=[ordered]@{id=('prefix-latency-'+$number.ToString('D4'));input=$prefix;tags=@('prefix_latency_probe','unlabeled','incomplete','synthetic_long_input')}
        $rows.Add(($row|ConvertTo-Json -Depth 8 -Compress))
    }
}
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,[Text.UTF8Encoding]::new($false))
Write-Output ('Generated unlabeled prefix-latency probes: '+$rows.Count)
