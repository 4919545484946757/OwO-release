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
        var root = FindElement(_item, "PresenterContentRootGrid");
        if (ReferenceEquals(root, _contentRoot)) return;
        Detach();
        _contentRoot = root;
        if (root is null) return; // Safely retain defaults if a future SDK changes the template.
        _marginToken = root.RegisterPropertyChangedCallback(FrameworkElement.MarginProperty,
            (sender, _) => ClearIndentation((FrameworkElement)sender));
        ClearIndentation(root);
        // Draw through each row's full height so adjacent plugin entries form
        // one continuous guide. The native hierarchy hides it when collapsed.
        _guideRoot = FindElement(_item, "NVIRootGrid") as Grid;
        if (_guideRoot is not null) {
            _guide = (Border)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
                <Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                        Width="1" Margin="2,0,0,0" HorizontalAlignment="Left"
                        VerticalAlignment="Stretch" IsHitTestVisible="False"
                        Background="{ThemeResource DividerStrokeColorDefaultBrush}" />
                """);
            Grid.SetRowSpan(_guide, Math.Max(1, _guideRoot.RowDefinitions.Count));
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
