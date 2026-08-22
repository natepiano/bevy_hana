# macOS monitor reconnect runbook

This runbook controls the Dell monitor through two macOS Shortcuts while the
`restore_after_reconnect` example records Clerestory's behavior.

Run it when the MacBook Pro is connected to the Dell monitor through USB-C:

```sh
shortcuts run "dell monitor off"
shortcuts run "dell monitor on"
```

Invoking this runbook authorizes those two shortcuts for the current test run.
It does not authorize changing display settings, controlling the pointer,
closing the MacBook lid, moving cables, or controlling another monitor.

## Request to run it

Use this prompt from the workspace root:

> Run `.claude/commands/clerestory_test.md` with the tracked local Dell hardware
> profile. Let the controller run the autonomous partition, preserve its
> artifacts, and return the Dell to the on state. Report the controller's result
> and the cases that still require an operator. Do not recreate the test steps
> as agent commands.

Add `and record the results in the example README` only when the run should
update release evidence.

### Progress reports

Before running commands, count the feasible autonomous physical cases and the
probe processes needed for them. Report both numbers. Count the three scripted
preflight gates separately; do not inflate the physical-case total with unit
tests or lint.

During the run:

- Report each preflight result as `Preflight 1/3`, `2/3`, or `3/3`.
- Report each physical result as `Physical N/T`, where `T` is the announced
  autonomous case total.
- When one probe process covers multiple cases, report each case separately
  after its evidence is available.
- For an activity lasting longer than one minute, report what is still running
  or which trace milestone is still awaited. Do not report a fixed device wait
  as a pass.
- Include the current Dell power state in every physical failure report.
- End with pass, fail, or unavailable for every announced case, all evidence
  paths, and confirmation that the Dell is on.

## What the shortcuts cover

Turning the Dell off and on can exercise these cases without an operator after
the probe is running:

- Same Dell, same USB-C connection.
- Repeated power-loss and return cycles in one application process.
- A rapid off/on request. The trace, not the request timing, determines whether
  macOS delivered separate or combined monitor changes.
- Windowed and borderless automatic return.
- The exclusive-fullscreen automatic-unarmed branch.
- Dell-to-built-in cross-DPI fallback and return, while the MacBook display is
  active.
- Automatic application-controlled restore on the first return and its
  built-in cancellation on the second loss.
- Authenticated move, resize, borderless, windowed, and cancellation commands
  for the managed automatic fallback. These commands affect only the probe
  window and do not focus it or generate input.

The shortcuts do not cover these physical cases:

- Returning the Dell through another USB-C port or dock.
- Substituting another panel on the Dell's connector.
- Two identical panels or duplicate identity evidence.
- Closing and reopening the MacBook lid.
- Rearranging displays in macOS Settings.
- Removing every display at once.
- Returning the Samsung before the Dell. The Samsung is not controlled by
  these shortcuts.

Repeated power cycling exercises repeated monitor loss and return. It does not
replace evidence specifically about changing a dock or cable path.

### Complete macOS case inventory

Keep every case in this runbook even when the current setup cannot perform it.
Run every case whose requirements are available, and record the others as
unavailable with the exact missing hardware or operator action.

| # | Case | With Dell shortcuts | Additional requirement |
| --- | --- | --- | --- |
| 1 | Same Dell, same connection | Autonomous | None after calibration. |
| 2 | Same Dell through another port or dock | Operator-assisted | Move the Dell USB-C cable or dock connection. |
| 3 | Different same-model panel at the same position | Hardware-dependent | A second DELL S3425DW. |
| 4 | Simultaneous duplicate identity evidence | Hardware-dependent | Two panels that expose indistinguishable qualified evidence. |
| 5 | Different panel on the original connector | Operator-assisted | Move the Samsung onto the Dell's connection. |
| 6 | MacBook lid close/open | Operator-assisted | Bluetooth input and a clamshell-capable external display. |
| 7 | Repeated dock or cable churn | Operator-assisted | Repeated physical dock/cable disconnects; shortcut power churn is a separate variant. |
| 8 | Arrangement-only change | Operator-assisted | Change display arrangement in macOS Settings without disconnecting a display. |
| 9 | Zero displays | Conditional | A safe way to keep and observe the process while the lid is closed and both externals are absent. |
| 10 | Non-target-first return | Operator-assisted | Disconnect and reconnect the Samsung independently. |
| 11 | Rapid Dell off/on | Autonomous | None after calibration. |
| 12 | Borderless return | Autonomous when native evidence agrees | Operator judgment only if native fullscreen evidence is unavailable or contradictory. |
| 13 | Exclusive automatic-unarmed branch | Autonomous | None after calibration. |
| 14 | Dell/built-in cross-DPI return | Autonomous | MacBook display active as fallback. |

