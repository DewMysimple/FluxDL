$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
$legacyExecutable = [Console]::In.ReadToEnd()
$running = @(Get-CimInstance Win32_Process -Filter "Name='FluxDL.exe'" | Where-Object {
    [string]::Equals($_.ExecutablePath, $legacyExecutable, [System.StringComparison]::OrdinalIgnoreCase)
})
if ($running.Count -gt 0) { 'true' } else { 'false' }
