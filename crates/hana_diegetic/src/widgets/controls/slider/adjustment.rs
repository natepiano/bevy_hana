use bevy::picking::pointer::PointerId;
use bevy::prelude::Commands;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::With;

use super::SliderAdjustment;
use super::SliderAuthoringLock;
use super::SliderDirection;
use super::SliderProposalValue;
use super::SliderRange;
use super::SliderState;
use super::SliderStepping;
use crate::PanelElementId;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetStepDirection;

/// What produced a [`SliderChangeRequested`] proposal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SliderProposalSource {
    /// A pointer press, drag, or release on the slider.
    Pointer(PointerId),
    /// A `WidgetInput::Step` from the widget input bindings (the arrow keys by default).
    Semantic,
    /// A `RequestSliderAdjustment` or another application-built proposal.
    Request,
}

/// Proposes a new applied value for a slider widget.
///
/// Hana emits proposals and never applies them; application code applies or
/// rejects each with [`SliderState::set_proposed_value`], or installs
/// [`slider_self_update`] for uncontrolled sliders. Pointer interactions set
/// [`SliderProposalSource::Pointer`], widget step input sets
/// [`SliderProposalSource::Semantic`], and [`RequestSliderAdjustment`] sets
/// [`SliderProposalSource::Request`]. Code that sets a real value directly uses
/// [`SliderState::set_value`].
#[derive(Clone, Debug, EntityEvent)]
pub struct SliderChangeRequested {
    /// Live slider entity receiving the proposal.
    #[event_target]
    pub entity:   Entity,
    /// Panel-local slider id.
    pub id:       PanelElementId,
    /// Proposed real value. Application code accepts it through
    /// [`SliderState::set_proposed_value`], which snaps motion and adjustment
    /// proposals while preserving reset defaults and held values.
    pub value:    SliderProposalValue,
    /// Whether this proposal completes its originating interaction.
    pub is_final: bool,
    /// Origin of this proposal.
    pub source:   SliderProposalSource,
}

/// Requests a computed slider-value proposal without applying it.
///
/// The handler reads the target's [`SliderState`], computes the raw-domain
/// target value, and emits one final [`SliderChangeRequested`] with
/// [`SliderProposalSource::Request`]. A controller starting from an authored
/// `(panel, id)` resolves the widget entity through
/// [`PanelWidgetReader`](crate::PanelWidgetReader)
/// before constructing the request. Disabled sliders ignore requests.
#[derive(Clone, Copy, Debug, EntityEvent)]
pub struct RequestSliderAdjustment {
    /// Live slider entity to adjust.
    #[event_target]
    pub entity:     Entity,
    /// How to compute the proposed value from the applied value.
    pub adjustment: SliderAdjustment,
}

/// Emits one final [`SliderChangeRequested`] proposal for a computed
/// adjustment on an enabled slider.
pub(super) fn handle_adjustment_request(
    request: On<RequestSliderAdjustment>,
    sliders: Query<(&PanelWidget, &WidgetKind, &SliderState, Has<WidgetDisabled>)>,
    locks: Query<(), With<SliderAuthoringLock>>,
    mut commands: Commands,
) {
    let entity = request.event_target();
    let Ok((widget, kind, state, disabled)) = sliders.get(entity) else {
        return;
    };
    if *kind != WidgetKind::Slider || disabled || locks.contains(entity) {
        return;
    }
    propose_slider_adjustment(
        entity,
        widget,
        state,
        request.adjustment,
        SliderProposalSource::Request,
        &mut commands,
    );
}

/// Emits one final proposal when `adjustment` yields a finite target.
pub(super) fn propose_slider_adjustment(
    entity: Entity,
    widget: &PanelWidget,
    state: &SliderState,
    adjustment: SliderAdjustment,
    source: SliderProposalSource,
    commands: &mut Commands<'_, '_>,
) {
    let Ok(value) = state.adjusted_value(adjustment) else {
        return;
    };
    commands.trigger(SliderChangeRequested {
        entity,
        id: widget.id().clone(),
        value,
        is_final: true,
        source,
    });
}

