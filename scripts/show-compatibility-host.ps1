param([ValidateSet('Wpf','WinForms')][string]$Framework='Wpf')
# A real local .NET UI fixture, not a mocked TSF TextStore. All framework
# assemblies come from Windows; no NuGet restore, download, or installation.
if($Framework -eq 'Wpf'){
    Add-Type -AssemblyName PresentationFramework
    [xml]$markup=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Title="OwO WPF compatibility host" Width="610" Height="430">
 <StackPanel Margin="16">
  <TextBlock Text="Editable"/><TextBox Height="36" Margin="0,4,0,10"/>
  <TextBlock Text="Password (must reject)"/><PasswordBox Height="36" Margin="0,4,0,10"/>
  <TextBlock Text="Readonly (must reject)"/><TextBox IsReadOnly="True" Height="36" Margin="0,4,0,10"/>
  <TextBlock Text="Disabled (must reject)"/><TextBox IsEnabled="False" Height="36" Margin="0,4,0,10"/>
 </StackPanel>
</Window>
'@
    $reader=[Xml.XmlNodeReader]::new($markup)
    $window=[Windows.Markup.XamlReader]::Load($reader)
    [void]$window.ShowDialog()
}else{
    Add-Type -AssemblyName System.Windows.Forms
    $form=[Windows.Forms.Form]::new();$form.Text='OwO WinForms compatibility host';$form.Width=610;$form.Height=440
    foreach($i in 0..3){
        $label=[Windows.Forms.Label]::new();$label.Text=@('Editable','Password (must reject)','Readonly (must reject)','Disabled (must reject)')[$i]
        $label.SetBounds(16,16+$i*85,400,24);$form.Controls.Add($label)
        $edit=[Windows.Forms.TextBox]::new();$edit.SetBounds(16,42+$i*85,550,30)
        if($i -eq 1){$edit.UseSystemPasswordChar=$true};if($i -eq 2){$edit.ReadOnly=$true};if($i -eq 3){$edit.Enabled=$false}
        $form.Controls.Add($edit)
    }
    [void]$form.ShowDialog();$form.Dispose()
}
