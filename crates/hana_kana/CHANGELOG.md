# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-08-22

### Added

- Add `Angle`, a finite signed angular-displacement type that preserves positive
  and negative multi-turn radians without modulo normalization
- Add exact `SequenceTime`, embedded `SequencePlayback`, compact non-allocating
  boundary traversal, and the chained arbitration, movement-production,
  movement-application, and sequence-evaluation system sets for retained domain
  sequences
- Add the shared movement contract: opaque `SequencePosition`, complete
  `SequenceMovement` with signed whole repetitions and compact non-allocating
  `RangeCrossings`, and immutable `SequenceStages` scope resolution
- Add producer-colocated `SequenceDriver` with required `SequenceEvaluation`,
  single-selection arbitration, one explicit `SequenceDriverTakeover`,
  freshness-gated restoration of the sole displaced producer, and observable
  claim, selection, release, and command rejections
- Add the targeted `SequenceCommand` vocabulary and the `SequenceCommands`
  system parameter as the only path that mutates local sequence position
- Add `SequenceCommandResponse::NoRetainedSequence`, so a command issued
  against an entity that carries no retained playback is distinguishable from
  one an owner rejected
- Add authored easing: `Easing` over stock Bevy easings and immutable owned
  `EasingCurve` values; opaque `EasingInput`, `EasingOutput`, and
  `EasingSlope` coordinates; role-named `EasingSlopes`; a reflection-only
  repeatable `EasingPlugin`; resource-free samplers; and scope-aware
  `SequenceEasing` replacement or composition through `SequenceEasingSampler`
- Add the optional `tween` feature with `SequenceTweenAdapterPlugin<TimeCtx>`,
  which creates and advances no clock, and `SequencePositionInterpolator`,
  which writes only the position portion of a producer's movement

### Removed

- Remove the provisional external-control model: `SequenceControlMode`,
  `SequenceControlChange`, `SequenceCommandOutcome::ExternalControl`,
  `ExternalAnimationProgress`, `ExternalAnimationSample`,
  `ExternalAnimationLease`, `ExternalControlRejection`,
  `ExternalAnimationSampleInterpolator`, `ExternalAnimationTweenClock`,
  `ExternalAnimationTweenPlugin`, their errors, and the `external_progress`
  example. Producers now publish `SequenceMovement` and claim a target with
  `SequenceDriver`; there are no compatibility aliases
- **Breaking:** Remove the `input` feature. `Keybindings` and the `action!`,
  `event!`, and `bind_action_system!` macros moved to
  [`hana_rubric`](https://crates.io/crates/hana_rubric) unchanged — no items
  were renamed. Drop `features = ["input"]` and depend on `hana_rubric`

### Changed

- **Breaking:** Renamed the crate from `bevy_kana` to `hana_kana`. The `bevy_`
  prefix is reserved for published legacy crates; every crate in this
  workspace now uses the `hana_` prefix. `bevy_kana` 0.3.1 is the final
  release under the old name. Change the dependency name and every
  `use bevy_kana::` path to `hana_kana`; no items were renamed

- **Breaking:** `Easing::Curve` now owns an `EasingCurve` by value. Easing
  authoring no longer uses asset handles or storage, and sampling has no
  readiness branch. Construct semantic input, output, and slope values before
  building a curve. A future editable source must be a distinct `Easing`
  variant with explicit availability semantics.

- Rename `SequencePlaybackSystems::ProduceExternalProgress` to
  `SequencePlaybackSystems::ProduceMovement`, and make every
  `SequencePlayback` mutator reachable only through `SequenceCommands`, so the
  producer a sequence selected is its only position writer
- Make `Orientation` storage private and opaque to structural reflection.
  Replace infallible raw-`Quat` construction with `TryFrom<Quat>`, reject
  non-finite and effectively zero-length values, normalize every accepted
  quaternion, and make interpolation fallible so it cannot introduce invalid
  state

## [0.3.0] - 2026-07-30

### Added

- Add `CascadeRootResource<A>`, letting any resource serve as a cascade's
  app-wide root value instead of the built-in `CascadeDefault<A>` newtype. A
  type can implement it for itself, so an attribute type that is already a
  resource needs no wrapper; a resource can also supply the root for an
  attribute it holds as one field. `CascadePlugin::with_root_resource` selects
  the root resource type, and the plugin inserts that resource from the root
  value given to `CascadePlugin::new` unless the app already inserted it

## [0.2.0] - 2026-07-29

### Added

- Add `Cascade<T>` with explicit `Inherit` / `Override(T)` authoring states and
  storage-independent single-layer and ordered resolution helpers
- Add the reusable ECS cascade engine: `CascadeFrom` / `CascadeChildren`,
  `CascadeAttribute`, `CascadeDefault<A>`, `Resolved<A>`, `CascadePlugin<A>`,
  `CascadeSet`, generic entity commands, and cached-value readers

### Changed

- Development moved into the `natepiano/hana` workspace; the crate is no longer
  developed in a standalone repository

## [0.1.0] - 2026-06-20

### Changed

- Update `bevy` to the 0.19.0 stable release

## [0.1.0-rc.1] - 2026-05-24

### Changed

- Update `bevy` from 0.18 to 0.19 and `bevy_enhanced_input` from 0.25 to 0.26

## [0.0.6] - 2026-05-20

### Changed

- Update `bevy_enhanced_input` from 0.24 to 0.25.0

## [0.0.5] - 2026-04-06

### Added

- `ToU32` impl for `i32`

## [0.0.4] - 2026-04-06

### Added

- `ToF64` impl for `u64`

## [0.0.3] - 2026-03-29

### Added

- `ToU8` cast trait with impls for `f32`, `u32`, `usize`
- `ToU16` cast trait with impls for `usize`, `u32`, `f32`
- `ToF64` cast trait with impls for `usize`, `u32`, `i32`, `f32`
- `ToF32` impl for `f64`
- `ToI32` impl for `f64`
- `ToU32` impl for `u64`
- Prelude re-exports for all new cast traits

## [0.0.2] - 2026-03-25

### Changed

- `input` feature is no longer a default — libraries only need `math` (the default), binaries opt into `input` explicitly with `features = ["input"]`

## [0.0.1] - 2026-03-25

### Added

- Semantic math newtypes (`Position`, `Displacement`, `Velocity`, `ScreenPosition`, `Orientation`) — zero-cost wrappers that act like `Vec3`/`Vec2`/`Quat` but prevent accidental mixing at compile time
- `new()` constructors for all newtypes (`Position::new(x, y, z)`, `ScreenPosition::new(x, y)`)
- Cross-type arithmetic: `Position + Vec3 → Position`, `Position - Vec3 → Position` for natural mixing with Bevy APIs
- `distance`, `distance_squared`, `lerp` methods accepting `impl Into<Self>` — work with both newtypes and raw `Vec3`/`Vec2`
- Numeric cast traits (`ToF32`, `ToI32`, `ToU32`, `ToUsize`) for clean numeric conversions that centralize clippy pedantic cast allows
- Input macros (`action!`, `event!`, `bind_action_system!`) for wiring keyboard actions through `bevy_enhanced_input`
- `Keybindings` builder for modifier-aware keybinding setup with platform-specific Cmd/Ctrl handling
- Feature flags: `math` (default) and `input` (default, requires `bevy_enhanced_input`)