/// Computes one adjustment from a panel direction and the slider's value axis.
pub(super) fn step_adjustment(
    state: &SliderState,
    direction: WidgetStepDirection,
) -> SliderAdjustment {
    let sign = step_sign(state.direction(), direction);
    match state.stepping() {
        SliderStepping::Stepped(_) => SliderAdjustment::RelativeSteps(sign),
        SliderStepping::Stepless => {
            SliderAdjustment::Absolute(stepless_step_target(state.range(), state.value(), sign))
        },
    }
}

const fn step_sign(axis: SliderDirection, direction: WidgetStepDirection) -> f32 {
    let increase = match axis {
        SliderDirection::LeftToRight => matches!(
            direction,
            WidgetStepDirection::Right | WidgetStepDirection::Up
        ),
        SliderDirection::RightToLeft => matches!(
            direction,
            WidgetStepDirection::Left | WidgetStepDirection::Up
        ),
        SliderDirection::BottomToTop => matches!(
            direction,
            WidgetStepDirection::Up | WidgetStepDirection::Right
        ),
        SliderDirection::TopToBottom => matches!(
            direction,
            WidgetStepDirection::Down | WidgetStepDirection::Right
        ),
    };
    if increase { 1.0 } else { -1.0 }
}

fn stepless_step_target(range: SliderRange, current: f32, sign: f32) -> f32 {
    let fraction = (range.value_to_fraction(current) + sign / 20.0).clamp(0.0, 1.0);
    range.fraction_to_value(fraction)
}

