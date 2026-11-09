[CmdletBinding()]
param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$PublishDirectory) { $PublishDirectory = Join-Path $root 'build/settings-publish/win-x64' }
$fixture = Join-Path $root ('build/model-manager-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$sdk = Join-Path $root 'build/dependencies/dotnet-10.0.403'
$env:DOTNET_ROOT = $sdk
$env:PATH = "$sdk;$env:PATH"
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
& dotnet run --project (Join-Path $root 'tests/settings_center/model_manager_tests.csproj') -c Release --no-restore -- --seed-ui $fixture
if ($LASTEXITCODE) { throw 'Cannot seed isolated synthetic model profile.' }
$env:OWO_CONFIG_PATH = Join-Path $fixture 'config/owo.conf'
$env:OWO_PLUGIN_STORE_PATH = Join-Path $fixture 'plugins'
$process = Start-Process (Join-Path $PublishDirectory 'OwO.Settings.exe') -WindowStyle Hidden -PassThru
$checks = 0
function Find-Id($window, [string]$id) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, [string]$name) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindAll([Windows.Automation.TreeScope]::Descendants, $condition) | Where-Object { !$_.Current.IsOffscreen } | Select-Object -First 1
}
function Check([bool]$value, [string]$label) { if (!$value) { throw $label }; $script:checks++ }
function Invoke($element) { if (!$element) { throw 'Missing button.' }; $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
try {
    $window = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        Start-Sleep -Milliseconds 150
        $process.Refresh()
        if ($process.HasExited) { throw 'Settings exited during startup.' }
        if ($process.MainWindowHandle -ne 0) { $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle) }
    } while (!$window -and [DateTime]::UtcNow -lt $deadline)
    Check ($null -ne $window) 'Settings window did not open.'
    $navigation = Find-Id $window 'ModelManagerNavigationItem'
    Check ($null -ne $navigation) 'Model manager navigation missing.'
    $navigation.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 500
    foreach ($id in @('TextAssistantEnabled','TextAssistantContext','ModelPreferredLanguage','ModelSecondaryLanguage','ModelPolishLevel','ModelCodeLevel','AddModelApi')) {
        $element = Find-Id $window $id
        Check ($null -ne $element) "Missing model control: $id"
    }
    $enabled = Find-Id $window 'TextAssistantEnabled'
    Check ($enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) 'Must default disabled.'
    Invoke (Find-Name $window '编辑')
    Start-Sleep -Milliseconds 250
    foreach ($id in @('ModelApiName','ModelApiEndpoint','ModelApiModel','ModelApiKey')) { Check ($null -ne (Find-Id $window $id)) "Profile editor field missing: $id" }
    $name = Find-Id $window 'ModelApiName'
    $name.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('Fixture edited')
    Invoke (Find-Name $window '保存')
    Start-Sleep -Milliseconds 300
    $config = Get-Content (Join-Path $fixture 'models/profiles.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Check ($config.Profiles[0].Name -eq 'Fixture edited') 'Editing profile did not persist.'
    Check (!$config.Enabled -and !$config.PrivacyConsent) 'Editing must not implicitly enable network.'
    Check (!(Get-Content (Join-Path $fixture 'models/profiles.json') -Raw).Contains('unit-test-not-a-real-api-key')) 'Secret leaked to ordinary JSON.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Check ($null -ne (Find-Name $window '我同意并启用')) 'Explicit privacy consent missing.'
    Invoke (Find-Name $window '取消')
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled'))) 'Cancel must not enable native trigger.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Invoke (Find-Name $window '我同意并启用')
    Start-Sleep -Milliseconds 250
    Check (Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled')) 'Consent did not enable isolated trigger.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled'))) 'Disable did not clear trigger.'
    Write-Output "PASS: $checks model-manager UI checks; synthetic profile only; no model API requests. Fixture: $fixture"
} finally {
    if (!$process.HasExited -and $window) { $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close(); [void]$process.WaitForExit(5000) }
}