The managed-window cancellation check is part of the autonomous windowed
probe. The controller uses authenticated commands to move, resize, change mode,
and cancel recovery. The equivalent `B`, `W`, and `Shift+C` keys remain useful
for manual exploration but are not part of the automated result.

## Safety rules

1. Start and end with the Dell on.
2. Before the first power action, confirm both shortcut names with
   `shortcuts list`. Do not run a similarly named shortcut.
3. Do not turn the Dell off until the live trace emits `recovery-ready`.
4. After every off command, the next power command must be the matching on
   command. If the probe, terminal, or a check fails, turn the Dell on before
   diagnosing the failure.
5. Allow 1 second after the off shortcut and 5 seconds after the on shortcut
   for the physical device action. These waits are not proof that macOS
   processed the change; wait for the corresponding trace record as well.
6. Never use `pkill`, a global quit shortcut, AppleScript input, pointer
   automation, or simulated keyboard input. Stop only the probe process that
   this run started.
7. Do not change code to make a physical failure disappear. Preserve the log,
   restore Dell power, and report the observed mismatch first.

## Preflight

Record the current source and hardware state:

```sh
git rev-parse HEAD
shortcuts list
system_profiler SPDisplaysDataType
```

Confirm that the exact shortcut names are present:

```text
dell monitor off
dell monitor on
```

Run the preflight and every available automated case through the controller:

```sh
python3 crates/hana_clerestory/tests/scripts/run_suite.py --automated \
  --hardware-profile crates/hana_clerestory/tests/config/hardware.example.json
```

The controller prints the three preflight results, owns every child process,
runs the six physical cases, restores monitor power in its cleanup path, and
writes one JSON and one Markdown report. The remaining commands below explain
the individual states for diagnosis; they are not additional steps in a normal
automated run.

Before launching a physical run, execute the on shortcut once and confirm the
Dell is listed by macOS:

```sh
shortcuts run "dell monitor on"
sleep 5
system_profiler SPDisplaysDataType
```

## Launching the probe

Use a separate log for every startup mode and retry. First use the bounded
startup command from the example README to read the current monitor inventory.
Select the Dell's cached winit index; do not assume that it is always `1`.

Launch the physical probe with the selected index and one startup mode:

```sh
CLERESTORY_PROBE_MONITOR_INDEX=<dell-index> \
CLERESTORY_PROBE_STARTUP_MODE=<windowed|borderless|exclusive> \
  cargo run -p hana_clerestory --example restore_after_reconnect \
  --features monitor-probe 2>&1 | tee <unique-log-path>
```

Keep the process attached. Wait for its live output instead of starting it in a
detached shell. Capture these initial records before touching monitor power:

- `probe-session`, including startup mode and selected index.
- The Dell monitor entity, scale, and `Verified(MonitorId(...))` identity.
- One `control-association-confirmed` record.
- One `pre-unplug-association` record for each canonical key.
- Exactly one `recovery-ready` record.
- The expected `recovery-accepted` records, or the documented
  `recovery-unarmed` record in exclusive mode.
- One `content-attached` record for each of the four initial windows.

Take a screenshot and inspect it when a visible result is part of the case.
All four initial windows should be on the Dell. If the screenshot does not make
the monitor or fullscreen presentation unambiguous, ask the operator instead of
inferring the result from the component trace.

