using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace OwO_Settings;

internal static class Program
{
    [STAThread]
    private static void Main()
    {
        try {
            App.LogStage("Program entry");
            // WinUI's resource lookup must use the self-extracted runtime directory,
            // not the directory containing the distributable single-file EXE.
#pragma warning disable IL3000 // All content is extracted by the .NET single-file host.
            var runtimeRoot = Path.GetDirectoryName(typeof(Program).Assembly.Location);
#pragma warning restore IL3000
            if (!string.IsNullOrEmpty(runtimeRoot)) {
                AppContext.SetData("APP_CONTEXT_BASE_DIRECTORY", runtimeRoot + Path.DirectorySeparatorChar);
                Environment.SetEnvironmentVariable("MICROSOFT_WINDOWSAPPRUNTIME_BASE_DIRECTORY", runtimeRoot + Path.DirectorySeparatorChar);
            }
            WinRT.ComWrappersSupport.InitializeComWrappers();
            App.LogStage("WinRT initialized: " + AppContext.BaseDirectory);
            Application.Start(initialization => {
                try {
                    SynchronizationContext.SetSynchronizationContext(
                        new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
                    _ = new App();
                } catch (Exception error) {
                    App.LogFailure(error);
                    throw;
                }
            });
        } catch (Exception error) {
            App.LogFailure(error);
            throw;
        }
    }
}
