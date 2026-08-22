# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.0] - 2026-07-30

### Changed

- Renamed from `bevy_liminal` to `hana_liminal`. No API changes; update the
  dependency name and any `bevy_liminal::` paths, including reflected
  `TypePath` strings in serialized scenes and the `bevy_liminal::view_helpers`
  shader import path. `bevy_liminal` 0.0.6 is a deprecated re-export shim and
  the final release under the old name.

## [0.0.5] - 2026-07-29

### Changed

- Requires `bevy_kana` 0.2.0 (was 0.1.0), so dependents that also use
  `bevy_kana` 0.2 no longer resolve two incompatible copies of it

## [0.0.4] - 2026-07-10

### Added

- `OutlineBarrier` marker to define a hierarchy boundary: outlines inherited from ancestors skip the marked entity and all its descendants, while an outline sourced on the marked entity propagates normally beneath it
- `OutlineBuilder::with_overlap` support for jump-flood outlines, allowing overlap behavior to be selected without assigning an explicit group source

## [0.0.3] - 2026-07-10

### Added

- Group-aware overlap for jump-flood outlines via `OutlineBuilder::with_group(Entity)`: outlines sharing a group merge into one silhouette but may draw over another group's surface, so a nested mesh keeps its own outline on top of its host (previously jump-flood was hardcoded to `Merged`)

### Fixed

- Jump-flood seed priority under reverse-Z so closer seeds win the flood instead of losing

## [0.0.2] - 2026-06-20

### Changed

- Updated to Bevy 0.19

## [0.0.2-rc.1] - 2026-06-08

### Changed

- Updated to Bevy 0.19

## [0.0.1] - 2026-04-06

### Added

- Three outline methods: JumpFlood (screen-space silhouette), WorldHull (world-unit extrusion), ScreenHull (pixel-width extrusion)
- Type-safe builder API: `Outline::jump_flood()`, `Outline::screen_hull()`, `Outline::world_hull()`
- Overlap modes for hull methods: Merged, Grouped, PerMesh
- Smoothed outline normal generation for correct extrusion on concave and hard-edged meshes
- HDR glow via intensity multiplier (works with Bevy's bloom)
- Automatic outline propagation from parent to descendant `Mesh3d` entities
- `NoOutline` marker to exclude specific children from propagation
- Skinning and morph target support
- Depth-aware rendering via depth prepass
