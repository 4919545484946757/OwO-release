using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace OwO_Settings;

[Bindable]
public sealed class SettingsSearchSuggestion
{
    internal SettingsSearchSuggestion(object value, SearchTerm term, Brush? foreground)
    {
        Value = value;
        Title = term.Title;
        NamespacePrefix = SettingsSearchQuery.Namespace(term.Area);
        NamespaceForeground = foreground;
    }
    internal object Value { get; }
    public string Title { get; }
    public string NamespacePrefix { get; }
    public Brush? NamespaceForeground { get; }
    public string SpokenLabel => $"{NamespacePrefix} · {Title}";
}

public sealed partial class MainPage
{
    private TextBox? _searchEditor;
    private bool _searchComposing;
    private int _compositionRevision;

    private void SettingsSearchBox_Loaded(object sender, RoutedEventArgs args)
    {
        SettingsSearchBox.ApplyTemplate();
        var editor = FindEditor(SettingsSearchBox);
        if (editor is null || ReferenceEquals(editor, _searchEditor)) return;
        if (_searchEditor is not null) {
            _searchEditor.TextCompositionStarted -= SearchCompositionStarted;
            _searchEditor.TextCompositionEnded -= SearchCompositionEnded;
            _searchEditor.LostFocus -= SearchEditorLostFocus;
        }
        _searchEditor = editor;
        _searchComposing = false;
        // Set the scope on the actual editable TSF client, not on the
        // AutoSuggestBox shell or its separate results popup.
        var scope = new InputScope();
        scope.Names.Add(new InputScopeName { NameValue = InputScopeNameValue.Default });
        editor.InputScope = scope;
        editor.TextCompositionStarted += SearchCompositionStarted;
        editor.TextCompositionEnded += SearchCompositionEnded;
        editor.LostFocus += SearchEditorLostFocus;
    }

