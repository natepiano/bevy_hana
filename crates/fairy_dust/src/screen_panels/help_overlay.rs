//! Keyboard shortcut help overlay for title bars.

use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::bindings;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_rubric::CommandId;
use hana_rubric::CommandKeystroke;
use hana_rubric::KeymapBindingUnavailability;
use hana_rubric::KeymapBindings;
use hana_rubric::KeymapCommand;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::action;
use hana_rubric::bind_action_system;
use hana_rubric::command;
use hana_rubric::event;

use super::ControlActivation;
use super::TitleBarControlState;
use super::constants::BODY_COLOR;
use super::constants::CAMERA_PRESET_LABEL;
use super::constants::CLOSE_HINT;
use super::constants::DIVIDER_COLOR;
use super::constants::HELP_CLOSE_CONTEXT_PRIORITY;
use super::constants::HELP_CLOSE_HINT_COLUMN_WIDTH;
use super::constants::HELP_CLOSE_HINT_SIZE;
use super::constants::HELP_CONTROL;
use super::constants::HELP_KEY_COLUMN_WIDTH;
use super::constants::HELP_PANEL_CHILD_GAP;
use super::constants::HELP_ROW_GAP;
use super::constants::HELP_SEPARATOR_HEIGHT;
use super::constants::HELP_TABLE_COLUMN_GAP;
use super::constants::HELP_TITLE;
use super::constants::HOME_AABB_LABEL;
use super::constants::SCREEN_PANEL_LABEL;
use super::constants::UNBOUND_KEYS;
use super::default_inner_background;
use super::screen_panel_frame;
use crate::camera_control_panel::CameraGuidancePanel;
use crate::camera_control_panel::CameraPresetSwitching;
use crate::camera_control_panel::CyclePresetEvent;
use crate::camera_home::CameraHomeMarker;
use crate::camera_home::ToggleHomeAabbGizmoEvent;
use crate::constants::LABEL_SIZE;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;
use crate::ensure_plugin;
use crate::screen_space_lights::ToggleScreenSpacePanelsEvent;

/// Higher-priority context inserted on the `KeyboardShortcutHelp` overlay
/// entity. It owns the `CloseHelp`/Esc action while that entity exists and
/// consumes Esc so closing the overlay does not also fire a caller's Esc binding.
#[derive(Component)]
struct HelpCloseContext;

command! {
    action:      ShowHelp,
    event:       ShowHelpEvent,
    id:          "fairy_dust::show_help",
    title:       "Show Keyboard Shortcuts",
    description: "Open the keyboard shortcut overlay, or close it when it is already open.",
}

// `CloseHelp` stays on the input-action layer: it is bound inside
// `HelpCloseContext` with `consume_input`, which is what stops Esc from also
// firing a caller's Esc binding while the overlay is open.
action!(CloseHelp);
event!(CloseHelpEvent);

#[derive(Component)]
pub(super) struct KeyboardShortcutHelp;

#[derive(Clone, Copy)]
enum ShortcutPresence {
    Present,
    Absent,
}

impl ShortcutPresence {
    const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

#[derive(Clone, Copy)]
struct HelpShortcuts {
    home_marker:   ShortcutPresence,
    camera_preset: ShortcutPresence,
}

struct HelpRow {
    keys:  String,
    label: &'static str,
}

pub(super) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.add_input_context::<HelpCloseContext>();
    app.add_observer(show_or_toggle_help);
    bind_action_system!(app, CloseHelp, CloseHelpEvent, close_help);
}

/// Toggles the overlay on Shift+/: despawns it when open, otherwise spawns it
/// (reading which optional shortcuts apply).
fn show_or_toggle_help(
    _: On<ShowHelpEvent>,
    mut commands: Commands,
    overlay: Query<Entity, With<KeyboardShortcutHelp>>,
    home_markers: Query<Entity, With<CameraHomeMarker>>,
    camera_panels: Query<Entity, With<CameraGuidancePanel>>,
    preset_switching: Option<Res<CameraPresetSwitching>>,
    keymap_bindings: Res<KeymapBindings>,
    mut bars: Query<&mut TitleBarControlState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    fonts: Res<FontRegistry>,
) {
    if !overlay.is_empty() {
        for entity in &overlay {
            commands.entity(entity).despawn();
        }
        set_help_chip(&mut bars, ControlActivation::Inactive);
        return;
    }

    let preset_switching_enabled =
        preset_switching.is_none_or(|switching| *switching == CameraPresetSwitching::Enabled);
    let home_marker = if home_markers.is_empty() {
        ShortcutPresence::Absent
    } else {
        ShortcutPresence::Present
    };
    let camera_preset = if !camera_panels.is_empty() && preset_switching_enabled {
        ShortcutPresence::Present
    } else {
        ShortcutPresence::Absent
    };
    spawn_help_overlay(
        &mut commands,
        shortcut_rows(
            HelpShortcuts {
                home_marker,
                camera_preset,
            },
            &keymap_bindings,
        ),
        &fonts,
        &mut materials,
    );
    set_help_chip(&mut bars, ControlActivation::Active);
}

