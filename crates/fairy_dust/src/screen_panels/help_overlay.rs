//! Keyboard shortcut help overlay for title bars.

use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::related;
use bevy::prelude::warn;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::Bindings;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Px;
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
use super::PanelTextEdge;
use super::TitleBar;
use super::TitleBarControlState;
use super::TitleBarOverflow;
use super::TitleBarOverflowSystems;
use super::constants::BODY_COLOR;
use super::constants::CAMERA_PRESET_LABEL;
use super::constants::CLOSE_HINT;
use super::constants::DIVIDER_COLOR;
use super::constants::HELP_CLOSE_CONTEXT_PRIORITY;
use super::constants::HELP_CLOSE_HINT_COLUMN_WIDTH;
use super::constants::HELP_CLOSE_HINT_SIZE;
use super::constants::HELP_CONTROL;
use super::constants::HELP_INNER_PADDING;
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
use super::title_bar;
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
    title:       "Toggle keyboard shortcuts",
    description: "Toggle the keyboard shortcut overlay.",
    toggle_state: keyboard_shortcuts_are_shown,
}

fn keyboard_shortcuts_are_shown(world: &World) -> bool {
    world
        .try_query_filtered::<(), With<KeyboardShortcutHelp>>()
        .is_some_and(|mut overlays| overlays.iter(world).next().is_some())
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

#[derive(Component, Clone, Copy)]
struct HelpShortcuts {
    home_marker:   ShortcutPresence,
    camera_preset: ShortcutPresence,
}

/// Text shown only in Help; title-bar labels remain their own presentation.
struct HelpPresentation {
    keys:  String,
    label: String,
}

impl HelpPresentation {
    fn from_control(control: &super::TitleBarControl) -> Self {
        let (keys, label) = control.help_parts();
        Self { keys, label }
    }
}

fn display_keymap_chord(keys: &str) -> String {
    keys.split(['-', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.add_input_context::<HelpCloseContext>();
    app.add_observer(show_or_toggle_help);
    bind_action_system!(app, CloseHelp, CloseHelpEvent, close_help);
    install_help_overlay_refresh(app);
}

fn install_help_overlay_refresh(app: &mut App) {
    app.add_systems(
        Update,
        refresh_help_overlay_for_overflow.in_set(TitleBarOverflowSystems::Read),
    );
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
    title_bars: Query<(&TitleBar, &TitleBarOverflow)>,
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
    let shortcuts = HelpShortcuts {
        home_marker,
        camera_preset,
    };
    spawn_help_overlay(
        &mut commands,
        shortcuts,
        shortcut_rows(shortcuts, &keymap_bindings)
            .into_iter()
            .chain(
                title_bars
                    .iter()
                    .flat_map(|(bar, overflow)| title_bar_help_rows(bar, overflow)),
            )
            .collect(),
        &fonts,
        &mut materials,
    );
    set_help_chip(&mut bars, ControlActivation::Active);
}

fn refresh_help_overlay_for_overflow(
    mut commands: Commands,
    overlays: Query<(Entity, &HelpShortcuts), With<KeyboardShortcutHelp>>,
    title_bars: Query<(&TitleBar, Ref<TitleBarOverflow>)>,
    keymap_bindings: Res<KeymapBindings>,
    fonts: Res<FontRegistry>,
) {
    if !keymap_bindings.is_changed()
        && !title_bars.iter().any(|(_, overflow)| overflow.is_changed())
    {
        return;
    }
    for (entity, shortcuts) in &overlays {
        let rows = shortcut_rows(*shortcuts, &keymap_bindings)
            .into_iter()
            .chain(
                title_bars
                    .iter()
                    .flat_map(|(bar, overflow)| title_bar_help_rows(bar, &overflow)),
            )
            .collect();
        if let Err(error) = commands.set_tree(entity, build_help_tree(rows, &fonts)) {
            warn!("failed to replace keyboard shortcut help {entity:?} tree: {error}");
        }
    }
}

fn title_bar_help_rows(bar: &TitleBar, overflow: &TitleBarOverflow) -> Vec<HelpPresentation> {
    title_bar::dropped_controls(bar, overflow)
        .map(HelpPresentation::from_control)
        .collect()
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
    shortcuts: HelpShortcuts,
    rows: Vec<HelpPresentation>,
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
                shortcuts,
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
                            related!(Bindings[Binding::from(KeyCode::Escape)]),
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

fn build_help_tree(rows: Vec<HelpPresentation>, fonts: &FontRegistry) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_help_layout(&mut builder, rows, fonts);
    builder.build()
}

fn build_help_layout(
    builder: &mut LayoutBuilder,
    rows: Vec<HelpPresentation>,
    fonts: &FontRegistry,
) {
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
                    .padding(Padding::new(
                        HELP_INNER_PADDING,
                        HELP_INNER_PADDING,
                        HELP_INNER_PADDING,
                        Px(0.0),
                    ))
                    .gap(HELP_PANEL_CHILD_GAP),
                |builder| {
                    build_title_row(builder, &title, &hint, fonts);
                    build_separator(builder);
                    build_shortcut_table(builder, rows, &label);
                },
            );
        },
    );
}

