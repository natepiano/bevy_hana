mod driver;
mod easing;
mod playback;
mod stages;
mod time;
mod traversal;

use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::SystemSet;
pub use driver::DisplacedDriver;
pub use driver::DriverRestoration;
pub use driver::SequenceCommand;
pub use driver::SequenceCommandRejected;
pub use driver::SequenceCommandResponse;
pub use driver::SequenceCommands;
pub use driver::SequenceDriver;
pub use driver::SequenceDriverClaimRejected;
pub use driver::SequenceDriverReleased;
pub use driver::SequenceDriverSelected;
pub use driver::SequenceDriverTakeover;
pub use driver::SequenceEvaluation;
pub use driver::SequenceMovementApplication;
pub use driver::SequenceOwner;
pub use driver::SequenceOwnership;
pub use driver::SequenceSeekResponse;
pub use driver::SequenceSourceState;
pub use easing::SequenceEasing;
pub use easing::SequenceEasingCurve;
pub use easing::SequenceEasingError;
pub use easing::SequenceEasingSample;
pub use easing::SequenceEasingSampler;
pub use playback::SequenceCommandOutcome;
pub use playback::SequenceDirection;
pub use playback::SequenceMovement;
pub use playback::SequenceMovementError;
pub use playback::SequencePlayback;
pub use playback::SequencePlaybackError;
pub use playback::SequencePosition;
pub use playback::SequencePositionError;
pub use stages::SequenceRange;
pub use stages::SequenceScope;
pub use stages::SequenceScopeError;
pub use stages::SequenceStageId;
pub use stages::SequenceStageSpan;
pub use stages::SequenceStages;
pub use stages::SequenceStagesRevision;
pub use time::SequenceTime;
pub use traversal::RangeCrossing;
pub use traversal::RangeCrossings;
pub use traversal::RangeCrossingsError;
pub use traversal::RangeEdge;
pub use traversal::RangeTransition;
pub use traversal::SequenceTraversal;
pub use traversal::SequenceUpdate;

use crate::easing::EasingPlugin;

/// Installs the shared driver arbitration, movement order, and evaluation order.
///
/// Domain plugins place their own evaluation systems in
/// [`SequencePlaybackSystems::EvaluateSequences`] and their producers in
/// [`SequencePlaybackSystems::ProduceMovement`]. `is_unique` returns `false`,
/// so several domain plugins can each add this plugin; only the first
/// composition installs the arbitration systems.
pub struct SequencePlaybackPlugin;

impl Plugin for SequencePlaybackPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EasingPlugin)
            .register_type::<DisplacedDriver>()
            .register_type::<DriverRestoration>()
            .register_type::<RangeCrossing>()
            .register_type::<RangeCrossings>()
            .register_type::<RangeEdge>()
            .register_type::<RangeTransition>()
            .register_type::<SequenceCommand>()
            .register_type::<SequenceCommandRejected>()
            .register_type::<SequenceDirection>()
            .register_type::<SequenceDriver>()
            .register_type::<SequenceDriverClaimRejected>()
            .register_type::<SequenceDriverReleased>()
            .register_type::<SequenceDriverSelected>()
            .register_type::<SequenceDriverTakeover>()
            .register_type::<SequenceEasing>()
            .register_type::<SequenceEvaluation>()
            .register_type::<SequenceMovement>()
            .register_type::<SequenceOwner>()
            .register_type::<SequencePosition>()
            .register_type::<SequenceScope>()
            .register_type::<SequenceSourceState>()
            .register_type::<SequenceStageId>()
            .register_type::<SequenceStageSpan>()
            .register_type::<SequenceStages>()
            .register_type::<SequenceStagesRevision>()
            .register_type::<SequenceTime>()
            .configure_sets(
                Update,
                (
                    SequencePlaybackSystems::ArbitrateDrivers,
                    SequencePlaybackSystems::ProduceMovement,
                    SequencePlaybackSystems::ApplyMovement,
                    SequencePlaybackSystems::EvaluateSequences,
                )
                    .chain(),
            );

        if app
            .world()
            .contains_resource::<SharedArbitrationInstalled>()
        {
            return;
        }

        app.insert_resource(SharedArbitrationInstalled).add_systems(
            Update,
            (
                driver::resolve_driver_claims,
                driver::resolve_driver_releases,
                driver::clear_producer_source_state,
            )
                .chain()
                .in_set(SequencePlaybackSystems::ArbitrateDrivers),
        );
    }

    fn is_unique(&self) -> bool { false }
}

