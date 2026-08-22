# Windows virtual monitor for reconnect testing (VDD)

Windows-only tooling that installs a **virtual monitor** on the ARM64 Windows
VMware guest so the `hana_clerestory` reconnect suite has a display it can
connect and disconnect on demand.

## Why this exists

The Windows test box is a VMware SVGA guest (ARM64 Windows on an Apple-Silicon
Mac). Its virtual displays expose **no EDID**, so every real monitor resolves to
`Unverified` and reconnect recovery never arms. VMware SVGA also cannot be driven
programmatically for topology changes — attempts to detach/reattach a display
left the desktop mirrored and required a VM restart.

The fix is a dedicated **Indirect Display Driver (IDD)**: a user-mode virtual
monitor, independent of VMware SVGA, that

- presents a real **256-byte EDID**, so the monitor comes up
  `Verified(MonitorId(..))` and recovery arms, and
- can be **connected / disconnected deterministically** by enabling / disabling
  its device — safe, because a user-mode (Session 0) IDD cannot disturb the real
  display topology.

Upstream driver: [VirtualDrivers/Virtual-Display-Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver)
(MTT), pinned to release **25.7.23**, ARM64 build. Device-node tool:
[nefarius/nefcon](https://github.com/nefarius/nefcon), pinned to **v1.17.40**.

## Prerequisites (set once per VM)

1. **ARM64 Windows 11** VMware Fusion guest.
2. **Secure Boot DISABLED** in the VM settings (VM powered off to change it).
   Test-signing cannot be enabled while Secure Boot is on.
3. An internet connection (setup downloads the two pinned releases).

## One-time install (re-creating the VM)

Run from an **elevated** PowerShell:

```powershell
# Phase 1: enables test-signing, then asks you to reboot.
powershell -ExecutionPolicy Bypass -File setup.ps1
# ... reboot the VM (you'll see a "Test Mode" watermark) ...
# Phase 2: downloads, trusts the catalog publisher, installs the driver.
powershell -ExecutionPolicy Bypass -File setup.ps1
```

Why the reboot: ARM64 driver-signature enforcement (24H2+) rejects the
community-signed (SignPath / GlobalSign, not Microsoft-attestation) catalog until
test-signing is on. `setup.ps1` trusts the publisher **and** enables test-signing;
both are needed.

When it finishes, the virtual monitor is installed and enabled. Confirm with:

```powershell
powershell -ExecutionPolicy Bypass -File inventory.ps1   # expect one {"name":"MTT1337"} entry
```

### Keep it disabled at rest

An enabled virtual monitor distorts the VMware guest's cursor mapping (a
horizontal offset) while you use the desktop manually. Leave it **off** except
during a test run:

```powershell
powershell -ExecutionPolicy Bypass -File disable.ps1
```

The reconnect suite enables and disables it automatically, so the offset only
appears mid-run (when you are not driving the mouse) and clears the instant the
run disables it. If the offset ever lingers, toggle the VM's fullscreen
(Fusion: Ctrl+Cmd+Return out and back) to re-sync the pointer.

## Scripts

| Script          | Role                                                              |
|-----------------|------------------------------------------------------------------|
| `setup.ps1`     | One-time install (phase-aware; downloads pinned releases).        |
| `teardown.ps1`  | Remove device + driver and revert test-signing.                  |
| `enable.ps1`    | Connect the virtual monitor — reconnect harness `power_on`.      |
| `disable.ps1`   | Disconnect the virtual monitor — reconnect harness `power_off`.  |
| `inventory.ps1` | Emit present-monitor JSON — reconnect harness `inventory`.       |
| `vdd_settings.xml` | Driver config (one monitor, default EDID) copied to `C:\VirtualDisplayDriver`. |

`enable`/`disable`/`inventory` require Administrator; run the whole reconnect
suite from **one elevated shell** so the toggles never prompt for UAC.

## How the reconnect harness uses it

`run_reconnect.py` drives any rig through a `HardwareProfile` of three commands.
The Windows profile (`tests/config/hardware.windows-vm.local.json`, git-ignored
and never committed) points `power_off` / `power_on` / `inventory` at
`disable.ps1` / `enable.ps1` / `inventory.ps1`, with:

- `target_matcher`: `"MTT1337"` — the VDD monitor's device id
- `inventory_name_field`: `"name"`
- `probe_monitor_matcher`: `physical_size: [800, 600]` — the VDD's default mode,
  unique among the real monitors, so it is matched by size, never by the volatile
  `\\.\DISPLAYn` name.

Enabled → inventory reports one `MTT1337` (count 1, connected); disabled → none
(count 0, disconnected).

## Running the reconnect test

From an **elevated** shell (the enable/disable toggles need admin), with the VDD
installed:

```
python crates/hana_clerestory/tests/scripts/windows_vdd/run_reconnect_test.py
```

It builds the probes, enables the virtual monitor, runs one windowed
disconnect→reconnect cycle (asserting the window returns to the same verified
`MonitorId`), disables the monitor, and prints `PASSED` / `FAILED`.

The same case also runs inside the full suite (enable the VDD first so discovery
finds it):

```
python crates/hana_clerestory/tests/scripts/run_suite.py --automated ^
    --hardware-profile crates/hana_clerestory/tests/config/hardware.windows-vm.local.json
```

## Software rendering (WARP) notes

The VMware guest has no GPU, so wgpu falls back to the **WARP** software D3D
rasterizer, which recurses deeply and slowly while reconfiguring surfaces during a
reconnect. Two accommodations make the test pass, both already wired in and inert
on hardware renderers:

- **Stack** — `crates/hana_clerestory/build.rs` reserves a 256 MB stack for the
  example executables on Windows-MSVC (WARP worker threads inherit it) so the
  deep-but-finite recursion does not overflow a 1 MB default.
- **Timeouts** — `run_reconnect.py` uses longer reconnect timeouts on Windows.

On a GPU-capable host (real hardware or GPU passthrough) neither is needed — the
hardware D3D12 driver handles the reconnect quickly.

## Teardown

```powershell
powershell -ExecutionPolicy Bypass -File teardown.ps1   # add -KeepTestSigning to leave test mode on
```

## macOS is unaffected

Everything here is Windows-only. The macOS reconnect profile (`macos.json`,
driven by `shortcuts` / `system_profiler`) is untouched, and the reconnect engine
is cross-platform — it simply runs whichever profile it is given.
