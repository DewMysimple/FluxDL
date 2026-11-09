param([Parameter(Mandatory=$true)][int]$ProcessId, [Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$allowedRoot = Join-Path $workspace 'artifacts'
$resolvedDestination = [IO.Path]::GetFullPath($Destination)
if (-not $resolvedDestination.StartsWith($allowedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Smoke export must remain inside workspace artifacts.' }
$owned = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId"
$expected = Join-Path $workspace 'src-tauri\target\debug\fluxdl.exe'
if (-not $owned -or $owned.ExecutablePath -ne $expected) { throw 'Only the workspace debug executable can be automated.' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FluxDiagnosticPicker {
    private delegate bool EnumProc(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder value, int count);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, string text);
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    public static IntPtr Find(uint processId) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((window,data)=> { uint owner; GetWindowThreadProcessId(window,out owner); if(owner==processId && IsWindowVisible(window)) { var name=new StringBuilder(128); GetClassName(window,name,name.Capacity); if(name.ToString()=="#32770") { if(found!=IntPtr.Zero) throw new Exception("Ambiguous owned dialogs"); found=window; } } return true; },IntPtr.Zero);
        return found;
    }
    public static void Save(IntPtr dialog, uint processId, string destination) {
        IntPtr edit=IntPtr.Zero;
        EnumChildWindows(dialog,(child,data)=> { uint owner; GetWindowThreadProcessId(child,out owner); var kind=new StringBuilder(128); GetClassName(child,kind,kind.Capacity); if(owner==processId && kind.ToString()=="Edit" && GetDlgCtrlID(child)==1001) edit=child; return true; },IntPtr.Zero);
        if(edit==IntPtr.Zero) throw new Exception("Owned File Name edit missing");
        SendMessage(edit,0x000C,IntPtr.Zero,destination);
        var save=GetDlgItem(dialog,1); uint pid; GetWindowThreadProcessId(save,out pid);
        if(save==IntPtr.Zero || pid!=processId) throw new Exception("Owned Save button missing");
        if(!PostMessage(save,0x00F5,IntPtr.Zero,IntPtr.Zero)) throw new Exception("Owned Save button invoke failed");
    }
}
'@
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $ProcessId)
$deadline = [DateTime]::UtcNow.AddSeconds(20)
$dialog = $null
while ([DateTime]::UtcNow -lt $deadline) {
    $handle = [FluxDiagnosticPicker]::Find($ProcessId)
    if ($handle -ne [IntPtr]::Zero) { $dialog = [System.Windows.Automation.AutomationElement]::FromHandle($handle) }
    if ($dialog) { break }
    Start-Sleep -Milliseconds 100
}
if (-not $dialog) { throw 'Owned diagnostic save dialog was not found.' }
[FluxDiagnosticPicker]::Save($handle, $ProcessId, $resolvedDestination)
Write-Output 'Owned native diagnostic save dialog confirmed.'
