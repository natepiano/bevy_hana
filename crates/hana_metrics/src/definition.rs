//! Metric metadata, sample meaning, validated policies and saved selections.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::time::Duration;

use bevy_diagnostic::DiagnosticPath;
use bevy_reflect::Reflect;
use bevy_reflect::ReflectDeserialize;
use bevy_reflect::ReflectSerialize;
use sealed::Sealed;
use serde::Deserialize;
use serde::Serialize;

use crate::ObservationInterval;
use crate::SampleWindow;
use crate::WindowReduction;

mod sealed {
    pub trait Sealed {}
}

/// Finite scalar observations with explicitly declared sample meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Gauge;

/// Exact nonnegative increments or externally maintained cumulative totals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Counter;

/// Completed work durations stored in seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimingSpan;

/// Closed set of metric kinds accepted by typed recorders.
pub trait MetricKind: sealed::Sealed + Send + Sync + 'static {}

impl Sealed for Gauge {}
impl Sealed for Counter {}
impl Sealed for TimingSpan {}
impl MetricKind for Gauge {}
impl MetricKind for Counter {}
impl MetricKind for TimingSpan {}

/// Physical or semantic unit of a metric reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricUnit {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
    /// Microseconds.
    Microseconds,
    /// Dimensionless exact or sampled count.
    Count,
    /// Bytes.
    Bytes,
    /// Bytes per second.
    BytesPerSecond,
    /// Decimal megabytes per second, with one MB equal to one million bytes.
    DecimalMegabytesPerSecond,
    /// Events per second.
    Hertz,
    /// Count per second.
    CountPerSecond,
    /// Dimensionless ratio.
    Ratio,
    /// Contrast ratio.
    ContrastRatio,
    /// Logical layout points.
    Points,
    /// Source has not declared a unit.
    Unspecified,
}

impl MetricUnit {
    /// Whether this unit already expresses a rate.
    #[must_use]
    pub const fn is_rate(self) -> bool {
        matches!(
            self,
            Self::BytesPerSecond
                | Self::DecimalMegabytesPerSecond
                | Self::Hertz
                | Self::CountPerSecond
        )
    }
}

/// Units supported by exact nonnegative counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CounterUnit {
    /// Number of events or objects.
    Count,
    /// Number of bytes.
    Bytes,
}

impl From<CounterUnit> for MetricUnit {
    fn from(unit: CounterUnit) -> Self {
        match unit {
            CounterUnit::Count => Self::Count,
            CounterUnit::Bytes => Self::Bytes,
        }
    }
}

/// Declared production schedule; interval summaries still carry actual endpoints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementCadence {
    /// One production opportunity per update.
    EveryUpdate,
    /// Production follows source events.
    OnEvent,
    /// Nominal source interval; zero is invalid at registration.
    Interval(Duration),
}

/// How a reading's age affects its availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    /// A live reading is fresh for this duration after measurement.
    LiveWithin(Duration),
    /// A completed event remains an event regardless of later inactivity.
    CompletedEvent,
}

/// Meaning of one scalar gauge observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GaugeSemantics {
    /// Instantaneous level; summing levels has no declared meaning.
    Level,
    /// Individual observations may be summed across retained samples.
    AdditiveSample,
    /// Each value summarizes its own actual source interval.
    IntervalSummary(IntervalSummaryKind),
}

/// Meaning of an interval-summary gauge value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntervalSummaryKind {
    /// Mean over a source interval; distributions describe interval means.
    Mean,
    /// Rate over a source interval, retaining the declared rate unit.
    Rate,
}

/// Gauge meaning exposed by descriptors of every metric kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GaugeMeaning {
    /// This metric is not a gauge.
    NotAGauge,
    /// The gauge's explicitly declared sample semantics.
    Gauge(GaugeSemantics),
}

/// Runtime metric kind displayed in a catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKindName {
    /// Scalar gauge.
    Gauge,
    /// Exact counter.
    Counter,
    /// Completed work duration in seconds.
    TimingSpan,
    /// Explicitly adopted native diagnostic.
    Native,
}

/// History capability available from the underlying source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricCapability {
    /// Only the latest value and declared smoothing are available.
    LatestOnly,
    /// Exact bounded retained-sample windows are available.
    Retained,
}

