use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FontSize;
use bevy::prelude::GlobalZIndex;
use bevy::prelude::Node;
use bevy::prelude::Pickable;
use bevy::prelude::PositionType;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::TextFont;
use bevy::prelude::Val;
use bevy::prelude::Vec2;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::ui::UiTargetCamera;
use hana_kana::ScreenPosition;

use super::visual::FitOverlayVisual;
use super::visual::FitOverlayVisualKind;
use crate::fit::geometry::ScreenSpaceBounds;
use crate::fit::overlay::constants::BOUNDS_LABEL_COLOR;
use crate::fit::overlay::constants::BOUNDS_LABEL_TEXT;
use crate::fit::overlay::constants::LABEL_FONT_SIZE;
use crate::fit::overlay::constants::LABEL_PIXEL_OFFSET;
use crate::fit::overlay::constants::OVERLAY_LABEL_Z_INDEX;
use crate::fit::overlay::geometry;
use crate::fit::overlay::geometry::Edge;

/// Component marking a margin percentage label.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct MarginLabel;

/// Parameters for creating or updating a margin label.
pub(super) struct MarginLabelParameters {
    pub(super) camera:          Entity,
    pub(super) ui_camera:       Entity,
    pub(super) edge:            Edge,
    pub(super) text:            String,
    pub(super) color:           Color,
    pub(super) screen_position: ScreenPosition,
    pub(super) viewport_size:   Vec2,
}

/// Component marking the "screen space bounds" label.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct BoundsLabel;

/// Calculates the viewport pixel position for a margin label, offset by a fixed
/// number of pixels from the screen-edge endpoint of the margin line.
pub(super) fn calculate_label_pixel_position(
    edge: Edge,
    bounds: &ScreenSpaceBounds,
    viewport_size: Vec2,
) -> ScreenPosition {
    let (screen_x, screen_y) = geometry::screen_edge_center(bounds, edge);
    let pixel_position = geometry::norm_to_viewport(
        screen_x,
        screen_y,
        bounds.half_extent_x,
        bounds.half_extent_y,
        viewport_size,
    );

    // Left/Right labels sit above the horizontal line;
    // Top/Bottom labels sit beside the vertical line with pixel offsets.
    let above_line = pixel_position.y - LABEL_FONT_SIZE - LABEL_PIXEL_OFFSET;

    match edge {
        Edge::Left => ScreenPosition::new(LABEL_PIXEL_OFFSET, above_line),
        Edge::Right => ScreenPosition::new(viewport_size.x - LABEL_PIXEL_OFFSET, above_line),
        Edge::Top => ScreenPosition::new(pixel_position.x + LABEL_PIXEL_OFFSET, LABEL_PIXEL_OFFSET),
        Edge::Bottom => ScreenPosition::new(
            pixel_position.x + LABEL_PIXEL_OFFSET,
            viewport_size.y - LABEL_PIXEL_OFFSET,
        ),
    }
}

/// Returns the final viewport position for the "screen space bounds" label.
pub(super) fn bounds_label_position(upper_left: ScreenPosition) -> ScreenPosition {
    ScreenPosition::new(
        upper_left.x + LABEL_PIXEL_OFFSET,
        upper_left.y - LABEL_FONT_SIZE - LABEL_PIXEL_OFFSET,
    )
}

/// Applies anchored placement for a margin label node based on edge semantics.
fn apply_margin_label_anchor(
    node: &mut Node,
    edge: Edge,
    screen_position: ScreenPosition,
    viewport_size: Vec2,
) {
    match edge {
        Edge::Left | Edge::Top => {
            node.left = Val::Px(screen_position.x);
            node.top = Val::Px(screen_position.y);
            node.right = Val::Auto;
            node.bottom = Val::Auto;
        },
        Edge::Right => {
            node.right = Val::Px(viewport_size.x - screen_position.x);
            node.top = Val::Px(screen_position.y);
            node.left = Val::Auto;
            node.bottom = Val::Auto;
        },
        Edge::Bottom => {
            node.left = Val::Px(screen_position.x);
            node.bottom = Val::Px(viewport_size.y - screen_position.y);
            node.right = Val::Auto;
            node.top = Val::Auto;
        },
    }
}

