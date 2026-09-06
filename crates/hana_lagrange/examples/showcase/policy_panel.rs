use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Local;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::error;
use bevy::prelude::warn;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::TITLE_COLOR;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::ChildDivider;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;

use super::*;

/// The diegetic screen panel that hosts the animation policy reference in the
/// bottom-left corner.
#[derive(Component)]
pub(crate) struct PolicyPanel;

/// Mirrors the camera's current policy values so the panel can highlight the
/// active variant. The toggle systems keep this in step with the camera
/// components they mutate.
#[derive(Resource, Default)]
pub(crate) struct PolicyDisplay {
    pub(crate) interrupt_behavior: CameraInputInterruptBehavior,
    pub(crate) conflict_policy:    AnimationConflictPolicy,
}

/// Tracks how long each toggle key stays highlighted after a press. A running
/// timer means the key renders in the active color, signaling the cycle.
#[derive(Resource, Default)]
pub(crate) struct KeyFlash {
    interrupt: Option<Timer>,
    conflict:  Option<Timer>,
}

impl KeyFlash {
    pub(crate) fn flash_interrupt(&mut self) {
        self.interrupt = Some(Timer::from_seconds(
            POLICY_PANEL_KEY_FLASH_SECONDS,
            TimerMode::Once,
        ));
    }

    pub(crate) fn flash_conflict(&mut self) {
        self.conflict = Some(Timer::from_seconds(
            POLICY_PANEL_KEY_FLASH_SECONDS,
            TimerMode::Once,
        ));
    }
}

/// Whether a variant row is the camera's current policy.
#[derive(Clone, Copy)]
enum RowState {
    Active,
    Inactive,
}

impl From<bool> for RowState {
    fn from(active: bool) -> Self { if active { Self::Active } else { Self::Inactive } }
}

const INTERRUPT_VARIANTS: [CameraInputInterruptBehavior; 3] = [
    CameraInputInterruptBehavior::Ignore,
    CameraInputInterruptBehavior::Cancel,
    CameraInputInterruptBehavior::Complete,
];

const CONFLICT_VARIANTS: [AnimationConflictPolicy; 2] = [
    AnimationConflictPolicy::LastWins,
    AnimationConflictPolicy::FirstWins,
];

const fn interrupt_behavior_label(behavior: CameraInputInterruptBehavior) -> &'static str {
    match behavior {
        CameraInputInterruptBehavior::Ignore => "Ignore",
        CameraInputInterruptBehavior::Cancel => "Cancel",
        CameraInputInterruptBehavior::Complete => "Complete",
    }
}

const fn interrupt_behavior_description(behavior: CameraInputInterruptBehavior) -> &'static str {
    match behavior {
        CameraInputInterruptBehavior::Ignore => "camera input during animation is ignored",
        CameraInputInterruptBehavior::Cancel => "camera input during animation will cancel it",
        CameraInputInterruptBehavior::Complete => {
            "camera input during animation will jump to final position"
        },
    }
}

const fn conflict_policy_label(animation_conflict_policy: AnimationConflictPolicy) -> &'static str {
    match animation_conflict_policy {
        AnimationConflictPolicy::LastWins => "LastWins",
        AnimationConflictPolicy::FirstWins => "FirstWins",
    }
}

const fn conflict_policy_description(
    animation_conflict_policy: AnimationConflictPolicy,
) -> &'static str {
    match animation_conflict_policy {
        AnimationConflictPolicy::LastWins => "new animation cancels current one",
        AnimationConflictPolicy::FirstWins => "new animation is rejected while one is playing",
    }
}

/// Width and height ceilings, in logical px, derived from the viewport: the
/// panel fits its content but never exceeds these fractions of the window.
fn panel_caps(window: &Window) -> Vec2 {
    Vec2::new(
        window.width() * POLICY_PANEL_WIDTH_PERCENT,
        window.height() * POLICY_PANEL_HEIGHT_PERCENT,
    )
}

