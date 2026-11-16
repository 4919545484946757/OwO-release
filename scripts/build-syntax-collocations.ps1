param(
    [Parameter(Mandatory=$true)][string]$Conllu,
    [string]$Output = (Join-Path $PSScriptRoot '..\data\syntax\zh-gsdsimp-r2.17.inc'),
    [string]$RoleOutput = (Join-Path $PSScriptRoot '..\data\syntax\zh-gsdsimp-r2.17-roles.inc')
)
$ErrorActionPreference = 'Stop'
$expectedSha256 = '956636FE612A1166E8B19E7413FEE2E73D68231ACA2F0455BE2C616B947D629D'
if ((Get-FileHash -LiteralPath $Conllu -Algorithm SHA256).Hash -ne $expectedSha256) {
    throw 'Use the unmodified Chinese-GSDSimp r2.17 train split (see data/syntax/README.md).'
}
$rows = @{}
$pairs = @{}
$roles = @{}
$joinedSupport = @{}
$nominal = @('NOUN', 'PROPN', 'PRON')
$predicate = @('VERB', 'ADJ')
function Get-Relation($head, $child) {
    $headPos = $head[3]; $depPos = $child[3]
    switch (($child[7] -split ':', 2)[0]) {
        'nsubj' { if ($depPos -in $nominal -and $headPos -in $predicate) { 'subject' } }
        { $_ -in 'obj', 'iobj' } { if ($headPos -eq 'VERB' -and $depPos -in $nominal) { 'object' } }
        { $_ -in 'amod', 'acl', 'nmod' } {
            if ($headPos -in $nominal -and ($depPos -in $nominal -or $depPos -in $predicate)) { 'adnominal' }
        }
        'advmod' { if ($headPos -in $predicate -and $depPos -eq 'ADV') { 'adverbial' } }
        'compound' { 'compound' }
        { $_ -in 'xcomp', 'ccomp' } { 'complement' }
        'clf' { 'classifier' }
    }
}
function Get-Role($token) {
    if ($token[4] -eq 'DEC' -or $token[7] -eq 'mark:rel') { return 'link' }
    if ($token[3] -in @('CCONJ', 'SCONJ', 'PUNCT')) { return 'boundary' }
    if ($token[4] -eq 'AS' -or $token[7] -eq 'aux:aspect') { return 'aspect' }
    if ($token[4] -eq 'NNB' -or $token[7] -eq 'clf' -or $token[3] -in @('DET', 'NUM')) { return 'quantity' }
    switch ($token[3]) {
        { $_ -in 'ADV', 'AUX' } { 'predicate_support' }
        'ADJ' { 'adjective' }
        { $_ -in $nominal } { 'nominal' }
        'VERB' { 'verb' }
        default { 'unknown' }
    }
}
function Add-SentencePairs {
    foreach ($token in $rows.Values) {
        $form = $token[1]
        if ($form -cnotmatch '^[\u3400-\u9FFF]{1,4}$') { continue }
        $role = Get-Role $token
        if (!$roles.ContainsKey($form)) { $roles[$form] = @{} }
        $roles[$form][$role] = 1 + $roles[$form][$role]
        $headId = [int]$token[6]
        $childId = [int]$token[0]
        # Preserve a frequent composite adverb when the IME's dictionary has
        # merged two tokens independently attached to the same predicate.
        $following = $rows[$childId + 1]
        if ($role -eq 'predicate_support' -and $following -and $headId -ne 0 -and
            [int]$following[6] -eq $headId -and (Get-Role $following) -eq 'predicate_support') {
            $joined = $form + $following[1]
            if ($joined -cmatch '^[\u3400-\u9FFF]{2,4}$') { $joinedSupport[$joined] = 1 + $joinedSupport[$joined] }
        }
        $distance = [Math]::Abs($headId - $childId)
        if ($distance -lt 1 -or $distance -gt 4 -or !$rows.ContainsKey($headId)) { continue }
        $head = $rows[$headId]
        $rel = Get-Relation $head $token
        if (!$rel) { continue }
        $left, $right = if ($headId -lt $childId) { $head[1], $token[1] } else { $token[1], $head[1] }
        if ($left -cnotmatch '^[\u3400-\u9FFF]{1,4}$' -or $right -cnotmatch '^[\u3400-\u9FFF]{1,4}$') { continue }
        $headOnRight = $headId -gt $childId
        if ($distance -gt 1) {
            # Only count actual annotated edges inside the same unpunctuated
            # span. Other relations are still eligible as adjacent evidence.
            if (!(($rel -eq 'subject' -and $headOnRight) -or
                  ($rel -eq 'object' -and !$headOnRight) -or
                  ($rel -eq 'adnominal' -and $headOnRight))) { continue }
            $blocked = $false
            for ($i = [Math]::Min($headId,$childId) + 1; $i -lt [Math]::Max($headId,$childId); ++$i) {
                if (!$rows.ContainsKey($i) -or (Get-Role $rows[$i]) -eq 'boundary') { $blocked = $true; break }
            }
            if ($blocked) { continue }
        }
        $key = "$left`t$right`t$rel`t$headOnRight"
        if (!$pairs.ContainsKey($key)) { $pairs[$key] = [int[]]@(0,0) }
        $pairs[$key][[int]($distance -gt 1)]++
    }
    $rows.Clear()
}
foreach ($line in [IO.File]::ReadLines((Resolve-Path -LiteralPath $Conllu).Path)) {
    if (!$line.Length) { Add-SentencePairs; continue }
    if ($line.StartsWith('#')) { continue }
    $token = $line.Split("`t")
    if ($token.Length -eq 10 -and $token[0] -cmatch '^[0-9]+$') { $rows[[int]$token[0]] = $token }
}
Add-SentencePairs
foreach ($entry in $joinedSupport.GetEnumerator()) {
    if ($entry.Value -lt 2) { continue }
    if (!$roles.ContainsKey($entry.Key)) { $roles[$entry.Key] = @{} }
    $roles[$entry.Key]['predicate_support'] = $entry.Value + $roles[$entry.Key]['predicate_support']
}
$entries = [Collections.Generic.List[string]]::new()
foreach ($pair in $pairs.GetEnumerator()) {
    $left, $right, $relation, $headOnRight = $pair.Key.Split("`t")
    $adjacent, $cross = $pair.Value
    $count = $adjacent + $cross
    if ($count -lt 2 -or ($left.Length -eq 1 -and $right.Length -eq 1 -and $count -lt 4)) { continue }
    $entries.Add(('{{"{0}", "{1}", Relation::{2}, {3}, {4}, {5}}},' -f $left, $right, $relation, $headOnRight.ToLowerInvariant(), $adjacent, $cross))
}
$entries.Sort([StringComparer]::Ordinal)
$roleEntries = [Collections.Generic.List[string]]::new()
foreach ($item in $roles.GetEnumerator()) {
    $counts = $item.Value
    $total = ($counts.Values | Measure-Object -Sum).Sum
    if ($total -lt 2) { continue }
    $names = [Collections.Generic.List[string]]::new()
    if ([int]$counts['boundary'] * 2 -ge $total) { $names.Add('Role::boundary') }
    else {
        foreach ($name in @('predicate_support', 'aspect', 'quantity', 'adjective', 'nominal', 'link', 'verb')) {
            # Discard rare roles of highly ambiguous words.
            if ($counts[$name] -ge [Math]::Max(1,[Math]::Ceiling($total / 4))) { $names.Add("Role::$name") }
        }
    }
    if ($names.Count) { $roleEntries.Add(('{{"{0}", {1}}},' -f $item.Key, ($names -join ' | '))) }
}
$roleEntries.Sort([StringComparer]::Ordinal)
function Write-Index($path, $records) {
    $lines = @('// SPDX-License-Identifier: CC-BY-SA-4.0', '// Generated by scripts/build-syntax-collocations.ps1. Source and attribution: README.md.') + $records.ToArray()
    [IO.File]::WriteAllText([IO.Path]::GetFullPath($path), ($lines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
}
Write-Index $Output $entries
Write-Index $RoleOutput $roleEntries
Write-Output ("Generated {0} directional dependency records and {1} word-role records." -f $entries.Count,$roleEntries.Count)
