//! Saved world state for reversible screen conversions.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Component;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::PanelWorldConversion;
use super::PanelWorldProjection;
use super::PanelWorldTarget;
use super::SavedWorldRestoreMode;
use crate::layout::Anchor;
use crate::layout::LayoutTree;
use crate::layout::Lighting;
use crate::layout::Sidedness;
use crate::layout::Unit;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;

/// Original world-authored state saved before a world panel becomes screen-space.
#[derive(Component, Clone, Debug)]
pub struct SavedPanelWorldState {
    /// Original entity transform.
    pub transform:          Transform,
    /// Original coordinate-space sizing.
    pub coordinate_space:   CoordinateSpace,
    /// Original panel width in `layout_unit`.
    pub width:              f32,
    /// Original panel height in `layout_unit`.
    pub height:             f32,
    /// Original panel layout unit.
    pub layout_unit:        Unit,
    /// Original resolved font unit.
    pub resolved_font_unit: Unit,
    /// Original resolved lighting.
    pub resolved_lighting:  Lighting,
    /// Original resolved sidedness.
    pub resolved_sidedness: Sidedness,
    /// Original panel anchor.
    pub anchor:             Anchor,
    /// Original target world width, if authored.
    pub world_width:        Option<f32>,
    /// Original target world height, if authored.
    pub world_height:       Option<f32>,
    /// Original layout tree.
    pub tree:               LayoutTree,
    /// Original render layers, if present.
    pub render_layers:      Option<RenderLayers>,
}

impl SavedPanelWorldState {
    pub(super) fn from_panel(
        panel: &DiegeticPanel,
        transform: &Transform,
        resolved_font_unit: Unit,
        resolved_lighting: Lighting,
        resolved_sidedness: Sidedness,
        render_layers: Option<&RenderLayers>,
    ) -> Self {
        Self {
            transform: *transform,
            coordinate_space: panel.coordinate_space().clone(),
            width: panel.width(),
            height: panel.height(),
            layout_unit: panel.layout_unit(),
            resolved_font_unit,
            resolved_lighting,
            resolved_sidedness,
            anchor: panel.anchor(),
            world_width: panel.authored_world_width(),
            world_height: panel.authored_world_height(),
            tree: panel.tree().clone(),
            render_layers: render_layers.cloned(),
        }
    }

    /// Returns a world target on the saved panel plane.
    ///
    /// The target leaves size unset so
    /// [`PanelProjectionParam::project_to_world`](super::PanelProjectionParam::project_to_world)
    /// derives the world size from the panel's current screen footprint, which keeps the
    /// panel the same apparent size across the conversion.
    #[must_use]
    pub fn world_target(&self) -> PanelWorldTarget {
        PanelWorldTarget::default()
            .transform(self.transform)
            .anchor(self.anchor)
    }

    /// Returns the saved unscaled world size in meters.
    #[must_use]
    pub fn world_size(&self) -> Vec2 {
        let physical_width = self.width * self.layout_unit.meters_per_unit();
        let physical_height = self.height * self.layout_unit.meters_per_unit();
        let width = match (self.world_width, self.world_height) {
            (Some(target_width), _) => target_width,
            (None, Some(target_height)) if physical_height > 0.0 => {
                physical_width * (target_height / physical_height)
            },
            (None, Some(_) | None) => physical_width,
        };
        let height = match (self.world_width, self.world_height) {
            (_, Some(target_height)) => target_height,
            (Some(target_width), None) if physical_width > 0.0 => {
                physical_height * (target_width / physical_width)
            },
            (Some(_) | None, None) => physical_height,
        };
        Vec2::new(width, height)
    }

    pub(super) fn world_conversion(
        &self,
        projection: PanelWorldProjection,
    ) -> PanelWorldConversion {
        let world_size = self.world_size();
        let scale = Vec3::new(
            projection.size.x / world_size.x.max(f32::EPSILON),
            projection.size.y / world_size.y.max(f32::EPSILON),
            self.transform.scale.z,
        );
        let (width, height) = match self.coordinate_space.clone() {
            CoordinateSpace::World { width, height } => (width, height),
            CoordinateSpace::Screen { .. } => (
                crate::layout::Sizing::fixed(crate::Mm(self.width)),
                crate::layout::Sizing::fixed(crate::Mm(self.height)),
            ),
        };
        PanelWorldConversion {
            transform: Transform {
                translation: projection.transform.translation,
                rotation: projection.transform.rotation,
                scale,
            },
            size: projection.size,
            panel_size: Vec2::new(self.width, self.height),
            layout_unit: self.layout_unit,
            anchor: Some(self.anchor),
            width,
            height,
            world_width: self.world_width,
            world_height: self.world_height,
            restore_saved_world: SavedWorldRestoreMode::Restore,
        }
    }

