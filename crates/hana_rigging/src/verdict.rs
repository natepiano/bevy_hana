//! Provider-reported serial evidence and identity verdicts produced during reconciliation.
//!
//! These types retain why a provider lacks evidence and record the kernel's resulting identity
//! decision, keeping raw reports separate from the policy that uses a reconciled device key.

use std::collections::HashSet;

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;

use super::identity::DeviceIdSource;
use super::identity::DeviceKey;
use super::scheme::ReportedId;

/// Serial evidence from a provider, retaining why a serial value is unavailable instead of using
/// `Option` and losing the policy-relevant distinction.
///
/// A serial-less display has a permanent hardware limitation, while a webcam scanned through a
/// platform API may have a serial that this platform cannot ask for. Providers retain that
/// difference so reconciliation can apply the appropriate recovery policy.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum ReportedSerial {
    /// The unit supplied this serial, such as the EDID serial from a display panel or the USB
    /// serial from an audio interface.
    ///
    /// `ReportedId` preserves the provider-defined text so it can participate in the registered
    /// identity scheme without the kernel assigning a different meaning to it.
    Provided(ReportedId),
    /// The unit itself exposes no serial, as with a serial-less webcam or panel whose firmware has
    /// no value to report.
    ///
    /// This limitation is permanent for the hardware and is not a provider failure that a retry
    /// can repair.
    NotExposedByUnit,
    /// The provider's platform API cannot ask the unit for a serial, such as a camera API that
    /// exposes capture devices but omits their USB serials.
    ///
    /// Another provider or operating system may be able to report a serial for the same physical
    /// unit, so this differs from `ReportedSerial::NotExposedByUnit`.
    PlatformCannotReport,
}

/// Reconciliation conclusion that states whether a live unit corresponds to its durable
/// `DeviceKey`.
///
/// `DeviceKey` is the durable identity, while `IdentityVerdict` records the kernel's conclusion
/// about one live unit. Computing the verdict during reconciliation prevents a raw provider claim
/// from contradicting the key. This enum is non-exhaustive because future reconciliation evidence
/// can require another conclusion; applications must retain a wildcard arm when matching it.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub enum IdentityVerdict {
    /// A `crate::DeviceIdSource::Reported` key matched one live unit uniquely, as when a display
    /// panel reports the EDID serial a saved layout was written against.
    ///
    /// `Proven` can mint an authorization for in-service use because a value reported by the unit
    /// establishes which physical device receives output.
    Proven,
    /// A `crate::DeviceIdSource::Synthesized` key matched one live unit uniquely.
    ///
    /// This restores a saved configuration, such as returning a window to its monitor, but never
    /// drives output. It is the usual result for webcams and serial-less panels whose location
    /// hints can change after a later scan.
    RestoreOnly,
    /// A human assigned this durable address because the unit reports no usable identity.
    ///
    /// The authored patch is authoritative by construction: for example, a lighting fixture with
    /// no serial uses its human-maintained patch as its identity. Like `Proven`, `Authored` can
    /// authorize in-service use after presence and claim checks succeed.
    Authored,
    /// A saved key matched no live unit even though a unit of the required kind occupies the same
    /// transport slot, as when a second camera is plugged into the USB port the saved camera used.
    ///
    /// The saved device is neither confirmed present nor known to be gone, so a human must decide
    /// whether the occupying unit replaces it before any automatic action can use it.
    Displaced {
        /// Durable key from saved configuration, retained instead of a process-local `DeviceId`
        /// so the verdict remains meaningful after the process that issued a handle exits.
        saved: DeviceKey,
    },
    /// A human-authored binding matched its saved key while the unit in that binding reports a
    /// different identity.
    ///
    /// This verdict never authorizes output: sending pan data to a misidentified fixture can send
    /// it to a dimmer instead.
    WrongUnit {
        /// Human-authored durable key that matched the saved binding and identifies the
        /// conflicting assignment without relying on a process-local `DeviceId`.
        authored: DeviceKey,
    },
    /// The unit cannot be identified because its identity is absent, duplicated in this scan, or
    /// unavailable through the platform API, as when two identical webcams report the same USB
    /// location value.
    ///
    /// `UnverifiedReason` retains which observation occurred because each one leads to different
    /// recovery policy. `Unverified` prohibits automatic action.
    Unverified(UnverifiedReason),
}

impl IdentityVerdict {
    /// Conclude one device's identity from its durable key and the keys this pass saw more than
    /// once, for the case where no human owes a decision about it.
    ///
    /// The merge reaches this conclusion on every pass that finds nothing outstanding, and
    /// `crate::Devices::discharge_identity_decision` reaches it the moment a human answers. They
    /// share this one predicate because the alternative — the discharge clearing the debt and
    /// leaving the merge to conclude the verdict on some later pass — is what kept an adopted unit
    /// refusing authorization until the next scan arrived.
    ///
    /// A key more than one live unit reported in a single scan is `Unverified` whatever its source
    /// claims: two identical webcams under one durable key cannot be told apart, and exact identity
    /// requires one unit rather than a plausible choice between two.
    pub(crate) fn concluded_from_scan(
        key: &DeviceKey,
        duplicate_keys: &HashSet<DeviceKey>,
    ) -> Self {
        if duplicate_keys.contains(key) {
            return Self::Unverified(UnverifiedReason::NotUniqueInScan);
        }

        match key.id {
            DeviceIdSource::Reported { .. } => Self::Proven,
            DeviceIdSource::Synthesized { .. } => Self::RestoreOnly,
            DeviceIdSource::Authored { .. } => Self::Authored,
        }
    }

