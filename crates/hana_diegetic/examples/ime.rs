//! IME editing example for one app-owned text value.

use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Rect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::window::PrimaryWindow;
use fairy_dust::Face;
use fairy_dust::OrbitCam;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::sprinkle_example;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::ImeAcceptCommit;
use hana_diegetic::ImeAppInputDisposition;
use hana_diegetic::ImeAppInputDispositionHook;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeAppliedResult;
use hana_diegetic::ImeCanceled;
use hana_diegetic::ImeCommitAuthority;
use hana_diegetic::ImeCommitRequested;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::ImeOpenSession;
use hana_diegetic::ImeRejectCommit;
use hana_diegetic::ImeRejection;
use hana_diegetic::ImeSessionAnchor;
use hana_diegetic::ImeStarted;
use hana_diegetic::ImeTarget;
use hana_diegetic::ImeValueRevision;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;

const FIELD_CENTER_Y_RATIO: f32 = 0.33;
const FIELD_HEIGHT: f32 = 42.0;
const FIELD_WIDTH: f32 = 430.0;
const TEXT_PADDING_X: f32 = 10.0;
const TEXT_PADDING_Y: f32 = 0.0;

const CAMERA_FOCUS: Vec3 = Vec3::new(-0.142_137_11, 1.073_453_3, -0.069_239_84);
const CAMERA_PITCH: f32 = 0.183_259_7;
const CAMERA_RADIUS: f32 = 7.254_312_5;
const CAMERA_YAW: f32 = -0.019_635_003;
const CUBE_COLOR: Color = Color::srgb(0.18, 0.24, 0.28);
const CUBE_LABEL_SIZE: f32 = 0.13;
const CUBE_SIZE: f32 = 0.82;
const GROUND_COLOR: Color = Color::srgba(0.11, 0.13, 0.14, 1.0);
const GROUND_SIZE: f32 = 5.8;

const FIELD_BORDER: Color = Color::srgba(0.42, 0.72, 0.86, 0.82);
const FIELD_BORDER_WIDTH: f32 = 1.0;
const FIELD_CORNER_RADIUS: f32 = 5.0;
const FIELD_BACKGROUND: Color = Color::srgba(0.030, 0.038, 0.044, 0.94);
const TEXT_COLOR: Color = Color::srgb(0.82, 0.92, 0.86);
const WARNING: Color = Color::srgb(1.0, 0.48, 0.36);

const TEXT_SIZE: f32 = 16.0;
const WARNING_SIZE: f32 = 13.0;

#[derive(Component)]
struct EditableTextPanel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditMode {
    Active,
    Idle,
}

#[derive(Resource, Clone, Debug)]
struct EditableTextState {
    text:     String,
    revision: u64,
    error:    Option<String>,
    mode:     EditMode,
}

impl Default for EditableTextState {
    fn default() -> Self {
        Self {
            text:     "editable text".to_owned(),
            revision: 1,
            error:    None,
            mode:     EditMode::Idle,
        }
    }
}

fn main() {
    sprinkle_example()
        .with_title_bar(title_bar())
        .with_studio_lighting()
        .aim_at(CAMERA_FOCUS)
        .with_ground_plane()
        .size(GROUND_SIZE)
        .color(GROUND_COLOR)
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_xyz(0.0, CUBE_SIZE * 0.5, 0.0))
        .face_text(Face::Front, "IME", CUBE_LABEL_SIZE, FIELD_BORDER)
        .with_orbit_cam_configured(configure_camera)
        .with_restore_camera_on_restart()
        .with_stable_transparency()
        .with_camera_control_panel()
        .with_save_window_position()
        .with_brp_extras()
        .init_resource::<EditableTextState>()
        .add_systems(Startup, setup_editable_text)
        .add_systems(Update, (open_editor_shortcut, refresh_editable_text_panel))
        .add_observer(mark_editing_started)
        .add_observer(mark_editing_canceled)
        .add_observer(apply_text_commit)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .with_title("IME editing")
        .controls(["/ Edit", "Enter Save", "Esc Cancel"])
}

fn configure_camera(orbit_cam: &mut OrbitCam) {
    OrbitCamPose {
        focus:  CAMERA_FOCUS,
        yaw:    CAMERA_YAW,
        pitch:  CAMERA_PITCH,
        radius: CAMERA_RADIUS,
    }
    .apply_to(orbit_cam);
}

fn setup_editable_text(
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut hook: ResMut<ImeAppInputDispositionHook>,
    mut commands: Commands,
    state: Res<EditableTextState>,
) {
    hook.set(|context| {
        if context.keys.just_pressed(KeyCode::Slash) {
            ImeAppInputDisposition::Surface
        } else {
            ImeAppInputDisposition::Edit
        }
    });

    let Ok((_, window)) = windows.single() else {
        return;
    };
    let field = field_anchor(window);
    let panel = DiegeticPanel::screen()
        .size(
            Sizing::fixed(Px(FIELD_WIDTH)),
            Sizing::fixed(Px(FIELD_HEIGHT)),
        )
        .anchor(Anchor::TopLeft)
        .screen_position(field.min.x, field.min.y)
        .layout(|builder| editable_text_tree(builder, &state))
        .build();
    let Ok(panel) = panel else {
        error!("failed to build IME text panel");
        return;
    };
    commands.spawn((EditableTextPanel, panel, Transform::default()));
}

fn open_editor_shortcut(
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    panels: Query<Entity, With<EditableTextPanel>>,
    state: Res<EditableTextState>,
    commands: Commands,
) {
    if keys.just_pressed(KeyCode::Slash) && state.mode == EditMode::Idle {
        open_editor_shortcut_target(windows, panels, state, commands);
    }
}

