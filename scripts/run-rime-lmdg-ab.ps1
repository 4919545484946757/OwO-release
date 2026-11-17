#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Harness,
    [Parameter(Mandatory)][string]$RimeDll,
    [Parameter(Mandatory)][string]$IceDirectory,
    [Parameter(Mandatory)][string]$WanxiangDirectory,
    [Parameter(Mandatory)][string]$Model,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(2,100)][int]$Rounds = 11,
    [switch]$ReverseOrder
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$utf8 = [Text.UTF8Encoding]::new($false)
$out = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $out) { throw 'Use a new output directory; existing data is never replaced.' }
foreach ($file in @($Harness, $RimeDll, $Model, (Join-Path $IceDirectory 'rime_ice.dict.yaml'),
        (Join-Path $WanxiangDirectory 'wanxiang.dict.yaml'), (Join-Path $WanxiangDirectory 'wanxiang_algebra.yaml'))) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing benchmark asset: $file" }
}
[void](New-Item -ItemType Directory -Path $out)
$fixtures = @(Get-Content -LiteralPath (Join-Path $root 'tests/engine/syntax_collocation_ab.jsonl') -Encoding UTF8 |
    ForEach-Object { $_ | ConvertFrom-Json }) + @(
    @{id='target'; input='zhifujiefei'; expected=@('制服劫匪'); tags=@('homophone')},
    @{id='thief'; input='zhifuxiaotou'; expected=@('制服小偷'); tags=@('homophone')},
    @{id='mob'; input='zhifubaotu'; expected=@('制服暴徒'); tags=@('homophone')},
    @{id='payment'; input='zhifufeiyong'; expected=@('支付费用'); tags=@('homophone')})

$schema = @'
schema:
  schema_id: owo_lmdg_ab
  name: OwO LMDG AB
  version: '1'
switches:
  - name: ascii_mode
    reset: 0
engine:
  processors: [speller, selector, express_editor]
  segmentors: [abc_segmentor, fallback_segmentor]
  translators: [script_translator]
  filters: [uniquifier]
menu:
  page_size: 7
translator:
  dictionary: @DICT@
  enable_user_dict: false
  enable_sentence: true
  enable_correction: false
  max_homophones: 8
  max_sentences: 2
speller:
  alphabet: abcdefghijklmnopqrstuvwxyz
  delimiter: " '"
@ALGEBRA@
'@
$grammar = @'
grammar:
  language: wanxiang-lts-zh-hans
  collocation_max_length: 6
  collocation_min_length: 2
  collocation_penalty: -14
  non_collocation_penalty: -6
  weak_collocation_penalty: -100
  rear_penalty: -18
'@
function Percentile([double[]]$values,[double]$q) {
    $ordered = @($values | Sort-Object)
    return [math]::Round($ordered[[math]::Ceiling($q * $ordered.Count) - 1], 4)
}
function Rank($candidates,$expected) {
    for ($i=0; $i -lt $candidates.Count; ++$i) {
        if ($expected -ccontains $candidates[$i]) { return $i + 1 }
    }
    return $null
}

