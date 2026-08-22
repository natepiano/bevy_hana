# Monitor inventory for the reconnect harness. Windows-only. Emits JSON:
#   {"monitors":[{"name":"MTT1337"|"DEFAULT_MONITOR"|...,"status":"OK","instance":"..."}]}
# The harness counts entries whose "name" == the profile's target_matcher ("MTT1337").
# The VDD monitor (MTT1337) is only PRESENT while the device is enabled, so:
#   enabled  -> one MTT1337 entry  -> count 1 (connected)
#   disabled -> no MTT1337 entry   -> count 0 (disconnected)
$ErrorActionPreference = 'SilentlyContinue'
$mons = Get-PnpDevice -Class Monitor -PresentOnly
$list = @()
foreach ($m in $mons) {
  $parts = $m.InstanceId -split '\\'
  $name = if ($parts.Length -ge 2) { $parts[1] } else { $m.InstanceId }
  $list += [pscustomobject]@{ name = $name; status = "$($m.Status)"; instance = $m.InstanceId }
}
([pscustomobject]@{ monitors = $list }) | ConvertTo-Json -Depth 5 -Compress
