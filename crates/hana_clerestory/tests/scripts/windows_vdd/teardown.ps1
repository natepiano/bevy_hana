<#
.SYNOPSIS
  Remove the Virtual Display Driver and revert test-signing. Windows-only, run elevated.
.DESCRIPTION
  Reverses setup.ps1: removes the device node, uninstalls the driver package, and
  (by default) turns test-signing back off. Uses inbox pnputil only.
#>
[CmdletBinding()]
param([switch]$KeepTestSigning)
$ErrorActionPreference = 'Continue'

$p = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'Run from an elevated PowerShell (Run as administrator).'
}

# 1. remove the device node
$dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }
if ($dev) {
  Write-Host "removing device $($dev.InstanceId)"
  & pnputil /remove-device "$($dev.InstanceId)" | Out-Host
} else {
  Write-Host 'no Virtual Display Driver device present'
}

# 2. delete the driver package(s) from the store
$drivers = & pnputil /enum-drivers 2>&1 | Out-String
$published = [regex]::Matches($drivers, '(?im)Published Name:\s*(oem\d+\.inf)\r?\n(?:.*\r?\n)*?\s*Original Name:\s*MttVDD\.inf')
foreach ($m in $published) {
  $oem = $m.Groups[1].Value
  Write-Host "deleting driver package $oem"
  & pnputil /delete-driver $oem /uninstall /force | Out-Host
}

# 3. remove the driver config directory
Remove-Item -Recurse -Force 'C:\VirtualDisplayDriver' -ErrorAction SilentlyContinue

# 4. revert test-signing unless asked to keep it
if (-not $KeepTestSigning) {
  & bcdedit /set testsigning off | Out-Host
  Write-Host 'test-signing turned OFF (reboot to apply).' -ForegroundColor Yellow
} else {
  Write-Host 'left test-signing enabled (-KeepTestSigning).'
}

Write-Host "`nteardown complete." -ForegroundColor Cyan
