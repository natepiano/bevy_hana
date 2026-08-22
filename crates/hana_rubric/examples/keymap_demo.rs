//! A headless downstream application that runs once to publish a default keymap and JSON Schema.
//!
//! This demonstrates one application-owned state dimension. The `with_app_name`,
//! `with_defaults`, and `with_state_dimension` builder calls publish the companion files and
//! declare the complete vocabulary JSONC predicates may use. The example exits after one frame;
//! it exists to publish the default document and schema rather than to handle input.

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::input::ButtonInput;
use bevy::input::InputSystems;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::App;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::PluginGroup;
use bevy::prelude::PreUpdate;
use bevy::prelude::States;
use bevy::prelude::World;
use bevy::state::app::AppExtStates;
use bevy::state::app::StatesPlugin;
use hana_rubric::KeymapPlugin;
use hana_rubric::KeymapSystems;
use hana_rubric::cancel_pending_sequences;
use strum::AsRefStr;
use strum::EnumIter;
use strum::EnumMessage;

mod commands {
    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy_enhanced_input::prelude::InputAction;
    use hana_rubric::ReflectKeymapCommand;
    use hana_rubric::command;

    command! {
        action:      OpenPaletteAction,
        event:       OpenPalette,
        id:          "demo::open_palette",
        title:       "Open Command Palette",
        description: "Opens the command palette for the keymap demo.",
    }

    command! {
        action:      OpenRecentAction,
        event:       OpenRecent,
        id:          "demo::open_recent",
        title:       "Open Recent Item",
        description: "Opens the most recently used item.",
    }

    command! {
        held,
        action:      MoveSelectionAction,
        event:       MoveSelection,
        id:          "demo::move_selection",
        title:       "Move Selection",
        description: "Moves the current selection while the key is held.",
    }

    command! {
        action:      SubmitEditAction,
        event:       SubmitEdit,
        id:          "demo::submit_edit",
        title:       "Submit Edit",
        description: "Submits the active edit.",
    }
}

#[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
#[strum(serialize_all = "snake_case")]
enum DemoContext {
    #[strum(message = "While the keymap demo is browsing commands")]
    Browsing,
    #[strum(message = "While the keymap demo is editing text")]
    Editing,
}

fn main() {
    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(StatesPlugin)
        .insert_state(DemoContext::Browsing)
        .register_type::<commands::OpenPalette>()
        .register_type::<commands::OpenRecent>()
        .register_type::<commands::MoveSelection>()
        .register_type::<commands::SubmitEdit>()
        .add_plugins(
            KeymapPlugin::new()
                .with_app_name("keymap_demo")
                .with_defaults(include_str!("keymap_demo.jsonc"))
                .with_state_dimension::<DemoContext>("application"),
        )
        .add_systems(
            PreUpdate,
            cancel_sequences_on_escape
                .after(InputSystems)
                .before(KeymapSystems::Route),
        )
        .run();
}

fn cancel_sequences_on_escape(world: &mut World) {
    let escape_was_pressed = world
        .get_resource::<ButtonInput<KeyCode>>()
        .is_some_and(|keyboard| keyboard.just_pressed(KeyCode::Escape));

    if escape_was_pressed {
        cancel_pending_sequences(world);
    }
}
