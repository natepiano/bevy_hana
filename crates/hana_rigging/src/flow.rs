use std::time::Duration;
use std::time::Instant;

use bevy::prelude::Reflect;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;
use thiserror::Error;

use crate::RiggingRuntimeClock;
use crate::RiggingRuntimeTime;
use crate::transport::DeviceTransport;
use crate::transport::TransportObservation;

/// Reason a continuous-flow interval constructor rejected a duration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum FlowIntervalError {
    /// A zero interval would expire as soon as its flow state was evaluated.
    #[error("continuous-flow intervals must be greater than zero")]
    Zero,
}

/// Maximum time an established session may wait for its first datum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
pub struct FirstDatumTimeout(Duration);

impl FirstDatumTimeout {
    /// Create a non-zero bound for the first datum from an established session.
    ///
    /// # Errors
    ///
    /// Returns [`FlowIntervalError::Zero`] when `duration` is zero.
    pub const fn new(duration: Duration) -> Result<Self, FlowIntervalError> {
        if duration.is_zero() {
            return Err(FlowIntervalError::Zero);
        }

        Ok(Self(duration))
    }

    /// Return the configured first-datum duration.
    #[must_use]
    pub const fn duration(self) -> Duration { self.0 }
}

/// Maximum time an established session may go without another datum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
pub struct MaximumDatumGap(Duration);

impl MaximumDatumGap {
    /// Create a non-zero bound between consecutive data arrivals.
    ///
    /// # Errors
    ///
    /// Returns [`FlowIntervalError::Zero`] when `duration` is zero.
    pub const fn new(duration: Duration) -> Result<Self, FlowIntervalError> {
        if duration.is_zero() {
            return Err(FlowIntervalError::Zero);
        }

        Ok(Self(duration))
    }

    /// Return the configured maximum duration between data arrivals.
    #[must_use]
    pub const fn duration(self) -> Duration { self.0 }
}

/// Timing bounds for a session whose data must continue arriving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
pub struct ContinuousFlowExpectation {
    first_datum_timeout: FirstDatumTimeout,
    maximum_datum_gap:   MaximumDatumGap,
}

impl ContinuousFlowExpectation {
    /// Configure the bounds before and after the first datum arrives.
    #[must_use]
    pub const fn new(
        first_datum_timeout: FirstDatumTimeout,
        maximum_datum_gap: MaximumDatumGap,
    ) -> Self {
        Self {
            first_datum_timeout,
            maximum_datum_gap,
        }
    }

    /// Return the maximum wait for the first datum.
    #[must_use]
    pub const fn first_datum_timeout(self) -> FirstDatumTimeout { self.first_datum_timeout }

    /// Return the maximum gap permitted after data begins arriving.
    #[must_use]
    pub const fn maximum_datum_gap(self) -> MaximumDatumGap { self.maximum_datum_gap }
}

/// Flow monitoring configured for an established binding session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum FlowExpectation {
    /// The session is not evaluated for data arrival.
    #[default]
    NotMonitored,
    /// The session must satisfy the configured continuous-flow bounds.
    Continuous(ContinuousFlowExpectation),
}

/// What was observed about one session's most recent datum arrival.
///
/// Absence is a named variant rather than an `Option` so both boundaries that carry this
/// evidence — the driver reporting a pre-establishment observation and the coalescing arrival slot
/// a live lease writes — state what was seen instead of leaving a reader to infer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionDatumArrivalEvidence {
    /// Nothing has been observed for this session since evidence was last collected.
    NoDatumObserved,
    /// A datum was delivered to a consumer at this frame-clock instant.
    ///
    /// Marked non-exhaustive so no driver can spell it directly: outside this crate the only way
    /// to produce it is
    /// [`DeviceTransport::pre_establishment_evidence`](crate::DeviceTransport::pre_establishment_evidence),
    /// which derives it from a driver's own classification of what its transport carried.
    #[non_exhaustive]
    ObservedAt(Instant),
    /// Transport activity carrying no datum was observed at this frame-clock instant.
    ///
    /// Activity proves the session is alive. It is not a datum, so it can never be the arrival
    /// that starts a session flowing. Non-exhaustive for the same reason as
    /// [`Self::ObservedAt`].
    #[non_exhaustive]
    TransportActivityAt(Instant),
}

