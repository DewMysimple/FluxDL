param([Parameter(Mandatory=$true)][string]$Folder, [string]$SelectedFile)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$expectedFolder = [IO.Path]::GetFullPath($Folder)
$artifactRoot = Join-Path $workspace 'artifacts\diagnostics'
if ($expectedFolder -ne (Join-Path $workspace 'dev-logs') -and -not $expectedFolder.StartsWith($artifactRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Only owned diagnostic folders may be inspected.' }
if ($SelectedFile -and [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($SelectedFile)) -ne $expectedFolder) { throw 'Selected file must remain in the owned folder.' }
$shell = New-Object -ComObject Shell.Application
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
