#requires -Version 5.1
<#
desktop-acceptance-common.ps1 — 桌面真机验收的共享原语（重构方案 §8.1/§8.2/§8.3）。

为什么存在：§8.2（冷启动请求预算）与 §8.3（故障矩阵）都要做同一批底层动作
——私有环境隔离地拉起壳、等 core_ready、抓窗口、按分辨率截图、带 token 读
服务端 ledger、经 DevTools 读 WebView 实际渲染的文本。此前这些能力只长在
verify-desktop-cold-boot.ps1 里；§8.3 若再抄一份就是审计反复点名的
「第二份实现」（方案 §7.2 同类问题）。本文件只放**无场景判断**的原语，
场景与断言留在调用方。

约定：
  * 只 dot-source 使用（` . scripts\desktop-acceptance-common.ps1`）；
    不得设置调用方的 $ErrorActionPreference（历史教训：Stop 泄漏进
    ci-gate 会话把 cargo/npm 的正常 stderr 判成终止错误）。
  * 函数一律 `Owo` 前缀；本文件顶层无副作用（不建目录、不联网、不弹窗）。
  * 取证产物 JSON 一律无 BOM 写出（PowerShell 5.1 的 `-Encoding UTF8` 带
    BOM，会让 node/JSON.parse 读证据文件时炸「Unexpected token ﻿」）。
#>

$script:OwoAcceptanceProbeHeader = 'probe'

function Add-OwoDesktopWin32 {
    <# 幂等加载截图/窗口几何所需程序集与 OwoWin32（可重复调用）。 #>
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName System.Windows.Forms
    if (-not ('OwoWin32' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class OwoWin32 {
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int W, int H, bool repaint);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out OwoRect rect);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [StructLayout(LayoutKind.Sequential)] public struct OwoRect { public int Left; public int Top; public int Right; public int Bottom; }

    // ---- 窗口枚举（R3-B 取证稳定性） -------------------------------------------
    // 为什么不能只信 Process.MainWindowHandle：.NET 的 MainWindowHandle 不保证是
    // "业务主窗口"——它按启发式挑选，一个同进程 15x15 的辅助窗口（WebView2/Tauri/
    // 其他桌面组件都会建）就能把它整个换掉。实测故障矩阵里同一场景上一轮量到
    // 1295x837、下一轮量到 15x15，截图门于是变成抛硬币。
    //
    // 委托回调**必须写在 C# 内部**：早期版本在 PowerShell 侧构造 EnumWindows 回调，
    // PS 5.1 在 native 回调里抛 "Argument types do not match"（嵌套委托/结构体封送
    // 的已知坑），换不来结果只换来 flaky。这里返回按客户面积降序排列的句柄数组，
    // PowerShell 侧从大到小逐个复验，辅助窗口永远排不到前面。
    private delegate bool OwoEnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] private static extern bool EnumWindows(OwoEnumProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);   // GA_ROOT=2
    private const uint GA_ROOT = 2;

    /// 返回属于 pid 的**顶层可见候选**窗口句柄，按面积从大到小排序（含不可见顶层窗口，
    /// 由调用方决定可见性要求；最小化窗口按 GetWindowRect 的真实伪矩形面积参与排序）。
    public static IntPtr[] TopLevelWindowHandlesByArea(uint processId) {
        var pairs = new List<KeyValuePair<long, IntPtr>>();
        EnumWindows((h, l) => {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (pid != processId) return true;
            if (GetAncestor(h, GA_ROOT) != h) return true;            // 只要根窗口：过滤子窗口/嵌入控件
            OwoRect r;
            if (!GetWindowRect(h, out r)) return true;
            long w = r.Right - r.Left, hh = r.Bottom - r.Top;
            if (w <= 0 || hh <= 0) return true;                        // 零尺寸/未实例化的残窗口
            pairs.Add(new KeyValuePair<long, IntPtr>(w * hh, h));
            return true;
        }, IntPtr.Zero);
        pairs.Sort((a, b) => b.Key.CompareTo(a.Key));                  // 面积降序：主窗口必然排第一
        var result = new IntPtr[pairs.Count];
        for (int i = 0; i < pairs.Count; i++) result[i] = pairs[i].Value;
        return result;
    }
}
'@
    }
    # 加载失败必须致命：静默降级会把"窗口身份检查"退化成猜——本文件存在的理由就是消除这类假通过。
    if (-not ('OwoWin32' -as [type])) {
        throw "OwoWin32 类型未能加载（Add-Type 静默失败）——窗口身份与截图验真不可用，验收必须停止"
    }
}

# dot-source 即加载：类型可用性与"哪个消费方记得调用"解耦（幂等，可重复 dot-source）。
Add-OwoDesktopWin32

function Get-OwoShellWindow {
    <# 等主窗口句柄出现；超时返回 IntPtr.Zero（不得把"没窗口"当成功）。
       与 Get-OwoValidatedWindow 同一套候选解析——本文件曾经并存两份窗口解析，
       分叉直接造成过一次假绿（见 verify-desktop-cold-boot.ps1 顶部注记）。 #>
    param([int]$ProcessId, [int]$TimeoutSec = 20)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $firstAlive = [IntPtr]::Zero
    while ((Get-Date) -lt $deadline) {
        $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if (-not $proc) { return [IntPtr]::Zero }
        foreach ($cand in @(Resolve-OwoShellWindowCandidate -ProcessId $ProcessId -Process $proc)) {
            if ($cand -eq [IntPtr]::Zero -or -not [OwoWin32]::IsWindow($cand)) { continue }
            $owner = [uint32]0
            $null = [OwoWin32]::GetWindowThreadProcessId($cand, [ref]$owner)
            if ([int]$owner -ne $ProcessId) { continue }
            # 优先返回"真的可见"的那个：等窗口出现时，面积最大的候选可能是
            # 还没显示的宿主窗口（返回它就等于让调用方拿一个黑窗口去截图）。
            if ([OwoWin32]::IsWindowVisible($cand) -and -not [OwoWin32]::IsIconic($cand)) { return $cand }
            if ($firstAlive -eq [IntPtr]::Zero) { $firstAlive = $cand }
        }
        Start-Sleep -Milliseconds 300
    }
    return $firstAlive
}

function Set-OwoWindowGeometry {
    <# 居中缩放到目标尺寸并返回实际尺寸（受虚拟屏限制，可能被夹小）。 #>
    param([IntPtr]$Hwnd, [int]$Width, [int]$Height)
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $targetW = $Width
    $targetH = $Height
    if ($targetW -gt $bounds.Width) { $targetW = $bounds.Width }
    if ($targetH -gt $bounds.Height) { $targetH = $bounds.Height }
    $x = [Math]::Max(0, [int](($bounds.Width - $targetW) / 2))
    $y = [Math]::Max(0, [int](($bounds.Height - $targetH) / 2))
    $null = [OwoWin32]::MoveWindow($Hwnd, $x, $y, $targetW, $targetH, $true)
    return "$targetW x $targetH"
}

function Set-OwoWindowShown {
    param([IntPtr]$Hwnd, [int]$Cmd)   # 0=SW_HIDE 5=SW_SHOW
    return [OwoWin32]::ShowWindow($Hwnd, $Cmd)
}