/// Statistic families supported by a descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatisticKind {
    /// Latest accepted observation.
    Latest,
    /// Result of the declared smoothing policy.
    Smoothed,
    /// Arithmetic mean of retained observations.
    Mean,
    /// Median of retained observations.
    Median,
    /// Largest retained observation.
    Maximum,
    /// Sum of additive observations.
    Sum,
    /// Exact counter change from a measured baseline.
    Delta,
    /// Counter change per second from a measured baseline.
    Rate,
    /// Exact percentile of retained observations.
    Percentile,
}

/// Validated smoothing algorithm and reset interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothingPolicy {
    method: SmoothingMethod,
}

/// Declared smoothing algorithm; enabled variants are constructed by a policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmoothingMethod {
    /// No derived smoothed reading.
    Disabled,
    /// Fixed weight applied to each accepted sample.
    FixedAlpha {
        /// Finite weight in `(0, 1]`.
        alpha:       f64,
        /// Gaps greater than this duration reseed the filter.
        reset_after: Duration,
    },
    /// Weight derived from elapsed time between accepted samples.
    TimeBased {
        /// Nonzero exponential time constant.
        time_constant: Duration,
        /// Gaps greater than this duration reseed the filter.
        reset_after:   Duration,
    },
}

impl Default for SmoothingPolicy {
    fn default() -> Self { Self::disabled() }
}

impl SmoothingPolicy {
    /// Declares no smoothing.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            method: SmoothingMethod::Disabled,
        }
    }

    /// Validates fixed-alpha smoothing and its reseed interval.
    ///
    /// # Errors
    /// Rejects non-finite alpha, alpha outside `(0, 1]`, or a zero reset interval.
    pub fn fixed_alpha(alpha: f64, reset_after: Duration) -> Result<Self, InvalidSmoothingPolicy> {
        if !alpha.is_finite() {
            return Err(InvalidSmoothingPolicy::NonFiniteAlpha);
        }
        if alpha <= 0.0 || alpha > 1.0 {
            return Err(InvalidSmoothingPolicy::AlphaOutOfRange);
        }
        if reset_after.is_zero() {
            return Err(InvalidSmoothingPolicy::ZeroResetInterval);
        }
        Ok(Self {
            method: SmoothingMethod::FixedAlpha { alpha, reset_after },
        })
    }

    /// Validates time-based exponential smoothing and its reseed interval.
    ///
    /// # Errors
    /// Rejects a zero time constant or reset interval.
    pub const fn time_based(
        time_constant: Duration,
        reset_after: Duration,
    ) -> Result<Self, InvalidSmoothingPolicy> {
        if time_constant.is_zero() {
            return Err(InvalidSmoothingPolicy::ZeroTimeConstant);
        }
        if reset_after.is_zero() {
            return Err(InvalidSmoothingPolicy::ZeroResetInterval);
        }
        Ok(Self {
            method: SmoothingMethod::TimeBased {
                time_constant,
                reset_after,
            },
        })
    }

    /// Returns the algorithm declared by this validated policy.
    #[must_use]
    pub const fn method(&self) -> SmoothingMethod { self.method }

    /// Whether this policy enables a smoothed reading.
    #[must_use]
    pub const fn is_enabled(&self) -> bool { !matches!(self.method, SmoothingMethod::Disabled) }
}

/// Finite interval-summary value paired with actual source endpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntervalGaugeSample {
    value:    f64,
    interval: ObservationInterval,
}

impl IntervalGaugeSample {
    /// Validates an interval-summary value.
    ///
    /// # Errors
    /// Rejects non-finite values.
    pub const fn new(
        value: f64,
        interval: ObservationInterval,
    ) -> Result<Self, InvalidMetricSample> {
        if !value.is_finite() {
            return Err(InvalidMetricSample::NonFinite);
        }
        Ok(Self { value, interval })
    }

    /// Returns the finite summary value.
    #[must_use]
    pub const fn value(self) -> f64 { self.value }

    /// Returns the actual source interval, not nominal cadence.
    #[must_use]
    pub const fn interval(self) -> ObservationInterval { self.interval }
}

