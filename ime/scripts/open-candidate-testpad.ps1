# A new, isolated Win32 text-control host for checking TSF deployments.
# Does not replace/restart Notepad, Explorer, Core or any existing user app.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class OwOCandidateTestVisibility {
 [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
}
'@
[System.Windows.Forms.Application]::EnableVisualStyles()
$form = New-Object System.Windows.Forms.Form
$form.Text = 'OwO - Candidate UI test (fresh process)'
$form.ClientSize = New-Object System.Drawing.Size(900, 420)
$form.StartPosition = 'CenterScreen'
$form.BackColor = [System.Drawing.Color]::FromArgb(26, 28, 33)
$form.ForeColor = [System.Drawing.Color]::WhiteSmoke
$form.MinimumSize = New-Object System.Drawing.Size(600, 320)
$layout = New-Object System.Windows.Forms.TableLayoutPanel
$layout.Dock = 'Fill'
$layout.Padding = New-Object System.Windows.Forms.Padding(20)
$layout.ColumnCount = 1
$layout.RowCount = 3
[void]$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', 40)))
[void]$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Percent', 100)))
[void]$layout.RowStyles.Add((New-Object System.Windows.Forms.RowStyle('Absolute', 44)))
$heading = New-Object System.Windows.Forms.Label
$heading.Text = 'Select OwO with Win+Space, then type pinyin here.'
$heading.Dock = 'Fill'
$heading.Font = New-Object System.Drawing.Font('Segoe UI', 12)
$edit = New-Object System.Windows.Forms.TextBox
$edit.Multiline = $true
$edit.Dock = 'Fill'
$edit.ImeMode = 'On'
$edit.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 16)
$edit.BackColor = [System.Drawing.Color]::FromArgb(36, 39, 46)
$edit.ForeColor = [System.Drawing.Color]::WhiteSmoke
$edit.BorderStyle = 'FixedSingle'
$edit.ScrollBars = 'Vertical'
$edit.AcceptsReturn = $true
$footer = New-Object System.Windows.Forms.Label
$footer.Text = 'Test text is not saved. Existing applications may still contain the previous TSF DLL.'
$footer.Dock = 'Fill'
$footer.TextAlign = 'MiddleLeft'
$footer.Font = New-Object System.Drawing.Font('Segoe UI', 10)
$layout.Controls.Add($heading, 0, 0)
$layout.Controls.Add($edit, 0, 1)
$layout.Controls.Add($footer, 0, 2)
$form.Controls.Add($layout)
$form.Add_Shown({
    # A hidden PowerShell console's startup show flag can otherwise hide the
    # first GUI window too. Show only this test form, never a user's app.
    [void][OwOCandidateTestVisibility]::ShowWindow($form.Handle, 1)
    [void]$edit.Focus()
})
try { [System.Windows.Forms.Application]::Run($form) }
finally { $form.Dispose() }
