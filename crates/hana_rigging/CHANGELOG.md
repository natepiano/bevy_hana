# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-05

### Added

- `DeviceNameDisambiguation::Unshared`: no other unit reports the same name, so the name alone tells the unit apart.
- The kernel centrally maps device-access failures to reconsideration, clearance
  waits, faults or stopped apply work. Reflected RoleStatus exposes actionable
  waits, retry progress and the last started-attempt ending; this policy method
  is internal.
- `DriverLedger` and `DriverRecords` retain driver attempt and session records with their kernel completion and lease authorities.
- `DeviceOpens`, `SharedDeviceOpens`, and `DeviceReaderToken` share one physical device open across roles. The open closes when its last reader leaves.
- `OutputProof`, `OutputConfirmation`, and `OutputProofPlugin` track output confirmation separately from session establishment. Confirmation expires when the source stops confirming output.
- `FlowExpectation`, `ContinuousFlowExpectation`, and `EstablishedFlowView` track the first datum and gaps between data arrivals. `DeviceTransport` supplies typed observations.
- `ReporterHealth` exposes retained discovery outcomes, progress, timing, and capability projection failures through reflected, serializable views.
- `KernelRolePresentation` and `RolePresentationView` provide one kernel-derived state for scanning, connection, data flow, and unavailable devices.
- `DeviceAccessClass` classifies provider failures. `DeviceAccessError` adds `Unresponsive`, `Releasing`, and `Unsupported` failures.

### Changed

- Requires Bevy 0.20.
- **Breaking:** authored inventory entries now carry a validated, device-class-neutral name.
  `ConfiguredDevice` gains `name: ConfiguredDeviceName`; `AuthoredDeviceName` distinguishes names
  reported by a unit, measured domain text, and operator assignments. The supporting
  `ReportedDeviceName`, `DeviceNameDisambiguationText`, and `OperatorAssignedDeviceName` types
  reject empty text and control characters at construction and deserialization.
- **Breaking:** `EndpointDriver` requires `type Target` and `resolve_target(...)
  -> TargetResolution<Self::Target>`. Only `Reached(target)` authorizes
  `start_apply`; `Deferred(TargetWait)` names the reporter or application
  condition that must change before an apply can start.
- **Breaking:** `DeviceKind::HidPanel` is renamed to
  `DeviceKind::ControlSurface`; old stored `HidPanel` values remain readable
  through a serde alias. DeviceKind is exhaustive, so source matches must use
  the new variant.
- **Breaking:** `add_endpoint_driver` returns `EndpointDriverRegistration<Configuration>`. Register roles through `BindingAuthoring`, `BindingPolicy`, and `BindingRegistration`, or `register_binding` and `replace_binding`. Registration returns the role entity.
- **Breaking:** `EndpointDriver` replaces `capture` and `poll` with completion and session callbacks. `AttemptCompletion`, `DriverCompletion`, and `SessionLease` report completion, configuration changes, and session loss.
- **Breaking:** Driver cleanup callbacks receive `DriverCleanupRoleEntity`. Release hardware even when the role entity no longer exists.
- **Breaking:** `RoleState` becomes private. Read `RoleStatus::view()`, `RoleEndpoint`, and `DeviceStatus` for role state, endpoint identity, and device availability.
- **Breaking:** `ReporterRegistration::required` and `ReporterRegistration::optional` require `first_complete_set_bound`. `ReporterDeferral` names incomplete discovery prerequisites. Unsupported reporters stop until an explicit request or enable.
- **Breaking:** `Capabilities::add` and `Capabilities::with` require `Component + Reflect + PartialEq`. Register reflected component metadata for each capability. Reporter health reports rejected capability projections.
- **Breaking:** `RiggingLimits` gains `departure_grace`. Confirmed device departure retains availability evidence during this interval before retirement.

### Removed

- **Breaking:** `LiveRoleChanged` and `RetiredRoleChanged` replace `RoleStateChanged`, `RoleAvailable`,
  `RoleAwaiting`, and `BindingRetired`. `LiveRoleChange::AttemptEnded` reports live attempt endings.
  `RegistrationAttemptEnded` reports endings without a live role entity. `HardwareInventoryChanged`
  replaces inventory change events. Use `SessionLease::report_loss` instead of `SessionLossReports`.
- **Breaking:** Kernel scheduling and lifecycle internals, including `DiscoveryStatus`, `Attempts`, and
  `BindingGeneration`, are no longer public. Several low-level helpers now require `test-support`.

### Fixed

- Replacing a binding no longer cancels the replacement apply through cleanup for its predecessor.
- Driver-reported contention and blocking remain recorded until a fresh reporter batch arrives. A fresh report can reopen a `RetryOn::NewRevision` retry.

## [0.1.0] - 2026-08-22

### Added

- Initial release. A Bevy kernel for durable device identity, presence,
  availability, and recovery policy. Hardware providers perform I/O and report
  their full device set to this crate; the crate does not enumerate or operate
  hardware itself.
- `DeviceKey` durable identity, whose `DeviceIdSource` records in the type
  whether a value was `Reported` by the unit, `Authored` by a human, or
  `Synthesized` from descriptors. All three can authorize driving a device once
  exact identity, availability, and claim checks succeed.
- Checked identity newtypes — `SchemeName`, `ReportedId`, `AuthoredId`,
  `Digest` — that no constructor, deserializer, or reflection path can bypass,
  and a `RegisteredSchemes` allowlist enforced when reports arrive.
- `DeviceReporter` and `EndpointDriver` provider contracts, registered through
  `RiggingAppExt`. `discover` receives no `World`, and `start_apply` accepts
  only an `ApplyPermit` the kernel issued for one specific unit.
- Discovery scheduling with on-demand, event-driven, and periodic cadences,
  bounded job concurrency, coalesced reruns, progress reporting, and startup
  readiness gating for optional reporters.
- Reconciliation that merges co-reporting providers into one device set, folds
  presence down reported parent chains, applies a staleness lease that
  withdraws evidence without asserting absence, and produces an
  `IdentityVerdict` per device.
- `Bindings`: application-stable `RoleKey`s bound to `DeviceEndpoint`s, each
  carrying its own `RecoveryPolicy`, `RetryOn`, `OnAbort`, and `OnSessionLoss`
  policy, plus `HardwareInventory` for authored offline devices.
- A kernel-driven apply path — authorization, dispatch, polling, deadlines with
  bounded overrun, abort on invalidation, and a stop after three consecutive
  failures — where every poll re-checks that the attempt still targets the same
  physical unit.
- `IdentityDecisions`, which raises a question for a human when a same-kind
  unit occupies a departed device's slot instead of adopting it silently.
- Read-only entity mirrors of every kernel fact, with application-observable device, role, attempt,
  identity, and discovery events.
- `RiggingPlugin` and the `RiggingSystems` ordering sets —
  `Collect → Reconcile → Prepare → SessionLoss → Apply`.
