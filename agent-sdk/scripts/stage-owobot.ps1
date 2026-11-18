#requires -Version 5.1
[CmdletBinding()]
param(
    [string] $NodeArchive = $env:OWOBOT_NODE_ARCHIVE,
    [string] $PnpmCli = $env:OWOBOT_PNPM_CLI
)

$ErrorActionPreference = 'Stop'
$sdkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$upstream = Join-Path $sdkRoot 'owobot\upstream'
$binaries = Join-Path $sdkRoot 'desktop\electron\binaries'
$nodeVersion = '24.22.3'
$pnpmVersion = '10.33.0'
$nodeDirectory = "node-v$nodeVersion-win-x64"
$archiveSha256 = 'E612AF99F626AB415EA225AF6BCE5C971FEB448CB5CAE250BB1C3FA50C5F7D9F'
$tools = Join-Path $sdkRoot 'target\owobot-runtime'
New-Item -ItemType Directory -Path $tools -Force | Out-Null
if (-not $NodeArchive) {
    $NodeArchive = Join-Path $tools "$nodeDirectory.zip"
    if (-not (Test-Path -LiteralPath $NodeArchive -PathType Leaf)) {
        $download = Join-Path $tools (([guid]::NewGuid().ToString('N')) + '.download')
        $url = "https://nodejs.org/dist/v$nodeVersion/$nodeDirectory.zip"
        Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 90 -OutFile $download
        if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $archiveSha256) {
            throw 'Downloaded OwOBot Node.js archive failed SHA-256 verification'
        }
        Move-Item -LiteralPath $download -Destination $NodeArchive -Force
    }
}
$archive = (Resolve-Path -LiteralPath $NodeArchive).Path
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $archiveSha256) {
    throw "OwOBot Node.js archive failed SHA-256 verification: $archive"
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$buildRoot = Join-Path $tools ([guid]::NewGuid().ToString('N'))
$staged = Join-Path $buildRoot 'owobot'
New-Item -ItemType Directory -Path $staged -Force | Out-Null
$nodeExe = Join-Path $staged 'node.exe'
$nodeLicense = Join-Path $buildRoot 'Node-LICENSE.txt'
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    foreach ($item in @(@{ Name = 'node.exe'; Target = $nodeExe }, @{ Name = 'LICENSE'; Target = $nodeLicense })) {
        $entry = $zip.GetEntry("$nodeDirectory/$($item.Name)")
        if (-not $entry) { throw "Node archive is missing $($item.Name)" }
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $item.Target)
    }
} finally {
    $zip.Dispose()
}

if ($PnpmCli) {
    $PnpmCli = (Resolve-Path -LiteralPath $PnpmCli).Path
} else {
    $npm = (Get-Command npm.cmd -ErrorAction Stop).Source
}

function Invoke-OwOBotPnpm([string[]] $PnpmArguments) {
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        if ($PnpmCli) {
            & $nodeExe $PnpmCli @PnpmArguments
        } else {
            & $npm exec --yes "--package=pnpm@$pnpmVersion" -- pnpm @PnpmArguments
        }
        if ($LASTEXITCODE -ne 0) {
            throw "OwOBot pnpm $PnpmArguments failed (exit $LASTEXITCODE)"
        }
    } finally {
        $ErrorActionPreference = $previous
    }
}

$originalPath = $env:Path
Push-Location -LiteralPath $upstream
try {
    $env:Path = "$staged;$originalPath"
    $reportedPnpm = Invoke-OwOBotPnpm @('--version') | Select-Object -Last 1
    if ($reportedPnpm -ne $pnpmVersion) {
        throw "OwOBot requires pnpm $pnpmVersion, found $reportedPnpm"
    }
    Invoke-OwOBotPnpm @('install', '--frozen-lockfile', '--ignore-scripts', '--filter', 'openmausbot')
    Invoke-OwOBotPnpm @('exec', 'vite', 'build', '--config', 'vite.owobot.config.ts')
    Invoke-OwOBotPnpm @('run', 'build:server')
} finally {
    $env:Path = $originalPath
    Pop-Location
}