## Calibration: prove that power-off is a disconnect

The smart plug is useful only if macOS removes the Dell from its monitor list.
Calibrate it once before treating any automated power cycle as physical
reconnect evidence:

1. Wait for `recovery-ready`.
2. Run `shortcuts run "dell monitor off"`.
3. Wait 1 second, then wait for the probe to record the Dell's monitor loss, an
   installed topology revision, and the expected `recovery-pending` records.
4. Confirm macOS no longer lists the Dell.
5. Run `shortcuts run "dell monitor on"`.
6. Wait 5 seconds, then wait for the Dell's monitor connection record and the
   eligible restore results.

If step 3 or 4 fails, run the on shortcut and stop. A dark panel that remains
enumerated over USB-C is not a monitor disconnect and cannot validate recovery.

## Autonomous runs

Use a new process and log for each subsection. Always wait for trace milestones;
the operating system and the Shortcuts service may take different amounts of
time on different cycles. After each off shortcut, wait 1 second before checking
the display state or issuing the on shortcut. After each on shortcut, wait 5
seconds before checking the display state. Continue waiting for the required
trace milestone if macOS has not reported it yet.

### Windowed same-connection and cross-DPI return

1. Launch in `windowed` mode and wait for `recovery-ready`.
2. Turn the Dell off.
3. Wait for one installed disconnect revision and one pending record per armed
   key. Record whether the original entities survived or linked deletion
   removed them.
4. Confirm exactly one replacement for each armed automatic key. Confirm the
   application-controlled and unregistered windows are absent.
5. Record the fallback monitor and scale. With the built-in display active,
   this should exercise scale-1 Dell to scale-2 built-in fallback.
6. Turn the Dell on.
7. Wait for its connection record. Confirm the returned entity may differ but
   the process-local verified `MonitorId` matches the original Dell.
8. Wait for exactly one result for each eligible key. Confirm exactly three
   windows are on the Dell and the unregistered control is absent.
9. Record every `recovery-restored` or `recovery-mismatch` result. A mismatch is
   a failed physical case, not a reason to retry silently.

### Repeated loss and return

Keep one `windowed` probe process alive for three complete off/on cycles.

For every cycle:

1. Turn the Dell off and wait for the installed disconnect revision.
2. Record the replacement entities and confirm at most one replacement per
   armed automatic key.
3. Turn the Dell on and wait for the installed reconnect revision.
4. Record one result per eligible key and confirm no duplicate result.

The example restores the application-controlled key on the first return and
cancels it after its second pending record. Later cycles therefore expect only
the two automatic keys to return. Neither automatic key may emit another
`recovery-accepted` record; both retain their original recovery generation.

### Rapid off/on

1. Launch in `windowed` mode and wait for `recovery-ready`.
2. Run the off shortcut.
3. Wait 1 second, then run the on shortcut.
4. Wait 5 seconds, then wait for the resulting trace records.
5. Report the physical request interval separately from macOS's notification
   interval.

Pass criteria are one replacement per armed automatic key, one restore per
eligible key, and no duplicates. State whether macOS delivered separate
disconnect/reconnect revisions or a combined revision. Do not describe the run
as coalesced unless the trace shows it.

### Borderless return

1. Launch in `borderless` mode and wait for `recovery-ready`.
2. Turn the Dell off and wait for the fallback presentation.
3. Record whether the built-in display leaves its menu bar visible. That is an
   observation, not the return criterion.
4. Turn the Dell on and wait for native fullscreen completion and one restore
   result for the managed automatic key.
5. Confirm the native AppKit fullscreen bit and full-display coverage are both
   true on the returned Dell.

The Bevy `Window` mode alone is not proof that AppKit completed fullscreen
presentation. In an unattended run, missing or contradictory native evidence
makes the case unavailable. An assisted run may ask the operator for visual
judgment.

### Exclusive automatic-unarmed branch

