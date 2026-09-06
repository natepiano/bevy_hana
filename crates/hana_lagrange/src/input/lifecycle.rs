use core::marker::PhantomData;
use std::time::Duration;

use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::With;
use bevy::prelude::World;

use super::CameraInputBlockers;
use super::CameraInputMetricKind;
use super::CameraInputMetricsMissing;
use super::CameraInputModeKind;
use super::CameraInputModeReplaced;
use super::CameraInputSourceLatches;
use super::CameraInputSurfaceMetrics;
use super::ControlSpeed;
use super::FreeCamActiveDirections;
use super::FreeCamInput;
use super::FreeCamInteractionEnded;
use super::FreeCamInteractionKind;
use super::FreeCamInteractionSourcesChanged;
use super::FreeCamInteractionSpeedChanged;
use super::FreeCamInteractionStarted;
use super::FreeCamInteractionState;
use super::InputIntent;
use super::InteractionSources;
use super::OrbitCamInput;
use super::OrbitCamInteractionEnded;
use super::OrbitCamInteractionKind;
use super::OrbitCamInteractionSourcesChanged;
use super::OrbitCamInteractionSpeedChanged;
use super::OrbitCamInteractionStarted;
use super::OrbitCamInteractionState;
use super::ResolvedCameraInputRoute;
use super::ZoomDirection;
use super::constants::DEFAULT_REPORTING_DEBOUNCE;
use super::constants::INTERACTION_CHANNEL_COUNT;
use crate::FreeCamKind;
use crate::OrbitCamKind;
use crate::system_sets::CameraInputPhase;

/// Settle delay that smooths the reported interaction state — both the active
/// source set per kind and the gamepad speed variant.
///
/// Holds a kind's reported sources for this window after the live input goes
/// quiet, bridging the per-frame gaps in bursty input (a trackpad emulating a
/// mouse button as intermittent press/release, smooth-scroll arriving in
/// bursts) so a control panel does not flicker. The same window holds the
/// gamepad speed's return to `Normal`, so `Normal` is not reported for the
/// frame or two when the `rb`/`lb` slow gate lands a frame apart from its
/// stick or trigger.
///
/// Reporting-only: it affects the reported camera interaction state and
/// interaction events. Camera motion reads its input intent directly and is
/// never delayed. A newly-engaged source and `Slow` report immediately; only a
/// source drop and the return to `Normal` wait out the window. Insert your own
/// value to override; `Duration::ZERO` disables the delay.
#[derive(Resource, Clone, Copy, Debug, Reflect)]
#[reflect(Resource, Default)]
pub struct CameraInputReportingDebounce(pub Duration);

impl Default for CameraInputReportingDebounce {
    fn default() -> Self { Self(DEFAULT_REPORTING_DEBOUNCE) }
}

#[derive(Clone, Copy, Debug)]
enum LifecycleEvent<K: CameraInputLifecycleKind> {
    Started {
        kind:    K::InteractionKind,
        sources: InteractionSources,
    },
    Ended {
        kind:    K::InteractionKind,
        sources: InteractionSources,
    },
    SourcesChanged {
        kind:     K::InteractionKind,
        previous: InteractionSources,
        current:  InteractionSources,
    },
    SpeedChanged {
        kind:  K::InteractionKind,
        speed: ControlSpeed,
    },
    MetricsMissing(CameraInputMetricsMissing),
}

#[derive(Clone, Copy, Debug)]
enum InputMetricStatus {
    Complete,
    Missing,
}

#[derive(Clone)]
struct FinalizedInput<K: CameraInputLifecycleKind> {
    camera:            Entity,
    input:             InputIntent<K>,
    interaction_state: K::InteractionState,
    settle:            CameraReportedInteractionSettle<K>,
    events:            Vec<LifecycleEvent<K>>,
}

/// The source and speed settle deadlines tracked per interaction kind.
#[derive(Clone, Copy, Debug, Default)]
struct KindSettle {
    source: Option<f32>,
    speed:  Option<f32>,
}

/// Per-camera settle deadlines (in `Time<Real>` seconds) for the
/// reported-interaction debounce.
#[derive(Component, Clone, Copy, Debug)]
struct CameraReportedInteractionSettle<K: CameraInputLifecycleKind> {
    channels: [KindSettle; INTERACTION_CHANNEL_COUNT],
    marker:   PhantomData<fn() -> K>,
}

#[cfg(test)]
type ReportedInteractionSettle = CameraReportedInteractionSettle<OrbitCamKind>;

impl<K: CameraInputLifecycleKind> Default for CameraReportedInteractionSettle<K> {
    fn default() -> Self {
        Self {
            channels: [KindSettle::default(); INTERACTION_CHANNEL_COUNT],
            marker:   PhantomData,
        }
    }
}

impl<K: CameraInputLifecycleKind> CameraReportedInteractionSettle<K> {
    fn source_deadline(self, kind: K::InteractionKind) -> Option<f32> {
        self.channels[K::kind_index(kind)].source
    }

    fn speed_deadline(self, kind: K::InteractionKind) -> Option<f32> {
        self.channels[K::kind_index(kind)].speed
    }

    fn set_source_deadline(&mut self, kind: K::InteractionKind, at: Option<f32>) {
        self.channels[K::kind_index(kind)].source = at;
    }

    fn set_speed_deadline(&mut self, kind: K::InteractionKind, at: Option<f32>) {
        self.channels[K::kind_index(kind)].speed = at;
    }
}

