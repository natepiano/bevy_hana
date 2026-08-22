//! Camera-agnostic descriptor validation and lowering shared by every camera kind's binding
//! validator.
//!
//! Functions:
//! - [`validate_held_entries`] / [`validate_impulse_entries`] — enforce the held/impulse invariants
//!   on a per-action slice of descriptor entries.
//! - [`validate_slow_mode`] — enforces the slow-mode scale-policy invariants.
//! - [`held_descriptors_to_set`] / [`held_descriptor_to_entry`] / [`action_descriptor_to_entry`] —
//!   lower validated descriptor entries into the generic [`HeldActionBindingSet`] /
//!   [`HeldActionBindingEntry`] / [`ImpulseActionBindingEntry`] storage.
//!
//! Each camera's `validate` module calls these to build its per-action binding sets, then wraps the
//! results in its own per-action newtypes.

use std::marker::PhantomData;

use super::action_set::BindingEngagement;
use super::action_set::HeldActionBindingEntry;
use super::action_set::HeldActionBindingSet;
use super::action_set::ImpulseActionBindingEntry;
use super::descriptor::ActionBindingDescriptor;
use super::descriptor::CameraInputScalePolicy;
use super::descriptor::CameraSlowMode;
use super::descriptor::HeldBindingDescriptor;
use super::descriptor::InputBindingDescriptor;
use super::descriptor::InputBindingEntry;
use super::error::BindingsError;
use super::held_binding::BindingGates;
use super::held_binding::GatePolarity;
use crate::input::CameraSemanticAction;
use crate::input::HeldCameraAction;
use crate::input::ImpulseCameraAction;

/// Validates the held-binding invariants for one action's descriptor entries.
///
/// # Errors
///
/// Returns [`BindingsError`] when any held entry lacks sources or engagement, mismatches its motion
/// and engagement sources, or carries an invalid modifier or gate.
pub fn validate_held_entries(
    action: &'static str,
    entries: &[HeldBindingDescriptor],
) -> Result<(), BindingsError> {
    for entry in entries {
        if entry.sources.is_empty() {
            return Err(BindingsError::MissingSources);
        }
        if entry.engagement.is_none() {
            return Err(BindingsError::HeldMotionMissingEngagement { action });
        }
        if entry.sources != entry.engagement_sources {
            return Err(BindingsError::HeldSourceMismatch { action });
        }
        if entry.motion.is_empty()
            || entry
                .engagement
                .as_ref()
                .is_some_and(InputBindingDescriptor::is_empty)
        {
            return Err(BindingsError::MissingSources);
        }
        validate_gates(action, &entry.gates)?;
        validate_descriptor_entries(&entry.motion)?;
        if let Some(engagement) = &entry.engagement {
            validate_descriptor_entries(engagement)?;
        }
    }
    Ok(())
}

/// Validates the impulse-binding invariants for one action's descriptor entries.
///
/// # Errors
///
/// Returns [`BindingsError`] when any impulse entry lacks sources, is empty, is configured with
/// held engagement, or carries an invalid modifier.
pub fn validate_impulse_entries(
    action: &'static str,
    entries: &[ActionBindingDescriptor],
) -> Result<(), BindingsError> {
    for entry in entries {
        if entry.sources.is_empty() || entry.binding.is_empty() {
            return Err(BindingsError::MissingSources);
        }
        if entry.engagement == BindingEngagement::Held {
            return Err(BindingsError::ImpulseEngagement { action });
        }
        validate_descriptor_entries(&entry.binding)?;
    }
    Ok(())
}

fn validate_gates(action: &'static str, gates: &BindingGates) -> Result<(), BindingsError> {
    for gate in gates.entries() {
        let opposite = match gate.polarity {
            GatePolarity::Required => GatePolarity::Blocked,
            GatePolarity::Blocked => GatePolarity::Required,
        };
        if gates
            .entries()
            .iter()
            .any(|candidate| candidate.input == gate.input && candidate.polarity == opposite)
        {
            return Err(BindingsError::ContradictoryGate { action });
        }
    }
    Ok(())
}

fn validate_descriptor_entries(descriptor: &InputBindingDescriptor) -> Result<(), BindingsError> {
    for entry in descriptor.entries_slice() {
        validate_entry(entry)?;
    }
    Ok(())
}

fn validate_entry(entry: &InputBindingEntry) -> Result<(), BindingsError> {
    entry.input_gain().validate()?;
    let modifiers = entry.modifiers();
    if modifiers.scale().is_some_and(|scale| !scale.is_finite()) {
        return Err(BindingsError::InvalidScale);
    }
    let Some(dead_zone) = modifiers.dead_zone() else {
        return Ok(());
    };
    let lower = dead_zone.lower_threshold;
    let upper = dead_zone.upper_threshold;
    if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || upper > 1.0 || lower >= upper {
        return Err(BindingsError::InvalidDeadZone);
    }
    Ok(())
}

/// Validates an optional slow-mode policy's scale invariants.
///
/// # Errors
///
/// Returns [`BindingsError::InvalidScale`] when the slow or normal scale is non-finite,
/// non-positive, or the slow scale exceeds the normal scale.
pub fn validate_slow_mode(slow_mode: Option<&CameraSlowMode>) -> Result<(), BindingsError> {
    let Some(slow_mode) = slow_mode else {
        return Ok(());
    };
    validate_scale_policy(slow_mode.scale)
}

fn validate_scale_policy(scale_policy: CameraInputScalePolicy) -> Result<(), BindingsError> {
    if !scale_policy.normal.is_finite()
        || !scale_policy.slow.is_finite()
        || scale_policy.normal <= 0.0
        || scale_policy.slow <= 0.0
        || scale_policy.slow > scale_policy.normal
    {
        return Err(BindingsError::InvalidScale);
    }
    Ok(())
}

/// Lowers a slice of validated held descriptors into a generic held action binding set.
///
/// # Errors
///
/// Returns [`BindingsError::HeldMotionMissingEngagement`] when a descriptor lacks its engagement.
pub fn held_descriptors_to_set<A: HeldCameraAction, E: CameraSemanticAction>(
    action: &'static str,
    descriptors: &[HeldBindingDescriptor],
) -> Result<HeldActionBindingSet<A, E>, BindingsError> {
    Ok(HeldActionBindingSet {
        entries: descriptors
            .iter()
            .map(|descriptor| held_descriptor_to_entry(action, descriptor))
            .collect::<Result<Vec<_>, _>>()?,
        action:  PhantomData,
    })
}

/// Lowers one validated held descriptor into a generic held action binding entry.
fn held_descriptor_to_entry<A: HeldCameraAction>(
    action: &'static str,
    descriptor: &HeldBindingDescriptor,
) -> Result<HeldActionBindingEntry<A>, BindingsError> {
    let engagement = descriptor
        .engagement
        .clone()
        .ok_or(BindingsError::HeldMotionMissingEngagement { action })?;

    Ok(HeldActionBindingEntry {
        motion: descriptor.motion.clone(),
        engagement,
        gates: descriptor.gates.clone(),
        sources: descriptor.sources,
        route: descriptor.route,
        speed: descriptor.speed,
        action: PhantomData,
    })
}

/// Lowers one validated impulse descriptor into a generic action binding entry.
pub fn action_descriptor_to_entry<A: ImpulseCameraAction>(
    descriptor: &ActionBindingDescriptor,
) -> ImpulseActionBindingEntry<A> {
    ImpulseActionBindingEntry {
        binding:    descriptor.binding.clone(),
        sources:    descriptor.sources,
        route:      descriptor.route,
        engagement: BindingEngagement::Impulse,
        action:     PhantomData,
    }
}
