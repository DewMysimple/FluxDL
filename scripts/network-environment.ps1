$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Read only. Do not read Clash files, subscriptions, controller keys or command lines.
$processes = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '(?i)(clash|mihomo)' } | ForEach-Object {
    [pscustomobject]@{ pid = [int]$_.ProcessId; name = [IO.Path]::GetFileNameWithoutExtension($_.Name) }
})
$ids = @($processes | ForEach-Object { $_.pid })
$listeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $ids -contains [int]$_.OwningProcess } | ForEach-Object {
    [pscustomobject]@{ pid = [int]$_.OwningProcess; address = if ($_.LocalAddress -in @('127.0.0.1','::1','0.0.0.0','::')) { $_.LocalAddress } else { 'non-loopback' }; port = [int]$_.LocalPort }
})
$systemProxyEnabled = $null
$systemProxyHasAuthentication = $false
$systemProxyLocalPorts = @()
try {
    $settings = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
    $systemProxyEnabled = [bool]$settings.ProxyEnable
    $rawProxy = [string]$settings.ProxyServer
    $systemProxyHasAuthentication = $rawProxy.Contains('@')
    if (-not $systemProxyHasAuthentication) {
        $systemProxyLocalPorts = @([regex]::Matches($rawProxy,'(?:127\.0\.0\.1|localhost):([0-9]{1,5})') | ForEach-Object { [int]$_.Groups[1].Value } | Where-Object { $_ -ge 1 -and $_ -le 65535 } | Select-Object -Unique)
    }
} catch { }
$environment = @()
foreach ($key in @('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','NO_PROXY')) {
    $value = [Environment]::GetEnvironmentVariable($key, 'Process')
    $record = [ordered]@{ variable = $key; present = -not [string]::IsNullOrWhiteSpace($value); authentication_present = $false; local_port = $null; protocol = $null }
    if ($key -ne 'NO_PROXY' -and $value) {
        try {
            $uri = [Uri]$value
            $record.authentication_present = -not [string]::IsNullOrEmpty($uri.UserInfo)
            if ($uri.Scheme -in @('http','https','socks5','socks5h')) { $record.protocol = $uri.Scheme }
            if ($uri.Host -in @('127.0.0.1','localhost','::1') -and $uri.Port -ge 1 -and $uri.Port -le 65535) { $record.local_port = $uri.Port }
        } catch { }
    }
    $environment += [pscustomobject]$record
}
$tunCandidates = @()
$otherVirtualAdapters = 0
try {
    foreach ($adapter in @(Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue)) {
        $combined = [string]$adapter.Name + ' ' + [string]$adapter.InterfaceDescription
        if ($combined -match '(?i)(mihomo|clash|wintun|\btun\b|verge)') {
            $tunCandidates += [pscustomobject]@{ index = [int]$adapter.ifIndex; up = $adapter.Status -eq 'Up' }
        } elseif ($combined -match '(?i)(VPN|TAP|WireGuard|ZeroTier|Tailscale|OpenVPN)') { $otherVirtualAdapters++ }
    }
} catch { }
$defaultInterfaces = @()
try { $defaultInterfaces = @(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty InterfaceIndex -Unique) } catch { }
$version = [Environment]::OSVersion.Version
[ordered]@{
    schema_version = 1
    captured_utc = [DateTime]::UtcNow.ToString('o')
    os = 'windows'
    os_major = $version.Major
    os_build = $version.Build
    architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    processes = $processes
    listeners = $listeners
    system_proxy_enabled = $systemProxyEnabled
    system_proxy_authentication_present = $systemProxyHasAuthentication
    system_proxy_local_ports = $systemProxyLocalPorts
    environment_proxy = $environment
    tun_candidates = $tunCandidates
    default_route_interface_indices = $defaultInterfaces
    other_vpn_adapter_count = $otherVirtualAdapters
    actual_outbound_route = 'uncertain; no controller credentials or configuration collected'
} | ConvertTo-Json -Depth 6 -Compress
