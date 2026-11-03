//! On-disk window-state formats.
//!
//! # Versioning strategy
//!
//! Every versioned RON file has a `version` envelope. The envelope is parsed before individual
//! entries so malformed RON and malformed envelopes remain whole-file failures, while a valid
//! envelope can discard one semantically invalid entry without discarding its valid siblings.
//! Only the current version and the one before it are read. Any other version, and a file with no
//! `version` at all, is rejected whole, the same as a file from a future build.
//!
//! ## Adding a new version
//!
//! 1. Bump [`CURRENT_STATE_VERSION`].
//! 2. Freeze the previous entry and state types; add distinct types for the new live format.
//! 3. Add a named adjacent conversion from the immediately previous version.
//! 4. Add the version's decode arm.
//! 5. Update [`encode`] so normal saves write only the latest format.
//! 6. Remove the decoder for the version two back, so only the new and previous formats load.
//! 7. Add direct tests for the new format and for the previous version's conversion.
//!
//! ## Supported formats
//!
//! | Format | Description |
//! |--------|-------------|
//! | v5 | Reporter-classified display keys or retained v4 evidence awaiting a live match |
//! | v6 | v5 window state plus the complete per-role binding policy |

use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::time::Duration;

use bevy::prelude::warn;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RegisteredSchemes;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
use hana_rigging::prelude::UnregisteredSchemeError;
use ron::Error;
use ron::Options;
use ron::extensions::Extensions;
use ron::from_str;
use ron::ser::PrettyConfig;
use ron::value::RawValue;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de::DeserializeOwned;
use thiserror::Error as ThisError;

use super::constants::PERSISTED_STATE_VERSION_V5;
use super::window_state::LoadedBindingPolicy;
use super::window_state::PersistedDisplayIdentityV4;
use super::window_state::PersistedWindowPlacement;
use super::window_state::PersistedWindowState;
use super::window_state::PersistedWindowTargetV5;
use crate::constants::CURRENT_STATE_VERSION;
use crate::constants::PRIMARY_WINDOW_KEY;
use crate::constants::RON_HEADER;
use crate::monitors::LiveDisplayEndpointLookup;

/// Wire-only discriminator that keeps the primary window distinct from a managed window named
/// `primary`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum PersistedWindowRole {
    /// The application's primary window.
    Primary,
    /// A secondary managed window identified by its stable name.
    Managed(String),
}

#[derive(Debug)]
pub(super) struct UnsupportedWindowRole;

impl TryFrom<&RoleKey> for PersistedWindowRole {
    type Error = UnsupportedWindowRole;

    fn try_from(role: &RoleKey) -> Result<Self, Self::Error> {
        if role.as_str() == super::PRIMARY_WINDOW_ROLE {
            return Ok(Self::Primary);
        }
        role.as_str()
            .strip_prefix(super::MANAGED_WINDOW_ROLE_PREFIX)
            .map(|name| Self::Managed(name.to_string()))
            .ok_or(UnsupportedWindowRole)
    }
}

impl TryFrom<PersistedWindowRole> for RoleKey {
    type Error = RoleKeyError;

    fn try_from(persisted_role: PersistedWindowRole) -> Result<Self, Self::Error> {
        match persisted_role {
            PersistedWindowRole::Primary => super::primary_window_role(),
            PersistedWindowRole::Managed(name) => super::managed_window_role(&name),
        }
    }
}

impl Display for PersistedWindowRole {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary => write!(formatter, "{PRIMARY_WINDOW_KEY}"),
            Self::Managed(name) => write!(formatter, "{name}"),
        }
    }
}

/// Whole-file result after the version envelope and all independently decodable entries run.
#[derive(Debug)]
pub(crate) enum PersistedWindowStateDecodeOutcome {
    /// The envelope was structurally valid; semantically rejected entries were logged and omitted.
    Decoded(HashMap<RoleKey, PersistedWindowPlacement>),
    /// RON syntax or the enclosing version envelope was invalid, so the loader must preserve it.
    WholeFileRejected,
}

