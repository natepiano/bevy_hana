//! Shared precision for continuous pointer motion, sampled before picking.

use bevy::input::InputSystems;
use bevy::picking::PickingSystems;
use bevy::prelude::*;

/// Precision applied to continuous drags this frame.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FineAdjustment {
    /// Pointer motion has its ordinary rate.
    #[default]
    Normal,
    /// Pointer motion is ten times finer.
    Fine,
}

impl FineAdjustment {
    /// The common rate for every fine drag.
    const FINE_RATE: f32 = 0.1;

    /// Multiplier for pointer travel; keyboard steps do not use this rate.
    #[must_use]
    pub const fn rate(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Fine => Self::FINE_RATE,
        }
    }
}

/// Samples fine adjustment after keyboard input and before pointer dispatch.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FineAdjustmentSystems;

/// Installs the shared fine-adjust input sample for widgets and scene drags.
pub struct FineAdjustmentPlugin;

impl Plugin for FineAdjustmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FineAdjustment>()
            .configure_sets(
                PreUpdate,
                FineAdjustmentSystems
                    .after(InputSystems)
                    .before(PickingSystems::ProcessInput),
            )
            .add_systems(
                PreUpdate,
                sample_fine_adjustment.in_set(FineAdjustmentSystems),
            );
    }
}

fn sample_fine_adjustment(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut adjustment: ResMut<FineAdjustment>,
) {
    let sampled =
        if keys.is_some_and(|keys| keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])) {
            FineAdjustment::Fine
        } else {
            FineAdjustment::Normal
        };
    adjustment.set_if_neq(sampled);
}
