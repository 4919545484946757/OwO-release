using Windows.ApplicationModel;
using Windows.ApplicationModel.Activation;
using Windows.Foundation;
using Windows.Foundation.Collections;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Navigation;
using Microsoft.UI.Xaml.Shapes;

// To learn more about WinUI, the WinUI project structure,
// and more about our project templates, see: http://aka.ms/winui-project-info.

namespace OwO_Settings;

/// <summary>
/// Provides application-specific behavior to supplement the default Application class.
/// </summary>
public partial class App : Application
{
    private Window? _window;
    internal static string? InstallPackageRoot { get; private set; }
    internal static string? InstallPreviewStage { get; private set; }
    internal static InstallerPayload? BundledInstaller => null;
    internal static string ResourceDirectory => AppContext.BaseDirectory;
    internal Window MainWindow => _window
        ?? throw new InvalidOperationException("主窗口尚未创建。");
    
    /// <summary>
    /// Initializes the singleton application object.  This is the first line of authored code
    /// executed, and as such is the logical equivalent of main() or WinMain().
    /// </summary>
    public App()
    {
        UnhandledException += (_, args) => WriteStartupFailure(args.Exception);
        InitializeComponent();
    }

    private static void WriteStartupFailure(Exception error)
    {
        try {
            var directory = System.IO.Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "OwO", "InputMethod", "logs");
            Directory.CreateDirectory(directory);
            File.WriteAllText(System.IO.Path.Combine(directory, "settings-crash.log"), error.ToString());
        } catch {
            // Preserve the original WinUI exception when diagnostics cannot be written.
        }
    }

    internal static void LogStage(string stage)
    {
        try {
            var directory = System.IO.Path.Combine(Environment.GetFolderPath(
                Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "logs");
            Directory.CreateDirectory(directory);
            File.AppendAllText(System.IO.Path.Combine(directory, "setup-startup.log"),
                $"{DateTimeOffset.Now:O} {stage}\n");
        } catch { }
    }

    /// <summary>
    /// Invoked when the application is launched.
    /// </summary>
    /// <param name="args">Details about the launch request and process.</param>
    protected override void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        var arguments = Environment.GetCommandLineArgs();
        var installIndex = Array.FindIndex(arguments, value =>
            value.Equals("--install", StringComparison.OrdinalIgnoreCase));
        if (installIndex >= 0 && installIndex + 1 < arguments.Length)
            InstallPackageRoot = arguments[installIndex + 1];
        var previewIndex = Array.FindIndex(arguments, value =>
            value.Equals("--install-preview-stage", StringComparison.OrdinalIgnoreCase));
        if (previewIndex >= 0 && previewIndex + 1 < arguments.Length)
            InstallPreviewStage = arguments[previewIndex + 1].ToLowerInvariant();
        _window = new MainWindow();
        _window.Activate();
    }
}
