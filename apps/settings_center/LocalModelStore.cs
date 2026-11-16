using System.IO.Compression;
using System.Security.Cryptography;
using System.Runtime.InteropServices;
using System.Runtime.Intrinsics.X86;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record ModelAsset(string Url, long Bytes, string Sha256);
internal sealed record LocalModel(string Id, string Name, string Description, ModelAsset Weights);

internal sealed class LocalModelStore(string modelRoot) {
    internal static readonly ModelAsset Runtime = new(
        "https://github.com/ggml-org/llama.cpp/releases/download/b11887/llama-b11887-bin-win-cpu-x64.zip",
        24219283, "cf6a48560e0c32baf875f0b84ac578b111e16115ccf2382836489b5e268809c4");
    internal static readonly IReadOnlyList<LocalModel> Catalog = [
        new("qwen35-08b-q4", "Qwen3.5-0.8B · Q4_0", "推荐 · 中文生成与少量上下文 · ggml-org / Apache-2.0",
            new("https://huggingface.co/ggml-org/Qwen3.5-0.8B-GGUF/resolve/004fd086eccea9f7cf867aa07ba328773beef81d/Qwen3.5-0.8B-Q4_0.gguf",
                563036064, "57d1997790d1744fba5b40a7317df71ea5e2acee28c47e78f0cce39c0703f8cf")),
    ];
    internal static LocalModel? Find(string? id) => Catalog.FirstOrDefault(x => x.Id == id);
    internal static bool Supported => OperatingSystem.IsWindows() && RuntimeInformation.ProcessArchitecture == Architecture.X64 && Avx2.IsSupported;
    internal string Root { get; } = Path.Combine(Path.GetFullPath(modelRoot), "local");
    private string RuntimeRoot => Path.Combine(Root, "llama-b11887");
    internal string Executable => Path.Combine(RuntimeRoot, "llama-server.exe");
    internal string ModelPath(LocalModel model) => Path.Combine(Root, model.Id + ".gguf");
    private static bool Regular(string path) => File.Exists(path) &&
        (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0;
    private static bool SafeDirectory(string path) => Directory.Exists(path) &&
        (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0;
    internal bool HasRuntime => SafeDirectory(Root) && SafeDirectory(RuntimeRoot) && Regular(Executable)
        && Regular(Path.Combine(RuntimeRoot, "verified.sha256"))
        && new FileInfo(Path.Combine(RuntimeRoot, "verified.sha256")).Length == Runtime.Sha256.Length
        && File.ReadAllText(Path.Combine(RuntimeRoot, "verified.sha256")) == Runtime.Sha256;
    internal bool HasWeights(LocalModel model) => SafeDirectory(Root) && Regular(ModelPath(model)) &&
        new FileInfo(ModelPath(model)).Length == model.Weights.Bytes;
    internal bool Installed(LocalModel model) {
        try { return Supported && HasRuntime && HasWeights(model); } catch (IOException) { return false; }
        catch (UnauthorizedAccessException) { return false; }
    }
    internal void Remove(LocalModel model) {
        if (SafeDirectory(Root) && Regular(ModelPath(model))) File.Delete(ModelPath(model));
    }

    internal async Task<bool> InstallAsync(LocalModel model, DownloadOperation operation, CancellationToken token) {
        if (!Supported) throw new InvalidOperationException("本地模型需要 Windows x64 和支持 AVX2 的 CPU。");
        Directory.CreateDirectory(Root);
        if (!SafeDirectory(Root) || !SafeDirectory(Path.GetDirectoryName(Root)!))
            throw new InvalidOperationException("本地模型目录不能是链接。");
        var runtimeMissing = !HasRuntime;
        var modelMissing = !HasWeights(model);
        if (runtimeMissing && Directory.Exists(RuntimeRoot))
            throw new InvalidOperationException("本地运行时文件不完整，请移走模型目录内的 llama-b11887 文件夹后重试。");
        var total = (runtimeMissing ? Runtime.Bytes : 0) + (modelMissing ? model.Weights.Bytes : 0);
        if (total == 0) return true;
        if (new DriveInfo(Path.GetPathRoot(Root)!).AvailableFreeSpace < total + 96L * 1024 * 1024)
            throw new InvalidOperationException("磁盘剩余空间不足。");
        using var client = new HttpClient(new HttpClientHandler { AllowAutoRedirect = false }) { Timeout = Timeout.InfiniteTimeSpan };
        long downloaded = 0;
        if (runtimeMissing) {
            var stage = Path.Combine(Root, ".runtime-" + Guid.NewGuid().ToString("N"));
            var archive = stage + ".zip";
            try {
                await FetchAsync(client, Runtime, archive, operation, downloaded, total, token).ConfigureAwait(false);
                downloaded += Runtime.Bytes;
                token.ThrowIfCancellationRequested();
                Directory.CreateDirectory(stage);
                UnpackRuntime(archive, stage, token);
                File.WriteAllText(Path.Combine(stage, "verified.sha256"), Runtime.Sha256);
                token.ThrowIfCancellationRequested();
                try { Directory.Move(stage, RuntimeRoot); } catch (IOException) when (HasRuntime) { }
            } finally {
                File.Delete(archive);
                if (Directory.Exists(stage)) Directory.Delete(stage, true);
            }
        }
        if (modelMissing) {
            var part = Path.Combine(Root, ".model-" + Guid.NewGuid().ToString("N") + ".part");
            try {
                await FetchAsync(client, model.Weights, part, operation, downloaded, total, token).ConfigureAwait(false);
                token.ThrowIfCancellationRequested();
                File.Move(part, ModelPath(model), true);
            } finally { File.Delete(part); }
        }
        return true;
    }

    internal static bool AllowedDownload(Uri uri) => uri.Scheme == Uri.UriSchemeHttps && uri.IsDefaultPort && uri.UserInfo.Length == 0 &&
        (uri.IdnHost is "github.com" or "release-assets.githubusercontent.com" or "huggingface.co" ||
            uri.IdnHost.EndsWith(".huggingface.co", StringComparison.Ordinal) || uri.IdnHost.EndsWith(".hf.co", StringComparison.Ordinal));

    internal static async Task FetchAsync(HttpClient client, ModelAsset asset, string target, DownloadOperation operation,
        long before, long total, CancellationToken token) {
        var uri = new Uri(asset.Url);
        HttpResponseMessage? final = null;
        for (int hop = 0; hop < 8; ++hop) {
            if (!AllowedDownload(uri)) throw new InvalidOperationException("下载跳转到了未经允许的站点。");
            var response = await client.GetAsync(uri, HttpCompletionOption.ResponseHeadersRead, token).ConfigureAwait(false);
            if ((int)response.StatusCode is 301 or 302 or 303 or 307 or 308 && response.Headers.Location is Uri redirect) {
                uri = new Uri(uri, redirect);
                response.Dispose();
            } else { final = response; break; }
        }
        using var download = final ?? throw new InvalidOperationException("下载跳转次数过多。");
        download.EnsureSuccessStatusCode();
        if (download.Content.Headers.ContentLength is long declared && declared != asset.Bytes)
            throw new InvalidDataException("下载长度与官方清单不符。");
        await using var input = await download.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
        await using var output = new FileStream(target, FileMode.CreateNew, FileAccess.Write, FileShare.None, 128 * 1024, true);
        using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        var buffer = new byte[128 * 1024];
        long received = 0;
        while (true) {
            await operation.WaitIfPausedAsync(token).ConfigureAwait(false);
            int count = await input.ReadAsync(buffer, token).ConfigureAwait(false);
            if (count == 0) break;
            received += count;
            if (received > asset.Bytes) throw new InvalidDataException("下载长度超出官方清单。");
            hash.AppendData(buffer, 0, count);
            await output.WriteAsync(buffer.AsMemory(0, count), token).ConfigureAwait(false);
            operation.Report(before + received, total);
        }
        if (received != asset.Bytes || !Convert.ToHexString(hash.GetHashAndReset()).Equals(asset.Sha256, StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("下载长度或 SHA-256 校验失败，文件未安装。");
    }
    internal static void UnpackRuntime(string zipFile, string directory, CancellationToken token) {
        using var zip = ZipFile.OpenRead(zipFile);
        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        long unpacked = 0;
        foreach (var entry in zip.Entries) {
            token.ThrowIfCancellationRequested();
            var name = entry.Name;
            if (name != "llama-server.exe" && !Regex.IsMatch(name, @"\A[a-zA-Z0-9_.-]+\.dll\z") &&
                !name.StartsWith("LICENSE", StringComparison.OrdinalIgnoreCase)) continue;
            if (!Regex.IsMatch(name, @"\A[a-zA-Z0-9_.-]+\z") || !names.Add(name) ||
                (unpacked += entry.Length) > 180 * 1024 * 1024)
                throw new InvalidDataException("运行时 ZIP 布局无效。");
            entry.ExtractToFile(Path.Combine(directory, name));
        }
        if (!names.Contains("llama-server.exe") || !names.Contains("llama.dll") || !names.Contains("ggml.dll"))
            throw new InvalidDataException("运行时 ZIP 不完整。");
    }
}
