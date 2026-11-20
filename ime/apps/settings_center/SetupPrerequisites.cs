namespace OwO_Settings;

internal static class SetupPrerequisites
{
    internal static string ArchitectureError(System.Runtime.InteropServices.Architecture architecture) =>
        architecture == System.Runtime.InteropServices.Architecture.X64 ? "" :
        "此安装包仅支持 x64 Windows，暂不支持 ARM64 或 32 位 Windows。";
    // Explicit marker + exit code: PowerShell nonterminating errors must never
    // look like a successful probe. Registry is the same per-user source used
    // by Windows language preferences, not the machine display language.
    internal const string LanguageProbe = """
        $ErrorActionPreference = 'Stop'
        try {
            $tags = @((Get-ItemProperty -LiteralPath 'HKCU:\Control Panel\International\User Profile' -Name Languages -ErrorAction SilentlyContinue).Languages)
            if (@($tags | Where-Object { $_ -in @('zh-Hans-CN','zh-CN') }).Count -gt 0) {
                Write-Output 'OWO_LANGUAGE_READY'; exit 0
            }
            foreach ($language in Get-WinUserLanguageList -ErrorAction Stop) {
                if ($language.LanguageTag -in @('zh-Hans-CN','zh-CN')) {
                    Write-Output 'OWO_LANGUAGE_READY'; exit 0
                }
            }
            Write-Output 'OWO_LANGUAGE_MISSING'; exit 1
        } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 2 }
        """;

    internal static bool LanguageReady(int exitCode, string output) =>
        exitCode == 0 && output.Trim() == "OWO_LANGUAGE_READY";

    internal static string BlockingReason(bool preview, bool checking, bool languageReady,
        bool packageReady, bool consent, bool pathValid, string pathError) =>
        preview ? "当前为界面预览模式，不会执行安装。" :
        checking ? "正在检查安装环境，请稍候。" :
        !packageReady ? "安装包不完整，请查看安装检查提示。" :
        !languageReady ? "语言检查尚未通过，请查看检测结果并重新检查。" :
        !pathValid ? pathError :
        !consent ? "请先确认安装来源可信。" : "";
}
