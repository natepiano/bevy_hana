use bevy::camera::RenderTarget;
use bevy::prelude::Add;
use bevy::prelude::Camera2d;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::FontSize;
use bevy::prelude::Node;
use bevy::prelude::On;
use bevy::prelude::PositionType;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Text;
use bevy::prelude::TextFont;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::info;
use bevy::ui::UiTargetCamera;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowName;

use super::constants::FONT_SIZE;
use super::constants::MANAGED_WINDOW_NAME_PREFIX;
use super::constants::MANAGED_WINDOW_TITLE_PREFIX;
use super::constants::MARGIN;
use super::constants::SECONDARY_WINDOW_HEIGHT;
use super::constants::SECONDARY_WINDOW_WIDTH;
use super::display::PrimaryDisplay;
use super::display::SecondaryDisplay;
use super::events::SpawnManagedWindow;

#[derive(Resource, Default)]
pub(crate) struct WindowCounter {
    next: usize,
}

pub(crate) fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(FONT_SIZE),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: MARGIN,
            left: MARGIN,
            ..default()
        },
        PrimaryDisplay,
    ));
}

pub(crate) fn on_spawn_managed_window(
    _trigger: On<SpawnManagedWindow>,
    mut commands: Commands,
    mut window_counter: ResMut<WindowCounter>,
) {
    window_counter.next += 1;
    let name = format!("{MANAGED_WINDOW_NAME_PREFIX}{}", window_counter.next);
    let title = format!("{MANAGED_WINDOW_TITLE_PREFIX}{name}");

    commands.spawn((
        Window {
            title,
            resolution: WindowResolution::new(SECONDARY_WINDOW_WIDTH, SECONDARY_WINDOW_HEIGHT),
            ..default()
        },
        ManagedWindowName(name.clone()),
    ));

    info!("[restore_window] Spawned managed window \"{name}\"");
}

pub(crate) fn on_secondary_window_added(
    add: On<Add, ManagedWindow>,
    mut commands: Commands,
    primary_query: Query<(), With<PrimaryWindow>>,
) {
    let entity = add.entity;
    if primary_query.get(entity).is_ok() {
        return;
    }

    let camera = commands
        .spawn((Camera2d, RenderTarget::Window(WindowRef::Entity(entity))))
        .id();

    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(FONT_SIZE),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: MARGIN,
            left: MARGIN,
            ..default()
        },
        UiTargetCamera(camera),
        SecondaryDisplay(entity),
    ));
}

pub(crate) fn on_secondary_window_removed(
    remove: On<Remove, ManagedWindow>,
    mut commands: Commands,
    displays: Query<(Entity, &SecondaryDisplay)>,
) {
    let entity = remove.entity;
    for (display_entity, display) in &displays {
        if display.0 == entity {
            commands.entity(display_entity).despawn();
        }
    }
}
