using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using WavePath = Microsoft.UI.Xaml.Shapes.Path;

namespace OwO_Settings;

public sealed partial class SetupPage
{
    private readonly TranslateTransform _bundledPluginTranslation = new();
    private readonly WavePath _bundledPluginWave = new() {
        Height = 5, Opacity = 0, StrokeThickness = 1.25,
        Stroke = new SolidColorBrush(Windows.UI.Color.FromArgb(255, 196, 43, 28)),
        IsHitTestVisible = false,
    };
    private Storyboard? _bundledPluginShake;
    private bool _bundledPluginFeedbackActive;
    private bool _bundledPluginFeedbackUnloaded;

    private void InitializeBundledPluginFeedback()
    {
        var label = (TextBlock)_bundledPlugins.Content;
        var content = new Grid();
        content.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        // Reserve the underline space: showing it must not move other options.
        content.RowDefinitions.Add(new RowDefinition { Height = new GridLength(6) });
        content.Children.Add(label);
        Grid.SetRow(_bundledPluginWave, 1);
        content.Children.Add(_bundledPluginWave);
        _bundledPlugins.Content = content;
        _bundledPlugins.RenderTransform = _bundledPluginTranslation;
        _bundledPlugins.Checked += (_, _) => ClearBundledPluginFeedback();
        label.SizeChanged += (_, e) => UpdateBundledPluginWave(e.NewSize.Width);
        Loaded += (_, _) => _bundledPluginFeedbackUnloaded = false;
        Unloaded += (_, _) => {
            _bundledPluginFeedbackUnloaded = true;
            ClearBundledPluginFeedback();
        };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_bundledPluginWave, "BundledPluginWarningWave");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(_bundledPluginWave,
            Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
    }

    private void UpdateBundledPluginWave(double width)
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
        _bundledPluginWave.Data = geometry;
    }

    private void ClearBundledPluginFeedback()
    {
        _bundledPluginShake?.Stop();
        _bundledPluginTranslation.X = 0;
        _bundledPluginWave.Opacity = 0;
    }

    private async Task ShowBundledPluginFeedbackAsync()
    {
        if (_bundledPluginFeedbackActive) return;
        _bundledPluginFeedbackActive = true;
        UpdateInstallAvailability();
        _bundledPluginWave.Opacity = 1;
        var storyboard = new Storyboard();
        _bundledPluginShake = storyboard;
        try {
            if (new Windows.UI.ViewManagement.UISettings().AnimationsEnabled) {
                var animation = new DoubleAnimationUsingKeyFrames { EnableDependentAnimation = true };
                foreach (var (milliseconds, offset) in new[] {
                    (0, 0), (35, -6), (75, 6), (115, -4), (155, 4), (195, -2), (235, 2), (300, 0)
                }) animation.KeyFrames.Add(new LinearDoubleKeyFrame {
                    KeyTime = KeyTime.FromTimeSpan(TimeSpan.FromMilliseconds(milliseconds)), Value = offset,
                });
                Storyboard.SetTarget(animation, _bundledPluginTranslation);
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
            _bundledPluginTranslation.X = 0;
            _bundledPluginShake = null;
            _bundledPluginFeedbackActive = false;
            if (!_bundledPluginFeedbackUnloaded) UpdateInstallAvailability();
        }
    }
}