function Save-OwoWindowShot {
    <# 按窗口矩形截屏到 $Path（PNG），返回 "宽 x 高"。 #>
    param([IntPtr]$Hwnd, [string]$Path)
    $rect = New-Object OwoWin32+OwoRect
    if (-not [OwoWin32]::GetWindowRect($Hwnd, [ref]$rect)) { throw 'GetWindowRect 失败' }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) { throw "窗口尺寸无效：${width}x${height}" }
    $bitmap = New-Object System.Drawing.Bitmap($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($width, $height)))
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
    return "$width x $height"
}

function Resolve-OwoShellWindowCandidate {
    <#
      在一个进程的多个顶层窗口里挑出"业务主窗口"候选，按优先级返回句柄列表。

      顺序：调用方钉住的句柄 → 面积降序的顶层窗口 → Process.MainWindowHandle 兜底。
      MainWindowHandle 排最后而不是排第一：它是 .NET 的启发式结果，实测会被同进程
      15x15 辅助窗口劫持（故障矩阵同一场景两轮量出 1295x837 / 15x15 即为此）。
      这里只负责"排候选"，是否可用仍由 Get-OwoValidatedWindow 逐个复验判定，
      因此不违反 §3.3.1「绝不复用缓存 HWND」——每个候选都要重新验身份。
    #>
    param([int]$ProcessId, [System.Diagnostics.Process]$Process, [IntPtr]$PreferredHwnd = [IntPtr]::Zero)
    $candidates = New-Object System.Collections.Generic.List[IntPtr]
    if ($PreferredHwnd -ne [IntPtr]::Zero) { $candidates.Add($PreferredHwnd) }
    try {
        foreach ($h in @([OwoWin32]::TopLevelWindowHandlesByArea([uint32]$ProcessId))) {
            if ($h -ne [IntPtr]::Zero -and -not $candidates.Contains($h)) { $candidates.Add($h) }
        }
    } catch {
        # 枚举不可用（极端权限/会话隔离）时静默降级到 MainWindowHandle，但必须留痕，
        # 否则"枚举修好了"这件事以后没人知道是真是假。
        $global:OwoWindowEnumError = $_.Exception.Message
    }
    if ($Process -and $Process.MainWindowHandle -ne [IntPtr]::Zero -and -not $candidates.Contains($Process.MainWindowHandle)) {
        $candidates.Add($Process.MainWindowHandle)
    }
    return @($candidates)
}

function Get-OwoValidatedWindow {
    <#
      R3-A1（指南 §3.3.1）：窗口身份检查——每次截图/隐藏/恢复/core 重启后**重新获取**
      窗口，绝不复用缓存 HWND（窗口被隐藏/重载/重建后旧句柄会指向失效区域，
      实测截出过 158×26 的黑条还被判通过）。

      通过条件（§3.3.1 逐条）：HWND 非零且 IsWindow；PID 属于本轮 shell；
      RequireVisible 时另需 IsWindowVisible、未最小化、宽 ≥800、高 ≥520。
      返回带 ok/fail_reasons 的描述符（不 throw：调用方要把失败写进断言报告）。
    #>
    param(
        [int]$ProcessId,
        # 已在前一阶段验证过的句柄（如 observe 阶段拿到的主窗口）。给了就先试它，
        # 但仍要复验身份；失效则自动落到"按面积排好的顶层窗口候选"重新解析。
        [IntPtr]$PreferredHwnd = [IntPtr]::Zero,
        [int]$TimeoutSec = 20,
        [int]$MinWidth = 800,
        [int]$MinHeight = 520,
        [switch]$RequireVisible
    )
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $info = $null
    while ($true) {
        $info = [ordered]@{
            hwnd           = [IntPtr]::Zero
            process_id     = 0
            visible        = $false
            minimized      = $false
            left = 0; top = 0; right = 0; bottom = 0
            width          = 0
            height         = 0
            discovered_at  = (Get-Date).ToUniversalTime().ToString('o')
            ok             = $false
            fail_reasons   = @()
            restored_before_measure = $false
            candidate_count = 0
        }
        $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if (-not $proc) {
            $info.fail_reasons = @('process_exited')
            return [pscustomobject]$info
        }
        # 每次重新取句柄（绝不复用缓存 HWND，§3.3.1）；"最小化伪矩形"由下面的
        # restored_before_measure 分支处理：先 SW_RESTORE 再量，避免把"只是最小化了"
        # 误报成"界面没恢复"，也避免拿 158x26 的伪矩形去判截图有效。
        $candidates = @(Resolve-OwoShellWindowCandidate -ProcessId $ProcessId -Process $proc -PreferredHwnd $PreferredHwnd)
        $info.candidate_count = $candidates.Count
        # 三级择优（R3.6 实测教训：只按面积挑"属于本 PID 的最大窗口"会挑中不可见的
        # 顶层辅助窗口，面积还可能比主窗口大，于是冷启动 3 条截图断言整片报
        # window_not_visible——不是界面没了，是脚本盯错了窗口）：
        #   ① 可见 + 未最小化 + 尺寸达标 → 直接认定主窗口；
        #   ② 可见或处于最小态（下面 SW_RESTORE 分支要救的就是它）→ 次选；
        #   ③ 仅"归属本 PID 的活窗口" → 兜底，只为把失败原因说清楚。
        $hwnd = [IntPtr]::Zero
        $softFallback = [IntPtr]::Zero
        $anyFallback = [IntPtr]::Zero
        foreach ($cand in $candidates) {
            if ($cand -eq [IntPtr]::Zero -or -not [OwoWin32]::IsWindow($cand)) { continue }
            $ownerCheck = [uint32]0
            $null = [OwoWin32]::GetWindowThreadProcessId($cand, [ref]$ownerCheck)
            # 只接受"确实属于本进程"的候选：MainWindowHandle 在句柄失效时会返回别的
            # 进程的窗口，放过来就是把别人的界面当我们的截图（R3-BUG-02 同一类）。
            if ([int]$ownerCheck -ne $ProcessId) { continue }
            if ($anyFallback -eq [IntPtr]::Zero) { $anyFallback = $cand }
            $candVisible = [OwoWin32]::IsWindowVisible($cand)
            $candIconic = [OwoWin32]::IsIconic($cand)
            if (($candVisible -or $candIconic) -and $softFallback -eq [IntPtr]::Zero) { $softFallback = $cand }
            if (-not $RequireVisible) { $hwnd = $cand; break }   # 软探测：不判可见性，首个归属即可
            if (-not $candVisible -or $candIconic) { continue }
            $cr = New-Object 'OwoWin32+OwoRect'
            if (-not [OwoWin32]::GetWindowRect($cand, [ref]$cr)) { continue }
            if (($cr.Right - $cr.Left) -lt $MinWidth -or ($cr.Bottom - $cr.Top) -lt $MinHeight) { continue }
            $hwnd = $cand
            break
        }
        if ($hwnd -eq [IntPtr]::Zero) {
            if ($softFallback -ne [IntPtr]::Zero) { $hwnd = $softFallback }
            elseif ($anyFallback -ne [IntPtr]::Zero) { $hwnd = $anyFallback }
        }
        $info.hwnd = $hwnd
        if ($hwnd -eq [IntPtr]::Zero) {
            $info.fail_reasons = @("no_candidate_window(count=$($candidates.Count))")
        } else {
            $ownerPid = [uint32]0
            $null = [OwoWin32]::GetWindowThreadProcessId($hwnd, [ref]$ownerPid)
            $info.process_id = [int]$ownerPid
            if ([int]$ownerPid -ne $ProcessId) {
                $info.fail_reasons = @("hwnd_pid_mismatch(owner=$ownerPid expected=$ProcessId)")
            } else {
                $rect = New-Object OwoWin32+OwoRect
                if (-not [OwoWin32]::GetWindowRect($hwnd, [ref]$rect)) {
                    $info.fail_reasons = @('get_window_rect_failed')
                } else {
                    $info.visible = [OwoWin32]::IsWindowVisible($hwnd)
                    $info.minimized = [OwoWin32]::IsIconic($hwnd)
                    $info.left = $rect.Left; $info.top = $rect.Top
                    $info.right = $rect.Right; $info.bottom = $rect.Bottom
                    $info.width = $rect.Right - $rect.Left
                    $info.height = $rect.Bottom - $rect.Top
                    # 最小化窗口的 GetWindowRect 返回的是"最小化伪矩形"（实测 158×26
                    # 且 IsIconic=true）——直接判失败会把"窗口其实只是被最小化"误报成
                    # "界面没恢复"。这里显式还原一次再量，并把动作留在结果里（证据必须
                    # 说明这张截图是在还原后拍的，而不是凭空变大）。
                    if ($info.minimized) {
                        $null = [OwoWin32]::ShowWindow($hwnd, 9)   # SW_RESTORE
                        $info.restored_before_measure = $true
                        Start-Sleep -Milliseconds 450
                        $rect2 = New-Object 'OwoWin32+OwoRect'
                        if ([OwoWin32]::GetWindowRect($hwnd, [ref]$rect2)) {
                            $rect = $rect2
                            $info.left = $rect2.Left; $info.top = $rect2.Top
                            $info.right = $rect2.Right; $info.bottom = $rect2.Bottom
                            $info.width = $rect2.Right - $rect2.Left
                            $info.height = $rect2.Bottom - $rect2.Top
                        }
                        $info.visible = [OwoWin32]::IsWindowVisible($hwnd)
                        $info.minimized = [OwoWin32]::IsIconic($hwnd)
                    }
                    $fails = @()
                    if (-not $info.visible) { $fails += 'window_not_visible' }
                    if ($info.minimized) { $fails += 'window_minimized' }
                    if ($info.width -lt $MinWidth) { $fails += "width_lt_${MinWidth}($($info.width))" }
                    if ($info.height -lt $MinHeight) { $fails += "height_lt_${MinHeight}($($info.height))" }
                    if ($info.width -le 0 -or $info.height -le 0) { $fails += 'non_positive_rect' }
                    if ($RequireVisible) {
                        $info.fail_reasons = @($fails)
                        $info.ok = ($fails.Count -eq 0)
                    } else {
                        $info.ok = $true   # 软探测：只保证"句柄属于本 PID 且几何可读"
                    }
                }
            }
        }
        if ($info.ok) { return [pscustomobject]$info }
        if ((Get-Date) -ge $deadline) { return [pscustomobject]$info }
        Start-Sleep -Milliseconds 300
    }
}

