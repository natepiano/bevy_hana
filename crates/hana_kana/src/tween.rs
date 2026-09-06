use std::marker::PhantomData;

use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::reflect::Reflect;
use bevy_tween::TweenSystemSet;
use bevy_tween::interpolate::Interpolator;
use bevy_tween::tween::component_tween_system_with_time_context;

use crate::SequenceMovement;
use crate::SequencePlaybackPlugin;
use crate::SequencePlaybackSystems;
use crate::SequencePosition;

/// Installs `bevy_tween` production of [`SequenceMovement`] positions for one
/// time context.
///
/// The adapter neither creates nor advances a clock. Install
/// `bevy_tween`'s `TimeRunnerPlugin<TimeCtx>` and `EaseKindPlugin<TimeCtx>`
/// yourself, in `Update`, so the interpolation value for the current frame
/// exists before [`SequencePlaybackSystems::ProduceMovement`] runs. This plugin
/// only applies finished interpolation values to the position portion of a
/// producer's movement, inside
/// [`SequencePlaybackSystems::ProduceMovement`] and after
/// [`TweenSystemSet::UpdateInterpolationValue`], so the selected producer's
/// movement reaches local position and domain evaluation in the same `Update`.
/// Running `bevy_tween` in a later schedule such as `PostUpdate` leaves that
/// ordering edge unbound and delays every tweened position by one frame.
///
/// Install it once per time context. Two contexts are two distinct plugin
/// types, so both can coexist.
pub struct SequenceTweenAdapterPlugin<TimeCtx> {
    time_context: PhantomData<fn() -> TimeCtx>,
}

impl<TimeCtx> Default for SequenceTweenAdapterPlugin<TimeCtx> {
    fn default() -> Self {
        Self {
            time_context: PhantomData,
        }
    }
}

impl<TimeCtx> Plugin for SequenceTweenAdapterPlugin<TimeCtx>
where
    TimeCtx: Default + Send + Sync + 'static,
{
    fn build(&self, app: &mut App) {
        app.add_plugins(SequencePlaybackPlugin).add_systems(
            Update,
            component_tween_system_with_time_context::<SequencePositionInterpolator, TimeCtx>()
                .after(TweenSystemSet::UpdateInterpolationValue)
                .in_set(SequencePlaybackSystems::ProduceMovement),
        );
    }
}

/// Interpolates the position portion of a [`SequenceMovement`] between two
/// authored endpoints.
///
/// The interpolator writes position only. Direction, whole repetitions, and
/// range crossings keep the values the producer published.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct SequencePositionInterpolator {
    start: SequencePosition,
    end:   SequencePosition,
}

impl SequencePositionInterpolator {
    /// Interpolates from `start` to `end`.
    #[must_use]
    pub const fn new(start: SequencePosition, end: SequencePosition) -> Self { Self { start, end } }

    /// Returns the authored start position.
    #[must_use]
    pub const fn start(self) -> SequencePosition { self.start }

    /// Returns the authored end position.
    #[must_use]
    pub const fn end(self) -> SequencePosition { self.end }
}

impl Interpolator for SequencePositionInterpolator {
    type Item = SequenceMovement;

    /// Returns without writing when the interpolated position is not finite. A
    /// non-finite `value` gives an infinite position across a nonzero-width
    /// span and NaN across a zero-width one; either way the movement keeps the
    /// position it already carries, so no consumer reads a NaN.
    fn interpolate(&self, movement: &mut Self::Item, value: f32, _previous_value: f32) {
        let start = self.start.normalized();
        let end = self.end.normalized();
        let interpolated = (end - start).mul_add(value, start);
        if !interpolated.is_finite() {
            return;
        }
        movement.set_position(SequencePosition::clamped(interpolated));
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exactly representable interpolated positions"
)]
mod tests {
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::resource::Resource;
    use bevy::ecs::system::Commands;
    use bevy::ecs::system::Query;
    use bevy::ecs::system::ResMut;
    use bevy::time::Time;
    use bevy::time::TimeUpdateStrategy;
    use bevy_tween::DefaultTweenPlugins;
    use bevy_tween::bevy_time_runner::TimeRunnerPlugin;
    use bevy_tween::combinator::AnimationBuilderExt;
    use bevy_tween::interpolation::EaseKind;
    use bevy_tween::interpolation::EaseKindPlugin;
    use bevy_tween::tween::TargetComponent;
    use bevy_tween::tween::Tween;

