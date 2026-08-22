[![Crates.io](https://img.shields.io/crates/v/hana_lagrange)](https://crates.io/crates/hana_lagrange)
[![docs.rs](https://docs.rs/hana_lagrange/badge.svg)](https://docs.rs/hana_lagrange)
[![Bevy tracking](https://img.shields.io/badge/Bevy%20tracking-released%20version-lightblue)](https://github.com/bevyengine/bevy/blob/main/docs/plugins_guidelines.md#main-branch-tracking)

# `hana_lagrange`

> **Renamed.** This crate was published as `bevy_lagrange` through v0.3.0. The
> API and feature names are unchanged; only the crate name and its `TypePath`
> strings differ.

> **Work in progress.** This crate is in active development and not subject to
> semver stability guarantees. APIs will change without notice between commits.
> Do not depend on this in production code yet.

A camera controller for [Bevy](https://bevyengine.org) that combines smooth orbit controls with event-driven camera operations — zoom-to-fit, retained camera journeys, and a debug overlay for fit targets.

![A screen recording showing camera movement](https://github.com/user-attachments/assets/d596d9cb-2874-47b1-90e4-d5c33ac53983 "Demo of hana_lagrange")

## Features

- Smooth orbit, pan, and zoom with configurable limits
- Zoom-to-fit, look-at, and retained camera journeys with easing
- Event-driven control with full lifecycle events for sequencing
- Orthographic and perspective projection, multi-viewport, render-to-texture
- Preset, custom, and manual input modes with source-attributed interaction events
- Touch, smooth-scroll, keyboard, and gamepad support
- Debug overlay for fit targets (optional `fit_overlay` feature)

## Quick Start

Add the plugin and spawn a camera:

```rust ignore
use bevy::prelude::*;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(LagrangePlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Transform::from_translation(Vec3::new(0.0, 1.5, 5.0)),
        OrbitCam::default(),
    ));
}
```

`OrbitCam` automatically requires `Camera3d`. Out of the box you get orbit, pan, and zoom with smoothing. For perspective cameras, the default near clip plane scales with orbit radius so close-up zooming does not clip away the target.

## Controls

Default mouse controls:

| Input | Action |
|-------|--------|
| Left Mouse | Orbit |
| Right Mouse | Pan |
| Scroll Wheel | Zoom |

The default input mode is `OrbitCamPreset::simple_mouse()`. Insert
`OrbitCamPreset::blender_like()` for editor navigation, use
`OrbitCamBindings` for app-owned keymaps or gamepad mappings, and use
`OrbitCamManual` with `OrbitCamManualInputWriter` only when your app computes
orbit, pan, or zoom intent itself.

Blender-like controls:

| Input | Action |
|-------|--------|
| Middle Mouse Drag | Orbit |
| Shift + Middle Mouse Drag | Pan |
| Trackpad Scroll | Orbit |
| Shift + Trackpad Scroll | Pan |
| Control + Trackpad Scroll | Zoom |
| Scroll Wheel or Pinch | Zoom |

Default touch controls:

| Input | Action |
|-------|--------|
| One finger | Orbit |
| Two fingers | Pan |
| Pinch | Zoom |

Camera behavior such as sensitivity, smoothing, limits, and projection handling
stays on `OrbitCam`. User input bindings live on input-mode components.

## Event-Driven Camera Control

Enable the `fit_overlay` feature:

```toml
hana_lagrange = { version = "...", features = ["fit_overlay"] }
```

### Zoom-to-fit

Frame a target entity in the camera view:

```rust ignore
commands.trigger(
    ZoomToFit::new(camera, target)
        .margin(0.15)
        .anchor(FitAnchor::Center)
        .duration(Duration::from_millis(800))
        .easing(EaseFunction::CubicOut),
);
```

Use `.anchor(FitAnchor::TopLeft)` / `.anchor(FitAnchor::BottomRight)` to place
the fitted projected bounds against a viewport edge or corner. Add
`.offset_px(Vec2::new(x, y))` when that placement needs a screen-pixel offset;
positive x moves right and positive y moves down.

### Look at

Rotate the camera in place to face a target:

```rust ignore
commands.trigger(
    LookAt::new(camera, target)
        .duration(Duration::from_millis(600)),
);
```

### Animate to a specific orientation

Animate to a chosen yaw/pitch while framing the target:

```rust ignore
commands.trigger(
    AnimateToFit::new(camera, target)
        .yaw(PI / 4.0)
        .pitch(PI / 6.0)
        .duration(Duration::from_millis(1200)),
);
```

### Retained camera journeys

Every camera move is built through a fallible constructor that validates the
authored pose, so a `CameraMove` that exists is one the runtime can play:

```rust ignore
let camera_move = CameraMove::try_to_orbital_look_at(
    Focus(Vec3::ZERO),
    OrbitAngles { yaw: 0.0, pitch: 0.5 },
    Radius(5.0),
    FreeCamRollTarget::InheritPrevious,
    Duration::from_millis(800),
    EaseFunction::CubicOut,
)?;

commands.trigger(PlayAnimation::new(camera, [camera_move]));
```

`FreeCamRollTarget::InheritPrevious` keeps whatever roll a free-flight camera
already carries; `FreeCamRollTarget::Explicit(Roll(..))` rolls to a chosen
angle. `OrbitCam` has no roll axis and ignores the choice.

`PlayAnimation` is the atomic request facade. To retain and replace authoring
directly, insert a never-empty `CameraSequence`; it describes its own stages:

```rust ignore
let sequence = CameraSequence::new(first_move).then(second_move).then(third_move);
commands.entity(camera).insert(sequence);
```

Use `CameraCommands::{owner, observe, apply}` for retained state and shared
`SequenceCommand` transport. `owner` and `observe` distinguish a missing
sequence from native playback and a selected driver; `apply` returns the shared
command outcome without exposing evaluator storage.

Easing is either a stock `EaseFunction` or an owned immutable `EasingCurve`.
Camera moves sample it directly and `LagrangePlugin` has no easing-related
`AssetPlugin` requirement. A future editable or asset-backed source would be a
separate `Easing` variant with explicit availability semantics.

If native or selected-driver easing evaluates to an invalid value, the retained
evaluator holds the last valid camera pose and logs one contextual warning for
that uninterrupted source/error episode. A valid result rearms the diagnostic.
Raw traversal remains authoritative, so invalid evaluated easing never removes
already-crossed move boundaries.

All operations support instant (`Duration::ZERO`) and animated paths with the
same lifecycle event names applications already use. `AnimationBegin` and
`AnimationEnd` now identify the `SequenceOwner`, direction, and raw
`CameraEventTiming`; `CameraMoveBegin` and `CameraMoveEnd` additionally name a
stable `SequenceStageId` and the exact `boundary_elapsed` authored time.
`CameraEventTiming` intentionally exposes only raw position and total duration:
normalized interior progress cannot be inverted into one exact elapsed time.

`AnimationRejected` carries an exact `AnimationRejectionReason`, including
controller availability, retained preparation, native conflict, and selected
driver ownership. There is no parallel `CameraAnimation*` event API. A native
facade request emits its begin event at admission; move events then translate
only crossed raw ledger boundaries. An instant fit or look request therefore
emits its begin, move begin, move end, and completion in that order in one
update (with `ZoomBegin` before, and `ZoomEnd` after, for `ZoomToFit`).

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `fit_overlay` | no | Zoom-to-fit, camera animations, event-driven control, and debug overlay |

For `egui` integration — preventing camera input while egui has focus — see [`docs/egui.md`](docs/egui.md).

## Version Compatibility

| Version                | Bevy |
|------------------------|------|
| `hana_lagrange` 0.4.0  | 0.19 |
| `bevy_lagrange` 0.3.0  | 0.19 |
| `bevy_lagrange` 0.2.0  | 0.19 |
| `bevy_lagrange` 0.1.0  | 0.19 |
| `bevy_lagrange` 0.0.4  | 0.19 |
| `bevy_lagrange` 0.0.3  | 0.18 |

## Credits

- [Plonq](https://github.com/Plonq) — `hana_lagrange` builds on [bevy_panorbit_camera](https://github.com/Plonq/bevy_panorbit_camera), with permission

## License

All code in this repository is dual-licensed under either:

- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.
