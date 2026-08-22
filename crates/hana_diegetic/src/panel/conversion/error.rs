//! Shared projection and conversion errors.

use bevy::transform::helper::ComputeGlobalTransformError;

use crate::panel::PanelAnchorGeometryError;

/// Why a panel could not be projected or converted.
#[derive(thiserror::Error, Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelProjectionError {
    /// The panel entity has no [`DiegeticPanel`](crate::DiegeticPanel).
    #[error("panel is missing")]
    PanelMissing,
    /// The camera entity has no [`Camera`](bevy::prelude::Camera).
    #[error("camera is missing")]
    CameraMissing,
    /// The camera does not render to a window target.
    #[error("camera target is not a window")]
    UnsupportedCameraTarget,
    /// The target window could not be resolved.
    #[error("window is missing")]
    WindowMissing,
    /// The camera has no usable viewport size yet.
    #[error("camera viewport size is unavailable")]
    NoViewportSize,
    /// A transform needed for projection could not be computed.
    #[error("transform is unavailable")]
    TransformUnavailable,
    /// The panel dimensions were non-finite or non-positive.
    #[error("panel size is invalid")]
    InvalidPanelSize,
    /// The panel's world plane was degenerate.
    #[error("panel plane is invalid")]
    InvalidPanelPlane,
    /// The world target was missing a usable plane or size.
    #[error("world target is invalid")]
    InvalidWorldTarget,
    /// The panel has no saved screen handoff camera/depth.
    #[error("screen handoff is missing")]
    ScreenHandoffMissing,
    /// The panel has no saved world-authored state.
    #[error("saved world state is missing")]
    SavedWorldStateMissing,
    /// The camera could not project or unproject the panel.
    #[error("panel projection failed")]
    ProjectionFailed,
    /// The resulting projection was non-finite or zero-sized.
    #[error("panel projection is invalid")]
    InvalidProjection,
}

impl From<ComputeGlobalTransformError> for PanelProjectionError {
    fn from(_: ComputeGlobalTransformError) -> Self { Self::TransformUnavailable }
}