1. Launch in `exclusive` mode and wait for `recovery-ready`.
2. Confirm the managed automatic key has zero accepted registrations and one
   `recovery-unarmed` record naming exclusive-fullscreen return.
3. Turn the Dell off, wait for the installed disconnect, then turn it on.
4. Confirm only the independently armed primary and application-controlled
   keys return. The exclusive managed window must not be reconstructed or
   reported as automatically restored.

## Operator-assisted and hardware-dependent runs

Do not simulate these interactions. Prepare the probe, wait for the named
trace milestone, and ask the operator for one exact physical action at a time.
After they confirm it, continue from the next milestone. Their observation is
the evidence for visible behavior that the trace or screenshot cannot show.

### Same Dell through another port or dock

1. Record the initial MacBook USB-C port or dock path and launch in `windowed`
   mode with the Dell selected.
2. Wait for `recovery-ready`, then ask the operator to disconnect the Dell's
   data connection.
3. Wait for the installed disconnect revision and pending records.
4. Ask the operator to reconnect the same Dell through a different MacBook
   USB-C port or dock path.
5. Confirm the returned monitor has fresh evidence and the same process-local
   verified `MonitorId`, even if its entity and index changed.
6. Confirm every eligible key returns once and no duplicate is created.

The connector path must not become the monitor's identity.

### Different same-model panel at the same position

Run only when a second DELL S3425DW is available.

1. Launch with the original Dell selected and wait for `recovery-ready`.
2. Record both panels' physical labels and serial numbers before moving a
   cable. Do not identify them only by the macOS display name.
3. Ask the operator to remove the original Dell and connect the second Dell
   through the same path and arrangement position.
4. Confirm the substitute panel receives a different verified `MonitorId` from
   the original panel. The original target's keys must remain pending and must
   not return to the substitute.
5. Reconnect the original Dell and confirm the eligible keys return only then.

If the second panel lacks qualified serial evidence, record the unverified
branch instead of claiming distinct verified identity.

### Simultaneous duplicate identity evidence

Run only when two connected panels expose identical qualified evidence.

1. Record the physical labels and the native evidence returned for both
   panels.
2. Confirm Clerestory marks the ambiguous identity as `Unverified`; it must not
   choose a panel by connector, position, entity, index, or enumeration order.
3. Disconnect and reconnect either panel.
4. Confirm no automatic return claims verified continuity through the
   ambiguous evidence.

Two panels with the same model name but distinct qualified serial evidence do
not satisfy this case.

### Different panel on the original connector

1. Launch with the Dell selected while the Samsung is also connected. Record
   both verified identities and wait for `recovery-ready`.
2. Ask the operator to disconnect the Dell, then disconnect the Samsung.
3. Ask the operator to connect the Samsung through the USB-C path previously
   used by the Dell.
4. Confirm the Samsung retains its own identity and the Dell-targeted windows
   stay pending on fallback. No Dell recovery result may be emitted.
5. Ask the operator to reconnect the Dell through the remaining external
   connection.
6. Confirm the eligible Dell-targeted windows return exactly once only after
   the Dell's verified identity reappears.

### MacBook lid close/open

This case targets the built-in display, not the Dell. Keep at least one external
display connected and use Bluetooth input so the application remains usable in
clamshell mode.

1. Discover the built-in display's current cached winit index and launch the
   probe with that index in `windowed` mode.
2. Wait for `recovery-ready` and record the built-in display's verified
   identity, entity, index, and scale.
3. Ask the operator to close the lid.
4. Confirm the process stays alive and the armed automatic windows appear on an
   external fallback display. Record linked deletion, pending records, and
   replacement entities.
5. Ask the operator to reopen the lid.
6. Confirm the built-in display returns with the same process-local verified
   identity and every eligible key returns exactly once.

### Repeated dock or cable churn

Use one process for at least three cycles. This is separate from repeated smart-
plug power churn.

For each cycle:

1. Ask the operator to disconnect the recorded dock or Dell data cable.
2. Wait for one installed disconnect revision and record the automatic
   replacements.
