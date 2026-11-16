//! Shared fixtures for metrics unit tests.

use std::num::NonZeroUsize;
use std::time::Duration;

use bevy_app::App;
use bevy_diagnostic::Diagnostic;
use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::DiagnosticsStore;

use crate::Counter;
use crate::CounterUnit;
use crate::Freshness;
use crate::Gauge;
use crate::GaugeSemantics;
use crate::IntervalSummaryKind;
use crate::ManualRunClock;
use crate::MeasurementCadence;
use crate::MetricDefinition;
use crate::MetricNumber;
use crate::MetricReading;
use crate::MetricSelection;
use crate::MetricStatistic;
use crate::MetricUnit;
use crate::MetricsPlugin;
use crate::MetricsSettings;
use crate::SampleWindow;
use crate::SmoothingPolicy;
use crate::TimingSpan;
use crate::WindowReduction;
use crate::register_metric;

pub(crate) fn interval_definition(metric: &'static str) -> MetricDefinition<Gauge> {
    MetricDefinition::<Gauge>::new(
        path(metric),
        "Interval means",
        MetricUnit::Count,
        MeasurementCadence::Interval(Duration::from_millis(500)),
        Freshness::LiveWithin(Duration::from_secs(1)),
        GaugeSemantics::IntervalSummary(IntervalSummaryKind::Mean),
    )
}

pub(crate) fn metrics_app() -> App { metrics_app_with_settings(MetricsSettings::default()) }

pub(crate) fn metrics_app_with_settings(settings: MetricsSettings) -> App {
    let mut app = App::new();
    app.insert_resource(ManualRunClock::default())
        .insert_resource(settings)
        .add_plugins(MetricsPlugin);
    app
}

pub(crate) fn gauge_definition(metric: &'static str) -> MetricDefinition<Gauge> {
    MetricDefinition::<Gauge>::new(
        path(metric),
        "Level",
        MetricUnit::Count,
        MeasurementCadence::EveryUpdate,
        Freshness::LiveWithin(Duration::from_secs(1)),
        GaugeSemantics::Level,
    )
}

