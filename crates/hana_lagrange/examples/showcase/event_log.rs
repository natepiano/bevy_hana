use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::On;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy_kana::ToF32;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::TITLE_COLOR;
use fairy_dust::TITLE_SIZE;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::AlignY;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Percent;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;

use super::*;

/// The diegetic screen panel that hosts the event log on the right half.
#[derive(Component)]
pub(crate) struct EventLogPanel;

/// Marker resource: when present, the next `AnimationEnd` enables the event log.
#[derive(Resource)]
pub(super) struct EnableLogOnAnimationEnd;

struct LogEntry {
    text:  String,
    color: Color,
}

#[derive(Default, PartialEq, Eq)]
enum EventLogState {
    #[default]
    Disabled,
    Enabled,
}

#[derive(Resource, Default)]
pub(crate) struct EventLog {
    state:      EventLogState,
    entries:    Vec<LogEntry>,
    /// Logical px scrolled up from the bottom; `0.0` follows the streaming tail.
    scrollback: f32,
}

impl EventLog {
    pub(crate) fn push(&mut self, text: String) { self.push_colored(text, EVENT_LOG_COLOR); }

    fn push_error(&mut self, text: String) { self.push_colored(text, EVENT_LOG_ERROR_COLOR); }

    fn separator(&mut self) { self.push_colored(EVENT_LOG_SEPARATOR.into(), EVENT_LOG_COLOR); }

    fn push_colored(&mut self, text: String, color: Color) {
        if self.state == EventLogState::Disabled {
            return;
        }
        self.entries.push(LogEntry { text, color });
        // A new entry re-pins the view to the bottom so the tail stays visible.
        self.scrollback = 0.0;
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.scrollback = 0.0;
    }
}

impl fairy_dust::TitleChipActivation for EventLog {
    fn activation(&self) -> ControlActivation {
        match self.state {
            EventLogState::Enabled => ControlActivation::Active,
            EventLogState::Disabled => ControlActivation::Inactive,
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// PANEL — a 50%-width, full-height diegetic screen panel on the right edge.
// ═════════════════════════════════════════════════════════════════════════════

pub(crate) fn spawn_log_panel(commands: &mut Commands, materials: &mut Assets<StandardMaterial>) {
    let unlit = screen_panel_material_handle(materials);
    let built = DiegeticPanel::screen()
        .size(Px(EVENT_LOG_WIDTH), Percent(0.5))
        .anchor(Anchor::TopRight)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_log_tree(&EventLog::default()))
        .build();

    match built {
        Ok(built) => {
            commands.spawn((
                EventLogPanel,
                built,
                Transform::default(),
                Visibility::Hidden,
            ));
        },
        Err(error) => {
            error!("showcase: failed to build event log panel: {error}");
        },
    }
}

/// Rebuilds the panel tree whenever entries or the scroll position change.
pub(crate) fn rebuild_log_panel(
    log: Res<EventLog>,
    panels: Query<Entity, With<EventLogPanel>>,
    mut commands: Commands,
) {
    if !log.is_changed() {
        return;
    }
    let Ok(entity) = panels.single() else {
        return;
    };
    if let Err(error) = commands.set_tree(entity, build_log_tree(&log)) {
        warn!("failed to replace event-log panel tree: {error}");
    }
}

fn build_log_tree(log: &EventLog) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::GROW).height(Sizing::GROW));
    build_log_layout(&mut builder, log);
    builder.build()
}

fn build_log_layout(builder: &mut LayoutBuilder, log: &EventLog) {
    let title = TextStyle::new(TITLE_SIZE).with_color(TITLE_COLOR);
    let hint = TextStyle::new(EVENT_LOG_HINT_SIZE).with_color(HINT_TEXT_COLOR);

    screen_panel_frame(
        builder,
        Sizing::GROW,
        Sizing::GROW,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .gap(EVENT_LOG_CHILD_GAP),
                |builder| {
                    builder.text((EVENT_LOG_TITLE, title));
                    title_divider(builder);
                    // Scroll viewport: fills the remaining height, clips overflow,
                    // and follows the tail (scrollback 0) until the user scrolls up.
                    builder.with(
                        El::column()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .gap(EVENT_LOG_ENTRY_GAP)
                            .scroll_y_from_end(log.scrollback),
                        |builder| {
                            for entry in &log.entries {
                                builder.text(
                                    Text::new(
                                        &entry.text,
                                        TextStyle::new(EVENT_LOG_TEXT_SIZE).with_color(entry.color),
                                    )
                                    .wrap(TextWrap::Words),
                                );
                            }
                        },
                    );
                    footer_hints(builder, &hint);
                },
            );
        },
    );
}

