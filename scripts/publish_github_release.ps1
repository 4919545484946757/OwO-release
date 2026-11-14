[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [string]$ReleaseDirectory,
    [string]$ResourceList,
    [string]$ReleaseNotesFile,
    [switch]$UseGitCredentialManager,
    [switch]$ResumeDraft,
    [switch]$Publish
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest
$repository = '4919545484946757/OwO-release'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$ReleaseDirectory) { $ReleaseDirectory = Join-Path $root "artifacts\release\$Version" }
$directory = [IO.Path]::GetFullPath($ReleaseDirectory)
$receipt = Join-Path $directory 'release-build.json'
if (!(Test-Path -LiteralPath $receipt -PathType Leaf)) { throw 'Missing release-build.json. Rebuild a fresh release; old payloads must not be relabeled.' }
$build = Get-Content -LiteralPath $receipt -Raw -Encoding UTF8 | ConvertFrom-Json
if ($build.version -ne $Version -or $build.git_commit -notmatch '^[a-f0-9]{40}$' -or $build.dirty) { throw 'Release build provenance is invalid or was built from a dirty tree.' }
$commit = [string]$build.git_commit
$current = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $current -ne $commit) { throw 'Release was not built from the current commit.' }
$names = @("OwO-Input-Method-$Version-windows-x64-Setup.exe", "OwO-Input-Method-$Version-windows-x64.zip", "OwO-Input-Method-$Version-source.zip")
$resources = @()
$items = @()
if ($ResourceList) {
    $items = Get-Content -LiteralPath $ResourceList -Raw -Encoding UTF8 | ConvertFrom-Json
    $ids = @{}
    $assetNames = @{}
    if (@($items).Count -gt 100) { throw 'Too many resources.' }
    foreach ($item in $items) {
        if ($item.id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]+$' -or $ids.ContainsKey($item.id) -or
            $item.kind -notin @('plugin','dictionary','model') -or [string]::IsNullOrWhiteSpace($item.name) -or
            [string]::IsNullOrWhiteSpace($item.license) -or [string]::IsNullOrWhiteSpace($item.version) -or
            $item.name.Length -gt 128 -or $item.license.Length -gt 256 -or $item.version.Length -gt 64 -or $item.id.Length -gt 180 -or
            $item.asset -notmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]+$' -or $item.asset.Length -gt 180 -or $item.asset.Contains('..') -or
            $item.asset -in @('owo-resources.json','release-build.json','SHA256SUMS.txt') -or $item.asset -in $names -or $assetNames.ContainsKey($item.asset)) { throw 'Invalid resource metadata.' }
        if (($item.kind -eq 'plugin' -and $item.asset -notmatch '\.owopkg$') -or
            ($item.kind -eq 'dictionary' -and $item.asset -notmatch '\.(json|txt|tsv|csv|yaml|yml|zip)$') -or
            ($item.kind -eq 'model' -and $item.asset -notmatch '\.lm$')) { throw 'Unsupported resource format.' }
        $ids[$item.id] = $true
        $assetNames[$item.asset] = $true
        $localName = if ($item.PSObject.Properties['local_file']) { [string]$item.local_file } else { [string]$item.asset }
        if ($item.PSObject.Properties['local_file'] -and ($item.kind -ne 'plugin' -or
            $localName -notmatch '^[^<>:"/\\|?*\p{Cc}\p{Cf}]{1,150}\.owopkg$' -or
            $localName.StartsWith('.') -or $names -contains $item.asset)) { throw 'Invalid local plugin filename.' }
        $path = Join-Path $directory $localName
        if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing resource asset: $($item.asset)" }
        if ($item.kind -eq 'dictionary' -and (Get-Item -LiteralPath $path).Length -gt 128MB) { throw 'Dictionary exceeds the 128 MiB import limit.' }
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $resources += [ordered]@{id=$item.id; name=$item.name; kind=$item.kind; version=$item.version; license=$item.license; asset=$item.asset; sha256=$hash}
        $names += $item.asset
    }
}
$names = @($names | Select-Object -Unique)
if ($names.Count + 3 -gt 100) { throw 'Too many release assets for the supported catalog.' }
$receiptLines = @(Get-Content -LiteralPath (Join-Path $directory 'SHA256SUMS.txt') -Encoding UTF8)
$assets = foreach ($name in $names) {
    $resourceInput = @($items | Where-Object { $_.asset -eq $name -and $_.PSObject.Properties['local_file'] })
    $label = if ($resourceInput.Count) { [string]$resourceInput[0].local_file } else { '' }
    $path = Join-Path $directory $(if ($label) { $label } else { $name })
    $file = Get-Item -LiteralPath $path
    if ($file.Length -le 0 -or $file.Length -gt 2GB -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid asset size or linked file.' }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($name -like "OwO-Input-Method-$Version-*") {
        if ($receiptLines -notcontains "$hash  $name") { throw "Checksum receipt mismatch: $name" }
    }
    [pscustomobject]@{name=$name; path=$path; size=$file.Length; sha256=$hash; label=$label}
}
$tag = "v$Version"
$notes = "Windows 11 x64. Includes x86 TSF compatibility. Verify SHA256SUMS.txt before installation. Source and third-party licenses are included."
if ($ReleaseNotesFile) {
    if ((Get-Item -LiteralPath $ReleaseNotesFile).Length -gt 128KB) { throw 'Release notes are too large.' }
    # Get-Content decorates strings with PSPath/PSDrive in Windows PowerShell
    # 5.1; ConvertTo-Json can serialize those as an object instead of text.
    $notes = [IO.File]::ReadAllText([IO.Path]::GetFullPath($ReleaseNotesFile), [Text.Encoding]::UTF8)
}
$body = @{tag_name=$tag;target_commitish=$commit;name="OwO $Version";draft=$true;prerelease=$false;body=$notes} | ConvertTo-Json
if (($body | ConvertFrom-Json).body -isnot [string]) { throw 'Release notes must serialize as plain JSON text.' }
Write-Output "Release plan: $repository $tag commit=$commit"
$assets | Select-Object name, size, sha256 | Format-Table -AutoSize
if (!$Publish) { Write-Output 'Validation only. No GitHub writes. Use -Publish after reviewing the plan.'; return }
if (@(& git -C $root status --porcelain).Count -ne 0) { throw 'Commit or preserve all pending changes before publishing.' }
$token = [Environment]::GetEnvironmentVariable('GH_TOKEN')
if (!$token) { $token = [Environment]::GetEnvironmentVariable('GITHUB_TOKEN') }
if (!$token -and $UseGitCredentialManager) {
    # Only use existing credentials for the explicitly selected release repo.
    # Never print helper output, start an interactive login or persist a token.
    $oldInteractive = $env:GCM_INTERACTIVE; $oldPrompt = $env:GIT_TERMINAL_PROMPT
    try {
        $env:GCM_INTERACTIVE = 'Never'; $env:GIT_TERMINAL_PROMPT = '0'
        $credentials = "protocol=https`nhost=github.com`npath=$repository.git`n`n" | & git -C $root credential fill 2>$null
        if ($LASTEXITCODE -eq 0) {
            foreach ($line in $credentials) { if ($line.StartsWith('password=')) { $token = $line.Substring(9) } }
        }
    } finally {
        $credentials = $null; $line = $null
        $env:GCM_INTERACTIVE = $oldInteractive; $env:GIT_TERMINAL_PROMPT = $oldPrompt
    }
}
if (!$token) { throw 'Publishing requires GH_TOKEN or GITHUB_TOKEN with repository Contents:write. Never store tokens in the repository or application.' }
$headers = @{ Authorization="Bearer $token"; Accept='application/vnd.github+json'; 'User-Agent'='OwO-release-publisher'; 'X-GitHub-Api-Version'='2026-03-10' }
$api = "https://api.github.com/repos/$repository"
# This also verifies that the source commit was pushed to the release repository.
$null = Invoke-RestMethod -Uri "$api/commits/$commit" -Headers $headers
$existing = $null
try { $existing = Invoke-RestMethod -Uri "$api/releases/tags/$tag" -Headers $headers }
catch { if ([int]$_.Exception.Response.StatusCode -ne 404) { throw 'Cannot verify whether the release tag already exists.' } }
if ($existing -and (!$ResumeDraft -or !$existing.draft -or $existing.target_commitish -ne $commit)) {
    throw 'Release tag already exists. Only an explicit resume of a draft for this exact commit is allowed; published assets are never overwritten.'
}
$catalog = [ordered]@{schema_version=1; release_tag=$tag; resources=@($resources)} | ConvertTo-Json -Depth 8
$catalogPath = Join-Path $directory 'owo-resources.json'
[IO.File]::WriteAllText($catalogPath, $catalog, [Text.UTF8Encoding]::new($false))
$assets += [pscustomobject]@{name='owo-resources.json';path=$catalogPath;size=(Get-Item -LiteralPath $catalogPath).Length;sha256=(Get-FileHash -LiteralPath $catalogPath -Algorithm SHA256).Hash.ToLowerInvariant();label=''}
$assets += foreach ($extra in @('SHA256SUMS.txt','release-build.json')) {
    $path = Join-Path $directory $extra
    [pscustomobject]@{name=$extra;path=$path;size=(Get-Item -LiteralPath $path).Length;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant();label=''}
}
if ($existing) { $release = $existing; Write-Output "Resuming verified draft: $($release.html_url)" }
else {
    $release = Invoke-RestMethod -Uri "$api/releases" -Method Post -Headers $headers -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body))
    Write-Output "Draft created: $($release.html_url)"
}
foreach ($asset in $assets) {
    $prior = @($release.assets | Where-Object name -eq $asset.name)
    if ($prior.Count -gt 0) {
        if ($prior.Count -ne 1 -or $prior[0].size -ne $asset.size -or $prior[0].digest -ne "sha256:$($asset.sha256)" -or
            ($asset.label -and $prior[0].label -cne $asset.label)) {
            throw 'Existing draft asset does not match; refusing to overwrite or delete it.'
        }
        Write-Output "Verified existing asset: $($asset.name)"; continue
    }
    Write-Output "Uploading: $($asset.name) ($($asset.size) bytes)"
    $upload = "https://uploads.github.com/repos/$repository/releases/$($release.id)/assets?name=$([Uri]::EscapeDataString($asset.name))"
    if ($asset.label) { $upload += "&label=$([Uri]::EscapeDataString($asset.label))" }
    $sent = Invoke-RestMethod -Uri $upload -Method Post -Headers $headers -ContentType 'application/octet-stream' -InFile $asset.path
    if ($sent.name -cne $asset.name -or $sent.size -ne $asset.size -or $sent.digest -ne "sha256:$($asset.sha256)" -or
        ($asset.label -and $sent.label -cne $asset.label)) { throw 'Uploaded asset verification failed. The draft is retained and was NOT published.' }
}
$published = Invoke-RestMethod -Uri "$api/releases/$($release.id)" -Method Patch -Headers $headers -ContentType 'application/json' -Body '{"draft":false,"make_latest":"true"}'
Write-Output "Published: $($published.html_url)"