3. Ask the operator to reconnect the same physical path.
4. Wait for one installed reconnect revision and exactly one return per
   eligible automatic key.

Confirm the automatic keys retain their original accepted generations through
all cycles. Record any application-controlled cancellation after the first
cycle separately.

### Arrangement-only change

1. Launch with all four windows on the Dell and wait for `recovery-ready`.
2. Ask the operator to move one display to another relative position in macOS
   Display Settings without disconnecting any display.
3. Confirm the existing monitor and window entities survive.
4. Confirm there is no public monitor connect/disconnect, pending recovery,
   replacement, or restore result.
5. Record whether Clerestory revalidated the existing identities and whether
   the topology revision remained unchanged.

This case verifies that arrangement alone does not begin recovery. It does not
claim live tracking of monitor arrangement metadata.

### Zero displays

Run only when the process can remain alive and its output can be observed from
another machine or a reliable out-of-band terminal. Do not run it when closing
the lid and removing both externals may suspend the MacBook or make recovery
unobservable.

When the prerequisite exists:

1. Launch on an external target and wait for `recovery-ready`.
2. Ask the operator to close the lid and remove or power off every external
   display.
3. Confirm all displays are absent, the process remains alive, and recovery
   state survives even if every window entity is removed.
4. Return one display and confirm the applicable replacement and recovery path.
5. Return the original verified target and confirm its eligible keys complete
   exactly once.

Otherwise record `Unavailable on this setup`; never infer a pass from the lid
or external-monitor rows.

### Non-target-first return

1. Launch with the Dell selected and the Samsung connected. Wait for
   `recovery-ready`.
2. Ask the operator to disconnect the Dell, then disconnect the Samsung.
3. Ask the operator to reconnect the Samsung first.
4. Confirm the Dell-targeted keys stay pending and no Dell restore result is
   emitted.
5. Ask the operator to reconnect the Dell.
6. Confirm every eligible key returns exactly once only after the Dell's
   verified identity is installed.

### Managed automatic cancellation during cycle 2

1. Complete the first windowed disconnect/reconnect cycle.
2. Disconnect the Dell again and wait until the two automatic fallback windows
   are visible.
3. Ask the operator to leave Primary Automatic untouched and focus Managed
   Automatic.
4. Ask them to move and resize it, press `B`, wait for the borderless
   `window-component-changed` record, press `W`, and wait for the windowed
   record.
5. Confirm those changes did not replace its registered Dell target.
6. Ask them to press `Shift+C` once. Confirm exactly one automatic
   `recovery-cancellation-requested` record.
7. Turn the Dell on or ask the operator to reconnect its data cable.
8. Confirm Primary Automatic returns, while Managed Automatic remains on its
   fallback display and performs no exact return.
9. Confirm the application-controlled key's second pending record preceded its
   built-in cancellation and that it creates no second replacement.

### Visual confirmation fallback

Use a screenshot first. Ask the operator only when it cannot establish the
window's display, title-bar state, menu-bar coverage, or duplicate count. Do not
substitute Bevy's `WindowMode` value for visible AppKit fullscreen completion.

## Cleanup and report

Before ending for any reason:

1. Run `shortcuts run "dell monitor on"`.
2. Wait 5 seconds, then confirm macOS lists the Dell again.
3. Stop only the attached probe process, normally with `Ctrl-C` in its terminal
   session if the primary window was not closed.
4. Preserve every log and screenshot path.

Report each run with:

- Source revision, date, Dell connector, selected startup index, and mode.
- Initial and returned Dell entity, index, scale, and process-local
  `MonitorId`.
- Installed topology revisions and the observed operating-system timing.
- Original, fallback, replacement, and returned window entities by key.
- Acceptance, pending, available, cancellation, restored, and mismatch counts.
- Visible result and screenshot path, or the operator's observation.
- Pass, fail, or unavailable, with the exact reason.
- Confirmation that the Dell was returned to the on state.

Never turn an operator-only gap into a pass based on another row or an
automated unit test.