fn open_editor_shortcut_target(
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    panels: Query<Entity, With<EditableTextPanel>>,
    state: Res<EditableTextState>,
    mut commands: Commands,
) {
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let Ok(panel) = panels.single() else { return };

    trigger_editor_open(window_entity, window, panel, &state, &mut commands);
}

fn trigger_editor_open(
    window_entity: Entity,
    window: &Window,
    owner: Entity,
    state: &EditableTextState,
    commands: &mut Commands,
) {
    commands.trigger(ImeOpenSession {
        target:       ImeTarget::AppOwned {
            owner,
            field_id: "text".into(),
        },
        window:       window_entity,
        initial_text: state.text.clone(),
        field_spec:   ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("text")),
        anchor:       Some(ImeSessionAnchor::screen_rect(field_anchor(window))),
    });
}

fn editable_text_tree(builder: &mut LayoutBuilder, state: &EditableTextState) {
    let mut frame = El::row()
        .width(Sizing::GROW)
        .height(Sizing::GROW)
        .padding(Padding::xy(TEXT_PADDING_X, TEXT_PADDING_Y))
        .alignment(AlignX::Left, AlignY::Center)
        .corner_radius(CornerRadius::all(FIELD_CORNER_RADIUS));
    if state.mode == EditMode::Active {
        frame = frame.background(Color::NONE);
    } else {
        frame = frame
            .background(FIELD_BACKGROUND)
            .border(Border::all(FIELD_BORDER_WIDTH, FIELD_BORDER));
    }

    builder.with(frame, |builder| {
        if state.mode == EditMode::Idle {
            builder.text((
                state.text.as_str(),
                TextStyle::new(TEXT_SIZE).with_color(TEXT_COLOR),
            ));
        }
        if let Some(error) = &state.error {
            builder.text((
                error.as_str(),
                TextStyle::new(WARNING_SIZE).with_color(WARNING),
            ));
        }
    });
}

fn field_center(window: &Window) -> Vec2 {
    Vec2::new(window.width() * 0.5, window.height() * FIELD_CENTER_Y_RATIO)
}

fn field_anchor(window: &Window) -> Rect {
    let center = field_center(window);
    let half = Vec2::new(FIELD_WIDTH * 0.5, FIELD_HEIGHT * 0.5);
    Rect::from_corners(
        Vec2::new(center.x - half.x, center.y - half.y),
        Vec2::new(center.x + half.x, center.y + half.y),
    )
}

fn mark_editing_started(
    event: On<ImeStarted>,
    mut state: ResMut<EditableTextState>,
    panels: Query<Entity, With<EditableTextPanel>>,
    mut commands: Commands,
) {
    if matches!(event.event().target, ImeTarget::AppOwned { .. }) {
        state.mode = EditMode::Active;
        set_editable_text_visibility(&panels, Visibility::Hidden, &mut commands);
        rebuild_editable_text_panel(&state, &panels, &mut commands);
    }
}

fn mark_editing_canceled(
    event: On<ImeCanceled>,
    mut state: ResMut<EditableTextState>,
    panels: Query<Entity, With<EditableTextPanel>>,
    mut commands: Commands,
) {
    if matches!(event.event().target, ImeTarget::AppOwned { .. }) {
        state.mode = EditMode::Idle;
        set_editable_text_visibility(&panels, Visibility::Visible, &mut commands);
        rebuild_editable_text_panel(&state, &panels, &mut commands);
    }
}

fn apply_text_commit(
    event: On<ImeCommitRequested>,
    authority: Res<ImeCommitAuthority>,
    mut state: ResMut<EditableTextState>,
    panels: Query<Entity, With<EditableTextPanel>>,
    mut commands: Commands,
) {
    let event = event.event();
    if !authority.is_current(event.session_id, event.attempt_id) {
        return;
    }
    if !matches!(event.target, ImeTarget::AppOwned { .. }) {
        return;
    }
    if event.text.trim().is_empty() {
        commands.trigger(ImeRejectCommit {
            session_id: event.session_id,
            attempt_id: event.attempt_id,
            reason:     ImeRejection::AppOwned("type something".to_owned()),
        });
        return;
    }

    event.text.trim().clone_into(&mut state.text);
    state.revision = state.revision.wrapping_add(1);
    state.error = None;
    state.mode = EditMode::Idle;
    set_editable_text_visibility(&panels, Visibility::Visible, &mut commands);
    rebuild_editable_text_panel(&state, &panels, &mut commands);
    commands.trigger(ImeAcceptCommit {
        session_id: event.session_id,
        attempt_id: event.attempt_id,
        result:     ImeAppliedResult::AppOwned {
            display_text:   Some(state.text.clone()),
            value_revision: Some(ImeValueRevision::new(state.revision)),
        },
    });
}

fn refresh_editable_text_panel(
    state: Res<EditableTextState>,
    panels: Query<Entity, With<EditableTextPanel>>,
    mut commands: Commands,
) {
    if !state.is_changed() {
        return;
    }
    rebuild_editable_text_panel(&state, &panels, &mut commands);
}

fn rebuild_editable_text_panel(
    state: &EditableTextState,
    panels: &Query<Entity, With<EditableTextPanel>>,
    commands: &mut Commands,
) {
    let Ok(panel) = panels.single() else { return };
    let mut builder = LayoutBuilder::new(Px(FIELD_WIDTH), Px(FIELD_HEIGHT));
    editable_text_tree(&mut builder, state);
    if let Err(error) = commands.set_tree(panel, builder.build()) {
        error!("failed to replace IME demo panel tree: {error}");
    }
}

fn set_editable_text_visibility(
    panels: &Query<Entity, With<EditableTextPanel>>,
    visibility: Visibility,
    commands: &mut Commands,
) {
    let Ok(panel) = panels.single() else { return };
    commands.entity(panel).insert(visibility);
}
