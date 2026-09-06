mod configuration;
mod edid;
mod native;

use bevy::prelude::Reflect;
pub(super) use configuration::MonitorConfiguration;
pub(super) use configuration::MonitorConfigurationState;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use edid::EdidIdentityEvidence;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReportedSerial;
#[cfg(any(test, feature = "test"))]
pub(super) use native::QualifiedEvidence;
pub(super) use native::monitor_evidence;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::constants::FNV_1A_OFFSET_BASIS;
use crate::constants::FNV_1A_PRIME;
/// Stable fingerprint of one physical display, derived only from the display's own identity data:
/// EDID bytes on Windows and X11, the `ColorSync` display UUID on macOS.
///
/// The same display produces the same fingerprint on every launch. That is what makes it safe to
/// write into a state file: a monitor-relative saved position is only meaningful if the monitor it
/// was measured from can still be recognised after displays are replugged, docked, or renumbered by
/// a driver update.
///
/// Two identical displays that report no serial number are indistinguishable to their operating
/// system and therefore share a fingerprint. That is a property of the evidence, not of this
/// type, and it is why a fingerprint match is treated as a strong hint rather than proof — the
/// saved position is still range-checked against the monitor it resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[type_path = "hana_clerestory::monitors"]
pub struct DisplayFingerprint(u64);

impl DisplayFingerprint {
    /// FNV-1a over the display's evidence bytes.
    ///
    /// Deliberately not `DefaultHasher`: its output is explicitly not guaranteed stable across
    /// Rust releases, so a toolchain bump would silently stop every saved fingerprint from
    /// matching and quietly turn identity-based restore back into index-based restore. FNV-1a is
    /// fixed by its specification, short enough to read, and needs no dependency.
    #[must_use]
    pub(crate) fn from_evidence_bytes(bytes: &[u8]) -> Self {
        let mut hash = FNV_1A_OFFSET_BASIS;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_1A_PRIME);
        }
        Self(hash)
    }

    /// Expose the fingerprint as the digest input `hana_rigging` hashes into a synthesized
    /// `DeviceKey`.
    ///
    /// The kernel's `Digest` is the same fixed-width FNV-1a value, so a display that reaches only
    /// tier 2 identity keeps one number across the reporter boundary instead of being re-hashed
    /// into a second value that no saved state file would match.
    #[must_use]
    pub(crate) const fn get(self) -> u64 { self.0 }

    pub(crate) const fn from_digest(digest: u64) -> Self { Self(digest) }
}

/// Whether a monitor's physical display can be recognised again in a later run.
///
/// Deliberately not `Option<DisplayFingerprint>`. The two states are not "a value" and "no value";
/// they are "this display identifies itself" and "this display cannot be told apart from any
/// other", and those compare differently: two anonymous displays are **not** the same display,
/// whereas `Option`'s derived equality says `None == None`. Writing the rule into the type means a
/// comparison cannot accidentally treat two unidentifiable displays as one and anchor a saved
/// position to whichever happened to enumerate first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[type_path = "hana_clerestory::monitors"]
pub enum DisplayIdentity {
    /// The display reported evidence unique to it. Stable across runs, reboots and replugs.
    Fingerprinted(DisplayFingerprint),
    /// No usable display evidence. Wayland withholds it, a virtual display may synthesize none,
    /// and two identical displays reporting no serial number are indistinguishable to the
    /// operating system. A position saved against such a monitor has no cross-restart target.
    #[default]
    Anonymous,
}

/// Kernel-facing identity evidence retained for one enumerated display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DisplayIdentityEvidence {
    /// Serial value the display published in its EDID.
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    ReportedSerial(ReportedId),
    /// Stable descriptor material that identifies only a restore target.
    Synthesized {
        /// Digest material retained from the display descriptor.
        display_fingerprint: DisplayFingerprint,
        /// Why this observation did not include a serial published by the display itself.
        serial:              ReportedSerial,
    },
    /// This scan produced no durable display identity.
    Unavailable {
        /// What the active platform can say about the missing serial.
        serial: ReportedSerial,
    },
}

impl DisplayIdentityEvidence {
    pub(crate) fn reported_serial(&self) -> ReportedSerial {
        match self {
            #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
            Self::ReportedSerial(reported_id) => ReportedSerial::Provided(reported_id.clone()),
            Self::Synthesized { serial, .. } | Self::Unavailable { serial } => serial.clone(),
        }
    }
}

