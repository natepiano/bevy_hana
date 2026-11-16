//! Evidence boundary acknowledged for one consumer, independent of other demand.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use bevy_diagnostic::DiagnosticPath;

use crate::MetricNumber;
use crate::ObservationEpoch;
use crate::RunOffset;
use crate::recording::EffectiveEpoch;

pub(crate) type SubscriptionBoundaries = Arc<Mutex<HashMap<DiagnosticPath, SubscriptionBoundary>>>;

pub(crate) struct SubscriptionBoundary {
    pub(crate) epoch:        ObservationEpoch,
    pub(crate) effective_at: RunOffset,
    pub(crate) baseline:     CounterBaseline,
    pub(super) activation:   Arc<Mutex<EffectiveEpoch>>,
}

#[derive(Clone, Copy)]
pub(crate) enum CounterBaseline {
    AwaitingSnapshot,
    Measured {
        total:    u64,
        at:       RunOffset,
        sequence: u64,
    },
}

impl CounterBaseline {
    pub(crate) const fn from_sample(sample: &crate::RetainedObservation) -> Self {
        match sample.number {
            MetricNumber::Count(total) => Self::Measured {
                total,
                at: sample.extent.end(),
                sequence: sample.sequence,
            },
            MetricNumber::Scalar(_) => Self::AwaitingSnapshot,
        }
    }
}
