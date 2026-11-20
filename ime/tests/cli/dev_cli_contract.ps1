param([string]$Cli,[string]$Parent,[string]$Repo,[string]$Mode,[string]$ProcessPlugin,[string]$SchemePlugin)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$utf8 = New-Object System.Text.UTF8Encoding($false)
$testParent = [IO.Path]::GetFullPath($Parent)
$testRoot = Join-Path $testParent ('cli contract ' + [Guid]::NewGuid().ToString('N'))
$session = Join-Path $testRoot ([string][char]0x6D4B + ' session')
$pluginIds = New-Object 'System.Collections.Generic.List[string]'
function Invoke-Cli([string[]]$Arguments,[int]$Expected = 0) {
    $raw = @(& $Cli @Arguments)
    $code = $LASTEXITCODE
    if ($code -ne $Expected) { throw "Unexpected exit $code (expected $Expected): $($Arguments -join ' '): $($raw -join ' ')" }
    $objects = @($raw | ForEach-Object { $_ | ConvertFrom-Json })
    if ($objects.Count -eq 0) { throw 'CLI returned no structured result.' }
    return $objects[-1]
}
function Call-Dev([string[]]$Tail,[int]$Expected = 0) { return Invoke-Cli -Arguments (@('--dev','--root',$session) + $Tail) -Expected $Expected }
function Assert-True($Value,[string]$Message) { if (-not $Value) { throw $Message } }
function Make-Plugin([string]$Executable,[string[]]$Permissions) {
    $id = 'owo.plugin.cli-' + [Guid]::NewGuid().ToString('N')
    $pluginIds.Add($id)
    $source = Join-Path $testRoot $id
    [void](New-Item -ItemType Directory -Path (Join-Path $source 'bin'))
    Copy-Item -LiteralPath $Executable -Destination (Join-Path $source 'bin\plugin.exe')
    $manifest = @{ id=$id;name='CLI Test';version='1.0.0';api_version=1;runtime='process';entry='bin/plugin.exe';permissions=@($Permissions);network=$false;config_schema='config.schema.json' }
    [IO.File]::WriteAllText((Join-Path $source 'manifest.json'),($manifest | ConvertTo-Json -Compress),$utf8)
    [IO.File]::WriteAllText((Join-Path $source 'config.schema.json'),'{}',$utf8)
    return @{ id=$id;source=$source }
}
function Install-TestPlugin($Plugin,[string]$Permissions) {
    $preview = Call-Dev @('plugin','inspect','--source',$Plugin.source)
    Assert-True $preview.ok 'Inspection failed.'
    Call-Dev @('plugin','install','--source',$Plugin.source,'--inventory',$preview.inventory,'--accept-risk','I_ACCEPT_PLUGIN_RISK_V1','--accept-untrusted','--permissions',$Permissions) | Out-Null
    Call-Dev @('plugin','activate','--id',$Plugin.id,'--version','1.0.0') | Out-Null
}
try {
    [void](New-Item -ItemType Directory -Path $testRoot)
    if ($Mode -eq 'contract') {
        Invoke-Cli @('--root',$session,'init') 3 | Out-Null
        Assert-True (-not (Test-Path -LiteralPath $session)) 'Rejected mode wrote files.'
        Invoke-Cli @('--dev','--root',$session,'init','--unknown','x') 2 | Out-Null
        Assert-True (-not (Test-Path -LiteralPath $session)) 'Unknown option mutated root.'
        Invoke-Cli @('--dev','--root',(Join-Path $Repo '.GIT\unsafe-cli'),'init') 3 | Out-Null
        Invoke-Cli @('--dev','--root',($session+'.'),'init') 3 | Out-Null
    }
    Call-Dev @('init') | Out-Null
    if ($Mode -eq 'contract') {
        Call-Dev @('init') 3 | Out-Null
        $doctor = Call-Dev @('doctor')
        Assert-True $doctor.isolated 'Missing isolated marker.'
        $configBefore = Call-Dev @('config','show')
        $invalidConfig = Join-Path $testRoot 'invalid.conf'
        [IO.File]::WriteAllText($invalidConfig,'candidate_page_size=999',$utf8)
        Call-Dev @('config','import','--file',$invalidConfig) 2 | Out-Null
        Assert-True ((Call-Dev @('config','show')).contents -eq $configBefore.contents) 'Invalid config changed session settings.'
        Call-Dev @('input','query','--text','nihao','--timeout-ms','oops') 2 | Out-Null
        Call-Dev @('input','parse','--text','123') 1 | Out-Null
        Call-Dev @('input','parse','--text','chnag') | Out-Null
        Call-Dev @('input','bench','--text','nihao','--samples','3') | Out-Null
        $lock = [IO.File]::Open((Join-Path $session '.owo-dev-lock'),[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try { Call-Dev @('doctor') 3 | Out-Null } finally { $lock.Dispose() }
        $one = Call-Dev @('input','query','--text','nihao')
        Assert-True ($one.candidates.Count -gt 0) 'Private Core returned no candidates.'
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $session 'user-frequency.owuf'))) 'Query unexpectedly persisted learning.'
        Call-Dev @('input','replay','--file',(Join-Path $Repo 'tests\data\developer-basic.owotest')) | Out-Null
        $learning = Join-Path $testRoot 'learning.owotest'
        [IO.File]::WriteAllText($learning,"query nihao`nselect 1`n",$utf8)
        Call-Dev @('input','replay','--file',$learning,'--learn') | Out-Null
        Assert-True (Test-Path -LiteralPath (Join-Path $session 'user-frequency.owuf')) 'Explicit learning was not persisted in the isolated session.'
        $bad = Join-Path $testRoot 'bad.owotest'
        [IO.File]::WriteAllText($bad,"query nihao`nexpect wrong`n",$utf8)
        Call-Dev @('input','replay','--file',$bad) 1 | Out-Null
        $payload = Join-Path $testRoot 'bad.json'
        [IO.File]::WriteAllText($payload,'{}',$utf8)
        Call-Dev @('protocol','validate','--kind','agent-request','--file',$payload) 1 | Out-Null
        $sample = Call-Dev @('agent','sample-request','--text','test','--pinyin','ceshi')
        Assert-True ($sample.schema_version -eq 3) 'Agent sample schema mismatch.'
        [IO.File]::WriteAllText($payload,($sample | ConvertTo-Json -Compress -Depth 50),$utf8)
        Call-Dev @('protocol','validate','--kind','agent-request','--file',$payload) | Out-Null
        [IO.File]::WriteAllText($payload,'password=secret123',$utf8)
        $filtered = Call-Dev @('agent','filter','--file',$payload)
        Assert-True ($filtered.redacted -and $filtered.text -notmatch 'secret123') 'Sensitive text was not redacted.'
        Call-Dev @('resource','validate','--kind','theme','--file',(Join-Path $Repo 'plugins\candidate-glass\resources\candidate.theme')) | Out-Null
        Call-Dev @('resource','validate','--kind','palette','--file',(Join-Path $Repo 'plugins\unicode-palette\resources\unicode.palette')) | Out-Null
        [IO.File]::WriteAllText($payload,"b`na`n",$utf8)
        $ranked = Call-Dev @('model','rank','--backend','mock','--text','a','--candidates-file',$payload)
        Assert-True ($ranked.candidates[0] -eq 'a') 'Mock rank result mismatch.'
        Call-Dev @('model','rank','--backend','mock','--text','a','--candidates-file',$payload,'--mock-latency-ms','200','--cancel-after-ms','10') 5 | Out-Null
        Call-Dev @('model','rank','--backend','mock','--text','a','--candidates-file',$payload,'--mock-latency-ms','200','--timeout-ms','10') 5 | Out-Null
        Call-Dev @('plugin','invoke','--id','owo.plugin.nope','--service','example.echo.v1') 3 | Out-Null
        $outside = Join-Path $testRoot 'redirect'
        $junction = Join-Path $session 'redirect'
        [void](New-Item -ItemType Directory -Path $outside)
        [void](New-Item -ItemType Junction -Path $junction -Target $outside)
        try { Call-Dev @('doctor') 3 | Out-Null } finally { [IO.Directory]::Delete($junction) }
        $userData = Join-Path $env:LOCALAPPDATA 'OwO\InputMethod'
        Invoke-Cli @('--dev','--root',$userData,'doctor') 3 | Out-Null
        Call-Dev @('plugin','deactivate','--id','..','--version','1.0.0') 2 | Out-Null
        # Read only daily IDs; the developer CLI must refuse before any mutation.
        $dailyVersions = Join-Path $userData 'plugins\versions'
        if (Test-Path -LiteralPath $dailyVersions -PathType Container) {
            $dailyId = Get-ChildItem -LiteralPath $dailyVersions -Directory | Select-Object -First 1
            if ($dailyId) {
                $blocked = Call-Dev @('plugin','deactivate','--id',$dailyId.Name,'--version','1.0.0') 3
                Assert-True ($blocked.error -match 'daily store') 'Daily plugin collision was not rejected.'
            }
        }
    } elseif ($Mode -eq 'plugins') {
        $process = Make-Plugin $ProcessPlugin @()
        $preview = Call-Dev @('plugin','inspect','--source',$process.source)
        Call-Dev @('plugin','install','--source',$process.source) 3 | Out-Null
        # Even empty permission sets require exact package/disclaimer consent.
        Call-Dev @('plugin','install','--source',$process.source,'--inventory',$preview.inventory,'--accept-risk','I_ACCEPT_PLUGIN_RISK_V1','--accept-untrusted') | Out-Null
        Call-Dev @('plugin','activate','--id',$process.id,'--version','1.0.0') | Out-Null
        $payload = Join-Path $testRoot 'payload.txt'
        [IO.File]::WriteAllText($payload,'hello',$utf8)
        $result = Call-Dev @('plugin','invoke','--id',$process.id,'--service','example.echo.v1','--payload-file',$payload,'--run-plugin','--timeout-ms','5000')
        Assert-True ($result.payload -eq 'hello') 'Echo transport mismatch.'
        [IO.File]::WriteAllBytes($payload,[byte[]]@(255,0,128))
        $binary = Call-Dev @('plugin','invoke','--id',$process.id,'--service','example.echo.v1','--payload-file',$payload,'--run-plugin','--timeout-ms','5000')
        Assert-True ($null -eq $binary.payload -and $binary.payload_hex -eq 'ff0080') 'Binary payload transport mismatch.'
        [IO.File]::WriteAllText($payload,'1000',$utf8)
        Call-Dev @('plugin','invoke','--id',$process.id,'--service','example.delay.v1','--payload-file',$payload,'--run-plugin','--timeout-ms','5000','--cancel-after-ms','30') 5 | Out-Null
        Call-Dev @('plugin','invoke','--id',$process.id,'--service','example.disconnect.v1','--run-plugin','--timeout-ms','1000') 5 | Out-Null
        $scheme = Make-Plugin $SchemePlugin @('input.scheme')
        Install-TestPlugin $scheme 'input.scheme'
        $schemes = Call-Dev @('scheme','list','--id',$scheme.id,'--run-plugin')
        Assert-True ($schemes.schemes.Count -eq 1) 'Scheme discovery mismatch.'
        $answer = Call-Dev @('scheme','query','--id',$scheme.id,'--scheme',($scheme.id+'/table'),'--text','a','--page-size','2','--run-plugin')
        Assert-True ($answer.candidates.Count -eq 2 -and $answer.has_more) 'Scheme paging mismatch.'
        Call-Dev @('scheme','query','--id',$scheme.id,'--scheme',($scheme.id+'/table'),'--text','a','--page-size','2','--page','1','--run-plugin') | Out-Null
        Call-Dev @('scheme','query','--id',$scheme.id,'--scheme',($scheme.id+'/table'),'--text','a','--select','1','--run-plugin') | Out-Null
        Call-Dev @('scheme','query','--id',$scheme.id,'--scheme',($scheme.id+'/table'),'--text','a','--sensitive','--run-plugin') 5 | Out-Null
        Call-Dev @('plugin','revoke','--id',$scheme.id,'--version','1.0.0') | Out-Null
        Call-Dev @('scheme','list','--id',$scheme.id,'--run-plugin') 5 | Out-Null
    } else { throw 'Unknown test mode.' }
    Write-Output 'Developer CLI contract passed.'
} finally {
    foreach ($id in $pluginIds) {
        & $Cli --dev --root $session plugin deactivate --id $id --version 1.0.0 | Out-Null
        & $Cli --dev --root $session plugin uninstall --id $id --version 1.0.0 | Out-Null
    }
    $resolved = [IO.Path]::GetFullPath($testRoot)
    if ($resolved.StartsWith(($testParent.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolved -Leaf).StartsWith('cli contract ')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    } else { throw 'Refusing cleanup outside dedicated test directory.' }
}