/// Records that this app already runs the shared arbitration systems.
///
/// [`SequencePlaybackPlugin`] is repeatable, so every composition after the
/// first configures ordering and reflection again but installs no second copy
/// of the arbitration systems.
#[derive(Resource)]
struct SharedArbitrationInstalled;

/// Shared ordering boundaries for driver arbitration, movement producers, and
/// sequence evaluators.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SequencePlaybackSystems {
    /// Shared systems that resolve which producer drives each sequence.
    ArbitrateDrivers,
    /// Systems that write producer movement, including the optional tween
    /// adapter.
    ProduceMovement,
    /// Systems that apply the selected producer's movement to local position.
    ApplyMovement,
    /// Systems that evaluate domain sequences from the current position.
    EvaluateSequences,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::app::Update;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::entity::Entity;
    use bevy::ecs::observer::On;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::resource::Resource;
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use bevy::ecs::system::ResMut;
    use bevy::reflect::TypePath;

    use super::*;
    use crate::SequenceTweenAdapterPlugin;
    use crate::easing::Easing;
    use crate::easing::EasingCurve;
    use crate::easing::EasingInput;
    use crate::easing::EasingInterpolation;
    use crate::easing::EasingKnot;
    use crate::easing::EasingMapping;
    use crate::easing::EasingOutput;
    use crate::easing::EasingSlope;
    use crate::easing::EasingSlopes;

    #[derive(Resource, Default)]
    struct SystemOrder(Vec<SequencePlaybackSystems>);

    #[test]
    fn playback_system_sets_run_as_an_update_chain() {
        let mut app = App::new();
        app.add_plugins(SequencePlaybackPlugin)
            .init_resource::<SystemOrder>()
            .add_systems(
                Update,
                (
                    record_driver_arbitration.in_set(SequencePlaybackSystems::ArbitrateDrivers),
                    record_movement_production.in_set(SequencePlaybackSystems::ProduceMovement),
                    record_movement_application.in_set(SequencePlaybackSystems::ApplyMovement),
                    record_sequence_evaluation.in_set(SequencePlaybackSystems::EvaluateSequences),
                ),
            );

        app.update();

        assert_eq!(
            app.world().resource::<SystemOrder>().0,
            vec![
                SequencePlaybackSystems::ArbitrateDrivers,
                SequencePlaybackSystems::ProduceMovement,
                SequencePlaybackSystems::ApplyMovement,
                SequencePlaybackSystems::EvaluateSequences,
            ]
        );
    }

    #[test]
    fn the_shared_contract_reaches_the_type_registry() {
        let mut app = App::new();
        app.add_plugins(SequencePlaybackPlugin);

        let type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let type_registry = type_registry.read();
        let unregistered: Vec<&str> = [
            contract_type::<DisplacedDriver>(),
            contract_type::<DriverRestoration>(),
            contract_type::<Easing>(),
            contract_type::<EasingCurve>(),
            contract_type::<EasingInput>(),
            contract_type::<EasingInterpolation>(),
            contract_type::<EasingKnot>(),
            contract_type::<EasingMapping>(),
            contract_type::<EasingOutput>(),
            contract_type::<EasingSlope>(),
            contract_type::<EasingSlopes>(),
            contract_type::<RangeCrossing>(),
            contract_type::<RangeCrossings>(),
            contract_type::<RangeEdge>(),
            contract_type::<RangeTransition>(),
            contract_type::<SequenceCommand>(),
            contract_type::<SequenceCommandRejected>(),
            contract_type::<SequenceDirection>(),
            contract_type::<SequenceDriver>(),
            contract_type::<SequenceDriverClaimRejected>(),
            contract_type::<SequenceDriverReleased>(),
            contract_type::<SequenceDriverSelected>(),
            contract_type::<SequenceDriverTakeover>(),
            contract_type::<SequenceEasing>(),
            contract_type::<SequenceEvaluation>(),
            contract_type::<SequenceMovement>(),
            contract_type::<SequenceOwner>(),
            contract_type::<SequenceOwnership>(),
            contract_type::<SequencePosition>(),
            contract_type::<SequenceScope>(),
            contract_type::<SequenceSourceState>(),
            contract_type::<SequenceStageId>(),
            contract_type::<SequenceStageSpan>(),
            contract_type::<SequenceStages>(),
            contract_type::<SequenceStagesRevision>(),
            contract_type::<SequenceTime>(),
        ]
        .into_iter()
        .filter(|(_, type_id)| type_registry.get(*type_id).is_none())
        .map(|(type_path, _)| type_path)
        .collect();

        assert!(
            unregistered.is_empty(),
            "the shared contract is inspectable only once every type registers: {unregistered:?}"
        );
    }

    #[test]
    fn composing_the_plugin_twice_arbitrates_one_claim_once() {
        let mut app = App::new();
        app.add_plugins((SequencePlaybackPlugin, SequencePlaybackPlugin));

        let (driver, selections) = arbitrate_one_claim(&mut app);

        assert_eq!(
            selections,
            vec![Selection {
                driver,
                displaced: DisplacedDriver::NoPriorDriver,
            }]
        );
    }

    #[test]
    fn composing_the_plugin_beside_the_tween_adapter_arbitrates_one_claim_once() {
        let mut app = App::new();
        app.add_plugins((
            AssetPlugin::default(),
            SequencePlaybackPlugin,
            SequenceTweenAdapterPlugin::<TweenTestClock>::default(),
        ));

        let (driver, selections) = arbitrate_one_claim(&mut app);

        assert_eq!(
            selections,
            vec![Selection {
                driver,
                displaced: DisplacedDriver::NoPriorDriver,
            }]
        );
    }

    /// Time context for the adapter composition test, which installs the
    /// adapter without running any clock.
    #[derive(Default, Clone, Copy)]
    struct TweenTestClock;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Selection {
        driver:    Entity,
        displaced: DisplacedDriver,
    }

    #[derive(Resource, Default)]
    struct ObservedSelections(Vec<Selection>);

    /// Runs one ordinary claim against `app` and returns the claiming producer
    /// beside every selection the app observed for it.
    fn arbitrate_one_claim(app: &mut App) -> (Entity, Vec<Selection>) {
        app.init_resource::<ObservedSelections>()
            .add_observer(record_selection);
        let target = app.world_mut().spawn_empty().id();
        let driver = app.world_mut().spawn(SequenceDriver::new(target)).id();

        app.update();

        (
            driver,
            app.world().resource::<ObservedSelections>().0.clone(),
        )
    }

    fn record_selection(
        selected: On<SequenceDriverSelected>,
        mut observed_selections: ResMut<ObservedSelections>,
    ) {
        observed_selections.0.push(Selection {
            driver:    selected.driver,
            displaced: selected.displaced,
        });
    }

    fn contract_type<T: TypePath + 'static>() -> (&'static str, TypeId) {
        (T::type_path(), TypeId::of::<T>())
    }

    fn record_driver_arbitration(mut system_order: ResMut<SystemOrder>) {
        system_order
            .0
            .push(SequencePlaybackSystems::ArbitrateDrivers);
    }

    fn record_movement_production(mut system_order: ResMut<SystemOrder>) {
        system_order
            .0
            .push(SequencePlaybackSystems::ProduceMovement);
    }

    fn record_movement_application(mut system_order: ResMut<SystemOrder>) {
        system_order.0.push(SequencePlaybackSystems::ApplyMovement);
    }

    fn record_sequence_evaluation(mut system_order: ResMut<SystemOrder>) {
        system_order
            .0
            .push(SequencePlaybackSystems::EvaluateSequences);
    }
}