/// Transient inputs threaded into finalization for the reported-interaction
/// settle.
#[derive(Clone, Copy)]
struct SettleContext<K: CameraInputLifecycleKind> {
    previous: CameraReportedInteractionSettle<K>,
    now:      f32,
    window:   f32,
}

#[derive(Clone, Copy, Debug)]
struct ChannelReport<K> {
    kind:    K,
    sources: InteractionSources,
    speed:   ControlSpeed,
}

trait CameraInputLifecycleKind: CameraInputModeKind<Input = InputIntent<Self>> + Sized {
    type InteractionKind: Copy + core::fmt::Debug;
    type InteractionState: Component + Clone + Copy + Default;

    fn kind_index(kind: Self::InteractionKind) -> usize;

    fn channel_reports(
        input: &InputIntent<Self>,
    ) -> [ChannelReport<Self::InteractionKind>; INTERACTION_CHANNEL_COUNT];

    fn state_sources(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> InteractionSources;

    fn set_state_sources(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    );

    fn state_speed(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> Option<ControlSpeed>;

    fn set_state_speed(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        speed: Option<ControlSpeed>,
    );

    fn apply_metric_guard(
        _camera: Entity,
        _input: &mut InputIntent<Self>,
        _metrics: Option<CameraInputSurfaceMetrics>,
        _events: &mut Vec<LifecycleEvent<Self>>,
    ) {
    }

    fn update_extra_state(
        _input: &InputIntent<Self>,
        _previous: Self::InteractionState,
        _state: &mut Self::InteractionState,
    ) {
    }

    fn trigger_started(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    );

    fn trigger_ended(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    );

    fn trigger_sources_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        previous: InteractionSources,
        current: InteractionSources,
    );

    fn trigger_speed_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        speed: ControlSpeed,
    );
}

impl CameraInputLifecycleKind for OrbitCamKind {
    type InteractionKind = OrbitCamInteractionKind;
    type InteractionState = OrbitCamInteractionState;

    fn kind_index(kind: Self::InteractionKind) -> usize {
        match kind {
            OrbitCamInteractionKind::Orbit => 0,
            OrbitCamInteractionKind::Pan => 1,
            OrbitCamInteractionKind::Zoom => 2,
        }
    }

    fn channel_reports(input: &OrbitCamInput) -> [ChannelReport<Self::InteractionKind>; 3] {
        [
            ChannelReport {
                kind:    OrbitCamInteractionKind::Orbit,
                sources: input.orbit_sources(),
                speed:   input.orbit_speed(),
            },
            ChannelReport {
                kind:    OrbitCamInteractionKind::Pan,
                sources: input.pan_sources(),
                speed:   input.pan_speed(),
            },
            ChannelReport {
                kind:    OrbitCamInteractionKind::Zoom,
                sources: input.zoom_sources(),
                speed:   input.zoom_speed(),
            },
        ]
    }

    fn state_sources(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> InteractionSources {
        state.sources(kind)
    }

    fn set_state_sources(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        state.set_sources(kind, sources);
    }

    fn state_speed(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> Option<ControlSpeed> {
        state.speed(kind)
    }

    fn set_state_speed(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        speed: Option<ControlSpeed>,
    ) {
        state.set_speed(kind, speed);
    }

    fn apply_metric_guard(
        camera: Entity,
        input: &mut OrbitCamInput,
        metrics: Option<CameraInputSurfaceMetrics>,
        events: &mut Vec<LifecycleEvent<Self>>,
    ) {
        apply_orbit_metric_guard(camera, input, metrics, events);
    }

    fn update_extra_state(
        input: &OrbitCamInput,
        previous: Self::InteractionState,
        state: &mut Self::InteractionState,
    ) {
        state.set_zoom_direction(reported_zoom_direction(
            input,
            state.zoom_sources(),
            previous.zoom_direction(),
        ));
    }

    fn trigger_started(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| OrbitCamInteractionStarted {
                camera,
                kind,
                sources,
            });
    }

    fn trigger_ended(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| OrbitCamInteractionEnded {
                camera,
                kind,
                sources,
            });
    }

    fn trigger_sources_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        previous: InteractionSources,
        current: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| OrbitCamInteractionSourcesChanged {
                camera,
                kind,
                previous,
                current,
            });
    }

    fn trigger_speed_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        speed: ControlSpeed,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| OrbitCamInteractionSpeedChanged {
                camera,
                kind,
                speed,
            });
    }
}

impl CameraInputLifecycleKind for FreeCamKind {
    type InteractionKind = FreeCamInteractionKind;
    type InteractionState = FreeCamInteractionState;

    fn kind_index(kind: Self::InteractionKind) -> usize {
        match kind {
            FreeCamInteractionKind::Translate => 0,
            FreeCamInteractionKind::Look => 1,
            FreeCamInteractionKind::Roll => 2,
        }
    }

    fn channel_reports(input: &FreeCamInput) -> [ChannelReport<Self::InteractionKind>; 3] {
        [
            ChannelReport {
                kind:    FreeCamInteractionKind::Translate,
                sources: input.translate_sources(),
                speed:   input.translate_speed(),
            },
            ChannelReport {
                kind:    FreeCamInteractionKind::Look,
                sources: input.look_sources(),
                speed:   input.look_speed(),
            },
            ChannelReport {
                kind:    FreeCamInteractionKind::Roll,
                sources: input.roll_sources(),
                speed:   input.roll_speed(),
            },
        ]
    }

