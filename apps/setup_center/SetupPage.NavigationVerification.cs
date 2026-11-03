namespace OwO_Settings;

public sealed partial class SetupPage
{
    // Read-only regression mode: never advance from Options into installation,
    // never authorize trust, and never extract/register the bundled payload.
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
                _bundledPlugins.IsChecked = false;
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
                if (!_bundledPluginFeedbackActive || _bundledPluginWave.Opacity != 1)
                    throw new InvalidOperationException("Clicking the grey installation action did not start feedback.");
                await Task.Delay(400);
                if (_bundledPluginWave.Opacity != 1 || _bundledPluginWave.Data is null ||
                    _bundledPluginTranslation.X != 0 || _bundledPluginFeedbackActive ||
                    _installing || _step != SetupStep.Options || _bundledPlugins.IsChecked != false)
                    throw new InvalidOperationException("Optional-plugin feedback changed installation intent or failed to settle.");
                if (_primary.IsEnabled || _consent.IsChecked == true ||
                    _blockedPrimaryFeedback.Visibility != Microsoft.UI.Xaml.Visibility.Visible)
                    throw new InvalidOperationException("Feedback must not enable installation without consent.");
                _bundledPlugins.IsChecked = true;
                if (_bundledPluginWave.Opacity != 0 || _bundledPluginTranslation.X != 0)
                    throw new InvalidOperationException("Checking the plugin option must clear its warning.");
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
        _detailsToggle.IsChecked = true;
        await Task.Delay(500);
        _detailsToggle.IsChecked = false;
        await Task.Delay(300);
        ShowOptions();
        await Task.Delay(500);
        if (_installing || _consent.IsChecked == true)
            throw new InvalidOperationException("Read-only navigation verification changed installation state.");
    }
}
