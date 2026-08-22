use std::time::Duration;

use bevy::prelude::Reflect;

/// Condition that permits another apply attempt after a provider reports a failure.
///
/// `RetryOn` distinguishes a failure that needs a new device report from one that can clear while
/// the report remains unchanged, preventing both missed camera recovery and needless retries.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub enum RetryOn {
    /// Retry after the device set advances because a provider observed a change.
    ///
    /// A permanently unavailable display does not retry until its provider reports a new set, so
    /// this policy cannot issue an unbounded sequence of attempts against the same report.
    NewRevision,
    /// Retry at a fixed cadence when another application can release the device without changing
    /// the reported set.
    ///
    /// A camera held open by another application becomes available when that application exits,
    /// even though no camera appears or disappears and the reporter revision does not advance.
    Interval(Duration),
}

/// Response to an apply attempt abandoned after the device set changes while the device remains
/// reachable.
///
/// The kernel consults `OnAbort` only for a revision change. A lost `Claim` makes reversion
/// impossible, and a deferred `ServiceVeto` makes it unsafe, so both outcomes bypass this policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
pub enum OnAbort {
    /// Keep the partial device configuration until the next reconciliation selects a new target.
    ///
    /// A projector lamp can take tens of seconds to reconfigure, so leaving its partial state is
    /// preferable when the revision change that abandoned the attempt immediately supplies the
    /// next target.
    #[default]
    LeaveAsIs,
    /// Apply the configuration captured before the abandoned attempt started.
    ///
    /// A window between two monitors is visibly broken while only partly placed, so its provider
    /// restores the captured configuration instead of waiting for the next reconciliation.
    Revert,
}

/// Response when a still-present device loses its local session without becoming absent.
///
/// `OnSessionLoss` differs from `RecoveryPolicy`, which handles a departing device, and `OnAbort`,
/// which handles an in-flight apply attempt. Nothing unplugged and no attempt was abandoned.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
pub enum OnSessionLoss {
    /// Open another local session and apply the captured configuration without application code.
    ///
    /// A camera panel can turn black for a frame after laptop wake, then repopulate through an
    /// ordinary attempt that retains its published image handle for bound materials. `RetryOn`
    /// gates that attempt, so a camera the operating system has wedged is not reopened in a
    /// loop.
    #[default]
    Recreate,
    /// Report the session loss without opening a replacement session.
    ///
    /// A window that should be placed differently after compositor destruction needs an
    /// application decision instead of repeating its earlier placement automatically.
    ReportOnly,
}

#[cfg(test)]
mod tests {
    use super::OnAbort;
    use super::OnSessionLoss;

    #[test]
    fn leave_as_is_is_the_default_abort_policy() {
        assert!(matches!(OnAbort::default(), OnAbort::LeaveAsIs));
    }

    #[test]
    fn recreate_is_the_default_session_loss_policy() {
        assert!(matches!(OnSessionLoss::default(), OnSessionLoss::Recreate));
    }
}
