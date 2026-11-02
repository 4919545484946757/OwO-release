using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

// One sizing policy for installed and online dictionaries, including DPI and
// narrow navigation-pane viewports. Do not let old desired widths pin the grid.
internal static class DictionaryCardLayout
{
    internal const double PreferredWidth = 210;
    internal const double Height = 228;
    internal static void Arrange(Grid grid) {
        var width = double.IsNaN(grid.Width) ? grid.ActualWidth : grid.Width;
        if (!double.IsFinite(width) || width < 0) return;
        var columns = Math.Max(1, (int)((width + 12) / (PreferredWidth + 12)));
        var cardWidth = Math.Max(0, Math.Min(PreferredWidth, (width - 12 * (columns - 1)) / columns));
        while (grid.ColumnDefinitions.Count > columns) grid.ColumnDefinitions.RemoveAt(grid.ColumnDefinitions.Count - 1);
        while (grid.ColumnDefinitions.Count < columns) grid.ColumnDefinitions.Add(new());
        foreach (var column in grid.ColumnDefinitions)
            if (column.Width.GridUnitType != GridUnitType.Pixel || Math.Abs(column.Width.Value - cardWidth) > 0.01)
                column.Width = new GridLength(cardWidth);
        var rows = (grid.Children.Count + columns - 1) / columns;
        while (grid.RowDefinitions.Count > rows) grid.RowDefinitions.RemoveAt(grid.RowDefinitions.Count - 1);
        while (grid.RowDefinitions.Count < rows) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        for (var index = 0; index < grid.Children.Count; index++) {
            var card = (FrameworkElement)grid.Children[index];
            if (Grid.GetRow(card) != index / columns) Grid.SetRow(card, index / columns);
            if (Grid.GetColumn(card) != index % columns) Grid.SetColumn(card, index % columns);
        }
    }
}