    pub(in crate::panel) fn apply_world_conversion(
        &self,
        panel: &mut DiegeticPanel,
        conversion: &PanelWorldConversion,
    ) -> Result<(), super::PanelProjectionError> {
        super::validate_world_conversion(conversion)?;
        panel.width = conversion.panel_size.x;
        panel.height = conversion.panel_size.y;
        panel.layout_unit = conversion.layout_unit;
        panel.anchor = conversion.anchor.unwrap_or(self.anchor);
        panel.world_width = conversion.world_width;
        panel.world_height = conversion.world_height;
        panel.coordinate_space = CoordinateSpace::World {
            width:  conversion.width,
            height: conversion.height,
        };
        panel.replace_tree_full_rebuild(self.tree.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::visibility::RenderLayers;
    use bevy::prelude::Entity;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;

    use super::SavedPanelWorldState;
    use super::SavedWorldRestoreMode;
    use crate::DiegeticPanel;
    use crate::Mm;
    use crate::TextStyle;
    use crate::Unit;

    #[test]
    fn from_panel_captures_authored_render_layers() -> Result<(), String> {
        let panel = saved_panel_fixture()?;
        let transform = Transform::from_xyz(1.0, 2.0, 3.0);
        let render_layers = RenderLayers::layer(3);
        let saved = SavedPanelWorldState::from_panel(
            &panel,
            &transform,
            Unit::Millimeters,
            crate::Lighting::Lit,
            crate::Sidedness::BothSides,
            Some(&render_layers),
        );
        let saved_without_layers = SavedPanelWorldState::from_panel(
            &panel,
            &transform,
            Unit::Millimeters,
            crate::Lighting::Lit,
            crate::Sidedness::BothSides,
            None,
        );

        assert_eq!(saved.render_layers, Some(render_layers));
        assert_eq!(saved_without_layers.render_layers, None);
        Ok(())
    }

    #[test]
    #[allow(
        clippy::expect_used,
        reason = "fixture construction should fail the test immediately"
    )]
    fn target_uses_saved_transform_and_anchor_without_size() {
        let panel = saved_panel_fixture().expect("fixture panel should build");
        let transform = Transform::from_xyz(1.0, 2.0, 3.0);
        let saved = SavedPanelWorldState::from_panel(
            &panel,
            &transform,
            Unit::Millimeters,
            crate::Lighting::Lit,
            crate::Sidedness::BothSides,
            None,
        );

        let target = saved.world_target();

        assert_eq!(target.transform_value(), Some(transform));
        assert_eq!(target.anchor_value(), Some(crate::Anchor::Center));
        assert!(target.world_height_value().is_none());
    }

    #[test]
    #[allow(
        clippy::expect_used,
        reason = "fixture construction should fail the test immediately"
    )]
    fn world_conversion_restores_saved_authoring_units_at_projected_size() {
        let panel = saved_panel_fixture().expect("fixture panel should build");
        let transform = Transform::from_xyz(1.0, 2.0, 3.0).with_scale(Vec3::new(1.0, 1.0, 4.0));
        let saved = SavedPanelWorldState::from_panel(
            &panel,
            &transform,
            Unit::Millimeters,
            crate::Lighting::Lit,
            crate::Sidedness::BothSides,
            None,
        );
        let saved_size = saved.world_size();
        let projected_size = Vec2::new(saved_size.x * 2.0, saved_size.y * 3.0);
        let projected_transform = Transform::from_xyz(4.0, 5.0, 6.0);
        let conversion = saved.world_conversion(crate::PanelWorldProjection {
            panel:               Entity::PLACEHOLDER,
            transform:           projected_transform,
            size:                projected_size,
            panel_size:          Vec2::new(200.0, 120.0),
            layout_unit:         Unit::Pixels,
            anchor:              crate::Anchor::BottomRight,
            width:               crate::Sizing::fixed(crate::Px(200.0)),
            height:              crate::Sizing::fixed(crate::Px(120.0)),
            world_width:         Some(projected_size.x),
            world_height:        Some(projected_size.y),
            restore_saved_world: SavedWorldRestoreMode::Skip,
        });

        assert_eq!(conversion.panel_size, Vec2::new(100.0, 40.0));
        assert_eq!(conversion.layout_unit, Unit::Millimeters);
        assert_eq!(conversion.anchor, Some(crate::Anchor::Center));
        assert_eq!(conversion.world_height, Some(0.5));
        assert_eq!(
            conversion.transform.translation,
            projected_transform.translation
        );
        assert_eq!(conversion.transform.scale, Vec3::new(2.0, 3.0, 4.0));
    }

    fn saved_panel_fixture() -> Result<DiegeticPanel, String> {
        DiegeticPanel::world()
            .size(Mm(100.0), Mm(40.0))
            .font_unit(Unit::Millimeters)
            .world_height(0.5)
            .anchor(crate::Anchor::Center)
            .layout(|builder| {
                builder.text(("saved", TextStyle::new(6.0)));
            })
            .build()
            .map_err(|error| error.to_string())
    }
}
