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
    private bool _uninstallMode;
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
        UnhandledException += (_, args) => { if (!_uninstallMode) WriteStartupFailure(args.Exception); };
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
    protected override async void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        var arguments = Environment.GetCommandLineArgs();
        if (arguments.Contains("--text-assistant-host")) {
            try { await TextAssistantHost.RunAsync(arguments); } catch { /* Never log selections or credentials. */ }
            Exit();
            return;
        }
        string? ValueOf(string option) {
            var index = Array.IndexOf(arguments, option);
            return index >= 0 && index + 1 < arguments.Length ? arguments[index + 1] : null;
        }
        if (arguments.Contains("--uninstall-root") || arguments.Contains("--uninstall-preview") ||
            arguments.Contains("--verify-uninstall-ui")) {
            _uninstallMode = true;
            var verify = arguments.Contains("--verify-uninstall-ui");
            var preview = verify ? "options" : arguments.Contains("--uninstall-preview") ?
                ValueOf("--uninstall-preview") ?? "options" : null;
            var window = new Window { Title = preview is null ? "卸载 OwO 输入法" : "卸载 OwO 输入法 · 预览" };
            _window = window;
            window.AppWindow.Resize(new Windows.Graphics.SizeInt32(920, 620));
            var report = ValueOf("--uninstall-test-report");
            try {
                var page = new UninstallPage(ValueOf("--uninstall-root"), ValueOf("--uninstall-workspace"), preview, window.Close);
                window.Content = page;
                WindowTitleBarAppearance.Bind(window, page);
                window.AppWindow.Closing += (_, closing) => { if (page.Busy) closing.Cancel = true; };
                window.Activate();
                if (verify) {
                    try {
                        await page.VerifyAsync(width => window.AppWindow.Resize(new Windows.Graphics.SizeInt32(width, 620)),
                            window.AppWindow.TitleBar);
                        if (report is not null) File.WriteAllLines(report, page.VerificationReport);
                    } catch (Exception error) {
                        Environment.ExitCode = 1;
                        if (report is not null) File.WriteAllText(report, error.ToString());
                    } finally { window.Close(); }
                }
            } catch (Exception error) {
                Environment.ExitCode = 1;
                if (verify) {
                    if (report is not null) File.WriteAllText(report, error.ToString());
                    Exit();
                } else {
                    window.Content = new TextBlock { Text = $"无法启动卸载助手：{error.Message}",
                        TextWrapping = TextWrapping.Wrap, Margin = new Thickness(24) };
                    window.Activate();
                }
            }
            return;
        }
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
