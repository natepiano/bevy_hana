use crate::MetricDescriptor;
/// Catalogue lookup without acquiring source demand.
#[derive(Clone, Copy, Debug)]
pub enum MetricLookup<'a> {
    /// Metadata for an owned, adopted or discovered series.
    Registered(&'a MetricDescriptor),
    /// The catalogue contains no descriptor at this path.
    Unregistered,
}
