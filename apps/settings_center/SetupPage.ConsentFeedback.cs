using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using WavePath = Microsoft.UI.Xaml.Shapes.Path;

namespace OwO_Settings;

public sealed partial class SetupPage
{
    private readonly TranslateTransform _installConsentTranslation = new();
    private readonly WavePath _installConsentWave = new() {
        Height = 5, Opacity = 0, StrokeThickness = 1.25,
        Stroke = new SolidColorBrush(Windows.UI.Color.FromArgb(255, 196, 43, 28)),
        IsHitTestVisible = false,
    };
    private Storyboard? _installConsentShake;
    private bool _installConsentFeedbackActive;
    private bool _installConsentFeedbackUnloaded;

    private void InitializeInstallConsentFeedback()
    {
        var label = (TextBlock)_consent.Content;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_consent, "InstallSourceConsent");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(_consent, label.Text);
        var content = new Grid();
        content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        // Reserve the underline space: showing it must not move other options.
        content.RowDefinitions.Add(new RowDefinition { Height = new GridLength(6) });
        content.Children.Add(label);
        Grid.SetRow(_installConsentWave, 1);
        content.Children.Add(_installConsentWave);
        _consent.Content = content;
        _consent.RenderTransform = _installConsentTranslation;
        _consent.Checked += (_, _) => ClearInstallConsentFeedback();
        label.SizeChanged += (_, e) => UpdateInstallConsentWave(e.NewSize.Width);
        Loaded += (_, _) => _installConsentFeedbackUnloaded = false;
        Unloaded += (_, _) => {
            _installConsentFeedbackUnloaded = true;
            ClearInstallConsentFeedback();
        };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_installConsentWave, "InstallConsentWarningWave");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(_installConsentWave,
            Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
    }

    private void UpdateInstallConsentWave(double width)
    {
        if (!double.IsFinite(width) || width <= 0) return;
        var figure = new PathFigure { StartPoint = new Windows.Foundation.Point(0, 2.5), IsClosed = false };
        for (double x = 0; x < width; x += 3) {
            var end = Math.Min(width, x + 3);
            figure.Segments.Add(new QuadraticBezierSegment {
                Point1 = new Windows.Foundation.Point((x + end) / 2, ((int)(x / 3) % 2 == 0) ? 0 : 5),
                Point2 = new Windows.Foundation.Point(end, 2.5),
            });
        }
        var geometry = new PathGeometry();
        geometry.Figures.Add(figure);
        _installConsentWave.Data = geometry;
    }

    private void ClearInstallConsentFeedback()
    {
        _installConsentShake?.Stop();
        _installConsentTranslation.X = 0;
        _installConsentWave.Opacity = 0;
    }

    private async Task ShowInstallConsentFeedbackAsync()
    {
        if (_installConsentFeedbackActive) return;
        _installConsentFeedbackActive = true;
        UpdateInstallAvailability();
        _installConsentWave.Opacity = 1;
        var storyboard = new Storyboard();
        _installConsentShake = storyboard;
        try {
            if (new Windows.UI.ViewManagement.UISettings().AnimationsEnabled) {
                var animation = new DoubleAnimationUsingKeyFrames { EnableDependentAnimation = true };
                foreach (var (milliseconds, offset) in new[] {
                    (0, 0), (35, -6), (75, 6), (115, -4), (155, 4), (195, -2), (235, 2), (300, 0)
                }) animation.KeyFrames.Add(new LinearDoubleKeyFrame {
                    KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromMilliseconds(milliseconds)), Value = offset,
                });
                Storyboard.SetTarget(animation, _installConsentTranslation);
                Storyboard.SetTargetProperty(animation, "X");
                storyboard.Children.Add(animation);
                var completed = new TaskCompletionSource();
                storyboard.Completed += (_, _) => completed.TrySetResult();
                storyboard.Begin();
                await Task.WhenAny(completed.Task, Task.Delay(350));
            } else {
                // Respect reduced-motion settings while preserving visible feedback.
                await Task.Delay(300);
            }
        } finally {
            storyboard.Stop();
            _installConsentTranslation.X = 0;
            _installConsentShake = null;
            _installConsentFeedbackActive = false;
            if (!_installConsentFeedbackUnloaded) UpdateInstallAvailability();
        }
    }
}