pub(crate) fn path(metric: &'static str) -> DiagnosticPath { DiagnosticPath::const_new(metric) }

pub(crate) fn latest(metric: &'static str) -> MetricSelection {
    MetricSelection {
        path:      path(metric),
        statistic: MetricStatistic::Latest,
    }
}

pub(crate) fn retained(metric: &'static str) -> MetricSelection {
    window_selection(metric, SampleWindow::LastSamples(nonzero(8)))
}

pub(crate) fn diagnostic_paths(app: &App) -> Vec<String> {
    let mut paths = app
        .world()
        .resource::<DiagnosticsStore>()
        .iter()
        .map(|diagnostic| diagnostic.path().as_str().to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

pub(crate) fn values(app: &App, metric: &'static str) -> Vec<f64> {
    let store = app.world().resource::<DiagnosticsStore>();
    let diagnostic = store.get(&path(metric));
    assert!(diagnostic.is_some(), "registered diagnostic");
    diagnostic
        .into_iter()
        .flat_map(Diagnostic::values)
        .copied()
        .collect()
}

pub(crate) fn nonzero(value: usize) -> NonZeroUsize {
    assert_ne!(value, 0, "positive capacity");
    NonZeroUsize::new(value).unwrap_or(NonZeroUsize::MIN)
}

pub(crate) fn window_selection(path: &'static str, window: SampleWindow) -> MetricSelection {
    reduction(path, window, WindowReduction::Mean)
}

pub(crate) fn enabled(app: &App, path: &'static str) -> bool {
    let store = app.world().resource::<DiagnosticsStore>();
    let diagnostic = store.get(&self::path(path));
    assert!(diagnostic.is_some(), "registered diagnostic");
    diagnostic.is_some_and(|diagnostic| diagnostic.is_enabled)
}

/// Reads through the public system parameter without acquiring demand.
pub(crate) fn read(
    app: &mut App,
    subscription: &crate::MetricSubscription,
    selection: &MetricSelection,
) -> Result<crate::MetricReading, Box<dyn std::error::Error>> {
    let mut state = bevy_ecs::system::SystemState::<crate::MetricReader>::new(app.world_mut());
    Ok(state.get(app.world())?.read(subscription, selection))
}

/// Takes an immutable series snapshot through the public system parameter.
pub(crate) fn series(
    app: &mut App,
    subscription: &crate::MetricSubscription,
    metric: &'static str,
    window: &SampleWindow,
) -> Result<crate::MetricSeries, Box<dyn std::error::Error>> {
    let mut state = bevy_ecs::system::SystemState::<crate::MetricReader>::new(app.world_mut());
    Ok(state
        .get(app.world())?
        .series(subscription, &path(metric), window))
}

/// Extracts an available estimate, preserving unexpected states in the failure.
pub(crate) fn available(
    reading: crate::MetricReading,
) -> Result<crate::MetricEstimate, Box<dyn std::error::Error>> {
    match reading {
        MetricReading::Available(estimate) => Ok(estimate),
        other => Err(format!("expected an available estimate, got {other:?}").into()),
    }
}

/// Extracts a scalar without silently rounding an exact count.
pub(crate) fn scalar(estimate: &crate::MetricEstimate) -> Result<f64, Box<dyn std::error::Error>> {
    match estimate.value {
        MetricNumber::Scalar(value) => Ok(value),
        other @ MetricNumber::Count(_) => Err(format!("expected a scalar, got {other:?}").into()),
    }
}

/// Builds a selection for a specific reduction over bounded retained samples.
pub(crate) fn reduction(
    metric: &'static str,
    window: SampleWindow,
    reduction: WindowReduction,
) -> MetricSelection {
    MetricSelection {
        path:      path(metric),
        statistic: MetricStatistic::Window { window, reduction },
    }
}

/// Inspects catalogue metadata without acquiring demand.
pub(crate) fn with_catalogue<R>(
    app: &mut App,
    inspect: impl FnOnce(crate::MetricCatalogue<'_>) -> Result<R, Box<dyn std::error::Error>>,
) -> Result<R, Box<dyn std::error::Error>> {
    let mut state = bevy_ecs::system::SystemState::<crate::MetricCatalogue>::new(app.world_mut());
    inspect(state.get(app.world())?)
}

/// Registers the declarations exercised by the descriptor capability table.
pub(crate) fn descriptor_app() -> Result<App, Box<dyn std::error::Error>> {
    let mut app = metrics_app();
    for (metric, unit, semantics) in [
        ("test/level", MetricUnit::Points, GaugeSemantics::Level),
        (
            "test/additive",
            MetricUnit::Count,
            GaugeSemantics::AdditiveSample,
        ),
        (
            "test/mean",
            MetricUnit::Microseconds,
            GaugeSemantics::IntervalSummary(IntervalSummaryKind::Mean),
        ),
        (
            "test/rate",
            MetricUnit::Hertz,
            GaugeSemantics::IntervalSummary(IntervalSummaryKind::Rate),
        ),
    ] {
        register_metric(
            &mut app,
            MetricDefinition::<Gauge>::new(
                path(metric),
                "Gauge",
                unit,
                MeasurementCadence::EveryUpdate,
                Freshness::CompletedEvent,
                semantics,
            ),
        )?;
    }
    register_metric(
        &mut app,
        gauge_definition("test/smoothed")
            .with_smoothing(SmoothingPolicy::fixed_alpha(0.2, Duration::from_secs(5))?),
    )?;
    register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(path("test/count"), "Count", CounterUnit::Count),
    )?;
    register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            path("test/bytes"),
            "Bytes",
            CounterUnit::Bytes,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )?;
    register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(path("test/span"), "Span", Freshness::CompletedEvent),
    )?;
    Ok(app)
}