$sourceCommit = & git -C $sdkRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'OwOBot source commit is unavailable' }
$sourceStatus = & git -C $sdkRoot status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'OwOBot source status is unavailable' }
Copy-Item -LiteralPath (Join-Path $upstream 'dist-owobot') -Destination (Join-Path $staged 'dist') -Recurse
Copy-Item -LiteralPath (Join-Path $upstream 'dist-server') -Destination (Join-Path $staged 'dist-server') -Recurse
$manifest = Get-Content -Raw -LiteralPath (Join-Path $upstream 'package.json') | ConvertFrom-Json
$utf8 = New-Object System.Text.UTF8Encoding($false)
$package = @{ type = 'module'; version = $manifest.version } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $staged 'package.json'), $package, $utf8)

$licenses = Join-Path $staged 'dist\licenses'
New-Item -ItemType Directory -Path $licenses -Force | Out-Null
Copy-Item -LiteralPath $nodeLicense -Destination $licenses
Copy-Item -LiteralPath (Join-Path $sdkRoot '..\LICENSE') -Destination (Join-Path $licenses 'OwO-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $upstream 'LICENSE') -Destination (Join-Path $licenses 'OpenMausBot-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $upstream 'NOTICE') -Destination (Join-Path $licenses 'OpenMausBot-NOTICE.txt')
$thirdParty = Join-Path $upstream 'third_party'
Get-ChildItem -LiteralPath $thirdParty -File -Recurse | Where-Object {
    $_.Name -like '*LICENSE*' -or $_.Name -like '*NOTICE*'
} | ForEach-Object {
    $relative = $_.FullName.Substring($thirdParty.Length + 1)
    $target = Join-Path (Join-Path $licenses 'upstream-components') $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath $_.FullName -Destination $target
}
$identity = [ordered] @{
    sourceCommit = $sourceCommit.Trim()
    sourceDirty = [bool]$sourceStatus
    upstreamCommit = 'a988d43206971d685b4198a6428b881a46035b4a'
    upstreamVersion = $manifest.version
    nodeVersion = $nodeVersion
    nodeArchiveSha256 = $archiveSha256
    nodeSha256 = (Get-FileHash -LiteralPath $nodeExe -Algorithm SHA256).Hash
    pnpmVersion = $pnpmVersion
    lockfileSha256 = (Get-FileHash -LiteralPath (Join-Path $upstream 'pnpm-lock.yaml') -Algorithm SHA256).Hash
    serverSha256 = (Get-FileHash -LiteralPath (Join-Path $staged 'dist-server\index.js') -Algorithm SHA256).Hash
    entrySha256 = (Get-FileHash -LiteralPath (Join-Path $staged 'dist\index.html') -Algorithm SHA256).Hash
} | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $staged 'owobot-build.json'), $identity, $utf8)

New-Item -ItemType Directory -Path $binaries -Force | Out-Null
$binariesRoot = (Resolve-Path -LiteralPath $binaries).Path
if ($binariesRoot -ne [IO.Path]::GetFullPath($binaries)) {
    throw 'OwOBot staging directory resolves outside the desktop binaries directory'
}
$destination = Join-Path $binariesRoot 'owobot'
$previousRuntime = Join-Path $buildRoot 'previous-runtime'
if (Test-Path -LiteralPath $destination) {
    $entry = Get-Item -LiteralPath $destination -Force
    if ($entry.FullName -ne $destination -or ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing to replace a redirected OwOBot staging directory: $destination"
    }
    Move-Item -LiteralPath $destination -Destination $previousRuntime
}
try {
    Move-Item -LiteralPath $staged -Destination $destination
} catch {
    if (Test-Path -LiteralPath $previousRuntime) {
        Move-Item -LiteralPath $previousRuntime -Destination $destination
    }
    throw
}
Write-Host "[OwOBot] Runtime staged at $destination from $sourceCommit"