/// Versioned persistence form of the complete policy authored for one window role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedBindingPolicy {
    recovery:        PersistedRecoveryPolicy,
    retry:           PersistedRetryOn,
    on_abort:        PersistedOnAbort,
    on_session_loss: PersistedOnSessionLoss,
    apply_deadline:  PersistedApplyDeadline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedRecoveryPolicy {
    Forget,
    ReapplyOnRequest,
    ReapplyOnReturn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedRetryOn {
    NewRevision,
    Interval(Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedOnAbort {
    LeaveAsIs,
    Revert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedOnSessionLoss {
    Recreate,
    ReportOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PersistedApplyDeadline {
    ProcessDefault,
    Authored(Duration),
}

impl From<BindingPolicy> for PersistedBindingPolicy {
    fn from(binding_policy: BindingPolicy) -> Self {
        Self {
            recovery:        match binding_policy.recovery() {
                RecoveryPolicy::Forget => PersistedRecoveryPolicy::Forget,
                RecoveryPolicy::ReapplyOnRequest => PersistedRecoveryPolicy::ReapplyOnRequest,
                RecoveryPolicy::ReapplyOnReturn => PersistedRecoveryPolicy::ReapplyOnReturn,
            },
            retry:           match binding_policy.retry() {
                RetryOn::NewRevision => PersistedRetryOn::NewRevision,
                RetryOn::Interval(interval) => PersistedRetryOn::Interval(interval),
            },
            on_abort:        match binding_policy.on_abort() {
                OnAbort::LeaveAsIs => PersistedOnAbort::LeaveAsIs,
                OnAbort::Revert => PersistedOnAbort::Revert,
            },
            on_session_loss: match binding_policy.on_session_loss() {
                OnSessionLoss::Recreate => PersistedOnSessionLoss::Recreate,
                OnSessionLoss::ReportOnly => PersistedOnSessionLoss::ReportOnly,
            },
            apply_deadline:  match binding_policy.apply_deadline() {
                ApplyDeadline::ProcessDefault => PersistedApplyDeadline::ProcessDefault,
                ApplyDeadline::Authored(apply_deadline) => {
                    PersistedApplyDeadline::Authored(apply_deadline)
                },
            },
        }
    }
}

impl From<PersistedBindingPolicy> for BindingPolicy {
    fn from(persisted: PersistedBindingPolicy) -> Self {
        Self::new(
            match persisted.recovery {
                PersistedRecoveryPolicy::Forget => RecoveryPolicy::Forget,
                PersistedRecoveryPolicy::ReapplyOnRequest => RecoveryPolicy::ReapplyOnRequest,
                PersistedRecoveryPolicy::ReapplyOnReturn => RecoveryPolicy::ReapplyOnReturn,
            },
            match persisted.retry {
                PersistedRetryOn::NewRevision => RetryOn::NewRevision,
                PersistedRetryOn::Interval(interval) => RetryOn::Interval(interval),
            },
            match persisted.on_abort {
                PersistedOnAbort::LeaveAsIs => OnAbort::LeaveAsIs,
                PersistedOnAbort::Revert => OnAbort::Revert,
            },
            match persisted.on_session_loss {
                PersistedOnSessionLoss::Recreate => OnSessionLoss::Recreate,
                PersistedOnSessionLoss::ReportOnly => OnSessionLoss::ReportOnly,
            },
            match persisted.apply_deadline {
                PersistedApplyDeadline::ProcessDefault => ApplyDeadline::ProcessDefault,
                PersistedApplyDeadline::Authored(apply_deadline) => {
                    ApplyDeadline::Authored(apply_deadline)
                },
            },
        )
    }
}

/// Current v6 entry written on every normal save.
///
/// `policy` is absent for a role still at `LoadedBindingPolicy::LegacyWindowMarkers`. That policy
/// is read from the window's recovery markers when the window spawns, so a save that runs while
/// the role has no window writes the entry without it, and the next decode loads the role as
/// `LegacyWindowMarkers` again.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntryV6 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowState,
    #[serde(
        rename = "policy",
        default,
        skip_serializing_if = "PersistedEntryPolicy::is_legacy_window_markers"
    )]
    binding_policy: PersistedEntryPolicy,
}

/// `policy` field of one v6 entry, written as a bare `PersistedBindingPolicy` when present.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PersistedEntryPolicy {
    Saved(PersistedBindingPolicy),
    /// The entry has no `policy` field.
    #[default]
    LegacyWindowMarkers,
}

impl PersistedEntryPolicy {
    const fn is_legacy_window_markers(&self) -> bool { matches!(self, Self::LegacyWindowMarkers) }
}

impl Serialize for PersistedEntryPolicy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Saved(binding_policy) => binding_policy.serialize(serializer),
            Self::LegacyWindowMarkers => serializer.serialize_unit(),
        }
    }
}

