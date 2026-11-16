//! Source time and causal context, independent of a renderer or wall clock.

use std::time::Duration;

use crate::InvalidObservationInterval;

/// Monotonic elapsed time since the current run's origin.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RunOffset(Duration);

impl RunOffset {
    /// Creates an offset from elapsed run time.
    #[must_use]
    pub const fn new(elapsed: Duration) -> Self { Self(elapsed) }

    /// Returns elapsed time since the run origin.
    #[must_use]
    pub const fn duration(self) -> Duration { self.0 }
}

/// Actual source interval measured within one run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationInterval {
    start: RunOffset,
    end:   RunOffset,
}

impl ObservationInterval {
    /// Creates a source interval with ordered endpoints; zero duration is valid.
    ///
    /// # Errors
    /// Returns an error when the end precedes the start.
    pub fn new(start: RunOffset, end: RunOffset) -> Result<Self, InvalidObservationInterval> {
        if end < start {
            return Err(InvalidObservationInterval::ReversedEndpoints);
        }
        Ok(Self { start, end })
    }

    /// Returns the source interval's opening endpoint.
    #[must_use]
    pub const fn start(self) -> RunOffset { self.start }

    /// Returns the source interval's closing endpoint.
    #[must_use]
    pub const fn end(self) -> RunOffset { self.end }

    /// Returns the actual source interval's length.
    #[must_use]
    pub const fn duration(self) -> Duration { self.end.0.saturating_sub(self.start.0) }
}

/// Identifier for a main-app update within a run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MainUpdateId(
    /// Run-local numeric identity.
    pub u64,
);

/// Identifier for a render-app update within a run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenderUpdateId(
    /// Run-local numeric identity.
    pub u64,
);

/// Identifier for a logical operation within a run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId(
    /// Run-local numeric identity.
    pub u64,
);

/// Identifier for the cause of an observed operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CausalId(
    /// Run-local numeric identity.
    pub u64,
);

/// Relationship between an observation and application frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameContext {
    /// Work in a known main-app update.
    MainUpdate(MainUpdateId),
    /// Work in a render update using data from the named main update.
    RenderUpdate {
        /// Render update performing the work.
        render:    RenderUpdateId,
        /// Main update whose extracted data is used.
        extracted: MainUpdateId,
    },
    /// Work outside an application frame.
    OutsideFrame,
    /// A source with its own schedule and no frame attribution.
    Independent,
}

/// Logical operation and optional causal attribution, stated explicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationContext {
    /// No logical operation is attributed.
    NoOperation,
    /// Work attributed to one logical operation.
    Operation(OperationId),
    /// Work attributed to an operation and its recorded cause.
    CausedBy {
        /// Operation performing the work.
        operation: OperationId,
        /// Cause that initiated the operation.
        cause:     CausalId,
    },
}

/// Frame and causal attribution carried beside an observation, never in its path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationContext {
    /// Application-frame attribution.
    pub frame:     FrameContext,
    /// Logical-operation attribution.
    pub operation: OperationContext,
}

/// Numeric reading preserving exact integer counters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricNumber {
    /// Floating-point gauge or duration value.
    Scalar(f64),
    /// Exact nonnegative counter value.
    Count(u64),
}
