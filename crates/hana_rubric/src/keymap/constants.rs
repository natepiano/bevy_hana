use std::time::Duration;

// sequence matching
/// Maximum elapsed time between keystrokes in one keymap sequence.
pub(super) const SEQUENCE_TIMEOUT: Duration = Duration::from_secs(1);