/// Source-owned reset identity for an externally maintained counter.
///
/// Advance the generation on reset or source replacement. It must not wrap
/// within a run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ExternalCounterEpoch(u64);

impl ExternalCounterEpoch {
    /// Creates an epoch from the source's reset generation.
    #[must_use]
    pub const fn new(source_generation: u64) -> Self { Self(source_generation) }

    /// Returns the source-owned reset generation.
    #[must_use]
    pub const fn generation(self) -> u64 { self.0 }
}

/// An exact external total, including its source's reset identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CumulativeCounterSample {
    /// Source-owned reset identity.
    pub epoch: ExternalCounterEpoch,
    /// Exact cumulative source total; a nonzero first snapshot is retained.
    pub total: u64,
}

/// Recording contract for an exact counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CounterRecording {
    /// Each call adds an observed increment.
    Increment,
    /// Each call supplies an externally maintained total.
    Cumulative,
}

/// Recording contract, with a mode only for exact counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetricRecording {
    Gauge,
    Counter(CounterRecording),
    TimingSpan,
}

/// Typed declaration validated before registration mutates the application.
#[derive(Clone, Debug)]
pub struct MetricDefinition<K: MetricKind> {
    descriptor: MetricDescriptor,
    recording:  MetricRecording,
    marker:     PhantomData<K>,
}

impl<K: MetricKind> MetricDefinition<K> {
    const fn from_descriptor(descriptor: MetricDescriptor, recording: MetricRecording) -> Self {
        Self {
            descriptor,
            recording,
            marker: PhantomData,
        }
    }

    pub(crate) fn describe(&self) -> Result<MetricDescriptor, MetricRegistrationError> {
        validate_metric_path(self.descriptor.path())?;
        self.describe_metadata()
    }

    fn describe_metadata(&self) -> Result<MetricDescriptor, MetricRegistrationError> {
        let descriptor = &self.descriptor;
        if descriptor.label.trim().is_empty() {
            return Err(MetricRegistrationError::InvalidLabel);
        }
        if matches!(descriptor.cadence, MeasurementCadence::Interval(duration) if duration.is_zero())
        {
            return Err(MetricRegistrationError::InvalidCadence);
        }
        if descriptor.gauge_semantics
            == GaugeMeaning::Gauge(GaugeSemantics::IntervalSummary(IntervalSummaryKind::Rate))
            && !descriptor.unit.is_rate()
        {
            return Err(MetricRegistrationError::IncompatibleUnitSemantics);
        }
        Ok(descriptor.clone())
    }

    pub(crate) const fn recording(&self) -> MetricRecording { self.recording }
}

impl MetricDefinition<Gauge> {
    /// Declares a finite gauge and the meaning of each observation.
    #[must_use]
    pub fn new(
        path: DiagnosticPath,
        label: &'static str,
        unit: MetricUnit,
        cadence: MeasurementCadence,
        freshness: Freshness,
        semantics: GaugeSemantics,
    ) -> Self {
        Self::from_descriptor(
            MetricDescriptor {
                path,
                label,
                unit,
                kind: MetricKindName::Gauge,
                cadence,
                freshness,
                capability: MetricCapability::Retained,
                gauge_semantics: GaugeMeaning::Gauge(semantics),
                smoothing: SmoothingPolicy::disabled(),
                supported_statistics: gauge_statistics(semantics),
            },
            MetricRecording::Gauge,
        )
    }

    /// Declares the derived filter to update from accepted observations.
    #[must_use]
    pub fn with_smoothing(mut self, policy: SmoothingPolicy) -> Self {
        self.descriptor.set_smoothing(policy);
        self
    }
}

impl MetricDefinition<Counter> {
    /// Declares observed increments, initially dormant.
    #[must_use]
    pub fn new(path: DiagnosticPath, label: &'static str, unit: CounterUnit) -> Self {
        Self::with_recording(
            path,
            label,
            unit,
            MeasurementCadence::OnEvent,
            Freshness::CompletedEvent,
            CounterRecording::Increment,
        )
    }

