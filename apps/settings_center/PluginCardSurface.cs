using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml;

namespace OwO_Settings;

internal static class PluginCardSurface
{
    // The normal card fill is translucent. An opaque, theme-aware backing keeps
    // page decoration outside the options without changing the card's own fill.
    internal static Border Wrap(FrameworkElement content, Style backingStyle)
    {
        return new Border { Style = backingStyle, Child = content };
    }
}
