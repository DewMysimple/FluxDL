param([Parameter(Mandatory=$true)][int]$OwnedPid, [int]$ProxyPort = 0)
$ErrorActionPreference = 'Stop'
# Only this runner's descendants; no command lines, remote IPs, titles or user paths.
$all = @(Get-CimInstance Win32_Process)
$ownedIds = @($OwnedPid)
for ($depth = 0; $depth -lt 6; $depth++) {
    $next = @($all | Where-Object { $ownedIds -contains [int]$_.ParentProcessId -and $ownedIds -notcontains [int]$_.ProcessId } | Select-Object -ExpandProperty ProcessId)
    if ($next.Count -eq 0) { break }
    $ownedIds += $next
}
$engines = @($all | Where-Object { $ownedIds -contains [int]$_.ProcessId -and $_.Name -match '(?i)^(yt-dlp|ffmpeg|deno|node).*\.exe$' })
$engineIds = @($engines | Select-Object -ExpandProperty ProcessId)
$connections = @(Get-NetTCPConnection -State Established -ErrorAction SilentlyContinue | Where-Object { $engineIds -contains [int]$_.OwningProcess })
$localProxy = @($connections | Where-Object { $_.RemoteAddress -in @('127.0.0.1','::1') -and $_.RemotePort -eq $ProxyPort -and $ProxyPort -gt 0 })
[ordered]@{ owned_engine_process_count = $engines.Count; established_tcp_count = $connections.Count; confirmed_proxy_loopback_connections = $localProxy.Count; local_port = if ($ProxyPort -gt 0) { $ProxyPort } else { $null }; actual_remote_node = 'not collected; route beyond local proxy uncertain' } | ConvertTo-Json -Compress
