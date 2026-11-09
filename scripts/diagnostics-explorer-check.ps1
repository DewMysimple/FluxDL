param([string]$Folder, [string]$SelectedFile, [ValidateSet('Check','Snapshot','Cleanup')][string]$Action = 'Check', [string]$BeforeHandles = '')
$ErrorActionPreference = 'Stop'
$shell = New-Object -ComObject Shell.Application
if ($Action -eq 'Snapshot') {
    $handles = @($shell.Windows() | ForEach-Object { try { [long]$_.HWND } catch {} })
    ConvertTo-Json -InputObject $handles -Compress
    exit 0
}
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$expectedFolder = [IO.Path]::GetFullPath($Folder)
$artifactRoot = Join-Path $workspace 'artifacts\diagnostics'
if ($expectedFolder -ne (Join-Path $workspace 'dev-logs') -and -not $expectedFolder.StartsWith($artifactRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Only owned diagnostic folders may be inspected.' }
if ($SelectedFile -and [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($SelectedFile)) -ne $expectedFolder) { throw 'Selected file must remain in the owned folder.' }
if ($Action -eq 'Cleanup') {
    $before = @($BeforeHandles.Split(',') | Where-Object { $_ })
    foreach ($window in @($shell.Windows())) {
        try {
            if ($before -contains ([string]$window.HWND)) { continue }
            $uri = [Uri]$window.LocationURL
            if ($uri.IsFile -and [IO.Path]::GetFullPath($uri.LocalPath) -eq $expectedFolder) { $window.Quit() }
        } catch {} # Never close pre-existing or unrelated windows.
    }
    $cleanupDeadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $remaining = 0
        foreach ($window in @($shell.Windows())) {
            try {
                $uri = [Uri]$window.LocationURL
                if ($before -notcontains ([string]$window.HWND) -and $uri.IsFile -and [IO.Path]::GetFullPath($uri.LocalPath) -eq $expectedFolder) { $remaining++ }
            } catch {}
        }
        if ($remaining -eq 0) { exit 0 }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $cleanupDeadline)
    throw 'Owned Explorer windows did not close; unrelated and pre-existing windows were preserved.'
}
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ([DateTime]::UtcNow -lt $deadline) {
    foreach ($window in $shell.Windows()) {
        try {
            $uri = [Uri]$window.LocationURL
            if (-not $uri.IsFile -or [IO.Path]::GetFullPath($uri.LocalPath) -ne $expectedFolder) { continue }
            if (-not $SelectedFile) { Write-Output 'Owned diagnostics directory is open in Explorer.'; exit 0 }
            $items = $window.Document.SelectedItems()
            for ($index = 0; $index -lt $items.Count; $index++) {
                if ($items.Item($index).Path -eq $SelectedFile) { Write-Output 'Owned diagnostic ZIP is selected in Explorer.'; exit 0 }
            }
        } catch { # Ignore non-filesystem windows; never print their locations.
        }
    }
    Start-Sleep -Milliseconds 100
}
throw 'The expected owned Explorer folder or selected ZIP was not observed.'