fn build_title_row(
    builder: &mut LayoutBuilder,
    title: &TextStyle,
    hint: &TextStyle,
    fonts: &FontRegistry,
) {
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
                    builder.text(super::screen_panel_ink_text(
                        HELP_TITLE,
                        title.clone(),
                        fonts,
                        TITLE_SIZE,
                        PanelTextEdge::First,
                    ));
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

fn build_shortcut_table(
    builder: &mut LayoutBuilder,
    rows: Vec<HelpPresentation>,
    label: &TextStyle,
) {
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

fn build_shortcut_row(builder: &mut LayoutBuilder, row: HelpPresentation, label: &TextStyle) {
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
fn shortcut_rows(
    shortcuts: HelpShortcuts,
    keymap_bindings: &KeymapBindings,
) -> Vec<HelpPresentation> {
    let mut rows = Vec::new();
    if shortcuts.home_marker.is_present() {
        rows.push(HelpPresentation {
            keys:  bound_keys::<ToggleHomeAabbGizmoEvent>(keymap_bindings),
            label: HOME_AABB_LABEL.to_owned(),
        });
    }
    rows.push(HelpPresentation {
        keys:  bound_keys::<ToggleScreenSpacePanelsEvent>(keymap_bindings),
        label: SCREEN_PANEL_LABEL.to_owned(),
    });
    if shortcuts.camera_preset.is_present() {
        rows.push(HelpPresentation {
            keys:  bound_keys::<CyclePresetEvent>(keymap_bindings),
            label: CAMERA_PRESET_LABEL.to_owned(),
        });
    }
    rows
}

/// The keys the live keymap runs `C` from, read when the overlay is built:
/// [`KeymapBindings`] is replaced on every keymap commit, so a value read once
/// at startup would print the chord a later user keymap replaced.
fn bound_keys<C: KeymapCommand>(keymap_bindings: &KeymapBindings) -> String {
    match keymap_bindings.keystroke(&CommandId::declared::<C>()) {
        CommandKeystroke::BoundTo(keystroke_sequence) => {
            display_keymap_chord(&keystroke_sequence.to_string())
        },
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

    #[test]
    fn command_palette_toggle_shortcuts_tracks_overlay_creation_and_removal() {
        let mut world = World::new();
        assert!(!keyboard_shortcuts_are_shown(&world));
        let overlay = world.spawn(KeyboardShortcutHelp).id();
        assert!(keyboard_shortcuts_are_shown(&world));
        world.despawn(overlay);
        assert!(!keyboard_shortcuts_are_shown(&world));
    }

    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::FontId;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::Unit;

    use super::*;
    use crate::screen_panels::HomeTitleBarFlash;

    #[test]
    fn help_control_wording_comes_from_control_data() {
        assert_eq!(HELP_TITLE, "Help");
        let click = super::super::TitleBarControl::from("Click Zoom in")
            .with_help_wording("Click", "Zoom in");
        let click_help = HelpPresentation::from_control(&click);
        assert_eq!(
            (click_help.keys.as_str(), click_help.label.as_str()),
            ("Click", "Zoom in")
        );

        let smoothing = super::super::TitleBarControl::segmented(
            "A",
            ["Off", "Anisotropic", "Supersample", "Both"]
                .map(|word| super::super::TitleBarSegment::new(word, word)),
        )
        .with_name("Smoothing");
        let smoothing_help = HelpPresentation::from_control(&smoothing);
        assert_eq!(smoothing_help.keys, "A");
        assert_eq!(
            smoothing_help.label,
            "Smoothing: Off, Anisotropic, Supersample, Both"
        );

        for (chip, key, action) in [
            ("- Remove", "-", "Remove"),
            ("1-7 Alpha mode", "1-7", "Alpha mode"),
        ] {
            let row = HelpPresentation::from_control(&super::super::TitleBarControl::from(
                chip.to_owned(),
            ));
            assert_eq!((row.keys.as_str(), row.label.as_str()), (key, action));
        }
        assert_eq!(display_keymap_chord("ctrl-shift-a"), "Ctrl Shift A");
        assert_eq!(display_keymap_chord("shift-c"), "Shift C");
        assert_eq!(display_keymap_chord("+"), "+");
        assert!(!smoothing_help.keys.contains('+'));
        let either_key =
            HelpPresentation::from_control(&super::super::TitleBarControl::from("↑ / ↓ Strength"));
        assert_eq!(
            (either_key.keys.as_str(), either_key.label.as_str()),
            ("↑ / ↓", "Strength")
        );
        let roll =
            HelpPresentation::from_control(&super::super::TitleBarControl::from("Q / E Roll"));
        assert_eq!((roll.keys.as_str(), roll.label.as_str()), ("Q / E", "Roll"));
    }

    #[test]
    fn close_hint_reaches_nine_pixel_cap_height() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let font = fonts
            .font(FontId::MONOSPACE)
            .map_err(|error| error.to_string())?;
        let size = super::super::integral_advance_size(&fonts, HELP_CLOSE_HINT_SIZE);
        let cap_height = font.metrics(size.0 * 4.0 / 3.0).cap_height;
        assert!(cap_height >= 9.0, "close hint cap height {cap_height}");
        Ok(())
    }

    #[test]
    fn help_lists_only_dropped_title_controls() {
        let bar = TitleBar::new()
            .control("D Outlines")
            .control("B Card")
            .control("V Photo");
        for dropped in [vec![], vec!["B Card".to_owned(), "V Photo".to_owned()]] {
            let overflow = TitleBarOverflow {
                dropped: dropped.clone(),
            };
            let rows = title_bar_help_rows(&bar, &overflow);
            assert_eq!(
                rows.iter().map(|row| row.keys.as_str()).collect::<Vec<_>>(),
                if dropped.is_empty() {
                    vec![]
                } else {
                    vec!["B", "V"]
                }
            );
        }
    }

    #[test]
    fn help_frame_has_even_visible_margins() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let rows = vec![
            HelpPresentation {
                keys:  "A".to_owned(),
                label: "Smoothing: Off, Anisotropic, Supersample, Both".to_owned(),
            },
            HelpPresentation {
                keys:  "Click".to_owned(),
                label: "Zoom in".to_owned(),
            },
        ];
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(build_help_tree(rows, &fonts))
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("help panel did not solve")?;
        let root = solved.computed[0].bounds;
        let text = solved
            .commands
            .iter()
            .filter(|command| format!("{:?}", command.kind).starts_with("Text { text:"))
            .collect::<Vec<_>>();
        let left = text
            .iter()
            .map(|command| command.bounds.x)
            .fold(f32::INFINITY, f32::min)
            - root.x;
        let right = root.x + root.width
            - text
                .iter()
                .map(|command| command.bounds.x + command.bounds.width)
                .fold(f32::NEG_INFINITY, f32::max);
        let title = text
            .iter()
            .find(|command| format!("{:?}", command.kind).contains(HELP_TITLE))
            .ok_or("Help title missing")?;
        let last = text
            .iter()
            .find(|command| format!("{:?}", command.kind).contains("Zoom in"))
            .ok_or("last Help row missing")?;
        let ppp = Unit::Pixels.to_points();
        let font = fonts
            .font(FontId::MONOSPACE)
            .map_err(|error| error.to_string())?;
        let title_size = super::super::integral_advance_size(&fonts, TITLE_SIZE);
        let title_metrics = font.metrics(title_size.0 * 4.0 / 3.0);
        let top = (title_metrics.line_gap.mul_add(0.5, title_metrics.ascent)
            - title_metrics.cap_height)
            .mul_add(ppp, title.bounds.y)
            - root.y;
        let label_size = super::super::integral_advance_size(&fonts, LABEL_SIZE);
        let label_metrics = font.metrics(label_size.0 * 4.0 / 3.0);
        let bottom = label_metrics
            .line_gap
            .mul_add(0.5, label_metrics.descent)
            .mul_add(
                ppp,
                root.y + root.height - last.bounds.y - last.bounds.height,
            );
        for margin in [right, top, bottom] {
            assert!(
                (margin - left).abs() < 2.0 * ppp,
                "margins {left}, {right}, {top}, {bottom}"
            );
        }
        Ok(())
    }

    #[test]
    fn open_shortcut_overlay_refreshes_dropped_rows_on_resize() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(fonts)
            .insert_resource(DiegeticTextMeasurer::default())
            .insert_resource(KeymapBindings::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        super::super::install_title_bar_overflow_refresh(&mut app);
        install_help_overlay_refresh(&mut app);
        app.init_resource::<HomeTitleBarFlash>();
        let mut window = Window::default();
        window.resolution.set(1200.0, 800.0);
        let window_entity = app.world_mut().spawn((window, PrimaryWindow)).id();
        let empty_panel = || {
            DiegeticPanel::screen()
                .size(Fit, Fit)
                .build()
                .map_err(|error| error.to_string())
        };
        app.world_mut().spawn((
            TitleBar::new()
                .with_title("Widget Lab")
                .control("H Home")
                .control("D Enable/disable"),
            TitleBarControlState::default(),
            TitleBarOverflow::default(),
            Visibility::Inherited,
            empty_panel()?,
        ));
        let overlay = app
            .world_mut()
            .spawn((
                KeyboardShortcutHelp,
                HelpShortcuts {
                    home_marker:   ShortcutPresence::Absent,
                    camera_preset: ShortcutPresence::Absent,
                },
                empty_panel()?,
            ))
            .id();
        for (width, home, toggle) in [
            (1200.0, false, false),
            (400.0, false, true),
            (200.0, true, true),
            (1200.0, false, false),
        ] {
            app.world_mut()
                .get_mut::<Window>(window_entity)
                .ok_or("window missing")?
                .resolution
                .set(width, 800.0);
            app.update();
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(overlay)
                .and_then(|panel| panel.layout().solved())
                .ok_or("shortcut overlay did not solve")?;
            let rows = format!("{:?}", solved.commands);
            assert_eq!(
                rows.contains("Enable/disable"),
                toggle,
                "D row at {width}: {rows}"
            );
            assert_eq!(rows.contains("Home"), home, "H row at {width}: {rows}");
        }
        Ok(())
    }

    #[test]
    fn narrow_widgets_bar_overflow_is_listed_in_shortcut_overlay() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new()
            .with_title("Widget Lab")
            .control("H Home")
            .control("D Enable/disable");
        let (_, overflow) = super::super::title_bar::visible_title_bar(
            &bar,
            &TitleBarControlState::default(),
            &fonts,
            &hana_diegetic::DiegeticTextMeasurer::default(),
            400.0,
            500.0,
        );
        let rows = title_bar_help_rows(&bar, &overflow);
        assert_eq!(
            rows.iter()
                .map(|row| (row.keys.as_str(), row.label.as_str()))
                .collect::<Vec<_>>(),
            vec![("D", "Enable/disable")]
        );
        Ok(())
    }

    fn every_row() -> HelpShortcuts {
        HelpShortcuts {
            home_marker:   ShortcutPresence::Present,
            camera_preset: ShortcutPresence::Present,
        }
    }

    #[test]
    fn help_key_column_separates_either_keys() {
        let mut app = crate::test_app();
        crate::keymap::install(&mut app);
        app.finish();
        let rows = shortcut_rows(every_row(), app.world().resource::<KeymapBindings>());
        for row in rows {
            for (index, character) in row.keys.char_indices() {
                if character == '/' {
                    assert!(row.keys[..index].ends_with(' '), "{}", row.keys);
                    assert!(row.keys[index + 1..].starts_with(' '), "{}", row.keys);
                }
            }
            let characters: Vec<_> = row.keys.chars().collect();
            assert!(
                !characters.windows(2).any(|pair| {
                    ['↑', '↓', '←', '→'].contains(&pair[0])
                        && ['↑', '↓', '←', '→'].contains(&pair[1])
                }),
                "{}",
                row.keys
            );
        }
    }

    /// The overlay prints what the keymap runs, so a user document that rebinds
    /// a capability moves the printed keys with it.
    #[test]
    fn each_row_prints_the_keys_the_live_keymap_binds() {
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
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
