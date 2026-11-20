using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;

namespace OwO_Settings;

public sealed partial class App : Application
{
    internal static InstallerPayload? BundledInstaller { get; private set; }
    internal static string? InstallPackageRoot => BundledInstaller?.PackageRoot;
    internal static string? InstallPreviewStage { get; private set; }
#pragma warning disable IL3000 // IncludeAllContentForSelfExtract guarantees extracted managed assemblies.
    internal static string ResourceDirectory => Path.GetDirectoryName(typeof(App).Assembly.Location) ?? AppContext.BaseDirectory;
#pragma warning restore IL3000
    internal Window MainWindow { get; private set; } = null!;

    public App()
    {
        LogStage("Application constructor");
        UnhandledException += (_, args) => {
            LogStage("Unhandled UI exception: " + args.Message);
            LogFailure(args.Exception);
        };
        ResourceManagerRequested += (_, args) => {
            var resources = new Microsoft.Windows.ApplicationModel.Resources.ResourceManager(Path.Combine(ResourceDirectory, "resources.pri"));
            resources.ResourceNotFound += (_, missing) => LogStage("Resource not found: " + missing.Name);
            args.CustomResourceManager = resources;
        };
        InitializeComponent();
        LogStage("XAML initialized");
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        LogStage("Application launched");
        BundledInstaller = InstallerPayload.FromAssembly(typeof(App).Assembly);
        var arguments = Environment.GetCommandLineArgs();
        var index = Array.FindIndex(arguments, value => value == "--install-preview-stage");
        if (index >= 0 && index + 1 < arguments.Length) InstallPreviewStage = arguments[index + 1];
        MainWindow = new Window { Title = "安装 OwO 输入法", SystemBackdrop = new MicaBackdrop() };
        var titleBar = new TitleBar {
            Title = InstallPreviewStage is null ? "安装 OwO 输入法" : "安装 OwO 输入法 · 阶段预览",
            IconSource = new ImageIconSource { ImageSource = new BitmapImage(new Uri(Path.Combine(ResourceDirectory, "Assets", "AppIcon.ico"))) }
        };
        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.Children.Add(titleBar);
        MainWindow.Content = root;
        MainWindow.ExtendsContentIntoTitleBar = true;
        MainWindow.SetTitleBar(titleBar);
        MainWindow.AppWindow.SetIcon(Path.Combine(ResourceDirectory, "Assets", "AppIcon.ico"));
        MainWindow.AppWindow.Resize(new SizeInt32(1040, 760));
        var page = new SetupPage();
        Grid.SetRow(page, 1);
        root.Children.Add(page);
        MainWindow.AppWindow.Closing += (_, args) => args.Cancel = page.InstallationInProgress;
        MainWindow.Closed += (_, _) => BundledInstaller.Cleanup();
        if (arguments.Contains("--verify-installer-navigation")) {
            page.Loaded += async (_, _) => {
                try {
                    await page.VerifyNavigationAsync();
                    LogStage("Navigation verification passed");
                } catch (Exception error) {
                    Environment.ExitCode = 1;
                    LogFailure(error);
                    LogStage("Navigation verification failed: " + error.Message);
                } finally {
                    MainWindow.Close();
                }
            };
        }
        MainWindow.Activate();
        LogStage("Wizard activated");
    }

    internal static void LogFailure(Exception error)
    {
        try {
            var root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "logs");
            Directory.CreateDirectory(root);
            File.WriteAllText(Path.Combine(root, "setup-crash.log"), $"{DateTimeOffset.Now:O}\n{error}");
        } catch { }
    }

    internal static void LogStage(string stage)
    {
        try {
            var root = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "logs");
            Directory.CreateDirectory(root);
            File.AppendAllText(Path.Combine(root, "setup-startup.log"), $"{DateTimeOffset.Now:O} {stage}\n");
        } catch { }
    }
}
