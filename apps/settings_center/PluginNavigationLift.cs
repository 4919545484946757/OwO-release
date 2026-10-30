using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml.Hosting;
using System.Numerics;
using System.Runtime.InteropServices;
using Windows.Foundation;
using Windows.UI.ViewManagement;

namespace OwO_Settings;

// Lift the existing presenter, not a differently styled copy. The logical
// NavigationViewItem stays in its original tree with a fixed-size slot.
internal sealed class PluginNavigationLift
{
    private readonly NavigationViewItem _item;
    private readonly NavigationView _navigation;
    private readonly TextBlock _label;
    private readonly Action _refreshLayout;
    private readonly Popup _popup = new() { IsLightDismissEnabled = false, ShouldConstrainToRootBounds = true };
    private readonly Border _surface;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer _closeTimer;
    private InsetClip? _clip;
    private CompositionScopedBatch? _animationBatch;
    private int _animationVersion;
    private FrameworkElement? _presenter;
    private Grid? _parent;
    private int _index;
    private double _slotWidth, _slotHeight, _presenterWidth, _presenterHeight;
    private string _savedHelpText = "";
    private double _collapsedWidth, _expandedWidth;
    private bool _closing;
    private bool _suppressUntilExit, _moving, _pressed;
    private long _outsideSince;
    private long _paneToken;
    private XamlRoot? _root;
    private Size _rootSize;
    private double _rootScale;
    [StructLayout(LayoutKind.Sequential)]
    private struct MousePoint { public int X; public int Y; }
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetCursorPos(out MousePoint point);
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ScreenToClient(IntPtr window, ref MousePoint point);