function Test-OwoScreenshot {
    <#
      R3-A1（指南 §3.3.2）：截图真实性检查。截屏只证明"文件存在"毫无意义——
      必须可解码、尺寸与窗口一致、体积达标、非黑非透明像素达标、方差非零
      （防纯色图假通过）。所有指标进报告；ok=false 时 fail_reasons 给出全部原因。

      截图只能证明"有画面"；可操作性由调用方的 CDP 断言继续负责。
    #>
    param(
        [string]$Path,
        [int]$ExpectedWidth = 0,
        [int]$ExpectedHeight = 0,
        [int]$MinWidth = 800,
        [int]$MinHeight = 520,
        [long]$MinBytes = 8192,
        [double]$MinNonBlackRatio = 0.01,
        [double]$MinVariance = 100.0
    )
    $result = [ordered]@{
        path            = $Path
        bytes           = 0
        width           = 0
        height          = 0
        non_black_ratio = 0.0
        variance        = 0.0
        samples         = 0
        ok              = $false
        fail_reasons    = @()
    }
    if (-not (Test-Path -LiteralPath $Path)) {
        $result.fail_reasons = @('missing_file')
        return [pscustomobject]$result
    }
    $result.bytes = (Get-Item -LiteralPath $Path).Length
    if ($result.bytes -lt $MinBytes) { $result.fail_reasons += "bytes_lt_${MinBytes}($($result.bytes))" }
    $bitmap = $null
    $fs = $null
    try {
        # 用 FileStream 打开再解码：FromFile 会锁文件句柄，取证目录随后还要被清理；
        # Bitmap 在其生命周期内持续引用该流，因此流必须比 bitmap 晚释放（外层 finally）。
        $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
        $bitmap = New-Object System.Drawing.Bitmap($fs)
        $result.width = $bitmap.Width
        $result.height = $bitmap.Height
    } catch {
        $result.fail_reasons += "undecodable($($_.Exception.Message))"
        if ($bitmap) { try { $bitmap.Dispose() } catch { } }
        if ($fs) { try { $fs.Dispose() } catch { } }
        $result.fail_reasons = @($result.fail_reasons)
        return [pscustomobject]$result
    }
    try {
        if ($result.width -lt $MinWidth -or $result.height -lt $MinHeight) {
            $result.fail_reasons += "undersized($($result.width)x$($result.height) < ${MinWidth}x${MinHeight})"
        }
        if ($ExpectedWidth -gt 0 -and $ExpectedHeight -gt 0) {
            $wDiff = [Math]::Abs($result.width - $ExpectedWidth) / $ExpectedWidth
            $hDiff = [Math]::Abs($result.height - $ExpectedHeight) / $ExpectedHeight
            if ($wDiff -gt 0.10 -or $hDiff -gt 0.10) {
                $result.fail_reasons += "size_mismatch_vs_window(${ExpectedWidth}x${ExpectedHeight} actual=$($result.width)x$($result.height))"
            }
        }
        # 像素采样（最多 ~30k 点）：非黑非透明比例 + 亮度方差。
        $rect = New-Object System.Drawing.Rectangle(0, 0, $result.width, $result.height)
        $locked = $bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $bufBytes = [Math]::Abs($locked.Stride) * $result.height
        $buf = New-Object 'byte[]' $bufBytes
        [System.Runtime.InteropServices.Marshal]::Copy($locked.Scan0, $buf, 0, $bufBytes)
        $bitmap.UnlockBits($locked)
        $strideAbs = [Math]::Abs($locked.Stride)
        $step = [Math]::Max(1, [int][Math]::Floor([Math]::Sqrt(($result.width * $result.height) / 30000.0)))
        $lumas = New-Object System.Collections.Generic.List[double]
        $nonBlack = 0
        for ($y = 0; $y -lt $result.height; $y += $step) {
            $rowBase = $y * $strideAbs
            for ($x = 0; $x -lt $result.width; $x += $step) {
                $i = $rowBase + $x * 4
                $b = $buf[$i]; $g = $buf[$i + 1]; $r = $buf[$i + 2]; $a = $buf[$i + 3]
                $luma = 0.299 * $r + 0.587 * $g + 0.114 * $b
                $null = $lumas.Add($luma)
                if ($a -ge 128 -and [Math]::Max([Math]::Max($r, $g), $b) -gt 24) { $nonBlack += 1 }
            }
        }
        $count = $lumas.Count
        $result.samples = $count
        if ($count -eq 0) {
            $result.fail_reasons += 'no_samples'
        } else {
            $result.non_black_ratio = [Math]::Round($nonBlack / $count, 6)
            $mean = 0.0
            foreach ($v in $lumas) { $mean += $v }
            $mean = $mean / $count
            $var = 0.0
            foreach ($v in $lumas) { $d = $v - $mean; $var += $d * $d }
            $result.variance = [Math]::Round($var / $count, 4)
            if ($result.non_black_ratio -lt $MinNonBlackRatio) {
                $result.fail_reasons += "mostly_black_or_transparent(ratio=$($result.non_black_ratio) < $MinNonBlackRatio)"
            }
            if ($result.variance -lt $MinVariance) {
                $result.fail_reasons += "flat_content(variance=$($result.variance) < $MinVariance)"
            }
        }
    } finally {
        $bitmap.Dispose()
        if ($fs) { try { $fs.Dispose() } catch { } }
    }
    $result.fail_reasons = @($result.fail_reasons)
    $result.ok = ($result.fail_reasons.Count -eq 0)
    return [pscustomobject]$result
}


