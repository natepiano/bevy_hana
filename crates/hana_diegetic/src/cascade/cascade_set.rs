//! Diegetic system ordering for cascade propagation.

use bevy::prelude::SystemSet;

/// System ordering for diegetic cascade propagation.
///
/// `bevy_kana`'s propagation systems run inside [`CascadeSet::Propagate`], so
/// scheduling against this set orders against every cascade attribute at once.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum CascadeSet {
    /// Updates resolved cascade values after authored values and relationships change.
    Propagate,
}
