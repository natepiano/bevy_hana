//! Standard Hana fold playback controls for Fairy Dust examples.

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::Startup;
use bevy::prelude::With;
use bevy::prelude::warn;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::Start;
use hana_rubric::Keybindings;
use hana_valence::FoldCommands;
use hana_valence::FoldPlugin;
use hana_valence::FoldSequencePlayback;
use hana_valence::SequenceCommand;
use hana_valence::SequenceCommandResponse;
use hana_valence::SequenceOwner;

use crate::constants::FOLD_CONTROL_DIAGNOSTIC_CAPACITY;
use crate::constants::FOLD_CONTROL_ID;
use crate::constants::FOLD_CONTROL_LABEL;
use crate::constants::FOLD_CONTROL_RESERVE_LABEL;
use crate::constants::FOLD_PLAY_CONTROL_ID;
use crate::constants::FOLD_PLAY_CONTROL_LABEL;
use crate::constants::FOLD_PLAY_RESERVE_LABEL;
use crate::constants::UNFOLD_CONTROL_ID;
use crate::constants::UNFOLD_CONTROL_LABEL;
use crate::ensure_plugin;
use crate::screen_panels;
use crate::screen_panels::ControlActivation;
use crate::screen_panels::TitleBarControlState;
use crate::screen_panels::TitleChip;
use crate::shortcuts;

/// Marks the sequence selected by Fairy Dust when multiple retained fold
/// sequences exist.
#[derive(Component)]
pub struct FairyDustFoldTarget;

/// Standard fold input associated with a routing diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldControlAction {
    /// One authored boundary toward the folded endpoint.
    Fold,
    /// One authored boundary toward the base endpoint.
    Unfold,
    /// Playback that selects the other endpoint from a terminal, follows the
    /// latest step direction while idle in the interior, and reverses during
    /// playback.
    Play,
}

/// Which endpoint a routed fold input travels toward.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoldTravel {
    /// Toward normalized sequence end, where every stage has folded.
    TowardFolded,
    /// Toward normalized sequence start, where every stage rests at its base.
    TowardBase,
}

impl FoldTravel {
    const fn reversed(self) -> Self {
        match self {
            Self::TowardFolded => Self::TowardBase,
            Self::TowardBase => Self::TowardFolded,
        }
    }
}

/// The fold input Fairy Dust last routed to one retained sequence.
///
/// The chip sync reads it to light exactly one control while the sequence is
/// still moving, and [`FoldControlAction::Play`] reads its travel to pick the
/// direction it continues or reverses.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
struct FoldControlIntent {
    action: FoldControlAction,
    travel: FoldTravel,
}

/// Whether a retained sequence is still moving.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoldControlMotion {
    /// A journey is running toward its destination.
    Moving,
    /// No journey is running, so the sequence holds its position.
    Settled,
}

/// Where a retained sequence rests between its two endpoints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoldStation {
    /// At normalized sequence start.
    Base,
    /// Strictly between the two endpoints.
    Interior,
    /// At normalized sequence end.
    Folded,
}

/// What the fold controls last recorded about one retained sequence.
///
/// [`sync_fold_control_chips`] writes this in `PostUpdate` from
/// [`FoldSequencePlayback`]. The input observers read it instead of playback
/// itself, because a system that holds [`FoldCommands`] already borrows every
/// [`FoldSequencePlayback`] mutably and cannot borrow one again.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
struct FoldControlProgress {
    motion:  FoldControlMotion,
    station: FoldStation,
}

impl From<&FoldSequencePlayback> for FoldControlProgress {
    fn from(playback: &FoldSequencePlayback) -> Self {
        let position = playback.normalized_position();
        Self {
            motion:  if playback.is_playing() {
                FoldControlMotion::Moving
            } else {
                FoldControlMotion::Settled
            },
            station: if position <= 0.0 {
                FoldStation::Base
            } else if position >= 1.0 {
                FoldStation::Folded
            } else {
                FoldStation::Interior
            },
        }
    }
}

impl Default for FoldControlProgress {
    fn default() -> Self {
        Self {
            motion:  FoldControlMotion::Settled,
            station: FoldStation::Base,
        }
    }
}