function Get-OwoScreenshotMetricLine {
    <# 断言 detail 用的单行指标摘要（宽/高/字节/非黑比例/方差/失败原因）。 #>
    param($Metric)
    $reasons = if (@($Metric.fail_reasons).Count -gt 0) { " fail=$(($Metric.fail_reasons) -join ';')" } else { '' }
    return "size=$($Metric.width)x$($Metric.height) bytes=$($Metric.bytes) non_black_ratio=$($Metric.non_black_ratio) variance=$($Metric.variance) samples=$($Metric.samples)$reasons"
}

function Save-OwoEvidenceJson {
    <# 无 BOM 写证据 JSON（禁止改用 Set-Content -Encoding UTF8）。 #>
    param([string]$Dir, [string]$Name, $InputObject, [int]$Depth = 8)
    if (-not (Test-Path -LiteralPath $Dir)) { New-Item -ItemType Directory -Force -Path $Dir | Out-Null }
    $path = Join-Path $Dir $Name
    $text = ($InputObject | ConvertTo-Json -Depth $Depth)
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
    return $path
}

function Invoke-OwoCoreApi {
    <# 取证用直连核心：带 x-owo-client: probe 标签（服务端 ledger 会据此把探测流量与真实首屏流量分桶）。 #>
    param(
        [string]$Base,
        [string]$Path,
        [string]$Token,
        [int]$TimeoutSec = 8,
        [string]$Method = 'GET',
        $Body = $null
    )
    $headers = @{ 'x-owo-client' = $script:OwoAcceptanceProbeHeader }
    if ($Token) { $headers['Authorization'] = "Bearer $Token" }
    $params = @{ Uri = ($Base + $Path); Headers = $headers; TimeoutSec = $TimeoutSec; Method = $Method }
    if ($null -ne $Body) {
        $params['ContentType'] = 'application/json'
        $params['Body'] = ($Body | ConvertTo-Json -Depth 6 -Compress)
    }
    return Invoke-RestMethod @params
}

function Get-OwoCount {
    <#
      安全计数：PowerShell 的函数返回会**拆掉单元素数组**（`@(1)` → `1`），
      而 PS 5.1 下标量 PSCustomObject 的 `.Count` 是 $null——于是 `≤5` 这类
      断言会拿着 null 空转成"通过"（实测踩过：重启后只有 1 条业务请求时
      web_business=null）。计数一律走这里。
    #>
    param($Items)
    return @(($Items | Where-Object { $null -ne $_ })).Count
}

function Get-OwoLedgerFacts {
    <#
      读 /diagnostics/requests 并按来源分桶。
      口径：业务 = 非 health / 非 auth/token / 非事件流；web 与 shell 分列，
      避免壳自己的配对引导把 WebView 首屏预算掩盖掉（反之亦然）。
      探测流量（source=probe）整体排除，不进任何断言。
    #>
    param([string]$Base, [string]$Token, [string]$Label)
    $report = Invoke-OwoCoreApi -Base $Base -Path '/diagnostics/requests?limit=512' -Token $Token
    $records = @($report.records)
    function BucketOf([string]$source) { @($records | Where-Object { $_.source -eq $source }) }
    function BusinessOf($set) {
        @($set | Where-Object {
                $_.route_template -ne '/health' -and
                $_.route_template -ne '/auth/token' -and
                $_.route_template -ne '/events/stream'
            })
    }
    # 调用点再包一层 @()：抵消函数返回时的单元素拆包（见 Get-OwoCount 注释）。
    $web = @(BucketOf 'web')
    $shell = @(BucketOf 'shell')
    $other = @(BucketOf 'other')
    $probe = @(BucketOf 'probe')
    $webBusiness = @(BusinessOf $web)
    $webRoutes = @($webBusiness | ForEach-Object { $_.method + ' ' + $_.route_template })
    $webHealthCount = Get-OwoCount @($web | Where-Object { $_.route_template -eq '/health' })
    $webAuthCount = Get-OwoCount @($web | Where-Object { $_.route_template -eq '/auth/token' })
    $webEventsCount = Get-OwoCount @($web | Where-Object { $_.route_template -eq '/events/stream' })
    $webBusinessCount = Get-OwoCount $webBusiness
    $facts = [ordered]@{
        label               = $Label
        captured_at         = (Get-Date).ToUniversalTime().ToString('o')
        server_total        = $report.total
        returned            = $report.returned
        web_total           = Get-OwoCount $web
        web_health          = $webHealthCount
        web_auth            = $webAuthCount
        web_events          = $webEventsCount
        web_business        = $webBusinessCount
        web_business_routes = $webRoutes
        # 同一路由在同一窗口被重复请求即为风暴信号（>1 次需在记录中解释）。
        web_route_repeats   = @(($webRoutes | Group-Object | Where-Object { $_.Count -gt 1 } |
                ForEach-Object { "$($_.Name) x$($_.Count)" }))
        shell_total         = Get-OwoCount $shell
        shell_auth          = Get-OwoCount @($shell | Where-Object { $_.route_template -eq '/auth/token' })
        shell_health        = Get-OwoCount @($shell | Where-Object { $_.route_template -eq '/health' })
        other_total         = Get-OwoCount $other
        probe_total         = Get-OwoCount $probe
        sources             = @($records | ForEach-Object { $_.source } | Sort-Object -Unique)
        records             = @($records | Where-Object { $_.source -ne $script:OwoAcceptanceProbeHeader })
        # 分桶自洽性：web 四桶之和必须等于 web_total。任何 null/漏桶都会立刻在此
        # 暴露（历史上正是"计数被拆成 null"导致 ≤5 断言空转通过）。
        web_buckets_sum     = $webHealthCount + $webAuthCount + $webEventsCount + $webBusinessCount
        web_buckets_ok      = (($webHealthCount + $webAuthCount + $webEventsCount + $webBusinessCount) -eq (Get-OwoCount $web))
    }
    return [pscustomobject]$facts
}

