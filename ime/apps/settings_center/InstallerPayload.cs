using System.IO.Compression;
using System.Reflection;
using System.Security.Cryptography;

namespace OwO_Settings;

internal sealed record InstallerPayloadProgress(int Percent, string Message);

internal sealed class InstallerPayload
{
    private readonly Func<Stream> _openArchive;
    private readonly string _archiveHash;
    private readonly string _workspace = Path.Combine(Path.GetTempPath(), "OwO-Setup-" + Guid.NewGuid().ToString("N"));
    private bool _ownsWorkspace;
    private bool _prepared;
    internal string PackageRoot => Path.Combine(_workspace, "Package");

    internal InstallerPayload(Func<Stream> openArchive, string archiveHash)
    {
        if (archiveHash.Length != 64 || archiveHash.Any(value => !Uri.IsHexDigit(value)))
            throw new InvalidDataException("安装载荷校验值无效。");
        _openArchive = openArchive;
        _archiveHash = archiveHash;
    }

    internal static InstallerPayload FromAssembly(Assembly assembly)
    {
        using var receipt = assembly.GetManifestResourceStream("OwO.Setup.Payload.sha256")
            ?? throw new InvalidDataException("缺少安装载荷校验记录。");
        using var reader = new StreamReader(receipt);
        return new InstallerPayload(() => assembly.GetManifestResourceStream("OwO.Setup.Payload.zip")
            ?? throw new InvalidDataException("缺少内置安装文件。"), reader.ReadToEnd().Trim());
    }

    internal Task PrepareAsync(IProgress<InstallerPayloadProgress>? progress = null) => Task.Run(() => {
        try {
            if (_prepared) { ValidatePackage(); return; }
            progress?.Report(new(0, "正在校验内置安装包…"));
            using var stream = _openArchive();
            var actual = Convert.ToHexString(SHA256.HashData(stream));
            if (!actual.Equals(_archiveHash, StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("内置安装包校验失败，未执行安装。");
            stream.Position = 0;
            using var archive = new ZipArchive(stream, ZipArchiveMode.Read);
            if (archive.Entries.Count is 0 or > 20000)
                throw new InvalidDataException("安装包文件数量异常。");
            var plans = new List<(ZipArchiveEntry Entry, string Target)>();
            var targets = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            string? prefix = null;
            long size = 0;
            foreach (var entry in archive.Entries) {
                var name = entry.FullName.Replace('\\', '/');
                var pieces = name.TrimEnd('/').Split('/');
                if (pieces.Length == 0 || pieces.Any(value => string.IsNullOrEmpty(value) ||
                    value is "." or ".." || value.EndsWith('.') || value.EndsWith(' ') ||
                    value.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0))
                    throw new InvalidDataException("安装包包含不安全路径。");
                prefix ??= pieces[0];
                if (!prefix.Equals(pieces[0], StringComparison.Ordinal) ||
                    (pieces.Length < 2 && !name.EndsWith('/')))
                    throw new InvalidDataException("安装包根目录异常。");
                // Reject Unix symlinks and Windows reparse-point attributes.
                if (((entry.ExternalAttributes >> 16) & 0xf000) == 0xa000 ||
                    (entry.ExternalAttributes & (int)FileAttributes.ReparsePoint) != 0)
                    throw new InvalidDataException("安装包不允许符号链接。");
                if (name.EndsWith('/')) continue;
                var target = ResolvePath(PackageRoot, string.Join('/', pieces.Skip(1)));
                if (!targets.Add(target)) throw new InvalidDataException("安装包文件路径重复。");
                size = checked(size + entry.Length);
                if (size > 2L * 1024 * 1024 * 1024) throw new InvalidDataException("安装包解压大小超限。");
                plans.Add((entry, target));
            }
            if (Directory.Exists(_workspace) || File.Exists(_workspace))
                throw new IOException("安装临时目录已存在，请重启安装器。");
            Directory.CreateDirectory(_workspace);
            _ownsWorkspace = true;
            Directory.CreateDirectory(PackageRoot);
            long extracted = 0;
            var reported = -1;
            foreach (var plan in plans) {
                Directory.CreateDirectory(Path.GetDirectoryName(plan.Target)!);
                using (var input = plan.Entry.Open())
                using (var output = new FileStream(plan.Target, FileMode.CreateNew, FileAccess.Write))
                    input.CopyTo(output);
                extracted += plan.Entry.Length;
                var percent = 10 + (int)(60 * extracted / Math.Max(1, size));
                if (percent != reported) {
                    progress?.Report(new(percent, "正在解压安装文件…"));
                    reported = percent;
                }
            }
            progress?.Report(new(75, "正在验证解压文件完整性…"));
            ValidatePackage();
            _prepared = true;
            progress?.Report(new(100, "安装文件已准备就绪。"));
        } catch {
            Cleanup();
            throw;
        }
    });

    private static string ResolvePath(string root, string relative)
    {
        if (Path.IsPathRooted(relative)) throw new InvalidDataException("安装包包含绝对路径。");
        var full = Path.GetFullPath(Path.Combine(root, relative));
        if (!full.StartsWith(Path.GetFullPath(root) + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("安装包路径超出临时目录。");
        return full;
    }

    private void ValidatePackage()
    {
        var verified = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var line in File.ReadLines(Path.Combine(PackageRoot, "SHA256SUMS.txt"))) {
            if (line.Length < 67 || line.Substring(64, 2) != "  " ||
                line.Take(64).Any(value => !Uri.IsHexDigit(value)))
                throw new InvalidDataException("安装包文件校验清单无效。");
            var file = ResolvePath(PackageRoot, line[66..]);
            if (!verified.Add(file)) throw new InvalidDataException("文件校验清单存在重复项。");
            using var stream = File.OpenRead(file);
            if (!Convert.ToHexString(SHA256.HashData(stream)).Equals(line[..64], StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("安装文件校验失败：" + line[66..]);
        }
        foreach (var file in Directory.EnumerateFiles(PackageRoot, "*", SearchOption.AllDirectories)) {
            if (file != Path.Combine(PackageRoot, "SHA256SUMS.txt") && !verified.Contains(file))
                throw new InvalidDataException("安装包含未校验文件。");
        }
        foreach (var required in new[] { "bin/OwO.TSF.dll", "bin/x86/OwO.TSF.dll", "bin/owo_core_service.exe",
            "scripts/Install-OwO.ps1", "scripts/Install-Prerequisites.ps1", "scripts/Uninstall-OwO.ps1",
            "settings/OwO.Settings.exe", "data/owo-cn.owolx", "model/zh_CN.lm" })
            if (!verified.Contains(ResolvePath(PackageRoot, required)))
                throw new InvalidDataException("安装文件缺失：" + required);
    }

    internal void Cleanup()
    {
        if (!_ownsWorkspace) return;
        try {
            // Delete only our exact GUID workspace; never follow redirected directories.
            if (Directory.Exists(_workspace)) {
                if ((File.GetAttributes(_workspace) & FileAttributes.ReparsePoint) != 0) return;
                var pending = new Stack<string>();
                pending.Push(_workspace);
                while (pending.TryPop(out var directory)) {
                    foreach (var path in Directory.EnumerateFileSystemEntries(directory)) {
                        var attributes = File.GetAttributes(path);
                        if ((attributes & FileAttributes.ReparsePoint) != 0) return;
                        if ((attributes & FileAttributes.Directory) != 0) pending.Push(path);
                    }
                }
                Directory.Delete(_workspace, recursive: true);
            }
            _ownsWorkspace = false;
            _prepared = false;
        } catch (IOException) { } catch (UnauthorizedAccessException) { }
    }
}
