using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
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
    private readonly Border _viewportClipHost = new();
    private readonly RectangleGeometry _viewportClip = new();
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer _closeTimer;
    private InsetClip? _clip;
    private CompositionScopedBatch? _animationBatch;
    private int _animationVersion;
    private FrameworkElement? _presenter;
    private Grid? _parent;
    private ScrollViewer? _scroll;
    private FrameworkElement? _viewport;
    private Point _anchorOffset;
    private double _visibleTop, _visibleBottom;
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
    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetSystemParameter(uint action, uint parameter, out uint value, uint flags);

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
        AutomationProperties.SetName(_surface, name);
        _viewportClipHost.Child = _surface;
        _popup.Child = _viewportClipHost;
        _surface.AddHandler(UIElement.PointerWheelChangedEvent, new PointerEventHandler(OnPointerWheelChanged), true);
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
            var select = _pressed && point.X >= 0 && point.X < _surface.ActualWidth &&
                point.Y >= _visibleTop && point.Y < _visibleBottom;
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
    internal void Show(bool monitorHover = true)
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
            _anchorOffset = presenter.TransformToVisual(_item).TransformPoint(new Point());
            (_scroll, _viewport) = FindScrollViewport(_item, _navigation);
            _slotWidth = _item.Width; _slotHeight = _item.Height;
            _savedHelpText = AutomationProperties.GetHelpText(_item);
            _presenterWidth = presenter.Width; _presenterHeight = presenter.Height;
            _index = parent.Children.IndexOf(presenter); _parent = parent; _presenter = presenter;
            _item.Width = _item.ActualWidth; _item.Height = _item.ActualHeight;
            _popup.XamlRoot = _item.XamlRoot; _popup.RequestedTheme = _item.ActualTheme;
            _popup.HorizontalOffset = position.X; _popup.VerticalOffset = position.Y;
            _surface.Height = presenter.ActualHeight;
            _visibleTop = 0; _visibleBottom = _surface.Height;
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
            if (_scroll is not null) _scroll.ViewChanged += OnScrollViewChanged;
            _item.LayoutUpdated += OnAnchorLayoutUpdated;
            _outsideSince = 0;
            if (monitorHover) _closeTimer.Start();
        } catch (Exception error) {
            _moving = false; Close();
            System.Diagnostics.Debug.WriteLine("Plugin navigation expansion: " + error);
            AutomationProperties.SetHelpText(_item, "名称展开失败：" + error.Message);
        }
        finally { _moving = false; }
        SyncPosition();
    }
    private void OnPointerWheelChanged(object sender, PointerRoutedEventArgs args)
    {
        var properties = args.GetCurrentPoint(_surface).Properties;
        if (_scroll is null || properties.IsHorizontalMouseWheel || properties.MouseWheelDelta == 0) return;
        ScrollByWheelDelta(properties.MouseWheelDelta);
        args.Handled = true;
    }
    internal bool ScrollByWheelDelta(int delta)
    {
        if (_scroll is not { } scroll || delta == 0 || scroll.ScrollableHeight <= 0) return false;
        // Popup input does not bubble to the pane's ScrollViewer. Use the
        // Windows wheel preference, including page scrolling and disabled wheel.
        const uint wheelScrollLines = 0x0068;
        const double wheelDelta = 120.0;
        var lines = GetSystemParameter(wheelScrollLines, 0, out var configured, 0) ? configured : 3U;
        var distance = lines == uint.MaxValue ? scroll.ViewportHeight : lines * 16.0;
        var target = Math.Clamp(scroll.VerticalOffset - delta / wheelDelta * distance, 0, scroll.ScrollableHeight);
        return target != scroll.VerticalOffset && scroll.ChangeView(null, target, null, disableAnimation: true);
    }
    private void OnScrollViewChanged(object? sender, ScrollViewerViewChangedEventArgs args) => SyncPosition();
    private void OnAnchorLayoutUpdated(object? sender, object args) => SyncPosition();
    private void SyncPosition()
    {
        if (_moving || !_popup.IsOpen || _item.XamlRoot?.Content is not FrameworkElement root) return;
        var origin = _item.TransformToVisual(root).TransformPoint(_anchorOffset);
        var top = 0.0;
        var bottom = _surface.Height;
        if (_viewport is { IsLoaded: true, ActualHeight: > 0 } viewport) {
            var viewportY = viewport.TransformToVisual(root).TransformPoint(new Point()).Y;
            top = Math.Max(0, viewportY - origin.Y);
            bottom = Math.Min(bottom, viewportY + viewport.ActualHeight - origin.Y);
            if (top >= bottom) { Close("已滚出列表"); return; }
        }
        if (_popup.HorizontalOffset != origin.X) _popup.HorizontalOffset = origin.X;
        if (_popup.VerticalOffset != origin.Y) _popup.VerticalOffset = origin.Y;
        _visibleTop = top; _visibleBottom = bottom;
        // Clip vertically at the pane viewport without clipping the horizontal
        // name expansion or replacing the compositor's reveal animation.
        if (top > 0 || bottom < _surface.Height) {
            var rect = new Rect(0, top, _expandedWidth, bottom - top);
            if (_viewportClip.Rect != rect) _viewportClip.Rect = rect;
            if (_viewportClipHost.Clip is null) _viewportClipHost.Clip = _viewportClip;
        } else if (_viewportClipHost.Clip is not null) _viewportClipHost.Clip = null;
    }
    internal static (ScrollViewer? Scroll, FrameworkElement? Viewport) FindScrollViewport(
        NavigationViewItem item, NavigationView navigation)
    {
        ScrollContentPresenter? viewport = null;
        for (var parent = VisualTreeHelper.GetParent(item); parent is not null && parent != navigation;
             parent = VisualTreeHelper.GetParent(parent)) {
            if (parent is ScrollContentPresenter content) viewport ??= content;
            if (parent is ScrollViewer scroll) return (scroll, viewport ?? (FrameworkElement)scroll);
        }
        return (null, null);
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
        return x >= _popup.HorizontalOffset && y >= _popup.VerticalOffset + _visibleTop &&
            x < _popup.HorizontalOffset + _surface.ActualWidth &&
            y < _popup.VerticalOffset + _visibleBottom;
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
            if (_scroll is not null) _scroll.ViewChanged -= OnScrollViewChanged;
            _item.LayoutUpdated -= OnAnchorLayoutUpdated;
            _scroll = null; _viewport = null; _viewportClipHost.Clip = null;
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
