using System.Diagnostics;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Microsoft.Win32.SafeHandles;

namespace OwO_Settings;

internal sealed class LocalModelInference : IDisposable {
    private readonly Process _server;
    private readonly SafeFileHandle _job;
    private readonly FileStream _slot;
    internal HttpClient Client { get; }
    internal Uri Endpoint { get; }
    internal string SessionKey { get; }

    private LocalModelInference(Process server, SafeFileHandle job, FileStream slot, int port, string secret) {
        _server = server; _job = job; _slot = slot;
        SessionKey = secret;
        Endpoint = new Uri($"http://127.0.0.1:{port}/v1/chat/completions");
        Client = new HttpClient(new SocketsHttpHandler {
            UseProxy = false, AllowAutoRedirect = false, UseCookies = false,
            ConnectCallback = async (_, token) => {
                var socket = new Socket(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp);
                try {
                    await socket.ConnectAsync(new IPEndPoint(IPAddress.Loopback, port), token).ConfigureAwait(false);
                    var clientPort = ((IPEndPoint)socket.LocalEndPoint!).Port;
                    if (server.HasExited || !OwnsConnection(server.Id, port, clientPort))
                        throw new IOException("本地推理连接不是 OwO 启动的进程。");
                    return new NetworkStream(socket, ownsSocket: true);
                } catch { socket.Dispose(); throw; }
            }
        }) { Timeout = Timeout.InfiniteTimeSpan };
    }

    internal static async Task<LocalModelInference> StartAsync(LocalModelStore store, LocalModel model, bool performance, CancellationToken token,
        bool debug = false) {
        if (!store.Installed(model)) throw new ZeroCandidateFailure("local-not-installed", "本地模型未完成下载或不受此设备支持。");
        FileStream? slot = null;
        for (int attempts = 0; slot is null; ++attempts) {
            token.ThrowIfCancellationRequested();
            try { slot = new FileStream(store.LockPath, FileMode.OpenOrCreate, FileAccess.ReadWrite,
                       FileShare.None, 1, FileOptions.None); }
            catch (IOException) when (attempts < 5) { await Task.Delay(150, token).ConfigureAwait(false); }
            catch (IOException) { throw new ZeroCandidateFailure("local-busy", "另一个本地候选正在生成。"); }
        }
        Process? server = null;
        SafeFileHandle? job = null;
        LocalModelInference? instance = null;
        try {
            var reservation = new TcpListener(IPAddress.Loopback, 0);
            int port;
            reservation.Start();
            try { port = ((IPEndPoint)reservation.LocalEndpoint).Port; }
            finally { reservation.Stop(); }
            var secret = Convert.ToHexString(RandomNumberGenerator.GetBytes(24));
            var start = new ProcessStartInfo(store.Executable) {
                UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = Path.GetDirectoryName(store.Executable)!
            };
            foreach (var key in start.Environment.Keys.Where(k => k.StartsWith("LLAMA_", StringComparison.OrdinalIgnoreCase) ||
                k.StartsWith("GGML_", StringComparison.OrdinalIgnoreCase) || k.Equals("HF_TOKEN", StringComparison.OrdinalIgnoreCase)).ToArray())
                start.Environment.Remove(key);
            start.Environment["LLAMA_API_KEY"] = secret;
            foreach (var arg in new[] { "-m", store.ModelPath(model), "--host", "127.0.0.1", "--port", port.ToString(),
                "--alias", model.Id, "-ngl", "0", "-c", debug ? "4096" : performance ? "1024" : "512", "-np", "1",
                "-t", Math.Clamp(Environment.ProcessorCount / 2, 1, performance ? 4 : 2).ToString(),
                "--reasoning", "off", "--offline", "--no-ui", "--log-disable", "--no-agent" })
                start.ArgumentList.Add(arg);
            job = NewJob();
            server = Process.Start(start);
            if (server is null || !AssignProcessToJobObject(job, server.SafeHandle))
                throw new ZeroCandidateFailure("local-start", "无法启动隔离的本地推理进程。");
            instance = new LocalModelInference(server, job, slot, port, secret);
            job = null; server = null; slot = null;
            await instance.WaitReadyAsync(token).ConfigureAwait(false);
            return instance;
        } catch {
            instance?.Dispose();
            if (server is not null) { Stop(server); server.Dispose(); }
            job?.Dispose(); slot?.Dispose();
            throw;
        }
    }