    fn state_sources(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> InteractionSources {
        state.sources(kind)
    }

    fn set_state_sources(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        state.set_sources(kind, sources);
    }

    fn state_speed(
        state: &Self::InteractionState,
        kind: Self::InteractionKind,
    ) -> Option<ControlSpeed> {
        state.speed(kind)
    }

    fn set_state_speed(
        state: &mut Self::InteractionState,
        kind: Self::InteractionKind,
        speed: Option<ControlSpeed>,
    ) {
        state.set_speed(kind, speed);
    }

    fn update_extra_state(
        input: &FreeCamInput,
        _: Self::InteractionState,
        state: &mut Self::InteractionState,
    ) {
        let directions = reported_free_directions(input, state);
        state.set_directions(directions);
    }

    fn trigger_started(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| FreeCamInteractionStarted {
                camera,
                kind,
                sources,
            });
    }

    fn trigger_ended(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        sources: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| FreeCamInteractionEnded {
                camera,
                kind,
                sources,
            });
    }

    fn trigger_sources_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        previous: InteractionSources,
        current: InteractionSources,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| FreeCamInteractionSourcesChanged {
                camera,
                kind,
                previous,
                current,
            });
    }

    fn trigger_speed_changed(
        world: &mut World,
        camera: Entity,
        kind: Self::InteractionKind,
        speed: ControlSpeed,
    ) {
        world
            .entity_mut(camera)
            .trigger(|_| FreeCamInteractionSpeedChanged {
                camera,
                kind,
                speed,
            });
    }
}

pub(crate) struct CameraInputLifecyclePlugin;

impl Plugin for CameraInputLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraInputReportingDebounce>()
            .add_observer(clear_reported_state_on_mode_replaced::<OrbitCamKind>)
            .add_observer(clear_reported_state_on_mode_replaced::<FreeCamKind>)
            .add_systems(
                PreUpdate,
                (
                    finalize_camera_input::<OrbitCamKind>,
                    finalize_camera_input::<FreeCamKind>,
                )
                    .in_set(CameraInputPhase::Finalize),
            );
    }
}

fn clear_reported_state_on_mode_replaced<K: CameraInputLifecycleKind>(
    replaced: On<CameraInputModeReplaced>,
    mut commands: Commands,
    cameras: Query<(), With<K::Camera>>,
) {
    if !cameras.contains(replaced.camera) {
        return;
    }
    commands.entity(replaced.camera).insert((
        K::InteractionState::default(),
        CameraReportedInteractionSettle::<K>::default(),
    ));
}

fn finalize_camera_input<K: CameraInputLifecycleKind>(world: &mut World) {
    let route = world.get_resource::<ResolvedCameraInputRoute>().cloned();
    let window = world
        .resource::<CameraInputReportingDebounce>()
        .0
        .as_secs_f32();
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let mut query = world.query::<(
        Entity,
        &InputIntent<K>,
        Option<&K::InteractionState>,
        Option<&CameraReportedInteractionSettle<K>>,
        Option<&CameraInputBlockers>,
        Option<&CameraInputSurfaceMetrics>,
    )>();
    let finalized = query
        .iter(world)
        .map(|(camera, input, state, settle, blockers, metrics)| {
            finalize_camera_input_state::<K>(
                camera,
                *input,
                state.copied().unwrap_or_default(),
                SettleContext {
                    previous: settle.copied().unwrap_or_default(),
                    now,
                    window,
                },
                blockers.copied().unwrap_or_default(),
                merged_surface_metrics(
                    route.as_ref().and_then(|route| route.metrics_for(camera)),
                    metrics.copied(),
                ),
            )
        })
        .collect::<Vec<_>>();

    for finalized in finalized {
        if let Some(mut input) = world.get_mut::<InputIntent<K>>(finalized.camera) {
            *input = finalized.input;
        }
        world
            .entity_mut(finalized.camera)
            .insert((finalized.interaction_state, finalized.settle));
        for event in finalized.events {
            apply_lifecycle_event::<K>(world, finalized.camera, event);
        }
    }
}

const fn merged_surface_metrics(
    routed: Option<CameraInputSurfaceMetrics>,
    explicit: Option<CameraInputSurfaceMetrics>,
) -> Option<CameraInputSurfaceMetrics> {
    match (routed, explicit) {
        (None, None) => None,
        (Some(metrics), None) | (None, Some(metrics)) => Some(metrics),
        (Some(mut metrics), Some(explicit)) => {
            if explicit.camera_view_size.is_some() {
                metrics.camera_view_size = explicit.camera_view_size;
            }
            if explicit.input_surface_size.is_some() {
                metrics.input_surface_size = explicit.input_surface_size;
            }
            Some(metrics)
        },
    }
}

fn finalize_camera_input_state<K: CameraInputLifecycleKind>(
    camera: Entity,
    mut input: InputIntent<K>,
    previous: K::InteractionState,
    settle_context: SettleContext<K>,
    blockers: CameraInputBlockers,
    metrics: Option<CameraInputSurfaceMetrics>,
) -> FinalizedInput<K> {
    let mut events = Vec::new();

    if blockers.is_blocked() {
        input.clear();
    } else {
        K::apply_metric_guard(camera, &mut input, metrics, &mut events);
    }

    let mut interaction_state = K::InteractionState::default();
    let mut settle = CameraReportedInteractionSettle::default();
    for report in K::channel_reports(&input) {
        let previous_sources = K::state_sources(&previous, report.kind);
        let reported = debounce_sources(
            &mut settle,
            report.kind,
            previous_sources,
            report.sources,
            settle_context,
        );
        push_state_transition(camera, report.kind, previous_sources, reported, &mut events);
        K::set_state_sources(&mut interaction_state, report.kind, reported);
    }
    K::update_extra_state(&input, previous, &mut interaction_state);

    report_speeds(
        &input,
        previous,
        settle_context,
        &mut interaction_state,
        &mut settle,
        &mut events,
    );

    FinalizedInput {
        camera,
        input,
        interaction_state,
        settle,
        events,
    }
}