/// Fit-to-content sizing capped at `max` px, or uncapped fit when `max` is not
/// yet known (the panel spawns before the window size is read).
fn capped_fit(max: f32) -> Sizing {
    if max > 0.0 {
        Sizing::fit_range(0.0, Px(max))
    } else {
        Sizing::FIT
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// PANEL — a content-fit diegetic screen panel anchored to the bottom-left corner.
// ═════════════════════════════════════════════════════════════════════════════

pub(crate) fn spawn_policy_panel(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
) {
    let unlit = screen_panel_material_handle(materials);
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_policy_tree(
            &PolicyDisplay::default(),
            &KeyFlash::default(),
            Vec2::ZERO,
        ))
        .build();

    match built {
        Ok(built) => {
            commands.spawn((
                PolicyPanel,
                built,
                Transform::default(),
                Visibility::Visible,
            ));
        },
        Err(error) => {
            error!("showcase: failed to build policy panel: {error}");
        },
    }
}

/// Rebuilds the panel tree when the policy values change or the window resizes.
/// `tick_key_flash` calls `set_changed` on `PolicyDisplay` when a highlight
/// ends, so reading the live `KeyFlash` here keeps the key colors in step.
pub(crate) fn rebuild_policy_panel(
    display: Res<PolicyDisplay>,
    flash: Res<KeyFlash>,
    windows: Query<&Window, With<PrimaryWindow>>,
    panels: Query<Entity, With<PolicyPanel>>,
    mut last_caps: Local<Vec2>,
    mut commands: Commands,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let caps = panel_caps(window);
    if !display.is_changed() && caps == *last_caps {
        return;
    }
    *last_caps = caps;

    let Ok(entity) = panels.single() else {
        return;
    };
    if let Err(error) = commands.set_tree(entity, build_policy_tree(&display, &flash, caps)) {
        warn!("failed to replace policy panel tree: {error}");
    }
}

/// Advances the key-flash timers and forces one panel rebuild when a highlight
/// ends, reverting the key to its resting color.
pub(crate) fn tick_key_flash(
    time: Res<Time>,
    mut flash: ResMut<KeyFlash>,
    mut display: ResMut<PolicyDisplay>,
) {
    if flash.interrupt.is_none() && flash.conflict.is_none() {
        return;
    }

    let interrupt_ended = flash
        .interrupt
        .as_mut()
        .is_some_and(|timer| timer.tick(time.delta()).just_finished());
    if interrupt_ended {
        flash.interrupt = None;
    }

    let conflict_ended = flash
        .conflict
        .as_mut()
        .is_some_and(|timer| timer.tick(time.delta()).just_finished());
    if conflict_ended {
        flash.conflict = None;
    }

    if interrupt_ended || conflict_ended {
        display.set_changed();
    }
}

/// Bundles the text styles the panel reuses across rows and groups.
struct PolicyTextStyles {
    header:      TextStyle,
    key:         TextStyle,
    key_active:  TextStyle,
    name:        TextStyle,
    active:      TextStyle,
    description: TextStyle,
}

impl PolicyTextStyles {
    fn new() -> Self {
        Self {
            header:      TextStyle::new(POLICY_PANEL_HEADER_SIZE).with_color(TITLE_COLOR),
            key:         TextStyle::new(POLICY_PANEL_KEY_TEXT_SIZE).with_color(HINT_TEXT_COLOR),
            key_active:  TextStyle::new(POLICY_PANEL_KEY_TEXT_SIZE)
                .with_color(POLICY_PANEL_ACTIVE_COLOR),
            name:        TextStyle::new(POLICY_PANEL_TEXT_SIZE).with_color(HINT_TEXT_COLOR),
            active:      TextStyle::new(POLICY_PANEL_TEXT_SIZE)
                .with_color(POLICY_PANEL_ACTIVE_COLOR),
            description: TextStyle::new(POLICY_PANEL_TEXT_SIZE).with_color(HINT_TEXT_COLOR),
        }
    }
}

fn build_policy_tree(display: &PolicyDisplay, flash: &KeyFlash, max_size: Vec2) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_policy_layout(&mut builder, display, flash, max_size);
    builder.build()
}

