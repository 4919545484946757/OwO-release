function Get-OwOPluginPackageName {
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Version)
    if ($Version -cnotmatch '^\d+\.\d+\.\d+$') { throw 'Invalid plugin version for package filename.' }
    $stem = ($Name -replace '[<>:"/\\|?*\p{Cc}\p{Cf}]', '_').Trim(' ', '.')
    if (!$stem -or $stem.Length -gt 128) { throw 'Invalid plugin name for package filename.' }
    if ($stem -match '^(CON|PRN|AUX|NUL|COM[0-9\u00B9\u00B2\u00B3]|LPT[0-9\u00B9\u00B2\u00B3])(?:\.|$)') { $stem = '_' + $stem }
    return "$stem-$Version.owopkg"
}
