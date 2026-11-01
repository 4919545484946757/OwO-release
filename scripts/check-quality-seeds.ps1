param([string]$SeedDirectory='', [string]$CharacterSource='')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SeedDirectory) { $SeedDirectory=Join-Path $repo 'tests/data/quality-review' }
if (-not $CharacterSource) { $CharacterSource=Join-Path $repo 'build/upstream/rime-ice-2026.06.30/cn_dicts/8105.dict.yaml' }
$mapping=@{}
foreach ($line in [IO.File]::ReadLines($CharacterSource,[Text.Encoding]::UTF8)) {
    $parts=$line.Split("`t")
    if ($parts.Count -ge 2 -and $parts[0].Length -eq 1 -and $parts[1] -match '^[a-z]+$') {
        if (-not $mapping.ContainsKey($parts[0])) { $mapping[$parts[0]]=[Collections.Generic.HashSet[string]]::new() }
        [void]$mapping[$parts[0]].Add($parts[1])
    }
}
$issues=[Collections.Generic.List[object]]::new(); $total=0
foreach ($file in Get-ChildItem -LiteralPath $SeedDirectory -Filter '*.seeds' -File) {
    $number=0
    foreach ($line in [IO.File]::ReadLines($file.FullName,[Text.Encoding]::UTF8)) {
        $number++; if (-not $line.Trim() -or $line.StartsWith('#')) { continue }
        $parts=$line.Split('|'); $total++
        if ($parts.Count -ne 3) { $issues.Add(@{file=$file.Name;line=$number;kind='bad_seed_format';raw=$line}); continue }
        $characters=$parts[0]; $syllables=@($parts[1].Split(' ',[StringSplitOptions]::RemoveEmptyEntries))
        if ($characters.Length -ne $syllables.Count) { $issues.Add(@{file=$file.Name;line=$number;kind='character_syllable_count';text=$characters;reading=$parts[1]}); continue }
        for ($index=0;$index -lt $characters.Length;$index++) {
            $character=$characters.Substring($index,1)
            if ($mapping.ContainsKey($character) -and -not $mapping[$character].Contains($syllables[$index])) {
                $issues.Add(@{file=$file.Name;line=$number;kind='reading_needs_human_confirmation';text=$characters;character=$character;reading=$syllables[$index];source_readings=@($mapping[$character])})
            }
        }
    }
}
[ordered]@{seeds=$total;annotation_checks=$issues.Count;checks=@($issues);note='Character reading coverage is only a label sanity check, not proof of word-context pronunciation or candidate correctness.'} | ConvertTo-Json -Depth 10