impl SessionDatumArrivalEvidence {
    /// Evidence for one classified transport observation.
    ///
    /// The kernel owns this translation so that no driver decides for itself what counts as an
    /// arrival: a driver states only what its transport carried, and everything downstream of
    /// that — including whether a session may begin flowing — follows from this one place.
    pub(crate) const fn from_observation(
        observation: TransportObservation,
        observed_at: Instant,
    ) -> Self {
        match observation {
            TransportObservation::DatumDelivered => Self::ObservedAt(observed_at),
            TransportObservation::ActivityWithoutDatum => Self::TransportActivityAt(observed_at),
            TransportObservation::Quiet => Self::NoDatumObserved,
        }
    }

    /// Fold one newly classified observation into a slot a driver retains for a session whose
    /// lease has not been issued yet.
    ///
    /// The slot exists so establishment can start a session's flow bounds from the freshest proof
    /// its transport delivered instead of from silence it never had. Only a delivered datum is
    /// that proof, so only a datum may displace one: activity and silence leave a retained datum
    /// where it is, and a session that lost its retained arrival to a buffer-less sample would
    /// begin awaiting a first datum it had already received.
    pub fn retain<Transport>(&mut self, observation: &Transport::Observation, observed_at: Instant)
    where
        Transport: DeviceTransport,
    {
        match Transport::classify(observation) {
            TransportObservation::DatumDelivered => *self = Self::ObservedAt(observed_at),
            TransportObservation::ActivityWithoutDatum => {
                if matches!(*self, Self::NoDatumObserved) {
                    *self = Self::TransportActivityAt(observed_at);
                }
            },
            TransportObservation::Quiet => {},
        }
    }
}

/// Published flow state for one established session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum EstablishedFlowView {
    /// The session is not evaluated for data arrival.
    NotMonitored,
    /// The session is evaluated for continuous data arrival.
    Continuous(ContinuousFlowView),
}

/// Published data-arrival state for a continuously monitored session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ContinuousFlowView {
    /// The session is established but has not supplied its first datum.
    AwaitingFirstDatum {
        /// Runtime time after which the first datum is overdue.
        deadline: RiggingRuntimeTime,
    },
    /// Data has arrived within the configured maximum gap.
    Flowing {
        /// Runtime time when the first accepted datum moved the session into flowing state.
        flowing_since: RiggingRuntimeTime,
    },
    /// The session exceeded one of its configured flow bounds.
    Stalled {
        /// Runtime time at which flow became overdue.
        since: RiggingRuntimeTime,
        /// Bound the session crossed.
        cause: ContinuousFlowExpiryCause,
    },
}

/// Kernel-owned flow state scoped to one established session.
#[derive(Debug)]
pub(super) struct EstablishedFlow {
    state: EstablishedFlowState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EstablishedFlowExpiry {
    Current,
    Crossed(ContinuousFlowExpiryCause),
    ReleaseRequired(ContinuousFlowExpiryCause),
}

/// Which configured flow bound a stalled session crossed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ContinuousFlowExpiryCause {
    /// The session never supplied a datum inside its first-datum timeout.
    FirstDatumOverdue,
    /// Data arrived and then stopped for longer than the permitted maximum gap.
    MaximumDatumGapExceeded,
}

impl EstablishedFlow {
    pub(super) const fn new(expectation: FlowExpectation, established_at: Instant) -> Self {
        let state = match expectation {
            FlowExpectation::NotMonitored => EstablishedFlowState::NotMonitored,
            FlowExpectation::Continuous(expectation) => EstablishedFlowState::Continuous(
                ContinuousEstablishedFlow::AwaitingFirstDatum(AwaitingFirstDatum {
                    expectation,
                    established_at,
                }),
            ),
        };
        Self { state }
    }

    pub(super) const fn is_monitored(&self) -> bool {
        matches!(&self.state, EstablishedFlowState::Continuous(_))
    }

    pub(super) fn view(&self, runtime_clock: RiggingRuntimeClock) -> EstablishedFlowView {
        match &self.state {
            EstablishedFlowState::NotMonitored => EstablishedFlowView::NotMonitored,
            EstablishedFlowState::Continuous(continuous) => {
                EstablishedFlowView::Continuous(continuous.view(runtime_clock))
            },
        }
    }