    private async Task WaitReadyAsync(CancellationToken token) {
        var ready = new Uri(Endpoint, "/health");
        while (true) {
            token.ThrowIfCancellationRequested();
            if (_server.HasExited) throw new ZeroCandidateFailure("local-start", "本地运行时退出，请检查 CPU、运行库和模型文件。");
            try {
                using var request = new HttpRequestMessage(HttpMethod.Get, ready);
                request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", SessionKey);
                using var response = await Client.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, token).ConfigureAwait(false);
                if (response.StatusCode == HttpStatusCode.OK) return;
            } catch (HttpRequestException) { /* Waiting for the owned socket; do not send input while starting. */ }
            await Task.Delay(140, token).ConfigureAwait(false);
        }
    }

    public void Dispose() {
        Client.Dispose(); _job.Dispose(); Stop(_server); _server.Dispose(); _slot.Dispose();
    }
    private static void Stop(Process process) {
        try { if (!process.HasExited) process.Kill(); process.WaitForExit(700); }
        catch (InvalidOperationException) { } catch (System.ComponentModel.Win32Exception) { }
    }

    // Match the accepted connection's server-side row, not just a listening
    // port. No prompt is sent to a process that won a port race.
    internal static bool OwnsConnection(int pid, int serverPort, int clientPort) {
        const int TableOwnerPidAll = 5, AfInet = 2, Address127001 = 0x0100007f;
        int size = 0;
        GetExtendedTcpTable(IntPtr.Zero, ref size, false, AfInet, TableOwnerPidAll, 0);
        if (size is < 4 or > 4 * 1024 * 1024) return false;
        var table = Marshal.AllocHGlobal(size);
        try {
            if (GetExtendedTcpTable(table, ref size, false, AfInet, TableOwnerPidAll, 0) != 0) return false;
            int count = Marshal.ReadInt32(table);
            if (count < 0 || count > (size - 4) / 24) return false;
            for (var offset = 4; offset < 4 + count * 24; offset += 24) {
                if (Marshal.ReadInt32(table, offset) == 5 && Marshal.ReadInt32(table, offset + 4) == Address127001 &&
                    Marshal.ReadInt32(table, offset + 12) == Address127001 && Marshal.ReadInt32(table, offset + 20) == pid &&
                    Port(Marshal.ReadInt32(table, offset + 8)) == serverPort && Port(Marshal.ReadInt32(table, offset + 16)) == clientPort)
                    return true;
            }
            return false;
        } finally { Marshal.FreeHGlobal(table); }
    }
    private static int Port(int raw) => ((raw & 0xff) << 8) | ((raw >> 8) & 0xff);

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicLimits {
        internal long ProcessTime, JobTime;
        internal uint Flags;
        internal nuint MinimumWorkingSet, MaximumWorkingSet;
        internal uint ActiveProcessLimit;
        internal nuint Affinity;
        internal uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct JobLimits {
        internal BasicLimits Basic;
        internal ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
        internal nuint ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    private static SafeFileHandle NewJob() {
        var job = CreateJobObjectW(IntPtr.Zero, null);
        var limits = new JobLimits { Basic = new BasicLimits { Flags = 0x2000 } }; // KILL_ON_JOB_CLOSE
        if (!job.IsInvalid && SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf<JobLimits>())) return job;
        job.Dispose();
        throw new ZeroCandidateFailure("local-start", "无法隔离本地推理进程。");
    }
    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true, ExactSpelling = true)]
    private static extern SafeFileHandle CreateJobObjectW(IntPtr attributes, string? name);
    [DllImport("kernel32", SetLastError = true)]
    private static extern bool SetInformationJobObject(SafeFileHandle job, int infoType, ref JobLimits value, uint size);
    [DllImport("kernel32", SetLastError = true)]
    private static extern bool AssignProcessToJobObject(SafeFileHandle job, SafeProcessHandle process);
    [DllImport("iphlpapi")]
    private static extern uint GetExtendedTcpTable(IntPtr buffer, ref int size, bool ordered, int family, int type, uint reserved);
}
