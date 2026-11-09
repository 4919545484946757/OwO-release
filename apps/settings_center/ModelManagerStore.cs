using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace OwO_Settings;

internal sealed record UserModelProfile(string Id, string Name, string Endpoint, string Model,
    string ProtectedKey, bool Enabled = true, bool LegacyTokenLimit = true);
internal sealed record TextAssistantSettings {
    public int Version { get; init; } = 1;
    public bool Enabled { get; init; }
    public bool PrivacyConsent { get; init; }
    public bool IncludeContext { get; init; } = true;
    public string ActiveProfile { get; init; } = "";
    public string PreferredLanguage { get; init; } = "简体中文";
    public string SecondaryLanguage { get; init; } = "英语";
    public int PolishLevel { get; init; } = 1;
    public int CodeLevel { get; init; } = 1;
    public int TimeoutSeconds { get; init; } = 45;
    public int MaxOutputTokens { get; init; } = 2048;
    public List<UserModelProfile> Profiles { get; init; } = [];
}

internal sealed class ModelManagerStore(string root) {
    internal string Root { get; } = Path.GetFullPath(root);
    internal static string DefaultRoot => RootForConfig(Environment.GetEnvironmentVariable("OWO_CONFIG_PATH") ??
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "config", "owo.conf"));
    internal static string RootForConfig(string configPath) => Path.Combine(
        Path.GetDirectoryName(Path.GetDirectoryName(Path.GetFullPath(configPath)))!, "models");
    private static readonly JsonSerializerOptions Json = new() {
        WriteIndented = true, UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow
    };
    internal TextAssistantSettings Load() {
        var path = Path.Combine(Root, "profiles.json");
        if (!File.Exists(path)) return new();
        if (new FileInfo(path).Length > 256 * 1024) throw new InvalidOperationException("模型配置文件过大。");
        var value = JsonSerializer.Deserialize<TextAssistantSettings>(File.ReadAllText(path), Json)
            ?? throw new InvalidOperationException("模型配置无效。");
        Validate(value);
        return value;
    }
    internal static Uri Endpoint(string value) {
        if (!Uri.TryCreate(value, UriKind.Absolute, out var uri) || uri.Scheme != "https" ||
            uri.UserInfo.Length != 0 || uri.Query.Length != 0 || uri.Fragment.Length != 0 || value.Length > 2048)
            throw new InvalidOperationException("API 地址必须是无用户名、查询参数和片段的 HTTPS 地址。");
        return uri.AbsolutePath.TrimEnd('/').EndsWith("/chat/completions", StringComparison.Ordinal)
            ? uri : new Uri(uri.AbsoluteUri.TrimEnd('/') + "/chat/completions");
    }
    internal static void Validate(TextAssistantSettings value) {
        if (value.Version != 1 || value.Profiles is null || value.Profiles.Count > 16 ||
            value.PolishLevel is < 1 or > 3 || value.CodeLevel is < 1 or > 3 ||
            value.TimeoutSeconds is < 5 or > 120 || value.MaxOutputTokens is < 128 or > 16384 ||
            !ValidText(value.PreferredLanguage, 64) || !ValidText(value.SecondaryLanguage, 64) ||
            value.PreferredLanguage.Equals(value.SecondaryLanguage, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("模型配置或语言、级别、长度限制无效；首选与次选语言须不同。");
        var ids = new HashSet<string>();
        foreach (var profile in value.Profiles) {
            if (profile is null || !Guid.TryParseExact(profile.Id, "N", out _) || !ids.Add(profile.Id) ||
                !ValidText(profile.Name, 80) || !ValidText(profile.Model, 160) ||
                profile.ProtectedKey is null || profile.ProtectedKey.Length > 16384)
                throw new InvalidOperationException("模型名称、标识或密钥格式无效。");
            _ = Endpoint(profile.Endpoint);
        }
        if (value.ActiveProfile is null || (value.ActiveProfile.Length != 0 && !ids.Contains(value.ActiveProfile)))
            throw new InvalidOperationException("请选择有效的默认模型。");
        if (value.Enabled && (!value.PrivacyConsent || !value.Profiles.Any(p =>
            p.Id == value.ActiveProfile && p.Enabled && p.ProtectedKey.Length != 0)))
            throw new InvalidOperationException("启用前须配置并选择模型、保存密钥并确认隐私提示。");
    }
    private static bool ValidText(string? value, int max) => !string.IsNullOrWhiteSpace(value) &&
        value.Length <= max && !value.Any(char.IsControl);
    internal void Save(TextAssistantSettings value) {
        Validate(value);
        Directory.CreateDirectory(Root);
        var path = Path.Combine(Root, "profiles.json");
        var temp = path + "." + Guid.NewGuid().ToString("N");
        try {
            File.WriteAllText(temp, JsonSerializer.Serialize(value, Json), new UTF8Encoding(false));
            File.Move(temp, path, true);
        } finally { if (File.Exists(temp)) File.Delete(temp); }
        // Native TSF reads only these non-secret gates; the worker revalidates JSON.
        SetGate("selection-assistant.enabled", value.Enabled && value.PrivacyConsent);
        SetGate("context.enabled", value.Enabled && value.PrivacyConsent && value.IncludeContext);
    }
    private void SetGate(string name, bool enabled) {
        var path = Path.Combine(Root, name);
        if (enabled) File.WriteAllText(path, "1");
        else if (File.Exists(path)) File.Delete(path);
    }
}

internal static class ModelKeyProtection {
    [StructLayout(LayoutKind.Sequential)] private struct Blob { public int Length; public IntPtr Data; }
    [DllImport("crypt32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool CryptProtectData(ref Blob input, string? description, IntPtr entropy,
        IntPtr reserved, IntPtr prompt, uint flags, out Blob output);
    [DllImport("crypt32.dll", SetLastError = true)]
    private static extern bool CryptUnprotectData(ref Blob input, IntPtr description, IntPtr entropy,
        IntPtr reserved, IntPtr prompt, uint flags, out Blob output);
    [DllImport("kernel32.dll")] private static extern IntPtr LocalFree(IntPtr value);
    internal static string Protect(string key) {
        if (key.Length is < 1 or > 4096 || key.Any(char.IsControl)) throw new InvalidOperationException("API 密钥无效。");
        return Convert.ToBase64String(Transform(Encoding.UTF8.GetBytes(key), true));
    }
    internal static string Unprotect(string value) => new UTF8Encoding(false, true).GetString(
        Transform(Convert.FromBase64String(value), false));
    private static byte[] Transform(byte[] data, bool protect) {
        var input = new Blob { Length = data.Length, Data = Marshal.AllocHGlobal(data.Length) };
        Blob output = default;
        try {
            Marshal.Copy(data, 0, input.Data, data.Length);
            var ok = protect ? CryptProtectData(ref input, "OwO user model API", IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, 1, out output)
                : CryptUnprotectData(ref input, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, 1, out output);
            if (!ok) throw new InvalidOperationException("无法使用当前 Windows 账户保护或读取 API 密钥，请重新填写。");
            var result = new byte[output.Length];
            Marshal.Copy(output.Data, result, 0, result.Length);
            return result;
        } finally {
            Marshal.Copy(new byte[data.Length], 0, input.Data, data.Length);
            Marshal.FreeHGlobal(input.Data);
            if (output.Data != IntPtr.Zero) { Marshal.Copy(new byte[output.Length], 0, output.Data, output.Length); LocalFree(output.Data); }
            CryptographicOperations.ZeroMemory(data);
        }
    }
}