    use super::*;
    use crate::RangeCrossings;
    use crate::SequenceDirection;
    use crate::SequenceMovement;

    #[derive(Default, Clone, Copy)]
    struct TestClock;

    #[derive(Resource, Default)]
    struct ProducedPositions(Vec<f32>);

    fn interpolator() -> SequencePositionInterpolator {
        SequencePositionInterpolator::new(SequencePosition::START, SequencePosition::END)
    }

    fn movement() -> SequenceMovement {
        SequenceMovement::try_new(
            SequencePosition::START,
            SequenceDirection::Forward,
            0,
            RangeCrossings::NONE,
        )
        .expect("a forward movement at the start is valid")
    }

    #[test]
    fn the_interpolator_writes_only_the_position_portion() {
        let mut movement = movement();

        interpolator().interpolate(&mut movement, 0.25, 0.0);

        assert_eq!(movement.position().normalized(), 0.25);
        assert_eq!(movement.direction(), SequenceDirection::Forward);
        assert_eq!(movement.whole_repetitions(), 0);
        assert_eq!(movement.range_crossings(), RangeCrossings::NONE);
    }

    #[test]
    fn the_interpolator_clamps_values_outside_the_unit_interval() {
        let mut movement = movement();

        interpolator().interpolate(&mut movement, 1.5, 0.0);

        assert_eq!(movement.position(), SequencePosition::END);
    }

    #[test]
    fn the_interpolator_holds_the_prior_position_on_a_non_finite_value() {
        let mut movement = movement();
        interpolator().interpolate(&mut movement, 0.25, 0.0);

        interpolator().interpolate(&mut movement, f32::NAN, 0.25);

        assert_eq!(movement.position().normalized(), 0.25);

        let zero_width =
            SequencePositionInterpolator::new(SequencePosition::START, SequencePosition::START);
        zero_width.interpolate(&mut movement, f32::INFINITY, 0.25);

        assert_eq!(movement.position().normalized(), 0.25);
    }

    #[test]
    fn the_adapter_creates_no_clock() {
        let mut app = App::new();
        app.add_plugins((
            AssetPlugin::default(),
            SequenceTweenAdapterPlugin::<TestClock>::default(),
        ));

        assert!(app.world().get_resource::<Time<TestClock>>().is_none());
        assert!(!app.is_plugin_added::<TimeRunnerPlugin<TestClock>>());
        assert!(!app.is_plugin_added::<EaseKindPlugin<TestClock>>());
    }

    #[test]
    fn a_tween_moves_the_producer_position_before_domain_evaluation() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(DefaultTweenPlugins::<()>::in_schedule(Update))
            .add_plugins(SequenceTweenAdapterPlugin::<()>::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                250,
            )))
            .init_resource::<ProducedPositions>()
            .add_systems(
                Update,
                record_positions.in_set(SequencePlaybackSystems::EvaluateSequences),
            );

        app.world_mut()
            .run_system_cached(spawn_position_tween)
            .expect("the spawning system runs");

        app.update();
        app.update();

        let produced = &app.world().resource::<ProducedPositions>().0;
        assert!(
            produced.last().copied().unwrap_or_default() > 0.0,
            "the tween advanced the producer position before evaluation, got {produced:?}"
        );
    }

    fn spawn_position_tween(mut commands: Commands) {
        let producer = commands.spawn(movement()).id();
        commands.entity(producer).animation().insert_tween_here(
            Duration::from_secs(1),
            EaseKind::Linear,
            Tween::<TargetComponent, SequencePositionInterpolator>::new_target(
                TargetComponent::entity(producer),
                interpolator(),
            ),
        );
    }

    fn record_positions(
        movements: Query<&SequenceMovement>,
        mut produced_positions: ResMut<ProducedPositions>,
    ) {
        for movement in &movements {
            produced_positions.0.push(movement.position().normalized());
        }
    }
}
