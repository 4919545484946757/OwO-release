
$ErrorActionPreference = 'Stop'
. 'T:\创新创业\OwO-master\agent-sdk\scripts\resolve-ort.ps1'
Resolve-OwoOrtEnv -Quiet | Out-Null
. 'T:\创新创业\OwO-master\agent-sdk\scripts\ci-shared.ps1'
$env:RUST_TEST_THREADS = '1'
$taskCwd = 'C:\Users\23843\AppData\Local\Temp\owo-audit-iteration-20261025'
$common = @('--manifest-path','T:\创新创业\OwO-master\agent-sdk\Cargo.toml','--config','T:\创新创业\OwO-master\agent-sdk\.cargo\config.toml','--target-dir','C:\Users\23843\AppData\Local\Temp\owo-audit-iteration-20261025\native-target','--locked','-j','1')
$phases = @(
 @{Name='fmt'; Args=@('fmt','--manifest-path','T:\创新创业\OwO-master\agent-sdk\Cargo.toml','--all','--','--check')},
 @{Name='check-core'; Args=@('check')+$common+@('-p','owo-agent-core','--lib')},
 @{Name='check'; Args=@('check')+$common+@('-p','owo-agent-server','--lib','--test','media_execution_api_tests','--test','route_contract_tests')},
 @{Name='check-cli'; Args=@('check')+$common+@('-p','owo-agent-cli','--bin','owo-agent')},
 @{Name='native-execution'; Args=@('test')+$common+@('-p','owo-agent-server','--test','media_execution_api_tests','--','--test-threads=1','--nocapture')},
 @{Name='native-core-media'; Args=@('test')+$common+@('-p','owo-agent-core','--lib','tools::media','--','--test-threads=1')},
 @{Name='native-media-contracts'; Args=@('test')+$common+@('-p','owo-agent-server','--test','route_contract_tests','media','--','--test-threads=1')},
 @{Name='clippy-core'; Args=@('clippy')+$common+@('-p','owo-agent-core','--lib','--','-D','warnings')},
 @{Name='clippy-server'; Args=@('clippy')+$common+@('-p','owo-agent-server','--lib','--test','media_execution_api_tests','--test','route_contract_tests','--','-D','warnings')},
 @{Name='clippy-cli'; Args=@('clippy')+$common+@('-p','owo-agent-cli','--bin','owo-agent','--','-D','warnings')}
)
foreach ($phase in $phases) {
 Write-Host ('PHASE_START '+$phase.Name)
 $taskLog=Join-Path $taskCwd ('media-invalid-content-final-20261027-'+$phase.Name+'.log')
 $taskExit=Invoke-CiCargo -Arguments $phase.Args -Cwd $taskCwd -PolicyMode strict -PassThru -HeartbeatSec 30 -LogFile $taskLog
 Write-Host ('PHASE_EXIT '+$phase.Name+'='+$taskExit)
 if ($taskExit -ne 0) {exit $taskExit}
}
