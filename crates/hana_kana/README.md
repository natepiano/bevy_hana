<div align="center">

<img src="assets/kana.svg" alt="仮名" width="260"/>

# hana_kana

**Ergonomic, opinionated utilities for Bevy — type-safe math and cascade values.**

[![crates.io](https://img.shields.io/crates/v/hana_kana.svg)](https://crates.io/crates/hana_kana)
[![docs.rs](https://docs.rs/hana_kana/badge.svg)](https://docs.rs/hana_kana)
[![license](https://img.shields.io/crates/l/hana_kana.svg)](LICENSE-MIT)

</div>

---

> **Work in progress.** This crate is in active development (v0.4.0) and not
> subject to semver stability guarantees. APIs will change without notice
> between commits. Do not depend on this in production code yet.

> **Renamed from `bevy_kana`.** Releases through 0.3.1 were published under
> that name; 0.4.0 is the first release as `hana_kana`. No items were renamed —
> change the dependency name and every `use bevy_kana::` path. The `input`
> feature did not come along: it now lives in
> [`hana_rubric`](https://crates.io/crates/hana_rubric).

**仮名** (*kana*) — from Japanese 仮 (*ka*, "simplified, borrowed") + 名 (*na*, "name, character"). The kana writing systems — hiragana (ひらがな) and katakana (カタカナ) — were born as simplified characters borrowed from complex kanji, making written language more accessible without losing meaning.

`hana_kana` follows the same philosophy: small, named abstractions borrowed from Bevy's existing types, making game code more expressive and type-safe without adding complexity. It is a growing collection of ergonomic utilities — not limited to any single category.

## What's in the box

### Semantic math types

Zero-cost newtype wrappers around Bevy's math primitives that prevent accidental mixing at compile time.

| Type | Wraps | Purpose |
|------|-------|---------|
| `Angle` | `f32` | A finite, signed, unwrapped angular displacement |
| `Position` | `Vec3` | A point in 3D space |
| `Displacement` | `Vec3` | A delta or offset |
| `Velocity` | `Vec3` | Rate of position change |
| `ScreenPosition` | `Vec2` | Pixel-space coordinates |
| `Orientation` | `Quat` | A finite, normalized rotation |

**Key properties:**

- **Read access** to wrapped values — vector and orientation wrappers provide
  `Deref`; `Angle::radians` returns the preserved scalar
- **Validated construction** — `Angle::from_radians` rejects non-finite values;
  `Orientation::try_from` rejects invalid quaternions and normalizes accepted input
- **Bevy interop** — infallible conversions expose validated values as `f32` or
  `Quat`; semantic vector wrappers retain `From`/`Into` in both directions
- **Type-safe arithmetic** — `Position + Position` works, `Position + Velocity` won't compile
- **`Reflect`** support — invariant-bearing wrappers use opaque reflection so
  structural reflection cannot replace validated state

```rust
use bevy::math::Quat;
use bevy::math::Vec3;
use hana_kana::Angle;
use hana_kana::Orientation;
use hana_kana::Position;
use hana_kana::Velocity;

fn example() -> Result<(), Box<dyn std::error::Error>> {
let start_position = Position(Vec3::new(1.0, 0.0, 0.0));
let end_position = Position(Vec3::new(3.0, 0.0, 0.0));

// Same-type arithmetic works
let centroid = (start_position + end_position) / 2.0;

// Cross-type mixing is a compile error
// let bad = start_position + Velocity(Vec3::X); // ERROR

let angle = Angle::from_radians(4.0 * std::f32::consts::TAU)?;
assert_eq!(angle.radians(), 4.0 * std::f32::consts::TAU);

let orientation = Orientation::try_from(Quat::from_xyzw(0.0, 0.0, 0.0, 2.0))?;
assert!(orientation.is_normalized());
Ok(())
}
```

### Numeric cast traits

Convenience traits that replace bare `as` casts for common numeric conversions, centralizing the clippy `#[allow]` so call sites stay clean.

| Trait | From | Suppresses |
|-------|------|------------|
| `ToU8` | `f32`, `u32`, `usize` | `cast_possible_truncation`, `cast_sign_loss` |
| `ToU16` | `usize`, `u32`, `f32` | `cast_possible_truncation`, `cast_sign_loss` |
| `ToF32` | `i32`, `u32`, `usize`, `f64` | `cast_precision_loss`, `cast_possible_truncation` |
| `ToI32` | `usize`, `u32`, `f32`, `f64` | `cast_possible_truncation`, `cast_possible_wrap` |
| `ToU32` | `usize`, `i32`, `f32`, `f64`, `u64` | `cast_possible_truncation`, `cast_sign_loss` |
| `ToUsize` | `u32`, `f32` | `cast_possible_truncation`, `cast_sign_loss` |
| `ToF64` | `usize`, `u32`, `i32`, `f32`, `u64` | `cast_precision_loss` |

**These conversions are deliberately lossy.** They will silently produce wrong results if the input exceeds the target type's representable range. It is the caller's responsibility to ensure values are in bounds. Typical safe usage: loop indices, mesh vertex counts, and other small geometry values.

```rust
use hana_kana::ToF32;
use hana_kana::ToU32;

let sides: u32 = 8;
let angle = (j.to_f32() / sides.to_f32()) * std::f32::consts::TAU;
let index = positions.len().to_u32();
```

### Shared cascades

`Cascade<T>` represents an authored value that either inherits from the next
lower-precedence scope or overrides it. Ordinary structs can resolve authored
layers without ECS storage:

```rust
use hana_kana::Cascade;
use hana_kana::resolve_cascade;

let member = Cascade::Inherit;
let stage = Cascade::Override(0.25_f32);
let sequence_default = 1.0;

assert_eq!(
    resolve_cascade([member, stage], sequence_default),
    0.25,
);
```

Resolution examines layers from highest to lowest precedence and uses the first
override. A required root value completes the cascade. There are deliberately
no conversions to or from `Option<T>`: `Inherit` is explicit authored state,
not a generic missing value.

For ECS attributes, `CascadePlugin<A>` propagates the same authored component
over an explicit `CascadeFrom` relationship.

```rust
use bevy::prelude::*;
use hana_kana::Cascade;
use hana_kana::CascadeEntityCommandsExt;
use hana_kana::CascadeFrom;
use hana_kana::CascadePlugin;

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
struct Opacity(f32);

fn register(app: &mut App) {
    app.add_plugins(CascadePlugin::new(Opacity(1.0)));
}

fn author(commands: &mut Commands, parent: Entity, child: Entity) {
    commands.entity(parent).override_cascade(Opacity(0.5));
    commands
        .entity(child)
        .insert(CascadeFrom::new(parent))
        .set_cascade(Cascade::<Opacity>::Inherit);
}
```

The engine maintains `Resolved<A>` only on entities carrying `Cascade<A>`.
It handles root-default changes, local authoring changes, relationship
retargeting and removal, participant removal, multi-level propagation, cycle
and depth termination, and change-guarded cache writes.

The root value lives in a resource. `CascadePlugin::new` installs
`CascadeDefault<A>` for it. A crate that already has a suitable resource
implements `CascadeRootResource<A>` on that type and selects it instead, which
lets an attribute serve as its own root:

```rust
use hana_kana::CascadeRootResource;

#[derive(Clone, Copy, Debug, PartialEq, Reflect, Resource)]
#[reflect(Resource)]
struct Opacity(f32);

impl CascadeRootResource<Self> for Opacity {
    fn root(&self) -> Self {
        *self
    }

    fn from_root(root: Self) -> Self {
        root
    }
}

fn register(app: &mut App) {
    app.add_plugins(CascadePlugin::new(Opacity(1.0)).with_root_resource::<Opacity>());
}
```

`insert_resource(Opacity(0.8))` then sets the app-wide default.

Run the interactive generic cascade example (in the
[repository](https://github.com/natepiano/bevy_hana/tree/main/crates/hana_kana/examples);
it depends on unpublished workspace crates, so it ships with the source tree
rather than the crates.io package):

```bash
cargo run --example cascade
```

### Shared sequence movement

A domain embeds `SequencePlayback` and publishes a `SequenceStages` description
of its authored stages. Nothing outside the domain writes local position
directly: the shared `SequenceCommands` system parameter is the only path, and
it enforces which writer owns each sequence.

`SequencePosition` is a finite `0.0..=1.0` position. `SequenceMovement` is one
producer's complete published movement: final position, direction, signed whole
repetitions, and the ordered `RangeCrossings` of that travel. It carries no
writer identity, clock, easing, target, or domain output. Construction rejects
contradictory direction and repetition signs, and crossings that contradict the
final direction or repetition count. A round trip that returns to its start
stays expressible.

A producer entity carries `SequenceDriver { target }`. Bevy's required
components pull in `SequenceEvaluation { scope, easing }`, whose `Default` is
`SequenceEvaluation::AUTHORED_WHOLE` — whole sequence, domain-authored easing —
so a scope and an easing relationship are always present values rather than
absences. Inserting or changing `SequenceDriver` requests ownership without
displacing a producer that already holds the target; the rejected claim
triggers `SequenceDriverClaimRejected` and changes nothing. Inserting
`SequenceDriverTakeover` beside it performs the one explicit takeover, records
the sole displaced producer, and triggers `SequenceDriverSelected`.

When a selection ends, the displaced producer is restored only if it still
targets that sequence, published `SequenceSourceState::mark_current()` after
sampling its own source, and its scope still resolves against the current
`SequenceStages`. Otherwise the target becomes unowned. Either way the target
sees `SequenceDriverReleased` with the exact `DriverRestoration`.

`SequenceCommand` covers play, play backward, pause, resume, cancel, step, and
step backward. The shared layer resolves absolute play destinations and journey
state; domains resolve adjacent step destinations from their own boundaries.
A command issued against a target another producer holds is rejected without
mutation or queuing and triggers `SequenceCommandRejected`. Once arbitration
permits a command, the outcome is only `Applied` or `NoChange`.

`SequencePlaybackPlugin` chains four `Update` boundaries:
`ArbitrateDrivers` → `ProduceMovement` → `ApplyMovement` → `EvaluateSequences`.
Domain evaluators observe the selected producer's movement in the same frame it
was produced.

### Authored easing

`Easing` selects either a stock Bevy `EaseFunction` or an owned immutable
`EasingCurve`. `EasingInput`, `EasingOutput`, and `EasingSlope` make knot and
tangent roles explicit before a curve is built; `EasingCurveError` identifies
the invalid role precisely. Its spline implementation stays private, so
downstream crates never depend on it. `EasingPlugin` is repeatable and
reflection-only. `EasingSampler` reports `Eased` or `NonFinite` without world
or asset access; the shared layer never substitutes linear easing.

`SequenceEasing` states how a producer's curve relates to authored stage easing:
`Authored`, `ReplacedBy(Easing)`, or `ComposedWith(Easing)`. A one-stage scope
treats the curve as output easing, so anticipation and overshoot are allowed. A
whole-sequence or multi-stage scope treats it as a remap of scope progress, so
`SequenceEasingSampler` rejects a curve that overshoots or reverses instead of
distorting stage boundaries.

Enable the optional `tween` feature to produce movement with `bevy_tween`:

```toml
hana_kana = { version = "0.4.0", features = ["tween"] }
```

`SequenceTweenAdapterPlugin<TimeCtx>` creates and advances no clock. Install
`bevy_tween`'s `TimeRunnerPlugin<TimeCtx>` and `EaseKindPlugin<TimeCtx>`
yourself, in `Update`; the adapter only applies finished interpolation values
inside `SequencePlaybackSystems::ProduceMovement`, after
`TweenSystemSet::UpdateInterpolationValue`. Running `bevy_tween` in a later
schedule such as `PostUpdate` delays every tweened position by one frame. `SequencePositionInterpolator`
writes the position portion of `SequenceMovement` and nothing else, so a tween
never invents traversal history.

Run the focused driver example:

```bash
cargo run --example sequence_drivers
```

### More to come

`hana_kana` will grow to include other convenience macros and generic utilities that are broadly useful across Bevy projects.

## Version Compatibility

| Crate | Version | Bevy |
|-------|---------|------|
| `hana_kana` | 0.4.0 | 0.19 |
| `bevy_kana` | 0.3.1 | 0.19 |
| `bevy_kana` | 0.3.0 | 0.19 |
| `bevy_kana` | 0.2.0 | 0.19 |
| `bevy_kana` | 0.1.0 | 0.19 |
| `bevy_kana` | 0.0.6 | 0.18 |

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
hana_kana = "0.4.0"
```

Run the example:

```bash
cargo run --example basics
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
