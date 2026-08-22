//! On-disk window-state formats and their one-way conversions.
//!
//! # Versioning strategy
//!
//! Every versioned RON file has a `version` envelope. The envelope is parsed before individual
//! entries so malformed RON and malformed envelopes remain whole-file failures, while a valid
//! envelope can discard one semantically invalid entry without discarding its valid siblings.
//! Legacy bare records predate the envelope and retain their original whole-record decoder.
//!
//! ## Adding a new version
//!
//! 1. Bump [`CURRENT_STATE_VERSION`].
//! 2. Freeze the previous entry and state types; add distinct types for the new live format.
//! 3. Add a named adjacent conversion from the immediately previous version.
//! 4. Add the version's decode arm.
//! 5. Update [`encode`] so normal saves write only the latest format.
//! 6. Keep every older decoder and add direct tests for the new format and each conversion path.
//!
//! ## Supported formats
//!
//! | Format | Description |
//! |--------|-------------|
//! | Legacy single-window | Bare v1-style state before multi-window support |
//! | v1 | Physical field names and an enumeration index |
//! | v2 | Logical dimensions and a pre-v3 absolute coordinate |
//! | v3 | Monitor-relative positions plus the retired enumeration index |
//! | v4 | Private `monitor_panel` fingerprints without reporter key strength |
//! | v5 | Reporter-classified display keys or retained v4 evidence awaiting a live match |

use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use bevy::prelude::IVec2;
use bevy::prelude::debug;
use bevy::prelude::warn;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::RegisteredSchemes;
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
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error as ThisError;

use super::constants::PERSISTED_STATE_VERSION_V1;
use super::constants::PERSISTED_STATE_VERSION_V2;
use super::constants::PERSISTED_STATE_VERSION_V3;
use super::constants::PERSISTED_STATE_VERSION_V4;
use super::window_state::PersistedPanelIdentityV4;
use super::window_state::PersistedPosition;
use super::window_state::PersistedWindowState;
use super::window_state::PersistedWindowTargetV5;
use super::window_state::SavedFullscreenVideoMode;
use super::window_state::SavedVideoMode;
use super::window_state::SavedWindowMode;
use super::window_state::UnrebasedDesktopPosition;
use super::window_state::default_monitor_scale;
use crate::constants::CURRENT_STATE_VERSION;
use crate::constants::PRIMARY_WINDOW_KEY;
use crate::constants::RON_HEADER;
use crate::monitors::MonitorDeviceAssociation;
use crate::monitors::MonitorDeviceKeyLookup;

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
    Decoded(HashMap<RoleKey, PersistedWindowState>),
    /// RON syntax or the enclosing version envelope was invalid, so the loader must preserve it.
    WholeFileRejected,
}

/// Result of translating a v4 target into v5's reporter-key authority.
#[derive(Debug)]
pub(crate) enum PersistedWindowIdentityMigrationOutcome {
    /// Fresh reporter evidence identified one exact kernel display key.
    Resolved(DeviceKey),
    /// No unique compatible live report exists yet, so preserve the v4 evidence for a later scan.
    AwaitingLiveEvidence(PersistedPanelIdentityV4),
    /// Fresh evidence named an invalid persistence key and this entry cannot be retained safely.
    Rejected(PersistedWindowIdentityMigrationFailure),
}

/// Why fresh evidence cannot become a v5 classified display target.
#[derive(Debug, ThisError)]
pub(crate) enum PersistedWindowIdentityMigrationFailure {
    /// A reporter association named a non-display device for a persisted window target.
    #[error("fresh monitor association named non-display device kind {found:?}")]
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

/// Current v5 entry written on every normal save.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntryV5 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowState,
}

/// Frozen v4 entry retained only to decode already-written files.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntryV4 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowStateV4,
}

