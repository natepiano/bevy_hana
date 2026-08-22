# Apply vdd_settings.xml to an installed VDD and reload it (disable+enable so the
# driver re-reads its config). Windows-only, run elevated. Leaves the VDD ENABLED.
$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$p = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run elevated.' }

New-Item -ItemType Directory -Force -Path 'C:\VirtualDisplayDriver' | Out-Null
Copy-Item (Join-Path $ScriptDir 'vdd_settings.xml') 'C:\VirtualDisplayDriver\vdd_settings.xml' -Force
Write-Host 'config -> C:\VirtualDisplayDriver\vdd_settings.xml'

$dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }
if (-not $dev) { throw 'Virtual Display Driver not found; run setup.ps1 first.' }
if ($dev.Status -eq 'OK') { $dev | Disable-PnpDevice -Confirm:$false; Start-Sleep -Seconds 2 }
(Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }) | Enable-PnpDevice -Confirm:$false
Start-Sleep -Seconds 3
Write-Host 'VDD reloaded and enabled.'
