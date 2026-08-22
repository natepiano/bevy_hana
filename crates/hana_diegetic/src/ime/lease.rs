//! Window-scoped IME focus lease and input blocker.

use bevy::prelude::Entity;
use bevy::prelude::Resource;

use super::ImeSessionId;

/// Crate-owned input blocker for the active IME lease.
#[derive(Resource, Clone, Debug, Default)]
pub struct ImeInputBlocker {
    lease: Option<ImeInputLease>,
}

impl ImeInputBlocker {
    /// Returns the active leased window, if any.
    #[must_use]
    pub const fn window(&self) -> Option<Entity> {
        match self.lease {
            Some(lease) => Some(lease.window),
            None => None,
        }
    }

    /// Returns the active leased session, if any.
    #[must_use]
    pub const fn session_id(&self) -> Option<ImeSessionId> {
        match self.lease {
            Some(lease) => Some(lease.session_id),
            None => None,
        }
    }

    /// Returns `true` when input for `window` should be blocked.
    #[must_use]
    pub fn blocks_window(&self, window: Entity) -> bool {
        matches!(self.lease, Some(lease) if lease.window == window)
    }

    /// Returns `true` when the activating gesture happened on `frame`.
    ///
    /// The IME input route uses this to assign one physical key press to one
    /// role: a key that opens an editor cannot also edit or close that newly
    /// opened session during the same frame.
    #[must_use]
    pub const fn captured_activation_frame(&self, frame: u32) -> bool {
        matches!(
            self.lease,
            Some(lease) if matches!(lease.activation_frame, Some(active) if active == frame)
        )
    }

    pub(super) const fn begin_session(
        &mut self,
        session_id: ImeSessionId,
        window: Entity,
        activation_frame: Option<u32>,
    ) {
        self.lease = Some(ImeInputLease {
            session_id,
            window,
            activation_frame,
        });
    }

    pub(super) fn clear_session(&mut self, session_id: ImeSessionId) {
        if self.session_id() == Some(session_id) {
            self.lease = None;
        }
    }

    pub(super) const fn clear(&mut self) { self.lease = None; }

    pub(super) fn matches_session_window(&self, session_id: ImeSessionId, window: Entity) -> bool {
        matches!(
            self.lease,
            Some(lease) if lease.session_id == session_id && lease.window == window
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct ImeInputLease {
    session_id:       ImeSessionId,
    window:           Entity,
    activation_frame: Option<u32>,
}
