$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$fixtures = Join-Path $root ('build\setup-packaging-tests-' + [Guid]::NewGuid().ToString('N'))
$utf8 = [Text.UTF8Encoding]::new($false)

foreach ($scenario in @('valid', 'tamper', 'extra', 'changed-manifest', 'traversal', 'duplicate', 'missing-resource')) {
    $output = Join-Path $fixtures $scenario
    $release = Join-Path $output '0.0.0'
    $stage = Join-Path $release 'OwO-Input-Method-0.0.0-windows-x64'
    foreach ($file in @('settings/OwO.Settings.exe', 'settings/OwO.Settings.dll',
            'settings/OwO.Settings.pri', 'settings/App.xbf', 'settings/MainWindow.xbf',
            'settings/MainPage.xbf', 'settings/Assets/AppIcon.ico',
            'settings/Assets/SetupProgressBanner.png', 'scripts/Install-OwO.ps1',
            'scripts/Install-Prerequisites.ps1', 'bin/OwO.TSF.dll', 'bin/x86/OwO.TSF.dll')) {
        $path = Join-Path $stage $file
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
        [IO.File]::WriteAllText($path, 'fixture - never execute', $utf8)
    }
    $lines = @(Get-ChildItem -LiteralPath $stage -File -Recurse | ForEach-Object {
        $relative = $_.FullName.Substring($stage.Length + 1).Replace('\', '/')
        (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + $relative
    })
    if ($scenario -eq 'traversal') { $lines += ('0' * 64) + '  ../outside.txt' }
    if ($scenario -eq 'duplicate') { $lines += $lines[0] }
    $manifest = Join-Path $stage 'SHA256SUMS.txt'
    [IO.File]::WriteAllLines($manifest, $lines, $utf8)
    $zip = Join-Path $release 'OwO-Input-Method-0.0.0-windows-x64.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Fastest, $true)
    [IO.File]::WriteAllText((Join-Path $release 'SHA256SUMS.txt'),
        (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() + '  ' + [IO.Path]::GetFileName($zip), $utf8)
    switch ($scenario) {
        'tamper' { [IO.File]::AppendAllText((Join-Path $stage 'settings/OwO.Settings.dll'), 'tampered', $utf8) }
        'extra' { [IO.File]::WriteAllText((Join-Path $stage 'extra.exe'), 'unverified', $utf8) }
        'changed-manifest' { [IO.File]::AppendAllText($manifest, 'changed', $utf8) }
        'missing-resource' { [IO.File]::Move((Join-Path $stage 'settings/App.xbf'), (Join-Path $stage 'settings/absent.xbf')) }
    }
    $rejected = $false
    try { & (Join-Path $root 'scripts/package_setup.ps1') -Version 0.0.0 -OutputDirectory $output -ValidateOnly | Out-Null }
    catch { $rejected = $true; if ($scenario -eq 'valid') { throw } }
    if ($rejected -eq ($scenario -eq 'valid')) { throw "Unexpected validation result: $scenario (rejected=$rejected)" }
    Write-Output "PASS: $scenario"
}
Write-Output "Setup packaging integrity tests passed (no compiler or installer executed). Fixtures: $fixtures"
