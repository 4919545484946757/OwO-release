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
