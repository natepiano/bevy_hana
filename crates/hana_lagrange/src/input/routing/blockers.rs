//! Per-camera input gating state derived from the routing snapshot.
//!
//! Types:
//! - [`CameraInputBlockers`] — component carrying the set of reasons (if any) why a camera should
//!   not receive input this frame. Computed once per frame from a
//!   [`CameraRoutingSnapshot`](super::snapshot::CameraRoutingSnapshot).
//! - [`CameraInputBlockerBits`] — internal bitflag set of blocker reasons
//!   (`DISABLED`/`INACTIVE_CAMERA`/`ANIMATION_IGNORE`/`UNAVAILABLE_OWNER` block all input;
//!   `SCROLL_DISABLED` blocks scroll alone).
//! - [`CameraInputContextGated`] — component flipping each camera's interaction context between
//!   allowed and blocked, derived from `CameraInputBlockers::is_blocked`.
//! - [`ContextGate`] — `Allowed`/`Blocked` enum that `CameraInputContextGated` wraps.

use bevy::prelude::Component;
use bevy::prelude::Entity;

use super::snapshot::CameraRoutingSnapshot;
use super::snapshot::CameraRoutingSnapshotFlags;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct CameraInputBlockerBits: u8 {
        const DISABLED = 1 << 0;
        const INACTIVE_CAMERA = 1 << 1;
        const ANIMATION_IGNORE = 1 << 2;
        const UNAVAILABLE_OWNER = 1 << 3;
        const SCROLL_DISABLED = 1 << 4;
        /// Reasons that withhold every input from the camera.
        const ALL_INPUT = Self::DISABLED.bits()
            | Self::INACTIVE_CAMERA.bits()
            | Self::ANIMATION_IGNORE.bits()
            | Self::UNAVAILABLE_OWNER.bits();
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CameraInputBlockers {
    pub(super) bits: CameraInputBlockerBits,
}

impl CameraInputBlockers {
    pub(crate) const fn is_blocked(self) -> bool {
        self.bits.intersects(CameraInputBlockerBits::ALL_INPUT)
    }

    pub(crate) const fn blocks_scroll(self) -> bool {
        self.bits.intersects(
            CameraInputBlockerBits::ALL_INPUT.union(CameraInputBlockerBits::SCROLL_DISABLED),
        )
    }

    pub(super) fn from_snapshot(
        snapshot: &CameraRoutingSnapshot,
        routed_camera: Option<Entity>,
    ) -> Self {
        let mut bits = CameraInputBlockerBits::empty();
        if snapshot.has(CameraRoutingSnapshotFlags::DISABLED) {
            bits.insert(CameraInputBlockerBits::DISABLED);
        }
        if !snapshot.has(CameraRoutingSnapshotFlags::MANUAL)
            && routed_camera != Some(snapshot.entity)
        {
            bits.insert(CameraInputBlockerBits::INACTIVE_CAMERA);
        }
        if snapshot.has(CameraRoutingSnapshotFlags::ANIMATION_IGNORE) {
            bits.insert(CameraInputBlockerBits::ANIMATION_IGNORE);
        }
        if snapshot.has(CameraRoutingSnapshotFlags::SCROLL_DISABLED) {
            bits.insert(CameraInputBlockerBits::SCROLL_DISABLED);
        }
        Self { bits }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CameraInputContextGated {
    pub(crate) context_gate: ContextGate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContextGate {
    Allowed,
    #[default]
    Blocked,
}

impl ContextGate {
    pub(crate) const fn is_allowed(self) -> bool { matches!(self, Self::Allowed) }
}

impl From<bool> for ContextGate {
    fn from(allowed: bool) -> Self {
        if allowed {
            Self::Allowed
        } else {
            Self::Blocked
        }
    }
}