fn build_policy_layout(
    builder: &mut LayoutBuilder,
    display: &PolicyDisplay,
    flash: &KeyFlash,
    max_size: Vec2,
) {
    let styles = PolicyTextStyles::new();

    screen_panel_frame(
        builder,
        capped_fit(max_size.x),
        capped_fit(max_size.y),
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(Px(POLICY_PANEL_GROUP_GAP))
                    .child_divider(ChildDivider::new(
                        Px(EVENT_LOG_DIVIDER_THICKNESS),
                        EVENT_LOG_DIVIDER_COLOR,
                    )),
                |builder| {
                    let interrupt_key_style = if flash.interrupt.is_some() {
                        &styles.key_active
                    } else {
                        &styles.key
                    };
                    build_group(
                        builder,
                        &styles,
                        POLICY_PANEL_INTERRUPT_HEADER,
                        POLICY_PANEL_INTERRUPT_KEY,
                        interrupt_key_style,
                        |builder| {
                            for behavior in INTERRUPT_VARIANTS {
                                build_variant_row(
                                    builder,
                                    &styles,
                                    interrupt_behavior_label(behavior),
                                    interrupt_behavior_description(behavior),
                                    (behavior == display.interrupt_behavior).into(),
                                );
                            }
                        },
                    );
                    let conflict_key_style = if flash.conflict.is_some() {
                        &styles.key_active
                    } else {
                        &styles.key
                    };
                    build_group(
                        builder,
                        &styles,
                        POLICY_PANEL_CONFLICT_HEADER,
                        POLICY_PANEL_CONFLICT_KEY,
                        conflict_key_style,
                        |builder| {
                            for policy in CONFLICT_VARIANTS {
                                build_variant_row(
                                    builder,
                                    &styles,
                                    conflict_policy_label(policy),
                                    conflict_policy_description(policy),
                                    (policy == display.conflict_policy).into(),
                                );
                            }
                        },
                    );
                },
            );
        },
    );
}

/// A header row, then the toggle key with its cycle arrow in a fixed-width
/// column (so both keys line up across groups), beside the variant rows the key
/// cycles through.
fn build_group(
    builder: &mut LayoutBuilder,
    styles: &PolicyTextStyles,
    header: &str,
    key: &str,
    key_style: &TextStyle,
    rows: impl FnOnce(&mut LayoutBuilder),
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(POLICY_PANEL_HEADER_GAP)),
        |builder| {
            builder.text((header, styles.header.clone()));
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(Px(POLICY_PANEL_COLUMN_GAP))
                    .align_y(AlignY::Center),
                |builder| {
                    build_key_cell(builder, key, key_style);
                    builder.with(
                        El::column()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .gap(Px(POLICY_PANEL_ROW_GAP)),
                        rows,
                    );
                },
            );
        },
    );
}

fn build_key_cell(builder: &mut LayoutBuilder, key: &str, key_style: &TextStyle) {
    builder.with(
        El::new()
            .width(Sizing::fixed(Px(POLICY_PANEL_KEY_COLUMN_WIDTH)))
            .height(Sizing::FIT)
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.text((format!("{key} {POLICY_PANEL_ARROW}"), key_style.clone()));
        },
    );
}

/// One row: the variant name in a fixed-width column, then its description left
/// aligned in the remaining width so descriptions line up across the group.
fn build_variant_row(
    builder: &mut LayoutBuilder,
    styles: &PolicyTextStyles,
    label: &str,
    description: &str,
    row_state: RowState,
) {
    let name_style = match row_state {
        RowState::Active => &styles.active,
        RowState::Inactive => &styles.name,
    };

    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(POLICY_PANEL_NAME_GAP))
            .align_y(AlignY::Top),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(Px(POLICY_PANEL_NAME_COLUMN_WIDTH)))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((label, name_style.clone()));
                },
            );
            builder.with(
                El::new().width(Sizing::GROW).height(Sizing::FIT),
                |builder| {
                    builder.text(
                        Text::new(description, styles.description.clone()).wrap(TextWrap::Words),
                    );
                },
            );
        },
    );
}
