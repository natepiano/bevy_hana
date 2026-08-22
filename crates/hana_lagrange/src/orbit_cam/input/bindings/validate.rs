//! Validates a [`super::builder::OrbitCamBindingsDescriptor`] and converts it into a
//! [`super::OrbitCamBindings`].
//!
//! [`validate_bindings`] is called by [`super::OrbitCamBindingsBuilder::build`]. It delegates the
//! held/impulse/slow-mode invariants and the generic descriptor lowering to the shared
//! `crate::input` helpers, and keeps only the orbit-specific adapter-binding (trackpad,
//! mouse wheel, pinch, button-drag, touch) validation.

use super::OrbitCamBindings;
use super::action_set::OrbitCamHomeActionBindings;
use super::action_set::OrbitCamOrbitActionBindings;
use super::action_set::OrbitCamPanActionBindings;
use super::action_set::OrbitCamZoomCoarseActionBindings;
use super::action_set::OrbitCamZoomSmoothActionBindings;
use super::binding_kinds::OrbitCamBindingWithInputGain;
use super::binding_kinds::OrbitCamTouchBindingConfig;
use super::builder::OrbitCamBindingsDescriptor;
use crate::input;
use crate::input::BindingsError;
use crate::input::ImpulseActionBindingSet;
use crate::input::ORBIT_ACTION_NAME;
use crate::input::ORBIT_HOME_ACTION_NAME;
use crate::input::PAN_ACTION_NAME;
use crate::input::ZOOM_COARSE_ACTION_NAME;
use crate::input::ZOOM_SMOOTH_ACTION_NAME;

/// Validates and builds `OrbitCamBindings` from a descriptor.
///
/// # Errors
///
/// Returns [`BindingsError`] when any binding invariant fails.
pub(super) fn validate_bindings(
    descriptor: &OrbitCamBindingsDescriptor,
) -> Result<OrbitCamBindings, BindingsError> {
    input::validate_held_entries(ORBIT_ACTION_NAME, &descriptor.orbit)?;
    input::validate_held_entries(PAN_ACTION_NAME, &descriptor.pan)?;
    input::validate_held_entries(ZOOM_SMOOTH_ACTION_NAME, &descriptor.zoom_smooth)?;
    input::validate_impulse_entries(ZOOM_COARSE_ACTION_NAME, &descriptor.zoom_coarse)?;
    input::validate_impulse_entries(ORBIT_HOME_ACTION_NAME, &descriptor.home)?;
    validate_adapter_entries(descriptor)?;
    input::validate_slow_mode(descriptor.slow_mode.as_ref())?;

    Ok(OrbitCamBindings {
        orbit:            OrbitCamOrbitActionBindings(input::held_descriptors_to_set(
            ORBIT_ACTION_NAME,
            &descriptor.orbit,
        )?),
        pan:              OrbitCamPanActionBindings(input::held_descriptors_to_set(
            PAN_ACTION_NAME,
            &descriptor.pan,
        )?),
        zoom_smooth:      OrbitCamZoomSmoothActionBindings(input::held_descriptors_to_set(
            ZOOM_SMOOTH_ACTION_NAME,
            &descriptor.zoom_smooth,
        )?),
        zoom_coarse:      OrbitCamZoomCoarseActionBindings(ImpulseActionBindingSet::from_entries(
            descriptor
                .zoom_coarse
                .iter()
                .map(input::action_descriptor_to_entry)
                .collect(),
        )),
        home:             OrbitCamHomeActionBindings(ImpulseActionBindingSet::from_entries(
            descriptor
                .home
                .iter()
                .map(input::action_descriptor_to_entry)
                .collect(),
        )),
        trackpad_orbit:   descriptor.trackpad_orbit.clone(),
        trackpad_pan:     descriptor.trackpad_pan.clone(),
        trackpad_zoom:    descriptor.trackpad_zoom.clone(),
        mouse_wheel_zoom: descriptor.mouse_wheel_zoom,
        pinch_zoom:       descriptor.pinch_zoom,
        touch:            descriptor.touch,
        gamepad:          descriptor.gamepad,
        zoom_inversion:   descriptor.zoom_inversion,
        button_drag_zoom: descriptor.button_drag_zoom,
        slow_mode:        descriptor.slow_mode,
    })
}

fn validate_adapter_entries(descriptor: &OrbitCamBindingsDescriptor) -> Result<(), BindingsError> {
    validate_sensitive_entries(&descriptor.trackpad_orbit)?;
    validate_sensitive_entries(&descriptor.trackpad_pan)?;
    validate_sensitive_entries(&descriptor.trackpad_zoom)?;
    validate_sensitive_option(descriptor.mouse_wheel_zoom)?;
    validate_sensitive_option(descriptor.pinch_zoom)?;
    validate_sensitive_option(descriptor.button_drag_zoom)?;
    if let Some(touch) = descriptor.touch {
        validate_touch_config(touch)?;
    }
    Ok(())
}

fn validate_sensitive_entries<T>(
    entries: &[OrbitCamBindingWithInputGain<T>],
) -> Result<(), BindingsError> {
    for entry in entries {
        entry.input_gain().validate()?;
    }
    Ok(())
}

fn validate_sensitive_option<T>(
    entry: Option<OrbitCamBindingWithInputGain<T>>,
) -> Result<(), BindingsError> {
    if let Some(entry) = entry {
        entry.input_gain().validate()?;
    }
    Ok(())
}

fn validate_touch_config(touch: OrbitCamTouchBindingConfig) -> Result<(), BindingsError> {
    touch.input_gain().validate()
}

#[cfg(test)]
mod tests {
    use bevy::prelude::MouseButton;

    use super::*;
    use crate::input::InputGain;
    use crate::input::OrbitCamMouseDrag;

    #[test]
    fn validation_preserves_authored_disabled_entries_but_enabled_views_filter_them()
    -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Left).with_input_gain(InputGain::DISABLED.0))
            .build()?;

        assert_eq!(bindings.orbit().entries().len(), 1);
        assert_eq!(bindings.orbit().enabled_entries().count(), 0);
        let [entry] = bindings.orbit().entries() else {
            assert_eq!(bindings.orbit().entries().len(), 1);
            return Ok(());
        };
        let [motion] = entry.motion_descriptor().entries_slice() else {
            assert_eq!(entry.motion_descriptor().entries_slice().len(), 1);
            return Ok(());
        };
        assert_eq!(motion.input_gain(), InputGain::DISABLED);
        assert_eq!(entry.enabled_motion_entries().count(), 0);
        assert_eq!(entry.engagement_descriptor().enabled_entries().count(), 1);

        Ok(())
    }
}