/// Reports each kind's debounced speed from the reported sources already written
/// to `state`, recording the speed-settle deadline and emitting a
/// speed-changed event when an engaged kind settles to a new speed.
fn report_speeds<K: CameraInputLifecycleKind>(
    input: &InputIntent<K>,
    previous: K::InteractionState,
    context: SettleContext<K>,
    state: &mut K::InteractionState,
    settle: &mut CameraReportedInteractionSettle<K>,
    events: &mut Vec<LifecycleEvent<K>>,
) {
    for report in K::channel_reports(input) {
        let kind = report.kind;
        let sources = K::state_sources(state, kind);
        let previous_speed = K::state_speed(&previous, kind);
        let (reported, deadline) = settled_speed(
            previous_speed,
            context.previous.speed_deadline(kind),
            !sources.is_empty(),
            report.speed,
            sources,
            context.now,
            context.window,
        );
        K::set_state_speed(state, kind, reported);
        settle.set_speed_deadline(kind, deadline);
        let settled_change = !sources.is_empty() && previous_speed != reported;
        if let Some(speed) = reported.filter(|_| settled_change) {
            events.push(LifecycleEvent::SpeedChanged { kind, speed });
        }
    }
}

fn apply_orbit_metric_guard(
    camera: Entity,
    input: &mut OrbitCamInput,
    metrics: Option<CameraInputSurfaceMetrics>,
    events: &mut Vec<LifecycleEvent<OrbitCamKind>>,
) {
    let manual_screen_input = input
        .orbit_sources()
        .union(input.pan_sources())
        .contains(InteractionSources::MANUAL);
    if !manual_screen_input || !input.has_orbit() && !input.has_pan() {
        return;
    }

    let Some(metrics) = metrics else {
        push_missing_metric(camera, CameraInputMetricKind::CameraViewSize, events);
        push_missing_metric(camera, CameraInputMetricKind::InputSurfaceSize, events);
        input.clear_orbit().clear_pan();
        return;
    };

    let mut status = InputMetricStatus::Complete;
    if metrics.camera_view_size.is_none() {
        status = InputMetricStatus::Missing;
        push_missing_metric(camera, CameraInputMetricKind::CameraViewSize, events);
    }
    if metrics.input_surface_size.is_none() {
        status = InputMetricStatus::Missing;
        push_missing_metric(camera, CameraInputMetricKind::InputSurfaceSize, events);
    }
    if matches!(status, InputMetricStatus::Missing) {
        input.clear_orbit().clear_pan();
    }
}

fn push_missing_metric(
    camera: Entity,
    metric: CameraInputMetricKind,
    events: &mut Vec<LifecycleEvent<OrbitCamKind>>,
) {
    events.push(LifecycleEvent::MetricsMissing(CameraInputMetricsMissing {
        camera,
        metric,
    }));
}

fn push_state_transition<K: CameraInputLifecycleKind>(
    _camera: Entity,
    kind: K::InteractionKind,
    previous: InteractionSources,
    current: InteractionSources,
    events: &mut Vec<LifecycleEvent<K>>,
) {
    match (previous.is_empty(), current.is_empty(), previous == current) {
        (true, false, _) => events.push(LifecycleEvent::Started {
            kind,
            sources: current,
        }),
        (false, true, _) => events.push(LifecycleEvent::Ended {
            kind,
            sources: previous,
        }),
        (false, false, false) => events.push(LifecycleEvent::SourcesChanged {
            kind,
            previous,
            current,
        }),
        (true, true, _) | (false, false, true) => {},
    }
}

/// Computes the debounced reported speed and its pending-settle deadline.
///
/// `Slow` reports immediately. A return to `Normal` — a fresh engage or a chord
/// release — is held back by `window`, so `Normal` is not reported for the frame
/// or two a gamepad slow-gate chord straddles. Only the gamepad has a slow gate,
/// so non-gamepad sources report their live speed at once. A `None` report means
/// no settled speed: either the kind is inactive, or it is active and still
/// inside the settle window.
fn settled_speed(
    previous: Option<ControlSpeed>,
    previous_deadline: Option<f32>,
    active: bool,
    live_speed: ControlSpeed,
    sources: InteractionSources,
    now: f32,
    window: f32,
) -> (Option<ControlSpeed>, Option<f32>) {
    if !active {
        return (None, None);
    }
    if window <= 0.0
        || !sources.intersects(InteractionSources::GAMEPAD)
        || matches!(live_speed, ControlSpeed::Slow)
    {
        return (Some(live_speed), None);
    }
    if matches!(previous, Some(ControlSpeed::Normal)) {
        return (Some(ControlSpeed::Normal), None);
    }
    // Active gamepad at `Normal`, not yet settled: hold the prior report (`Slow`
    // on a chord release, `None` on a fresh engage) until the deadline elapses.
    match previous_deadline {
        None => (previous, Some(now + window)),
        Some(deadline) if now >= deadline => (Some(ControlSpeed::Normal), None),
        Some(_) => (previous, previous_deadline),
    }
}

