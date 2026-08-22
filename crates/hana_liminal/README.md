# hana_liminal

[![Crates.io](https://img.shields.io/crates/v/hana_liminal.svg)](https://crates.io/crates/hana_liminal)
[![Downloads](https://img.shields.io/crates/d/hana_liminal.svg)](https://crates.io/crates/hana_liminal)
[![CI](https://github.com/natepiano/hana/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/hana/actions/workflows/ci.yml)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/natepiano/hana/tree/main/crates/hana_liminal#license)

A Bevy plugin for rendering 3D mesh outlines using jump-flood and hull-extrusion methods.

> **Renamed.** This crate was published as `bevy_liminal` through v0.0.5. The
> API is unchanged; only the crate name and its `TypePath` strings differ.

> **Work in progress.** This crate is in active development (v0.1.0) and not
> subject to semver stability guarantees. APIs will change without notice
> between commits. Do not depend on this in production code yet.

## Usage

```rust
use bevy::prelude::*;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(LiminalPlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.spawn((
        Camera3d::default(),
        OutlineCamera,
    ));

    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0))),
        Outline::screen_hull(2.0).build(),
    ));
}
```

## Outline Methods

Three rendering methods, each suited to different use cases:

- **JumpFlood** -- Screen-space silhouette expansion. Works on all geometry including flat panels. Width in pixels.
- **WorldHull** -- Vertex extrusion with world-unit width. Outline thickness scales with camera distance.
- **ScreenHull** -- Vertex extrusion with pixel width. Outline thickness stays constant on screen.

```rust
// JumpFlood (default) -- 3px screen-space outline
Outline::jump_flood(3.0).build();

// ScreenHull -- 2px constant-width hull outline
Outline::screen_hull(2.0).build();

// WorldHull -- 0.05 world-unit hull outline
Outline::world_hull(0.05).build();
```

## Overlap Modes

All outline methods support three overlap modes for controlling how outlines interact:

- **Merged** (default) -- Overlapping outlines merge into one shared silhouette.
- **Grouped** -- Meshes in the same entity hierarchy merge, but are distinct from other groups.
- **PerMesh** -- Every individual mesh gets its own outline boundary.

```rust
use hana_liminal::OverlapMode;

Outline::screen_hull(2.0)
    .with_overlap(OverlapMode::PerMesh)
    .build();
```

## Outline Normals

Hull methods automatically generate smoothed outline normals when an `Outline` component is added. These angle-weighted normals produce correct silhouette extrusion on concave and hard-edged meshes. Meshes without outline normals fall back to radial extrusion from the object origin.

## HDR Glow

Set intensity > 1.0 to produce HDR glow when used with Bevy's bloom. Requires an HDR camera with bloom enabled -- without HDR, values above 1.0 are clamped. In multi-camera setups, HDR must be consistent across all cameras or rendering silently breaks ([bevy#15467](https://github.com/bevyengine/bevy/issues/15467)).

```rust
Outline::jump_flood(3.0)
    .with_intensity(5.0)
    .build();
```

## Hierarchy Propagation

Adding `Outline` to a parent entity automatically propagates it through the `ChildOf` hierarchy to all descendant `Mesh3d` entities. Use `NoOutline` to exclude a single mesh. `OutlineBarrier` defines a hierarchy boundary: outlines inherited from ancestors skip the marked entity and all its descendants, while an outline placed on the marked entity still propagates normally beneath it.

## Version Compatibility

| Version                   | Bevy |
|---------------------------|------|
| hana_liminal 0.1.0        | 0.19 |
| bevy_liminal 0.0.2–0.0.6  | 0.19 |
| bevy_liminal 0.0.0–0.0.1  | 0.18 |

## License

`hana_liminal` is free, open source and permissively licensed!
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
