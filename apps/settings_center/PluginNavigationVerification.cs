using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using System.Text.Json;
using Windows.Graphics;

namespace OwO_Settings;

public sealed partial class MainPage
{
    // Opt-in regression mode in a fixture process. Never changes display
    // resolution, system DPI, plugin grants or user configuration.
    internal async Task VerifyPluginNavigationAsync()
    {
        var deadline = DateTime.UtcNow.AddSeconds(20);
        while ((_settingsBusy || _pluginPagesGroup?.MenuItems.Count < 2 || _pluginPagesGroup is null) && DateTime.UtcNow < deadline)
            await Task.Delay(100);
        if (_pluginPagesGroup is null || _pluginPagesGroup.MenuItems.Count < 2)
            throw new InvalidOperationException("Seed at least two plugins in an isolated store.");
        var items = _pluginPagesGroup.MenuItems.OfType<NavigationViewItem>().ToArray();
        var margins = items.Select(item => item.Margin.Left).ToArray();
        var offsets = new Dictionary<NavigationViewItem, double>();
        var results = new List<object>();
        SettingsNavigation.IsHitTestVisible = false; // Physical cursor must not launch a concurrent hover lift.
        foreach (var width in new[] { 1300, 920, 560, 1100, 820, 1300 }) {
            ((App)Application.Current).MainWindow.AppWindow.Resize(new SizeInt32(width, 980));
            RequestedTheme = width == 920 || width == 820 ? ElementTheme.Dark : ElementTheme.Light;
            SettingsNavigation.PaneDisplayMode = NavigationViewPaneDisplayMode.Auto;
            await Task.Delay(180);
            // Validate the visible expanded pane after each responsive mode
            // transition, rather than a compact pane's unrealized template.
            SettingsNavigation.PaneDisplayMode = NavigationViewPaneDisplayMode.Left;
            SettingsNavigation.IsPaneOpen = true; _pluginPagesGroup.IsExpanded = true;
            await Task.Delay(300); UpdateLayout();
            foreach (var item in items) {
                var presenter = NavigationPart(item, "NavigationViewItemPresenter")
                    ?? throw new InvalidOperationException("Native presenter not realized.");
                if (VisualTreeHelper.GetParent(presenter) is not Grid parent)
                    throw new InvalidOperationException($"Native presenter parent changed at width {width}: {VisualTreeHelper.GetParent(presenter)?.GetType().FullName ?? "null"}");
                var slotWidth = item.Width; var slotHeight = item.Height;
                var index = parent.Children.IndexOf(presenter);
                item.Width = item.ActualWidth; item.Height = item.ActualHeight;
                var popup = new Popup { XamlRoot = item.XamlRoot };
                try {
                    parent.Children.Remove(presenter); popup.Child = presenter; popup.IsOpen = true;
                    await Task.Delay(60);
                } finally {
                    popup.Child = null; parent.Children.Insert(Math.Min(index, parent.Children.Count), presenter);
                    popup.IsOpen = false; item.Width = slotWidth; item.Height = slotHeight;
                }
            }
            await Task.Delay(150); UpdateLayout();
            for (var index = 0; index < items.Length; ++index) {
                var item = items[index];
                var root = NavigationPart(item, "PresenterContentRootGrid")
                    ?? throw new InvalidOperationException("Content root missing after reparenting.");
                var guideRoot = NavigationPart(item, "NVIRootGrid") as Grid;
                var guides = guideRoot?.Children.OfType<Border>().Count(border => Equals(border.Tag, "OwO.PluginNavigationGuide")) ?? 0;
                if (root.Margin.Left != 0 || item.Margin.Left != margins[index] || guides != 1)
                    throw new InvalidOperationException($"Unstable native indent/guide at width {width}: item={index}, margin={root.Margin.Left}, guides={guides}");
                var label = (TextBlock)item.Content;
                var offset = label.TransformToVisual(item).TransformPoint(default).X;
                if (offsets.TryGetValue(item, out var previous) && Math.Abs(offset - previous) > 1)
                    throw new InvalidOperationException($"Text indentation changed at width {width}: {previous} -> {offset}");
                offsets[item] = offset;
                results.Add(new { width, item = index, nativeIndent = root.Margin.Left, rowIndent = item.Margin.Left,
                    textOffset = offset, guides, scale = XamlRoot.RasterizationScale });
            }
            _pluginPagesGroup.IsExpanded = false; await Task.Delay(80);
            _pluginPagesGroup.IsExpanded = true; await Task.Delay(80);
        }
        var report = Path.Combine(Path.GetDirectoryName(_client.ConfigPath)!, "plugin-navigation-report.json");
        await File.WriteAllTextAsync(report, JsonSerializer.Serialize(results, new JsonSerializerOptions { WriteIndented = true }));
        await VerifyPluginNavigationScrollAsync(items[0]);
    }

