$script:OwOUninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OwO Input Method'

function Get-OwOUninstallUserData {
    [IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'OwO\InputMethod'))
}

function Assert-OwOPlainPath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    # Require a fixed drive-rooted path. Reject junctions at every existing
    # component before trusting a path derived from the installation record.
    if ($current -notmatch '^[A-Za-z]:\\' -or $current.Length -le 3) {
        throw "Not a supported local directory: $current"
    }
    while ($current.Length -gt 3) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing to traverse a linked path: $current"
            }
        }
        $current = [IO.Path]::GetDirectoryName($current)
    }
}

function Get-OwOUninstallTarget([string]$ExpectedRoot = '') {
    $entry = Get-ItemProperty -LiteralPath $script:OwOUninstallKey -ErrorAction SilentlyContinue
    $registered = if ($null -ne $entry) { [string]$entry.InstallLocation } else { '' }
    $fallback = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\OwO\InputMethod\0.3.3'
    $root = [IO.Path]::GetFullPath($(if ($registered) { $registered } else { $fallback })).TrimEnd('\')
    Assert-OwOPlainPath $root
    if ($root.Length -le 3 -or ($ExpectedRoot -and -not $root.Equals(
        [IO.Path]::GetFullPath($ExpectedRoot).TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase))) {
        throw 'The active OwO installation changed. Reopen the uninstaller.'
    }
    $data = Get-OwOUninstallUserData
    if ($root.Equals($data, [StringComparison]::OrdinalIgnoreCase) -or
        $root.StartsWith($data + '\', [StringComparison]::OrdinalIgnoreCase) -or
        $data.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The installed program overlaps the user data directory.'
    }
    $pending = $false
    if ($registered -and $entry.PSObject.Properties['OwORemovalPending'] -and
        [string]$entry.OwORemovalPending -match '^[a-f0-9]{32}$') {
        $resumeFile = Join-Path $root '.owo-removal-pending'
        Assert-OwOPlainPath $resumeFile
        $pending = -not (Test-Path -LiteralPath $root) -or
            ((Test-Path -LiteralPath $resumeFile -PathType Leaf) -and
             [IO.File]::ReadAllText($resumeFile) -ceq [string]$entry.OwORemovalPending)
    }
    if (-not $pending) {
        foreach ($relative in @('bin\OwO.TSF.dll', 'scripts\Uninstall-OwO.ps1', 'settings\OwO.Settings.exe')) {
            $marker = Join-Path $root $relative
            Assert-OwOPlainPath $marker
            if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) {
                throw "No verified OwO installation was found at $root"
            }
        }
    }
    [pscustomobject]@{ Root = $root; Pending = [bool]$pending }
}

function Assert-OwOUninstallWorkspace([string]$Directory) {
    $work = [IO.Path]::GetFullPath($Directory).TrimEnd('\')
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
    if (-not [IO.Path]::GetDirectoryName($work).Equals($temp, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($work) -notmatch '^OwO-UninstallUi-[a-f0-9]{32}$') {
        throw 'Refusing to use an invalid uninstall workspace.'
    }
    Assert-OwOPlainPath $work
    $work
}

function Assert-OwOPlainTree([string]$Directory) {
    Assert-OwOPlainPath $Directory
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return }
    foreach ($child in Get-ChildItem -LiteralPath $Directory -Force) {
        if (($child.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to traverse a linked path: $($child.FullName)"
        }
        if ($child.PSIsContainer) { Assert-OwOPlainTree $child.FullName }
    }
}

function Copy-OwOPlainTree([string]$Source, [string]$Destination) {
    Assert-OwOPlainPath $Source
    Assert-OwOPlainPath $Destination
    [void][IO.Directory]::CreateDirectory($Destination)
    foreach ($child in Get-ChildItem -LiteralPath $Source -Force) {
        if (($child.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to copy a linked path: $($child.FullName)"
        }
        $target = Join-Path $Destination $child.Name
        if ($child.PSIsContainer) { Copy-OwOPlainTree $child.FullName $target }
        else { [IO.File]::Copy($child.FullName, $target, $false) }
    }
}

function Remove-OwOOwnedTree([string]$Path, [string]$Boundary) {
    $root = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    $allowed = [IO.Path]::GetFullPath($Boundary).TrimEnd('\')
    if ($root.Length -le 3 -or -not ($root.Equals($allowed, [StringComparison]::OrdinalIgnoreCase) -or
        $root.StartsWith($allowed + '\', [StringComparison]::OrdinalIgnoreCase))) {
        throw "Refusing to remove a path outside the designated OwO directory: $root"
    }
    Assert-OwOPlainPath $root
    if (-not (Test-Path -LiteralPath $root)) { return }
    # Check links as each node is reached, and use nonrecursive filesystem
    # operations. A new/unseen link is never followed by Remove-Item -Recurse.
    $item = Get-Item -LiteralPath $root -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to remove through a linked path: $root"
    }
    if ($item.PSIsContainer) {
        foreach ($child in Get-ChildItem -LiteralPath $root -Force) {
            Remove-OwOOwnedTree $child.FullName $allowed
        }
        [IO.Directory]::Delete($root, $false)
    } else {
        Remove-Item -LiteralPath $root -Force
    }
}
