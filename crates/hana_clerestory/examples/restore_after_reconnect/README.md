# Restore after reconnect

This probe demonstrates Clerestory's kernel-backed display recovery. It creates a primary window,
an automatically recovered managed window, an application-controlled managed window, and an
unmanaged control window.

Each managed window receives one kernel `Binding` when Clerestory can resolve its exact reported
display endpoint. A binding retains its role, endpoint, driver, policy, requested
`EstablishedWindowPlacement`, last-known-good state, and deadline. The window driver starts the
native move asynchronously; the kernel owns attempt identity, authorization, retry decisions, and
completion.

## Recovery behavior

The automatic managed window uses `RecoveryPolicy::ReapplyOnReturn`. If its reported display
departs, the kernel waits for that exact display. Clerestory may settle the window on another live
display while the original is absent, but its private per-role recovery state causes a return to the
original display when it is verified again.

The application-controlled managed window carries `ManagedWindowReapplyOnRequest`. When its exact
display becomes available after a departure, the probe targets the retained binding entity with
`ReapplyConfiguration`. That event only discharges the kernel's outstanding departure debt; it
does not select a display or offer a general request to move a window.

The control window has no binding. It is present solely for operator commands and cannot cause
Clerestory to authorize a display operation.

## Observations

The probe records kernel `RoleAvailable`, `RoleAwaiting`, `RoleStateChanged`,
`IdentityQuestionRaised`, and `AttemptFinished` events, plus Clerestory's public
`WindowRestored` and `WindowRestoreMismatch` projections. Identity questions and stopped roles
remain externally observable: the probe neither adopts a candidate nor restarts a stopped role.

The BRP surface reports live monitor descriptors, retained binding mirrors, and the ordered event
records for the controller. It never chooses a monitor by enumeration index; a missing or ambiguous
identity remains unresolved.

## Running

Use the project controller command to exercise the live probe after both examples compile. The
delegate validates only compilation; controller orchestration owns the live display session.
