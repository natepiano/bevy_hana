use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use bevy::prelude::Resource;
use serde::Serialize;

/// One causal record emitted by the probe's observers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TraceRecord {
    pub(crate) sequence:              u64,
    pub(crate) timestamp_unix_micros: u128,
    pub(crate) frame_count:           u32,
    pub(crate) producer:              String,
    pub(crate) kind:                  String,
    pub(crate) fields:                Vec<(String, String)>,
}

struct TraceState {
    next_sequence: u64,
    instant:       Instant,
    unix_micros:   u128,
    records:       Vec<TraceRecord>,
}

impl Default for TraceState {
    fn default() -> Self {
        let started_unix_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_micros());
        Self {
            next_sequence: 1,
            instant:       Instant::now(),
            unix_micros:   started_unix_micros,
            records:       Vec::new(),
        }
    }
}

/// Collects the [`TraceRecord`]s of one probe process, stamping each with the next sequence
/// number and a timestamp taken from a single start point. Clones share one record list.
#[derive(Clone, Default, Resource)]
pub(crate) struct ProbeTrace(Arc<Mutex<TraceState>>);

impl ProbeTrace {
    pub(crate) fn record(
        &self,
        frame_count: u32,
        producer: impl Into<String>,
        kind: impl Into<String>,
        fields: Vec<(String, String)>,
    ) {
        let mut state = match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let record = TraceRecord {
            sequence: state.next_sequence,
            timestamp_unix_micros: state.unix_micros + state.instant.elapsed().as_micros(),
            frame_count,
            producer: producer.into(),
            kind: kind.into(),
            fields,
        };
        state.next_sequence += 1;
        state.records.push(record);
    }

    pub(crate) fn records(&self) -> Vec<TraceRecord> {
        match self.0.lock() {
            Ok(state) => state.records.clone(),
            Err(poisoned) => poisoned.into_inner().records.clone(),
        }
    }
}