    private static TextBox? FindEditor(DependencyObject node)
    {
        if (node is TextBox text) return text;
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(node); ++i)
            if (FindEditor(VisualTreeHelper.GetChild(node, i)) is { } result) return result;
        return null;
    }

    private void SearchCompositionStarted(TextBox sender, TextCompositionStartedEventArgs args)
    {
        SetSearchComposing(true);
    }

    private void SearchCompositionEnded(TextBox sender, TextCompositionEndedEventArgs args)
    {
        var revision = _compositionRevision;
        // Wait for the committed string, and avoid navigating if the same
        // Enter key also raises the AutoSuggestBox's submit event.
        DispatcherQueue.TryEnqueue(() => {
            if (revision != _compositionRevision) return;
            SetSearchComposing(false);
            RefreshSearchSuggestions();
        });
    }

    private void SetSearchComposing(bool composing)
    {
        ++_compositionRevision;
        _searchComposing = composing;
        if (composing) {
            SettingsSearchBox.ItemsSource = null;
            SettingsSearchBox.IsSuggestionListOpen = false;
        }
    }

    private void SearchEditorLostFocus(object sender, RoutedEventArgs args) => SetSearchComposing(false);

    private void SettingsSearchBox_TextChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason == AutoSuggestionBoxTextChangeReason.UserInput && args.CheckCurrent())
            RefreshSearchSuggestions();
    }

    private void SettingsSearchBox_ActualThemeChanged(FrameworkElement sender, object args)
    {
        if (SettingsSearchBox.IsSuggestionListOpen) RefreshSearchSuggestions();
    }

    private void RefreshSearchSuggestions()
    {
        if (_searchComposing || _searchEditor?.FocusState == FocusState.Unfocused) return;
        var matches = SettingsSearchQuery.Find(SettingsSearchItems(), SettingsSearchBox.Text, item => item.Term);
        var dark = SettingsSearchBox.ActualTheme == ElementTheme.Dark;
        var highContrast = new AccessibilitySettings().HighContrast;
        SettingsSearchBox.ItemsSource = matches.Select(item => {
            var term = item.Term;
            return new SettingsSearchSuggestion(item, term, SearchNamespaceBrush(term.Area, dark, highContrast));
        }).ToArray();
        SettingsSearchBox.IsSuggestionListOpen = matches.Count != 0;
    }

    private static Brush SearchNamespaceBrush(SearchArea area, bool dark, bool highContrast)
    {
        if (highContrast) return new SolidColorBrush(new UISettings().GetColorValue(UIColorType.Foreground));
        var (r, g, b) = (area, dark) switch {
            (SearchArea.Page, true) => (164, 205, 255), (SearchArea.Page, false) => (17, 75, 149),
            (SearchArea.Shortcut, true) => (255, 208, 141), (SearchArea.Shortcut, false) => (116, 63, 9),
            (SearchArea.Plugin, true) => (215, 185, 255), (SearchArea.Plugin, false) => (102, 49, 154),
            (_, true) => (144, 225, 202), _ => (16, 104, 81)
        };
        return new SolidColorBrush(Color.FromArgb(255, (byte)r, (byte)g, (byte)b));
    }

    private async void SettingsSearchBox_QuerySubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        if (_searchComposing || string.IsNullOrWhiteSpace(args.QueryText)) return;
        var selected = (args.ChosenSuggestion as SettingsSearchSuggestion)?.Value as SearchItem;
        selected ??= SettingsSearchQuery.Find(SettingsSearchItems(), args.QueryText, item => item.Term).FirstOrDefault();
        if (selected is null) {
            ShowStatus("未找到匹配的设置。可以试试页面名，或用 page:、key:、plugin: 缩小范围。", InfoBarSeverity.Warning);
            return;
        }
        sender.IsSuggestionListOpen = false;
        sender.Text = selected.Title;
        await NavigateToSearchItemAsync(selected);
    }

    internal async Task<IReadOnlyList<string>> VerifySettingsSearchAsync()
    {
        await Task.Delay(220); // Let the actual AutoSuggestBox template attach to its XamlRoot.
        var checks = new List<string>();
        void Check(bool yes, string label) {
            if (!yes) throw new InvalidOperationException(label);
            checks.Add("PASS " + label);
        }
        var editor = _searchEditor ?? throw new InvalidOperationException("Search editor was not attached");
        Check(editor.InputScope?.Names.FirstOrDefault()?.NameValue == InputScopeNameValue.Default &&
            !editor.IsReadOnly && !SettingsSearchBox.UpdateTextOnSelect, "actual WinUI editor has an ordinary text scope");
        var exact = SettingsSearchQuery.Find(SettingsSearchItems(), "词库管理器", item => item.Term);
        Check(exact.FirstOrDefault() is { IsPage: true, Page: "user-lexicon" }, "page name resolves to the page");
        var alias = SettingsSearchQuery.Find(SettingsSearchItems(), "key:切页", item => item.Term);
        Check(alias.Count >= 2 && alias.All(row => row.Term.Area == SearchArea.Shortcut), "shortcut scope and Chinese alias");
        var typo = SettingsSearchQuery.Find(SettingsSearchItems(), "page:dicitonary", item => item.Term);
        Check(typo.FirstOrDefault()?.Page == "user-lexicon", "English alias tolerates transposed letters");
        var grouped = SettingsSearchQuery.Find(SettingsSearchItems(), "模型", item => item.Term);
        var areas = grouped.Select(item => item.Term.Area).ToArray();
        Check(areas.Distinct().Count() > 1 && areas.Select((area, i) => i == 0 || area != areas[i - 1])
                .Count(start => start) == areas.Distinct().Count(), "namespaces stay contiguous");
        editor.Focus(FocusState.Programmatic);
        SettingsSearchBox.Text = "词库管理器";
        RefreshSearchSuggestions();
        Check(SettingsSearchBox.ItemsSource is SettingsSearchSuggestion[] { Length: > 0 } rows &&
              rows[0].NamespacePrefix == "page" && rows[0].NamespaceForeground is not null,
              "live suggestions expose a colored page badge");
        Check(SettingsSearchBox.ItemTemplate.LoadContent() is Grid { Children.Count: 2 } row &&
              row.Children[0] is Border badge && row.Children[1] is TextBlock { TextWrapping: TextWrapping.NoWrap } title &&
              Grid.GetColumn(title) == 1 && Grid.GetRow(title) == Grid.GetRow(badge),
              "search results show only a badge and a single-line title");
        SetSearchComposing(true);
        RefreshSearchSuggestions();
        Check(!SettingsSearchBox.IsSuggestionListOpen && SettingsSearchBox.Text == "词库管理器",
            "suggestions are suppressed without changing composing text");
        SetSearchComposing(false);
        RefreshSearchSuggestions();
        Check(SettingsSearchBox.IsSuggestionListOpen, "suggestions resume after composition");
        await NavigateToSearchItemAsync(exact[0]);
        Check(_currentPage == "user-lexicon", "page result navigates");

        var logo = _settingsThemeLogo ?? throw new InvalidOperationException("Logo button unavailable.");
        var root = (FrameworkElement)XamlRoot.Content;
        var next = logo.ActualTheme == ElementTheme.Dark ? ElementTheme.Light : ElementTheme.Dark;
        var previousLogo = LogoAppearance.Mode;
        var press = (IInvokeProvider)new ButtonAutomationPeer(logo).GetPattern(PatternInterface.Invoke);
        press.Invoke();
        await Task.Delay(180);
        Check(logo.Visibility == Visibility.Visible && root.RequestedTheme == next && logo.ActualTheme == next &&
              SettingsUiPreferences.Load(UiPreferencesPath).SettingsTheme == _settingsThemeMode && LogoAppearance.Mode == previousLogo,
              "clickable Logo toggles and persists the window theme without changing the logo preference");
        _settingsThemeSelector.SelectedIndex = 0;
        Check(root.RequestedTheme == ElementTheme.Default && SettingsUiPreferences.Load(UiPreferencesPath).SettingsTheme == "system",
              "general settings restore the system theme");

        var digits = SettingsSearchQuery.Find(SettingsSearchItems(), "key:小键盘", item => item.Term);
        Check(digits.FirstOrDefault()?.Target == CandidateNumberKeysCard, "candidate number area is searchable in shortcuts");
        ShowPage("shortcuts");
        await Task.Delay(140);
        Check(CandidateNumberKeys.SelectedIndex == 0 && CandidateNumberKeys.IsEnabled, "candidate numbering defaults to the top row");
        CandidateNumberKeys.SelectedIndex = 1;
        await _client.SaveAsync(CaptureSettings());
        Check((await _client.LoadAsync()).CandidateSelectionNumpad, "selecting numpad persists through the native configuration bridge");
        return checks;
    }
}