/// Computes a kind's debounced reported sources and records its source-settle
/// deadline into `settle`. A thin wrapper over [`settled_sources`] that threads
/// the kind's previous deadline in and the new one back out.
fn debounce_sources<K: CameraInputLifecycleKind>(
    settle: &mut CameraReportedInteractionSettle<K>,
    kind: K::InteractionKind,
    previous: InteractionSources,
    live: InteractionSources,
    context: SettleContext<K>,
) -> InteractionSources {
    let (reported, deadline) = settled_sources(
        previous,
        context.previous.source_deadline(kind),
        live,
        context.now,
        context.window,
    );
    settle.set_source_deadline(kind, deadline);
    reported
}

/// Computes the debounced reported source set and its pending-settle deadline.
///
/// A newly-engaged source reports at once, as does any frame the live set keeps
/// everything it reported. When the live set drops a source, the dropped source
/// stays in the reported set until `window` elapses, so the per-frame gaps in
/// bursty input — a trackpad emulating a mouse button as intermittent
/// press/release, or smooth-scroll arriving in bursts — do not blink the report
/// off. `window <= 0.0` reports the live set verbatim.
fn settled_sources(
    previous: InteractionSources,
    previous_deadline: Option<f32>,
    live: InteractionSources,
    now: f32,
    window: f32,
) -> (InteractionSources, Option<f32>) {
    let dropped = previous.difference(live);
    if window <= 0.0 || dropped.is_empty() {
        return (live, None);
    }
    let held = previous.union(live);
    match previous_deadline {
        None => (held, Some(now + window)),
        Some(deadline) if now >= deadline => (live, None),
        Some(_) => (held, previous_deadline),
    }
}

/// The reported zoom direction from the live zoom sign. Held to the previous
/// direction on a zero-delta frame so it persists through the reporting-debounce
/// window, and cleared when no zoom is reported. Reading the live sign means a
/// reversal (zoom-in to zoom-out) updates at once, without waiting on a settle.
fn reported_zoom_direction(
    input: &OrbitCamInput,
    zoom_reported: InteractionSources,
    previous: Option<ZoomDirection>,
) -> Option<ZoomDirection> {
    if zoom_reported.is_empty() {
        return None;
    }
    let amount = input.zoom_coarse().amount() + input.zoom_smooth().amount();
    if amount > 0.0 {
        Some(ZoomDirection::In)
    } else if amount < 0.0 {
        Some(ZoomDirection::Out)
    } else {
        previous
    }
}

/// The reported `FreeCam` move directions, gated on the reported translate or
/// roll sources so they clear once the interaction ends. The adapter fills the
/// live set from the resolved move vector, boost gate, and roll sign; a control
/// panel holds the last non-empty set through its own release window so the
/// highlighted rows fade rather than blink.
const fn reported_free_directions(
    input: &FreeCamInput,
    state: &FreeCamInteractionState,
) -> FreeCamActiveDirections {
    let translate_reported = !state.sources(FreeCamInteractionKind::Translate).is_empty();
    let roll_reported = !state.sources(FreeCamInteractionKind::Roll).is_empty();
    if translate_reported || roll_reported {
        input.directions()
    } else {
        FreeCamActiveDirections::NONE
    }
}