    /// Declares source-owned totals, including a nonzero first snapshot.
    #[must_use]
    pub fn cumulative(
        path: DiagnosticPath,
        label: &'static str,
        unit: CounterUnit,
        cadence: MeasurementCadence,
        freshness: Freshness,
    ) -> Self {
        Self::with_recording(
            path,
            label,
            unit,
            cadence,
            freshness,
            CounterRecording::Cumulative,
        )
    }

    fn with_recording(
        path: DiagnosticPath,
        label: &'static str,
        unit: CounterUnit,
        cadence: MeasurementCadence,
        freshness: Freshness,
        recording: CounterRecording,
    ) -> Self {
        Self::from_descriptor(
            MetricDescriptor {
                path,
                label,
                unit: unit.into(),
                kind: MetricKindName::Counter,
                cadence,
                freshness,
                capability: MetricCapability::Retained,
                gauge_semantics: GaugeMeaning::NotAGauge,
                smoothing: SmoothingPolicy::disabled(),
                supported_statistics: vec![
                    StatisticKind::Latest,
                    StatisticKind::Delta,
                    StatisticKind::Rate,
                ],
            },
            MetricRecording::Counter(recording),
        )
    }
}

impl MetricDefinition<TimingSpan> {
    /// Declares completed work durations stored in seconds.
    #[must_use]
    pub fn new(path: DiagnosticPath, label: &'static str, freshness: Freshness) -> Self {
        let statistics = gauge_statistics(GaugeSemantics::AdditiveSample);
        Self::from_descriptor(
            MetricDescriptor {
                path,
                label,
                unit: MetricUnit::Seconds,
                kind: MetricKindName::TimingSpan,
                cadence: MeasurementCadence::OnEvent,
                freshness,
                capability: MetricCapability::Retained,
                gauge_semantics: GaugeMeaning::NotAGauge,
                smoothing: SmoothingPolicy::disabled(),
                supported_statistics: statistics,
            },
            MetricRecording::TimingSpan,
        )
    }

    /// Declares the derived filter to update from accepted observations.
    #[must_use]
    pub fn with_smoothing(mut self, policy: SmoothingPolicy) -> Self {
        self.descriptor.set_smoothing(policy);
        self
    }
}

fn gauge_statistics(semantics: GaugeSemantics) -> Vec<StatisticKind> {
    let mut statistics = vec![
        StatisticKind::Latest,
        StatisticKind::Mean,
        StatisticKind::Median,
        StatisticKind::Maximum,
        StatisticKind::Percentile,
    ];
    if semantics == GaugeSemantics::AdditiveSample {
        statistics.push(StatisticKind::Sum);
    }
    statistics
}

/// Metadata explicitly adopting an existing native diagnostic without replacing it.
#[derive(Clone, Debug)]
pub struct NativeDiagnosticDescription {
    definition: MetricDefinition<Gauge>,
}

impl NativeDiagnosticDescription {
    /// Declares the native scalar's unit and sample meaning.
    #[must_use]
    pub fn new(
        path: DiagnosticPath,
        label: &'static str,
        unit: MetricUnit,
        semantics: GaugeSemantics,
    ) -> Self {
        Self {
            definition: MetricDefinition::<Gauge>::new(
                path,
                label,
                unit,
                MeasurementCadence::EveryUpdate,
                Freshness::CompletedEvent,
                semantics,
            ),
        }
    }

    pub(crate) fn describe(&self) -> Result<MetricDescriptor, MetricRegistrationError> {
        validate_native_path(self.definition.descriptor.path().as_str())?;
        let mut descriptor = self.definition.describe_metadata()?;
        descriptor.kind = MetricKindName::Native;
        descriptor.capability = MetricCapability::LatestOnly;
        descriptor
            .supported_statistics
            .retain(|statistic| matches!(statistic, StatisticKind::Latest));
        Ok(descriptor)
    }
}

/// Immutable metadata validated during metric registration.
#[derive(Clone, Debug)]
pub struct MetricDescriptor {
    path:                 DiagnosticPath,
    label:                &'static str,
    unit:                 MetricUnit,
    kind:                 MetricKindName,
    cadence:              MeasurementCadence,
    freshness:            Freshness,
    capability:           MetricCapability,
    gauge_semantics:      GaugeMeaning,
    smoothing:            SmoothingPolicy,
    supported_statistics: Vec<StatisticKind>,
}