/// Builds an anchored node for a new margin label.
fn margin_label_node(edge: Edge, screen_position: ScreenPosition, viewport_size: Vec2) -> Node {
    let mut node = Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    apply_margin_label_anchor(&mut node, edge, screen_position, viewport_size);
    node
}

fn label_root_components() -> impl Bundle {
    (GlobalZIndex(OVERLAY_LABEL_Z_INDEX), Pickable::IGNORE)
}

fn repair_label_root(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).insert(label_root_components());
}

/// Updates an existing margin label or creates a new one.
pub(super) fn update_or_create_margin_label(
    commands: &mut Commands,
    label_query: &mut Query<(
        Entity,
        &MarginLabel,
        &FitOverlayVisual,
        &mut Text,
        &mut Node,
        &mut TextColor,
        &mut UiTargetCamera,
    )>,
    parameters: MarginLabelParameters,
) {
    let existing = label_query.iter_mut().find(|(_, _, visual, _, _, _, _)| {
        visual.camera == parameters.camera
            && visual.kind
                == FitOverlayVisualKind::MarginLabel {
                    edge: parameters.edge,
                }
    });

    if let Some((entity, _, _, mut label_text, mut node, mut text_color, mut ui_camera)) = existing
    {
        (**label_text).clone_from(&parameters.text);
        text_color.0 = parameters.color;
        ui_camera.0 = parameters.ui_camera;
        repair_label_root(commands, entity);
        apply_margin_label_anchor(
            &mut node,
            parameters.edge,
            parameters.screen_position,
            parameters.viewport_size,
        );
    } else {
        commands.spawn((
            Text::new(parameters.text),
            TextFont {
                font_size: FontSize::Px(LABEL_FONT_SIZE),
                ..default()
            },
            TextColor(parameters.color),
            margin_label_node(
                parameters.edge,
                parameters.screen_position,
                parameters.viewport_size,
            ),
            MarginLabel,
            FitOverlayVisual {
                camera: parameters.camera,
                kind:   FitOverlayVisualKind::MarginLabel {
                    edge: parameters.edge,
                },
            },
            UiTargetCamera(parameters.ui_camera),
            label_root_components(),
        ));
    }
}