/// Reason a standard fold input reached no retained sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldControlDiagnosticReason {
    /// No entity carries a retained [`FoldSequencePlayback`].
    NoReadySequence,
    /// Multiple sequences are ready, but the ready sequences do not contain
    /// exactly one [`FairyDustFoldTarget`].
    AmbiguousReadySequences {
        /// Number of ready sequences.
        ready_sequences: usize,
        /// Number of ready sequences carrying [`FairyDustFoldTarget`].
        marked_targets:  usize,
    },
    /// A sequence was selected, but this owner holds its local position, so the
    /// command mutated nothing.
    SequenceHeld {
        /// Owner that rejected the command.
        owner: SequenceOwner,
    },
}

/// One standard fold input that could not be routed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldControlDiagnostic {
    /// Input that could not be routed.
    pub action: FoldControlAction,
    /// Routing failure for the input.
    pub reason: FoldControlDiagnosticReason,
}

/// Bounded history of standard fold inputs that could not be routed.
#[derive(Debug, Resource)]
pub struct FoldControlDiagnostics {
    entries: VecDeque<FoldControlDiagnostic>,
}

impl FoldControlDiagnostics {
    /// Iterates over retained diagnostics in insertion order.
    pub fn entries(&self) -> impl Iterator<Item = &FoldControlDiagnostic> { self.entries.iter() }

    /// Number of retained failed inputs.
    #[must_use]
    pub fn len(&self) -> usize { self.entries.len() }

    /// Whether no failed input has been retained.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    fn record(&mut self, diagnostic: FoldControlDiagnostic) {
        warn!(
            action = ?diagnostic.action,
            reason = ?diagnostic.reason,
            "fairy_dust fold input could not reach a retained sequence"
        );
        self.entries.push_back(diagnostic);
        while self.entries.len() > FOLD_CONTROL_DIAGNOSTIC_CAPACITY {
            self.entries.pop_front();
        }
    }
}

impl Default for FoldControlDiagnostics {
    fn default() -> Self {
        Self {
            entries: VecDeque::with_capacity(FOLD_CONTROL_DIAGNOSTIC_CAPACITY),
        }
    }
}

#[derive(Component)]
struct FoldControlContext;

#[derive(InputAction)]
#[action_output(bool)]
struct FoldStep;

#[derive(InputAction)]
#[action_output(bool)]
struct UnfoldStep;

#[derive(InputAction)]
#[action_output(bool)]
struct PlayFold;

#[derive(InputAction)]
#[action_output(bool)]
struct FoldShift;

#[derive(Resource)]
struct FoldControlsInstalled;

enum ReadySelection {
    None,
    Selected(Entity),
    Ambiguous {
        ready_sequences: usize,
        marked_targets:  usize,
    },
}

pub(crate) fn install(app: &mut App) {
    if app.world().contains_resource::<FoldControlsInstalled>() {
        return;
    }
    app.insert_resource(FoldControlsInstalled);
    ensure_plugin(app, FoldPlugin);
    ensure_plugin(app, EnhancedInputPlugin);
    shortcuts::install(app);
    shortcuts::reserve_key::<FoldControlContext>(app, KeyCode::Space, FOLD_CONTROL_RESERVE_LABEL);
    shortcuts::reserve_key::<FoldControlContext>(app, KeyCode::KeyP, FOLD_PLAY_RESERVE_LABEL);
    screen_panels::register_title_control(app, TitleChip::new(FOLD_CONTROL_ID, FOLD_CONTROL_LABEL));
    screen_panels::register_title_control(
        app,
        TitleChip::new(UNFOLD_CONTROL_ID, UNFOLD_CONTROL_LABEL),
    );
    screen_panels::register_title_control(
        app,
        TitleChip::new(FOLD_PLAY_CONTROL_ID, FOLD_PLAY_CONTROL_LABEL),
    );
    app.init_resource::<FoldControlDiagnostics>()
        .add_input_context::<FoldControlContext>()
        .add_systems(Startup, spawn_fold_control_actions)
        .add_systems(
            PostUpdate,
            sync_fold_control_chips.before(screen_panels::refresh_changed_title_bar),
        )
        .add_observer(on_fold_step)
        .add_observer(on_unfold_step)
        .add_observer(on_play_fold);
}