impl MetricDescriptor {
    pub(crate) fn discovered(path: DiagnosticPath) -> Self {
        Self {
            path,
            label: "Native diagnostic",
            unit: MetricUnit::Unspecified,
            kind: MetricKindName::Native,
            cadence: MeasurementCadence::EveryUpdate,
            freshness: Freshness::CompletedEvent,
            capability: MetricCapability::LatestOnly,
            gauge_semantics: GaugeMeaning::Gauge(GaugeSemantics::Level),
            smoothing: SmoothingPolicy::disabled(),
            supported_statistics: vec![StatisticKind::Latest],
        }
    }

    fn set_smoothing(&mut self, policy: SmoothingPolicy) {
        self.smoothing = policy;
        self.supported_statistics
            .retain(|statistic| *statistic != StatisticKind::Smoothed);
        if policy.is_enabled() {
            self.supported_statistics.push(StatisticKind::Smoothed);
        }
    }

    /// Returns the stable series identity.
    #[must_use]
    pub const fn path(&self) -> &DiagnosticPath { &self.path }

    /// Returns the presentation label.
    #[must_use]
    pub const fn label(&self) -> &str { self.label }

    /// Returns the base observation unit.
    #[must_use]
    pub const fn unit(&self) -> MetricUnit { self.unit }

    /// Returns the registered kind.
    #[must_use]
    pub const fn kind(&self) -> MetricKindName { self.kind }

    /// Returns the declared production schedule.
    #[must_use]
    pub const fn cadence(&self) -> MeasurementCadence { self.cadence }

    /// Returns the reading freshness contract.
    #[must_use]
    pub const fn freshness(&self) -> Freshness { self.freshness }

    /// Returns the source's history capability.
    #[must_use]
    pub const fn capability(&self) -> MetricCapability { self.capability }

    /// Returns the explicitly declared gauge meaning, if this is a gauge.
    #[must_use]
    pub const fn gauge_semantics(&self) -> GaugeMeaning { self.gauge_semantics }

    /// Returns the validated smoothing declaration.
    #[must_use]
    pub const fn smoothing(&self) -> &SmoothingPolicy { &self.smoothing }

    /// Returns statistic families supported by both declaration and source.
    #[must_use]
    pub fn supported_statistics(&self) -> &[StatisticKind] { &self.supported_statistics }

    /// Validates a statistic and returns its resulting unit.
    ///
    /// # Errors
    /// Rejects disabled smoothing, a window on a latest-only source, or a
    /// statistic not supported by this declaration.
    pub fn result_unit(
        &self,
        statistic: &MetricStatistic,
    ) -> Result<MetricUnit, MetricCapabilityFailure> {
        let kind = statistic.kind();
        if kind == StatisticKind::Smoothed && !self.smoothing.is_enabled() {
            return Err(MetricCapabilityFailure::SmoothingDisabled);
        }
        if matches!(statistic, MetricStatistic::Window { .. })
            && self.capability == MetricCapability::LatestOnly
        {
            return Err(MetricCapabilityFailure::LatestOnlySource);
        }
        if !self.supported_statistics.contains(&kind) {
            return Err(MetricCapabilityFailure::StatisticNotSupported);
        }
        if kind == StatisticKind::Rate {
            return match (self.kind, self.unit) {
                (MetricKindName::Counter, MetricUnit::Count) => Ok(MetricUnit::CountPerSecond),
                (MetricKindName::Counter, MetricUnit::Bytes) => Ok(MetricUnit::BytesPerSecond),
                _ => Err(MetricCapabilityFailure::StatisticNotSupported),
            };
        }
        Ok(self.unit)
    }
}

/// Saved selection identified by the stable path string, never a runtime handle.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSelection {
    /// Stable series path; saved native paths carry their grammar provenance.
    pub path:      DiagnosticPath,
    /// Requested statistic and any exact retained window.
    pub statistic: MetricStatistic,
}