impl<'de> Deserialize<'de> for PersistedEntryPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PersistedBindingPolicy::deserialize(deserializer).map(Self::Saved)
    }
}

impl From<PersistedEntryPolicy> for LoadedBindingPolicy {
    fn from(entry_policy: PersistedEntryPolicy) -> Self {
        match entry_policy {
            PersistedEntryPolicy::Saved(binding_policy) => Self::Saved(binding_policy.into()),
            PersistedEntryPolicy::LegacyWindowMarkers => Self::LegacyWindowMarkers,
        }
    }
}

impl From<LoadedBindingPolicy> for PersistedEntryPolicy {
    fn from(loaded_binding_policy: LoadedBindingPolicy) -> Self {
        match loaded_binding_policy {
            LoadedBindingPolicy::Saved(binding_policy) => Self::Saved(binding_policy.into()),
            LoadedBindingPolicy::LegacyWindowMarkers => Self::LegacyWindowMarkers,
        }
    }
}

/// Result of translating a v4 target into v5's reporter-key authority.
#[derive(Debug)]
pub(crate) enum PersistedWindowIdentityMigrationOutcome {
    /// Fresh reporter evidence identified one exact kernel display key.
    Resolved(DeviceKey),
    /// No unique compatible live report exists yet, so preserve the v4 evidence for a later scan.
    AwaitingLiveEvidence,
    /// Fresh evidence named an invalid persistence key and this entry cannot be retained safely.
    Rejected(PersistedWindowIdentityMigrationFailure),
}

/// Why fresh evidence cannot become a v5 classified display target.
#[derive(Debug, ThisError)]
pub(crate) enum PersistedWindowIdentityMigrationFailure {
    /// A reporter association named a non-display device for a persisted window target.
    #[error("fresh monitor association named non-monitor device kind {found:?}")]
    NonDisplayKey {
        /// Device kind that cannot anchor a window position.
        found: DeviceKind,
    },
    /// A reported key's scheme was not registered by the running application.
    #[error("fresh monitor association named an unregistered reported scheme: {error}")]
    UnregisteredScheme {
        /// Kernel validation error naming the missing scheme.
        #[from]
        error: UnregisteredSchemeError,
    },
}

/// Frozen v5 entry retained only to decode already-written files.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntryV5 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowState,
}

/// Strict structural envelope shared by every version.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionedEnvelope {
    version: u8,
    entries: Vec<Box<RawValue>>,
}

/// Minimal version probe used to name the backup of a file that cannot be read.
#[derive(Deserialize)]
struct VersionProbe {
    version: u8,
}

/// Reads a `PersistedWindowRole` from a rejected `RawValue` so `decode_entries` can name its
/// window.
#[derive(Deserialize)]
struct RejectedEntryRoleProbe {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
}

#[derive(Debug, PartialEq, Eq)]
enum RejectedEntryRoleEvidence {
    Known(PersistedWindowRole),
    Unavailable,
}

/// Decode a v5 or v6 file into live state without rewriting it.
pub(super) fn decode(
    contents: &str,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    let Ok(envelope) = from_str::<VersionedEnvelope>(contents) else {
        return PersistedWindowStateDecodeOutcome::WholeFileRejected;
    };

    match envelope.version {
        PERSISTED_STATE_VERSION_V5 => decode_v5(envelope.entries, registered_schemes),
        CURRENT_STATE_VERSION => decode_v6(envelope.entries, registered_schemes),
        unsupported => {
            warn!(
                "[decode] Unsupported persisted state version {unsupported} \
                 (supported: {PERSISTED_STATE_VERSION_V5} and {CURRENT_STATE_VERSION})"
            );
            PersistedWindowStateDecodeOutcome::WholeFileRejected
        },
    }
}

/// Read the version for a backup filename without accepting the rest of the file as valid.
pub(super) fn probe_version(contents: &str) -> Option<u8> {
    from_str::<VersionProbe>(contents)
        .ok()
        .map(|probe| probe.version)
}