fn spawn_fold_control_actions(mut commands: Commands) {
    commands.spawn((
        FoldControlContext,
        Actions::<FoldControlContext>::spawn(SpawnWith(
            |spawner: &mut ActionSpawner<FoldControlContext>| {
                let keybindings = Keybindings::new::<FoldShift>(spawner, ActionSettings::default());
                keybindings.spawn_key::<FoldStep>(spawner, KeyCode::Space);
                keybindings.spawn_shift_key::<UnfoldStep>(spawner, KeyCode::Space);
                keybindings.spawn_key::<PlayFold>(spawner, KeyCode::KeyP);
            },
        )),
    ));
}

fn on_fold_step(
    _: On<Start<FoldStep>>,
    sequences: Query<(Entity, Has<FairyDustFoldTarget>), With<FoldSequencePlayback>>,
    control: FoldControlState,
    diagnostics: ResMut<FoldControlDiagnostics>,
    fold_commands: FoldCommands,
    commands: Commands,
) {
    route_action(
        FoldControlAction::Fold,
        &sequences,
        &control,
        diagnostics,
        fold_commands,
        commands,
    );
}

fn on_unfold_step(
    _: On<Start<UnfoldStep>>,
    sequences: Query<(Entity, Has<FairyDustFoldTarget>), With<FoldSequencePlayback>>,
    control: FoldControlState,
    diagnostics: ResMut<FoldControlDiagnostics>,
    fold_commands: FoldCommands,
    commands: Commands,
) {
    route_action(
        FoldControlAction::Unfold,
        &sequences,
        &control,
        diagnostics,
        fold_commands,
        commands,
    );
}

fn on_play_fold(
    _: On<Start<PlayFold>>,
    sequences: Query<(Entity, Has<FairyDustFoldTarget>), With<FoldSequencePlayback>>,
    control: FoldControlState,
    diagnostics: ResMut<FoldControlDiagnostics>,
    fold_commands: FoldCommands,
    commands: Commands,
) {
    route_action(
        FoldControlAction::Play,
        &sequences,
        &control,
        diagnostics,
        fold_commands,
        commands,
    );
}

/// Retained control state for every sequence, readable beside [`FoldCommands`].
#[derive(SystemParam)]
struct FoldControlState<'w, 's> {
    intents:  Query<'w, 's, &'static FoldControlIntent>,
    progress: Query<'w, 's, &'static FoldControlProgress>,
}

impl FoldControlState<'_, '_> {
    fn intent(&self, sequence: Entity) -> FoldControlIntent {
        self.intents
            .get(sequence)
            .copied()
            .unwrap_or(FoldControlIntent {
                action: FoldControlAction::Fold,
                travel: FoldTravel::TowardFolded,
            })
    }

    fn progress(&self, sequence: Entity) -> FoldControlProgress {
        self.progress.get(sequence).copied().unwrap_or_default()
    }

    /// Resolves the shared command and retained travel one input issues.
    fn resolve(
        &self,
        action: FoldControlAction,
        sequence: Entity,
    ) -> (SequenceCommand, FoldTravel) {
        match action {
            FoldControlAction::Fold => (SequenceCommand::Step, FoldTravel::TowardFolded),
            FoldControlAction::Unfold => (SequenceCommand::StepBackward, FoldTravel::TowardBase),
            FoldControlAction::Play => {
                let travel = self.play_travel(sequence);
                match travel {
                    FoldTravel::TowardFolded => (SequenceCommand::Play, travel),
                    FoldTravel::TowardBase => (SequenceCommand::PlayBackward, travel),
                }
            },
        }
    }

    /// At a terminal, travel toward the other endpoint. In the interior,
    /// reverse an active play and otherwise follow the latest step direction,
    /// which continues an active step to its terminal.
    fn play_travel(&self, sequence: Entity) -> FoldTravel {
        let intent = self.intent(sequence);
        match (self.progress(sequence), intent.action) {
            (
                FoldControlProgress {
                    station: FoldStation::Folded,
                    ..
                },
                _,
            ) => FoldTravel::TowardBase,
            (
                FoldControlProgress {
                    station: FoldStation::Base,
                    ..
                },
                _,
            ) => FoldTravel::TowardFolded,
            (
                FoldControlProgress {
                    motion: FoldControlMotion::Moving,
                    ..
                },
                FoldControlAction::Play,
            ) => intent.travel.reversed(),
            _ => intent.travel,
        }
    }
}

