param(
    [Parameter(Mandatory=$true)][string]$Baseline,
    [Parameter(Mandatory=$true)][string]$Candidate,
    [switch]$FailOnRegression
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
function Read-Report([string]$Directory) {
    $directoryPath = [IO.Path]::GetFullPath($Directory)
    $summary = Get-Content -LiteralPath (Join-Path $directoryPath 'summary.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $summary.complete -or $summary.schema_version -ne 1 -or $summary.metrics.errors -ne 0) { throw 'Report is incomplete, incompatible, or contains runtime errors.' }
    $recordPath = Join-Path $directoryPath 'cases.jsonl'
    if ((Get-Item -LiteralPath $recordPath).Length -gt 268435456) { throw 'Comparison report exceeds 256 MiB.' }
    $cases = @{}; $manifest = $null; $completed = $false
    foreach ($line in [IO.File]::ReadLines($recordPath,[Text.Encoding]::UTF8)) {
        $record = $line | ConvertFrom-Json
        if ($record.schema_version -ne 1) { throw 'Unknown JSONL record schema.' }
        switch ($record.record) {
            manifest { if ($manifest) { throw 'Duplicate manifest.' }; $manifest = $record }
            case { if ($cases.ContainsKey($record.id)) { throw 'Duplicate case ID.' }; $cases[$record.id] = $record }
            summary { $completed = $record.complete -and $record.metrics.cases -eq $summary.metrics.cases }
            default { throw 'Unknown JSONL record kind.' }
        }
    }
    if (-not $manifest -or -not $completed -or $cases.Count -ne $summary.metrics.cases) { throw 'Report record count/completion mismatch.' }
    return @{ summary=$summary; manifest=$manifest; cases=$cases }
}
try {
    $left = Read-Report $Baseline; $right = Read-Report $Candidate
    if ($left.cases.Count -ne $right.cases.Count) { throw 'Case sets differ; compare the same dataset.' }
    foreach ($field in @('model','mode','typing','top_k','max_pages','repeat','check_expanded','learning')) {
        if ($left.manifest.$field -cne $right.manifest.$field) { throw "Evaluation policy differs: $field" }
    }
    # Validate the complete corpus before emitting any apparent improvements.
    foreach ($id in $left.cases.Keys) {
        if (-not $right.cases.ContainsKey($id)) { throw "Missing candidate case: $id" }
        if (($left.cases[$id].test | ConvertTo-Json -Depth 10 -Compress) -cne ($right.cases[$id].test | ConvertTo-Json -Depth 10 -Compress)) { throw "Case labels/input/context differ: $id" }
    }
    $regressions = 0; $improvements = 0; $changed = 0
    foreach ($id in @($left.cases.Keys | Sort-Object)) {
        $before = $left.cases[$id]; $after = $right.cases[$id]
        $regressed = ($before.pass -and -not $after.pass)
        $improved = (-not $before.pass -and $after.pass)
        $rankField = if ($left.manifest.model -eq 'base') { 'base_rank' } else { 'final_first_page_rank' }
        $oldRank = if ($null -eq $before.$rankField) { [int]::MaxValue } else { [int]$before.$rankField }
        $newRank = if ($null -eq $after.$rankField) { [int]::MaxValue } else { [int]$after.$rankField }
        $regressed = $regressed -or $newRank -gt $oldRank
        $improved = $improved -or $newRank -lt $oldRank
        if ($regressed) { $regressions++ }; if ($improved) { $improvements++ }
        if ($regressed -or $improved) {
            $changed++
            @{schema_version=1;record='comparison_case';id=$id;input=$after.input;regression=$regressed;improvement=$improved;before_rank=$before.$rankField;after_rank=$after.$rankField;before_pass=$before.pass;after_pass=$after.pass} | ConvertTo-Json -Compress -Depth 10
        }
    }
    @{schema_version=1;record='comparison_summary';ok=(-not $FailOnRegression -or $regressions -eq 0);cases=$left.cases.Count;changed=$changed;regressions=$regressions;improvements=$improvements;baseline_metrics=$left.summary.metrics;candidate_metrics=$right.summary.metrics;baseline_config=$left.manifest.effective_config;candidate_config=$right.manifest.effective_config;baseline_lexicon=$left.manifest.lexicon;candidate_lexicon=$right.manifest.lexicon;baseline_cli=$left.manifest.cli_asset;candidate_cli=$right.manifest.cli_asset;baseline_bridge=$left.manifest.bridge;candidate_bridge=$right.manifest.bridge;baseline_model=$left.manifest.model_asset;candidate_model=$right.manifest.model_asset} | ConvertTo-Json -Compress -Depth 12
    if ($FailOnRegression -and $regressions) { exit 1 }
    exit 0
} catch {
    @{schema_version=1;record='comparison_error';ok=$false;error=$_.Exception.Message} | ConvertTo-Json -Compress
    exit 2
}
