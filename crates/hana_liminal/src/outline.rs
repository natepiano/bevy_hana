use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;

use super::constants::MERGED_OVERLAP_SHADER_FACTOR;
use super::constants::NON_SCREEN_HULL_SHELL_MODE_SHADER_FACTOR;
use super::constants::SCREEN_HULL_SHELL_MODE_SHADER_FACTOR;
use super::constants::SEPARATED_OVERLAP_SHADER_FACTOR;
use super::outline_builder::JumpFloodState;
use super::outline_builder::OutlineBuilder;
use super::outline_builder::ScreenHullState;
use super::outline_builder::WorldHullState;

/// Which outline algorithm to use.
///
/// - `JumpFlood`: Screen-space silhouette expansion. Works on **all** geometry including flat
///   panels and UI planes. Width is in pixels.
/// - `WorldHull`: Vertex extrusion with world-unit width. Best for 3D volumetric meshes where
///   outline thickness should scale with distance.
/// - `ScreenHull`: Vertex extrusion with pixel width. Best for 3D volumetric meshes where outline
///   thickness should remain constant on screen.
#[derive(Debug, Clone, Copy, Reflect, PartialEq, Eq, Default)]
pub enum OutlineMethod {
    /// Screen-space silhouette expansion via jump-flood algorithm. Width is in pixels.
    #[default]
    JumpFlood,
    /// Vertex extrusion with world-unit width that scales with camera distance.
    WorldHull,
    /// Vertex extrusion with pixel-unit width that stays constant on screen.
    ScreenHull,
}

impl OutlineMethod {
    /// Returns the shader factor that selects shell-based shading
    /// (1.0 for `ScreenHull`, 0.0 otherwise).
    #[must_use]
    pub(crate) const fn as_shell_mode_factor(self) -> f32 {
        match self {
            Self::ScreenHull => SCREEN_HULL_SHELL_MODE_SHADER_FACTOR,
            Self::JumpFlood | Self::WorldHull => NON_SCREEN_HULL_SHELL_MODE_SHADER_FACTOR,
        }
    }
}

/// How overlapping outlines from different entities interact.
///
/// - [`Merged`](OverlapMode::Merged): Overlapping outlined meshes share a single unified silhouette
///   outline. No outline is drawn where two outlined surfaces overlap — they merge into a single
///   silhouette.
///
/// - [`Grouped`](OverlapMode::Grouped): All meshes within the same entity hierarchy (parent +
///   children sharing a `group_source`) merge into one outline, but that group is visually distinct
///   from other groups. A cube with child spheres looks like one outlined unit, while a neighboring
///   torus has its own separate outline.
///
/// - [`PerMesh`](OverlapMode::PerMesh): Every individual `Mesh3d` gets its own distinct outline
///   boundary, even if it's a child of a larger entity. Child spheres inside a cube each show their
///   own outline.
#[derive(Debug, Clone, Copy, Reflect, PartialEq, Eq, Default)]
pub enum OverlapMode {
    /// Overlapping outlines merge into one shared silhouette.
    #[default]
    Merged,
    /// Meshes in the same group (via `group_source`) merge, but are distinct from other groups.
    Grouped,
    /// Every individual mesh gets its own outline boundary.
    PerMesh,
}

impl OverlapMode {
    /// Returns the shader factor for this overlap mode (0.0 for `Merged`, 1.0 otherwise).
    #[must_use]
    pub const fn as_shader_factor(self) -> f32 {
        match self {
            Self::Merged => MERGED_OVERLAP_SHADER_FACTOR,
            Self::Grouped | Self::PerMesh => SEPARATED_OVERLAP_SHADER_FACTOR,
        }
    }
}

/// Visual style of the outline stroke.
#[derive(Debug, Clone, Copy, Reflect, PartialEq, Eq, Default)]
pub enum LineStyle {
    /// A continuous solid stroke.
    #[default]
    Solid,
}

/// Whether an `Outline` is active without removing the component.
#[derive(Debug, Clone, Copy, Reflect, PartialEq, Eq, Default)]
pub enum OutlineActivity {
    /// The outline participates in extraction and rendering.
    #[default]
    Enabled,
    /// The outline is present but skipped during extraction.
    Disabled,
}