fn decode_v5(
    entries: Vec<Box<RawValue>>,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_v5_entry::<PersistedEntryV5>(entry)?;
        validate_window_target(&entry.window_state.target, registered_schemes)
            .map_err(|error| error.to_string())?;
        Ok((entry.persisted_role, entry.window_state.into()))
    })
}

fn decode_v6(
    entries: Vec<Box<RawValue>>,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_v5_entry::<PersistedEntryV6>(entry)?;
        validate_window_target(&entry.window_state.target, registered_schemes)
            .map_err(|error| error.to_string())?;
        Ok((
            entry.persisted_role,
            PersistedWindowPlacement::new(entry.window_state, entry.binding_policy.into()),
        ))
    })
}

fn decode_v5_entry<T>(entry: &RawValue) -> Result<T, String>
where
    T: DeserializeOwned,
{
    persistence_ron_options()
        .from_str(entry.get_ron())
        .map_err(|error| error.to_string())
}

fn persistence_ron_options() -> Options {
    Options::default().with_default_extension(Extensions::UNWRAP_NEWTYPES)
}

fn recover_rejected_entry_role(entry: &RawValue) -> RejectedEntryRoleEvidence {
    persistence_ron_options()
        .from_str::<RejectedEntryRoleProbe>(entry.get_ron())
        .map_or(RejectedEntryRoleEvidence::Unavailable, |entry_role| {
            RejectedEntryRoleEvidence::Known(entry_role.persisted_role)
        })
}

fn decode_entries(
    entries: Vec<Box<RawValue>>,
    mut decode_entry: impl FnMut(
        &RawValue,
    ) -> Result<(PersistedWindowRole, PersistedWindowPlacement), String>,
) -> PersistedWindowStateDecodeOutcome {
    let mut states = HashMap::with_capacity(entries.len());
    for entry in entries {
        let (persisted_role, state) = match decode_entry(&entry) {
            Ok(decoded) => decoded,
            Err(error) => {
                match recover_rejected_entry_role(&entry) {
                    RejectedEntryRoleEvidence::Known(persisted_role) => {
                        warn!(
                            "[decode] Rejected persisted window entry for {persisted_role}; that \
                             window will open at its default position: {error}"
                        );
                    },
                    RejectedEntryRoleEvidence::Unavailable => {
                        warn!("[decode] Rejected one persisted window entry: {error}");
                    },
                }
                continue;
            },
        };
        let role = match RoleKey::try_from(persisted_role.clone()) {
            Ok(role) => role,
            Err(error) => {
                warn!("[decode] Rejected persisted role {persisted_role}: {error}");
                continue;
            },
        };
        if states.insert(role.clone(), state).is_some() {
            warn!("[decode] Rejected duplicate persisted role {role}");
            states.remove(&role);
        }
    }
    PersistedWindowStateDecodeOutcome::Decoded(states)
}