function Invoke-OwoShellIpc {
    <#
      经 CDP 调壳的 IPC 命令（`__TAURI_INTERNALS__.invoke`）并取回结果。
      CDP 的 Runtime.evaluate 不接 awaitPromise，因此把 Promise 结果落到一个
      一次性全局上再轮询读——比"猜它跑完了"可靠。
      返回 $null 即通道不可用：调用方必须判失败，不得当成成功继续。
    #>
    param([int]$Port, [string]$Command, [hashtable]$Arguments = @{}, [int]$TimeoutSec = 10)
    $key = '__owoIpc'
    $payload = ($Arguments | ConvertTo-Json -Compress -Depth 4)
    if (-not $payload) { $payload = '{}' }
    $js = "window.$key='pending';window.__TAURI_INTERNALS__.invoke('$Command',$payload)" +
          ".then(function(r){window.$key=JSON.stringify(r);})" +
          ".catch(function(e){window.$key='ERR:'+e;});'fired'"
    $null = Invoke-OwoCdpEval -Port $Port -Expression $js -TimeoutSec $TimeoutSec
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $raw = Invoke-OwoCdpEval -Port $Port -Expression "String(window.$key)" -TimeoutSec 5
        if ($raw -and $raw -ne 'pending') {
            if ($raw -like 'ERR:*') { $global:OwoCdpLastError = $raw; return $null }
            try { return ($raw | ConvertFrom-Json) } catch { return $raw }
        }
        Start-Sleep -Milliseconds 300
    }
    $global:OwoCdpLastError = "ipc_timeout:$Command"
    return $null
}

function Wait-OwoCoreReady {
    <# 从壳捕获的核心日志里读本次代际的 core_ready 行（ExcludePid 排除上一代）。 #>
    param([string]$LogDir, [int]$TimeoutSec, [int]$ExcludePid = 0)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $logs = @(Get-ChildItem $LogDir -Filter 'desktop-core-*.log' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending)
        foreach ($log in $logs) {
            $lines = @(Get-Content -LiteralPath $log.FullName -ErrorAction SilentlyContinue)
            for ($i = $lines.Count - 1; $i -ge 0; $i--) {
                # core_ready 的键按 serde_json 字典序输出，故只按 "event":"core_ready" 子句判定。
                if ($lines[$i] -notmatch '"event"\s*:\s*"core_ready"') { continue }
                $start = $lines[$i].IndexOf('{')
                $end = $lines[$i].LastIndexOf('}')
                if ($start -lt 0 -or $end -le $start) { continue }
                $ready = $null
                try {
                    $ready = $lines[$i].Substring($start, $end - $start + 1) | ConvertFrom-Json
                } catch { continue }
                if ($ready.event -eq 'core_ready' -and [int]$ready.port -gt 0 -and [int]$ready.pid -ne $ExcludePid) {
                    return $ready
                }
            }
        }
        Start-Sleep -Milliseconds 400
    }
    return $null
}

function Get-OwoCoreLogTail {
    <# 取最近的壳/核心日志尾部文本（错误页断言与失败排障都要用）。 #>
    param([string]$Dir, [string]$Filter = '*.log', [int]$Lines = 60)
    $logs = @(Get-ChildItem -LiteralPath $Dir -Filter $Filter -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 2)
    $text = New-Object System.Collections.Generic.List[string]
    foreach ($log in $logs) {
        foreach ($line in @(Get-Content -LiteralPath $log.FullName -Tail $Lines -ErrorAction SilentlyContinue)) {
            $text.Add($line)
        }
    }
    return ($text -join "`n")
}

