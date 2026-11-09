# Run with Windows PowerShell 5.1 (UIAutomation assemblies are built into Windows).
# Does not launch/kill processes, change the clipboard, or select/write user files.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$TargetProcessId,
    [Parameter(Mandatory = $true)][string]$TestDataDir,
    [string]$WorkspaceRoot = (Split-Path -Parent $PSScriptRoot),
    [switch]$ProbeOnly,
    [switch]$CancelDialogOnly,
    [switch]$AllowExit
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
if ($ProbeOnly -and $CancelDialogOnly) { throw 'ProbeOnly and CancelDialogOnly are mutually exclusive.' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName WindowsBase
Add-Type @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public sealed class FluxNativeWindow {
  public IntPtr Handle; public uint ProcessId; public string Title; public string ClassName;
  public bool Visible; public bool Iconic;
}
public sealed class FluxRegisteredTray {
  public IntPtr Handle; public uint Id;
  public int Left, Top, Right, Bottom;
}
public sealed class FluxChildWait : IDisposable {
  private IntPtr handle;
  public readonly uint ProcessId;
  [DllImport("kernel32.dll", SetLastError=true)] private static extern IntPtr OpenProcess(uint rights, bool inherit, uint id);
  [DllImport("kernel32.dll")] private static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
  [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
  public FluxChildWait(uint id) { ProcessId=id; handle=OpenProcess(0x00100000,false,id); if(handle==IntPtr.Zero) throw new Win32Exception(); }
  public bool IsAlive { get { return WaitForSingleObject(handle,0)==258; } }
  public bool WaitExited(uint milliseconds) { return WaitForSingleObject(handle,milliseconds)==0; }
  public void Dispose() { if(handle!=IntPtr.Zero) { CloseHandle(handle); handle=IntPtr.Zero; } }
}
public static class FluxNativeSmoke {
  private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr data);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr hwnd, StringBuilder value, int size);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, StringBuilder value, int size);
  [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint id);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
  [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr hwnd, uint command);
  [DllImport("user32.dll", SetLastError=true)] private static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam, uint flags, uint timeout, out IntPtr result);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [StructLayout(LayoutKind.Sequential)] private struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public UIntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] private struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mouse; }
  [StructLayout(LayoutKind.Sequential)] private struct INPUT { public uint type; public INPUTUNION data; }
  [DllImport("user32.dll")] private static extern bool GetCursorPos(out POINT point);
  [DllImport("user32.dll", SetLastError=true)] private static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll", SetLastError=true)] private static extern uint SendInput(uint count, INPUT[] input, int size);
  [StructLayout(LayoutKind.Sequential)] private struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] private struct NOTIFYICONIDENTIFIER { public uint Size; public IntPtr Window; public uint Id; public Guid GuidItem; }
  [DllImport("shell32.dll")] private static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER icon, out RECT rectangle);
  private static string WindowClass(IntPtr hwnd) {
    var text = new StringBuilder(128); GetClassName(hwnd,text,text.Capacity); return text.ToString();
  }
  public static FluxNativeWindow[] Windows(uint processId) {
    var result = new List<FluxNativeWindow>();
    EnumWindows((hwnd, data) => {
      uint owner; GetWindowThreadProcessId(hwnd, out owner);
      if (owner == processId) {
        var title = new StringBuilder(512); var kind = new StringBuilder(128);
        GetWindowText(hwnd, title, title.Capacity); GetClassName(hwnd, kind, kind.Capacity);
        result.Add(new FluxNativeWindow { Handle=hwnd, ProcessId=owner, Title=title.ToString(), ClassName=kind.ToString(), Visible=IsWindowVisible(hwnd), Iconic=IsIconic(hwnd) });
      }
      return true;
    }, IntPtr.Zero);
    return result.ToArray();
  }
  public static void RequireOwner(IntPtr hwnd, uint expected) {
    uint owner; GetWindowThreadProcessId(hwnd, out owner);
    if (!IsWindow(hwnd) || owner != expected) throw new InvalidOperationException("Window is not owned by the selected test process.");
  }
  private static bool TrayRect(IntPtr hwnd, uint id, out RECT rectangle) {
    var identifier = new NOTIFYICONIDENTIFIER { Size=(uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER)), Window=hwnd, Id=id, GuidItem=Guid.Empty };
    return Shell_NotifyIconGetRect(ref identifier,out rectangle)==0 && rectangle.Right>rectangle.Left && rectangle.Bottom>rectangle.Top;
  }
  // Read only. Current locked tray-icon 0.25.1 registers (HWND,uID), without NIF_GUID.
  public static FluxRegisteredTray RegisteredTray(uint expected) {
    var found = new List<FluxRegisteredTray>();
    foreach (var window in Windows(expected)) {
      if (window.ClassName!="tray_icon_app") continue;
      RequireOwner(window.Handle,expected);
      for (uint id=1;id<=16;id++) {
        RECT rectangle;
        if (TrayRect(window.Handle,id,out rectangle)) found.Add(new FluxRegisteredTray { Handle=window.Handle, Id=id, Left=rectangle.Left, Top=rectangle.Top, Right=rectangle.Right, Bottom=rectangle.Bottom });
      }
    }
    if (found.Count>1) throw new NotSupportedException("Registered tray identity is ambiguous; no callback will be sent.");
    return found.Count==1 ? found[0] : null;
  }
  // Only one private message is allowed: 6002, whose parameters are plain integers.
  // Never send 6003/6004/6006: tray-icon interprets those parameters as Rust pointers.
  public static void RegisteredTrayCallback(FluxRegisteredTray tray, uint expected, bool right) {
    if (tray==null || tray.Id<1 || tray.Id>16) throw new InvalidOperationException("Invalid registered tray identity.");
    RequireOwner(tray.Handle,expected);
    if (WindowClass(tray.Handle)!="tray_icon_app") throw new InvalidOperationException("Tray HWND class changed.");
    RECT rectangle;
    if (!TrayRect(tray.Handle,tray.Id,out rectangle)) throw new NotSupportedException("Tray icon is no longer registered; callback refused.");
    // No NIM_SETVERSION is used by this locked crate: wParam=uID, lParam=WM_*.
    if (!PostMessage(tray.Handle,6002,new IntPtr((long)tray.Id),new IntPtr(right ? 0x0205 : 0x0202))) throw new Win32Exception();
  }
  public static bool CancelOwnMenu(IntPtr menu, uint expected) {
    RequireOwner(menu,expected);
    if (WindowClass(menu)!="#32768") throw new InvalidOperationException("Only the owned popup menu may be dismissed.");
    var owner = GetWindow(menu,4); // GW_OWNER: the HWND that called TrackPopupMenu.
    if (owner==IntPtr.Zero) { var tray=RegisteredTray(expected); if(tray==null) return false; owner=tray.Handle; }
    RequireOwner(owner,expected);
    if (WindowClass(owner)!="tray_icon_app") return false;
    // Documented cancellation of menu mode; not keyboard input or a guessed command ID.
    if (!PostMessage(owner,0x001F,IntPtr.Zero,IntPtr.Zero)) throw new Win32Exception();
    return true;
  }
  public static void PostOwn(IntPtr hwnd, uint expected, uint message, int wparam) {
    RequireOwner(hwnd, expected);
    if (!PostMessage(hwnd, message, new IntPtr(wparam), IntPtr.Zero)) throw new Win32Exception();
  }
  public static void ClickOwnButton(IntPtr hwnd, uint expected) {
    RequireOwner(hwnd, expected); IntPtr result;
    if (SendMessageTimeout(hwnd, 0x00F5, IntPtr.Zero, IntPtr.Zero, 2, 2000, out result)==IntPtr.Zero) throw new Win32Exception();
  }
  // Only called after exact PID tooltip matching and a second UIA hit test.
  public static void MouseClick(int x, int y, bool right) {
    POINT previous; if (!GetCursorPos(out previous)) throw new Win32Exception();
    try {
      if (!SetCursorPos(x,y)) throw new Win32Exception();
      var input = new INPUT[2];
      input[0].data.mouse.dwFlags = right ? 8u : 2u;
      input[1].data.mouse.dwFlags = right ? 16u : 4u;
      if (SendInput(2,input,Marshal.SizeOf(typeof(INPUT)))!=2) throw new Win32Exception();
    } finally { SetCursorPos(previous.X,previous.Y); }
  }
}
'@
function UiText([string]$Value) { [regex]::Unescape($Value) }
function NormalizePath([string]$Value) {
    $path = [IO.Path]::GetFullPath($Value)
    if ($path.StartsWith('\\?\')) { $path = $path.Substring(4) }
    $path.TrimEnd('\')
}
$WorkspaceRoot = NormalizePath $WorkspaceRoot
$artifactRoot = NormalizePath (Join-Path $WorkspaceRoot 'artifacts')
$TestDataDir = NormalizePath $TestDataDir
if (-not $TestDataDir.StartsWith($artifactRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'TestDataDir must be an existing isolated directory inside this workspace artifacts directory.'
}
if (-not (Test-Path -LiteralPath (Join-Path $TestDataDir 'fluxdl.sqlite3') -PathType Leaf)) {
    throw 'The selected isolated test database does not exist.'
}
$expectedExe = NormalizePath (Join-Path $WorkspaceRoot 'src-tauri\target\debug\fluxdl.exe')
$target = Get-Process -Id $TargetProcessId -ErrorAction Stop
if (-not [string]::Equals((NormalizePath $target.Path), $expectedExe, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Target must be this workspace src-tauri/target/debug/fluxdl.exe, never release/FluxDL.exe.'
}
$targetStart = $target.StartTime.ToUniversalTime().Ticks
$null = $target.Handle # Retain a read handle so graceful exit code can be inspected later.
$isolationProof = $false
$proofFile = Join-Path $TestDataDir 'native-smoke-target.json'
if (Test-Path -LiteralPath $proofFile -PathType Leaf) {
    $proof = Get-Content -LiteralPath $proofFile -Raw | ConvertFrom-Json
    $proofStart = [DateTime]::Parse([string]$proof.processStartTimeUtc, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind).ToUniversalTime().Ticks
    $isolationProof = $proof.schemaVersion -eq 1 -and $proof.processId -eq $TargetProcessId -and
        $proofStart -eq $targetStart -and $proof.testGate -is [bool] -and $proof.testGate -and
        [string]::Equals((NormalizePath $proof.debugExecutable), $expectedExe, [StringComparison]::OrdinalIgnoreCase) -and
        [string]::Equals((NormalizePath $proof.testDataDir), $TestDataDir, [StringComparison]::OrdinalIgnoreCase)
    if (-not $isolationProof) { throw 'Controller isolation proof does not match the exact selected test process.' }
}
$lockText = Get-Content -LiteralPath (Join-Path $WorkspaceRoot 'src-tauri\Cargo.lock') -Raw
$trayVersions = [regex]::Matches($lockText, '(?m)^name = "tray-icon"\r?\nversion = "([^"]+)"')
$registeredCallbackSupported = $trayVersions.Count -eq 1 -and $trayVersions[0].Groups[1].Value -eq '0.25.1'
$protected = @(Get-Process -Name fluxdl -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $TargetProcessId } | ForEach-Object {
    [pscustomobject]@{ id = $_.Id; started = $_.StartTime.ToUniversalTime().Ticks }
})
$expectedTrayName = UiText "FluxDL \u00b7 \u6d4b\u8bd5\u5b9e\u4f8b $TargetProcessId"
$checks = New-Object 'System.Collections.Generic.List[object]'
$trayRoutes = New-Object 'System.Collections.Generic.List[object]'
$report = [ordered]@{
    schemaVersion = 1; generatedAt = [DateTime]::UtcNow.ToString('o'); processId = $TargetProcessId
    probeOnly = [bool]$ProbeOnly; cancelDialogOnly = [bool]$CancelDialogOnly; allowExit = [bool]$AllowExit
    gate = [ordered]@{ debugExecutableMatched = $true; isolatedArtifactDirectory = $true; controllerIsolationProof = [bool]$isolationProof; exactTestTrayMarker = $false; registeredTrayIdentity = $false }
    checks = $checks; errors = @(); protectedProcesses = @($protected | ForEach-Object { [ordered]@{ id = $_.id } }); trayRoutes = $trayRoutes
}
function AssertTarget {
    $current = Get-Process -Id $TargetProcessId -ErrorAction Stop
    if ($current.StartTime.ToUniversalTime().Ticks -ne $targetStart -or
        -not [string]::Equals((NormalizePath $current.Path), $expectedExe, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Target identity changed; refusing further native actions.'
    }
}
function WaitNative([scriptblock]$Condition, [string]$Description, [int]$Seconds = 8) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out: $Description"
}
function OwnedWindow([string]$Title) {
    AssertTarget
    $matches = @([FluxNativeSmoke]::Windows($TargetProcessId) | Where-Object { $_.Title -eq $Title })
    if ($matches.Count -ne 1) { throw "Expected exactly one owned window: $Title" }
    $matches[0]
}
function ExactUi($Root, [string]$Name, $ControlType) {
    $conditions = New-Object 'System.Collections.Generic.List[System.Windows.Automation.Condition]'
    $conditions.Add((New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $Name)))
    if ($ControlType) { $conditions.Add((New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $ControlType))) }
    $condition = if ($conditions.Count -eq 1) { $conditions[0] } else { New-Object System.Windows.Automation.AndCondition($conditions.ToArray()) }
    $found = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    if ($found.Count -gt 1) { throw "Ambiguous native control: $Name" }
    if ($found.Count -eq 1) { return $found[0] }
    return $null
}
function InvokeUi($Element, [bool]$ExpectExit = $false) {
    AssertTarget
    if (-not $Element -or -not $Element.Current.IsEnabled) { throw [NotSupportedException]::new('Required scoped UIA control is unavailable or disabled.') }
    $pattern = $null
    if (-not $Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        throw [NotSupportedException]::new('Scoped control does not expose InvokePattern; no keyboard or coordinate fallback was attempted.')
    }
    try { $pattern.Invoke() }
    catch {
        # An invoked Exit menu can disconnect its UIA provider before Invoke returns.
        if (-not ($ExpectExit -and $target.HasExited -and $target.ExitCode -eq 0)) { throw }
    }
}
function TrayElement {
    $desktop = [System.Windows.Automation.AutomationElement]::RootElement
    $matches = New-Object 'System.Collections.Generic.List[System.Windows.Automation.AutomationElement]'
    foreach ($kind in @('Shell_TrayWnd', 'NotifyIconOverflowWindow', 'TopLevelWindowForOverflowXamlIsland')) {
        $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, $kind)
        $roots = $desktop.FindAll([System.Windows.Automation.TreeScope]::Children, $condition)
        foreach ($root in $roots) {
            $element = ExactUi $root $expectedTrayName $null
            if ($element) { $matches.Add($element) }
        }
    }
    if ($matches.Count -gt 1) { throw 'Multiple matching PID tray markers; refusing ambiguous native actions.' }
    if ($matches.Count -eq 1) { return $matches[0] }
    return $null
}
function ClickTray([bool]$Right) {
    AssertTarget
    $icon = TrayElement
    if ($icon -and -not $icon.Current.IsOffscreen) {
        $box = $icon.Current.BoundingRectangle
        if (-not $box.IsEmpty -and $box.Width -ge 1 -and $box.Height -ge 1) {
            $x = [int]($box.Left + $box.Width / 2); $y = [int]($box.Top + $box.Height / 2)
            $point = New-Object System.Windows.Point($x, $y)
            $hit = [System.Windows.Automation.AutomationElement]::FromPoint($point)
            if ($hit.Current.Name -eq $expectedTrayName) {
                [FluxNativeSmoke]::MouseClick($x, $y, $Right)
                $trayRoutes.Add([ordered]@{ button = $(if($Right){'right'}else{'left'}); route = 'physicalPointerHit'; exactPidTooltip = $true })
                return
            }
        }
    }
    # No physical hit was safe. Use only a freshly verified, registered icon owned by this PID.
    if (-not $registeredCallbackSupported) { throw [NotSupportedException]::new('Registered callback fallback requires the audited locked tray-icon 0.25.1; no private callback sent.') }
    $registered = [FluxNativeSmoke]::RegisteredTray($TargetProcessId)
    if (-not $registered) { throw [NotSupportedException]::new('Neither an exact UIA tray target nor a unique owned registered icon is available; no callback sent.') }
    AssertTarget
    [FluxNativeSmoke]::RegisteredTrayCallback($registered, $TargetProcessId, $Right)
    $trayRoutes.Add([ordered]@{ button = $(if($Right){'right'}else{'left'}); route = 'registeredTrayCallback'; windowHandle = $registered.Handle.ToInt64(); iconId = $registered.Id; message = 6002; mouseMessage = $(if($Right){517}else{514}) })
}
function TrayCommand([string]$Caption, [bool]$ExpectExit = $false) {
    ClickTray $true
    $script:menuHandle = [IntPtr]::Zero
    try {
        WaitNative {
            $ownedMenus = @([FluxNativeSmoke]::Windows($TargetProcessId) | Where-Object { $_.Visible -and $_.ClassName -eq '#32768' })
            if ($ownedMenus.Count -gt 1) { throw 'Ambiguous popup menus in the target process.' }
            if ($ownedMenus.Count -eq 1) { $script:menuHandle = $ownedMenus[0].Handle; return $true }
            return $false
        } 'owned native tray popup menu'
        [FluxNativeSmoke]::RequireOwner($script:menuHandle, $TargetProcessId)
        $menu = [System.Windows.Automation.AutomationElement]::FromHandle($script:menuHandle)
        $item = ExactUi $menu $Caption ([System.Windows.Automation.ControlType]::MenuItem)
        InvokeUi $item $ExpectExit
    } catch {
        if ($script:menuHandle -ne [IntPtr]::Zero -and [FluxNativeSmoke]::IsWindowVisible($script:menuHandle)) {
            try {
                AssertTarget
                $dismissed = [FluxNativeSmoke]::CancelOwnMenu($script:menuHandle, $TargetProcessId)
                $report['nativeMenuCleanup'] = [ordered]@{ processId = $TargetProcessId; requested = $dismissed; method = 'WM_CANCELMODE to the exact owned tray menu owner' }
            } catch { $report['nativeMenuCleanupError'] = $_.Exception.GetBaseException().Message }
        }
        throw
    }
}
function CheckNative([string]$Name, [scriptblock]$Action) {
    try {
        $null = & $Action
        $checks.Add([ordered]@{ name = $Name; status = 'passed'; method = 'owned native target; inspect trayRoutes for exact interaction path' })
        Write-Host "PASS $Name"
    } catch {
        $cause = $_.Exception.GetBaseException()
        if ($cause -is [NotSupportedException]) {
            $checks.Add([ordered]@{ name = $Name; status = 'unmeasured'; reason = $cause.Message })
            Write-Host "UNMEASURED $Name - $($cause.Message)"
        } else {
            $checks.Add([ordered]@{ name = $Name; status = 'failed'; reason = $cause.Message })
            throw
        }
    }
}
function CancelOwnedDialog([string]$Title) {
    AssertTarget
    WaitNative {
        $dialogs = @([FluxNativeSmoke]::Windows($TargetProcessId) | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq $Title })
        if ($dialogs.Count -gt 1) { throw 'Ambiguous owned native dialogs.' }
        if ($dialogs.Count -eq 1) { $script:dialogHandle = $dialogs[0].Handle; return $true }
        return $false
    } 'owned native directory dialog with exact title'
    [FluxNativeSmoke]::RequireOwner($script:dialogHandle, $TargetProcessId)
    $cancel = [FluxNativeSmoke]::GetDlgItem($script:dialogHandle, 2)
    $method = if ($cancel -ne [IntPtr]::Zero) {
        [FluxNativeSmoke]::ClickOwnButton($cancel, $TargetProcessId)
        'IDCANCEL button'
    } else {
        [FluxNativeSmoke]::PostOwn($script:dialogHandle, $TargetProcessId, 0x0010, 0)
        'WM_CLOSE on the exact owned dialog'
    }
    WaitNative { -not [FluxNativeSmoke]::IsWindowVisible($script:dialogHandle) } 'directory dialog cancellation'
    $report['nativeDialog'] = [ordered]@{ processId = $TargetProcessId; title = $Title; windowClass = '#32770'; cancelMethod = $method }
}
$exitCode = 0
try {
    $marker = TrayElement
    $report.gate.exactTestTrayMarker = [bool]$marker
    $registered = if ($CancelDialogOnly -or -not $registeredCallbackSupported) { $null } else { [FluxNativeSmoke]::RegisteredTray($TargetProcessId) }
    $report.gate.registeredTrayIdentity = [bool]$registered
    if ($registered) { $report['registeredTray'] = [ordered]@{ processId = $TargetProcessId; windowClass = 'tray_icon_app'; windowHandle = $registered.Handle.ToInt64(); iconId = $registered.Id; registration = 'Shell_NotifyIconGetRect S_OK' } }
    $report['ownedWindows'] = @([FluxNativeSmoke]::Windows($TargetProcessId) | Where-Object { $_.Title -in @('FluxDL', (UiText 'FluxDL \u00b7 Quick Capture')) } | ForEach-Object {
        [ordered]@{ title = $_.Title; handle = $_.Handle.ToInt64(); visible = $_.Visible; minimized = $_.Iconic }
    })
    if (-not $ProbeOnly -and -not $marker -and -not $isolationProof) {
        throw [NotSupportedException]::new('An exact test PID tooltip or matching native-smoke-target.json controller proof is required before native actions.')
    }
    if ($CancelDialogOnly) {
        # The owning CDP test must start this uniquely titled picker and verify its Promise resolves null.
        # This mode never touches the tray and never enumerates FileDialog descendants/file names.
        CheckNative 'cancel externally opened uniquely titled native directory picker' {
            CancelOwnedDialog "FluxDL Native Smoke $TargetProcessId"
        }
    } elseif ($ProbeOnly) {
        $checks.Add([ordered]@{ name = 'read-only native isolation probe'; status = 'observed'; pidMarkerVisible = [bool]($marker -and -not $marker.Current.IsOffscreen); registeredOwnedIcon = [bool]$registered })
    } elseif (-not $marker -and -not $registered) {
        throw [NotSupportedException]::new('Neither exact debug PID tooltip nor unique owned registered icon is accessible. No native actions were performed.')
    } else {
        CheckNative 'native directory picker opens and Cancel keeps the path unchanged' {
            $workspace = OwnedWindow 'FluxDL'
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($workspace.Handle)
            InvokeUi (ExactUi $root (UiText '\u8bbe\u7f6e') ([System.Windows.Automation.ControlType]::Button))
            $pickerName = UiText '\u9009\u62e9\u9ed8\u8ba4\u4fdd\u5b58\u76ee\u5f55'
            $edit = ExactUi $root (UiText '\u9ed8\u8ba4\u4fdd\u5b58\u76ee\u5f55') ([System.Windows.Automation.ControlType]::Edit)
            if (-not $edit) { throw [NotSupportedException]::new('Owned WebView edit is not exposed to UIA.') }
            $value = $null
            if (-not $edit.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$value)) { throw [NotSupportedException]::new('Directory edit has no readable ValuePattern.') }
            $before = $value.Current.Value
            if (@([FluxNativeSmoke]::Windows($TargetProcessId) | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }).Count) { throw 'A pre-existing owned dialog is open; refusing to interfere.' }
            InvokeUi (ExactUi $root $pickerName ([System.Windows.Automation.ControlType]::Button))
            CancelOwnedDialog $pickerName
            if ($value.Current.Value -ne $before) { throw 'Cancel changed the configured directory draft.' }
        }
        CheckNative 'real tray menu restores Workspace from minimized state' {
            $workspace = OwnedWindow 'FluxDL'
            [FluxNativeSmoke]::PostOwn($workspace.Handle, $TargetProcessId, 0x0112, 0xF020)
            WaitNative { [FluxNativeSmoke]::IsIconic($workspace.Handle) } 'owned Workspace minimization'
            TrayCommand (UiText '\u6253\u5f00\u5de5\u4f5c\u53f0')
            WaitNative { [FluxNativeSmoke]::IsWindowVisible($workspace.Handle) -and -not [FluxNativeSmoke]::IsIconic($workspace.Handle) } 'tray Workspace restore'
        }
        CheckNative 'real tray menu opens Quick Capture and native Close hides it' {
            $quick = OwnedWindow (UiText 'FluxDL \u00b7 Quick Capture')
            [FluxNativeSmoke]::PostOwn($quick.Handle, $TargetProcessId, 0x0010, 0)
            WaitNative { -not [FluxNativeSmoke]::IsWindowVisible($quick.Handle) } 'owned Quick Capture hide'
            TrayCommand (UiText '\u5feb\u901f\u6355\u83b7')
            WaitNative { [FluxNativeSmoke]::IsWindowVisible($quick.Handle) } 'tray Quick Capture show'
            [FluxNativeSmoke]::PostOwn($quick.Handle, $TargetProcessId, 0x0010, 0)
            WaitNative { -not [FluxNativeSmoke]::IsWindowVisible($quick.Handle) } 'Quick Capture closes without exiting application'
            AssertTarget
        }
        CheckNative 'native Workspace Close hides it and tray left-button route restores it' {
            $workspace = OwnedWindow 'FluxDL'
            [FluxNativeSmoke]::PostOwn($workspace.Handle, $TargetProcessId, 0x0010, 0)
            # An active test download presents the application's background/exit prompt.
            Start-Sleep -Milliseconds 200
            if ([FluxNativeSmoke]::IsWindowVisible($workspace.Handle)) {
                $root = [System.Windows.Automation.AutomationElement]::FromHandle($workspace.Handle)
                $background = ExactUi $root (UiText '\u5728\u540e\u53f0\u7ee7\u7eed') ([System.Windows.Automation.ControlType]::Button)
                if (-not $background) { throw [NotSupportedException]::new('Workspace stayed visible and its scoped background button is not exposed; close/background native assertion is unmeasured.') }
                InvokeUi $background
            }
            WaitNative { -not [FluxNativeSmoke]::IsWindowVisible($workspace.Handle) } 'Workspace background hide'
            AssertTarget
            ClickTray $false
            WaitNative { [FluxNativeSmoke]::IsWindowVisible($workspace.Handle) } 'tray left click Workspace restore'
        }
        CheckNative 'real tray Exit waits for and exits only the selected test application' {
            if (-not $AllowExit) { throw [NotSupportedException]::new('AllowExit was not supplied; the test process was preserved.') }
            $engineWait = $null
            $report['engineStopped'] = [ordered]@{ status = 'unmeasured'; reason = 'No uniquely identified live direct child yt-dlp.exe before tray Quit.' }
            try {
                try {
                    $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$TargetProcessId" | Where-Object { $_.Name -ieq 'yt-dlp.exe' })
                    if ($children.Count -eq 1) {
                        $child = $children[0]
                        $engineWait = [FluxChildWait]::new([uint32]$child.ProcessId)
                        $again = Get-CimInstance Win32_Process -Filter "ProcessId=$($child.ProcessId)"
                        if (-not $again -or $again.ParentProcessId -ne $TargetProcessId -or $again.CreationDate -ne $child.CreationDate -or -not $engineWait.IsAlive) {
                            $engineWait.Dispose(); $engineWait = $null
                        }
                    }
                } catch {
                    if ($engineWait) { $engineWait.Dispose(); $engineWait = $null }
                    $report['engineStopped'] = [ordered]@{ status = 'unmeasured'; reason = 'The owned engine process could not be identified and opened with SYNCHRONIZE access.' }
                }
                AssertTarget
                TrayCommand (UiText '\u9000\u51fa FluxDL\uff08\u505c\u6b62\u4e0b\u8f7d\uff09') $true
                WaitNative { $null -eq (Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue) } 'owned graceful tray exit' 15
                if ($target.ExitCode -ne 0) { throw "Selected application exited with nonzero code $($target.ExitCode)." }
                $report['targetExitCode'] = $target.ExitCode
                if ($engineWait) {
                    $stopped = $engineWait.WaitExited(5000)
                    $report['engineStopped'] = [ordered]@{ status = $(if($stopped){'passed'}else{'failed'}); processId = $engineWait.ProcessId; observedAliveBeforeQuit = $true; kernelHandleSignaledAfterQuit = $stopped }
                    if (-not $stopped) { throw 'The previously live owned engine kernel handle was not signaled after tray Quit.' }
                    $checks.Add([ordered]@{ name = 'owned running engine stopped after native tray Quit'; status = 'passed'; method = 'fixed SYNCHRONIZE process handle, not a PID-only lookup' })
                } else {
                    $checks.Add([ordered]@{ name = 'owned running engine stopped after native tray Quit'; status = 'unmeasured'; reason = $report.engineStopped.reason })
                }
            } finally { if ($engineWait) { $engineWait.Dispose() } }
        }
    }
} catch {
    $cause = $_.Exception.GetBaseException()
    $report.errors = @($cause.Message)
    $exitCode = if ($cause -is [NotSupportedException]) { 2 } else { 1 }
} finally {
    $report['protectedProcessesUnchanged'] = @($protected | ForEach-Object {
        $current = Get-Process -Id $_.id -ErrorAction SilentlyContinue
        [ordered]@{ id = $_.id; sameProcessStillRunning = [bool]($current -and $current.StartTime.ToUniversalTime().Ticks -eq $_.started) }
    })
    if (@($report.protectedProcessesUnchanged | Where-Object { -not $_.sameProcessStillRunning }).Count) {
        $report.errors += 'A pre-existing FluxDL process changed during the run; preservation cannot be claimed.'
        $exitCode = 1
    }
    $report['summary'] = [ordered]@{
        passed = @($checks | Where-Object { $_.status -eq 'passed' }).Count
        unmeasured = @($checks | Where-Object { $_.status -eq 'unmeasured' }).Count
        failed = @($checks | Where-Object { $_.status -eq 'failed' }).Count
    }
    $json = $report | ConvertTo-Json -Depth 8
    if (-not $ProbeOnly) { [IO.File]::WriteAllText((Join-Path $TestDataDir 'native-smoke.json'), $json, (New-Object Text.UTF8Encoding($false))) }
    Write-Output $json
    $target.Dispose()
}
exit $exitCode