impl From<PanelAnchorGeometryError> for PanelProjectionError {
    fn from(error: PanelAnchorGeometryError) -> Self {
        match error {
            PanelAnchorGeometryError::PanelMissing => Self::PanelMissing,
            PanelAnchorGeometryError::WindowMissing => Self::WindowMissing,
            PanelAnchorGeometryError::WindowZeroSized => Self::NoViewportSize,
            PanelAnchorGeometryError::TransformUnavailable => Self::TransformUnavailable,
            PanelAnchorGeometryError::InvalidPanelSize => Self::InvalidPanelSize,
            PanelAnchorGeometryError::InvalidPanelPlane => Self::InvalidPanelPlane,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::transform::TransformPlugin;

    use super::*;
    use crate::Anchor;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::HeadlessLayoutPlugin;
    use crate::Mm;
    use crate::PanelAttachment;
    use crate::PanelEntity;
    use crate::PanelEntityReader;
    use crate::PanelScreenConversion;
    use crate::PanelWorldProjection;
    use crate::Px;
    use crate::Sizing;
    use crate::Unit;
    use crate::World as WorldSpace;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::PanelSpace;
    use crate::panel::conversion::SavedWorldRestoreMode;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::PanelWidget;
    use crate::widgets::PanelWidgetIndex;
    use crate::widgets::WidgetOf;

    fn world_handle(app: &mut App, entity: Entity) -> PanelEntity<WorldSpace> {
        app.world_mut()
            .run_system_once(move |reader: PanelEntityReader| reader.world(entity))
            .expect("panel reader system runs")
            .expect("panel is currently world-space")
    }

    fn attach_world_panel(app: &mut App, source: Entity, target: Entity) {
        let source = world_handle(app, source);
        let target = world_handle(app, target);
        app.world_mut()
            .run_system_once(move |mut attachments: Commands| {
                attachments.attach_to_panel(
                    source,
                    target,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
            })
            .expect("attachment system runs");
    }

    fn try_screen_conversion(
        app: &mut App,
        panel: PanelEntity<WorldSpace>,
    ) -> Result<(), PanelProjectionError> {
        app.world_mut()
            .run_system_once(move |mut conversions: Commands| {
                conversions.apply_to_screen(
                    panel,
                    PanelScreenConversion::at_pixels(Vec2::new(50.0, 60.0), Vec2::new(80.0, 40.0)),
                )
            })
            .expect("conversion system runs")
    }

    fn empty_world_panel() -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Mm(200.0), Mm(100.0))
            .world_width(2.0)
            .layout(|_| {})
            .build()
            .expect("world panel builds")
    }

    fn empty_screen_panel() -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(200.0), Px(100.0))
            .screen_position(0.0, 0.0)
            .layout(|_| {})
            .build()
            .expect("screen panel builds")
    }

    fn conversion_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.insert_resource(DiegeticTextMeasurer {
            measure_fn: Arc::new(|_text: &str, measure: &TextMeasure| TextDimensions {
                width:       measure.size,
                height:      measure.size,
                line_height: measure.size,
            }),
        });
        app.add_plugins(HeadlessLayoutPlugin);
        app
    }

    fn world_projection(panel: Entity, translation: Vec3) -> PanelWorldProjection {
        PanelWorldProjection {
            panel,
            transform: Transform::from_translation(translation),
            size: Vec2::new(2.0, 1.0),
            panel_size: Vec2::new(200.0, 100.0),
            layout_unit: Unit::Millimeters,
            anchor: Anchor::Center,
            width: Sizing::fixed(Mm(200.0)),
            height: Sizing::fixed(Mm(100.0)),
            world_width: Some(2.0),
            world_height: Some(1.0),
            restore_saved_world: SavedWorldRestoreMode::Skip,
        }
    }

    #[test]
    fn panel_projection_error_messages_are_stable() {
        let cases = [
            (PanelProjectionError::PanelMissing, "panel is missing"),
            (PanelProjectionError::CameraMissing, "camera is missing"),
            (
                PanelProjectionError::UnsupportedCameraTarget,
                "camera target is not a window",
            ),
            (PanelProjectionError::WindowMissing, "window is missing"),
            (
                PanelProjectionError::NoViewportSize,
                "camera viewport size is unavailable",
            ),
            (
                PanelProjectionError::TransformUnavailable,
                "transform is unavailable",
            ),
            (
                PanelProjectionError::InvalidPanelSize,
                "panel size is invalid",
            ),
            (
                PanelProjectionError::InvalidPanelPlane,
                "panel plane is invalid",
            ),
            (
                PanelProjectionError::InvalidWorldTarget,
                "world target is invalid",
            ),
            (
                PanelProjectionError::ScreenHandoffMissing,
                "screen handoff is missing",
            ),
            (
                PanelProjectionError::SavedWorldStateMissing,
                "saved world state is missing",
            ),
            (
                PanelProjectionError::ProjectionFailed,
                "panel projection failed",
            ),
            (
                PanelProjectionError::InvalidProjection,
                "panel projection is invalid",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn transform_error_conversion_is_lossy() {
        let error = PanelProjectionError::from(ComputeGlobalTransformError::MissingTransform(
            Entity::PLACEHOLDER,
        ));

        assert_eq!(error, PanelProjectionError::TransformUnavailable);
        assert!(error.source().is_none());
    }

    #[test]
    fn anchor_geometry_error_conversions_are_normalized() {
        let cases = [
            (
                PanelAnchorGeometryError::PanelMissing,
                PanelProjectionError::PanelMissing,
            ),
            (
                PanelAnchorGeometryError::WindowMissing,
                PanelProjectionError::WindowMissing,
            ),
            (
                PanelAnchorGeometryError::WindowZeroSized,
                PanelProjectionError::NoViewportSize,
            ),
            (
                PanelAnchorGeometryError::TransformUnavailable,
                PanelProjectionError::TransformUnavailable,
            ),
            (
                PanelAnchorGeometryError::InvalidPanelSize,
                PanelProjectionError::InvalidPanelSize,
            ),
            (
                PanelAnchorGeometryError::InvalidPanelPlane,
                PanelProjectionError::InvalidPanelPlane,
            ),
        ];

        for (source, expected) in cases {
            let error = PanelProjectionError::from(source);
            assert_eq!(error, expected);
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn queued_conversion_rejects_stale_handle_without_mutation() {
        let mut app = conversion_test_app();
        let panel = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();
        let handle = world_handle(&mut app, panel);
        app.world_mut()
            .entity_mut(panel)
            .insert(empty_screen_panel());

        assert_eq!(try_screen_conversion(&mut app, handle), Ok(()));
        assert!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .is_some_and(|panel| panel.coordinate_space().is_screen()),
        );
    }

    #[test]
    fn queued_conversion_rejects_all_attachment_graph_roles_without_mutation() {
        let mut app = conversion_test_app();

        let outgoing = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let outgoing_target = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();
        attach_world_panel(&mut app, outgoing, outgoing_target);

        let incoming = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let incoming_source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();
        attach_world_panel(&mut app, incoming_source, incoming);

        let widget_owner = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let widget_source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();
        let widget_id = crate::PanelElementId::named("target");
        let widget = app
            .world_mut()
            .spawn((
                PanelWidget::new(widget_id.clone()),
                WidgetOf::new(widget_owner),
            ))
            .id();
        app.world_mut()
            .get_mut::<PanelWidgetIndex>(widget_owner)
            .expect("widget owner has an index")
            .insert(widget_id, widget);
        let widget_source_handle = world_handle(&mut app, widget_source);
        let widget_handle =
            crate::WidgetEntity::from_validated(widget, widget_owner, PanelSpace::World);
        app.world_mut()
            .run_system_once(move |mut attachments: Commands| {
                attachments.attach_to_widget(
                    widget_source_handle,
                    widget_handle,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
            })
            .expect("attachment system runs");

        for panel in [outgoing, incoming, widget_owner] {
            let handle = world_handle(&mut app, panel);
            assert_eq!(try_screen_conversion(&mut app, handle), Ok(()));
            assert!(
                app.world()
                    .get::<DiegeticPanel>(panel)
                    .is_some_and(|panel| !panel.coordinate_space().is_screen()),
            );
        }
    }

    #[test]
    fn queued_attach_then_conversion_keeps_the_attachment_and_rejects_conversion() {
        let mut app = conversion_test_app();
        let source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();

        let source_handle = world_handle(&mut app, source);
        let target_handle = world_handle(&mut app, target);
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.attach_to_panel(
                    source_handle,
                    target_handle,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
                commands.apply_to_screen(
                    source_handle,
                    PanelScreenConversion::at_pixels(Vec2::new(50.0, 60.0), Vec2::new(80.0, 40.0)),
                )
            })
            .expect("mixed-operation system runs")
            .expect("screen conversion recipe is valid");

        assert!(
            app.world()
                .get::<DiegeticPanel>(source)
                .is_some_and(|panel| !panel.coordinate_space().is_screen()),
        );
        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(source)
                .map(PanelAttachmentAuthored::target),
            Some(target),
        );
    }

    #[test]
    fn queued_conversion_then_attach_converts_and_rejects_the_stale_attachment() {
        let mut app = conversion_test_app();
        let source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();

        let source_handle = world_handle(&mut app, source);
        let target_handle = world_handle(&mut app, target);
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                let result = commands.apply_to_screen(
                    source_handle,
                    PanelScreenConversion::at_pixels(Vec2::new(50.0, 60.0), Vec2::new(80.0, 40.0)),
                );
                commands.attach_to_panel(
                    source_handle,
                    target_handle,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
                result
            })
            .expect("mixed-operation system runs")
            .expect("screen conversion recipe is valid");

        assert!(
            app.world()
                .get::<DiegeticPanel>(source)
                .is_some_and(|panel| panel.coordinate_space().is_screen()),
        );
        assert!(
            !app.world()
                .entity(source)
                .contains::<PanelAttachmentAuthored>()
        );
    }

    #[test]
    fn queued_detach_then_conversion_applies_both_operations_in_order() {
        let mut app = conversion_test_app();
        let source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        app.update();
        attach_world_panel(&mut app, source, target);

        let source_handle = world_handle(&mut app, source);
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.detach(source_handle);
                commands.apply_to_screen(
                    source_handle,
                    PanelScreenConversion::at_pixels(Vec2::new(50.0, 60.0), Vec2::new(80.0, 40.0)),
                )
            })
            .expect("mixed-operation system runs")
            .expect("screen conversion recipe is valid");

        assert!(
            app.world()
                .get::<DiegeticPanel>(source)
                .is_some_and(|panel| panel.coordinate_space().is_screen()),
        );
        assert!(
            !app.world()
                .entity(source)
                .contains::<PanelAttachmentAuthored>()
        );
    }

    #[test]
    fn detach_convert_and_reattach_round_trip_reacquires_destination_handles() {
        let mut app = conversion_test_app();
        let source = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::default()))
            .id();
        let target = app
            .world_mut()
            .spawn((empty_world_panel(), Transform::from_xyz(2.0, 0.0, 0.0)))
            .id();
        app.update();
        attach_world_panel(&mut app, source, target);

        let source_world = world_handle(&mut app, source);
        app.world_mut()
            .run_system_once(move |mut attachments: Commands| attachments.detach(source_world))
            .expect("detach system runs");

        let source_world = world_handle(&mut app, source);
        let target_world = world_handle(&mut app, target);
        app.world_mut()
            .run_system_once(move |mut conversions: Commands| {
                conversions.apply_to_screen(
                    source_world,
                    PanelScreenConversion::at_pixels(
                        Vec2::new(100.0, 120.0),
                        Vec2::new(200.0, 100.0),
                    ),
                )?;
                conversions.apply_to_screen(
                    target_world,
                    PanelScreenConversion::at_pixels(
                        Vec2::new(300.0, 120.0),
                        Vec2::new(200.0, 100.0),
                    ),
                )?;
                Ok::<_, PanelProjectionError>(())
            })
            .expect("screen conversion system runs")
            .expect("detached panels can convert to screen space");
        app.update();

        let (source_screen, target_screen) = app
            .world_mut()
            .run_system_once(move |reader: PanelEntityReader| {
                (reader.screen(source), reader.screen(target))
            })
            .expect("screen handle reader runs");
        let source_screen = source_screen.expect("source conversion applied");
        let target_screen = target_screen.expect("target conversion applied");
        app.world_mut()
            .run_system_once(move |mut attachments: Commands| {
                attachments.attach_to_panel(
                    source_screen,
                    target_screen,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
            })
            .expect("screen attachment system runs");

        app.world_mut()
            .run_system_once(move |mut attachments: Commands| attachments.detach(source_screen))
            .expect("detach system runs");
        app.world_mut()
            .run_system_once(move |mut conversions: Commands| {
                conversions.apply_to_world(source_screen, world_projection(source, Vec3::ZERO))?;
                conversions
                    .apply_to_world(target_screen, world_projection(target, Vec3::X * 2.0))?;
                Ok::<_, PanelProjectionError>(())
            })
            .expect("world conversion system runs")
            .expect("detached panels can convert to world space");
        app.update();

        let (source_world, target_world) = app
            .world_mut()
            .run_system_once(move |reader: PanelEntityReader| {
                (reader.world(source), reader.world(target))
            })
            .expect("world handle reader runs");
        let source_world = source_world.expect("source conversion applied");
        let target_world = target_world.expect("target conversion applied");
        app.world_mut()
            .run_system_once(move |mut attachments: Commands| {
                attachments.attach_to_panel(
                    source_world,
                    target_world,
                    PanelAttachment::new(Anchor::BottomLeft, Anchor::TopLeft),
                );
            })
            .expect("world attachment system runs");

        assert!(
            app.world()
                .get::<DiegeticPanel>(source)
                .is_some_and(|panel| !panel.coordinate_space().is_screen()),
        );
        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(source)
                .map(PanelAttachmentAuthored::target),
            Some(target),
        );
    }
}