impl OutlineActivity {
    /// Returns whether the outline should participate in extraction and rendering.
    #[must_use]
    pub const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

/// Adds a mesh outline effect to an entity with a `Mesh3d` component.
///
/// Construct via one of the three named constructors — each returns a type-safe
/// builder that only exposes settings valid for that method.
///
/// # Example
///
/// ```rust,no_run
/// # use bevy::prelude::*;
/// # use hana_liminal::Outline;
/// # use hana_liminal::OverlapMode;
/// // `JumpFlood` — screen-space silhouette, works on all geometry
/// Outline::jump_flood(4.0).with_color(Color::WHITE).build();
///
/// // `ScreenHull` — pixel-width vertex extrusion for 3D meshes
/// Outline::screen_hull(3.0)
///     .with_overlap(OverlapMode::PerMesh)
///     .build();
///
/// // `WorldHull` — world-unit vertex extrusion for 3D meshes
/// Outline::world_hull(0.05)
///     .with_overlap(OverlapMode::Grouped)
///     .build();
/// ```
#[derive(Debug, Component, Reflect, Clone)]
#[reflect(Component)]
pub struct Outline {
    /// Outline width. Pixels for `JumpFlood`/`ScreenHull`, world units for `WorldHull`.
    pub width:               f32,
    /// Outline color.
    pub color:               Color,
    /// Multiplier applied to `color` in the shader. Values > 1.0 produce HDR glow via bloom.
    pub intensity:           f32,
    /// Which algorithm to use. See `OutlineMethod` for guidance.
    pub method:              OutlineMethod,
    /// How overlapping outlines from different entities interact.
    pub overlap_mode:        OverlapMode,
    /// Line style (currently only [`Solid`](LineStyle::Solid)).
    pub line_style:          LineStyle,
    /// Whether this outline participates in extraction and rendering.
    pub activity:            OutlineActivity,
    /// Set internally by propagation. When [`Grouped`](OverlapMode::Grouped), all propagated
    /// children share this entity's ID as the owner for overlap resolution. Not user-facing.
    pub(crate) group_source: Option<Entity>,
}

impl Outline {
    /// Create a `JumpFlood` outline builder. Width is in pixels.
    #[must_use]
    pub const fn jump_flood(width: f32) -> OutlineBuilder<JumpFloodState> {
        OutlineBuilder::jump_flood(width)
    }

    /// Create a screen-space hull outline builder. Width is in pixels.
    #[must_use]
    pub const fn screen_hull(width: f32) -> OutlineBuilder<ScreenHullState> {
        OutlineBuilder::screen_hull(width)
    }

    /// Create a world-space hull outline builder. Width is in world units.
    #[must_use]
    pub const fn world_hull(width: f32) -> OutlineBuilder<WorldHullState> {
        OutlineBuilder::world_hull(width)
    }
}

/// Marker component that prevents outline propagation to this entity.
///
/// When a parent entity has an outline that propagates to descendant `Mesh3d` entities,
/// any descendant with `NoOutline` will be skipped. This is useful for invisible helper
/// meshes (e.g. backside pick planes with `AlphaMode::Blend`) that should never receive
/// an outline, even when their ancestor is outlined.
///
/// # Example
///
/// ```rust,no_run
/// # use bevy::prelude::*;
/// # use hana_liminal::NoOutline;
/// // Invisible pick plane that should not receive outline propagation
/// commands.spawn((
///     Name::new("Backside Pick Plane"),
///     Mesh3d(mesh),
///     MeshMaterial3d(transparent_material),
///     NoOutline,
/// ));
/// ```
#[derive(Debug, Component, Reflect, Clone, Copy, Default)]
#[reflect(Component)]
pub struct NoOutline;

/// Marker component that stops outline propagation at this entity's subtree.
///
/// An ancestor's `Outline` never crosses a barrier: neither the barrier entity
/// nor any of its descendants inherit it. The barrier can still source its own
/// outline — inserting `Outline` on the barrier entity propagates to its
/// subtree as usual.
///
/// Use [`NoOutline`] to exempt a single mesh; use `OutlineBarrier` to exempt a
/// whole subtree that manages its own outline, such as an interactive control
/// mounted on an outlined host.
///
/// # Example
///
/// ```rust,no_run
/// # use bevy::prelude::*;
/// # use hana_liminal::Outline;
/// # use hana_liminal::OutlineBarrier;
/// # fn setup(mut commands: Commands) {
/// // A jack mounted on a screen: the screen's outline stops at the jack,
/// // and hovering the jack outlines only the jack's own subtree.
/// commands.spawn((
///     Name::new("Input Jack"),
///     OutlineBarrier,
///     Outline::jump_flood(2.0).with_color(Color::WHITE).build(),
/// ));
/// # }
/// ```
#[derive(Debug, Component, Reflect, Clone, Copy, Default)]
#[reflect(Component)]
pub struct OutlineBarrier;