/// Frozen v4 state that never receives new fields or new semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWindowStateV4 {
    position:          PersistedPosition,
    logical_width:     u32,
    logical_height:    u32,
    #[serde(default, rename = "monitor_panel")]
    monitor_panel:     PersistedPanelIdentityV4,
    #[serde(rename = "mode")]
    saved_window_mode: SavedWindowModeV4,
    #[serde(default)]
    app_name:          String,
}

/// Frozen v4 fullscreen representation whose absent mode meant the current display mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum SavedWindowModeV4 {
    Windowed,
    BorderlessFullscreen,
    Fullscreen { video_mode: Option<SavedVideoMode> },
}

impl From<SavedWindowModeV4> for SavedWindowMode {
    fn from(window_mode: SavedWindowModeV4) -> Self {
        match window_mode {
            SavedWindowModeV4::Windowed => Self::Windowed,
            SavedWindowModeV4::BorderlessFullscreen => Self::BorderlessFullscreen,
            SavedWindowModeV4::Fullscreen { video_mode } => Self::Fullscreen {
                video_mode: video_mode.map_or(
                    SavedFullscreenVideoMode::Current,
                    SavedFullscreenVideoMode::Specific,
                ),
            },
        }
    }
}

/// v3 state whose enumeration index was deliberately discarded by v3-to-v4 migration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWindowStateV3 {
    position:          PersistedPosition,
    logical_width:     u32,
    logical_height:    u32,
    monitor_index:     usize,
    #[serde(default, rename = "monitor_panel")]
    monitor_panel:     PersistedPanelIdentityV4,
    #[serde(rename = "mode")]
    saved_window_mode: SavedWindowModeV4,
    #[serde(default)]
    app_name:          String,
}

/// v3 entry retained only to decode files from before index removal.
#[derive(Debug, Clone, Deserialize)]
struct PersistedEntryV3 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowStateV3,
}

/// v2 state with an absolute desktop coordinate and legacy monitor scale.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWindowStateV2 {
    logical_position:  Option<(i32, i32)>,
    logical_width:     u32,
    logical_height:    u32,
    #[serde(default = "default_monitor_scale", rename = "monitor_scale")]
    scale:             f64,
    #[serde(rename = "monitor_index")]
    monitor:           usize,
    #[serde(rename = "mode")]
    saved_window_mode: SavedWindowModeV4,
    #[serde(default)]
    app_name:          String,
}

/// v2 entry retained only to decode files from before monitor-relative coordinates.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEntryV2 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowStateV2,
}

/// v1 state with the historical width and height wire names.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWindowStateV1 {
    #[serde(rename = "position")]
    logical_position:  Option<(i32, i32)>,
    #[serde(rename = "width")]
    logical_width:     u32,
    #[serde(rename = "height")]
    logical_height:    u32,
    monitor_index:     usize,
    #[serde(rename = "mode")]
    saved_window_mode: SavedWindowModeV4,
    #[serde(default)]
    app_name:          String,
}

/// v1 entry retained only to decode the first multi-window format.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEntryV1 {
    #[serde(rename = "key")]
    persisted_role: PersistedWindowRole,
    #[serde(rename = "state")]
    window_state:   PersistedWindowStateV1,
}

/// Strict structural envelope for the current format and unsupported future formats.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionedEnvelope {
    version: u8,
    entries: Vec<Box<RawValue>>,
}

/// Frozen structural envelope retaining v1-v4's shipped unknown-field compatibility.
#[derive(Deserialize)]
struct FrozenVersionedEnvelope {
    version: u8,
    entries: Vec<Box<RawValue>>,
}

/// Minimal version probe used before selecting the frozen decoder.
#[derive(Deserialize)]
struct VersionProbe {
    version: u8,
}

