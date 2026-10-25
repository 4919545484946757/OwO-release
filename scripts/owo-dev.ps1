$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cliPath = Join-Path $repoRoot 'build\windows-developer\Release\owo_dev_cli.exe'
$cliArguments = @($args)
try {
    if ($cliArguments.Count -gt 0 -and $cliArguments[0] -eq 'build') {
        if ($cliArguments.Count -ne 1) { throw 'build accepts no other arguments.' }
        Push-Location -LiteralPath $repoRoot
        try {
            & cmake --preset windows-developer
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            & cmake --build --preset windows-developer --target owo_dev_cli --parallel 4 -- /nodeReuse:false
            exit $LASTEXITCODE
        } finally { Pop-Location }
    }
    if ($cliArguments.Count -gt 0 -and $cliArguments[0] -eq 'test') {
        if ($cliArguments.Count -ne 3 -or $cliArguments[1] -ne '--suite') { throw 'Usage: owo-dev test --suite cli|engine|plugin|scheme|agent' }
        $testSuites = @{
            cli = '^owo\.dev_cli\.'
            engine = '^owo\.engine\.(full_pinyin|ambiguous_boundaries|dictionary_bridges|phrase_chains|candidate_partition|user_word_retention|local_corrections|candidate_search_budget|lookup_reuse|binary_lexicon|candidate_scheduler)$'
            plugin = '^owo\.plugin\.'
            scheme = '^owo\.(engine\.input_method|core\.input_method|ipc\.input_method|tsf\.input_method)'
            agent = '^owo\.agent\.'
        }
        $suiteName = $cliArguments[2]
        if (-not $testSuites.ContainsKey($suiteName)) { throw 'Unknown test suite.' }
        $buildDir = Join-Path $repoRoot 'build\windows-developer'
        if (-not (Test-Path -LiteralPath $cliPath -PathType Leaf)) { throw 'Run owo-dev build first.' }
        Push-Location -LiteralPath $repoRoot
        try {
            # Build test executables in this developer tree, not the user's runtime.
            if ($suiteName -eq 'cli') {
                & cmake --build --preset windows-developer --target owo_dev_cli owo_example_process_plugin owo_example_input_method_plugin --parallel 4 -- /nodeReuse:false
            } else {
                & cmake --build --preset windows-developer --parallel 4 -- /nodeReuse:false
            }
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            & ctest --test-dir $buildDir -C Release -R $testSuites[$suiteName] --output-on-failure --no-tests=error
            exit $LASTEXITCODE
        } finally { Pop-Location }
    }
    if (-not (Test-Path -LiteralPath $cliPath -PathType Leaf)) { throw 'Developer CLI is not built. Run owo-dev build first.' }
    & $cliPath @cliArguments
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
}