fn route_action(
    action: FoldControlAction,
    sequences: &Query<(Entity, Has<FairyDustFoldTarget>), With<FoldSequencePlayback>>,
    control: &FoldControlState,
    mut diagnostics: ResMut<FoldControlDiagnostics>,
    mut fold_commands: FoldCommands,
    mut commands: Commands,
) {
    let reason = match select_ready_sequence(sequences) {
        ReadySelection::Selected(sequence) => {
            let (command, travel) = control.resolve(action, sequence);
            match fold_commands.apply(sequence, SequenceOwner::NativePlayback, command) {
                SequenceCommandResponse::Permitted(_) => {
                    commands
                        .entity(sequence)
                        .insert(FoldControlIntent { action, travel });
                    return;
                },
                SequenceCommandResponse::Rejected(owner) => {
                    FoldControlDiagnosticReason::SequenceHeld { owner }
                },
                // `select_ready_sequence` queries `With<FoldSequencePlayback>`,
                // so the selected sequence always carries retained playback.
                SequenceCommandResponse::NoRetainedSequence => {
                    FoldControlDiagnosticReason::NoReadySequence
                },
            }
        },
        ReadySelection::None => FoldControlDiagnosticReason::NoReadySequence,
        ReadySelection::Ambiguous {
            ready_sequences,
            marked_targets,
        } => FoldControlDiagnosticReason::AmbiguousReadySequences {
            ready_sequences,
            marked_targets,
        },
    };

    diagnostics.record(FoldControlDiagnostic { action, reason });
}

fn select_ready_sequence(
    sequences: &Query<(Entity, Has<FairyDustFoldTarget>), With<FoldSequencePlayback>>,
) -> ReadySelection {
    let mut ready_sequences = 0;
    let mut marked_targets = 0;
    let mut sole_ready = None;
    let mut sole_marked = None;

    for (entity, marked) in sequences.iter() {
        ready_sequences += 1;
        sole_ready = Some(entity);
        if marked {
            marked_targets += 1;
            sole_marked = Some(entity);
        }
    }

    match (ready_sequences, marked_targets) {
        (0, _) => ReadySelection::None,
        (1, _) => sole_ready.map_or(ReadySelection::None, ReadySelection::Selected),
        (_, 1) => sole_marked.map_or(ReadySelection::None, ReadySelection::Selected),
        _ => ReadySelection::Ambiguous {
            ready_sequences,
            marked_targets,
        },
    }
}