    /// Credit one delivered datum against this session's flow bounds.
    ///
    /// Which transition this performs belongs to the state, not to this method: an awaiting
    /// session takes its one path into [`Flowing`], a flowing session extends its permitted gap,
    /// and a stalled session has no transition that accepts a datum at all.
    pub(super) const fn record_datum_arrival(&mut self, delivered_at: Instant) {
        let EstablishedFlowState::Continuous(continuous) = &mut self.state else {
            return;
        };
        *continuous = continuous.receive_datum(delivered_at);
    }

    /// Credit transport activity that carried no datum against this session's flow bounds.
    ///
    /// [`Flowing`] is the only state that has a method for this. A session awaiting its first
    /// datum is therefore left exactly where it is, and cannot be carried into the state every
    /// presenting reading is derived from by traffic it has nothing to show for.
    pub(super) const fn record_transport_activity(&mut self, observed_at: Instant) {
        let EstablishedFlowState::Continuous(continuous) = &mut self.state else {
            return;
        };
        *continuous = continuous.observe_transport_activity(observed_at);
    }

    pub(super) fn judgment_due(&self, now: Instant) -> bool {
        match &self.state {
            EstablishedFlowState::NotMonitored => false,
            EstablishedFlowState::Continuous(continuous) => continuous.judgment_due(now),
        }
    }

    pub(super) fn evaluate_expiry(&mut self, now: Instant) -> EstablishedFlowExpiry {
        let EstablishedFlowState::Continuous(continuous) = &mut self.state else {
            return EstablishedFlowExpiry::Current;
        };
        let (next, expiry) = continuous.evaluate_expiry(now);
        *continuous = next;
        expiry
    }
}

/// A continuously monitored session that has not yet delivered a datum.
///
/// [`Self::receive_first_datum`] is the one transition out of it, and it takes the instant a
/// datum was delivered. Nothing here accepts transport activity: a device whose transport is
/// alive but has produced no picture has nothing for a consumer to present, and giving that
/// device a method to call is exactly how it reaches a presenting reading it never earned.
#[derive(Clone, Copy, Debug)]
struct AwaitingFirstDatum {
    expectation:    ContinuousFlowExpectation,
    established_at: Instant,
}

impl AwaitingFirstDatum {
    /// The one way into [`Flowing`]. Consumes the awaiting state and requires a delivered datum.
    const fn receive_first_datum(self, delivered_at: Instant) -> Flowing {
        Flowing {
            expectation:    self.expectation,
            first_datum_at: delivered_at,
            last_datum_at:  delivered_at,
        }
    }

    const fn permitted_silence(self) -> Duration {
        self.expectation.first_datum_timeout().duration()
    }

    fn judgment_due(self, now: Instant) -> bool {
        now.saturating_duration_since(self.established_at) > self.permitted_silence()
    }

    fn expire(self, now: Instant) -> FlowExpiryEvaluation<Self> {
        if !self.judgment_due(now) {
            return FlowExpiryEvaluation::Current(self);
        }
        FlowExpiryEvaluation::Crossed(Stalled {
            expiry: ContinuousFlowExpiry {
                cause:              ContinuousFlowExpiryCause::FirstDatumOverdue,
                silence_started_at: self.established_at,
                permitted_silence:  self.permitted_silence(),
            },
        })
    }

    fn deadline(self, runtime_clock: RiggingRuntimeClock) -> RiggingRuntimeTime {
        let elapsed = runtime_clock
            .time_at(self.established_at)
            .elapsed()
            .saturating_add(self.permitted_silence());
        RiggingRuntimeTime::from_elapsed(elapsed)
    }
}

/// A continuously monitored session that has delivered at least one datum.
///
/// Its fields are private and [`AwaitingFirstDatum::receive_first_datum`] is its only
/// constructor, so this state cannot be reached without a datum having been delivered. Every
/// presenting reading a consumer sees is derived from it.
#[derive(Clone, Copy, Debug)]
struct Flowing {
    expectation:    ContinuousFlowExpectation,
    first_datum_at: Instant,
    last_datum_at:  Instant,
}

