# Integrating `hana_lagrange` with `egui`

If your app uses [`bevy_egui`](https://github.com/vladbat00/bevy_egui) and you want `OrbitCam` to
stop orbiting/panning/zooming while the pointer or keyboard is interacting with
an egui panel, add a small system to your own app. This page shows how.

> This integration was previously a `bevy_egui` cargo feature. It moved to
> user space so the crate carries no egui dependency. The guide will be kept for
> a release or two, then removed.

## The mechanism

`hana_lagrange` exposes the `CameraInputDisabled` component. While a camera
entity has `CameraInputDisabled`, `OrbitCam` ignores all input for that camera.
The integration is therefore: detect when egui wants input, and add or remove
`CameraInputDisabled` on your `OrbitCam` entities accordingly.

## The system

```rust
use bevy::prelude::*;
use bevy_egui::EguiContext;
use hana_lagrange::{CameraInputDisabled, OrbitCam};

/// Disable `OrbitCam` input on every camera whenever egui wants the pointer
/// or keyboard. Re-enables it as soon as egui releases focus.
fn block_orbit_cam_while_egui_focused(
    mut contexts: Query<&mut EguiContext>,
    cameras: Query<(Entity, Has<CameraInputDisabled>), With<OrbitCam>>,
    mut commands: Commands,
) {
    let egui_wants_input = contexts.iter_mut().any(|mut context| {
        let context = context.get_mut();
        context.wants_pointer_input() || context.wants_keyboard_input()
    });

    for (camera, already_disabled) in &cameras {
        match (egui_wants_input, already_disabled) {
            (true, false) => {
                commands.entity(camera).insert(CameraInputDisabled);
            }
            (false, true) => {
                commands.entity(camera).remove::<CameraInputDisabled>();
            }
            _ => {}
        }
    }
}
```

Register it after egui has initialized its contexts and before `OrbitCam` reads
input. `OrbitCam` runs in `OrbitCamSystemSet` during `PostUpdate`, and
`bevy_egui` updates its contexts in `PreUpdate`, so scheduling in `PreUpdate`
after `EguiPreUpdateSet::InitContexts` works:

```rust
use bevy_egui::EguiPreUpdateSet;

app.add_systems(
    PreUpdate,
    block_orbit_cam_while_egui_focused.after(EguiPreUpdateSet::InitContexts),
);
```

## Per-camera opt-in

The system above blocks every `OrbitCam`. To block only some cameras, mark them
with your own component and filter the query:

```rust
#[derive(Component)]
struct BlockOnEgui;

// query: Query<(Entity, Has<CameraInputDisabled>), (With<OrbitCam>, With<BlockOnEgui>)>
```

## Two nuances

The built-in feature handled two cases this snippet does not. Add them only if
you need them:

1. **One-frame click lag.** When a click first lands inside an egui area,
   `wants_pointer_input()` returns `false` for one frame before returning
   `true`. For that single frame both egui and the camera react to the click. To
   suppress it, track the previous frame's focus and treat the camera as blocked
   when either the previous or current frame wanted input.

2. **Hover blocking.** `wants_pointer_input()` reflects clicks and drags, not
   mere hovering. To also block the camera when the pointer is merely over an
   egui area, include `context.is_pointer_over_area()` in the `egui_wants_input`
   check.

```rust
let egui_wants_input = contexts.iter_mut().any(|mut context| {
    let context = context.get_mut();
    context.wants_pointer_input()
        || context.wants_keyboard_input()
        || context.is_pointer_over_area() // hover blocking
});
```