pub(super) fn resolve_legacy_target(
    legacy_identity: PersistedDisplayIdentityV4,
    live_displays: &LiveDisplayEndpointLookup,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowIdentityMigrationOutcome {
    let Ok(device_key) = live_displays.key_for_legacy_identity(legacy_identity.into()) else {
        return PersistedWindowIdentityMigrationOutcome::AwaitingLiveEvidence;
    };
    match validate_classified_key(&device_key, registered_schemes) {
        Ok(()) => PersistedWindowIdentityMigrationOutcome::Resolved(device_key),
        Err(error) => PersistedWindowIdentityMigrationOutcome::Rejected(error),
    }
}

fn validate_window_target(
    target: &PersistedWindowTargetV5,
    registered_schemes: &RegisteredSchemes,
) -> Result<(), PersistedWindowIdentityMigrationFailure> {
    match target {
        PersistedWindowTargetV5::Classified(device_key) => {
            validate_classified_key(device_key, registered_schemes)
        },
        PersistedWindowTargetV5::AwaitingLegacyEvidence(_) => Ok(()),
    }
}

fn validate_classified_key(
    device_key: &DeviceKey,
    registered_schemes: &RegisteredSchemes,
) -> Result<(), PersistedWindowIdentityMigrationFailure> {
    if device_key.kind != DeviceKind::Display {
        return Err(PersistedWindowIdentityMigrationFailure::NonDisplayKey {
            found: device_key.kind,
        });
    }
    registered_schemes
        .validate(device_key)
        .map_err(PersistedWindowIdentityMigrationFailure::from)
}

/// Encode live v6 state for a normal save.
pub(super) fn encode(
    states: &HashMap<PersistedWindowRole, PersistedWindowPlacement>,
) -> Result<String, Error> {
    let mut entries: Vec<PersistedEntryV6> = states
        .iter()
        .map(|(persisted_role, placement)| PersistedEntryV6 {
            persisted_role: persisted_role.clone(),
            window_state:   placement.window_state.clone(),
            binding_policy: placement.loaded_binding_policy.into(),
        })
        .collect();
    entries.sort_by(|left, right| left.persisted_role.cmp(&right.persisted_role));
    let envelope = PersistedStateV6 {
        version: CURRENT_STATE_VERSION,
        entries,
    };
    let ron_body =
        persistence_ron_options().to_string_pretty(&envelope, PrettyConfig::default())?;
    Ok(format!("{RON_HEADER}{ron_body}"))
}

/// v6 envelope written by [`encode`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedStateV6 {
    version: u8,
    entries: Vec<PersistedEntryV6>,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedStateV5 {
    version: u8,
    entries: Vec<PersistedEntryV5>,
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {

    #[test]
    fn window_role_keys_round_trip_through_the_persisted_discriminator() -> Result<(), String> {
        let primary = super::super::primary_window_role()
            .map_err(|error| format!("failed to create the primary role: {error}"))?;
        let managed = super::super::managed_window_role("inspector")
            .map_err(|error| format!("failed to create a managed role: {error}"))?;

        assert_eq!(primary.as_str(), "window:primary");
        assert_eq!(managed.as_str(), "window:managed:inspector");
        assert_eq!(
            PersistedWindowRole::try_from(&primary).ok(),
            Some(PersistedWindowRole::Primary)
        );
        assert_eq!(
            PersistedWindowRole::try_from(&managed).ok(),
            Some(PersistedWindowRole::Managed(String::from("inspector")))
        );
        assert_eq!(
            RoleKey::try_from(PersistedWindowRole::Primary).ok(),
            Some(primary)
        );
        assert_eq!(
            RoleKey::try_from(PersistedWindowRole::Managed(String::from("inspector"))).ok(),
            Some(managed)
        );
        Ok(())
    }
    use std::collections::HashMap;

    use bevy::prelude::IVec2;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::Digest;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::SchemeName;
    use ron::ser::PrettyConfig;
    use ron::ser::to_string_pretty;

    use super::*;
    use crate::persistence::window_state::PersistedDisplayFingerprintV4;
    use crate::persistence::window_state::PersistedPosition;
    use crate::persistence::window_state::SavedWindowMode;

    fn schemes() -> RegisteredSchemes {
        let mut schemes = RegisteredSchemes::default();
        let scheme = SchemeName::new("edid-serial")
            .unwrap_or_else(|error| panic!("test scheme rejected: {error}"));
        schemes.register(scheme);
        schemes
    }

    fn decode(contents: &str) -> PersistedWindowStateDecodeOutcome {
        super::decode(contents, &schemes())
    }

    fn decoded_states(contents: &str) -> HashMap<RoleKey, PersistedWindowState> {
        match decode(contents) {
            PersistedWindowStateDecodeOutcome::Decoded(states) => states
                .into_iter()
                .map(|(role, placement)| (role, placement.window_state))
                .collect(),
            PersistedWindowStateDecodeOutcome::WholeFileRejected => {
                panic!("expected a structurally valid persisted state")
            },
        }
    }

    fn awaiting_state() -> PersistedWindowState {
        PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::MonitorOffset(IVec2::new(10, 20)),
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("test-app"),
        }
    }

    fn test_binding_policy() -> BindingPolicy {
        BindingPolicy::new(
            RecoveryPolicy::ReapplyOnReturn,
            RetryOn::Interval(Duration::from_secs(2)),
            OnAbort::Revert,
            OnSessionLoss::ReportOnly,
            ApplyDeadline::Authored(Duration::from_secs(7)),
        )
    }

    fn saved_placement(window_state: PersistedWindowState) -> PersistedWindowPlacement {
        PersistedWindowPlacement::new(
            window_state,
            LoadedBindingPolicy::Saved(test_binding_policy()),
        )
    }

    fn primary_role() -> RoleKey {
        super::super::primary_window_role()
            .unwrap_or_else(|error| panic!("test primary role rejected: {error}"))
    }

    #[test]
    fn a_rejected_entry_with_an_intact_key_reports_its_role() {
        let entry = from_str::<Box<RawValue>>(
            r#"(
                key: Managed("inspector"),
                state: "malformed",
            )"#,
        )
        .unwrap_or_else(|error| panic!("test entry did not parse as raw RON: {error}"));

        assert_eq!(
            recover_rejected_entry_role(&entry),
            RejectedEntryRoleEvidence::Known(PersistedWindowRole::Managed(String::from(
                "inspector"
            )))
        );
    }

    #[test]
    fn v5_envelope_rejects_unknown_top_level_fields() {
        let contents = "(
            version: 5,
            future_envelope: \"rejected by the v5 decoder\",
            entries: [],
        )";
        assert!(matches!(
            decode(contents),
            PersistedWindowStateDecodeOutcome::WholeFileRejected
        ));
    }

    #[test]
    fn v5_fingerprinted_target_without_fresh_evidence_stays_awaiting() {
        let contents = "(
            version: 5,
            entries: [(
                key: Primary,
                state: (
                    target: AwaitingLegacyEvidence(Fingerprinted(9)),
                    position: MonitorOffset((10, 20)),
                    logical_width: 800,
                    logical_height: 600,
                    mode: Windowed,
                ),
            )],
        )";
        let states = decoded_states(contents);
        assert_eq!(
            states[&primary_role()].target,
            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Fingerprinted(PersistedDisplayFingerprintV4(9)),
            )
        );
    }

    #[test]
    fn files_older_than_v5_are_rejected_whole() {
        let v1 = "(
            version: 1,
            entries: [(
                key: Primary,
                state: (position: Some((10, 20)), width: 800, height: 600, monitor_index: 0, mode: Windowed),
            )],
        )";
        let v4 = "(
            version: 4,
            entries: [(
                key: Primary,
                state: (
                    position: MonitorOffset((10, 20)),
                    logical_width: 800,
                    logical_height: 600,
                    monitor_panel: Fingerprinted((9)),
                    mode: Windowed,
                ),
            )],
        )";
        let unversioned_single_window =
            "(position: Some((10, 20)), width: 800, height: 600, monitor_index: 0, mode: Windowed)";

        for contents in [v1, v4, unversioned_single_window] {
            assert!(
                matches!(
                    decode(contents),
                    PersistedWindowStateDecodeOutcome::WholeFileRejected
                ),
                "expected a whole-file rejection for:\n{contents}"
            );
        }
    }

    #[test]
    fn v6_roundtrip_keeps_targets_and_the_complete_binding_policy() {
        let classified = PersistedWindowState {
            target: PersistedWindowTargetV5::Classified(DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Synthesized {
                    digest: Digest::new(17),
                },
            }),
            ..awaiting_state()
        };
        let states = HashMap::from([
            (
                PersistedWindowRole::Primary,
                saved_placement(classified.clone()),
            ),
            (
                PersistedWindowRole::Managed(String::from("offline")),
                saved_placement(awaiting_state()),
            ),
        ]);
        let encoded =
            encode(&states).unwrap_or_else(|error| panic!("v6 state did not serialize: {error}"));
        assert!(encoded.contains("version: 6"));
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) = decode(&encoded) else {
            panic!("encoded v6 state did not decode");
        };
        let primary = &decoded[&primary_role()];
        assert_eq!(primary.window_state, classified);
        assert_eq!(
            primary.loaded_binding_policy,
            LoadedBindingPolicy::Saved(test_binding_policy())
        );
        assert_eq!(decoded.len(), 2);
    }

    /// A role whose window has not spawned still holds `LoadedBindingPolicy::LegacyWindowMarkers`,
    /// and its entry is saved beside resolved roles instead of failing the whole encode.
    #[test]
    fn v6_roundtrip_keeps_an_unresolved_legacy_policy_beside_a_saved_one() {
        let unspawned = super::super::managed_window_role("unspawned")
            .unwrap_or_else(|error| panic!("test managed role rejected: {error}"));
        let states = HashMap::from([
            (
                PersistedWindowRole::Primary,
                saved_placement(awaiting_state()),
            ),
            (
                PersistedWindowRole::Managed(String::from("unspawned")),
                PersistedWindowPlacement::from(awaiting_state()),
            ),
        ]);
        let encoded = encode(&states).unwrap_or_else(|error| {
            panic!("v6 state with a legacy policy did not serialize: {error}")
        });
        let PersistedWindowStateDecodeOutcome::Decoded(decoded) = decode(&encoded) else {
            panic!("encoded v6 state did not decode:\n{encoded}");
        };
        assert_eq!(
            decoded[&primary_role()].loaded_binding_policy,
            LoadedBindingPolicy::Saved(test_binding_policy())
        );
        assert_eq!(
            decoded[&unspawned].loaded_binding_policy,
            LoadedBindingPolicy::LegacyWindowMarkers
        );
    }

    #[test]
    fn v5_roundtrip_accepts_a_registered_reported_display_key() {
        let scheme = SchemeName::new("edid-serial")
            .unwrap_or_else(|error| panic!("test scheme rejected: {error}"));
        let value = ReportedId::new("reported-display")
            .unwrap_or_else(|error| panic!("test reported ID rejected: {error}"));
        let state = PersistedWindowState {
            target: PersistedWindowTargetV5::Classified(DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Reported { scheme, value },
            }),
            ..awaiting_state()
        };
        let encoded = encode(&HashMap::from([(
            PersistedWindowRole::Primary,
            saved_placement(state.clone()),
        )]))
        .unwrap_or_else(|error| panic!("reported v6 state did not serialize: {error}"));
        let decoded = decoded_states(&encoded);
        assert_eq!(
            decoded.get(&primary_role()),
            Some(&state),
            "reported key was rejected from encoded v5 state:\n{encoded}"
        );
    }

    #[test]
    fn invalid_v5_targets_are_rejected_without_losing_valid_siblings() {
        let valid = PersistedEntryV5 {
            persisted_role: PersistedWindowRole::Primary,
            window_state:   awaiting_state(),
        };
        let non_display = PersistedEntryV5 {
            persisted_role: PersistedWindowRole::Managed(String::from("camera")),
            window_state:   PersistedWindowState {
                target: PersistedWindowTargetV5::Classified(DeviceKey {
                    kind: DeviceKind::Camera,
                    id:   DeviceIdSource::Synthesized {
                        digest: Digest::new(44),
                    },
                }),
                ..awaiting_state()
            },
        };
        let unregistered = PersistedEntryV5 {
            persisted_role: PersistedWindowRole::Managed(String::from("unknown-scheme")),
            window_state:   PersistedWindowState {
                target: PersistedWindowTargetV5::Classified(DeviceKey {
                    kind: DeviceKind::Display,
                    id:   DeviceIdSource::Reported {
                        scheme: SchemeName::new("unregistered")
                            .unwrap_or_else(|error| panic!("test scheme rejected: {error}")),
                        value:  ReportedId::new("display")
                            .unwrap_or_else(|error| panic!("test id rejected: {error}")),
                    },
                }),
                ..awaiting_state()
            },
        };
        let contents = to_string_pretty(
            &PersistedStateV5 {
                version: PERSISTED_STATE_VERSION_V5,
                entries: vec![valid, non_display, unregistered],
            },
            PrettyConfig::default(),
        )
        .unwrap_or_else(|error| panic!("test envelope did not serialize: {error}"));
        let states = decoded_states(&contents);
        assert_eq!(states.len(), 1);
        assert!(states.contains_key(&primary_role()));
    }

    #[test]
    fn malformed_entry_isolated_after_valid_envelope() {
        let contents = "(
            version: 5,
            entries: [
                (key: Primary, state: (target: AwaitingLegacyEvidence(Anonymous), position: Unpositioned, logical_width: 800, logical_height: 600, mode: Windowed)),
                (key: Managed(\"bad\"), state: (target: Classified((kind: Display, id: Synthesized(digest: (1)))), position: Unpositioned)),
            ],
        )";
        let states = decoded_states(contents);
        assert_eq!(states.len(), 1);
        assert!(states.contains_key(&primary_role()));
    }

    #[test]
    fn syntactically_invalid_ron_remains_a_whole_file_failure() {
        assert!(matches!(
            decode("(version: 5, entries: ["),
            PersistedWindowStateDecodeOutcome::WholeFileRejected
        ));
    }
}
