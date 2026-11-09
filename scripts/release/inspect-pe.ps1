$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$files = [Console]::In.ReadToEnd() | ConvertFrom-Json
$results = @($files | ForEach-Object {
    $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($_)
    [pscustomobject]@{
        product = $info.ProductName
        productVersion = $info.ProductVersion
        fileVersion = $info.FileVersion
    }
})
ConvertTo-Json -InputObject $results -Compress
