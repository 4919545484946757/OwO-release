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
    private long _marginToken;

    internal PluginNavigationLayout(NavigationViewItem item)
    {
        _item = item;
        item.Loaded += (_, _) => Attach();
        item.Unloaded += (_, _) => Detach();
        item.SizeChanged += (_, _) => { if (item.IsLoaded) Attach(); };
        item.ActualThemeChanged += (_, _) => item.DispatcherQueue.TryEnqueue(() => {
            if (item.IsLoaded) Attach();
        });
    }

    private void Attach()
    {
        var root = FindContentRoot(_item);
        if (ReferenceEquals(root, _contentRoot)) return;
        Detach();
        _contentRoot = root;
        if (root is null) return; // Safely retain defaults if a future SDK changes the template.
        _marginToken = root.RegisterPropertyChangedCallback(FrameworkElement.MarginProperty,
            (sender, _) => ClearIndentation((FrameworkElement)sender));
        ClearIndentation(root);
    }

    private void Detach()
    {
        _contentRoot?.UnregisterPropertyChangedCallback(FrameworkElement.MarginProperty, _marginToken);
        _contentRoot = null;
    }

    private static void ClearIndentation(FrameworkElement root)
    {
        var margin = root.Margin;
        if (margin.Left != 0) root.Margin = new Thickness(0, margin.Top, margin.Right, margin.Bottom);
    }

    private static FrameworkElement? FindContentRoot(DependencyObject parent)
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); ++index) {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is FrameworkElement { Name: "PresenterContentRootGrid" } root) return root;
            if (FindContentRoot(child) is { } nested) return nested;
        }
        return null;
    }
}
