//! Descriptors and the paths with active consumer demand.

use std::collections::HashMap;

use bevy_diagnostic::DiagnosticPath;
use bevy_ecs::prelude::Resource;

use super::RegisteredMetric;

#[derive(Default, Resource)]
pub(crate) struct MetricRegistry {
    pub(crate) entries: HashMap<DiagnosticPath, RegisteredMetric>,
    pub(crate) active:  Vec<DiagnosticPath>,
}