    private async Task VerifyPluginNavigationScrollAsync(NavigationViewItem existingItem)
    {
        var (scroll, viewport) = PluginNavigationLift.FindScrollViewport(existingItem, SettingsNavigation);
        if (scroll is null || viewport is null) throw new InvalidOperationException("No plugin pane scroll viewport.");
        var group = _pluginPagesGroup!;
        var item = new NavigationViewItem { Content = new string('W', 80), Icon = new FontIcon { Glyph = "\uE74C" } };
        var layout = new PluginNavigationLayout(item);
        var lift = new PluginNavigationLift(item, SettingsNavigation, layout.Refresh);
        group.MenuItems.Insert(0, item);
        for (var index = 0; index < 18; ++index)
            group.MenuItems.Add(new NavigationViewItem { Content = $"Scroll fixture {index}" });
        ((App)Application.Current).MainWindow.AppWindow.Resize(new SizeInt32(1000, 640));
        await WaitForAsync(() => scroll.ScrollableHeight > 300 && item.IsLoaded, "scrollable plugin pane");
        var viewportY = item.TransformToVisual(viewport).TransformPoint(default).Y;
        var target = Math.Clamp(scroll.VerticalOffset + viewportY - 72, 0, scroll.ScrollableHeight);
        await ScrollToAsync(target);
        lift.Show(monitorHover: false);
        var popup = VisualTreeHelper.GetOpenPopupsForXamlRoot(XamlRoot).Single(p =>
            p.Child is Border { Child: Border surface } && AutomationProperties.GetName(surface) == new string('W', 80));
        var host = (Border)popup.Child;
        var y = popup.VerticalOffset;
        var original = scroll.VerticalOffset;
        if (!lift.ScrollByWheelDelta(-15)) throw new InvalidOperationException("Expanded row did not forward wheel input.");
        await WaitForAsync(() => scroll.VerticalOffset > original, "wheel scroll");
        await WaitForAsync(() => popup.IsOpen && Math.Abs(popup.VerticalOffset - (y - (scroll.VerticalOffset - original))) < 1,
            "expanded row tracking the wheel scroll");
        if (!lift.ScrollByWheelDelta(15)) throw new InvalidOperationException("Expanded row did not forward reverse wheel input.");
        await WaitForAsync(() => popup.IsOpen && Math.Abs(scroll.VerticalOffset - original) < 1 &&
            Math.Abs(popup.VerticalOffset - y) < 1, "expanded row tracking the reverse wheel scroll");

        var root = (FrameworkElement)XamlRoot.Content;
        var top = viewport.TransformToVisual(root).TransformPoint(default).Y;
        await ScrollToAsync(scroll.VerticalOffset + popup.VerticalOffset - top + 8);
        await WaitForAsync(() => popup.IsOpen && host.Clip is RectangleGeometry clip && Math.Abs(clip.Rect.Y - 8) < 1,
            "expanded row clipped by the viewport");
        await ScrollToAsync(scroll.VerticalOffset + item.ActualHeight + 8);
        await WaitForAsync(() => !popup.IsOpen && NavigationPart(item, "NavigationViewItemPresenter") is not null,
            "paint and hit target removed after the row scrolls out");

        var report = Path.Combine(Path.GetDirectoryName(_client.ConfigPath)!, "plugin-navigation-scroll-report.json");
        await File.WriteAllTextAsync(report, JsonSerializer.Serialize(new[] { "wheel forwarding", "anchor tracking", "viewport clipping", "closes outside viewport" }));

        async Task ScrollToAsync(double offset)
        {
            scroll.ChangeView(null, offset, null, disableAnimation: true);
            await WaitForAsync(() => Math.Abs(scroll.VerticalOffset - offset) < 1, $"scroll offset {offset}");
        }
        static async Task WaitForAsync(Func<bool> ready, string description)
        {
            var deadline = DateTime.UtcNow.AddSeconds(3);
            while (!ready()) {
                if (DateTime.UtcNow >= deadline) throw new InvalidOperationException($"Timed out: {description}");
                await Task.Delay(25);
            }
        }
    }

    private static FrameworkElement? NavigationPart(DependencyObject parent, string name)
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); ++index) {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is FrameworkElement element && element.Name == name) return element;
            if (NavigationPart(child, name) is { } nested) return nested;
        }
        return null;
    }
}
