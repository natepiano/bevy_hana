use bevy_diagnostic::DiagnosticPath;
use bevy_ecs::prelude::Res;
use bevy_ecs::system::SystemParam;

use super::MetricLookup;
use crate::MetricDescriptor;
use crate::recording::MetricRegistry;
/// Read-only access to descriptors, including dormant and native sources.
#[derive(SystemParam)]
pub struct MetricCatalogue<'w> {
    registry: Res<'w, MetricRegistry>,
}
impl MetricCatalogue<'_> {
    /// Enumerates descriptor metadata without enabling collection.
    pub fn descriptors(&self) -> impl Iterator<Item = &MetricDescriptor> {
        self.registry
            .entries
            .values()
            .map(|entry| &entry.descriptor)
    }
    /// Looks up a stable path without acquiring demand.
    #[must_use]
    pub fn lookup(&self, path: &DiagnosticPath) -> MetricLookup<'_> {
        self.registry
            .entries
            .get(path)
            .map_or(MetricLookup::Unregistered, |entry| {
                MetricLookup::Registered(&entry.descriptor)
            })
    }
}