/// A latest, smoothed or retained-window reading request.
#[derive(Clone, Copy, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(opaque, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MetricStatistic {
    /// Latest accepted observation.
    Latest,
    /// Derived value from the registered smoothing policy.
    Smoothed,
    /// Reduction over a bounded retained-sample window.
    Window {
        /// Retained samples requested by this reading.
        window:    SampleWindow,
        /// Reduction permitted by the metric's declaration.
        reduction: WindowReduction,
    },
}

impl MetricStatistic {
    /// Returns the statistic family used in descriptor capability checks.
    #[must_use]
    pub const fn kind(&self) -> StatisticKind {
        match self {
            Self::Latest => StatisticKind::Latest,
            Self::Smoothed => StatisticKind::Smoothed,
            Self::Window { reduction, .. } => match reduction {
                WindowReduction::Mean => StatisticKind::Mean,
                WindowReduction::Median => StatisticKind::Median,
                WindowReduction::Maximum => StatisticKind::Maximum,
                WindowReduction::Sum => StatisticKind::Sum,
                WindowReduction::Delta => StatisticKind::Delta,
                WindowReduction::Rate => StatisticKind::Rate,
                WindowReduction::Percentile(_) => StatisticKind::Percentile,
            },
        }
    }
}

/// Validates a registered path in every build profile.
///
/// A path has at least two slash-separated segments. Each segment starts with
/// an ASCII lowercase letter and contains only lowercase letters, digits or `_`.
///
/// # Errors
/// Rejects paths outside this grammar.
pub fn validate_metric_path(path: &DiagnosticPath) -> Result<(), MetricRegistrationError> {
    validate_path_string(path.as_str())
}

fn validate_path_string(path: &str) -> Result<(), MetricRegistrationError> {
    let mut segments = 0;
    for segment in path.split('/') {
        let mut bytes = segment.bytes();
        if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(MetricRegistrationError::InvalidPath);
        }
        segments += 1;
    }
    if segments < 2 {
        return Err(MetricRegistrationError::InvalidPath);
    }
    Ok(())
}

pub(crate) fn validate_native_path(path: &str) -> Result<(), MetricRegistrationError> {
    if path.split('/').any(str::is_empty) {
        Err(MetricRegistrationError::InvalidPath)
    } else {
        Ok(())
    }
}

mod selection_path {
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;
    use serde::de::Error as _;
    use serde::ser::Error as _;
    use serde::ser::SerializeStruct;

    use super::MetricSelection;
    use super::MetricStatistic;
    use super::validate_native_path;
    use super::validate_path_string;

    // Wire provenance selects a validating grammar; it never acquires demand.
    #[derive(Deserialize)]
    struct SavedSelection {
        path:      String,
        statistic: MetricStatistic,
        #[serde(default)]
        native:    bool,
    }

    impl Serialize for MetricSelection {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            validate_native_path(self.path.as_str()).map_err(S::Error::custom)?;
            let native = validate_path_string(self.path.as_str()).is_err();
            let mut saved =
                serializer.serialize_struct("MetricSelection", if native { 3 } else { 2 })?;
            saved.serialize_field("path", self.path.as_str())?;
            saved.serialize_field("statistic", &self.statistic)?;
            if native {
                saved.serialize_field("native", &true)?;
            }
            saved.end()
        }
    }

    impl<'de> Deserialize<'de> for MetricSelection {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let saved = SavedSelection::deserialize(deserializer)?;
            if saved.native {
                validate_native_path(&saved.path)
            } else {
                validate_path_string(&saved.path)
            }
            .map_err(D::Error::custom)?;
            Ok(Self {
                path:      bevy_diagnostic::DiagnosticPath::new(saved.path),
                statistic: saved.statistic,
            })
        }
    }
}

/// Why a requested statistic cannot be supplied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricCapabilityFailure {
    /// The declaration does not support this statistic.
    StatisticNotSupported,
    /// No smoothing policy was enabled.
    SmoothingDisabled,
    /// The source cannot supply retained-sample windows.
    LatestOnlySource,
    /// The requested window exceeds retained evidence.
    RetentionExceeded,
}

impl Display for MetricCapabilityFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StatisticNotSupported => "statistic is not supported by the declaration",
            Self::SmoothingDisabled => "smoothing is disabled",
            Self::LatestOnlySource => "source supplies latest readings only",
            Self::RetentionExceeded => "requested window exceeds retained evidence",
        })
    }
}