/// Closes the overlay on Esc. Bound inside [`HelpCloseContext`], which consumes
/// Esc so a caller's Esc binding doesn't also fire while the overlay is open.
fn close_help(
    mut commands: Commands,
    overlay: Query<Entity, With<KeyboardShortcutHelp>>,
    mut bars: Query<&mut TitleBarControlState>,
) {
    for entity in &overlay {
        commands.entity(entity).despawn();
    }
    set_help_chip(&mut bars, ControlActivation::Inactive);
}

/// Highlights or clears the always-present `?` help chip on every title bar.
fn set_help_chip(bars: &mut Query<&mut TitleBarControlState>, activation: ControlActivation) {
    for mut bar in bars.iter_mut() {
        bar.set_active(HELP_CONTROL, activation);
    }
}

fn spawn_help_overlay(
    commands: &mut Commands,
    rows: Vec<HelpRow>,
    fonts: &FontRegistry,
    materials: &mut Assets<StandardMaterial>,
) {
    let unlit = super::screen_panel_material_handle(materials);
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::Center)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_help_tree(rows, fonts))
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((
                KeyboardShortcutHelp,
                panel,
                Transform::default(),
                HelpCloseContext,
                ContextPriority::<HelpCloseContext>::new(HELP_CLOSE_CONTEXT_PRIORITY),
                Actions::<HelpCloseContext>::spawn(SpawnWith(
                    |spawner: &mut ActionSpawner<HelpCloseContext>| {
                        spawner.spawn((
                            Action::<CloseHelp>::new(),
                            ActionSettings {
                                consume_input: true,
                                ..default()
                            },
                            bindings![KeyCode::Escape],
                        ));
                    },
                )),
            ));
        },
        Err(error) => {
            error!("fairy_dust: failed to build keyboard shortcut help: {error}");
        },
    }
}

fn build_help_tree(rows: Vec<HelpRow>, fonts: &FontRegistry) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_help_layout(&mut builder, rows, fonts);
    builder.build()
}

fn build_help_layout(builder: &mut LayoutBuilder, rows: Vec<HelpRow>, fonts: &FontRegistry) {
    let title =
        TextStyle::new(super::integral_advance_size(fonts, TITLE_SIZE)).with_color(TITLE_COLOR);
    let hint = TextStyle::new(super::integral_advance_size(fonts, HELP_CLOSE_HINT_SIZE))
        .with_color(BODY_COLOR);
    let label =
        TextStyle::new(super::integral_advance_size(fonts, LABEL_SIZE)).with_color(BODY_COLOR);

    screen_panel_frame(
        builder,
        Sizing::FIT,
        Sizing::FIT,
        default_inner_background(),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(HELP_PANEL_CHILD_GAP),
                |builder| {
                    build_title_row(builder, &title, &hint);
                    build_separator(builder);
                    build_shortcut_table(builder, rows, &label);
                },
            );
        },
    );
}

fn build_title_row(builder: &mut LayoutBuilder, title: &TextStyle, hint: &TextStyle) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(HELP_TABLE_COLUMN_GAP)
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::new().width(Sizing::GROW).height(Sizing::FIT),
                |builder| {
                    builder.text((HELP_TITLE, title.clone()));
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(HELP_CLOSE_HINT_COLUMN_WIDTH))
                    .height(Sizing::FIT)
                    .align_x(AlignX::Right),
                |builder| {
                    builder.text((CLOSE_HINT, hint.clone()));
                },
            );
        },
    );
}

fn build_separator(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(HELP_SEPARATOR_HEIGHT))
            .background(DIVIDER_COLOR),
        |_| {},
    );
}

fn build_shortcut_table(builder: &mut LayoutBuilder, rows: Vec<HelpRow>, label: &TextStyle) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(HELP_ROW_GAP),
        |builder| {
            for row in rows {
                build_shortcut_row(builder, row, label);
            }
        },
    );
}

fn build_shortcut_row(builder: &mut LayoutBuilder, row: HelpRow, label: &TextStyle) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(HELP_TABLE_COLUMN_GAP)
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(HELP_KEY_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((row.keys.as_str(), label.clone()));
                },
            );
            builder.with(
                El::new().width(Sizing::FIT).height(Sizing::FIT),
                |builder| {
                    builder.text((row.label, label.clone()));
                },
            );
        },
    );
}