fn apply_lifecycle_event<K: CameraInputLifecycleKind>(
    world: &mut World,
    camera: Entity,
    event: LifecycleEvent<K>,
) {
    match event {
        LifecycleEvent::Started { kind, sources } => {
            world
                .resource_mut::<CameraInputSourceLatches>()
                .acquire_sources(camera, sources);
            K::trigger_started(world, camera, kind, sources);
        },
        LifecycleEvent::Ended { kind, sources } => {
            world
                .resource_mut::<CameraInputSourceLatches>()
                .release_sources(camera, sources);
            K::trigger_ended(world, camera, kind, sources);
        },
        LifecycleEvent::SourcesChanged {
            kind,
            previous,
            current,
        } => {
            let removed = previous.difference(current);
            let added = current.difference(previous);
            {
                let mut latches = world.resource_mut::<CameraInputSourceLatches>();
                latches.release_sources(camera, removed);
                latches.acquire_sources(camera, added);
            }
            K::trigger_sources_changed(world, camera, kind, previous, current);
        },
        LifecycleEvent::SpeedChanged { kind, speed } => {
            K::trigger_speed_changed(world, camera, kind, speed);
        },
        LifecycleEvent::MetricsMissing(event) => {
            world.entity_mut(camera).trigger(|_| event);
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::camera::RenderTarget;
    use bevy::math::curve::easing::EaseFunction;
    use bevy::prelude::Bundle;
    use bevy::prelude::Camera;
    use bevy::prelude::In;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::ResMut;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::window::WindowRef;
    use hana_kana::EasingPlugin;

    use super::*;
    use crate::CameraCommands;
    use crate::CameraInputInterruptBehavior;
    use crate::CameraMove;
    use crate::CameraPlaybackObservation;
    use crate::Focus;
    use crate::FreeCamRollTarget;
    use crate::OrbitAngles;
    use crate::OrbitCam;
    use crate::PlayAnimation;
    use crate::Radius;
    use crate::SequenceOwner;
    use crate::SequenceOwnership;
    use crate::animation::AnimationPlugin;
    use crate::input::CameraInputDisabled;
    use crate::input::CameraInputModesPlugin;
    use crate::input::CameraInputRoutingConfig;
    use crate::input::CameraInputRoutingPlugin;
    use crate::input::InputGain;
    use crate::input::OrbitCamInputGain;
    use crate::input::OrbitCamInputMode;
    use crate::input::OrbitCamPreset;
    use crate::input::OrbitCamSimpleMousePreset;
    use crate::system_sets::LagrangeSystemSetsPlugin;

    #[derive(Resource, Default)]
    struct LifecycleCounts {
        started:         usize,
        ended:           usize,
        sources_changed: usize,
        metrics_missing: usize,
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            LagrangeSystemSetsPlugin,
            CameraInputRoutingPlugin,
            CameraInputLifecyclePlugin,
        ));
        app.init_resource::<LifecycleCounts>();
        // The integration tests exercise the raw start/change/end lifecycle;
        // disable the reporting debounce so a source drop ends in the same frame.
        app.insert_resource(CameraInputReportingDebounce(Duration::ZERO));
        app
    }

    fn lifecycle_only_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            LagrangeSystemSetsPlugin,
            CameraInputLifecyclePlugin,
        ));
        app.init_resource::<LifecycleCounts>()
            .init_resource::<CameraInputSourceLatches>();
        app.insert_resource(CameraInputReportingDebounce(Duration::ZERO));
        app
    }

    fn mode_cleanup_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            EasingPlugin,
            AnimationPlugin,
            LagrangeSystemSetsPlugin,
            CameraInputModesPlugin,
            CameraInputRoutingPlugin,
            CameraInputLifecyclePlugin,
        ));
        app.init_resource::<LifecycleCounts>();
        app.insert_resource(CameraInputReportingDebounce(Duration::from_secs_f32(
            WINDOW,
        )));
        app
    }

    fn spawn_camera(world: &mut World, components: impl Bundle) -> Entity {
        world
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                components,
            ))
            .id()
    }

    fn observe_camera_playback(
        input: In<Entity>,
        commands: CameraCommands,
    ) -> (SequenceOwnership, CameraPlaybackObservation) {
        (commands.owner(input.0), commands.observe(input.0))
    }

    fn observe_counts(world: &mut World, camera: Entity) {
        world.entity_mut(camera).observe(
            |_event: On<OrbitCamInteractionStarted>, mut counts: ResMut<LifecycleCounts>| {
                counts.started += 1;
            },
        );
        world.entity_mut(camera).observe(
            |_event: On<OrbitCamInteractionEnded>, mut counts: ResMut<LifecycleCounts>| {
                counts.ended += 1;
            },
        );
        world.entity_mut(camera).observe(
            |_event: On<OrbitCamInteractionSourcesChanged>, mut counts: ResMut<LifecycleCounts>| {
                counts.sources_changed += 1;
            },
        );
        world.entity_mut(camera).observe(
            |_event: On<CameraInputMetricsMissing>, mut counts: ResMut<LifecycleCounts>| {
                counts.metrics_missing += 1;
            },
        );
    }

    type TestResult = Result<(), &'static str>;

    fn update_input(
        app: &mut App,
        camera: Entity,
        apply: impl FnOnce(&mut OrbitCamInput),
    ) -> TestResult {
        let mut input = app
            .world_mut()
            .get_mut::<OrbitCamInput>(camera)
            .ok_or("camera missing OrbitCamInput")?;
        apply(&mut input);
        Ok(())
    }

    fn input(app: &App, camera: Entity) -> Result<OrbitCamInput, &'static str> {
        app.world()
            .get::<OrbitCamInput>(camera)
            .copied()
            .ok_or("camera missing OrbitCamInput")
    }

    fn interaction_state(
        app: &App,
        camera: Entity,
    ) -> Result<OrbitCamInteractionState, &'static str> {
        app.world()
            .get::<OrbitCamInteractionState>(camera)
            .copied()
            .ok_or("camera missing OrbitCamInteractionState")
    }

    fn reported_settle(
        app: &App,
        camera: Entity,
    ) -> Result<ReportedInteractionSettle, &'static str> {
        app.world()
            .get::<ReportedInteractionSettle>(camera)
            .copied()
            .ok_or("camera missing ReportedInteractionSettle")
    }

    fn zero_sensitive_simple_mouse_mode() -> OrbitCamInputMode {
        let disabled = InputGain::DISABLED.0;
        OrbitCamInputMode::with_preset(
            OrbitCamSimpleMousePreset::default()
                .mouse_input_gain(OrbitCamInputGain::uniform(disabled))
                .smooth_scroll_input_gain(OrbitCamInputGain::uniform(disabled)),
        )
    }

    fn cleanup_camera_move() -> CameraMove {
        CameraMove::try_to_orbital_look_at(
            Focus(Vec3::ZERO),
            OrbitAngles {
                yaw:   CLEANUP_YAW,
                pitch: CLEANUP_PITCH,
            },
            Radius(CLEANUP_RADIUS),
            FreeCamRollTarget::InheritPrevious,
            Duration::from_millis(CLEANUP_MOVE_DURATION_MILLIS),
            EaseFunction::Linear,
        )
        .expect("cleanup camera move is finite")
    }

    #[test]
    fn held_interaction_starts_changes_and_ends() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        observe_counts(app.world_mut(), camera);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        update_input(&mut app, camera, |input| {
            input.mark_orbit_active_with_sources(InteractionSources::MOUSE);
        })?;
        app.update();
        assert_eq!(app.world().resource::<LifecycleCounts>().started, 1);
        assert_eq!(
            interaction_state(&app, camera)?.orbit_sources(),
            InteractionSources::MOUSE
        );

        update_input(&mut app, camera, |input| {
            input.mark_orbit_active_with_sources(
                InteractionSources::MOUSE.union(InteractionSources::KEYBOARD),
            );
        })?;
        app.update();
        assert_eq!(app.world().resource::<LifecycleCounts>().sources_changed, 1);

        update_input(&mut app, camera, |input| {
            input.clear();
        })?;
        app.update();
        assert_eq!(app.world().resource::<LifecycleCounts>().ended, 1);

        Ok(())
    }

    #[test]
    fn coarse_zoom_registers_then_ends_when_input_clears() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        observe_counts(app.world_mut(), camera);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        update_input(&mut app, camera, |input| {
            input.add_zoom_coarse_with_sources(1.0, InteractionSources::WHEEL);
        })?;
        app.update();
        assert_eq!(app.world().resource::<LifecycleCounts>().started, 1);
        assert_eq!(
            interaction_state(&app, camera)?.zoom_sources(),
            InteractionSources::WHEEL
        );

        // The wheel is a one-frame impulse: once its input clears, the zoom
        // interaction ends — immediately here, since `test_app` disables the
        // reporting debounce.
        update_input(&mut app, camera, |input| {
            input.clear();
        })?;
        app.update();
        assert_eq!(app.world().resource::<LifecycleCounts>().ended, 1);
        assert!(interaction_state(&app, camera)?.zoom_sources().is_empty());

        Ok(())
    }

    #[test]
    fn manual_zero_delta_active_state_emits_lifecycle() -> TestResult {
        let mut app = lifecycle_only_app();
        let camera = spawn_camera(
            app.world_mut(),
            (
                OrbitCamInputMode::Manual,
                CameraInputSurfaceMetrics::camera_view_and_input_surface(Vec2::ONE, Vec2::ONE),
            ),
        );
        observe_counts(app.world_mut(), camera);

        update_input(&mut app, camera, |input| {
            input.mark_pan_active_with_sources(InteractionSources::MANUAL);
        })?;
        app.update();

        assert_eq!(app.world().resource::<LifecycleCounts>().started, 1);
        assert_eq!(
            interaction_state(&app, camera)?.pan_sources(),
            InteractionSources::MANUAL
        );

        Ok(())
    }

    #[test]
    fn blocked_camera_ends_existing_interaction() -> TestResult {
        let mut app = test_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        observe_counts(app.world_mut(), camera);
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));

        update_input(&mut app, camera, |input| {
            input.mark_orbit_active_with_sources(InteractionSources::MOUSE);
        })?;
        app.update();
        app.world_mut()
            .entity_mut(camera)
            .insert(CameraInputDisabled);
        update_input(&mut app, camera, |input| {
            input.mark_orbit_active_with_sources(InteractionSources::MOUSE);
        })?;
        app.update();

        assert_eq!(app.world().resource::<LifecycleCounts>().ended, 1);
        assert!(!input(&app, camera)?.has_input());

        Ok(())
    }

    #[test]
    fn zero_sensitive_mode_replacement_flushes_debounced_state() -> TestResult {
        let mut app = mode_cleanup_app();
        let camera = spawn_camera(
            app.world_mut(),
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        app.update();

        update_input(&mut app, camera, |input| {
            input.mark_orbit_active_with_sources(InteractionSources::MOUSE);
        })?;
        app.update();
        assert_eq!(
            interaction_state(&app, camera)?.orbit_sources(),
            InteractionSources::MOUSE
        );

        update_input(&mut app, camera, |input| {
            input.clear();
        })?;
        app.update();
        assert_eq!(
            interaction_state(&app, camera)?.orbit_sources(),
            InteractionSources::MOUSE
        );
        assert!(
            reported_settle(&app, camera)?
                .source_deadline(OrbitCamInteractionKind::Orbit)
                .is_some()
        );
        assert_ne!(
            app.world().resource::<CameraInputSourceLatches>(),
            &CameraInputSourceLatches::default()
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(CameraInputInterruptBehavior::Cancel);
        app.world_mut()
            .trigger(PlayAnimation::new(camera, [cleanup_camera_move()]));
        app.world_mut()
            .entity_mut(camera)
            .insert(zero_sensitive_simple_mouse_mode());
        app.update();

        assert!(interaction_state(&app, camera)?.orbit_sources().is_empty());
        assert!(
            reported_settle(&app, camera)?
                .source_deadline(OrbitCamInteractionKind::Orbit)
                .is_none()
        );
        assert_eq!(
            app.world().resource::<CameraInputSourceLatches>(),
            &CameraInputSourceLatches::default()
        );
        assert!(!input(&app, camera)?.has_input());
        let (ownership, observation) = app
            .world_mut()
            .run_system_cached_with(observe_camera_playback, camera)
            .map_err(|_| "camera playback observation system did not run")?;
        assert_eq!(
            ownership,
            SequenceOwnership::Retained(SequenceOwner::NativePlayback)
        );
        assert!(matches!(
            observation,
            CameraPlaybackObservation::Retained {
                owner: SequenceOwner::NativePlayback,
                ..
            }
        ));

        Ok(())
    }

    const WINDOW: f32 = 0.5;
    const CLEANUP_MOVE_DURATION_MILLIS: u64 = 1_000;
    const CLEANUP_YAW: f32 = 1.0;
    const CLEANUP_PITCH: f32 = 0.0;
    const CLEANUP_RADIUS: f32 = 2.0;

    #[test]
    fn fresh_gamepad_normal_stays_pending_until_window_then_settles() {
        let gamepad = InteractionSources::GAMEPAD;
        // Fresh engage at Normal reports pending (None) and arms the deadline.
        let (reported, deadline) =
            settled_speed(None, None, true, ControlSpeed::Normal, gamepad, 1.0, WINDOW);
        assert_eq!(reported, None);
        assert_eq!(deadline, Some(1.5));
        // Within the window it stays pending.
        let (reported, _) = settled_speed(
            None,
            Some(1.5),
            true,
            ControlSpeed::Normal,
            gamepad,
            1.25,
            WINDOW,
        );
        assert_eq!(reported, None);
        // Once the deadline passes it settles to Normal.
        let (reported, deadline) = settled_speed(
            None,
            Some(1.5),
            true,
            ControlSpeed::Normal,
            gamepad,
            1.5,
            WINDOW,
        );
        assert_eq!(reported, Some(ControlSpeed::Normal));
        assert_eq!(deadline, None);
    }

    #[test]
    fn gamepad_slow_reports_immediately() {
        let gamepad = InteractionSources::GAMEPAD;
        let (reported, deadline) =
            settled_speed(None, None, true, ControlSpeed::Slow, gamepad, 1.0, WINDOW);
        assert_eq!(reported, Some(ControlSpeed::Slow));
        assert_eq!(deadline, None);
    }

    #[test]
    fn chord_release_holds_slow_until_window_then_settles() {
        let gamepad = InteractionSources::GAMEPAD;
        // Live drops to Normal on release; the prior Slow is held and armed.
        let (reported, deadline) = settled_speed(
            Some(ControlSpeed::Slow),
            None,
            true,
            ControlSpeed::Normal,
            gamepad,
            2.0,
            WINDOW,
        );
        assert_eq!(reported, Some(ControlSpeed::Slow));
        assert_eq!(deadline, Some(2.5));
        // After the deadline it settles to Normal.
        let (reported, _) = settled_speed(
            Some(ControlSpeed::Slow),
            Some(2.5),
            true,
            ControlSpeed::Normal,
            gamepad,
            2.5,
            WINDOW,
        );
        assert_eq!(reported, Some(ControlSpeed::Normal));
    }

    #[test]
    fn non_gamepad_normal_reports_immediately() {
        let mouse = InteractionSources::MOUSE;
        let (reported, deadline) =
            settled_speed(None, None, true, ControlSpeed::Normal, mouse, 1.0, WINDOW);
        assert_eq!(reported, Some(ControlSpeed::Normal));
        assert_eq!(deadline, None);
    }

    #[test]
    fn zero_window_disables_the_settle_delay() {
        let gamepad = InteractionSources::GAMEPAD;
        let (reported, deadline) =
            settled_speed(None, None, true, ControlSpeed::Normal, gamepad, 1.0, 0.0);
        assert_eq!(reported, Some(ControlSpeed::Normal));
        assert_eq!(deadline, None);
    }

    #[test]
    fn inactive_kind_clears_report_and_deadline() {
        let gamepad = InteractionSources::GAMEPAD;
        let (reported, deadline) = settled_speed(
            Some(ControlSpeed::Slow),
            Some(5.0),
            false,
            ControlSpeed::Normal,
            gamepad,
            1.0,
            WINDOW,
        );
        assert_eq!(reported, None);
        assert_eq!(deadline, None);
    }

    #[test]
    fn fresh_source_reports_immediately() {
        let (reported, deadline) = settled_sources(
            InteractionSources::NONE,
            None,
            InteractionSources::MOUSE,
            1.0,
            WINDOW,
        );
        assert_eq!(reported, InteractionSources::MOUSE);
        assert_eq!(deadline, None);
    }

    #[test]
    fn gaining_a_source_reports_immediately() {
        let both = InteractionSources::MOUSE.union(InteractionSources::KEYBOARD);
        let (reported, deadline) =
            settled_sources(InteractionSources::MOUSE, None, both, 1.0, WINDOW);
        assert_eq!(reported, both);
        assert_eq!(deadline, None);
    }

    #[test]
    fn dropped_source_holds_until_window_then_clears() {
        let mouse = InteractionSources::MOUSE;
        // Live drops to nothing: the source is held and the deadline armed.
        let (reported, deadline) =
            settled_sources(mouse, None, InteractionSources::NONE, 2.0, WINDOW);
        assert_eq!(reported, mouse);
        assert_eq!(deadline, Some(2.5));
        // Within the window it stays held.
        let (reported, _) =
            settled_sources(mouse, Some(2.5), InteractionSources::NONE, 2.25, WINDOW);
        assert_eq!(reported, mouse);
        // Once the deadline passes the source clears.
        let (reported, deadline) =
            settled_sources(mouse, Some(2.5), InteractionSources::NONE, 2.5, WINDOW);
        assert!(reported.is_empty());
        assert_eq!(deadline, None);
    }

    #[test]
    fn re_engaging_during_hold_clears_the_deadline() {
        let mouse = InteractionSources::MOUSE;
        let (reported, deadline) = settled_sources(mouse, Some(2.5), mouse, 2.25, WINDOW);
        assert_eq!(reported, mouse);
        assert_eq!(deadline, None);
    }

    #[test]
    fn partial_drop_holds_the_union_until_the_window() {
        let both = InteractionSources::MOUSE.union(InteractionSources::KEYBOARD);
        // KEYBOARD drops while MOUSE stays live: the union is held.
        let (reported, deadline) =
            settled_sources(both, None, InteractionSources::MOUSE, 1.0, WINDOW);
        assert_eq!(reported, both);
        assert_eq!(deadline, Some(1.5));
    }

    #[test]
    fn zero_window_reports_live_sources_immediately() {
        let (reported, deadline) = settled_sources(
            InteractionSources::MOUSE,
            None,
            InteractionSources::NONE,
            1.0,
            0.0,
        );
        assert!(reported.is_empty());
        assert_eq!(deadline, None);
    }
}
