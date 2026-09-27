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
    public MainWindow()
    {
        InitializeComponent();

        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);

        AppWindow.SetIcon("Assets/AppIcon.ico");

        if (App.InstallPackageRoot is not null) {
            AppWindow.Resize(new SizeInt32(1040, 760));
            AppTitleBar.Title = App.InstallPreviewStage is null
                ? "安装 OwO 输入法" : "安装 OwO 输入法 · 阶段预览";
            try {
                RootFrame.Content = new SetupPage();
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
        }
    }
}