/// Builds the table from the live keymap rather than from written-out chords, so
/// a rebound capability prints the keys that now run it.
fn shortcut_rows(shortcuts: HelpShortcuts, keymap_bindings: &KeymapBindings) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    if shortcuts.home_marker.is_present() {
        rows.push(HelpRow {
            keys:  bound_keys::<ToggleHomeAabbGizmoEvent>(keymap_bindings),
            label: HOME_AABB_LABEL,
        });
    }
    rows.push(HelpRow {
        keys:  bound_keys::<ToggleScreenSpacePanelsEvent>(keymap_bindings),
        label: SCREEN_PANEL_LABEL,
    });
    if shortcuts.camera_preset.is_present() {
        rows.push(HelpRow {
            keys:  bound_keys::<CyclePresetEvent>(keymap_bindings),
            label: CAMERA_PRESET_LABEL,
        });
    }
    rows
}

/// The keys the live keymap runs `C` from, read when the overlay is built:
/// [`KeymapBindings`] is replaced on every keymap commit, so a value read once
/// at startup would print the chord a later user keymap replaced.
fn bound_keys<C: KeymapCommand>(keymap_bindings: &KeymapBindings) -> String {
    match keymap_bindings.keystroke(&CommandId::declared::<C>()) {
        CommandKeystroke::BoundTo(keystroke_sequence) => keystroke_sequence.to_string(),
        CommandKeystroke::Unbound => UNBOUND_KEYS.to_owned(),
        CommandKeystroke::KeymapUnavailable(unavailability) => {
            keymap_unavailability_label(unavailability).to_owned()
        },
    }
}

/// The key column's short explanation when no committed keymap table exists.
const fn keymap_unavailability_label(unavailability: KeymapBindingUnavailability) -> &'static str {
    match unavailability {
        KeymapBindingUnavailability::AwaitingInitialLoad => "keymap loading",
        KeymapBindingUnavailability::Unconfigured => "keymap unconfigured",
        KeymapBindingUnavailability::MissingDefault => "embedded defaults missing",
        KeymapBindingUnavailability::InvalidDefault => "embedded defaults invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_row() -> HelpShortcuts {
        HelpShortcuts {
            home_marker:   ShortcutPresence::Present,
            camera_preset: ShortcutPresence::Present,
        }
    }

    /// The overlay prints what the keymap runs, so a user document that rebinds
    /// a capability moves the printed keys with it.
    #[test]
    fn each_row_prints_the_keys_the_live_keymap_binds() {
        let mut app = App::new();
        crate::keymap::install(&mut app);
        app.finish();
        let keymap_bindings = app.world().resource::<KeymapBindings>();

        let rows = shortcut_rows(every_row(), keymap_bindings);

        assert_eq!(
            rows.iter().map(|row| row.keys.clone()).collect::<Vec<_>>(),
            vec![
                bound_keys::<ToggleHomeAabbGizmoEvent>(keymap_bindings),
                bound_keys::<ToggleScreenSpacePanelsEvent>(keymap_bindings),
                bound_keys::<CyclePresetEvent>(keymap_bindings),
            ]
        );
        assert!(rows.iter().all(|row| row.keys != UNBOUND_KEYS));
    }

    /// A loaded keymap that binds none of them prints the authored-unbound label rather than a
    /// keymap availability label.
    #[test]
    fn a_loaded_keymap_that_binds_nothing_to_a_capability_prints_unbound() {
        let mut app = App::new();
        crate::keymap::configure(
            &mut app,
            crate::keymap::CommandPaletteKeymap::new(r#"{ "bindings": [] }"#),
        );
        crate::keymap::install(&mut app);
        app.finish();
        let rows = shortcut_rows(every_row(), app.world().resource::<KeymapBindings>());

        assert!(rows.iter().all(|row| row.keys == UNBOUND_KEYS));
    }

    #[test]
    fn unavailable_keymaps_print_each_reason_instead_of_the_authored_unbound_label() {
        for (unavailability, label) in [
            (
                KeymapBindingUnavailability::AwaitingInitialLoad,
                "keymap loading",
            ),
            (
                KeymapBindingUnavailability::Unconfigured,
                "keymap unconfigured",
            ),
            (
                KeymapBindingUnavailability::MissingDefault,
                "embedded defaults missing",
            ),
            (
                KeymapBindingUnavailability::InvalidDefault,
                "embedded defaults invalid",
            ),
        ] {
            let rows = shortcut_rows(every_row(), &KeymapBindings::from(unavailability));

            assert_eq!(keymap_unavailability_label(unavailability), label);
            assert!(rows.iter().all(|row| row.keys == label));
            assert!(rows.iter().all(|row| row.keys != UNBOUND_KEYS));
        }
    }
}
