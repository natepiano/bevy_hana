# Enable the Virtual Display Driver monitor (reconnect harness "power_on").
# Windows-only. Requires Administrator (run the reconnect suite from one elevated shell).
$ErrorActionPreference = 'Stop'
$dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }
if (-not $dev) { Write-Error 'Virtual Display Driver device not found; run setup.ps1 first.'; exit 1 }
$dev | Enable-PnpDevice -Confirm:$false

# Enabling the device is async: the monitor child (MTT1337) takes a moment to appear.
# Wait for it so a caller that discovers monitors right after (the suite's initial
# discovery) actually sees the virtual monitor.
for ($i = 0; $i -lt 40; $i++) {
    $monitor = Get-PnpDevice -Class Monitor -PresentOnly -ErrorAction SilentlyContinue |
        Where-Object { $_.InstanceId -match 'MTT1337' }
    if ($monitor) { exit 0 }
    Start-Sleep -Milliseconds 500
}
Write-Error 'Virtual Display Driver enabled but its monitor did not appear within 20s.'
exit 1
