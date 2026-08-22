# Disable the Virtual Display Driver monitor (reconnect harness "power_off").
# Windows-only. Requires Administrator. Disabling is safe: the IDD is a user-mode
# (Session 0) driver, so toggling it cannot disturb the real display topology.
$ErrorActionPreference = 'Stop'
$dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }
if (-not $dev) { Write-Error 'Virtual Display Driver device not found; run setup.ps1 first.'; exit 1 }
$dev | Disable-PnpDevice -Confirm:$false
exit 0