impl Flowing {
    /// Extend the permitted gap with a later delivered datum.
    const fn receive_datum(mut self, delivered_at: Instant) -> Self {
        self.last_datum_at = delivered_at;
        self
    }

    /// Extend the permitted gap with transport activity that carried no datum.
    ///
    /// Only a session that has already delivered may be held open this way: activity over a
    /// delivered session means nothing has changed since the last datum, so that datum is still
    /// the current picture.
    const fn observe_transport_activity(mut self, observed_at: Instant) -> Self {
        self.last_datum_at = observed_at;
        self
    }

    const fn first_datum_at(self) -> Instant { self.first_datum_at }

    const fn permitted_silence(self) -> Duration { self.expectation.maximum_datum_gap().duration() }

    fn judgment_due(self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_datum_at) > self.permitted_silence()
    }

    fn expire(self, now: Instant) -> FlowExpiryEvaluation<Self> {
        if !self.judgment_due(now) {
            return FlowExpiryEvaluation::Current(self);
        }
        FlowExpiryEvaluation::Crossed(Stalled {
            expiry: ContinuousFlowExpiry {
                cause:              ContinuousFlowExpiryCause::MaximumDatumGapExceeded,
                silence_started_at: self.last_datum_at,
                permitted_silence:  self.permitted_silence(),
            },
        })
    }
}

/// A continuously monitored session that crossed one of its configured bounds.
///
/// There is deliberately no transition out. A stalled session is released, and release ends the
/// session, so nothing a device reports afterwards can carry it back to [`Flowing`].
#[derive(Clone, Copy, Debug)]
struct Stalled {
    expiry: ContinuousFlowExpiry,
}

impl Stalled {
    const fn cause(self) -> ContinuousFlowExpiryCause { self.expiry.cause }

    fn overdue_at(self, runtime_clock: RiggingRuntimeClock) -> RiggingRuntimeTime {
        self.expiry.overdue_at(runtime_clock)
    }
}

/// What evaluating one live flow state against its configured bound produced.
enum FlowExpiryEvaluation<Live> {
    /// The bound holds, and the state is handed back unchanged.
    Current(Live),
    /// The bound was crossed, and the session is now stalled.
    Crossed(Stalled),
}

#[derive(Clone, Copy, Debug)]
struct ContinuousFlowExpiry {
    cause:              ContinuousFlowExpiryCause,
    silence_started_at: Instant,
    permitted_silence:  Duration,
}

impl ContinuousFlowExpiry {
    fn overdue_at(self, runtime_clock: RiggingRuntimeClock) -> RiggingRuntimeTime {
        let elapsed = runtime_clock
            .time_at(self.silence_started_at)
            .elapsed()
            .saturating_add(self.permitted_silence);
        RiggingRuntimeTime::from_elapsed(elapsed)
    }
}

#[derive(Clone, Copy, Debug)]
enum EstablishedFlowState {
    NotMonitored,
    Continuous(ContinuousEstablishedFlow),
}

/// Kernel-owned state for a continuously monitored established session.
///
/// Each variant carries the one type that owns the transitions legal from it. Which transitions
/// exist is the invariant this enum rests on, so the arms below only route: they never decide
/// whether a transition was permitted.
#[derive(Clone, Copy, Debug)]
enum ContinuousEstablishedFlow {
    AwaitingFirstDatum(AwaitingFirstDatum),
    Flowing(Flowing),
    /// The session crossed a configured bound and awaits release.
    Stalled(Stalled),
}

impl ContinuousEstablishedFlow {
    const fn receive_datum(self, delivered_at: Instant) -> Self {
        match self {
            Self::AwaitingFirstDatum(awaiting) => {
                Self::Flowing(awaiting.receive_first_datum(delivered_at))
            },
            Self::Flowing(flowing) => Self::Flowing(flowing.receive_datum(delivered_at)),
            // `Stalled` has no transition that accepts a datum, so late testimony changes
            // nothing any consumer can see.
            Self::Stalled(stalled) => Self::Stalled(stalled),
        }
    }

