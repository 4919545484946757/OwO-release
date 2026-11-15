[CmdletBinding()]
param([switch]$Quiet, [switch]$RemoveUserData)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Uninstall-Common.ps1')
. (Join-Path $PSScriptRoot 'Install-Prerequisites.ps1')

if ($RemoveUserData -and -not $Quiet) { throw '-RemoveUserData requires the explicit -Quiet option.' }
$target = Get-OwOUninstallTarget
$backend = Join-Path $target.Root 'scripts\Uninstall-OwO.ps1'
$common = Join-Path $target.Root 'scripts\Uninstall-Common.ps1'
if (-not (Test-Path -LiteralPath $backend -PathType Leaf) -or
    -not (Test-Path -LiteralPath $common -PathType Leaf)) {
    throw 'The installed version does not contain the uninstall wizard backend. Use its original uninstaller or upgrade it first.'
}
Assert-OwOPlainPath $backend
Assert-OwOPlainPath $common
$work = Assert-OwOUninstallWorkspace ([IO.Path]::Combine([IO.Path]::GetTempPath(),
    'OwO-UninstallUi-' + [Guid]::NewGuid().ToString('N')))
[void][IO.Directory]::CreateDirectory($work)

$exitCode = 1
try {
    if ($Quiet) {
        $status = Join-Path $work ('progress-' + [Guid]::NewGuid().ToString('N') + '.jsonl')
        $native = Join-Path (Get-OwONativeSystemDirectory) 'WindowsPowerShell\v1.0\powershell.exe'
        $user = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = [Security.Principal.WindowsPrincipal]::new($user)
        $flags = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass' +
            ' -File "' + $backend + '" -Worker -InstallRoot "' + $target.Root +
            '" -ExpectedUserSid "' + $user.User.Value + '" -ProgressPath "' + $status + '"'
        if ($RemoveUserData) { $flags += ' -RemoveUserData' }
        $start = @{ FilePath=$native; ArgumentList=$flags; WorkingDirectory=$work; WindowStyle='Hidden'; Wait=$true; PassThru=$true }
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { $start.Verb = 'RunAs' }
        $process = Start-Process @start
        $exitCode = $process.ExitCode
        if (Test-Path -LiteralPath $status) { Get-Content -LiteralPath $status -Encoding UTF8 }
    } else {
        $app = Join-Path $target.Root 'settings'
        if (-not (Test-Path -LiteralPath (Join-Path $app 'OwO.Settings.exe') -PathType Leaf)) {
            throw 'The installed WinUI uninstaller is unavailable. Run the command with -Quiet to resume with user data preserved.'
        }
        Assert-OwOPlainPath $app
        # Stage the self-contained WinUI runtime outside the program tree so
        # the open wizard does not prevent the worker from finishing deletion.
        $staged = Join-Path $work 'settings'
        Copy-OwOPlainTree $app $staged
        $arguments = '--uninstall-root "' + $target.Root + '" --uninstall-workspace "' + $work + '"'
        $process = Start-Process -FilePath (Join-Path $staged 'OwO.Settings.exe') `
            -ArgumentList $arguments -WorkingDirectory $work -Wait -PassThru
        $exitCode = $process.ExitCode
    }
} finally {
    try { Remove-OwOOwnedTree $work $work }
    catch { Write-Warning "Could not remove the temporary uninstall UI directory: $work" }
}
exit $exitCode
