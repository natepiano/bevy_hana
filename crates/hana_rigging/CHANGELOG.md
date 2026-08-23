# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-22

### Added

- Initial release. A Bevy kernel for durable device identity, presence,
  availability, and recovery policy. Hardware providers perform I/O and report
  their full device set to this crate; the crate does not enumerate or operate
  hardware itself.
- `DeviceKey` durable identity, whose `DeviceIdSource` records in the type
  whether a value was `Reported` by the unit, `Authored` by a human, or
  `Synthesized` from descriptors. Only the first two can authorize driving a
  device; a synthesized key may restore saved configuration and nothing more.
- Checked identity newtypes — `SchemeName`, `ReportedId`, `AuthoredId`,
  `Digest` — that no constructor, deserializer, or reflection path can bypass,
  and a `RegisteredSchemes` allowlist enforced when reports arrive.
- `DeviceReporter` and `EndpointDriver` provider contracts, registered through
  `RiggingAppExt`. `discover` receives no `World`, and `start_apply` accepts
  only an `ApplyPermit` the kernel minted for one specific unit.
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
- Read-only entity mirrors of every kernel fact, with the derived event set
  (`DeviceArrived`, `DeviceDeparted`, `PresenceChanged`, `ClaimChanged`,
  `IdentityChanged`, `RoleStateChanged`, `AttemptFinished`, and the rest).
- `RiggingPlugin` and the `RiggingSystems` ordering sets —
  `Collect → Reconcile → Prepare → SessionLoss → Apply`.