    const fn observe_transport_activity(self, observed_at: Instant) -> Self {
        match self {
            // Neither of these states has a method taking transport activity, which is what
            // keeps a session that has delivered nothing out of `Flowing`.
            Self::AwaitingFirstDatum(awaiting) => Self::AwaitingFirstDatum(awaiting),
            Self::Stalled(stalled) => Self::Stalled(stalled),
            Self::Flowing(flowing) => {
                Self::Flowing(flowing.observe_transport_activity(observed_at))
            },
        }
    }

    fn judgment_due(self, now: Instant) -> bool {
        match self {
            Self::AwaitingFirstDatum(awaiting) => awaiting.judgment_due(now),
            Self::Flowing(flowing) => flowing.judgment_due(now),
            Self::Stalled(_) => true,
        }
    }

    fn evaluate_expiry(self, now: Instant) -> (Self, EstablishedFlowExpiry) {
        match self {
            Self::AwaitingFirstDatum(awaiting) => match awaiting.expire(now) {
                FlowExpiryEvaluation::Current(awaiting) => (
                    Self::AwaitingFirstDatum(awaiting),
                    EstablishedFlowExpiry::Current,
                ),
                FlowExpiryEvaluation::Crossed(stalled) => (
                    Self::Stalled(stalled),
                    EstablishedFlowExpiry::Crossed(stalled.cause()),
                ),
            },
            Self::Flowing(flowing) => match flowing.expire(now) {
                FlowExpiryEvaluation::Current(flowing) => {
                    (Self::Flowing(flowing), EstablishedFlowExpiry::Current)
                },
                FlowExpiryEvaluation::Crossed(stalled) => (
                    Self::Stalled(stalled),
                    EstablishedFlowExpiry::Crossed(stalled.cause()),
                ),
            },
            Self::Stalled(stalled) => (
                Self::Stalled(stalled),
                EstablishedFlowExpiry::ReleaseRequired(stalled.cause()),
            ),
        }
    }

    fn view(self, runtime_clock: RiggingRuntimeClock) -> ContinuousFlowView {
        match self {
            Self::AwaitingFirstDatum(awaiting) => ContinuousFlowView::AwaitingFirstDatum {
                deadline: awaiting.deadline(runtime_clock),
            },
            Self::Flowing(flowing) => ContinuousFlowView::Flowing {
                flowing_since: runtime_clock.time_at(flowing.first_datum_at()),
            },
            Self::Stalled(stalled) => ContinuousFlowView::Stalled {
                since: stalled.overdue_at(runtime_clock),
                cause: stalled.cause(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::ContinuousFlowExpectation;
    use super::ContinuousFlowExpiryCause;
    use super::ContinuousFlowView;
    use super::EstablishedFlow;
    use super::EstablishedFlowExpiry;
    use super::EstablishedFlowView;
    use super::FirstDatumTimeout;
    use super::FlowExpectation;
    use super::FlowIntervalError;
    use super::MaximumDatumGap;
    use crate::RiggingRuntimeClock;
    use crate::RiggingRuntimeTime;

    #[test]
    fn zero_first_datum_timeout_is_rejected() {
        assert_eq!(
            FirstDatumTimeout::new(Duration::ZERO),
            Err(FlowIntervalError::Zero)
        );
    }

    #[test]
    fn zero_maximum_datum_gap_is_rejected() {
        assert_eq!(
            MaximumDatumGap::new(Duration::ZERO),
            Err(FlowIntervalError::Zero)
        );
    }

    #[test]
    fn continuous_expectation_retains_each_typed_interval() -> Result<(), FlowIntervalError> {
        let first_datum_duration = Duration::from_secs(2);
        let maximum_gap_duration = Duration::from_secs(3);
        let first_datum_timeout = FirstDatumTimeout::new(first_datum_duration)?;
        let maximum_datum_gap = MaximumDatumGap::new(maximum_gap_duration)?;
        let expectation = ContinuousFlowExpectation::new(first_datum_timeout, maximum_datum_gap);

        assert_eq!(
            expectation.first_datum_timeout().duration(),
            first_datum_duration
        );
        assert_eq!(
            expectation.maximum_datum_gap().duration(),
            maximum_gap_duration
        );
        Ok(())
    }

    #[test]
    fn flow_is_not_monitored_by_default() {
        assert_eq!(FlowExpectation::default(), FlowExpectation::NotMonitored);
    }

    #[test]
    fn continuous_flow_expectation_preserves_the_full_duration_range()
    -> Result<(), FlowIntervalError> {
        let first_datum_timeout = FirstDatumTimeout::new(Duration::MAX)?;
        let maximum_datum_gap = MaximumDatumGap::new(Duration::MAX)?;
        let expectation = ContinuousFlowExpectation::new(first_datum_timeout, maximum_datum_gap);

        assert_eq!(expectation.first_datum_timeout(), first_datum_timeout);
        assert_eq!(expectation.maximum_datum_gap(), maximum_datum_gap);
        Ok(())
    }

    #[test]
    fn continuous_flow_begins_awaiting_its_first_datum() -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let establishment_offset = Duration::from_secs(5);
        let established_at = runtime_started_at + establishment_offset;
        let first_datum_duration = Duration::from_secs(2);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(first_datum_duration)?,
            MaximumDatumGap::new(Duration::from_secs(3))?,
        );
        let flow = EstablishedFlow::new(FlowExpectation::Continuous(expectation), established_at);

        assert!(flow.is_monitored());
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum {
                deadline: RiggingRuntimeTime::from_elapsed(
                    establishment_offset + first_datum_duration,
                ),
            })
        );
        Ok(())
    }

