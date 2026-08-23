//! Authored fold sequences, fold recipes, and retained fold playback.

mod apply;
mod constants;
mod error;
mod evaluate;
mod events;
mod group;
mod ledger;
mod playback;
mod recipe;
mod recipes;
mod sequence;
mod stage;
mod target;
mod timing;
mod winding;

pub(crate) use apply::apply_fold_recipe;
use bevy_app::App;
use bevy_app::Plugin;
use bevy_app::Update;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::SystemSet;
pub use error::FoldAuthorError;
pub use evaluate::EasedFoldFraction;
pub use evaluate::FoldEvaluationError;
pub use evaluate::evaluate_fold_angle;
pub use evaluate::fold_fraction;
pub use events::FoldEndpointReached;
pub use events::FoldEventTiming;
pub use events::FoldMemberBegin;
pub use events::FoldMemberEnd;
pub use events::FoldStageBegin;
pub use events::FoldStageEnd;
pub use group::FoldGroup;
pub use group::FoldGroups;
use hana_kana::SequencePlaybackPlugin;
use hana_kana::SequencePlaybackSystems;
pub use ledger::FoldBoundary;
pub use ledger::FoldBoundaryRecord;
pub use ledger::FoldEndpoint;
pub use ledger::FoldLedger;
pub use ledger::FoldMemberBoundary;
pub use ledger::FoldMemberSample;
pub use ledger::FoldMemberTrack;
pub use ledger::FoldSegment;
pub use ledger::FoldSegmentProgress;
pub use playback::FoldCommands;
pub use playback::FoldFractionScratch;
pub use playback::FoldMemberFraction;
pub use playback::FoldSequencePlayback;
pub use recipe::FoldAssignment;
pub use recipe::FoldRecipe;
pub use recipe::FoldRecipeCapability;
pub use recipe::NoCapability;
pub use recipe::ProviderCapability;
pub use recipes::Accordion;
pub use recipes::Coil;
pub use recipes::Wrap;
pub use sequence::FoldSequence;
pub use sequence::FoldSequenceBuilder;
pub use stage::FoldStage;
pub use target::FoldTarget;
pub use timing::FoldTiming;
pub use winding::WindingClearance;

use crate::ArrangementPlugin;

/// Installs retained fold playback on top of arrangement construction.
///
/// `FoldPlugin` composes [`ArrangementPlugin`] and
/// [`SequencePlaybackPlugin`], which brings shared driver arbitration and the
/// shared easing service with it. Both compositions are idempotent, so an
/// application may add either plugin itself first.
///
/// The plugin does not install anchor geometry providers, anchor resolution, or
/// transform propagation. Consumers continue to own those systems, and
/// [`ArrangementPlugin`] remains the sole registrar of
/// [`hinge_to_pose`](crate::hinge_to_pose).
///
/// Adding [`ArrangementPlugin`] without `FoldPlugin` leaves a
/// [`FoldSequence`] component present and inert: nothing rebuilds playback for
/// it, so every hinge rests at its base endpoint. That is the intended split,
/// not a defect.
pub struct FoldPlugin;

impl Plugin for FoldPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ArrangementPlugin>() {
            app.add_plugins(ArrangementPlugin);
        }
        app.add_plugins(SequencePlaybackPlugin)
            .configure_sets(
                Update,
                FoldSystems::Advance.in_set(SequencePlaybackSystems::EvaluateSequences),
            )
            .add_systems(
                Update,
                (
                    playback::rebuild_fold_playback
                        .before(SequencePlaybackSystems::ArbitrateDrivers),
                    playback::apply_fold_movement.in_set(SequencePlaybackSystems::ApplyMovement),
                    playback::evaluate_fold_sequences.in_set(FoldSystems::Advance),
                ),
            );
    }
}

/// Ordered system sets for fold playback.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum FoldSystems {
    /// Evaluates every retained fold sequence from its current local position.
    ///
    /// This set runs in `Update` inside
    /// [`SequencePlaybackSystems::EvaluateSequences`], after the selected
    /// producer's movement reached local position. Hinge-to-pose conversion
    /// reads the fractions it caches later, in `PostUpdate`.
    Advance,
}