/// Full-width blue rule under the panel title, like the title bar's separators.
fn title_divider(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(EVENT_LOG_DIVIDER_THICKNESS))
            .background(EVENT_LOG_DIVIDER_COLOR),
        |_| {},
    );
}

/// The two scroll/clear hints side by side, split by a vertical blue separator.
fn footer_hints(builder: &mut LayoutBuilder, hint: &TextStyle) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .gap(EVENT_LOG_CHILD_GAP)
            .align_y(AlignY::Center),
        |builder| {
            builder.text((LOG_SCROLL_HINT_TEXT, hint.clone()));
            builder.with(
                El::new()
                    .width(Sizing::fixed(EVENT_LOG_DIVIDER_THICKNESS))
                    .height(Sizing::fixed(EVENT_LOG_HINT_SEPARATOR_HEIGHT))
                    .background(EVENT_LOG_DIVIDER_COLOR),
                |_| {},
            );
            builder.text((LOG_CLEAR_HINT_TEXT, hint.clone()));
        },
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// VISIBILITY + SCROLL — keyboard control over the panel.
// ═════════════════════════════════════════════════════════════════════════════

/// Enables the event log when the initial `AnimateToFit` animation completes.
pub(crate) fn enable_log_on_initial_fit(
    _animation_end: On<AnimationEnd>,
    mut commands: Commands,
    marker: Option<Res<EnableLogOnAnimationEnd>>,
    mut log: ResMut<EventLog>,
    mut panels: Query<&mut Visibility, With<EventLogPanel>>,
) {
    if marker.is_none() {
        return;
    }
    commands.remove_resource::<EnableLogOnAnimationEnd>();
    log.state = EventLogState::Enabled;
    for mut visibility in &mut panels {
        *visibility = Visibility::Inherited;
    }
}

pub(crate) fn toggle_event_log(
    mut log: ResMut<EventLog>,
    mut panels: Query<&mut Visibility, With<EventLogPanel>>,
) {
    log.state = match log.state {
        EventLogState::Disabled => EventLogState::Enabled,
        EventLogState::Enabled => EventLogState::Disabled,
    };

    let visibility = match log.state {
        EventLogState::Enabled => Visibility::Inherited,
        EventLogState::Disabled => {
            log.clear();
            Visibility::Hidden
        },
    };
    for mut panel_visibility in &mut panels {
        *panel_visibility = visibility;
    }
}

/// Clears the event log (bound to `C`).
pub(crate) fn clear_event_log(mut log: ResMut<EventLog>) {
    if log.state == EventLogState::Disabled {
        return;
    }
    log.clear();
}

/// Applies a scroll delta (logical px) to the log, bounded to an upper estimate
/// of the content height so holding a direction at the limit doesn't accumulate
/// a value the other direction must then unwind. `delta` is already frame-rate
/// scaled by the caller. Called every frame the `Up`/`Down` action fires.
pub(crate) fn scroll_log(log: &mut EventLog, delta: f32) {
    if log.state == EventLogState::Disabled {
        return;
    }
    let cap = log.entries.len().to_f32() * EVENT_LOG_MAX_ENTRY_HEIGHT;
    log.scrollback = (log.scrollback + delta).clamp(0.0, cap);
}

fn format_vec3(vector: Vec3) -> String {
    format!("({:.1}, {:.1}, {:.1})", vector.x, vector.y, vector.z)
}

pub(crate) fn log_animation_begin(event: On<AnimationBegin>, mut log: ResMut<EventLog>) {
    log.push(format!(
        "AnimationBegin\n  source={:?}\n  owner={:?}\n  direction={:?}\n  raw_position={:.3}\n  total={:?}",
        event.source,
        event.owner,
        event.direction,
        event.timing.position().normalized(),
        event.timing.total(),
    ));
}