/// Decode a file into live v5 state without rewriting it.
pub(super) fn decode(
    contents: &str,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    let Ok(version_probe) = from_str::<VersionProbe>(contents) else {
        return decode_legacy_single_window(contents, association, registered_schemes);
    };

    let envelope = match version_probe.version {
        PERSISTED_STATE_VERSION_V1
        | PERSISTED_STATE_VERSION_V2
        | PERSISTED_STATE_VERSION_V3
        | PERSISTED_STATE_VERSION_V4 => {
            from_str::<FrozenVersionedEnvelope>(contents).map(|frozen_envelope| VersionedEnvelope {
                version: frozen_envelope.version,
                entries: frozen_envelope.entries,
            })
        },
        _ => from_str::<VersionedEnvelope>(contents),
    };
    let Ok(envelope) = envelope else {
        return PersistedWindowStateDecodeOutcome::WholeFileRejected;
    };
    if envelope.version != version_probe.version {
        return PersistedWindowStateDecodeOutcome::WholeFileRejected;
    }

    match envelope.version {
        PERSISTED_STATE_VERSION_V1 => decode_v1(envelope.entries, association, registered_schemes),
        PERSISTED_STATE_VERSION_V2 => decode_v2(envelope.entries, association, registered_schemes),
        PERSISTED_STATE_VERSION_V3 => decode_v3(envelope.entries, association, registered_schemes),
        PERSISTED_STATE_VERSION_V4 => decode_v4(envelope.entries, association, registered_schemes),
        CURRENT_STATE_VERSION => decode_v5(envelope.entries, registered_schemes),
        unsupported => {
            warn!(
                "[decode] Unsupported persisted state version {unsupported} \
                 (latest supported: {CURRENT_STATE_VERSION})"
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

fn decode_legacy_single_window(
    contents: &str,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    let Ok(window_state_v1) = from_str::<PersistedWindowStateV1>(contents) else {
        return PersistedWindowStateDecodeOutcome::WholeFileRejected;
    };
    let window_state = match convert_v4_state_to_v5(
        convert_v1_state_to_v4(window_state_v1, &PersistedWindowRole::Primary),
        association,
        registered_schemes,
    ) {
        PersistedWindowStateConversionOutcome::Converted(window_state) => window_state,
        PersistedWindowStateConversionOutcome::Rejected(error) => {
            warn!("[decode] Rejected legacy single-window state: {error}");
            return PersistedWindowStateDecodeOutcome::Decoded(HashMap::new());
        },
    };
    PersistedWindowStateDecodeOutcome::Decoded(HashMap::from([(
        match RoleKey::try_from(PersistedWindowRole::Primary) {
            Ok(role) => role,
            Err(error) => {
                warn!("[decode] legacy primary role is invalid: {error}");
                return PersistedWindowStateDecodeOutcome::Decoded(HashMap::new());
            },
        },
        window_state,
    )]))
}

fn decode_v1(
    entries: Vec<Box<RawValue>>,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_frozen_entry::<PersistedEntryV1>(entry)?;
        let state = convert_v1_state_to_v4(entry.window_state, &entry.persisted_role);
        match convert_v4_state_to_v5(state, association, registered_schemes) {
            PersistedWindowStateConversionOutcome::Converted(state) => {
                Ok((entry.persisted_role, state))
            },
            PersistedWindowStateConversionOutcome::Rejected(error) => Err(error.to_string()),
        }
    })
}

fn decode_v2(
    entries: Vec<Box<RawValue>>,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_frozen_entry::<PersistedEntryV2>(entry)?;
        let state = convert_v2_state_to_v4(entry.window_state, &entry.persisted_role);
        match convert_v4_state_to_v5(state, association, registered_schemes) {
            PersistedWindowStateConversionOutcome::Converted(state) => {
                Ok((entry.persisted_role, state))
            },
            PersistedWindowStateConversionOutcome::Rejected(error) => Err(error.to_string()),
        }
    })
}

fn decode_v3(
    entries: Vec<Box<RawValue>>,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_frozen_entry::<PersistedEntryV3>(entry)?;
        let state = convert_v3_state_to_v4(entry.window_state);
        match convert_v4_state_to_v5(state, association, registered_schemes) {
            PersistedWindowStateConversionOutcome::Converted(state) => {
                Ok((entry.persisted_role, state))
            },
            PersistedWindowStateConversionOutcome::Rejected(error) => Err(error.to_string()),
        }
    })
}

