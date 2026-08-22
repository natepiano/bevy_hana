//! Capability: Ctrl+Shift+L toggles screen-space panel visibility.
//!
//! Declared with `hana_rubric`'s `command!`, modeled on [`crate::restart`], so
//! the keymap owns the chord and the palette lists it.

use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::With;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_diegetic::ScreenSpaceCamera;
use hana_diegetic::ScreenSpaceLight;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;

use crate::ensure_plugin;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ScreenSpacePanelsVisibility {
    #[default]
    Shown,
    Hidden,
}

impl ScreenSpacePanelsVisibility {
    const fn toggled(self) -> Self {
        match self {
            Self::Shown => Self::Hidden,
            Self::Hidden => Self::Shown,
        }
    }
}

#[derive(Resource, Default)]
struct ScreenSpacePanelsEnabled(ScreenSpacePanelsVisibility);

command! {
    action:      ToggleScreenSpacePanels,
    event:       ToggleScreenSpacePanelsEvent,
    id:          "fairy_dust::toggle_screen_space_panels",
    title:       "Toggle Screen-Space Panels",
    description: "Hide or restore the screen-space panel camera and its lights.",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreenSpaceCameraState {
    Active,
    Inactive,
}

impl From<bool> for ScreenSpaceCameraState {
    fn from(is_active: bool) -> Self {
        if is_active {
            Self::Active
        } else {
            Self::Inactive
        }
    }
}

impl From<ScreenSpaceCameraState> for bool {
    fn from(state: ScreenSpaceCameraState) -> Self {
        matches!(state, ScreenSpaceCameraState::Active)
    }
}

#[derive(Component)]
struct ScreenSpaceCameraRestore {
    state: ScreenSpaceCameraState,
}

#[derive(Component)]
struct ScreenSpaceLightRestore {
    illuminance: f32,
}

pub(crate) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.init_resource::<ScreenSpacePanelsEnabled>();
    app.add_systems(Update, apply_screen_space_panels);
    app.add_observer(toggle_screen_space_panels);
}

fn toggle_screen_space_panels(
    _: On<ToggleScreenSpacePanelsEvent>,
    mut enabled: ResMut<ScreenSpacePanelsEnabled>,
) {
    enabled.0 = enabled.0.toggled();
}

fn apply_screen_space_panels(
    enabled: Res<ScreenSpacePanelsEnabled>,
    mut commands: Commands,
    mut cameras: Query<
        (Entity, &mut Camera, Option<&ScreenSpaceCameraRestore>),
        With<ScreenSpaceCamera>,
    >,
    mut lights: Query<
        (
            Entity,
            &mut DirectionalLight,
            Option<&ScreenSpaceLightRestore>,
        ),
        With<ScreenSpaceLight>,
    >,
) {
    for (entity, mut camera, restore) in &mut cameras {
        if enabled.0 == ScreenSpacePanelsVisibility::Shown {
            if let Some(restore) = restore {
                camera.is_active = restore.state.into();
                commands.entity(entity).remove::<ScreenSpaceCameraRestore>();
            }
        } else {
            if restore.is_none() {
                commands.entity(entity).insert(ScreenSpaceCameraRestore {
                    state: camera.is_active.into(),
                });
            }
            camera.is_active = false;
        }
    }

    for (entity, mut light, restore) in &mut lights {
        if enabled.0 == ScreenSpacePanelsVisibility::Shown {
            if let Some(restore) = restore {
                light.illuminance = restore.illuminance;
                commands.entity(entity).remove::<ScreenSpaceLightRestore>();
            }
        } else {
            if restore.is_none() {
                commands.entity(entity).insert(ScreenSpaceLightRestore {
                    illuminance: light.illuminance,
                });
            }
            light.illuminance = 0.0;
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::ButtonInput;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use hana_rubric::CommandId;
    use hana_rubric::CommandKeystroke;
    use hana_rubric::KeymapBindings;
    use hana_rubric::PrimaryTrigger;

    use super::*;

    /// Presses the keys the live keymap runs `command_id` from, in one frame, so
    /// the routed chord is the one the shipped document actually binds.
    fn press_bound_keystroke(app: &mut App, command_id: &CommandId) {
        let CommandKeystroke::BoundTo(keystroke_sequence) = app
            .world()
            .resource::<KeymapBindings>()
            .keystroke(command_id)
        else {
            panic!("`{command_id}` is unbound in the shipped defaults");
        };
        let keystroke = keystroke_sequence.first();
        let PrimaryTrigger::OrdinaryKey(ordinary_key) = keystroke.primary_trigger() else {
            panic!("`{command_id}` is bound to a bare modifier family");
        };
        let modifiers = keystroke.modifiers();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        if modifiers.has_control() {
            keys.press(KeyCode::ControlLeft);
        }
        if modifiers.has_shift() {
            keys.press(KeyCode::ShiftLeft);
        }
        if modifiers.has_alt() {
            keys.press(KeyCode::AltLeft);
        }
        if modifiers.has_platform() {
            keys.press(KeyCode::SuperLeft);
        }
        keys.press(ordinary_key.key_code());
    }

    /// The baseline keymap is what dispatches Fairy Dust's own capabilities, so
    /// an application that never opens the command palette still reaches this
    /// one from its keystroke.
    #[test]
    fn the_bound_chord_reaches_the_handler_without_the_palette() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ButtonInput<KeyCode>>();
        install(&mut app);
        crate::keymap::install(&mut app);
        app.finish();

        press_bound_keystroke(
            &mut app,
            &CommandId::declared::<ToggleScreenSpacePanelsEvent>(),
        );
        app.update();

        assert_eq!(
            app.world().resource::<ScreenSpacePanelsEnabled>().0,
            ScreenSpacePanelsVisibility::Hidden
        );
    }
}
