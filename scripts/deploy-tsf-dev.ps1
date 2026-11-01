param([ValidateSet('Debug','Release')][string]$Configuration='Release',
      [ValidateSet('x64','x86')][string[]]$Architecture=@('x64','x86'))
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'tsf-dev-image.ps1')
foreach($arch in $Architecture){
    $preset=if($arch -eq 'x64'){'windows-'+$Configuration.ToLowerInvariant()}else{'windows-x86-'+$Configuration.ToLowerInvariant()}
    $source=Join-Path $root "build/$preset/$Configuration/OwO.TSF.dll"
    Assert-TsfImageArchitecture -Path $source -Architecture $arch
    # Stable COM path avoids accumulating hashed paths in application registry
    # views. Updating a mapped DLL is refused by Windows, never forced.
    $directory=Join-Path $root "build/tsf-dev/$arch"
    [void](New-Item -ItemType Directory -Path $directory -Force)
    $target=Join-Path $directory 'OwO.TSF.dll'
    Copy-Item -LiteralPath (Join-Path $root 'third_party/ia2/LICENSE') -Destination (Join-Path $directory 'IAccessible2.LICENSE.txt') -Force
    if(!(Test-Path -LiteralPath $target) -or (Get-FileHash $source).Hash -ne (Get-FileHash $target).Hash){
        if(Test-Path -LiteralPath $target){
            $backup=Join-Path $directory ('backup-'+[guid]::NewGuid().ToString('N')+'.dll')
            Copy-Item -LiteralPath $target -Destination $backup
        }
        try{Copy-Item -LiteralPath $source -Destination $target -Force}
        catch{throw "Close applications loading $target before deployment. No process was killed. $_"}
    }
    & (Join-Path $PSScriptRoot 'register-dev.ps1') -Configuration $Configuration -Architecture $arch -DllPath $target
    Write-Output "$arch $target SHA256=$((Get-FileHash $target).Hash)"
}