/// Failure while collecting one display's platform evidence.
///
/// These failures describe reporter input only. They never allocate a Clerestory identity or
/// retain a second identity verdict beside `hana_rigging::IdentityVerdict`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(super) enum MonitorIdentificationError {
    /// A platform query needed for the reporter failed.
    #[error("operating-system monitor query failed: {0}")]
    OperatingSystemQuery(#[from] OperatingSystemQueryError),
    /// The platform returned identity data that failed its format or placeholder checks.
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    #[error(
        "stable physical monitor identity data is missing, incomplete, malformed, or a placeholder"
    )]
    InvalidStableIdentity,
    /// This platform cannot expose stable display evidence through the active API.
    #[error("platform cannot expose stable physical monitor identity")]
    StablePhysicalIdentityUnavailable,
    /// The display-configuration listener exhausted its monotonic generation.
    #[error("monitor configuration generation is exhausted")]
    ConfigurationGenerationExhausted,
}

/// Platform operation that failed while collecting display evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(super) enum OperatingSystemQueryError {
    /// The platform display-configuration query failed.
    #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
    #[error("display-configuration query failed")]
    DisplayConfiguration,
    /// Registration for display-configuration notifications failed.
    #[error("monitor configuration-notification registration failed")]
    ConfigurationNotificationRegistration,
    /// The registered display-configuration notification stream failed.
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    #[error("monitor configuration-notification stream failed")]
    ConfigurationNotificationStream,
    /// Removing a display-configuration notification registration failed.
    #[error("monitor configuration-notification removal failed")]
    ConfigurationNotificationRemoval,
    /// Windows could not enumerate a display's device interface.
    #[cfg(target_os = "windows")]
    #[error("monitor device-interface query failed")]
    MonitorDeviceInterface,
    /// The platform property used for durable display evidence could not be read.
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    #[error("stable monitor identity-property query failed")]
    StableIdentityProperty,
}

/// Convert one fresh platform observation into reporter evidence and the legacy persistence
/// adapter value written by state format v4.
///
/// The reporter evidence is the only input that can produce a `DeviceKey`. `DisplayIdentity`
/// remains here only so an existing v4 file can be decoded before its entries are converted to
/// the reporter's exact key.
pub(super) fn classify_display_evidence(
    evidence: &native::QualifiedEvidence,
) -> (DisplayIdentityEvidence, DisplayIdentity) {
    let fingerprint = DisplayFingerprint::from_evidence_bytes(evidence.stable_bytes());
    let legacy = DisplayIdentity::Fingerprinted(fingerprint);
    let reported = match evidence {
        #[cfg(any(test, target_os = "windows"))]
        native::QualifiedEvidence::WindowsEdid(EdidIdentityEvidence::ReportedSerial(serial)) => {
            DisplayIdentityEvidence::ReportedSerial(serial.clone())
        },
        #[cfg(any(test, target_os = "windows"))]
        native::QualifiedEvidence::WindowsEdid(EdidIdentityEvidence::Descriptor(_)) => {
            synthesized_without_serial(fingerprint)
        },
        #[cfg(any(test, all(unix, not(target_os = "macos"))))]
        native::QualifiedEvidence::X11Edid(EdidIdentityEvidence::ReportedSerial(serial)) => {
            DisplayIdentityEvidence::ReportedSerial(serial.clone())
        },
        #[cfg(any(test, all(unix, not(target_os = "macos"))))]
        native::QualifiedEvidence::X11Edid(EdidIdentityEvidence::Descriptor(_)) => {
            synthesized_without_serial(fingerprint)
        },
        #[cfg(any(test, target_os = "linux"))]
        native::QualifiedEvidence::InternalConnector(_) => DisplayIdentityEvidence::Synthesized {
            display_fingerprint: fingerprint,
            serial:              ReportedSerial::PlatformCannotReport,
        },
        #[cfg(target_os = "macos")]
        native::QualifiedEvidence::MacOsDisplayUuid(_) => DisplayIdentityEvidence::Synthesized {
            display_fingerprint: fingerprint,
            serial:              ReportedSerial::PlatformCannotReport,
        },
        #[cfg(any(test, feature = "test"))]
        QualifiedEvidence::Synthetic(_) => synthesized_without_serial(fingerprint),
    };
    (reported, legacy)
}

#[cfg(any(
    test,
    feature = "test",
    target_os = "windows",
    all(unix, not(target_os = "macos"))
))]
const fn synthesized_without_serial(fingerprint: DisplayFingerprint) -> DisplayIdentityEvidence {
    DisplayIdentityEvidence::Synthesized {
        display_fingerprint: fingerprint,
        serial:              ReportedSerial::NotExposedByUnit,
    }
}

pub(super) use native::attachment_path_when_unreported;
