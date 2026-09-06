# hana_rigging

[![Crates.io](https://img.shields.io/crates/v/hana_rigging.svg)](https://crates.io/crates/hana_rigging)
[![Downloads](https://img.shields.io/crates/d/hana_rigging.svg)](https://crates.io/crates/hana_rigging)
[![docs.rs](https://docs.rs/hana_rigging/badge.svg)](https://docs.rs/hana_rigging)
[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/natepiano/bevy_hana/tree/main/crates/hana_rigging#license)

A Bevy kernel for durable device identity, presence, availability, and recovery
policy. Hardware providers perform I/O and report their full device set to this
crate; the crate does not enumerate or operate hardware itself.

> **Work in progress.** This crate is in active development (v0.1.0) and not
> subject to semver stability guarantees. APIs will change without notice
> between commits. Do not depend on this in production code yet.

**Rigging** — the ropes, pulleys, and counterweights above a stage that hold the
lights, screens, and scenery, and let an operator move any of them on cue. The
rigging does not produce the light or the image; it is what everything hangs
from, and what makes a fixture addressable by name instead of by where it
happens to be hanging tonight.

## The problem

Devices are addressed by whatever the platform hands you: monitor index 1,
camera 0, "the nearest display". Those handles move. Unplug a projector and plug
in a different one, and index 1 now points at another physical unit — so the
saved window layout, the camera feed, or the DMX patch quietly drives the wrong
device. Every subsystem then grows its own hand-rolled reconnect logic, and each
one guesses differently about what "the same device" means.

`hana_rigging` makes the answer exact and shared. Identity is match-or-nothing:
a saved key that matches no live unit yields nothing, never a fallback. Nothing
enters service without an explicit authorization the kernel issued for that one
physical unit.

## What it does

- **Durable identity** — `DeviceKey` survives restarts and replugs, and records
  in its own type how much the identity can be trusted
- **Presence and reachability** — present, absent, or unreachable-for-a-duration,
  merged across every provider that reports the same unit
- **Discovery scheduling** — on-demand, event-driven, or periodic scans, with
  bounded concurrency, coalesced reruns, and startup readiness gating
- **Roles and bindings** — application-stable `RoleKey`s bound to device
  endpoints, so "the presenter display" outlives the display it currently means
- **Recovery policy** — per binding, decide whether a saved configuration is
  forgotten, retained, reapplied on request, or reapplied when the unit returns
- **Authorized applies** — drivers configure hardware only through kernel-issued
  permits, on attempts the kernel starts, polls, deadlines, and retires
- **Identity adjudication** — when a replacement unit occupies a departed one's
  slot, the kernel raises a question for a human instead of guessing
- **Entity mirror** — every kernel fact is projected onto Bevy entities as
  read-only components, so change detection and remote inspection just work

## Trust is part of the identity

A `DeviceKey` carries where its value came from, and that determines what the
key is allowed to authorize:

| Source | Where it comes from | What it can do |
|---|---|---|
| `Reported` | The unit published it — an EDID serial, a CoreAudio UID | Drive output |
| `Authored` | A human assigned it, for units that report nothing | Drive output |
| `Synthesized` | Derived from an exact unit-or-location match | Drive output after one live unit matches |

That distinction is the point. A webcam with no serial gets a synthesized key,
but it can drive output only when one live unit matches that exact key. Ambiguous
matches authorize nothing, while displaced and wrong-unit questions stay with
the identity-decision machinery.

Reconciliation turns a key plus live evidence into an `IdentityVerdict`:
`Proven`, `Presumed`, `Authored`, `Displaced` (a same-kind unit took the
slot — a human decides), `WrongUnit`, or `Unverified`.

## Usage

Add the plugin, register the identity schemes your providers are allowed to
report, and register the reporters and drivers that touch hardware:

```rust ignore
use bevy::prelude::*;
use hana_rigging::prelude::*;

let mut app = App::new();
app.add_plugins(MinimalPlugins)
    .add_plugins(RiggingPlugin)
    .register_device_scheme(SchemeName::new("edid")?);

let driver = app.add_endpoint_driver(WindowDriver::default());

let reporter = app.add_device_reporter(
    MonitorReporter::default(),
    ReporterRegistration::required(
        DiscoveryCadence::EventDriven { backstop: Duration::from_secs(30) },
        ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
            CoveredDeviceIdentitySpace::ReportedScheme {
                kind:   DeviceKind::Display,
                scheme: SchemeName::new("edid")?,
            },
        )),
    std::time::Duration::from_secs(10),
    ),
);
```

A reporter hands back its **whole current device set** each scan, never a delta.
That is what lets the kernel conclude a device is genuinely gone rather than
merely unmentioned this frame:

```rust ignore
impl DeviceReporter for MonitorReporter {
    fn discover(&mut self) -> DiscoveryWork {
        DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_world: &mut World| {
            DeviceScan::Complete(enumerate_monitors())
        }))
    }
}
```

`discover` receives no `World` on purpose — it is the boundary that keeps
enumeration out of the kernel's own state.

### Implementing an endpoint driver

Every `EndpointDriver` names the live handle it needs as `type Target`. Before
starting each apply, the kernel calls
`resolve_target(...) -> TargetResolution<Self::Target>`:

- `Reached(target)` proves that the driver can reach the target. This is the
  only outcome that proceeds to `start_apply`, and the kernel passes that exact
  `target: Self::Target` into the call.
- `TargetDetached` means the application entity the driver operates is not
  attached for this lifetime, even if the physical device is healthy. No apply
  starts.
- `DeviceUnavailable(error)` retains the classified `DeviceAccessError` for
  kernel policy. No apply starts.

`ApplyStart` no longer exists; these `TargetResolution` variants carry its
former outcomes. `start_apply` returns `()` and reports later progress through `poll`. The
kernel uses `DeviceAccessError::apply_failure_disposition()` for the same
class-to-policy decision whether the error came from target resolution or a
started apply: `Reconsider`, `AwaitClearance`, or `Fault`. A reconsidered
resolution failure rolls back the unstarted attempt and emits no attempt-ending
event. The role's reflected `RoleStatus` retains the actionable wait, retry run,
and last started-attempt ending for operator diagnostics.

Bind an application-stable role to a device endpoint, and state every policy
explicitly. There are no implicit recovery defaults hiding in the kernel:

```rust ignore
app.world_mut().resource_mut::<Bindings>().register(Binding {
    role:     RoleKey::new("presenter-display")?,
    endpoint: DeviceEndpoint { device: saved_key, id: EndpointId::Whole },
    driver,
    recovery: RecoveryPolicy::ReapplyOnReturn,
    retry:    RetryOn::NewRevision,
    on_abort: OnAbort::Revert,
    on_loss:  OnSessionLoss::Recreate,
    requested: RequestedConfiguration::new(WindowPlacement { left: 0, top: 0 }),
    last_known_good: LastKnownGoodConfiguration::default(),
    apply_deadline:  ApplyDeadline::ProcessDefault,
    flow_expectation: FlowExpectation::NotMonitored,
})?;
```

Then read `RoleStatus` off the mirrored entities, or observe `DeviceArrived`, `DeviceChange`,
`IdentityChanged`, `LiveRoleChanged`, `RetiredRoleChanged`, `RegistrationAttemptEnded`, and the
discovery events.

The `rigging_kernel` example is a complete headless run: two reporters pushing
overlapping scans that agree on one panel and disagree about everything else, an
authored inventory entry, one apply attempt driven to a terminal outcome, and a
provoked departure. It ships with the crate — `cargo run --example rigging_kernel`.

A second example, `identity_decision`, walks the displaced-unit adjudication
path end to end. It lives in the [source repository](https://github.com/natepiano/bevy_hana/tree/main/crates/hana_rigging/examples)
rather than the published package, because it drives the kernel with a scripted
device harness that is not itself published.

## Schedule

`RiggingPlugin` chains five ordered sets in `Update`, exposed as
`RiggingSystems` so integration crates can place their own systems precisely:

```text
Collect → Reconcile → Prepare → SessionLoss → Apply
```

`Prepare` is deliberately empty — it is the one interval where identity is
settled but no apply has started, which is where consumer systems build the
configuration they want applied.

## Design rules

These are enforced, not aspirational:

- The kernel performs no I/O and knows nothing about any specific device kind
- Exact match or nothing — no nearest-monitor, no first-camera, no tolerance
- Scans are whole sets; absence is always a named variant, never `Option`
- The kernel never silently puts a device in service; the default policy is
  `Forget`
- Resources are authoritative, entity components are read-only mirrors
- `resolve_target` must return `Reached` before `start_apply` runs;
  `start_apply` returns immediately, and every poll re-validates that the
  attempt still targets the same physical unit

## Version Compatibility

| Version            | Bevy |
|--------------------|------|
| hana_rigging 0.1.0 | 0.19 |

## License

`hana_rigging` is free, open source and permissively licensed!
Except where noted (below and/or in individual files), all code in this repository is dual-licensed under either:

* MIT License ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

at your option.

### Your contributions

Unless you explicitly state otherwise,
any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license,
shall be dual licensed as above,
without any additional terms or conditions.
