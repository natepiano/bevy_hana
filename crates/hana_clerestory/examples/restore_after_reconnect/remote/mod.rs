mod snapshot;

use std::collections::BTreeMap;

use bevy::prelude::AppExit;
use bevy::prelude::In;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy_remote::BrpError;
use bevy_remote::BrpResult;
use bevy_remote::RemotePlugin;
use bevy_remote::error_codes::INVALID_PARAMS;
use bevy_remote::http::RemoteHttpPlugin;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
pub(super) use snapshot::ProbeReadiness;
pub(super) use snapshot::record_probe_readiness;
use snapshot::snapshot;

use super::constants::KIND_MONITOR_DISCONNECTED;
use super::constants::PROBE_COMMAND_METHOD;
use super::constants::PROBE_RECORDS_METHOD;
use super::constants::PROBE_SCHEMA_VERSION;
use super::constants::PROBE_SHUTDOWN_METHOD;
use super::constants::PROBE_SNAPSHOT_METHOD;
use super::control::CommandReceipts;
use super::control::ProbeCommand;
use super::control::ProbeCommandIntent;
use super::trace::ProbeTrace;

#[derive(Resource)]
pub(super) struct ProbeSession {
    pub(super) run_id:     String,
    pub(super) boot_nonce: String,
    capability:            String,
}

impl ProbeSession {
    pub(super) const fn new(run_id: String, boot_nonce: String, capability: String) -> Self {
        Self {
            run_id,
            boot_nonce,
            capability,
        }
    }
}

#[derive(Deserialize)]
struct AuthenticatedRequest {
    capability: String,
}

#[derive(Deserialize)]
struct RecordRequest {
    capability:     String,
    #[serde(default)]
    after_sequence: u64,
}

trait CapabilityRequest {
    fn capability(&self) -> &str;
}

impl CapabilityRequest for AuthenticatedRequest {
    fn capability(&self) -> &str { &self.capability }
}

impl CapabilityRequest for RecordRequest {
    fn capability(&self) -> &str { &self.capability }
}

#[derive(Deserialize)]
struct CommandRequest {
    capability: String,
    command_id: String,
    command:    ProbeCommand,
}

impl CapabilityRequest for CommandRequest {
    fn capability(&self) -> &str { &self.capability }
}

#[derive(Serialize)]
struct WireRecord {
    run_id:                String,
    boot_nonce:            String,
    sequence:              u64,
    cycle_id:              usize,
    timestamp_unix_micros: u128,
    frame_count:           u32,
    producer:              String,
    kind:                  String,
    fields:                BTreeMap<String, String>,
}

#[derive(Serialize)]
struct RecordResponse {
    schema_version: u32,
    run_id:         String,
    boot_nonce:     String,
    next_cursor:    u64,
    records:        Vec<WireRecord>,
}

enum ProbeRequestParameters {
    Provided(Value),
    Missing,
}

impl From<Option<Value>> for ProbeRequestParameters {
    fn from(params: Option<Value>) -> Self { params.map_or(Self::Missing, Self::Provided) }
}

pub(super) fn plugin() -> RemotePlugin {
    RemotePlugin::default()
        .with_method_main(PROBE_COMMAND_METHOD, command_handler)
        .with_method_main(PROBE_SNAPSHOT_METHOD, snapshot_handler)
        .with_method_main(PROBE_RECORDS_METHOD, records_handler)
        .with_method_main(PROBE_SHUTDOWN_METHOD, shutdown_handler)
}

pub(super) fn http_plugin(port: u16) -> RemoteHttpPlugin {
    RemoteHttpPlugin::default().with_port(port)
}

fn authenticated<T: for<'de> Deserialize<'de> + CapabilityRequest>(
    params: ProbeRequestParameters,
    session: &ProbeSession,
) -> Result<T, BrpError> {
    let params = match params {
        ProbeRequestParameters::Provided(params) => params,
        ProbeRequestParameters::Missing => {
            return Err(invalid_params("missing request parameters"));
        },
    };
    let request: T = serde_json::from_value(params).map_err(invalid_params)?;
    if request.capability() != session.capability {
        return Err(invalid_params("invalid capability"));
    }
    Ok(request)
}

fn snapshot_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let session = world
        .get_resource::<ProbeSession>()
        .ok_or_else(|| BrpError::internal("probe session is unavailable"))?;
    let _: AuthenticatedRequest = authenticated(params.into(), session)?;
    let probe_snapshot = snapshot(world).map_err(BrpError::internal)?;
    serde_json::to_value(probe_snapshot).map_err(BrpError::internal)
}

fn records_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let session = world
        .get_resource::<ProbeSession>()
        .ok_or_else(|| BrpError::internal("probe session is unavailable"))?;
    let request: RecordRequest = authenticated(params.into(), session)?;
    let trace = world
        .get_resource::<ProbeTrace>()
        .ok_or_else(|| BrpError::internal("probe trace is unavailable"))?;
    let mut cycle_id = 0;
    let mut records = Vec::new();
    for record in trace.records() {
        if record.kind == KIND_MONITOR_DISCONNECTED {
            cycle_id += 1;
        }
        if record.sequence <= request.after_sequence {
            continue;
        }
        records.push(WireRecord {
            run_id: session.run_id.clone(),
            boot_nonce: session.boot_nonce.clone(),
            sequence: record.sequence,
            cycle_id,
            timestamp_unix_micros: record.timestamp_unix_micros,
            frame_count: record.frame_count,
            producer: record.producer,
            kind: record.kind,
            fields: record.fields.into_iter().collect(),
        });
    }
    let next_cursor = records
        .last()
        .map_or(request.after_sequence, |record| record.sequence);
    serde_json::to_value(RecordResponse {
        schema_version: PROBE_SCHEMA_VERSION,
        run_id: session.run_id.clone(),
        boot_nonce: session.boot_nonce.clone(),
        next_cursor,
        records,
    })
    .map_err(BrpError::internal)
}

fn shutdown_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let session = world
        .get_resource::<ProbeSession>()
        .ok_or_else(|| BrpError::internal("probe session is unavailable"))?;
    let _: AuthenticatedRequest = authenticated(params.into(), session)?;
    world.write_message(AppExit::Success);
    serde_json::to_value(serde_json::json!({ "accepted": true })).map_err(BrpError::internal)
}

fn command_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let session = world
        .get_resource::<ProbeSession>()
        .ok_or_else(|| BrpError::internal("probe session is unavailable"))?;
    let request: CommandRequest = authenticated(params.into(), session)?;
    if request.command_id.is_empty() {
        return Err(invalid_params("command_id must not be empty"));
    }
    if let Some(receipt) = world
        .resource::<CommandReceipts>()
        .0
        .get(&request.command_id)
    {
        return serde_json::to_value(receipt).map_err(BrpError::internal);
    }
    let command_id = request.command_id.clone();
    world.trigger(ProbeCommandIntent {
        command_id: request.command_id,
        command:    request.command,
    });
    let receipt = world
        .resource::<CommandReceipts>()
        .0
        .get(&command_id)
        .ok_or_else(|| BrpError::internal("probe command produced no receipt"))?;
    serde_json::to_value(receipt).map_err(BrpError::internal)
}

fn invalid_params(error: impl ToString) -> BrpError {
    BrpError {
        code:    INVALID_PARAMS,
        message: error.to_string(),
        data:    None,
    }
}
