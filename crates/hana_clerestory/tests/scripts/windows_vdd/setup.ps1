<#
.SYNOPSIS
  Install the Virtual Display Driver (VDD) IDD virtual monitor used by the
  hana_clerestory reconnect test suite on the ARM64 Windows VMware guest.

.DESCRIPTION
  Windows-only, ARM64. Reproduces the whole setup on a fresh VM. Run elevated.
  It is phase-aware: driver-signature enforcement on ARM64 24H2+ requires
  test-signing, which needs a reboot. First run enables test-signing and stops;
  after you reboot, run it again to finish the install.

  Pinned upstream releases (both signed):
    VirtualDrivers/Virtual-Display-Driver  25.7.23  (ARM64 UMDF IddCx driver)
    nefarius/nefcon                        v1.17.40 (root device-node tool)

  Prerequisite you must set once in VMware Fusion (VM powered off):
    Secure Boot DISABLED  (otherwise test-signing cannot be enabled).

.NOTES
  Reversible: teardown.ps1 removes everything and turns test-signing back off.
#>
[CmdletBinding()]
param(
  [string]$WorkDir = (Join-Path $env:TEMP 'vdd_setup')
)
$ErrorActionPreference = 'Stop'
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

# --- pinned upstream releases -------------------------------------------------
$VddTag    = '25.7.23'
$VddAsset  = 'VirtualDisplayDriver-ARM64.Driver.Only.zip'
$VddUrl    = "https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/$VddTag/$VddAsset"
$NefTag    = 'v1.17.40'
$NefAsset  = 'nefcon_v1.17.40.zip'
$NefUrl    = "https://github.com/nefarius/nefcon/releases/download/$NefTag/$NefAsset"
$Hwid      = 'Root\MttVDD'
$ClassGuid = '4d36e968-e325-11ce-bfc1-08002be10318'

function Assert-Admin {
  $p = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
  if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script from an elevated PowerShell (Run as administrator).'
  }
}

function Get-OsArch {
  # OSArchitecture is reliable across the x64 emulation the inbox PowerShell runs under.
  return (Get-CimInstance Win32_OperatingSystem).OSArchitecture
}

function Test-SecureBootOn {
  $v = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\State' -EA SilentlyContinue).UEFISecureBootEnabled
  return ($v -eq 1)
}

function Test-TestSigningOn {
  $out = & bcdedit /enum '{current}' 2>&1 | Out-String
  return ($out -match '(?im)^\s*testsigning\s+Yes\s*$')
}

function Save-Download($url, $path) {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  Write-Host "  downloading $url"
  Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
}

Assert-Admin

$arch = Get-OsArch
Write-Host "OS architecture: $arch"
if ($arch -notmatch 'ARM') {
  throw "This setup targets ARM64 Windows (VMware on Apple Silicon). Detected: $arch. Adjust `$VddAsset for your architecture."
}

if (Test-SecureBootOn) {
  throw 'Secure Boot is ON. Power off the VM, disable Secure Boot in VMware Fusion settings, boot, and re-run. (Test-signing is blocked while Secure Boot is enabled.)'
}

# --- phase 1: test-signing (needs a reboot) -----------------------------------
if (-not (Test-TestSigningOn)) {
  Write-Host "`nEnabling test-signing (ARM64 driver-signature enforcement requires it)..." -ForegroundColor Yellow
  & bcdedit /set testsigning on | Out-Host
  Write-Host "`n==> REBOOT the VM now, then run setup.ps1 again to finish the install." -ForegroundColor Yellow
  return
}
Write-Host 'test-signing: enabled'

# --- phase 2: download + install ---------------------------------------------
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
$vddZip = Join-Path $WorkDir $VddAsset
$nefZip = Join-Path $WorkDir $NefAsset
Save-Download $VddUrl $vddZip
Save-Download $NefUrl $nefZip

$vddDir = Join-Path $WorkDir 'vdd'
$nefDir = Join-Path $WorkDir 'nefcon'
Remove-Item -Recurse -Force $vddDir, $nefDir -EA SilentlyContinue
Expand-Archive -Path $vddZip -DestinationPath $vddDir -Force
Expand-Archive -Path $nefZip -DestinationPath $nefDir -Force

$driverDir = Join-Path $vddDir 'VirtualDisplayDriver'
$inf     = Join-Path $driverDir 'MttVDD.inf'
$cat     = Join-Path $driverDir 'mttvdd.cat'
$nefconc = Join-Path $nefDir 'ARM64\nefconc.exe'
foreach ($f in @($inf, $cat, $nefconc)) { if (-not (Test-Path $f)) { throw "expected file missing after extract: $f" } }

# Driver config (single monitor) -> the location the driver reads.
New-Item -ItemType Directory -Force -Path 'C:\VirtualDisplayDriver' | Out-Null
Copy-Item (Join-Path $ScriptDir 'vdd_settings.xml') 'C:\VirtualDisplayDriver\vdd_settings.xml' -Force
Write-Host 'config: C:\VirtualDisplayDriver\vdd_settings.xml'

# Trust the SignPath Foundation catalog publisher (Authenticode) so pnputil accepts it.
Write-Host "`nTrusting driver catalog publisher..."
Add-Type -AssemblyName System.Security
$cms = New-Object System.Security.Cryptography.Pkcs.SignedCms
$cms.Decode([IO.File]::ReadAllBytes($cat))
$i = 0
foreach ($c in $cms.Certificates) {
  $cer = Join-Path $WorkDir "vdd_cert_$i.cer"
  [IO.File]::WriteAllBytes($cer, $c.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert))
  & certutil -addstore -f TrustedPublisher "$cer" | Out-Null
  & certutil -addstore -f Root "$cer" | Out-Null
  Write-Host ("  trusted: {0}" -f $c.Subject)
  $i++
}

# Create the root device node, then install/bind the driver with native pnputil.
$env:PROCESSOR_ARCHITECTURE = 'ARM64'
Write-Host "`nCreating device node $Hwid ..."
& $nefconc --create-device-node --hardware-id $Hwid --class-name Display --class-guid $ClassGuid --no-duplicates | Out-Host
Write-Host "`nInstalling driver (pnputil)..."
& pnputil /add-driver "$inf" /install | Out-Host
if ($LASTEXITCODE -ne 0) { throw "pnputil /add-driver failed with exit $LASTEXITCODE" }
Start-Sleep -Seconds 3

# --- verify -------------------------------------------------------------------
$dev = Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.FriendlyName -match 'Virtual Display' }
if (-not $dev) { throw 'install finished but the Virtual Display Driver device is not present.' }
Write-Host ("`ndevice: {0}  status={1}  problem={2}" -f $dev.FriendlyName, $dev.Status, $dev.Problem)
$mtt = Get-ChildItem 'HKLM:\SYSTEM\CurrentControlSet\Enum\DISPLAY\MTT1337' -EA SilentlyContinue
if ($mtt) { Write-Host 'EDID: MTT1337 monitor present (Verified identity expected).' -ForegroundColor Green }

Write-Host "`nDONE. The VDD is installed and enabled. Disable it at rest with disable.ps1 (keeps the mouse normal); the reconnect suite toggles it automatically." -ForegroundColor Cyan
