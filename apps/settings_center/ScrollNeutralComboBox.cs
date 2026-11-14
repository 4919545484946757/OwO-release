using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace OwO_Settings;

public sealed partial class ScrollNeutralComboBox : ComboBox
{
    protected override void OnPointerWheelChanged(PointerRoutedEventArgs e)
    {
        // A closed selector must not change settings just because the pointer
        // happens to be over it while the user scrolls the settings page.
        if (IsDropDownOpen) base.OnPointerWheelChanged(e);
    }
}
