using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using OwO_Settings;

internal static class InstallerPayloadTests
{
    private static readonly string[] Required = ["bin/OwO.TSF.dll", "bin/x86/OwO.TSF.dll", "bin/owo_core_service.exe",
        "scripts/Install-OwO.ps1", "scripts/Install-Prerequisites.ps1", "scripts/Uninstall-OwO.ps1",
        "settings/OwO.Settings.exe", "data/owo-cn.owolx", "model/zh_CN.lm"];

    private static byte[] MakeArchive(string? extra = null, bool wrongManifest = false, bool symlink = false)
    {
        using var buffer = new MemoryStream();
        using (var zip = new ZipArchive(buffer, ZipArchiveMode.Create, leaveOpen: true)) {
            var manifest = new StringBuilder();
            foreach (var name in Required) {
                var data = Encoding.UTF8.GetBytes("fixture:" + name);
                var entry = zip.CreateEntry("root/" + name);
                using (var stream = entry.Open()) stream.Write(data);
                manifest.Append(wrongManifest ? new string('0', 64) : Convert.ToHexString(SHA256.HashData(data)))
                    .Append("  ").Append(name).Append('\n');
            }
            using (var stream = zip.CreateEntry("root/SHA256SUMS.txt").Open())
                stream.Write(Encoding.UTF8.GetBytes(manifest.ToString()));
            if (extra is not null) {
                var entry = zip.CreateEntry(extra);
                if (symlink) entry.ExternalAttributes = unchecked((int)0xa0000000);
                using var stream = entry.Open();
                stream.WriteByte(1);
            }
        }
        return buffer.ToArray();
    }

    internal static async Task RunAsync()
    {
        var good = MakeArchive();
        var payload = new InstallerPayload(() => new MemoryStream(good), Convert.ToHexString(SHA256.HashData(good)));
        if (Directory.Exists(payload.PackageRoot)) throw new Exception("Payload extracted before install confirmation");
        try {
            await payload.PrepareAsync();
            foreach (var name in Required)
                if (!File.Exists(Path.Combine(payload.PackageRoot, name))) throw new Exception("Missing extracted fixture");
            await payload.PrepareAsync();
            File.AppendAllText(Path.Combine(payload.PackageRoot, Required[0]), "tampered");
            var tamperingRejected = false;
            try { await payload.PrepareAsync(); } catch (InvalidDataException) { tamperingRejected = true; }
            if (!tamperingRejected || Directory.Exists(payload.PackageRoot))
                throw new Exception("Retry accepted a modified payload or leaked the workspace");
        } finally { payload.Cleanup(); }
        if (Directory.Exists(payload.PackageRoot)) throw new Exception("Owned workspace was not cleaned up");
        foreach (var bad in new[] {
            MakeArchive("root/../escaped.txt"), MakeArchive("root/bin/OWO.TSF.DLL"),
            MakeArchive("root/link", symlink: true), MakeArchive("root/unlisted.exe"),
            MakeArchive(wrongManifest: true)
        }) await RejectAsync(bad, Convert.ToHexString(SHA256.HashData(bad)));
        await RejectAsync(good, new string('0', 64));
        Console.WriteLine("Installer payload integrity, delayed extraction, traversal/duplicate/symlink rejection and cleanup tests passed.");
    }

    private static async Task RejectAsync(byte[] archive, string hash)
    {
        var payload = new InstallerPayload(() => new MemoryStream(archive), hash);
        var rejected = false;
        try { await payload.PrepareAsync(); } catch (InvalidDataException) { rejected = true; }
        finally { payload.Cleanup(); }
        if (!rejected || Directory.Exists(payload.PackageRoot)) throw new Exception("Unsafe payload was accepted or leaked files");
    }

    internal static async Task VerifyReleaseAsync(string path)
    {
        string hash;
        using (var stream = File.OpenRead(path)) hash = Convert.ToHexString(SHA256.HashData(stream));
        var payload = new InstallerPayload(() => File.OpenRead(path), hash);
        try { await payload.PrepareAsync(); }
        finally { payload.Cleanup(); }
        if (Directory.Exists(payload.PackageRoot)) throw new Exception("Release extraction leaked workspace");
        Console.WriteLine("Actual release payload extraction, per-file verification and cleanup passed (no installation executed).");
    }
}