    internal PluginNavigationLift(NavigationViewItem item, NavigationView navigation, Action refreshLayout)
    {
        _item = item; _navigation = navigation;
        _refreshLayout = refreshLayout;
        var name = item.Content?.ToString() ?? "";
        _label = new TextBlock { Text = name, TextWrapping = TextWrapping.NoWrap, TextTrimming = TextTrimming.CharacterEllipsis };
        item.Content = _label; AutomationProperties.SetName(item, name);
        _surface = (Border)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
            <Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                    CornerRadius="4" Background="{ThemeResource SolidBackgroundFillColorBaseBrush}" />
            """);
        AutomationProperties.SetAutomationId(_surface, "PluginNavigationExpanded");
        AutomationProperties.SetName(_surface, name); _popup.Child = _surface;
        _closeTimer = item.DispatcherQueue.CreateTimer();
        _closeTimer.Interval = TimeSpan.FromMilliseconds(40); _closeTimer.IsRepeating = true;
        _closeTimer.Tick += (_, _) => {
            if (!_popup.IsOpen) { _closeTimer.Stop(); return; }
            if (MouseInsideSurface()) { _outsideSince = 0; return; }
            var now = Environment.TickCount64;
            if (_outsideSince == 0) _outsideSince = now;
            else if (now - _outsideSince >= 120) Close("鼠标已离开", animate: true);
        };
        item.PointerEntered += (_, _) => { ResumeExpansion(); if (!_suppressUntilExit) Show(); };
        item.PointerExited += (_, _) => { if (!_moving) _suppressUntilExit = false; };
        _surface.PointerEntered += (_, _) => { ResumeExpansion(); SetState(false); };
        _surface.PointerPressed += (_, args) => {
            if (!args.GetCurrentPoint(_surface).Properties.IsLeftButtonPressed) return;
            _pressed = true; _surface.CapturePointer(args.Pointer); SetState(true); args.Handled = true;
        };
        _surface.PointerReleased += (_, args) => {
            var point = args.GetCurrentPoint(_surface).Position;
            var select = _pressed && point.X >= 0 && point.Y >= 0 && point.X < _surface.ActualWidth && point.Y < _surface.ActualHeight;
            _pressed = false; _surface.ReleasePointerCapture(args.Pointer);
            if (select) { _suppressUntilExit = true; Close(); navigation.SelectedItem = item; }
            else SetState(false);
            args.Handled = true;
        };
        _surface.PointerCanceled += (_, _) => { _pressed = false; Close(); };
        item.Loaded += OnLoaded;
        item.Unloaded += (_, _) => {
            Close(); navigation.SizeChanged -= OnNavigationSizeChanged;
            navigation.SelectionChanged -= OnSelectionChanged;
            navigation.UnregisterPropertyChangedCallback(NavigationView.OpenPaneLengthProperty, _paneToken);
            if (_root is not null) _root.Changed -= OnRootChanged;
            _root = null;
        };
        item.RegisterPropertyChangedCallback(UIElement.VisibilityProperty, (_, _) => { if (item.Visibility != Visibility.Visible) Close(); });
        item.SizeChanged += (_, args) => {
            if (!_moving && _presenter is not null &&
                (Math.Abs(args.NewSize.Width - _item.Width) > 0.5 ||
                 Math.Abs(args.NewSize.Height - _item.Height) > 0.5)) Close("列表项尺寸变化");
        };
        UpdateLimit();
    }
    private void OnLoaded(object sender, RoutedEventArgs args)
    {
        _navigation.SizeChanged += OnNavigationSizeChanged;
        _navigation.SelectionChanged += OnSelectionChanged;
        _paneToken = _navigation.RegisterPropertyChangedCallback(NavigationView.OpenPaneLengthProperty, (_, _) => { Close(); UpdateLimit(); });
        _root = _item.XamlRoot;
        if (_root is not null) { _rootSize = _root.Size; _rootScale = _root.RasterizationScale; _root.Changed += OnRootChanged; }
        UpdateLimit();
    }
    private void OnNavigationSizeChanged(object sender, SizeChangedEventArgs args) { Close(); UpdateLimit(); }
    private void OnSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args) => Close();
    private void OnRootChanged(XamlRoot sender, XamlRootChangedEventArgs args)
    {
        if (!sender.IsHostVisible || sender.Size != _rootSize || sender.RasterizationScale != _rootScale)
            Close("窗口尺寸变化");
        _rootSize = sender.Size; _rootScale = sender.RasterizationScale;
        UpdateLimit(); _refreshLayout();
    }
    private void UpdateLimit()
    {
        if (_presenter is not null) return;
        var paneWidth = _navigation.OpenPaneLength;
        if (_navigation.ActualWidth > 0) paneWidth = Math.Min(paneWidth, _navigation.ActualWidth);
        var width = Math.Max(0, paneWidth - _item.Margin.Left - _item.Margin.Right - 8);
        if (_item.MaxWidth != width) _item.MaxWidth = width;
        var labelWidth = Math.Max(0, width - 60);
        if (_label.MaxWidth != labelWidth) _label.MaxWidth = labelWidth;
    }
    private void Show()
    {
        if (_popup.IsOpen || _moving || _item.XamlRoot?.Content is not FrameworkElement root || !_navigation.IsPaneOpen || _item.Visibility != Visibility.Visible) return;
        _refreshLayout();
        var presenter = FindPresenter(_item);
        if (presenter is null || VisualTreeHelper.GetParent(presenter) is not Grid parent || presenter.ActualWidth <= 0 || presenter.ActualHeight <= 0) return;
        // Measure detached text only; never mutate the live row to measure it.
        var measure = new TextBlock { Text = _label.Text, FontFamily = _label.FontFamily, FontSize = _label.FontSize,
            FontWeight = _label.FontWeight, FontStyle = _label.FontStyle, CharacterSpacing = _label.CharacterSpacing };
        measure.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        var position = presenter.TransformToVisual(root).TransformPoint(new Point());
        var width = presenter.ActualWidth;
        var target = Math.Min(Math.Max(0, root.ActualWidth - position.X - 4), Math.Ceiling(width - _label.ActualWidth + measure.DesiredSize.Width + 8));
        if (target <= width + 1) return;
        _moving = true;
        try {
            _slotWidth = _item.Width; _slotHeight = _item.Height;
            _savedHelpText = AutomationProperties.GetHelpText(_item);
            _presenterWidth = presenter.Width; _presenterHeight = presenter.Height;
            _index = parent.Children.IndexOf(presenter); _parent = parent; _presenter = presenter;
            _item.Width = _item.ActualWidth; _item.Height = _item.ActualHeight;
            _popup.XamlRoot = _item.XamlRoot; _popup.RequestedTheme = _item.ActualTheme;
            _popup.HorizontalOffset = position.X; _popup.VerticalOffset = position.Y;
            _surface.Height = presenter.ActualHeight;
            parent.Children.Remove(presenter); _label.MaxWidth = double.PositiveInfinity;
            presenter.Height = _surface.Height; presenter.Width = target;
            _collapsedWidth = width; _expandedWidth = target;
            _surface.Child = presenter;
            var visual = ElementCompositionPreview.GetElementVisual(_surface);
            _clip = visual.Compositor.CreateInsetClip();
            _clip.RightInset = new UISettings().AnimationsEnabled ? (float)(target - width) : 0;
            visual.Clip = _clip;
            _popup.IsOpen = true; SetState(false);
            AutomationProperties.SetHelpText(_item, "完整名称已展开");
            if (new UISettings().AnimationsEnabled) AnimateClip(0, 160);
            _outsideSince = 0; _closeTimer.Start();
        } catch (Exception error) {
            _moving = false; Close();
            System.Diagnostics.Debug.WriteLine("Plugin navigation expansion: " + error);
            AutomationProperties.SetHelpText(_item, "名称展开失败：" + error.Message);
        }
        finally { _moving = false; }
    }
    private void SetState(bool pressed)
    {
        if (_presenter is Control presenter)
            VisualStateManager.GoToState(presenter, _item.IsSelected ? pressed ? "PressedSelected" : "PointerOverSelected" : pressed ? "Pressed" : "PointerOver", false);
    }
    private bool MouseInsideSurface()
    {
        if (_item.XamlRoot is not { IsHostVisible: true } root ||
            !GetCursorPos(out var point)) return false;
        var window = WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow);
        if (!ScreenToClient(window, ref point)) return false;
        var x = point.X / root.RasterizationScale;
        var y = point.Y / root.RasterizationScale;
        return x >= _popup.HorizontalOffset && y >= _popup.VerticalOffset &&
            x < _popup.HorizontalOffset + _surface.ActualWidth &&
            y < _popup.VerticalOffset + _surface.ActualHeight;
    }
    private void AnimateClip(float to, int milliseconds, Action? completed = null)
    {
        if (_clip is not { } clip) return;
        var version = ++_animationVersion;
        clip.StopAnimation("RightInset");
        var compositor = clip.Compositor;
        var animation = compositor.CreateScalarKeyFrameAnimation();
        animation.Duration = TimeSpan.FromMilliseconds(milliseconds);
        animation.StopBehavior = AnimationStopBehavior.LeaveCurrentValue;
        animation.InsertKeyFrame(1, to, compositor.CreateCubicBezierEasingFunction(
            new Vector2(0.16F, 1), new Vector2(0.3F, 1)));
        var batch = compositor.CreateScopedBatch(CompositionBatchTypes.Animation);
        _animationBatch = batch;
        batch.Completed += (_, _) => {
            if (version != _animationVersion || !ReferenceEquals(_clip, clip)) return;
            clip.RightInset = to; clip.StopAnimation("RightInset");
            _animationBatch = null; completed?.Invoke();
        };
        clip.StartAnimation("RightInset", animation);
        batch.End();
    }
    private void ResumeExpansion()
    {
        if (!_closing || _presenter is not { } presenter) return;
        _closing = false;
        AutomationProperties.SetHelpText(_item, "完整名称已展开");
        AnimateClip(0, 100);
        _outsideSince = 0; _closeTimer.Start();
    }
    private void Close(string? reason = null, bool animate = false)
    {
        if (_moving) return;
        if (animate && _presenter is { } expanded && new UISettings().AnimationsEnabled) {
            if (_closing) return;
            _closing = true; _closeTimer.Stop();
            AutomationProperties.SetHelpText(_item, "完整名称正在收回");
            AnimateClip((float)(_expandedWidth - _collapsedWidth), 80,
                () => { _closing = false; Close(reason); });
            return;
        }
        _moving = true;
        try {
            _closing = false;
            _closeTimer.Stop(); ++_animationVersion;
            _clip?.StopAnimation("RightInset");
            ElementCompositionPreview.GetElementVisual(_surface).Clip = null;
            _clip = null; _animationBatch = null; _surface.ReleasePointerCaptures();
            if (_presenter is { } presenter && _parent is { } parent) {
                _surface.Child = null;
                presenter.Width = _presenterWidth; presenter.Height = _presenterHeight;
                if (parent.Children.IndexOf(presenter) < 0)
                    parent.Children.Insert(Math.Min(_index, parent.Children.Count), presenter);
                _item.Width = _slotWidth; _item.Height = _slotHeight;
                AutomationProperties.SetHelpText(_item, reason is null ? _savedHelpText : "完整名称已收回（" + reason + "）");
                if (presenter is Control control) VisualStateManager.GoToState(control, _item.IsSelected ? "Selected" : "Normal", false);
            }
            _presenter = null; _parent = null; _popup.IsOpen = false;
            _pressed = false; _outsideSince = 0; UpdateLimit(); _refreshLayout();
        } finally { _moving = false; }
    }
    private static FrameworkElement? FindPresenter(DependencyObject parent)
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); ++index) {
            var child = VisualTreeHelper.GetChild(parent, index);
            if (child is FrameworkElement { Name: "NavigationViewItemPresenter" } presenter) return presenter;
            if (FindPresenter(child) is { } nested) return nested;
        }
        return null;
    }
}
