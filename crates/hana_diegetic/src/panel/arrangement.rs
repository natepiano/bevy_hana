//! Diegetic-panel membership adapter for `hana_valence` arrangements.

use bevy::prelude::Bundle;
use bevy::prelude::Entity;
use hana_valence::Member;

/// Insert-only bundle that makes a panel a member root of an arrangement.
///
/// This bundle carries only the logical [`Member`] relationship. It does not
/// create a physical attachment, pose, or hinge: panel code that needs those
/// values authors [`hana_valence::AnchoredTo`] and later hinge state directly.
/// Replacing this bundle's `Member` component retargets the panel to a new
/// arrangement controller through Bevy's relationship hooks.
#[derive(Bundle, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrangedPanel {
    member: Member,
}

impl ArrangedPanel {
    /// Creates membership of the arrangement controller at `arrangement`.
    #[must_use]
    pub const fn new(arrangement: Entity) -> Self {
        Self {
            member: Member::new(arrangement),
        }
    }

    /// Returns the arrangement controller this bundle makes the panel a member of.
    #[must_use]
    pub const fn arrangement(&self) -> Entity { self.member.arrangement_entity }
}