/// Updates an existing bounds label position or creates a new one.
pub(super) fn update_or_create_bounds_label(
    commands: &mut Commands,
    bounds_query: &mut Query<
        (
            Entity,
            &BoundsLabel,
            &FitOverlayVisual,
            &mut Node,
            &mut UiTargetCamera,
        ),
        Without<MarginLabel>,
    >,
    camera: Entity,
    ui_camera: Entity,
    screen_position: ScreenPosition,
) {
    let existing = bounds_query.iter_mut().find(|(_, _, visual, _, _)| {
        visual.camera == camera && visual.kind == FitOverlayVisualKind::BoundsLabel
    });

    if let Some((entity, _, _, mut node, mut target_camera)) = existing {
        node.left = Val::Px(screen_position.x);
        node.top = Val::Px(screen_position.y);
        target_camera.0 = ui_camera;
        repair_label_root(commands, entity);
    } else {
        commands.spawn((
            Text::new(BOUNDS_LABEL_TEXT),
            TextFont {
                font_size: FontSize::Px(LABEL_FONT_SIZE),
                ..default()
            },
            TextColor(BOUNDS_LABEL_COLOR),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(screen_position.x),
                top: Val::Px(screen_position.y),
                ..default()
            },
            BoundsLabel,
            FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::BoundsLabel,
            },
            UiTargetCamera(ui_camera),
            label_root_components(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Res;
    use bevy::prelude::Resource;
    use bevy::prelude::Update;
    use hana_kana::ScreenPosition;

    use super::*;

    #[derive(Resource)]
    struct TestLabelEntities {
        camera:    Entity,
        ui_camera: Entity,
    }

    #[test]
    fn margin_label_root_spawns_high_z_and_ignored_picking() {
        let mut app = test_app();
        app.add_systems(Update, spawn_test_margin_label);

        app.update();

        let camera = app.world().resource::<TestLabelEntities>().camera;
        let mut query =
            app.world_mut()
                .query::<(&FitOverlayVisual, &GlobalZIndex, &Pickable, &UiTargetCamera)>();
        let rows: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(rows.len(), 1);
        let (visual, z_index, pickable, ui_camera) = rows[0];
        assert_eq!(visual.camera, camera);
        assert_eq!(
            visual.kind,
            FitOverlayVisualKind::MarginLabel { edge: Edge::Left }
        );
        assert_eq!(z_index, &GlobalZIndex(OVERLAY_LABEL_Z_INDEX));
        assert_eq!(pickable, &Pickable::IGNORE);
        assert_eq!(
            ui_camera.0,
            app.world().resource::<TestLabelEntities>().ui_camera
        );
    }

    #[test]
    fn bounds_label_update_repairs_high_z_and_ignored_picking() {
        let mut app = test_app();
        let entities = app.world().resource::<TestLabelEntities>();
        let camera = entities.camera;
        app.world_mut().spawn((
            Text::new(BOUNDS_LABEL_TEXT),
            TextFont {
                font_size: FontSize::Px(LABEL_FONT_SIZE),
                ..default()
            },
            TextColor(BOUNDS_LABEL_COLOR),
            Node::default(),
            BoundsLabel,
            FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::BoundsLabel,
            },
            UiTargetCamera(camera),
        ));
        app.add_systems(Update, update_test_bounds_label);

        app.update();

        let mut query =
            app.world_mut()
                .query::<(&FitOverlayVisual, &GlobalZIndex, &Pickable, &UiTargetCamera)>();
        let rows: Vec<_> = query.iter(app.world()).collect();
        assert_eq!(rows.len(), 1);
        let (_, z_index, pickable, ui_camera) = rows[0];
        assert_eq!(z_index, &GlobalZIndex(OVERLAY_LABEL_Z_INDEX));
        assert_eq!(pickable, &Pickable::IGNORE);
        assert_eq!(
            ui_camera.0,
            app.world().resource::<TestLabelEntities>().ui_camera
        );
    }

    fn test_app() -> App {
        let mut app = App::new();
        let camera = app.world_mut().spawn_empty().id();
        let ui_camera = app.world_mut().spawn_empty().id();
        app.insert_resource(TestLabelEntities { camera, ui_camera });
        app
    }

    fn spawn_test_margin_label(
        mut commands: Commands,
        entities: Res<TestLabelEntities>,
        mut label_query: Query<(
            Entity,
            &MarginLabel,
            &FitOverlayVisual,
            &mut Text,
            &mut Node,
            &mut TextColor,
            &mut UiTargetCamera,
        )>,
    ) {
        update_or_create_margin_label(
            &mut commands,
            &mut label_query,
            MarginLabelParameters {
                camera:          entities.camera,
                ui_camera:       entities.ui_camera,
                edge:            Edge::Left,
                text:            "margin: 1.000%".to_string(),
                color:           Color::WHITE,
                screen_position: ScreenPosition::new(8.0, 8.0),
                viewport_size:   Vec2::new(640.0, 480.0),
            },
        );
    }

    fn update_test_bounds_label(
        mut commands: Commands,
        entities: Res<TestLabelEntities>,
        mut bounds_query: Query<
            (
                Entity,
                &BoundsLabel,
                &FitOverlayVisual,
                &mut Node,
                &mut UiTargetCamera,
            ),
            Without<MarginLabel>,
        >,
    ) {
        update_or_create_bounds_label(
            &mut commands,
            &mut bounds_query,
            entities.camera,
            entities.ui_camera,
            ScreenPosition::new(16.0, 16.0),
        );
    }
}