    /// Report whether this verdict identifies the physical unit and nothing more.
    ///
    /// This is not the in-service predicate: `Devices::authorize_service` also requires presence
    /// and a claim before `Proven` or `Authored` identity can authorize output.
    /// `IdentityVerdict::RestoreOnly` is identified so saved configuration can return to the
    /// matching unit while it remains ineligible to drive output.
    #[must_use]
    pub(crate) const fn identified(&self) -> bool {
        matches!(self, Self::Proven | Self::RestoreOnly | Self::Authored)
    }
}

/// Whether a verdict only a human can resolve is outstanding for a device, independent of what the
/// current pass concluded about it.
///
/// `IdentityVerdict::Displaced` and `IdentityVerdict::WrongUnit` describe a join between an
/// arriving unit and the slot a saved one left, and that evidence exists only on the pass the unit
/// arrived. A later pass that reports a scan observation instead — a key duplicated in one scan
/// above all — would destroy the join if the outstanding verdict lived only in
/// `crate::ReconciledDeviceState::verdict`, and the unit a human never accepted would be authorized
/// as soon as the scan went back to reporting it once.
#[derive(Clone, Default, PartialEq, Eq, Debug, Reflect)]
pub enum IdentityDecisionOwed {
    /// Nothing is outstanding, so `crate::ReconciledDeviceState::verdict` is the whole conclusion.
    #[default]
    Nothing,
    /// This verdict stands until a human accepts or rejects the unit, and is reported again as
    /// soon as no scan observation of the current pass outranks it.
    HumanDecision(IdentityVerdict),
}

/// Evidence that prevents reconciliation from identifying a live unit, preserved so recovery
/// policy can distinguish hardware limits from platform limits and duplicate reports.
///
/// `UnverifiedReason::NotExposedByUnit` and `UnverifiedReason::PlatformCannotReport` name the same
/// two limits as the `ReportedSerial` variants of those names. `ReportedSerial` records what a
/// provider observed about one unit; these record that reconciliation could not identify the unit
/// as a result.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
pub enum UnverifiedReason {
    /// The unit itself exposes no serial, as with a serial-less display panel or webcam firmware
    /// that cannot provide a reported identifier on any platform.
    NotExposedByUnit,
    /// More than one live unit reported the same key in one complete provider scan.
    ///
    /// Reconciliation keeps the duplicate key out of service because exact identity requires one
    /// unit, not a plausible choice among two USB devices with indistinguishable reports.
    NotUniqueInScan,
    /// The provider's platform API cannot ask this unit for a serial, even though another platform
    /// may be able to obtain one from the same physical device, as when a macOS camera API reports
    /// a USB location value in place of the unit's serial.
    PlatformCannotReport,
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use bevy::reflect::PartialReflect;

    use super::IdentityVerdict;
    use super::UnverifiedReason;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::ReportedId;
    use crate::SchemeName;

    #[test]
    fn identified_is_true_for_confirmed_identity_verdicts() {
        for identity_verdict in [
            IdentityVerdict::Proven,
            IdentityVerdict::RestoreOnly,
            IdentityVerdict::Authored,
        ] {
            assert!(identity_verdict.identified());
        }
    }

    #[test]
    fn identified_is_false_for_verdicts_without_a_confirmed_identity() -> Result<(), Box<dyn Error>>
    {
        let saved = reported_display_key()?;
        let displaced = IdentityVerdict::Displaced {
            saved: saved.clone(),
        };
        let wrong_unit = IdentityVerdict::WrongUnit { authored: saved };
        let unverified = IdentityVerdict::Unverified(UnverifiedReason::NotUniqueInScan);

        assert!(!displaced.identified());
        assert!(!wrong_unit.identified());
        assert!(!unverified.identified());

        Ok(())
    }

    #[test]
    fn not_unique_in_scan_prevents_identification() {
        let unverified = IdentityVerdict::Unverified(UnverifiedReason::NotUniqueInScan);

        assert!(!unverified.identified());
    }

    #[test]
    fn identity_verdicts_compare_by_their_conclusions() {
        assert_eq!(IdentityVerdict::Proven, IdentityVerdict::Proven);
        assert_ne!(IdentityVerdict::Proven, IdentityVerdict::RestoreOnly);
    }

    #[test]
    fn identity_verdicts_compare_the_keys_they_carry() -> Result<(), Box<dyn Error>> {
        let saved = reported_display_key()?;
        let displaced = IdentityVerdict::Displaced {
            saved: saved.clone(),
        };

        assert_eq!(
            displaced,
            IdentityVerdict::Displaced {
                saved: saved.clone(),
            }
        );
        assert_ne!(displaced, IdentityVerdict::WrongUnit { authored: saved });

        Ok(())
    }

    #[test]
    fn reflected_comparison_answers_for_equal_and_unequal_verdicts() {
        assert_eq!(
            IdentityVerdict::Proven.reflect_partial_eq(&IdentityVerdict::Proven),
            Some(true)
        );
        assert_eq!(
            IdentityVerdict::Proven.reflect_partial_eq(&IdentityVerdict::RestoreOnly),
            Some(false)
        );
    }

    fn reported_display_key() -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new("edid-serial")?,
                value:  ReportedId::new("DELL-U2723QE-9J4K2H3")?,
            },
        })
    }
}