    #[test]
    fn repeated_arrivals_extend_expiry_without_republishing_flowing()
    -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let first_datum_at = runtime_started_at + Duration::from_secs(1);
        let later_datum_at = runtime_started_at + Duration::from_secs(3);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(Duration::from_secs(2))?,
            MaximumDatumGap::new(Duration::from_secs(4))?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), runtime_started_at);

        flow.record_datum_arrival(first_datum_at);
        let flowing = EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
            flowing_since: RiggingRuntimeTime::from_elapsed(Duration::from_secs(1)),
        });
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            flowing
        );
        flow.record_datum_arrival(later_datum_at);
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            flowing
        );
        assert_eq!(
            flow.evaluate_expiry(later_datum_at + Duration::from_secs(4)),
            EstablishedFlowExpiry::Current
        );
        assert!(!flow.judgment_due(later_datum_at + Duration::from_secs(4)));
        assert!(
            flow.judgment_due(later_datum_at + Duration::from_secs(4) + Duration::from_nanos(1),)
        );
        assert_eq!(
            flow.evaluate_expiry(
                later_datum_at + Duration::from_secs(4) + Duration::from_nanos(1),
            ),
            EstablishedFlowExpiry::Crossed(
                ContinuousFlowExpiryCause::MaximumDatumGapExceeded
            )
        );
        Ok(())
    }

    /// Activity is not a datum, so it cannot be the arrival that starts a session flowing.
    ///
    /// A session held here by activity alone has delivered nothing to present. Letting it reach
    /// `Flowing` is what let a replugged display claim to be presenting a picture that had never
    /// arrived, because every presenting reading is derived from that one state.
    #[test]
    fn transport_activity_cannot_start_flow() -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let timeout = Duration::from_secs(2);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(timeout)?,
            MaximumDatumGap::new(Duration::from_secs(3))?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), runtime_started_at);

        flow.record_transport_activity(runtime_started_at + Duration::from_secs(1));

        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum {
                deadline: RiggingRuntimeTime::from_elapsed(timeout),
            })
        );
        // The first-datum budget still runs from establishment, so activity cannot postpone it.
        assert_eq!(
            flow.evaluate_expiry(runtime_started_at + timeout + Duration::from_nanos(1)),
            EstablishedFlowExpiry::Crossed(ContinuousFlowExpiryCause::FirstDatumOverdue)
        );
        Ok(())
    }

    /// Activity over a delivering session means nothing has changed since its last datum, so that
    /// datum is still current and the session keeps flowing.
    #[test]
    fn transport_activity_extends_a_flowing_session() -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let gap = Duration::from_secs(4);
        let first_datum_at = runtime_started_at + Duration::from_secs(1);
        let activity_at = first_datum_at + Duration::from_secs(3);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(Duration::from_secs(2))?,
            MaximumDatumGap::new(gap)?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), runtime_started_at);

        flow.record_datum_arrival(first_datum_at);
        flow.record_transport_activity(activity_at);

        let flowing = EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
            flowing_since: RiggingRuntimeTime::from_elapsed(Duration::from_secs(1)),
        });
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            flowing
        );
        // The gap now runs from the activity: without it this moment would already have crossed.
        assert!(first_datum_at + gap < activity_at + gap);
        assert_eq!(
            flow.evaluate_expiry(activity_at + gap),
            EstablishedFlowExpiry::Current
        );
        assert_eq!(
            flow.evaluate_expiry(activity_at + gap + Duration::from_nanos(1)),
            EstablishedFlowExpiry::Crossed(ContinuousFlowExpiryCause::MaximumDatumGapExceeded)
        );
        Ok(())
    }

    /// A stalled session is released before it can flow again, so activity changes nothing.
    #[test]
    fn transport_activity_cannot_revive_a_stalled_session() -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let timeout = Duration::from_secs(2);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(timeout)?,
            MaximumDatumGap::new(Duration::from_secs(3))?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), runtime_started_at);
        let crossed_at = runtime_started_at + timeout + Duration::from_nanos(1);
        let stalled = EstablishedFlowView::Continuous(ContinuousFlowView::Stalled {
            since: RiggingRuntimeTime::from_elapsed(timeout),
            cause: ContinuousFlowExpiryCause::FirstDatumOverdue,
        });

        assert_eq!(
            flow.evaluate_expiry(crossed_at),
            EstablishedFlowExpiry::Crossed(ContinuousFlowExpiryCause::FirstDatumOverdue)
        );
        flow.record_transport_activity(crossed_at + Duration::from_nanos(1));

        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            stalled
        );
        Ok(())
    }

    #[test]
    fn first_datum_expiry_crosses_once_then_requires_release() -> Result<(), FlowIntervalError> {
        let runtime_started_at = Instant::now();
        let timeout = Duration::from_secs(2);
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(timeout)?,
            MaximumDatumGap::new(Duration::from_secs(3))?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), runtime_started_at);

        assert_eq!(
            flow.evaluate_expiry(runtime_started_at + timeout),
            EstablishedFlowExpiry::Current
        );
        assert!(!flow.judgment_due(runtime_started_at + timeout));
        assert!(flow.judgment_due(runtime_started_at + timeout + Duration::from_nanos(1),));
        assert_eq!(
            flow.evaluate_expiry(runtime_started_at + timeout + Duration::from_nanos(1)),
            EstablishedFlowExpiry::Crossed(ContinuousFlowExpiryCause::FirstDatumOverdue)
        );
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            EstablishedFlowView::Continuous(ContinuousFlowView::Stalled {
                since: RiggingRuntimeTime::from_elapsed(timeout),
                cause: ContinuousFlowExpiryCause::FirstDatumOverdue,
            })
        );
        assert!(flow.judgment_due(runtime_started_at + timeout + Duration::from_nanos(1),));
        flow.record_datum_arrival(runtime_started_at + timeout + Duration::from_nanos(2));
        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(runtime_started_at)),
            EstablishedFlowView::Continuous(ContinuousFlowView::Stalled {
                since: RiggingRuntimeTime::from_elapsed(timeout),
                cause: ContinuousFlowExpiryCause::FirstDatumOverdue,
            })
        );
        assert_eq!(
            flow.evaluate_expiry(runtime_started_at + timeout + Duration::from_nanos(2)),
            EstablishedFlowExpiry::ReleaseRequired(ContinuousFlowExpiryCause::FirstDatumOverdue)
        );
        Ok(())
    }

    #[test]
    fn unmonitored_flow_never_expires() {
        let runtime_started_at = Instant::now();
        let mut flow = EstablishedFlow::new(FlowExpectation::NotMonitored, runtime_started_at);

        assert!(!flow.is_monitored());
        assert_eq!(
            flow.evaluate_expiry(runtime_started_at + Duration::from_secs(1_000)),
            EstablishedFlowExpiry::Current
        );
        assert!(!flow.judgment_due(runtime_started_at + Duration::from_secs(1_000),));
    }
}