fn sync_fold_control_chips(
    mut commands: Commands,
    mut sequences: Query<(
        Entity,
        &FoldSequencePlayback,
        Has<FairyDustFoldTarget>,
        Option<&mut FoldControlProgress>,
    )>,
    intents: Query<&FoldControlIntent>,
    mut bars: Query<&mut TitleBarControlState>,
) {
    let mut ready_sequences = 0;
    let mut marked_targets = 0;
    let mut sole_ready = None;
    let mut sole_marked = None;

    for (entity, playback, marked, retained) in &mut sequences {
        let observed = FoldControlProgress::from(playback);
        match retained {
            Some(mut retained) => {
                retained.set_if_neq(observed);
            },
            None => {
                commands.entity(entity).insert(observed);
            },
        }
        ready_sequences += 1;
        sole_ready = Some((entity, observed));
        if marked {
            marked_targets += 1;
            sole_marked = Some((entity, observed));
        }
    }

    let selected = match (ready_sequences, marked_targets) {
        (0, _) => None,
        (1, _) => sole_ready,
        (_, 1) => sole_marked,
        _ => None,
    };
    let active = selected.and_then(|(entity, progress)| {
        matches!(progress.motion, FoldControlMotion::Moving)
            .then(|| intents.get(entity).ok().map(|intent| intent.action))
            .flatten()
    });
    let activation = |control: FoldControlAction| {
        if active == Some(control) {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    };

    for mut bar in &mut bars {
        bar.set_active(FOLD_CONTROL_ID, activation(FoldControlAction::Fold));
        bar.set_active(UNFOLD_CONTROL_ID, activation(FoldControlAction::Unfold));
        bar.set_active(FOLD_PLAY_CONTROL_ID, activation(FoldControlAction::Play));
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::time::Duration;

    use bevy::app::TaskPoolPlugin;
    use bevy::asset::AssetPlugin;
    use bevy::math::curve::EaseFunction;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Component;
    use bevy::prelude::Real;
    use bevy::prelude::Resource;
    use bevy::prelude::Startup;
    use bevy::prelude::Time;
    use bevy::prelude::Virtual;
    use bevy_enhanced_input::prelude::Action;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::InputAction;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use bevy_enhanced_input::prelude::Start;
    use bevy_enhanced_input::prelude::actions;
    use bevy_enhanced_input::prelude::bindings;
    use hana_valence::FoldSequenceBuilder;
    use hana_valence::FoldTiming;

    use super::*;
    use crate::cube_spin;
    use crate::screen_panels::TitleBarControlState;

    const STAGE: Duration = Duration::from_secs(1);

    #[derive(Component)]
    struct CubeMarker;

    #[derive(Component)]
    struct AlgorithmContext;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct ToggleAlgorithm;

    #[derive(Resource, Default)]
    struct AlgorithmToggles(usize);

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time>()
            .init_resource::<Time<Real>>()
            .insert_resource(Time::<Virtual>::default());
        install(&mut app);
        app.finish();
        app.update();
        app
    }

    /// Spawns one member entity per stage and a sequence that folds each in
    /// turn, one authored stage per second.
    fn spawn_sequence(app: &mut App, stages: usize) -> Entity {
        let members = (0..stages)
            .map(|_| app.world_mut().spawn_empty().id())
            .collect::<Vec<_>>();
        let sequence = FoldSequenceBuilder::new(FoldTiming::new(STAGE, EaseFunction::Linear))
            .stages(members)
            .build();
        let sequence = app.world_mut().spawn(sequence).id();
        app.update();
        sequence
    }

    /// Presses and releases one key while virtual time stands still.
    ///
    /// These apps run no `TimePlugin`, so `Time<Virtual>` keeps the delta of
    /// the last [`advance`] until something replaces it. Updating without
    /// replacing it would replay that delta on each of the two frames a press
    /// costs, travelling two whole stages inside what reads as one input.
    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        advance(app, Duration::ZERO);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
        advance(app, Duration::ZERO);
    }

    fn press_shift_space(app: &mut App, shift: KeyCode) {
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(shift);
            keys.press(KeyCode::Space);
        }
        advance(app, Duration::ZERO);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::Space);
            keys.release(shift);
        }
        advance(app, Duration::ZERO);
    }

    fn advance(app: &mut App, delta: Duration) {
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(delta);
        app.update();
    }

    fn position(app: &App, sequence: Entity) -> f64 {
        app.world()
            .get::<FoldSequencePlayback>(sequence)
            .map_or(f64::NAN, FoldSequencePlayback::normalized_position)
    }

    fn routed(app: &App, sequence: Entity) -> Option<FoldControlAction> {
        app.world()
            .get::<FoldControlIntent>(sequence)
            .map(|intent| intent.action)
    }

    fn chip_activation(app: &App, bar: Entity, control: &str) -> ControlActivation {
        app.world()
            .get::<TitleBarControlState>(bar)
            .map_or(ControlActivation::Inactive, |state| {
                state.activation(control)
            })
    }

    #[test]
    fn fold_control_installation_is_idempotent() {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time>()
            .init_resource::<Time<Real>>()
            .insert_resource(Time::<Virtual>::default());

        install(&mut app);
        install(&mut app);
        app.finish();
        app.update();

        assert!(app.is_plugin_added::<FoldPlugin>());
        assert!(app.is_plugin_added::<EnhancedInputPlugin>());
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<FoldControlContext>>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&Action<FoldStep>>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&Action<UnfoldStep>>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&Action<PlayFold>>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn bare_and_shift_space_bindings_route_separately_for_either_shift_key() {
        let mut app = test_app();
        let sequence = spawn_sequence(&mut app, 2);

        press(&mut app, KeyCode::Space);
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Fold));

        press_shift_space(&mut app, KeyCode::ShiftLeft);
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Unfold));

        press(&mut app, KeyCode::Space);
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Fold));

        press_shift_space(&mut app, KeyCode::ShiftRight);
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Unfold));
    }

    #[test]
    fn cube_spin_play_key_conflicts_with_fold_play() {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()));
        install(&mut app);

        let collision = std::panic::catch_unwind(AssertUnwindSafe(|| {
            cube_spin::install::<CubeMarker>(&mut app, cube_spin::CubeSpinConfig::default());
        }));

        assert!(collision.is_err());
    }

    #[test]
    fn passive_zero_ready_sequences_keeps_chips_inactive_without_diagnostics() {
        let mut app = test_app();
        let bar = app.world_mut().spawn(TitleBarControlState::default()).id();

        app.update();

        assert_eq!(
            chip_activation(&app, bar, FOLD_CONTROL_ID),
            ControlActivation::Inactive
        );
        assert_eq!(
            chip_activation(&app, bar, UNFOLD_CONTROL_ID),
            ControlActivation::Inactive
        );
        assert_eq!(
            chip_activation(&app, bar, FOLD_PLAY_CONTROL_ID),
            ControlActivation::Inactive
        );
        assert!(app.world().resource::<FoldControlDiagnostics>().is_empty());
    }

    #[test]
    fn input_with_zero_ready_sequences_records_one_stable_diagnostic() {
        let mut app = test_app();

        press(&mut app, KeyCode::KeyP);

        let diagnostics = app.world().resource::<FoldControlDiagnostics>();
        assert_eq!(
            diagnostics.entries().copied().collect::<Vec<_>>(),
            vec![FoldControlDiagnostic {
                action: FoldControlAction::Play,
                reason: FoldControlDiagnosticReason::NoReadySequence,
            }]
        );
    }

    #[test]
    fn a_sequence_without_retained_playback_is_never_selected() {
        let mut app = test_app();
        let unbuilt = app.world_mut().spawn(FairyDustFoldTarget).id();

        press(&mut app, KeyCode::Space);

        assert_eq!(routed(&app, unbuilt), None);
        assert_eq!(
            app.world()
                .resource::<FoldControlDiagnostics>()
                .entries()
                .next()
                .copied(),
            Some(FoldControlDiagnostic {
                action: FoldControlAction::Fold,
                reason: FoldControlDiagnosticReason::NoReadySequence,
            })
        );
    }

    #[test]
    fn sole_ready_sequence_routes_without_a_marker() {
        let mut app = test_app();
        let sequence = spawn_sequence(&mut app, 1);

        press(&mut app, KeyCode::Space);

        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Fold));
    }

    #[test]
    fn exactly_one_marked_ready_sequence_routes_among_multiple() {
        let mut app = test_app();
        let unmarked = spawn_sequence(&mut app, 1);
        let selected = spawn_sequence(&mut app, 1);
        app.world_mut()
            .entity_mut(selected)
            .insert(FairyDustFoldTarget);

        press(&mut app, KeyCode::KeyP);

        assert_eq!(routed(&app, selected), Some(FoldControlAction::Play));
        assert_eq!(routed(&app, unmarked), None);
    }

    #[test]
    fn multiple_unmarked_ready_sequences_are_ambiguous() {
        let mut app = test_app();
        spawn_sequence(&mut app, 1);
        spawn_sequence(&mut app, 1);

        press(&mut app, KeyCode::Space);

        assert_eq!(
            app.world()
                .resource::<FoldControlDiagnostics>()
                .entries()
                .copied()
                .collect::<Vec<_>>(),
            vec![FoldControlDiagnostic {
                action: FoldControlAction::Fold,
                reason: FoldControlDiagnosticReason::AmbiguousReadySequences {
                    ready_sequences: 2,
                    marked_targets:  0,
                },
            }]
        );
    }

    #[test]
    fn multiple_marked_ready_sequences_are_ambiguous() {
        let mut app = test_app();
        let first = spawn_sequence(&mut app, 1);
        let second = spawn_sequence(&mut app, 1);
        app.world_mut()
            .entity_mut(first)
            .insert(FairyDustFoldTarget);
        app.world_mut()
            .entity_mut(second)
            .insert(FairyDustFoldTarget);

        press(&mut app, KeyCode::KeyP);

        assert_eq!(routed(&app, first), None);
        assert_eq!(routed(&app, second), None);
        assert_eq!(
            app.world()
                .resource::<FoldControlDiagnostics>()
                .entries()
                .next()
                .copied(),
            Some(FoldControlDiagnostic {
                action: FoldControlAction::Play,
                reason: FoldControlDiagnosticReason::AmbiguousReadySequences {
                    ready_sequences: 2,
                    marked_targets:  2,
                },
            })
        );
    }

    #[test]
    fn step_chip_clears_on_the_exact_settle_frame() {
        let mut app = test_app();
        spawn_sequence(&mut app, 1);
        let bar = app.world_mut().spawn(TitleBarControlState::default()).id();

        // The first step consumes the records that share the start position and
        // settles without travelling, so the chip only lights on the step that
        // crosses the stage.
        press(&mut app, KeyCode::Space);
        press(&mut app, KeyCode::Space);
        advance(&mut app, STAGE / 2);
        assert_eq!(
            chip_activation(&app, bar, FOLD_CONTROL_ID),
            ControlActivation::Active
        );

        advance(&mut app, STAGE / 2);
        assert_eq!(
            chip_activation(&app, bar, FOLD_CONTROL_ID),
            ControlActivation::Inactive
        );
    }

    #[test]
    fn play_chip_stays_active_after_reversal_and_clears_on_the_exact_settle_frame() {
        let mut app = test_app();
        let sequence = spawn_sequence(&mut app, 2);
        let bar = app.world_mut().spawn(TitleBarControlState::default()).id();

        press(&mut app, KeyCode::KeyP);
        advance(&mut app, STAGE);
        assert_eq!(
            chip_activation(&app, bar, FOLD_PLAY_CONTROL_ID),
            ControlActivation::Active
        );

        press(&mut app, KeyCode::KeyP);
        advance(&mut app, Duration::ZERO);
        assert_eq!(
            chip_activation(&app, bar, FOLD_PLAY_CONTROL_ID),
            ControlActivation::Active
        );
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Play));

        advance(&mut app, STAGE);
        assert_eq!(
            chip_activation(&app, bar, FOLD_PLAY_CONTROL_ID),
            ControlActivation::Inactive
        );
        assert_eq!(
            app.world()
                .get::<FoldSequencePlayback>(sequence)
                .map(FoldSequencePlayback::normalized_position),
            Some(0.0)
        );
    }

    #[test]
    fn a_press_moves_no_sequence_of_its_own() {
        let mut app = test_app();
        let sequence = spawn_sequence(&mut app, 2);

        press(&mut app, KeyCode::KeyP);
        advance(&mut app, STAGE);
        let travelled = position(&app, sequence);
        press(&mut app, KeyCode::KeyP);

        assert!(
            travelled > 0.0 && travelled < 1.0,
            "the sequence must be mid-travel for this to prove anything: {travelled}",
        );
        assert_eq!(position(&app, sequence).to_bits(), travelled.to_bits());
    }

    #[test]
    fn fold_controls_coexist_with_an_example_owned_bei_action() {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time>()
            .init_resource::<Time<Real>>()
            .insert_resource(Time::<Virtual>::default())
            .init_resource::<AlgorithmToggles>();
        install(&mut app);
        app.add_input_context::<AlgorithmContext>()
            .add_systems(Startup, spawn_algorithm_action)
            .add_observer(on_toggle_algorithm);
        app.finish();
        app.update();
        let sequence = spawn_sequence(&mut app, 1);

        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyT);
            keys.press(KeyCode::Space);
        }
        app.update();

        assert_eq!(app.world().resource::<AlgorithmToggles>().0, 1);
        assert_eq!(routed(&app, sequence), Some(FoldControlAction::Fold));
    }

    fn spawn_algorithm_action(mut commands: Commands) {
        commands.spawn((
            AlgorithmContext,
            actions!(
                AlgorithmContext[(Action::<ToggleAlgorithm>::new(), bindings![KeyCode::KeyT],)]
            ),
        ));
    }

    fn on_toggle_algorithm(_: On<Start<ToggleAlgorithm>>, mut toggles: ResMut<AlgorithmToggles>) {
        toggles.0 += 1;
    }
}
