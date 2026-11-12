[CmdletBinding()]
param([switch]$AcceptDevelopmentPackageRisk, [string]$StorePath, [string]$BuildDirectory)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$build=if($BuildDirectory){[IO.Path]::GetFullPath($BuildDirectory)}else{Join-Path $root 'build/windows-release'}
& cmake --build $build --config Release --target owo_wubi86_package owo_plugin_shell
if($LASTEXITCODE){throw 'Wubi Release build failed'}
$store=if($StorePath){[IO.Path]::GetFullPath($StorePath)}else{Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'OwO/InputMethod/plugins'}
$shell=Join-Path $build 'Release/owo_plugin_shell.exe'
$stage=Join-Path $build 'wubi86-plugin/Release'
$preview=& $shell $store inspect-install $stage | ConvertFrom-Json
if($LASTEXITCODE -or !$preview.ok){throw 'Package inspection failed'}
$installed=Join-Path $store ("versions/{0}/{1}" -f $preview.plugin_id,$preview.version)
$same=$false
if(Test-Path -LiteralPath $installed){
 $existing=& $shell $store inspect-install $installed | ConvertFrom-Json
 if($LASTEXITCODE -or !$existing.ok){throw 'Installed package inspection failed'}
 $same=$existing.inventory_sha256 -eq $preview.inventory_sha256
 if(!$same){throw 'Installed version has different contents; bump the plugin version before deploying, preserving existing data'}
}
if(!$same){
if($preview.requires_risk_consent){
 if(!$AcceptDevelopmentPackageRisk){throw 'Unsigned development package requires explicit -AcceptDevelopmentPackageRisk; no permissions changed'}
 & $shell $store install-risk $stage $preview.inventory_sha256 1 I_ACCEPT_PLUGIN_RISK_V1
}else{& $shell $store install $stage}
if($LASTEXITCODE){throw 'Plugin installation failed'}
}
$snapshot=& $shell $store list | ConvertFrom-Json
if($LASTEXITCODE){throw 'Cannot read installed plugin state'}
$version=$snapshot.plugins | Where-Object { $_.id -eq $preview.plugin_id -and $_.version -eq $preview.version } | Select-Object -First 1
if(!$version){throw 'Published plugin missing from store'}
if(!$version.permissions_authorized -and !$AcceptDevelopmentPackageRisk){throw 'Existing plugin permission is revoked; explicit authorization is required'}
if(!$version.active -or !$version.permissions_authorized){
 & $shell $store activate $preview.plugin_id $preview.version
 if($LASTEXITCODE){throw 'Plugin activation failed'}
}
$schemes=& $shell $store input-schemes
if($LASTEXITCODE){throw 'Plugin discovery failed'}

if(!(($schemes | ConvertFrom-Json).schemes | Where-Object id -eq 'plugin:owo.plugin.wubi86/wubi86')){throw 'Wubi descriptor missing after activation'}
$schemes