$codes = for ($i=0; $i -le $Rounds; ++$i) {
    # Rotate the sequence between rounds to avoid always charging a single
    # test for the first model lookup. Round zero is reported but not aggregated.
    for ($j=0; $j -lt $fixtures.Count; ++$j) { $fixtures[($i+$j) % $fixtures.Count].input }
}
$summaries = [Collections.Generic.List[object]]::new()
$sourceAssets = [ordered]@{
    dll=(Get-FileHash -LiteralPath $RimeDll -Algorithm SHA256).Hash;
    model=(Get-FileHash -LiteralPath $Model -Algorithm SHA256).Hash;
    ice=@(Get-Item -LiteralPath (Join-Path $IceDirectory 'rime_ice.dict.yaml');
        Get-ChildItem -LiteralPath (Join-Path $IceDirectory 'cn_dicts') -File -Filter '*.dict.yaml');
    wanxiang=@(Get-Item -LiteralPath (Join-Path $WanxiangDirectory 'wanxiang.dict.yaml'),(Join-Path $WanxiangDirectory 'wanxiang_algebra.yaml');
        Get-ChildItem -LiteralPath (Join-Path $WanxiangDirectory 'dicts') -File -Filter '*.dict.yaml')
}
foreach ($name in 'ice','wanxiang') {
    $sourceAssets[$name] = @($sourceAssets[$name] | Sort-Object Name | ForEach-Object {
        [ordered]@{ name=$_.Name; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
}
[IO.File]::WriteAllText((Join-Path $out 'assets.json'),($sourceAssets | ConvertTo-Json -Depth 4),$utf8)
$priorEncoding = [Console]::OutputEncoding
try {
    [Console]::OutputEncoding = $utf8
    $dictionaryOrder = if ($ReverseOrder) { 'wanxiang','ice' } else { 'ice','wanxiang' }
    $grammarOrder = if ($ReverseOrder) { $true,$false } else { $false,$true }
    foreach ($dictionary in $dictionaryOrder) {
        foreach ($useGrammar in $grammarOrder) {
            $arm = $dictionary + $(if ($useGrammar) {'-on'} else {'-off'})
            $dir = Join-Path $out $arm
            $shared = Join-Path $dir 'shared'
            $user = Join-Path $dir 'user'
            [void](New-Item -ItemType Directory -Path $shared,$user)
            if ($dictionary -eq 'ice') {
                $dict = 'rime_ice'
                Copy-Item -LiteralPath (Join-Path $IceDirectory 'rime_ice.dict.yaml'),(Join-Path $IceDirectory 'cn_dicts') -Destination $shared -Recurse
                $algebra = "  algebra:`n    - abbrev/^([a-z]).+`$/`$1/`n    - abbrev/^(zh|ch|sh).+`$/`$1/"
            } else {
                $dict = 'wanxiang'
                Copy-Item -LiteralPath (Join-Path $WanxiangDirectory 'wanxiang.dict.yaml'),(Join-Path $WanxiangDirectory 'dicts'),(Join-Path $WanxiangDirectory 'wanxiang_algebra.yaml') -Destination $shared -Recurse
                $algebra = "  algebra:`n    __patch: wanxiang_algebra:/base/" + [string][char]0x5168 + [string][char]0x62fc
            }
            $modelName = '-'
            $schemaText = $schema.Replace('@DICT@',$dict).Replace('@ALGEBRA@',$algebra)
            if ($useGrammar) {
                $modelName = 'wanxiang-lts-zh-hans'
                $destination = Join-Path $shared ($modelName + '.gram')
                if ([IO.Path]::GetPathRoot((Resolve-Path -LiteralPath $Model).Path) -eq [IO.Path]::GetPathRoot($destination)) {
                    [void](New-Item -ItemType HardLink -Path $destination -Target (Resolve-Path -LiteralPath $Model).Path)
                } else { Copy-Item -LiteralPath $Model -Destination $destination }
                $schemaText += "`n" + $grammar
            }
            [IO.File]::WriteAllText((Join-Path $shared 'owo_lmdg_ab.schema.yaml'),$schemaText + "`n",$utf8)
            [IO.File]::WriteAllText((Join-Path $shared 'default.yaml'),"config_version: '1'`nschema_list:`n  - schema: owo_lmdg_ab`nmenu:`n  page_size: 7`n",$utf8)
            $err = Join-Path $dir 'stderr.log'
            $lines = @($codes | & $Harness $RimeDll $shared $user owo_lmdg_ab $modelName 2> $err)
            if ($LASTEXITCODE -ne 0) { throw "Harness failed for $arm ($LASTEXITCODE); see $err" }
            [IO.File]::WriteAllLines((Join-Path $dir 'raw.jsonl'),[string[]]$lines,$utf8)
            if ($lines.Count -ne 1 + $codes.Count) { throw "Incomplete candidate output for $arm" }
            if ($useGrammar) {
                $logs = @(Get-ChildItem -LiteralPath $user -File | Where-Object Name -Match 'INFO|WARNING|ERROR' |
                    ForEach-Object { Get-Content -LiteralPath $_.FullName }) + @(Get-Content -LiteralPath $err)
                if (!($logs -match 'gram_db\.cc:\d+\] loading gram db:.*wanxiang-lts-zh-hans\.gram') -or
                    !($logs -match 'gram_db\.cc:\d+\] found double array image of size [1-9]')) {
                    throw "Cannot prove .gram loaded for $arm"
                }
            }
            $records = @($lines | Foreach-Object { $_ | ConvertFrom-Json })
            $rows = foreach ($fixture in $fixtures) {
                $samples = @($records | Where-Object input -CEQ $fixture.input)
                if ($samples.Count -ne $Rounds + 1) { throw "Wrong sample count for $arm / $($fixture.input)" }
                $first = $samples[0]
                if (!$first.first.Count) { throw "No first page for $arm / $($fixture.input)" }
                $expectedRank = Rank $first.top32 $fixture.expected
                if ($samples | Where-Object {(Rank $_.top32 $fixture.expected) -ne $expectedRank}) {
                    throw "Rank not stable across rounds for $arm / $($fixture.input)"
                }
                [ordered]@{ id=$fixture.id; input=$fixture.input; expected=$fixture.expected;
                    first=$first.first; rank=$expectedRank; first_page_rank=(Rank $first.first $fixture.expected) }
            }
            $times = @($records | Select-Object -Skip (1 + $fixtures.Count) | Foreach-Object first_page_ms)
            $summary = [ordered]@{ arm=$arm; runtime=$records[0]; cases=$fixtures.Count;
                top1=@($rows | Where-Object {$_.rank -eq 1}).Count;
                top7=@($rows | Where-Object {$null -ne $_.first_page_rank}).Count;
                top32=@($rows | Where-Object {$null -ne $_.rank}).Count;
                warm_p50_ms=(Percentile $times .5); warm_p95_ms=(Percentile $times .95);
                first_query_ms=$records[1].first_page_ms; target=($rows | Where-Object {$_.id -eq 'target'}) }
            [IO.File]::WriteAllText((Join-Path $dir 'cases.json'),($rows | ConvertTo-Json -Depth 6),$utf8)
            $summaries.Add($summary)
            $summary | ConvertTo-Json -Depth 7 -Compress
        }
    }
} finally { [Console]::OutputEncoding = $priorEncoding }
[IO.File]::WriteAllText((Join-Path $out 'summary.json'),($summaries.ToArray() | ConvertTo-Json -Depth 7),$utf8)
