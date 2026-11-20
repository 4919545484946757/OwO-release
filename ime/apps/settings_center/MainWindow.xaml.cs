using Microsoft.UI.Xaml;
using Windows.Graphics;

// To learn more about WinUI, the WinUI project structure,
// and more about our project templates, see: http://aka.ms/winui-project-info.

namespace OwO_Settings;

/// <summary>
/// The application window. This hosts a Frame that displays pages. Add your
/// UI and logic to MainPage.xaml / MainPage.xaml.cs instead of here so you
/// can use Page features such as navigation events and the Loaded lifecycle.
/// </summary>
public sealed partial class MainWindow : Window
{
    private bool _closingAfterSave, _checkingSaveOnClose;
    public MainWindow()
    {
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);

        AppWindow.SetIcon(System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico"));

        if (App.InstallPackageRoot is not null) {
            AppWindow.Resize(new SizeInt32(1040, 760));
            AppTitleBar.Title = App.InstallPreviewStage is null
                ? "安装 OwO 输入法" : "安装 OwO 输入法 · 阶段预览";
            try {
                var setupPage = new SetupPage();
                RootFrame.Content = setupPage;
                if (Environment.GetCommandLineArgs().Contains("--verify-installer-navigation")) {
                    setupPage.Loaded += async (_, _) => {
                        try {
                            await setupPage.VerifyNavigationAsync();
                            App.LogStage("Navigation verification passed");
                        } catch (Exception error) {
                            Environment.ExitCode = 1;
                            App.LogStage("Navigation verification failed: " + error);
                        } finally { Close(); }
                    };
                }
                AppWindow.Closing += (_, args) => {
                    if (RootFrame.Content is SetupPage page && page.InstallationInProgress)
                        args.Cancel = true;
                };
            } catch (Exception error) {
                RootFrame.Content = new Microsoft.UI.Xaml.Controls.TextBlock {
                    Text = $"安装界面无法启动：{error.Message}",
                    TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
                    Margin = new Microsoft.UI.Xaml.Thickness(32)
                };
            }
        } else {
            AppWindow.Resize(new SizeInt32(1080, 760));
            RootFrame.Navigate(typeof(MainPage));
            AppWindow.Closing += async (_, args) => {
                if (_closingAfterSave || RootFrame.Content is not MainPage page) return;
                args.Cancel = true;
                if (page.ConfigurationTransferInProgress) return;
                if (_checkingSaveOnClose) return;
                _checkingSaveOnClose = true;
                page.IsEnabled = false;
                try {
                    if (await page.FlushAutoSaveAsync()) {
                        _closingAfterSave = true;
                        Close();
                    } else {
                        var dialog = new Microsoft.UI.Xaml.Controls.ContentDialog {
                            XamlRoot = page.XamlRoot,
                            Title = "部分设置尚未保存",
                            Content = "自动保存失败。返回修正配置，或关闭并放弃尚未保存的修改。",
                            PrimaryButtonText = "返回设置", SecondaryButtonText = "放弃修改并关闭",
                            CloseButtonText = "取消", DefaultButton = Microsoft.UI.Xaml.Controls.ContentDialogButton.Primary
                        };
                        if (await dialog.ShowAsync() == Microsoft.UI.Xaml.Controls.ContentDialogResult.Secondary) {
                            _closingAfterSave = true;
                            Close();
                        }
                    }
                } finally {
                    _checkingSaveOnClose = false;
                    if (!_closingAfterSave) page.IsEnabled = true;
                }
            };
        }
    }
}
