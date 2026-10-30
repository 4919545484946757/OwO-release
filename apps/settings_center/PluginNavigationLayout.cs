using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

// Retain the native navigation item and its accessibility/keyboard behavior.
// WinUI adds depth indentation to this presenter grid, not the item's Padding.
internal sealed class PluginNavigationLayout
{
    private readonly NavigationViewItem _item;
    private FrameworkElement? _contentRoot;
    private Grid? _guideRoot;
    private Border? _guide;
    private long _marginToken;
    private XamlRoot? _root;
    private bool _refreshQueued;

    internal PluginNavigationLayout(NavigationViewItem item)
    {
        _item = item;
        // Indent the entire child row (including hover/selection), placing the
        // guide near the parent icon rather than at the navigation pane edge.
        item.Margin = new Thickness(24, 0, 0, 0);
        item.Loaded += (_, _) => { ObserveRoot(); Refresh(); };
        item.Unloaded += (_, _) => { ObserveRoot(null); Detach(); };
        item.SizeChanged += (_, _) => QueueRefresh();
        item.ActualThemeChanged += (_, _) => QueueRefresh();
        item.LayoutUpdated += (_, _) => { if (item.IsLoaded) { ObserveRoot(); Refresh(); } };
    }

    private void ObserveRoot() => ObserveRoot(_item.XamlRoot);
    private void ObserveRoot(XamlRoot? root)
    {
        if (ReferenceEquals(root, _root)) return;
        if (_root is not null) _root.Changed -= OnRootChanged;
        _root = root;
        if (_root is not null) _root.Changed += OnRootChanged;
    }
    private void OnRootChanged(XamlRoot sender, XamlRootChangedEventArgs args) => QueueRefresh();
    private void QueueRefresh()
    {
        if (_refreshQueued) return;
        _refreshQueued = true;
        if (!_item.DispatcherQueue.TryEnqueue(() => {
            _refreshQueued = false;
            if (_item.IsLoaded) Refresh();
        })) _refreshQueued = false;
    }
    internal void Refresh()
    {
        var root = FindElement(_item, "PresenterContentRootGrid");
        // A name-overflow popup temporarily reparents the native presenter.
        // Keep its callback alive while detached; reapply after it returns.
        if (root is null || !root.IsLoaded) return;
        if (!ReferenceEquals(root, _contentRoot)) {
            Detach();
            _contentRoot = root;
            _marginToken = root.RegisterPropertyChangedCallback(FrameworkElement.MarginProperty,
                (sender, _) => ClearIndentation((FrameworkElement)sender));
        }
        ClearIndentation(root);
        // Draw through each row's full height so adjacent plugin entries form
        // one continuous guide. The native hierarchy hides it when collapsed.
        var guideRoot = FindElement(_item, "NVIRootGrid") as Grid;
        if (ReferenceEquals(guideRoot, _guideRoot) && _guide is not null &&
            guideRoot?.Children.Contains(_guide) == true) return;
        if (_guide is not null) _guideRoot?.Children.Remove(_guide);
        _guide = null;
        _guideRoot = guideRoot;
        if (_guideRoot is not null) {
            _guide = (Border)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
                <Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                        Width="1" Margin="2,0,0,0" HorizontalAlignment="Left"
                        VerticalAlignment="Stretch" IsHitTestVisible="False"
                        Background="{ThemeResource DividerStrokeColorDefaultBrush}" />
                """);
            Grid.SetRowSpan(_guide, Math.Max(1, _guideRoot.RowDefinitions.Count));
            _guide.Tag = "OwO.PluginNavigationGuide";
            _guideRoot.Children.Add(_guide);
        }
    }

    private void Detach()
    {
        _contentRoot?.UnregisterPropertyChangedCallback(FrameworkElement.MarginProperty, _marginToken);
        _contentRoot = null;
        if (_guide is not null) _guideRoot?.Children.Remove(_guide);
        _guide = null;
        _guideRoot = null;
    }

    private static void ClearIndentation(FrameworkElement root)
    {
        var margin = root.Margin;
        if (margin.Left != 0) root.Margin = new Thickness(0, margin.Top, margin.Right, margin.Bottom);
    }

    private static FrameworkElement? FindElement(DependencyObject parent, string name)
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); ++index) {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is FrameworkElement root && root.Name == name) return root;
            if (FindElement(child, name) is { } nested) return nested;
        }
        return null;
    }
}