function New-OwoShellStartInfo {
    <#
      构造"完全私有环境"的壳启动参数：LOCALAPPDATA/APPDATA/TEMP 全部重定向到
      本次运行目录（壳的数据根、工作区指针、日志都从 LOCALAPPDATA 派生，因此
      这一组变量足以把真实用户目录隔离在外）。
      -CdpPort 非 0 时开启 WebView2 远程调试端口，供 DOM 断言使用。
    #>
    param(
        [string]$ShellExe,
        [string]$LocalAppData,
        [string]$AppData,
        [string]$TempDir,
        [string]$ApiKey,
        [int]$CdpPort = 0,
        [string]$WebViewUserDataDir = '',
        # R3-A3（指南 §3.3.3）：非空即进入壳的验收模式（仅 debug 壳生效）：
        # sidecar 候选只来自该根目录，禁止回退仓库/PATH/历史安装目录。
        [string]$AcceptanceSidecarRoot = '',
        [switch]$NoApiKey
    )
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $ShellExe
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = Split-Path -Parent $ShellExe
    $psi.EnvironmentVariables['LOCALAPPDATA'] = $LocalAppData
    $psi.EnvironmentVariables['APPDATA'] = $AppData
    $psi.EnvironmentVariables['TEMP'] = $TempDir
    $psi.EnvironmentVariables['TMP'] = $TempDir
    if ($AcceptanceSidecarRoot) {
        $psi.EnvironmentVariables['OWO_DESKTOP_ACCEPTANCE'] = '1'
        $psi.EnvironmentVariables['OWO_SIDECAR_ROOT'] = $AcceptanceSidecarRoot
    }
    if ($NoApiKey) {
        $psi.EnvironmentVariables.Remove('OPENAI_API_KEY')
    } elseif ($ApiKey) {
        $psi.EnvironmentVariables['OPENAI_API_KEY'] = $ApiKey
    }
    # 取证需要直连核心读 ledger：不得走只影响 token 引导路径的开发开关。
    $psi.EnvironmentVariables.Remove('OWO_DESKTOP_DEV_AUTH')
    if ($CdpPort -gt 0) {
        $psi.EnvironmentVariables['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = "--remote-debugging-port=$CdpPort"
    }
    if ($WebViewUserDataDir) {
        # WebView2 的用户数据目录由 known-folder API 派生（LOCALAPPDATA 环境变量重定向
        # 管不到它），因此验收必须显式换目录：否则与**已安装版本**共用浏览器进程组，
        # 表现为 DevTools 端口/页面目标不可控（实测踩过）。
        $psi.EnvironmentVariables['WEBVIEW2_USER_DATA_FOLDER'] = $WebViewUserDataDir
    }
    return $psi
}

function Get-OwoWebviewEtbDir {
    <# WebView2 实际写入的目录 = 给定 UDF 下的 EBWebView（实测）。 #>
    param([string]$UserDataDir)
    return (Join-Path $UserDataDir 'EBWebView')
}

# ---------------------------------------------------------------------------
# 验收运行骨架（§3.3.3 私有环境 + R3-A3 验收模式 + §7 内嵌前端新鲜度门）
#
# 为什么收在这里：冷启动（verify-desktop-cold-boot）与故障矩阵
# （verify-desktop-failure-matrix）各自抄了一份"私有目录 + 随包 core + 私有 bin +
# CDP 端口 + UDF"的开头，R4/R5 再加一个脚本就是第三、第四份——这类复制以前直接
# 造成过两份 ledger 实现分叉导致的假绿。新脚本一律走本函数；两份旧脚本的开头
# 待 R6 去重时一并迁移（见执行记录未完成项）。
# ---------------------------------------------------------------------------

function New-OwoAcceptanceRun {
    <#
      组装一次隔离验收运行的全部路径与二进制（**不启动进程**）。
      返回 hashtable：run_root/bin_dir/shell_exe/sidecar_exe/local_app_data/app_data/
      temp_dir/project/agent_state_dir/data_root/log_dir/token_file/
      webview_udf/webview_data_dir/cdp_requested/staged/identity/stamp/scenario。
      -PrepareWorkspace：预置壳的"最近项目"指针（等价用户在引导页选过一次目录）。
    #>
    param(
        [Parameter(Mandatory = $true)][string]$SdkRoot,
        [Parameter(Mandatory = $true)][string]$Scenario,
        [string]$Stamp = '',
        [string]$ShellExe = '',
        [string]$SidecarExe = '',
        [ValidateSet('debug', 'release')][string]$Configuration = 'debug',
        [switch]$PrepareWorkspace,
        [switch]$SkipFreshnessGate
    )
    if (-not $Stamp) { $Stamp = Get-Date -Format 'yyyyMMdd-HHmmss' }
    if (-not $ShellExe) {
        # ADR-003：桌面壳已由 Tauri 收敛为 Electron。旧壳是"单个 exe + 同级 dll"，
        # Electron 是"目录形态"（app + node_modules 或 win-unpacked），本原语的
        # 复制/启动模型不再适用，因此不再给出已失效的默认路径——直接要求调用方
        # 显式指定，避免把迁移缺口伪装成"文件缺失"。
        throw "ADR-003：桌面壳已换 Electron，$($MyInvocation.MyCommand) 的旧默认路径 desktop\tauri\src-tauri\...\owo-agent-desktop.exe 已不存在。请显式传 -ShellExe（Electron 壳入口）并完成本脚本迁移。"
    }
    if (-not $SidecarExe) {
        $SidecarExe = Join-Path $SdkRoot "target\$Configuration\owo-agent.exe"
    }
    if (-not (Test-Path -LiteralPath $ShellExe)) { throw "缺少桌面壳可执行文件：$ShellExe" }
    if (-not (Test-Path -LiteralPath $SidecarExe)) {
        throw "缺少 sidecar 可执行文件：$SidecarExe（先 cargo build -p owo-agent-cli）"
    }
    # §7 构建自包含：壳内嵌前端资产，改了 web 不重建壳就是在测上一版界面。
    if (-not $SkipFreshnessGate) {
        $null = Assert-OwoShellEmbedsCurrentWeb -ShellExe $ShellExe -WebRoot (Join-Path $SdkRoot 'desktop\web')
    }

    $runRoot = Join-Path ([IO.Path]::GetTempPath()) "owo-desktop-acceptance\$Stamp\$Scenario"
    $localAppData = Join-Path $runRoot 'LocalAppData'
    $appData = Join-Path $runRoot 'RoamingAppData'
    $runTemp = Join-Path $runRoot 'Temp'
    $project = Join-Path $runRoot 'project'
    $webviewUdf = Join-Path $runRoot 'WebView2'
    foreach ($dir in @($localAppData, $appData, $runTemp, $project, $webviewUdf)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }
    $agentStateDir = Join-Path $localAppData 'OwO\Agent'
    New-Item -ItemType Directory -Force -Path $agentStateDir | Out-Null
    if ($PrepareWorkspace) {
        # §4.6：预置"最近项目"指针（数据目录仍是全新的）。
        Set-Content -Path (Join-Path $agentStateDir 'workspace.json') `
            -Value ('{"path":"' + ($project -replace '\\', '\\') + '"}') -Encoding ASCII -NoNewline
    }

    . (Join-Path $PSScriptRoot 'stage-desktop-sidecar.ps1')
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'   # 子进程正常 stderr 不得被当成终止错误
    try {
        $staged = Stage-OwoDesktopSidecar -Configuration $Configuration -Quiet
    } finally {
        $ErrorActionPreference = $prevEap
    }
    if (-not $staged -or -not $staged.source) { throw "随包 core 解析失败（stage-desktop-sidecar 无输出）" }
    # 同世代校验：随包 core 与 SDK 构建产物不同世代时，测的是历史二进制（§8.3 踩过）。
    $stagedCommit = ([regex]::Match([string]$staged.identity, 'commit=(\S+)')).Groups[1].Value
    $sdkLine = ((& $SidecarExe --version 2>&1 | Out-String) -split '\r?\n' | Where-Object { $_ -match 'commit=' } | Select-Object -First 1)
    $sdkCommit = ([regex]::Match([string]$sdkLine, 'commit=(\S+)')).Groups[1].Value
    if ($stagedCommit -and $sdkCommit -and ($stagedCommit -ne $sdkCommit)) {
        throw "随包 core 与 SDK 构建产物不同世代：staged=$stagedCommit sdk=$sdkCommit"
    }
    # R3-A3：本轮全部二进制进私有 bin；壳以验收模式启动（OWO_SIDECAR_ROOT=bin），
    # 仓库/同目录/历史安装产物一律不参与解析。
    $binDir = Join-Path $runRoot 'bin'
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    Copy-Item -LiteralPath $ShellExe -Destination (Join-Path $binDir 'owo-agent-desktop.exe') -Force
    $shellDir = Split-Path -Parent $ShellExe
    foreach ($dll in @(Get-ChildItem -LiteralPath $shellDir -Filter '*.dll' -ErrorAction SilentlyContinue)) {
        Copy-Item -LiteralPath $dll.FullName -Destination (Join-Path $binDir $dll.Name) -Force
    }
    Copy-Item -LiteralPath $staged.source -Destination (Join-Path $binDir 'owo-agent.exe') -Force
    $launchedCore = Join-Path $binDir 'owo-agent.exe'
    $launchedSha = (Get-FileHash -LiteralPath $launchedCore -Algorithm SHA256).Hash
    if ($launchedSha -ne $staged.sha256) {
        throw "私有 bin 中的 core 与随包产物哈希不一致：bin=$launchedSha staged=$($staged.sha256)"
    }

    $cdpRequested = Get-OwoFreeTcpPort
    $webviewDataDir = Get-OwoWebviewEtbDir -UserDataDir $webviewUdf
    $null = Clear-OwoCdpPortFile -DataDir $webviewDataDir
    return @{
        scenario          = $Scenario
        stamp             = $Stamp
        run_root          = $runRoot
        bin_dir           = $binDir
        shell_exe         = (Join-Path $binDir 'owo-agent-desktop.exe')
        repo_shell_exe    = $ShellExe
        sidecar_exe       = $SidecarExe
        launched_core     = $launchedCore
        launched_sha256   = $launchedSha
        staged            = $staged
        identity          = [string]$staged.identity
        local_app_data    = $localAppData
        app_data          = $appData
        temp_dir          = $runTemp
        project           = $project
        agent_state_dir   = $agentStateDir
        data_root         = (Join-Path $agentStateDir 'data')
        log_dir           = (Join-Path $agentStateDir 'logs')
        token_file        = (Join-Path $agentStateDir 'data\auth\token')
        webview_udf       = $webviewUdf
        webview_data_dir  = $webviewDataDir
        cdp_requested     = $cdpRequested
    }
}

function Start-OwoAcceptanceShell {
    <# 按 New-OwoAcceptanceRun 的结果启动壳（凭据只注入子进程，绝不回显）。 #>
    param(
        [Parameter(Mandatory = $true)]$Run,
        [int]$CdpPort = 0,
        [switch]$NoApiKey
    )
    $apiKey = ''
    if (-not $NoApiKey) {
        $apiKey = [Environment]::GetEnvironmentVariable('OPENAI_API_KEY', 'User')
        if (-not $apiKey) {
            throw '用户级环境变量 OPENAI_API_KEY 缺失：sidecar 无法启动（AGENTS.md 凭据红线：只注入，不回显）'
        }
    }
    $port = if ($CdpPort -gt 0) { $CdpPort } else { [int]$Run.cdp_requested }
    $psi = New-OwoShellStartInfo -ShellExe $Run.shell_exe -LocalAppData $Run.local_app_data `
        -AppData $Run.app_data -TempDir $Run.temp_dir -ApiKey $apiKey -CdpPort $port `
        -WebViewUserDataDir $Run.webview_udf -AcceptanceSidecarRoot $Run.bin_dir -NoApiKey:$NoApiKey
    return [System.Diagnostics.Process]::Start($psi)
}

function Assert-OwoShellEmbedsCurrentWeb {
    <#
      §7 构建自包含：桌面壳 exe **内嵌** desktop/web 前端资产（tauri-build 在构建期
      打包），web 侧任何改动都必须重建壳才会被本次验收观察到。这里把"证据与源码
      不同批次"变成开场即失败——而不是跑完 8 分钟后拿一份旧界面的数据当结论
      （实测踩过：服务端 SSE 语义已修，壳里仍是旧 events.js）。
    #>
    param([string]$ShellExe, [string]$WebRoot)
    if (-not (Test-Path -LiteralPath $ShellExe)) { throw "缺少桌面壳：$ShellExe（先在 src-tauri 执行 cargo build）" }
    if (-not (Test-Path -LiteralPath $WebRoot)) { throw "找不到前端资产目录：$WebRoot" }
    $built = (Get-Item -LiteralPath $ShellExe).LastWriteTime
    $stale = @(Get-ChildItem -LiteralPath $WebRoot -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
            ($_.Extension -in '.js', '.html', '.css') -and
            ($_.FullName -notmatch '\\tests\\') -and
            ($_.LastWriteTime -gt $built)
        } | Select-Object -First 5)
    if ($stale.Count -gt 0) {
        throw ("桌面壳早于前端资产，验收证据会与源码不同批次：{0}（壳构建于 {1}）—— 请先在 src-tauri 执行 cargo build" -f `
                (($stale | ForEach-Object { $_.Name }) -join ', '), $built.ToString('yyyy-MM-dd HH:mm:ss'))
    }
    return $built
}

