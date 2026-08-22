//! Example demonstrating explicit path configuration with `WindowManagerPlugin`.
//!
//! Run with: `cargo run --example custom_path`
//!
//! This shows how to manually construct a cross-platform config path using `dirs`,
//! giving you full control over the app name and filename. Of course you can put it anywhere you
//! want, we're just using `dirs` for convenience in this example.
//!
//! Window state is saved to:
//! - macOS: `~/Library/Application Support/my_custom_app/window_state.ron`
//! - Linux: `~/.config/my_custom_app/window_state.ron`
//! - Windows: `C:\Users\{user}\AppData\Roaming\my_custom_app\window_state.ron`

use bevy::prelude::App;
use bevy::prelude::Camera2d;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::FontSize;
use bevy::prelude::Node;
use bevy::prelude::PluginGroup;
use bevy::prelude::PositionType;
use bevy::prelude::Query;
use bevy::prelude::Single;
use bevy::prelude::Startup;
use bevy::prelude::Text;
use bevy::prelude::TextFont;
use bevy::prelude::Update;
use bevy::prelude::Val;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::window::Monitor;
use bevy::window::PrimaryWindow;
use dirs::config_dir;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::WindowManagerPlugin;

const APP_DIRECTORY_NAME: &str = "my_custom_app";
const PRIMARY_WINDOW_TITLE: &str = "Custom Path Example";
const STATE_FILE_NAME: &str = "window_state.ron";
const NOT_AVAILABLE_TEXT: &str = "N/A";
const FONT_SIZE: f32 = 20.0;
const MARGIN: Val = Val::Px(10.0);
const MILLIHERTZ_PER_HERTZ: u32 = 1000;

#[derive(Component)]
struct InfoText;

#[expect(
    clippy::expect_used,
    reason = "example code - panicking on missing config dir is acceptable"
)]
fn main() {
    // Construct a cross-platform config path manually.
    let config_path = config_dir()
        .expect("Could not find config directory")
        .join(APP_DIRECTORY_NAME)
        .join(STATE_FILE_NAME);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: PRIMARY_WINDOW_TITLE.to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(WindowManagerPlugin::with_path(config_path))
        .add_systems(Startup, setup)
        .add_systems(Update, update_info_text)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        InfoText,
        Text::default(),
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
    ));
}

fn update_info_text(
    window_query: Single<(&Window, &CurrentMonitor), With<PrimaryWindow>>,
    bevy_monitors: Query<&Monitor>,
    mut text: Single<&mut Text, With<InfoText>>,
) {
    let (window, current_monitor) = *window_query;
    let effective_window_mode = current_monitor.effective_window_mode;

    // Find refresh rate from Bevy's `Monitor` by matching position.
    let refresh_rate = bevy_monitors
        .iter()
        .find(|monitor| monitor.physical_position == current_monitor.physical_position)
        .and_then(|monitor| monitor.refresh_rate_millihertz)
        .map(|refresh_rate| refresh_rate / MILLIHERTZ_PER_HERTZ);

    let refresh_display =
        refresh_rate.map_or_else(|| NOT_AVAILABLE_TEXT.into(), |hz| format!("{hz}Hz"));

    text.0 = format!(
        "Window Position: {:?}\n\
         Window Size: {}x{}\n\
         Mode: {:?} (set value only, not dynamically updated)\n\
         Effective Mode: {:?}\n\
         \n\
         Monitor {}\n\
         Position: ({}, {})\n\
         Size: {}x{}\n\
         Scale: {}\n\
         Refresh Rate: {}",
        window.position,
        window.physical_width(),
        window.physical_height(),
        window.mode,
        effective_window_mode,
        current_monitor.index,
        current_monitor.physical_position.x,
        current_monitor.physical_position.y,
        current_monitor.physical_size.x,
        current_monitor.physical_size.y,
        current_monitor.scale,
        refresh_display
    );
}