fn decode_v4(
    entries: Vec<Box<RawValue>>,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_frozen_entry::<PersistedEntryV4>(entry)?;
        match convert_v4_state_to_v5(entry.window_state, association, registered_schemes) {
            PersistedWindowStateConversionOutcome::Converted(state) => {
                Ok((entry.persisted_role, state))
            },
            PersistedWindowStateConversionOutcome::Rejected(error) => Err(error.to_string()),
        }
    })
}

fn decode_v5(
    entries: Vec<Box<RawValue>>,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateDecodeOutcome {
    decode_entries(entries, |entry| {
        let entry = decode_v5_entry::<PersistedEntryV5>(entry)?;
        validate_window_target(&entry.window_state.target, registered_schemes)
            .map_err(|error| error.to_string())?;
        Ok((entry.persisted_role, entry.window_state))
    })
}

fn decode_frozen_entry<T>(entry: &RawValue) -> Result<T, String>
where
    T: DeserializeOwned,
{
    from_str(entry.get_ron()).map_err(|error| error.to_string())
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

fn decode_entries(
    entries: Vec<Box<RawValue>>,
    mut decode_entry: impl FnMut(
        &RawValue,
    ) -> Result<(PersistedWindowRole, PersistedWindowState), String>,
) -> PersistedWindowStateDecodeOutcome {
    let mut states = HashMap::with_capacity(entries.len());
    for entry in entries {
        let (persisted_role, state) = match decode_entry(&entry) {
            Ok(decoded) => decoded,
            Err(error) => {
                warn!("[decode] Rejected one persisted window entry: {error}");
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

fn convert_v1_state_to_v4(
    state: PersistedWindowStateV1,
    persisted_role: &PersistedWindowRole,
) -> PersistedWindowStateV4 {
    debug!(
        "[convert_v1_state_to_v4] Ignoring legacy monitor index {} for {persisted_role}",
        state.monitor_index
    );
    PersistedWindowStateV4 {
        position:          legacy_position(
            state.logical_position,
            default_monitor_scale(),
            persisted_role,
        ),
        logical_width:     state.logical_width,
        logical_height:    state.logical_height,
        monitor_panel:     PersistedPanelIdentityV4::Anonymous,
        saved_window_mode: state.saved_window_mode,
        app_name:          state.app_name,
    }
}

fn convert_v2_state_to_v4(
    state: PersistedWindowStateV2,
    persisted_role: &PersistedWindowRole,
) -> PersistedWindowStateV4 {
    debug!(
        "[convert_v2_state_to_v4] Ignoring legacy monitor index {} for {persisted_role}",
        state.monitor
    );
    PersistedWindowStateV4 {
        position:          legacy_position(state.logical_position, state.scale, persisted_role),
        logical_width:     state.logical_width,
        logical_height:    state.logical_height,
        monitor_panel:     PersistedPanelIdentityV4::Anonymous,
        saved_window_mode: state.saved_window_mode,
        app_name:          state.app_name,
    }
}

fn convert_v3_state_to_v4(state: PersistedWindowStateV3) -> PersistedWindowStateV4 {
    debug!(
        "[convert_v3_state_to_v4] Ignoring legacy monitor index {}",
        state.monitor_index
    );
    PersistedWindowStateV4 {
        position:          state.position,
        logical_width:     state.logical_width,
        logical_height:    state.logical_height,
        monitor_panel:     state.monitor_panel,
        saved_window_mode: state.saved_window_mode,
        app_name:          state.app_name,
    }
}

/// Result of turning a structurally valid legacy entry into live v5 state.
enum PersistedWindowStateConversionOutcome {
    /// The entry now has one v5 target and can remain in the state file.
    Converted(PersistedWindowState),
    /// The entry's fresh classified evidence contradicted persistence rules.
    Rejected(PersistedWindowIdentityMigrationFailure),
}

fn convert_v4_state_to_v5(
    state: PersistedWindowStateV4,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowStateConversionOutcome {
    let target = match resolve_legacy_target(state.monitor_panel, association, registered_schemes) {
        PersistedWindowIdentityMigrationOutcome::Resolved(device_key) => {
            PersistedWindowTargetV5::Classified(device_key)
        },
        PersistedWindowIdentityMigrationOutcome::AwaitingLiveEvidence(panel_identity) => {
            PersistedWindowTargetV5::AwaitingLegacyEvidence(panel_identity)
        },
        PersistedWindowIdentityMigrationOutcome::Rejected(error) => {
            warn!("[convert_v4_state_to_v5] Rejected legacy window target: {error}");
            return PersistedWindowStateConversionOutcome::Rejected(error);
        },
    };
    PersistedWindowStateConversionOutcome::Converted(PersistedWindowState {
        target,
        position: state.position,
        logical_width: state.logical_width,
        logical_height: state.logical_height,
        saved_window_mode: state.saved_window_mode.into(),
        app_name: state.app_name,
    })
}

pub(super) fn resolve_legacy_target(
    panel_identity: PersistedPanelIdentityV4,
    association: &MonitorDeviceAssociation,
    registered_schemes: &RegisteredSchemes,
) -> PersistedWindowIdentityMigrationOutcome {
    let MonitorDeviceKeyLookup::Exact(device_key) =
        association.device_for_legacy_panel(panel_identity)
    else {
        return PersistedWindowIdentityMigrationOutcome::AwaitingLiveEvidence(panel_identity);
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

fn legacy_position(
    logical_position: Option<(i32, i32)>,
    captured_scale: f64,
    persisted_role: &PersistedWindowRole,
) -> PersistedPosition {
    let Some((x, y)) = logical_position else {
        return PersistedPosition::Unpositioned;
    };
    UnrebasedDesktopPosition::from_legacy(IVec2::new(x, y), captured_scale).map_or_else(
        || {
            warn!(
                "[legacy_position] [{persisted_role}] Discarding saved position: \
                 monitor_scale {captured_scale} is not a finite number greater than zero"
            );
            PersistedPosition::Unpositioned
        },
        PersistedPosition::Unrebased,
    )
}

/// Encode live v5 state for a normal save.
pub(super) fn encode(
    states: &HashMap<PersistedWindowRole, PersistedWindowState>,
) -> Result<String, Error> {
    let mut entries: Vec<PersistedEntryV5> = states
        .iter()
        .map(|(persisted_role, window_state)| PersistedEntryV5 {
            persisted_role: persisted_role.clone(),
            window_state:   window_state.clone(),
        })
        .collect();
    entries.sort_by(|left, right| left.persisted_role.cmp(&right.persisted_role));
    let envelope = PersistedStateV5 {
        version: CURRENT_STATE_VERSION,
        entries,
    };
    let ron_body =
        persistence_ron_options().to_string_pretty(&envelope, PrettyConfig::default())?;
    Ok(format!("{RON_HEADER}{ron_body}"))
}

/// v5 envelope written by [`encode`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedStateV5 {
    version: u8,
    entries: Vec<PersistedEntryV5>,
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::collections::HashMap;

    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::Digest;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::SchemeName;
    use ron::ser::PrettyConfig;
    use ron::ser::to_string_pretty;

    use super::*;

    fn schemes() -> RegisteredSchemes {
        let mut schemes = RegisteredSchemes::default();
        let scheme = SchemeName::new("edid-serial")
            .unwrap_or_else(|error| panic!("test scheme rejected: {error}"));
        schemes.register(scheme);
        schemes
    }

    fn decode(contents: &str) -> PersistedWindowStateDecodeOutcome {
        super::decode(contents, &MonitorDeviceAssociation::default(), &schemes())
    }

    fn decoded_states(contents: &str) -> HashMap<RoleKey, PersistedWindowState> {
        match decode(contents) {
            PersistedWindowStateDecodeOutcome::Decoded(states) => states,
            PersistedWindowStateDecodeOutcome::WholeFileRejected => {
                panic!("expected a structurally valid persisted state")
            },
        }
    }

    fn awaiting_state() -> PersistedWindowState {
        PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::MonitorOffset(IVec2::new(10, 20)),
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("test-app"),
        }
    }

    fn primary_role() -> RoleKey {
        super::super::primary_window_role()
            .unwrap_or_else(|error| panic!("test primary role rejected: {error}"))
    }

    #[test]
    fn checked_in_v3_fixture_runs_both_adjacent_conversions() {
        let contents = include_str!("../../tests/config/ron/windows/v3_anonymous_awaiting.ron");
        let states = decoded_states(contents);
        let state = states
            .get(&primary_role())
            .unwrap_or_else(|| panic!("v3 fixture did not retain primary state"));
        assert_eq!(
            state.target,
            PersistedWindowTargetV5::AwaitingLegacyEvidence(PersistedPanelIdentityV4::Anonymous)
        );
    }

    #[test]
    fn frozen_v3_and_v4_keep_their_shipped_unknown_field_compatibility() {
        let v3 = "(
            version: 3,
            future_envelope: \"accepted by the shipped v3 decoder\",
            entries: [(
                key: Primary,
                future_entry: \"accepted by the shipped v3 decoder\",
                state: (
                    position: MonitorOffset((10, 20)),
                    logical_width: 800,
                    logical_height: 600,
                    monitor_index: 0,
                    monitor_panel: Anonymous,
                    mode: Windowed,
                ),
            )],
        )";
        let v4 = "(
            version: 4,
            future_envelope: \"accepted by the shipped v4 decoder\",
            entries: [(
                key: Primary,
                future_entry: \"accepted by the shipped v4 decoder\",
                state: (
                    position: MonitorOffset((10, 20)),
                    logical_width: 800,
                    logical_height: 600,
                    monitor_panel: Anonymous,
                    mode: Windowed,
                    future_state: \"accepted by the shipped v4 decoder\",
                ),
            )],
        )";

        for contents in [v3, v4] {
            let states = decoded_states(contents);
            assert_eq!(states.len(), 1);
            assert!(states.contains_key(&primary_role()));
        }
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
    fn malformed_v3_and_v4_known_fields_preserve_valid_siblings() {
        let v3 = "(
            version: 3,
            entries: [
                (
                    key: Primary,
                    state: (
                        position: MonitorOffset((10, 20)),
                        logical_width: 800,
                        logical_height: 600,
                        monitor_index: 0,
                        monitor_panel: Anonymous,
                        mode: Windowed,
                    ),
                ),
                (
                    key: Managed(\"malformed\"),
                    state: (
                        position: MonitorOffset((10, 20)),
                        logical_width: 800,
                        logical_height: \"not a number\",
                        monitor_index: 0,
                        monitor_panel: Anonymous,
                        mode: Windowed,
                    ),
                ),
            ],
        )";
        let v4 = "(
            version: 4,
            entries: [
                (
                    key: Primary,
                    state: (
                        position: MonitorOffset((10, 20)),
                        logical_width: 800,
                        logical_height: 600,
                        monitor_panel: Anonymous,
                        mode: Windowed,
                    ),
                ),
                (
                    key: Managed(\"malformed\"),
                    state: (
                        position: MonitorOffset((10, 20)),
                        logical_width: 800,
                        logical_height: \"not a number\",
                        monitor_panel: Anonymous,
                        mode: Windowed,
                    ),
                ),
            ],
        )";

        for contents in [v3, v4] {
            let states = decoded_states(contents);
            assert_eq!(states.len(), 1);
            assert!(states.contains_key(&primary_role()));
        }
    }

    #[test]
    fn v4_fingerprinted_target_without_fresh_evidence_stays_awaiting() {
        let contents = "(
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
        let states = decoded_states(contents);
        assert_eq!(
            states[&primary_role()].target,
            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Fingerprinted(
                    super::super::window_state::PersistedPanelFingerprintV4(9),
                ),
            )
        );
    }

    #[test]
    fn checked_in_v4_fixtures_retain_evidence_and_self_heal_rejected_siblings() {
        for contents in [
            include_str!("../../tests/config/ron/macos/v4_synthesized_awaiting.ron"),
            include_str!("../../tests/config/ron/windows/v4_reported_live_match.ron"),
            include_str!("../../tests/config/ron/linux/v4_anonymous_awaiting.ron"),
        ] {
            let states = decoded_states(contents);
            assert_eq!(states.len(), 1);
            assert!(matches!(
                states.get(&primary_role()),
                Some(PersistedWindowState {
                    target: PersistedWindowTargetV5::AwaitingLegacyEvidence(_),
                    ..
                })
            ));
        }

        let recovered = decoded_states(include_str!(
            "../../tests/config/ron/linux/v4_rejected_sibling_self_heal.ron"
        ));
        let primary = recovered
            .get(&primary_role())
            .cloned()
            .unwrap_or_else(|| panic!("valid v4 sibling did not survive entry rejection"));
        assert_eq!(recovered.len(), 1);
        let self_healed = encode(&HashMap::from([(PersistedWindowRole::Primary, primary)]))
            .unwrap_or_else(|error| panic!("recovered v4 entry did not serialize: {error}"));
        assert!(self_healed.contains("version: 5"));
        assert_eq!(decoded_states(&self_healed).len(), 1);
    }

    #[test]
    fn v1_and_v2_keep_anonymous_targets_without_index_fallback() {
        let v1 = "(
            version: 1,
            entries: [(
                key: Primary,
                state: (position: Some((10, 20)), width: 800, height: 600, monitor_index: 7, mode: Windowed),
            )],
        )";
        let v2 = "(
            version: 2,
            entries: [(
                key: Primary,
                state: (logical_position: Some((10, 20)), logical_width: 800, logical_height: 600, monitor_scale: 2.0, monitor_index: 9, mode: Windowed),
            )],
        )";
        for contents in [v1, v2] {
            assert_eq!(
                decoded_states(contents)[&primary_role()].target,
                PersistedWindowTargetV5::AwaitingLegacyEvidence(
                    PersistedPanelIdentityV4::Anonymous,
                )
            );
        }
    }

    #[test]
    fn v5_roundtrip_keeps_a_classified_and_an_awaiting_target() {
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
            (PersistedWindowRole::Primary, classified.clone()),
            (
                PersistedWindowRole::Managed(String::from("offline")),
                awaiting_state(),
            ),
        ]);
        let encoded =
            encode(&states).unwrap_or_else(|error| panic!("v5 state did not serialize: {error}"));
        assert!(encoded.contains("version: 5"));
        let decoded = decoded_states(&encoded);
        assert_eq!(decoded[&primary_role()], classified);
        assert_eq!(decoded.len(), 2);
    }

    #[test]
    fn v5_roundtrip_accepts_a_registered_reported_display_key() {
        let scheme = SchemeName::new("edid-serial")
            .unwrap_or_else(|error| panic!("test scheme rejected: {error}"));
        let value = ReportedId::new("reported-panel")
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
            state.clone(),
        )]))
        .unwrap_or_else(|error| panic!("reported v5 state did not serialize: {error}"));
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
                        value:  ReportedId::new("panel")
                            .unwrap_or_else(|error| panic!("test id rejected: {error}")),
                    },
                }),
                ..awaiting_state()
            },
        };
        let contents = to_string_pretty(
            &PersistedStateV5 {
                version: CURRENT_STATE_VERSION,
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