/// Opt-in uncontrolled-slider observer that applies every proposal.
///
/// Hana never installs this observer; an application that does not own its
/// slider values adds it with
/// `app.add_observer(slider_self_update)`. Controlled sliders instead observe
/// [`SliderChangeRequested`] directly and apply or reject each proposal with
/// [`SliderState::set_proposed_value`].
pub fn slider_self_update(
    change: On<SliderChangeRequested>,
    mut sliders: Query<&mut SliderState>,
    locks: Query<(), With<SliderAuthoringLock>>,
) {
    if locks.contains(change.entity) {
        return;
    }
    let Ok(mut state) = sliders.get_mut(change.event_target()) else {
        return;
    };
    if state
        .bypass_change_detection()
        .set_proposed_value(change.value)
    {
        state.set_changed();
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::App;
    use bevy::prelude::Commands;
    use bevy::prelude::Component;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::SpawnRelated;
    use bevy::prelude::SpawnWith;
    use bevy::prelude::Startup;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy_enhanced_input::prelude::ActionSettings;
    use bevy_enhanced_input::prelude::ActionSpawner;
    use bevy_enhanced_input::prelude::Actions;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::Fire;
    use bevy_enhanced_input::prelude::InputAction;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use hana_rubric::Keybindings;

    use super::super::*;
    use super::*;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::WidgetInteractivity;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetsPlugin;

    #[test]
    fn every_slider_axis_maps_arrows_to_the_expected_step_sign() {
        let rows = [
            (SliderDirection::LeftToRight, [false, true, true, false]),
            (SliderDirection::RightToLeft, [true, false, true, false]),
            (SliderDirection::BottomToTop, [false, true, true, false]),
            (SliderDirection::TopToBottom, [false, true, false, true]),
        ];
        let directions = [
            WidgetStepDirection::Left,
            WidgetStepDirection::Right,
            WidgetStepDirection::Up,
            WidgetStepDirection::Down,
        ];
        for (axis, expected) in rows {
            for (direction, increase) in directions.into_iter().zip(expected) {
                assert_eq!(step_sign(axis, direction) > 0.0, increase);
            }
        }
    }

    #[test]
    fn stepless_target_uses_one_twentieth_of_the_f64_span_and_stays_finite() {
        let range = SliderRange::new(f32::MIN, f32::MAX).expect("full range is valid");
        let step = stepless_step_target(range, 0.0, 1.0);
        assert!(step.is_finite());
        assert!(step > 0.0 && step < f32::MAX);
        assert!((stepless_step_target(range, f32::MAX, 1.0) - f32::MAX).abs() < f32::EPSILON);
        assert!((stepless_step_target(range, f32::MIN, -1.0) - f32::MIN).abs() < f32::EPSILON);
        let unit = SliderRange::new(0.0, 1.0).expect("unit range is valid");
        assert!((stepless_step_target(unit, 0.5, 1.0) - 0.55).abs() < 1e-6);
    }

    #[derive(Default, Resource)]
    struct RecordedProposals(Vec<(Entity, f32, bool, SliderProposalSource)>);

    #[derive(Resource)]
    struct SliderTarget(Entity);

    #[derive(Component)]
    struct SliderInputContext;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct AdjustShift;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct AdjustHeld;

    fn record_proposal(change: On<SliderChangeRequested>, mut recorded: ResMut<RecordedProposals>) {
        recorded.0.push((
            change.event_target(),
            change.value.get(),
            change.is_final,
            change.source,
        ));
    }

    fn send_held_adjustment(
        _: On<Fire<AdjustHeld>>,
        target: Res<SliderTarget>,
        mut commands: Commands,
    ) {
        commands.trigger(RequestSliderAdjustment {
            entity:     target.0,
            adjustment: SliderAdjustment::RelativeSteps(1.0),
        });
    }

    fn spawn_slider_input(mut commands: Commands) {
        commands.spawn((
            SliderInputContext,
            Actions::<SliderInputContext>::spawn(SpawnWith(spawn_slider_actions)),
        ));
    }

    fn spawn_slider_actions(spawner: &mut ActionSpawner<SliderInputContext>) {
        let keybindings = Keybindings::new::<AdjustShift>(spawner, ActionSettings::default());
        keybindings.spawn_key::<AdjustHeld>(spawner, KeyCode::ArrowRight);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .init_resource::<RecordedProposals>()
            .add_observer(record_proposal);
        app
    }
    fn slider_tree(id: &str, slider: Slider) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).widget(id, slider), |_| {});
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &str) -> Entity {
        let id = PanelElementId::named(id);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
            .expect("widget should be reified")
    }

    fn request_adjustment(app: &mut App, entity: Entity, adjustment: SliderAdjustment) {
        app.world_mut()
            .trigger(RequestSliderAdjustment { entity, adjustment });
        app.world_mut().flush();
    }

    fn slider_value(app: &App, widget: Entity) -> f32 {
        app.world()
            .get::<SliderState>(widget)
            .expect("widget should carry slider state")
            .value()
    }

    fn press_key(app: &mut App, window: Entity, key_code: KeyCode) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn proposals_apply_only_through_explicit_app_decisions() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=1.0).value(0.5);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(0.8));
        {
            let recorded = &app.world().resource::<RecordedProposals>().0;
            assert_eq!(recorded.len(), 1, "the request should emit one proposal");
            let (target, value, is_final, source) = recorded[0];
            assert_eq!(target, widget);
            assert_close(value, 0.8);
            assert!(is_final);
            assert_eq!(source, SliderProposalSource::Request);
        }
        assert_close(slider_value(&app, widget), 0.5);

        // The app accepts the first proposal explicitly.
        let applied = app
            .world_mut()
            .get_mut::<SliderState>(widget)
            .expect("widget should carry slider state")
            .set_value(0.8);
        assert_eq!(applied, Ok(true));
        assert_close(slider_value(&app, widget), 0.8);

        // The app rejects the second proposal by not applying it.
        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(0.2));
        assert_eq!(app.world().resource::<RecordedProposals>().0.len(), 2);
        assert_close(slider_value(&app, widget), 0.8);
    }

    #[test]
    fn opt_in_self_update_applies_each_proposal() {
        let mut app = test_app();
        app.add_observer(slider_self_update);
        let slider = Slider::new(0.0..=10.0).value(5.0).step(0.5);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(7.3));
        assert_close(slider_value(&app, widget), 7.5);
        request_adjustment(&mut app, widget, SliderAdjustment::Relative(1.2));
        assert_close(slider_value(&app, widget), 8.5);
        request_adjustment(&mut app, widget, SliderAdjustment::RelativeSteps(2.0));
        assert_close(slider_value(&app, widget), 9.5);
        assert_eq!(app.world().resource::<RecordedProposals>().0.len(), 3);
    }

    #[test]
    fn self_update_marks_state_changed_only_when_the_applied_value_changes() {
        let mut app = test_app();
        app.add_observer(slider_self_update);
        let slider = Slider::new(0.0..=1.0).value(0.5);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(2.0));
        assert_close(slider_value(&app, widget), 1.0);
        let applied_tick = app
            .world()
            .entity(widget)
            .get_ref::<SliderState>()
            .expect("widget should carry slider state")
            .last_changed();

        // The repeated proposal clamps to the same endpoint; applying it must
        // not wake change detection.
        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(2.0));
        assert_close(slider_value(&app, widget), 1.0);
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<SliderState>()
                .expect("widget should carry slider state")
                .last_changed(),
            applied_tick,
            "an unchanged proposal must not mark slider state changed",
        );
    }

    #[test]
    fn adjustment_request_emits_raw_target_that_set_value_normalizes_to_endpoint() {
        let mut app = test_app();
        app.add_observer(slider_self_update);
        // Range end 1.0 is off the step-0.3 lattice; applied 0.9 sits on it.
        let slider = Slider::new(0.0..=1.0).value(0.9).step(0.3);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert_close(slider_value(&app, widget), 0.9);

        // One step up from 0.9 targets raw 1.2; the proposal carries that raw
        // value without pre-normalizing it.
        request_adjustment(&mut app, widget, SliderAdjustment::RelativeSteps(1.0));
        let recorded = &app.world().resource::<RecordedProposals>().0;
        assert_eq!(recorded.len(), 1);
        assert_close(recorded[0].1, 1.2);

        // set_proposed_value snaps 1.2 back onto the lattice then clamps into range, so
        // the slider lands on the range endpoint 1.0. Pre-normalizing the
        // proposal to 1.0 would instead snap back to 0.9.
        assert_close(slider_value(&app, widget), 1.0);
    }

    #[test]
    fn remote_requests_resolve_from_panel_and_id_and_validate_input() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=10.0).value(5.0);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        app.update();

        // An app controller starts from the authored `(panel, id)` pair and
        // resolves the live widget entity before constructing the request.
        let widget = resolve_widget(&mut app, panel, "level");
        request_adjustment(&mut app, widget, SliderAdjustment::Relative(0.4));
        let recorded = &app.world().resource::<RecordedProposals>().0;
        assert_eq!(recorded.len(), 1);
        assert_close(recorded[0].1, 5.4);

        // A step-relative request without a step and non-finite numeric
        // input both emit no proposal.
        request_adjustment(&mut app, widget, SliderAdjustment::RelativeSteps(1.0));
        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(f32::NAN));
        request_adjustment(&mut app, widget, SliderAdjustment::Relative(f32::INFINITY));
        assert_eq!(app.world().resource::<RecordedProposals>().0.len(), 1);
    }

    #[test]
    fn disabled_sliders_ignore_semantic_requests() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=1.0).value(0.5);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .widget("level", slider)
                .widget_interactivity(WidgetInteractivity::Disabled),
            |_| {},
        );
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());

        request_adjustment(&mut app, widget, SliderAdjustment::Absolute(0.8));
        assert_eq!(app.world().resource::<RecordedProposals>().0, []);
        assert_close(slider_value(&app, widget), 0.5);
    }

    #[test]
    fn held_kana_action_sends_one_adjustment_request_per_fire() {
        let mut app = test_app();
        app.add_plugins((InputPlugin, EnhancedInputPlugin))
            .add_input_context::<SliderInputContext>()
            .add_systems(Startup, spawn_slider_input)
            .add_observer(send_held_adjustment)
            .add_observer(slider_self_update);
        let slider = Slider::new(0.0..=10.0).value(5.0).step(0.5);
        let panel = spawn_panel(&mut app, slider_tree("level", slider));
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.finish();
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        app.insert_resource(SliderTarget(widget));

        press_key(&mut app, window, KeyCode::ArrowRight);
        app.update();
        assert_eq!(app.world().resource::<RecordedProposals>().0.len(), 1);
        assert_close(slider_value(&app, widget), 5.5);

        // The key stays held; each later frame fires the action again and
        // each fire sends exactly one request.
        app.update();
        assert_eq!(app.world().resource::<RecordedProposals>().0.len(), 2);
        assert_close(slider_value(&app, widget), 6.0);
    }
}
