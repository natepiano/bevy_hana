//! Validates the builder's per-action descriptor lists and lowers them into
//! [`super::FreeCamBindings`].
//!
//! [`validate_free_cam_bindings`] is the single entry point called by
//! [`super::FreeCamBindingsBuilder::build`]. It delegates the held-binding and slow-mode invariants
//! and the descriptor lowering to the shared `crate::input` helpers, then wraps the
//! lowered sets in the free-camera per-action newtypes.

use super::FreeCamBindings;
use super::action_set::FreeCamHomeActionBindings;
use super::action_set::FreeCamLookActionBindings;
use super::action_set::FreeCamRollActionBindings;
use super::action_set::FreeCamTranslateActionBindings;
use super::preset::FreeCamLookPitch;
use crate::input;
use crate::input::ActionBindingDescriptor;
use crate::input::BindingsError;
use crate::input::CameraInputGamepadSelectionPolicy;
use crate::input::CameraSlowMode;
use crate::input::FREE_CAM_HOME_ACTION_NAME;
use crate::input::FREE_CAM_LOOK_ACTION_NAME;
use crate::input::FREE_CAM_ROLL_ACTION_NAME;
use crate::input::FREE_CAM_TRANSLATE_ACTION_NAME;
use crate::input::HeldBindingDescriptor;
use crate::input::ImpulseActionBindingSet;

/// Validates and builds `FreeCamBindings` from the builder's per-action descriptor lists.
///
/// # Errors
///
/// Returns [`BindingsError`] when any binding invariant fails.
pub(super) fn validate_free_cam_bindings(
    translate: &[HeldBindingDescriptor],
    look: &[HeldBindingDescriptor],
    roll: &[HeldBindingDescriptor],
    look_pitch: FreeCamLookPitch,
    slow_mode: Option<CameraSlowMode>,
    gamepad: CameraInputGamepadSelectionPolicy,
    home: &[ActionBindingDescriptor],
) -> Result<FreeCamBindings, BindingsError> {
    input::validate_held_entries(FREE_CAM_TRANSLATE_ACTION_NAME, translate)?;
    input::validate_held_entries(FREE_CAM_LOOK_ACTION_NAME, look)?;
    input::validate_held_entries(FREE_CAM_ROLL_ACTION_NAME, roll)?;
    input::validate_impulse_entries(FREE_CAM_HOME_ACTION_NAME, home)?;
    input::validate_slow_mode(slow_mode.as_ref())?;

    Ok(FreeCamBindings {
        translate: FreeCamTranslateActionBindings(input::held_descriptors_to_set(
            FREE_CAM_TRANSLATE_ACTION_NAME,
            translate,
        )?),
        look: FreeCamLookActionBindings(input::held_descriptors_to_set(
            FREE_CAM_LOOK_ACTION_NAME,
            look,
        )?),
        roll: FreeCamRollActionBindings(input::held_descriptors_to_set(
            FREE_CAM_ROLL_ACTION_NAME,
            roll,
        )?),
        home: FreeCamHomeActionBindings(ImpulseActionBindingSet::from_entries(
            home.iter().map(input::action_descriptor_to_entry).collect(),
        )),
        look_pitch,
        slow_mode,
        gamepad,
    })
}