function Get-OwoFreeTcpPort {
    <# 交给系统分配空闲端口（不猜端口号）。 #>
    $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

function Get-OwoWebviewDataDir {
    <#
      WebView2 用户数据目录。它由 Windows known-folder API 决定，**不受子进程
      LOCALAPPDATA 环境变量重定向影响**（实测落在真实 %LOCALAPPDATA%\com.owo.agent\
      EBWebView）：桌面验收能隔离核心数据根，却隔离不了 WebView2 的浏览器进程组，
      因此"上一轮的浏览器进程还活着"必须被显式处理。
    #>
    param([string]$Identifier = 'com.owo.agent')
    return (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) (Join-Path $Identifier 'EBWebView'))
}

function Clear-OwoCdpPortFile {
    <# 删掉 DevToolsActivePort，使"本轮是否真的起了调试端口"成为可判定事实。 #>
    param([string]$DataDir)
    $file = Join-Path $DataDir 'DevToolsActivePort'
    if (Test-Path -LiteralPath $file) {
        Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue
        return $true
    }
    return $false
}

function Test-OwoCdpAlive {
    param([int]$Port)
    if ($Port -le 0) { return $false }
    try {
        $null = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/version" -TimeoutSec 3
        return $true
    } catch { return $false }
}

function Resolve-OwoCdpPort {
    <#
      返回本轮**实际**可用的 DevTools 端口。
      指定端口被占用时 Chromium 会静默换口，所以真相来源是 UDF 下的
      DevToolsActivePort 第一行；申请端口只作回落候选。都探不通返回 0。
    #>
    param([string]$DataDir, [int]$RequestedPort, [int]$TimeoutSec = 20)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $file = Join-Path $DataDir 'DevToolsActivePort'
        if (Test-Path -LiteralPath $file) {
            $firstLine = ''
            try { $firstLine = (Get-Content -LiteralPath $file -TotalCount 1 | Select-Object -First 1) } catch { }
            $actual = 0
            if ([int]::TryParse(($firstLine -replace '\D', ''), [ref]$actual) -and $actual -gt 0) {
                if (Test-OwoCdpAlive -Port $actual) { return $actual }
            }
        }
        if (Test-OwoCdpAlive -Port $RequestedPort) { return $RequestedPort }
        Start-Sleep -Milliseconds 400
    }
    return 0
}

function Get-OwoWebviewDiagnostics {
    <# 取不到 DOM 事实时的现场取证：谁在用这个 UDF、申请端口有没有人监听。 #>
    param([int]$RequestedPort, [string]$DataDir)
    $procs = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -and $_.CommandLine -like "*$DataDir*" })
    $listening = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
        Where-Object { $_.LocalPort -eq $RequestedPort } |
        ForEach-Object { "$($_.LocalAddress):$($_.LocalPort)" })
    return [pscustomobject]@{
        requested_port      = $RequestedPort
        requested_listening = $listening
        webview_data_dir    = $DataDir
        port_file_present   = (Test-Path -LiteralPath (Join-Path $DataDir 'DevToolsActivePort'))
        webview_procs       = $procs.Count
        webview_cmdlines    = @($procs | ForEach-Object { $_.CommandLine.Substring(0, [Math]::Min(400, $_.CommandLine.Length)) })
    }
}

function Wait-OwoCdpReady {
    <# 等 DevTools HTTP 端点起来并返回 page 目标的 webSocketDebuggerUrl。 #>
    param([int]$Port, [int]$TimeoutSec = 15)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $pages = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 3
            $page = @($pages | Where-Object { $_.type -eq 'page' } | Select-Object -First 1)
            if ($page -and $page[0].webSocketDebuggerUrl) { return $page[0].webSocketDebuggerUrl }
        } catch { Start-Sleep -Milliseconds 300 }
    }
    return $null
}

