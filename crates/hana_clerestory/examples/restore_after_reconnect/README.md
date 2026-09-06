# Restore after reconnect

This probe demonstrates Clerestory's kernel-backed display recovery. It creates a primary window,
an automatically recovered managed window, an application-controlled managed window, a
restore-only managed window, and an unmanaged control window.

Each managed window receives one kernel `Binding` when Clerestory can resolve its exact reported
display endpoint. A binding retains its role, endpoint, driver, policy, requested
`EstablishedWindowPlacement`, last-known-good state, and deadline. The window driver starts the
native move asynchronously; the kernel owns attempt identity, authorization, retry decisions, and
completion.

## Recovery behavior

The primary window and automatic managed window opt in with `RecoverOnReturn`, which authors
`RecoveryPolicy::ReapplyOnReturn`. If their reported display
departs, the kernel waits for that exact display. Clerestory may settle the window on another live
display while the original is absent, but its private per-role recovery state causes a return to the
original display when it is verified again.

The application-controlled managed window carries `RecoverOnRequest`. When its exact
display becomes available after a departure, the probe targets the retained binding entity with
`ReapplyConfiguration`. That event only discharges the kernel's outstanding departure debt; it
does not select a display or offer a general request to move a window.

The restore-only managed window carries neither marker, so it authors `RecoveryPolicy::Forget`.
Its saved position is restored at launch, but after fallback it stays on the live display when the
saved display returns.

The control window has no binding. It is present solely for operator commands and cannot cause
Clerestory to authorize a display operation.

## Startup monitor selection

The probe requests monitor index 1 by default. If index 1 is unavailable, it selects the lowest
live index and still opens all five scenario windows there. The fallback warning logs both the
requested index and the active index. Trace records and the BRP snapshot's
`selected_monitor_index` field report the active index, so a controller observes the monitor the
windows actually use.

## Observations

The probe records kernel `LiveRoleChanged` and `IdentityQuestionRaised` events, plus Clerestory's public
`WindowRestored` and `WindowRestoreMismatch` projections. Identity questions and stopped roles
remain externally observable: the probe neither adopts a candidate nor restarts a stopped role.

The BRP surface reports live monitor descriptors, retained binding mirrors, and the ordered event
records for the controller. The startup index only selects where the probe windows first open;
recovery never derives durable identity from that index. A missing or ambiguous identity remains
unresolved.

## Running

Use the project controller command to exercise the live probe after both examples compile. The
delegate validates only compilation; controller orchestration owns the live display session.
