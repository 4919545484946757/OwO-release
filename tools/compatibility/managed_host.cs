using System;
using Forms = System.Windows.Forms;
using Wpf = System.Windows;
using Controls = System.Windows.Controls;
class CompatibilityHost {
    [STAThread] static void Main(string[] args) {
        if(args.Length > 0 && args[0] == "winforms") {
            var form = new Forms.Form { Text = "OwO WinForms compatibility host", Width=610, Height=440 };
            string[] names={"Editable","Password (must reject)","Readonly (must reject)","Disabled (must reject)"};
            for(int i=0;i<4;i++) {
                var label=new Forms.Label {Text=names[i]};label.SetBounds(16,16+i*85,400,24);form.Controls.Add(label);
                var edit=new Forms.TextBox();edit.SetBounds(16,42+i*85,550,30);
                edit.UseSystemPasswordChar=i==1;edit.ReadOnly=i==2;edit.Enabled=i!=3;form.Controls.Add(edit);
            }
            Forms.Application.Run(form);
        } else {
            var panel=new Controls.StackPanel {Margin=new Wpf.Thickness(16)};
            panel.Children.Add(new Controls.TextBlock {Text="Editable"});
            panel.Children.Add(new Controls.TextBox {Height=36,Margin=new Wpf.Thickness(0,4,0,10)});
            panel.Children.Add(new Controls.TextBlock {Text="Password (must reject)"});
            panel.Children.Add(new Controls.PasswordBox {Height=36,Margin=new Wpf.Thickness(0,4,0,10)});
            panel.Children.Add(new Controls.TextBlock {Text="Readonly (must reject)"});
            panel.Children.Add(new Controls.TextBox {IsReadOnly=true,Height=36,Margin=new Wpf.Thickness(0,4,0,10)});
            panel.Children.Add(new Controls.TextBlock {Text="Disabled (must reject)"});
            panel.Children.Add(new Controls.TextBox {IsEnabled=false,Height=36,Margin=new Wpf.Thickness(0,4,0,10)});
            new Wpf.Application().Run(new Wpf.Window {Title="OwO WPF compatibility host", Width=610,Height=430,Content=panel});
        }
    }
}
