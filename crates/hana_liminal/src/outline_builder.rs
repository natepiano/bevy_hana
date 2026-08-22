use std::marker::PhantomData;

use bevy::prelude::Color;
use bevy::prelude::Entity;

use super::constants::DEFAULT_OUTLINE_INTENSITY;
use super::outline::LineStyle;
use super::outline::Outline;
use super::outline::OutlineActivity;
use super::outline::OutlineMethod;
use super::outline::OverlapMode;

/// Sealed trait implemented by outline mode type-state markers.
pub trait OutlineModeState {
    /// The [`OutlineMethod`] variant this state represents.
    const MODE: OutlineMethod;
}

/// Marker trait for hull-based outline modes (`WorldHull`, `ScreenHull`).
pub trait HullModeState: OutlineModeState {}

/// Type-state marker for the jump-flood outline method.
#[derive(Debug, Clone, Copy)]
pub struct JumpFloodState;

/// Type-state marker for the world-space hull outline method.
#[derive(Debug, Clone, Copy)]
pub struct WorldHullState;

/// Type-state marker for the screen-space hull outline method.
#[derive(Debug, Clone, Copy)]
pub struct ScreenHullState;

impl OutlineModeState for JumpFloodState {
    const MODE: OutlineMethod = OutlineMethod::JumpFlood;
}

impl OutlineModeState for WorldHullState {
    const MODE: OutlineMethod = OutlineMethod::WorldHull;
}

impl OutlineModeState for ScreenHullState {
    const MODE: OutlineMethod = OutlineMethod::ScreenHull;
}

impl HullModeState for WorldHullState {}
impl HullModeState for ScreenHullState {}

/// Type-safe builder for constructing an `Outline` component.
#[derive(Debug, Clone)]
pub struct OutlineBuilder<M: OutlineModeState> {
    width:        f32,
    intensity:    f32,
    color:        Color,
    overlap_mode: OverlapMode,
    group_source: Option<Entity>,
    mode:         PhantomData<M>,
}

impl OutlineBuilder<JumpFloodState> {
    /// Create a new jump-flood outline builder with the given pixel width.
    #[must_use]
    pub const fn jump_flood(width: f32) -> Self { defaults(width) }

    /// Consume the builder and produce a configured `Outline` component.
    #[must_use]
    pub const fn build(self) -> Outline {
        Outline {
            intensity:    self.intensity,
            width:        self.width,
            overlap_mode: self.overlap_mode,
            group_source: self.group_source,
            color:        self.color,
            method:       OutlineMethod::JumpFlood,
            line_style:   LineStyle::Solid,
            activity:     OutlineActivity::Enabled,
        }
    }
}

impl OutlineBuilder<WorldHullState> {
    /// Create a new world-space hull outline builder with the given world-unit width.
    #[must_use]
    pub const fn world_hull(width: f32) -> Self { defaults(width) }
}

impl OutlineBuilder<ScreenHullState> {
    /// Create a new screen-space hull outline builder with the given pixel width.
    #[must_use]
    pub const fn screen_hull(width: f32) -> Self { defaults(width) }
}

/// Settings available on all outline methods.
impl<M: OutlineModeState> OutlineBuilder<M> {
    /// Override the outline width.
    #[must_use]
    pub const fn with_width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Set the color intensity multiplier (values > 1.0 produce HDR glow).
    #[must_use]
    pub const fn with_intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Set the outline color.
    #[must_use]
    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Assign this outline to a group and switch to [`OverlapMode::Grouped`].
    ///
    /// Outlines sharing the same `group` entity merge into one silhouette that
    /// stays visually distinct from other groups: a grouped outline may draw
    /// over another group's outlined surface (subject to depth), so nested
    /// outlined meshes keep their own outline on top of their host.
    ///
    /// Use this when inserting `Outline` per mesh instead of relying on
    /// hierarchy propagation, which assigns the group automatically.
    #[must_use]
    pub const fn with_group(mut self, group: Entity) -> Self {
        self.group_source = Some(group);
        self.overlap_mode = OverlapMode::Grouped;
        self
    }

    /// Set how overlapping outlines interact. See [`OverlapMode`].
    ///
    /// Unlike [`with_group`](Self::with_group), this leaves `group_source`
    /// unset, so an outline inserted on a hierarchy root still propagates to
    /// descendant meshes — propagation assigns the group automatically.
    #[must_use]
    pub const fn with_overlap(mut self, overlap_mode: OverlapMode) -> Self {
        self.overlap_mode = overlap_mode;
        self
    }
}

/// Settings only available on hull methods (`WorldHull`, `ScreenHull`).
impl<M: HullModeState> OutlineBuilder<M> {
    /// Consume the builder and produce a configured `Outline` component.
    #[must_use]
    pub const fn build(self) -> Outline {
        Outline {
            intensity:    self.intensity,
            width:        self.width,
            overlap_mode: self.overlap_mode,
            color:        self.color,
            method:       M::MODE,
            line_style:   LineStyle::Solid,
            activity:     OutlineActivity::Enabled,
            group_source: None,
        }
    }
}

const fn defaults<M: OutlineModeState>(width: f32) -> OutlineBuilder<M> {
    OutlineBuilder {
        width,
        intensity: DEFAULT_OUTLINE_INTENSITY,
        color: Color::BLACK,
        overlap_mode: OverlapMode::Merged,
        group_source: None,
        mode: PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Color;

    use crate::Outline;
    use crate::OutlineActivity;
    use crate::OutlineMethod;
    use crate::OverlapMode;

    #[test]
    fn jump_flood_builds_correctly() {
        let outline = Outline::jump_flood(4.0).with_color(Color::WHITE).build();

        assert_eq!(outline.method, OutlineMethod::JumpFlood);
        assert!((outline.width - 4.0).abs() < f32::EPSILON);
        assert_eq!(outline.overlap_mode, OverlapMode::Merged);
        assert_eq!(outline.activity, OutlineActivity::Enabled);
    }

    #[test]
    fn screen_hull_with_overlap() {
        let outline = Outline::screen_hull(3.0)
            .with_overlap(OverlapMode::PerMesh)
            .build();

        assert_eq!(outline.method, OutlineMethod::ScreenHull);
        assert!((outline.width - 3.0).abs() < f32::EPSILON);
        assert_eq!(outline.overlap_mode, OverlapMode::PerMesh);
    }

    #[test]
    fn world_hull_with_grouped_overlap() {
        let outline = Outline::world_hull(0.05)
            .with_overlap(OverlapMode::Grouped)
            .build();

        assert_eq!(outline.method, OutlineMethod::WorldHull);
        assert_eq!(outline.overlap_mode, OverlapMode::Grouped);
    }

    #[test]
    fn jump_flood_with_overlap_keeps_group_source_unset() {
        let outline = Outline::jump_flood(4.0)
            .with_overlap(OverlapMode::Grouped)
            .build();

        assert_eq!(outline.overlap_mode, OverlapMode::Grouped);
        assert_eq!(outline.group_source, None);
    }

    #[test]
    fn activity_defaults_to_enabled() {
        let outline = Outline::jump_flood(2.0).build();
        assert_eq!(outline.activity, OutlineActivity::Enabled);
    }
}
