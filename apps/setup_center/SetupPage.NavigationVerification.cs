using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;

namespace OwO_Settings;

public sealed partial class SetupPage
{
    // Read-only regression mode: never advance from Options into installation,
    // never extract/register payloads. Any simulated UI checkbox consent is
    // reset immediately, without invoking installation while consent is set.
    internal async Task VerifyNavigationAsync()
    {
        if (_previewStage is not null)
            throw new InvalidOperationException("Navigation verification cannot use preview prerequisites.");
        await Task.Delay(500);
        for (var round = 0; round < 4; round++) {
            if (_step != SetupStep.Welcome)
                throw new InvalidOperationException("Expected welcome page before Next.");
            await AdvanceAsync();
            await Task.Delay(500);
            UpdateLayout();
            if (_step != SetupStep.Options || _installDirectory.ActualWidth <= 0 || _checking || _installing)
                throw new InvalidOperationException("Options page did not finish loading.");
            if (_consent.IsChecked == true || _primary.IsEnabled)
                throw new InvalidOperationException("Installation must remain blocked without consent.");
            if (_bundledPlugins.IsChecked != true || _bundledPlugins.ActualWidth <= 0)
                throw new InvalidOperationException("Bundled plugin option must render and default to enabled.");
            if (round == 0) {
                if (_browseDirectory.ActualWidth <= 0 || VisualText(this).Any(text =>
                    text.StartsWith("浏览时会在所选文件夹下创建", StringComparison.Ordinal) ||
                    text.StartsWith("安装到当前用户账户；注册输入法组件时", StringComparison.Ordinal)))
                    throw new InvalidOperationException("Compact options must retain browsing and omit the two redundant help blocks.");
                _consent.IsChecked = false;
                if (_primary.IsEnabled || _blockedPrimaryFeedback.Visibility != Microsoft.UI.Xaml.Visibility.Visible)
                    throw new InvalidOperationException("Blocked installation must retain a separate feedback hit target.");
                var bounds = _blockedPrimaryFeedback.TransformToVisual(null).TransformBounds(
                    new Windows.Foundation.Rect(0, 0, _blockedPrimaryFeedback.ActualWidth, _blockedPrimaryFeedback.ActualHeight));
                var center = new Windows.Foundation.Point(bounds.X + bounds.Width / 2, bounds.Y + bounds.Height / 2);
                if (bounds.Width <= 0 || bounds.Height <= 0 ||
                    !Microsoft.UI.Xaml.Media.VisualTreeHelper.FindElementsInHostCoordinates(center, this)
                        .Any(element => ReferenceEquals(element, _blockedPrimaryFeedback)))
                    throw new InvalidOperationException("Grey installation feedback must remain pointer hit-testable.");
                var peer = new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(_blockedPrimaryFeedback);
                var invoke = (Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)peer.GetPattern(
                    Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke);
                invoke.Invoke();
                await Task.Delay(60);
                if (!_installConsentFeedbackActive || _installConsentWave.Opacity != 1)
                    throw new InvalidOperationException("Clicking the grey installation action did not start feedback.");
                await Task.Delay(400);
                if (_installConsentWave.Opacity != 1 || _installConsentWave.Data is null ||
                    _installConsentTranslation.X != 0 || _installConsentFeedbackActive ||
                    _installing || _step != SetupStep.Options || _consent.IsChecked != false ||
                    _bundledPlugins.IsChecked != true)
                    throw new InvalidOperationException("Source-consent feedback changed installation intent or failed to settle.");
                if (_primary.IsEnabled || _consent.IsChecked == true ||
                    _blockedPrimaryFeedback.Visibility != Microsoft.UI.Xaml.Visibility.Visible)
                    throw new InvalidOperationException("Feedback must not enable installation without consent.");
                _consent.IsChecked = true;
                if (_installConsentWave.Opacity != 0 || _installConsentTranslation.X != 0)
                    throw new InvalidOperationException("Checking source consent must clear its warning.");
                _consent.IsChecked = false;
                _bundledPlugins.IsChecked = false;
                if (_installConsentWave.Opacity != 0)
                    throw new InvalidOperationException("The optional plugin checkbox must not own the source-consent warning.");
                _bundledPlugins.IsChecked = true;
            }
            App.LogStage($"Navigation round {round + 1}: options rendered; package={_packageReady}; language={_languageReady}");
            BackButton_Click(this, new Microsoft.UI.Xaml.RoutedEventArgs());
            await Task.Delay(300);
        }
        ShowProgress("Read-only navigation verification", false);
        ShowBackendLine(new InstallerProcessLine("OWO_SETUP_PROGRESS|45|copy-settings", false));
        if (_progress.Value != 45 || _progressPercentage.Text != "45%" || _progress.IsIndeterminate)
            throw new InvalidOperationException("Backend stage did not update visible progress.");
        ShowBackendLine(new InstallerProcessLine("OWO_SETUP_PROGRESS|3|prepare", false));
        ShowBackendLine(new InstallerProcessLine("OWO_SETUP_PROGRESS|99|complete", true));
        if (_progress.Value != 45 || _progressPercentage.Text != "45%")
            throw new InvalidOperationException("Stale or stderr progress changed the visible stage.");
        await Task.Delay(150);
        var tips = _progressTipsGrid ?? throw new InvalidOperationException("Missing usage tips on progress page.");
        var timer = _progressTipsTimer ?? throw new InvalidOperationException("Missing usage tip timer.");
        if (tips.Children.Count != 4 || tips.ActualWidth <= 0 || !timer.IsRunning ||
            !timer.IsRepeating || timer.Interval != TimeSpan.FromSeconds(6))
            throw new InvalidOperationException("Usage tip carousel did not start on the progress page.");
        VerifyProgressTip(0);
        var window = ((App)Microsoft.UI.Xaml.Application.Current).MainWindow.AppWindow;
        var wideSize = window.Size;
        try {
            window.Resize(new Windows.Graphics.SizeInt32(Math.Min(wideSize.Width, 470), wideSize.Height));
            await Task.Delay(220);
            UpdateLayout();
            var tipsHeight = tips.ActualHeight;
            var detailsTop = DetailsTop();
            if (detailsTop < tips.TransformToVisual(this).TransformPoint(new Windows.Foundation.Point(0, tipsHeight)).Y)
                throw new InvalidOperationException("Usage tips overlap the details action at narrow width.");

            // Run the same UI timer as the installer, only faster in this fixture.
            var complete = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var interval = timer.Interval;
            var ticks = 0;
            void ObserveTick(Microsoft.UI.Dispatching.DispatcherQueueTimer sender, object args)
            {
                try {
                    VerifyProgressTip(++ticks % tips.Children.Count);
                    UpdateLayout();
                    if (Math.Abs(tips.ActualHeight - tipsHeight) > 0.5 || Math.Abs(DetailsTop() - detailsTop) > 0.5 ||
                        _progress.Value != 45 || _installLog.Visibility != Microsoft.UI.Xaml.Visibility.Collapsed)
                        throw new InvalidOperationException("Rotating tips moved the narrow layout or changed progress and details.");
                    if (ticks == tips.Children.Count) { sender.Stop(); complete.TrySetResult(); }
                } catch (Exception error) { sender.Stop(); complete.TrySetException(error); }
            }
            timer.Stop();
            timer.Interval = TimeSpan.FromMilliseconds(120);
            timer.Tick += ObserveTick;
            timer.Start();
            try { await complete.Task.WaitAsync(TimeSpan.FromSeconds(4)); }
            finally {
                timer.Stop();
                timer.Tick -= ObserveTick;
                timer.Interval = interval;
                timer.Start();
            }
        } finally {
            window.Resize(wideSize);
        }
        _detailsToggle.IsChecked = true;
        await Task.Delay(500);
        _detailsToggle.IsChecked = false;
        await Task.Delay(300);
        ShowOptions();
        if (timer.IsRunning) throw new InvalidOperationException("Usage tip timer is still running outside the progress page.");
        await Task.Delay(500);
        ShowProgress("Resume carousel verification", false);
        await Task.Delay(150);
        VerifyProgressTip(0);
        if (!timer.IsRunning) throw new InvalidOperationException("Usage tip timer did not resume on reentering the progress page.");
        ShowOptions();
        if (_installing || _consent.IsChecked == true)
            throw new InvalidOperationException("Read-only navigation verification changed installation state.");

        double DetailsTop() => _detailsToggle.TransformToVisual(this).TransformPoint(default).Y;
        static IEnumerable<string> VisualText(Microsoft.UI.Xaml.DependencyObject node)
        {
            if (node is Microsoft.UI.Xaml.Controls.TextBlock text) yield return text.Text;
            var children = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(node);
            for (var index = 0; index < children; index++)
                foreach (var value in VisualText(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(node, index)))
                    yield return value;
        }
        void VerifyProgressTip(int index)
        {
            if (_progressTipIndex != index || _progressTipPosition?.Text != $"{index + 1} / {tips.Children.Count}")
                throw new InvalidOperationException($"Usage tips did not select position {index + 1}.");
            for (var row = 0; row < tips.Children.Count; row++) {
                var item = (Microsoft.UI.Xaml.Controls.StackPanel)tips.Children[row];
                var view = row == index ? AccessibilityView.Content : AccessibilityView.Raw;
                if (item.Opacity != (row == index ? 1 : 0) || item.Children.Any(line =>
                    AutomationProperties.GetAccessibilityView(line) != view))
                    throw new InvalidOperationException("Only the current usage tip should be visible and accessible.");
            }
        }
    }
}