pub(crate) fn log_animation_end(event: On<AnimationEnd>, mut log: ResMut<EventLog>) {
    match &event.reason {
        AnimationReason::Completed => {
            log.push(format!(
                "AnimationEnd\n  source={:?}\n  owner={:?}\n  direction={:?}\n  raw_position={:.3}\n  total={:?}\n  reason=Completed",
                event.source,
                event.owner,
                event.direction,
                event.timing.position().normalized(),
                event.timing.total(),
            ));
            if event.source != AnimationSource::ZoomToFit {
                log.separator();
            }
        },
        AnimationReason::Cancelled {
            interrupted_stage_id,
            interrupted_move,
        } => {
            log.push_error(format!(
                "AnimationEnd\n  source={:?}\n  owner={:?}\n  direction={:?}\n  raw_position={:.3}\n  total={:?}\n  reason=Cancelled\n  stage_id={interrupted_stage_id:?}\n  move_position={}\n  \
                 move_target={}",
                event.source,
                event.owner,
                event.direction,
                event.timing.position().normalized(),
                event.timing.total(),
                format_vec3(interrupted_move.position().0),
                format_vec3(interrupted_move.focus().0),
            ));
        },
    }
}

pub(crate) fn log_camera_homed(event: On<CameraHomed>, mut log: ResMut<EventLog>) {
    log.push(format!(
        "CameraHomed\n  camera={:?}\n  sources={:?}",
        event.camera, event.sources
    ));
    log.separator();
}

pub(crate) fn log_camera_move_start(event: On<CameraMoveBegin>, mut log: ResMut<EventLog>) {
    log.push(format!(
        "CameraMoveBegin\n  stage_id={:?}\n  owner={:?}\n  direction={:?}\n  boundary_elapsed={:?}\n  raw_position={:.3}\n  total={:?}\n  position={}\n  target={}\n  duration={:.0}ms\n  easing={:?}",
        event.stage_id,
        event.owner,
        event.direction,
        event.boundary_elapsed,
        event.timing.position().normalized(),
        event.timing.total(),
        format_vec3(event.camera_move.position().0),
        format_vec3(event.camera_move.focus().0),
        event.camera_move.duration_ms(),
        event.camera_move.easing(),
    ));
}

pub(crate) fn log_camera_move_end(camera_move_end: On<CameraMoveEnd>, mut log: ResMut<EventLog>) {
    log.push(format!(
        "{EVENT_LOG_CAMERA_MOVE_END}\n  stage_id={:?}\n  owner={:?}\n  direction={:?}\n  boundary_elapsed={:?}\n  raw_position={:.3}\n  total={:?}",
        camera_move_end.stage_id,
        camera_move_end.owner,
        camera_move_end.direction,
        camera_move_end.boundary_elapsed,
        camera_move_end.timing.position().normalized(),
        camera_move_end.timing.total(),
    ));
}

pub(crate) fn log_zoom_begin(event: On<ZoomBegin>, mut log: ResMut<EventLog>) {
    log.push(format!(
        "ZoomBegin\n  margin={:.2}\n  duration={:.0}ms\n  easing={:?}",
        event.margin,
        event.duration.as_secs_f32() * SECONDS_TO_MILLIS,
        event.easing,
    ));
}

pub(crate) fn log_zoom_end(event: On<ZoomEnd>, mut log: ResMut<EventLog>) {
    match event.reason {
        ZoomReason::Completed => {
            log.push(EVENT_LOG_ZOOM_COMPLETED.into());
            log.separator();
        },
        ZoomReason::Cancelled => {
            log.push_error(EVENT_LOG_ZOOM_CANCELLED.into());
        },
    }
}

pub(crate) fn log_animation_rejected(event: On<AnimationRejected>, mut log: ResMut<EventLog>) {
    log.push_error(format!(
        "AnimationRejected\n  source={:?}\n  target={:?}\n  reason={:?}",
        event.source, event.target, event.reason
    ));
}
