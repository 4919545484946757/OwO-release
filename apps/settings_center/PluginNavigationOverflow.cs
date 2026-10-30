using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Windows.Foundation;

namespace OwO_Settings;

// The measured row stays bounded. Only its hover copy can cross the pane edge,
// so pointer movement never feeds a wider desired size back into NavigationView.
internal sealed class PluginNavigationOverflow
{
    private readonly NavigationViewItem _item;
    private readonly NavigationView _navigation;
    private readonly TextBlock _label;
    private readonly Popup _popup = new() { IsLightDismissEnabled = false, ShouldConstrainToRootBounds = true };
    private readonly Button _expanded;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer _closeTimer;
    private bool _overRow, _overPopup, _suppressUntilExit;
    private long _paneToken;
    private XamlRoot? _root;

    internal PluginNavigationOverflow(NavigationViewItem item, NavigationView navigation)
    {
        _item = item; _navigation = navigation;
        var name = item.Content?.ToString() ?? "";
        _label = new TextBlock { Text = name, TextWrapping = TextWrapping.NoWrap,
            TextTrimming = TextTrimming.CharacterEllipsis };
        item.Content = _label;
        AutomationProperties.SetName(item, name);
        _expanded = (Button)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
            <Button xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                    MinWidth="0" Padding="3,0,12,0" CornerRadius="4" IsTabStop="False"
                    HorizontalContentAlignment="Stretch" VerticalContentAlignment="Center"
                    Background="{ThemeResource CardBackgroundFillColorDefaultBrush}"
                    BorderBrush="{ThemeResource CardStrokeColorDefaultBrush}" BorderThickness="1">
                <Grid>
                    <Grid.ColumnDefinitions><ColumnDefinition Width="48"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
                    <FontIcon Glyph="&#xE74C;" FontSize="16" VerticalAlignment="Center" />
                    <TextBlock Grid.Column="1" Margin="4,0,0,0" VerticalAlignment="Center"
                               TextWrapping="NoWrap" TextTrimming="CharacterEllipsis" />
                </Grid>
            </Button>
            """);
        ((TextBlock)((Grid)_expanded.Content).Children[1]).Text = name;
        AutomationProperties.SetName(_expanded, name);
        _popup.Child = _expanded;
        _closeTimer = item.DispatcherQueue.CreateTimer();
        _closeTimer.Interval = TimeSpan.FromMilliseconds(120); _closeTimer.IsRepeating = false;
        _closeTimer.Tick += (_, _) => { if (!_overRow && !_overPopup) Close(); };
        item.PointerEntered += (_, _) => { _overRow = true; _closeTimer.Stop(); if (!_suppressUntilExit) Show(); };
        item.PointerExited += (_, _) => { _overRow = false; _suppressUntilExit = false; _closeTimer.Start(); };
        _expanded.PointerEntered += (_, _) => { _overPopup = true; _closeTimer.Stop(); };
        _expanded.PointerExited += (_, _) => { _overPopup = false; _closeTimer.Start(); };
        _expanded.Click += (_, _) => { _suppressUntilExit = true; Close(); navigation.SelectedItem = item; };
        item.Loaded += OnLoaded;
        item.Unloaded += (_, _) => {
            Close(); navigation.SizeChanged -= OnNavigationSizeChanged;
            navigation.SelectionChanged -= OnSelectionChanged;
            navigation.UnregisterPropertyChangedCallback(NavigationView.OpenPaneLengthProperty, _paneToken);
            if (_root is not null) _root.Changed -= OnRootChanged;
            _root = null;
        };
        item.RegisterPropertyChangedCallback(UIElement.VisibilityProperty, (_, _) => {
            if (item.Visibility != Visibility.Visible) Close();
        });
        item.SizeChanged += (_, _) => Close();
        UpdateLimit();
    }

    private void OnLoaded(object sender, RoutedEventArgs args)
    {
        _navigation.SizeChanged += OnNavigationSizeChanged;
        _navigation.SelectionChanged += OnSelectionChanged;
        _paneToken = _navigation.RegisterPropertyChangedCallback(NavigationView.OpenPaneLengthProperty,
            (_, _) => { Close(); UpdateLimit(); });
        _root = _item.XamlRoot;
        if (_root is not null) _root.Changed += OnRootChanged;
        UpdateLimit();
    }
    private void OnNavigationSizeChanged(object sender, SizeChangedEventArgs args) { Close(); UpdateLimit(); }
    private void OnSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args) => Close();
    private void OnRootChanged(XamlRoot sender, XamlRootChangedEventArgs args) => Close();
    private void UpdateLimit()
    {
        var paneWidth = _navigation.OpenPaneLength;
        if (_navigation.ActualWidth > 0) paneWidth = Math.Min(paneWidth, _navigation.ActualWidth);
        var width = Math.Max(0, paneWidth - _item.Margin.Left - _item.Margin.Right - 8);
        if (_item.MaxWidth != width) _item.MaxWidth = width;
        var labelWidth = Math.Max(0, width - 60);
        if (_label.MaxWidth != labelWidth) _label.MaxWidth = labelWidth;
    }
    private void Show()
    {
        if (_popup.IsOpen || _item.XamlRoot?.Content is not FrameworkElement root ||
            !_navigation.IsPaneOpen || _item.Visibility != Visibility.Visible) return;
        // Measure an unconstrained copy, not the clipped label in the live row.
        _expanded.MinWidth = 0;
        _expanded.MaxWidth = double.PositiveInfinity;
        _expanded.Height = Math.Max(32, _item.ActualHeight);
        _expanded.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        if (_expanded.DesiredSize.Width <= _item.ActualWidth) return;
        var position = _item.TransformToVisual(root).TransformPoint(new Point());
        var available = Math.Max(0, root.ActualWidth - position.X - 4);
        if (available <= _item.ActualWidth) return;
        _expanded.MaxWidth = available;
        _expanded.MinWidth = _item.ActualWidth;
        _popup.XamlRoot = _item.XamlRoot;
        _popup.RequestedTheme = _item.ActualTheme;
        _popup.HorizontalOffset = position.X; _popup.VerticalOffset = position.Y;
        _popup.IsOpen = true;
    }
    private void Close()
    {
        _closeTimer.Stop(); _popup.IsOpen = false;
        _overRow = _overPopup = false;
    }
}