function Invoke-OwoCdpEval {
    <#
      经 CDP Runtime.evaluate 读取 WebView **实际渲染**的结果（returnByValue）。
      这是 §8.3 唯一的"错误页是否真的可见"硬断言通道——截屏只能给人看，
      不能自动判定；而"HTTP 200 + 窗口存在"更证明不了视图内容。

      失败一律返回 $null 并把原因写进 $global:OwoCdpLastError / OwoCdpTrace：
      取不到 DOM 事实时，"为什么取不到"本身就是验收要留档的证据，
      不允许静默降级成"当作没渲染"。
    #>
    param([int]$Port, [string]$Expression, [int]$TimeoutSec = 10)
    $global:OwoCdpLastError = $null
    $trace = New-Object System.Collections.Generic.List[string]
    $wsUrl = Wait-OwoCdpReady -Port $Port -TimeoutSec $TimeoutSec
    $trace.Add("ws_url=$(if ($wsUrl) { $wsUrl } else { '<none>' })")
    if (-not $wsUrl) {
        $global:OwoCdpLastError = "no_page_target (port=$Port)"
        $global:OwoCdpTrace = @($trace)
        return $null
    }
    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $ct = [System.Threading.CancellationToken]::None
    try {
        $null = $ws.ConnectAsync([Uri]$wsUrl, $ct).GetAwaiter().GetResult()
        $trace.Add("connected state=$($ws.State)")
        $payload = (@{
                id     = 1
                method = 'Runtime.evaluate'
                params = @{ expression = $Expression; returnByValue = $true }
            } | ConvertTo-Json -Compress -Depth 6)
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
        $seg = New-Object 'System.ArraySegment[byte]' -ArgumentList @(, $bytes)
        # PS 5.1 不允许换行后接 `.Method()`，整条链必须同一行。
        $null = $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).GetAwaiter().GetResult()
        $trace.Add("sent bytes=$($bytes.Length)")
        $buffer = New-Object 'byte[]' 262144
        $recv = New-Object 'System.ArraySegment[byte]' -ArgumentList @(, $buffer)
        $decoder = [System.Text.Encoding]::UTF8
        $deadline = (Get-Date).AddSeconds($TimeoutSec)
        $frames = 0
        while ((Get-Date) -lt $deadline) {
            $task = $ws.ReceiveAsync($recv, $ct)
            if (-not $task.Wait(4000)) { $trace.Add('receive_timeout_4s'); $global:OwoCdpLastError = 'receive_timeout'; break }
            $result = $task.Result
            if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
                $trace.Add("peer_close state=$($ws.State)"); $global:OwoCdpLastError = 'peer_closed'; break
            }
            $chunk = $decoder.GetString($buffer, 0, $result.Count)
            if (-not $result.EndOfMessage) { $trace.Add('fragmented_message'); $global:OwoCdpLastError = 'fragmented'; break }
            $frames += 1
            $message = $null
            try { $message = $chunk | ConvertFrom-Json } catch { $trace.Add("bad_json len=$($chunk.Length)"); continue }
            if ($null -eq $message) { continue }
            if ($message.id -ne 1) { continue }   # 事件通知帧：继续等响应
            # CDP 成功响应**没有** error 字段，失败响应没有 result 字段——两者都必须
            # 按"属性是否存在"判定：直取不存在的属性在 Set-StrictMode -Version Latest
            # 的调用方里会抛 "property cannot be found"，把真实故障伪装成 CDP 异常。
            if ($message.PSObject.Properties['error']) {
                $err = $message.error
                $global:OwoCdpLastError = 'cdp_error {0}: {1}' -f `
                    $(if ($err.PSObject.Properties['code']) { $err.code } else { '?' }), `
                    $(if ($err.PSObject.Properties['message']) { $err.message } else { '<no message>' })
                $trace.Add($global:OwoCdpLastError)
                break
            }
            if (-not $message.PSObject.Properties['result']) {
                $global:OwoCdpLastError = 'cdp_response_without_result'
                $trace.Add($global:OwoCdpLastError)
                break
            }
            $trace.Add("frames=$frames ok")
            $global:OwoCdpTrace = @($trace)
            $res = $message.result
            if (-not $res.PSObject.Properties['result']) { return $null }   # 无 value 语义（如被 CDP 丢弃）
            $inner = $res.result
            if ($inner.PSObject.Properties['value']) { return $inner.value }
            if ($inner.PSObject.Properties['description']) { return $inner.description }
            return $null
        }
        if (-not $global:OwoCdpLastError) {
            $global:OwoCdpLastError = "no_response frames=$frames budget=${TimeoutSec}s"
        }
        $global:OwoCdpTrace = @($trace)
        return $null
    } catch {
        $global:OwoCdpLastError = "exception: $($_.Exception.Message)"
        $trace.Add($global:OwoCdpLastError)
        $global:OwoCdpTrace = @($trace)
        return $null
    } finally {
        try { $ws.Dispose() } catch { }
    }
}

function Get-OwoVisibleUiText {
    <#
      一次求值得到"首屏到底渲染了什么"的判定素材（选择器全部对齐 desktop/web 真实
      DOM：#health 连接指示、#routeContent 路由视图、#prompt 任务输入框、
      .service-error 统一错误卡、.setup-guide 引导页）。
      不用截屏当断言（只能给人看），也不用 HTTP 200 当断言（证明不了视图内容）。
    #>
    param([int]$Port, [int]$TimeoutSec = 10)
    $expression = @'
(function () {
  function txt(el) { return el ? (el.innerText || "").replace(/\s+/g, " ").trim() : ""; }
  function visible(el) {
    if (!el) return false;
    var r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }
  var actionable = false;
  var fields = document.querySelectorAll("textarea,input,select,button");
  for (var i = 0; i < fields.length; i++) {
    if (visible(fields[i])) { actionable = true; break; }
  }
  return JSON.stringify({
    healthText: txt(document.getElementById("health")),
    routeText: txt(document.getElementById("routeContent")),
    errorCard: txt(document.querySelector(".service-error")),
    setupGuide: txt(document.querySelector(".setup-guide, .setup-card")),
    composerVisible: visible(document.getElementById("prompt")),
    actionable: actionable,
    bodyText: txt(document.body).slice(0, 600)
  });
})()
'@
    $raw = Invoke-OwoCdpEval -Port $Port -Expression $expression -TimeoutSec $TimeoutSec
    if (-not $raw) { return $null }
    try { return ($raw | ConvertFrom-Json) } catch { return $null }
}

function Stop-OwoShellTree {
    <#
      关窗 → 必要时强杀；顺带回收本次私有根下遗留的壳/子进程。
      只按 CommandLine 含 $RunRoot 收敛，不碰用户真实会话里的同名进程。
    #>
    param([int]$ProcessId, [string]$RunRoot)
    $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($proc) {
        try { $null = $proc.CloseMainWindow() } catch { }
        Start-Sleep -Milliseconds 800
        if (-not $proc.HasExited) { Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue }
    }
    if ($RunRoot) {
        $escaped = $RunRoot.Replace('\', '\\')
        Get-CimInstance Win32_Process -Filter "Name='owo-agent-desktop.exe' OR Name='owo-agent.exe'" -ErrorAction SilentlyContinue |
            Where-Object { $_.CommandLine -and $_.CommandLine -like "*$RunRoot*" } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
}
