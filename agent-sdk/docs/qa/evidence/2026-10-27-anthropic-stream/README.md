# 本地 Anthropic HTTP / Worker 检查点证据

日期：2026-10-27。对应 [专项报告](../../../reports/Anthropic请求与流式完整性修复-2026-10-27.md)。

- before.log：实际修复前 8 项 HTTP 回归全部失败，exit=101；不是编译失败。
- check.log：冻结源码的定向 Core 与两个 HTTP 测试目标编译检查，exit=0。
- native.log：5 项请求路由/用量 + 13 项流式完整性回归通过，exit=0。
- unit.log：8 项 Anthropic 单元测试通过，exit=0。
- checkpoint.log：4 项 Worker 检查点测试通过，exit=0。
- clippy.log：Core lib 与两个 HTTP 目标，-D warnings，exit=0。
- result.json：完整 Cargo 参数、真实退出码、范围、耗时、源码与二进制 SHA-256。
- source-identity.json：634 项 Rust/manifest/lock/config 输入前后快照，变化列表为空。

源代码以基线 HEAD 加记录的本地修改及哈希为准；HEAD 本身不包含后来修改。日志中的本地模型与凭据均为测试夹具，未调用云模型或验收 Electron。原生二进制和 SQLite 文件不入本目录。

复跑时先核对源码身份和依赖配置，不把本快照结果自动套用到未来源码。使用 Windows PowerShell 5.1，沿用证据 JSON 中已存在的 C 盘验证目录和预热 target；真实构建卷须通过 canonical 资源门禁。若换目录，应同步 target-dir，不可借更换工作目录指向另一个资源不足的构建卷。

以下复用 result.json 的原始参数。命令只展示复跑方法，不代表阅读文档时又运行了一次：

~~~powershell
$sdkRoot = 'T:\创新创业\OwO-master\agent-sdk'
$evidenceFile = Join-Path $sdkRoot 'docs\qa\evidence\2026-10-27-anthropic-stream\result.json'
$evidence = Get-Content -Raw -Encoding UTF8 -LiteralPath $evidenceFile | ConvertFrom-Json
Set-Location -LiteralPath $evidence.resource_policy.cwd
. (Join-Path $sdkRoot 'scripts\resolve-ort.ps1')
Resolve-OwoOrtEnv -Quiet | Out-Null
. (Join-Path $sdkRoot 'scripts\ci-shared.ps1')
foreach ($phase in @('check','native','unit','checkpoint','clippy')) {
    $cargoArgs = [string[]]$evidence.commands.PSObject.Properties[$phase].Value
    $logPath = Join-Path $evidence.resource_policy.cwd ('rerun-anthropic-stream-{0}.log' -f $phase)
    $ciExit = Invoke-CiCargo -Arguments $cargoArgs -Cwd (Get-Location).Path -PolicyMode strict -PassThru -HeartbeatSec 30 -LogFile $logPath -Label ('rerun-{0}' -f $phase)
    if ($ciExit -ne 0) { throw ('{0} exit={1}' -f $phase,$ciExit) }
}
~~~

受影响格式检查为独立 rustfmt --edition 2021 --check（适配器、子模块、测试），另有 git diff --check 和 python scripts/check-agent-boundary.py。没有用完整 workspace 测试替代定向验收，也没有停止用户服务或删除缓存。