impl Error for MetricCapabilityFailure {}

/// Why a metric registration cannot be installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricRegistrationError {
    /// The stable path violates the registered-path grammar.
    InvalidPath,
    /// The path has already been registered, even if metadata matches.
    DuplicatePath,
    /// The metrics service has not been installed in the app.
    ServiceNotInstalled,
    /// The declared nominal interval is zero.
    InvalidCadence,
    /// The unit does not match the declared kind or sample meaning.
    IncompatibleUnitSemantics,
    /// The smoothing declaration is invalid or incompatible.
    InvalidSmoothing,
    /// The presentation label is empty or contains only whitespace.
    InvalidLabel,
    /// Native adoption requires an existing diagnostic.
    NativeDiagnosticMissing,
}

impl Display for MetricRegistrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPath => "invalid metric path",
            Self::DuplicatePath => "metric path is already registered",
            Self::ServiceNotInstalled => "metrics service is not installed",
            Self::InvalidCadence => "measurement interval must be nonzero",
            Self::IncompatibleUnitSemantics => "unit is incompatible with metric semantics",
            Self::InvalidSmoothing => "invalid smoothing declaration",
            Self::InvalidLabel => "metric label must not be empty",
            Self::NativeDiagnosticMissing => "native diagnostic is not installed",
        })
    }
}

impl Error for MetricRegistrationError {}

/// Why a smoothing policy cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidSmoothingPolicy {
    /// The fixed alpha is not finite.
    NonFiniteAlpha,
    /// The fixed alpha is outside `(0, 1]`.
    AlphaOutOfRange,
    /// The exponential time constant is zero.
    ZeroTimeConstant,
    /// The filter reset interval is zero.
    ZeroResetInterval,
}

impl Display for InvalidSmoothingPolicy {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteAlpha => "smoothing alpha must be finite",
            Self::AlphaOutOfRange => "smoothing alpha must be in (0, 1]",
            Self::ZeroTimeConstant => "smoothing time constant must be nonzero",
            Self::ZeroResetInterval => "smoothing reset interval must be nonzero",
        })
    }
}

impl Error for InvalidSmoothingPolicy {}

/// Why an observation cannot contribute complete evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidMetricSample {
    /// The scalar value is NaN or infinite.
    NonFinite,
    /// The actual source interval is missing.
    MissingInterval,
    /// The operation does not match the registered sample meaning.
    WrongRecordingMethod,
    /// The counter overflowed within its epoch.
    CounterOverflow,
    /// A cumulative total fell within an unchanged source epoch.
    FallingCounterTotal,
    /// A lost sequence leaves incomplete observation evidence.
    LostSequence,
    /// A clock discontinuity prevents temporal attribution.
    ClockDiscontinuity,
    /// The observed operation did not complete.
    AbortedRecord,
}

impl Display for InvalidMetricSample {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "metric value must be finite",
            Self::MissingInterval => "missing source interval",
            Self::WrongRecordingMethod => "recording method does not match the declaration",
            Self::CounterOverflow => "counter overflow invalidates the epoch",
            Self::FallingCounterTotal => "counter total fell within the same epoch",
            Self::LostSequence => "observation sequence was lost",
            Self::ClockDiscontinuity => "observation clock is discontinuous",
            Self::AbortedRecord => "observation was aborted",
        })
    }
}

impl Error for InvalidMetricSample {}

/// Why a percentile fraction cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidPercentile {
    /// The fraction is NaN or infinite.
    NonFinite,
    /// The fraction is outside the inclusive range `[0, 1]`.
    OutOfRange,
}

impl Display for InvalidPercentile {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "percentile fraction must be finite",
            Self::OutOfRange => "percentile fraction must be in [0, 1]",
        })
    }
}

impl Error for InvalidPercentile {}

/// Why source endpoints do not form a valid interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidObservationInterval {
    /// The end precedes the start.
    ReversedEndpoints,
}

impl Display for InvalidObservationInterval {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReversedEndpoints => "source interval ends before it starts",
        })
    }
}

impl Error for InvalidObservationInterval {}
