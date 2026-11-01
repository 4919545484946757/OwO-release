param([string]$SdkVersion='10.0.26100.0',[string]$CompilerDirectory)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$sdk=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
$midl=Join-Path $sdk "bin/$SdkVersion/x64/midl.exe"
if(!(Test-Path -LiteralPath $midl)){throw 'Existing Windows SDK MIDL is required; nothing will be installed'}
if(!$CompilerDirectory){
    $compiler=Get-ChildItem "${env:ProgramFiles}/Microsoft Visual Studio/2022/*/VC/Tools/MSVC/*/bin/Hostx64/x64/cl.exe" |
        Sort-Object FullName -Descending | Select-Object -First 1
    if(!$compiler){throw 'Existing MSVC compiler is required'}
    $CompilerDirectory=Split-Path -Parent $compiler.FullName
}
$output=Join-Path $root 'include/owo/compatibility/ia2'
$source=Join-Path $root 'third_party/ia2/api'
$scratch=Join-Path $root 'build/ia2-generation'
[void](New-Item -ItemType Directory -Path $output,$scratch -Force)
$savedCompilerPath=$env:PATH
try{
    # Child-tool search path only; no persistent user or system environment write.
    $env:PATH=$CompilerDirectory+';'+$savedCompilerPath
    foreach($name in @('AccessibleStates','IA2CommonTypes','AccessibleRelation','Accessible2')){
        & $midl /nologo /env x64 /cpp_cmd cl.exe /I $source /I (Join-Path $sdk "Include/$SdkVersion/um") `
            /I (Join-Path $sdk "Include/$SdkVersion/shared") /out $scratch (Join-Path $source "$name.idl")
        if($LASTEXITCODE -ne 0){throw "MIDL failed: $name"}
        $header=[IO.File]::ReadAllText((Join-Path $scratch "$name.h"))
        # SDK imports already declare these marshal routines with authoritative
        # SAL. MIDL's duplicate, unannotated prototypes produce C28251; no proxy
        # source is built here, so keep the SDK declarations instead.
        $header=[regex]::Replace($header,'(?s)/\* Additional Prototypes for ALL interfaces \*/.*?/\* end of Additional Prototypes \*/',
            '/* User-marshal routines are declared by the imported Windows SDK headers. */')
        [IO.File]::WriteAllText((Join-Path $output "$name.h"),$header,[Text.UTF8Encoding]::new($false))
    }
}finally{$env:PATH=$savedCompilerPath}
