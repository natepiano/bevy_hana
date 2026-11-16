//! Private-state assertions for registration, quality, smoothing and temporal evidence.
#![allow(
    clippy::expect_used,
    reason = "tests stop when valid fixtures or diagnostics are absent"
)]

use std::cell::Cell;
use std::time::Duration;

use bevy_app::App;
use bevy_app::Update;
use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::DiagnosticsPlugin;
use bevy_diagnostic::DiagnosticsStore;
use bevy_ecs::world::World;

use super::FilterState;
use super::MetricRegistry;
use super::MetricSource;
use super::RetainedObservation;
use crate::Counter;
use crate::CounterUnit;
use crate::CumulativeCounterSample;
use crate::ExternalCounterEpoch;
use crate::Freshness;
use crate::Gauge;
use crate::GaugeSemantics;
use crate::IntervalGaugeSample;
use crate::IntervalSummaryKind;
use crate::InvalidMetricSample;
use crate::ManualRunClock;
use crate::MeasurementCadence;
use crate::MeasurementFailure;
use crate::MetricDefinition;
use crate::MetricDescriptor;
use crate::MetricNumber;
use crate::MetricQuality;
use crate::MetricSelection;
use crate::MetricStatistic;
use crate::MetricUnit;
use crate::MetricsPlugin;
use crate::ObservationExtent;
use crate::ObservationInterval;
use crate::RunOffset;
use crate::SmoothingPolicy;
use crate::TimingSpan;
use crate::register_metric;
use crate::subscribe_metrics;
use crate::test_support;

#[test]
fn recording_invalid_registration_leaves_no_partial_diagnostic_or_demand() {
    let mut app = test_support::metrics_app();
    register_metric(&mut app, test_support::gauge_definition("test/existing"))
        .expect("existing metric");
    let before = test_support::diagnostic_paths(&app);
    for invalid in [
        test_support::gauge_definition("one_segment"),
        test_support::gauge_definition("test//level"),
        test_support::gauge_definition("test/Uppercase"),
        test_support::gauge_definition("test/123"),
        MetricDefinition::<Gauge>::new(
            test_support::path("test/zero_cadence"),
            "Invalid cadence",
            MetricUnit::Count,
            MeasurementCadence::Interval(Duration::ZERO),
            Freshness::CompletedEvent,
            GaugeSemantics::Level,
        ),
        MetricDefinition::<Gauge>::new(
            test_support::path("test/rate_with_count_unit"),
            "Invalid rate unit",
            MetricUnit::Count,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
            GaugeSemantics::IntervalSummary(IntervalSummaryKind::Rate),
        ),
    ] {
        assert!(register_metric(&mut app, invalid).is_err());
        assert_eq!(test_support::diagnostic_paths(&app), before);
        assert_eq!(
            descriptor(app.world(), &test_support::path("test/existing"))
                .expect("existing descriptor survives rejected registration")
                .label(),
            "Level"
        );
    }
    for rejected in [
        "one_segment",
        "test//level",
        "test/Uppercase",
        "test/123",
        "test/zero_cadence",
        "test/rate_with_count_unit",
    ] {
        assert!(
            descriptor(app.world(), &test_support::path(rejected)).is_none(),
            "failed registration inserted descriptor {rejected}"
        );
    }
    for rejected in ["test/zero_cadence", "test/rate_with_count_unit"] {
        assert!(subscribe_metrics(app.world_mut(), &[test_support::latest(rejected)]).is_err());
        assert!(
            register_metric(&mut app, test_support::gauge_definition(rejected)).is_ok(),
            "failed registration reserved {rejected}"
        );
    }
}

#[test]
fn recording_duplicate_registration_keeps_original_history_and_recorder() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))
        .expect("first registration");
    let _subscription = subscribe_metrics(app.world_mut(), &[test_support::retained("test/level")])
        .expect("subscribe to the original metric");
    app.update();
    recorder.record_with(|| 3.0);
    app.update();
    let before = test_support::diagnostic_paths(&app);

    assert!(register_metric(&mut app, test_support::gauge_definition("test/level")).is_err());
    assert!(
        register_metric(
            &mut app,
            MetricDefinition::<Counter>::new(
                test_support::path("test/level"),
                "Replacement",
                CounterUnit::Bytes
            ),
        )
        .is_err()
    );
    assert_eq!(test_support::diagnostic_paths(&app), before);
    let original =
        descriptor(app.world(), &test_support::path("test/level")).expect("original descriptor");
    assert_eq!(original.label(), "Level");
    assert_eq!(original.unit(), MetricUnit::Count);
    assert_eq!(original.kind(), crate::MetricKindName::Gauge);
    assert_eq!(test_support::values(&app, "test/level"), [3.0]);
    recorder.record_with(|| 9.0);
    app.update();
    assert_eq!(test_support::values(&app, "test/level"), [3.0, 9.0]);
}

#[test]
fn recording_wrong_counter_methods_mark_quality_incomplete_without_publishing() {
    let mut app = test_support::metrics_app();
    let increments = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/increments"),
            "Events",
            CounterUnit::Count,
        ),
    )
    .expect("increment counter");
    let totals = register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            test_support::path("test/totals"),
            "External total",
            CounterUnit::Bytes,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )
    .expect("cumulative counter");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[
            test_support::latest("test/increments"),
            test_support::latest("test/totals"),
        ],
    )
    .expect("subscribe to both counters");
    app.update();
    increments.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 900,
    });
    totals.increment_with(|| 11);
    app.update();

    for metric in ["test/increments", "test/totals"] {
        assert_eq!(
            quality(app.world(), &test_support::path(metric)),
            Some(MetricQuality::Incomplete {
                invalid_samples: 1,
                first_failure:   MeasurementFailure::InvalidSample(
                    InvalidMetricSample::WrongRecordingMethod
                ),
            })
        );
        assert!(
            test_support::values(&app, metric).is_empty(),
            "wrong method published a value"
        );
    }
    increments.increment_with(|| 7);
    totals.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 123,
    });
    app.update();
    assert_eq!(test_support::values(&app, "test/increments"), [7.0]);
    assert_eq!(test_support::values(&app, "test/totals"), [123.0]);
    for metric in ["test/increments", "test/totals"] {
        assert!(matches!(
            quality(app.world(), &test_support::path(metric)),
            Some(MetricQuality::Incomplete {
                invalid_samples: 1,
                ..
            })
        ));
    }
}

#[test]
fn recording_interval_closure_error_marks_incomplete_and_preserves_first_failure() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        test_support::interval_definition("test/interval_mean"),
    )
    .expect("interval-mean gauge");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[test_support::retained("test/interval_mean")],
    )
    .expect("subscribe to interval means");
    app.update();
    let calls = Cell::new(0);
    recorder.record_interval_with(|_| {
        calls.set(calls.get() + 1);
        Err(InvalidMetricSample::MissingInterval)
    });
    app.update();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        test_support::values(&app, "test/interval_mean"),
        [] as [f64; 0]
    );
    assert_eq!(
        quality(app.world(), &test_support::path("test/interval_mean")),
        Some(MetricQuality::Incomplete {
            invalid_samples: 1,
            first_failure:   MeasurementFailure::InvalidSample(
                InvalidMetricSample::MissingInterval
            ),
        })
    );

    recorder.record_interval_with(|_| Err(InvalidMetricSample::LostSequence));
    app.world_mut()
        .resource_mut::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_interval_with(|_| {
        IntervalGaugeSample::new(
            12.0,
            ObservationInterval::new(
                RunOffset::new(Duration::ZERO),
                RunOffset::new(Duration::from_secs(1)),
            )
            .expect("ordered interval"),
        )
    });
    app.update();
    assert_eq!(test_support::values(&app, "test/interval_mean"), [12.0]);
    assert_eq!(
        quality(app.world(), &test_support::path("test/interval_mean")),
        Some(MetricQuality::Incomplete {
            invalid_samples: 2,
            first_failure:   MeasurementFailure::InvalidSample(
                InvalidMetricSample::MissingInterval
            ),
        })
    );
}

#[test]
fn recording_instantaneous_interval_summary_call_does_not_fabricate_coverage() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        test_support::interval_definition("test/interval_mean"),
    )
    .expect("interval-mean gauge");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[test_support::latest("test/interval_mean")],
    )
    .expect("subscribe to interval means");
    app.update();
    recorder.record_with(|| 31.0);
    app.update();

    assert_eq!(
        test_support::values(&app, "test/interval_mean"),
        [] as [f64; 0]
    );
    assert_eq!(
        retained_samples(app.world(), &test_support::path("test/interval_mean")),
        [] as [RetainedObservation; 0]
    );
    assert_eq!(
        quality(app.world(), &test_support::path("test/interval_mean")),
        Some(MetricQuality::Incomplete {
            invalid_samples: 1,
            first_failure:   MeasurementFailure::InvalidSample(
                InvalidMetricSample::MissingInterval
            ),
        })
    );
}

#[test]
fn recording_fixed_alpha_seeds_smooths_repeated_times_and_resets_after_stale_gap() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(
            test_support::path("test/work"),
            "Work",
            Freshness::CompletedEvent,
        )
        .with_smoothing(
            SmoothingPolicy::fixed_alpha(0.2, Duration::from_secs(5))
                .expect("frame collector smoothing policy"),
        ),
    )
    .expect("smoothed span");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[MetricSelection {
            path:      test_support::path("test/work"),
            statistic: MetricStatistic::Smoothed,
        }],
    )
    .expect("subscribe to declared smoothing");
    app.update();

    // Expected milliseconds use the compatibility frame collector's fixed 0.2 weight.
    for (gap, millis, expected) in [
        (Duration::ZERO, 10, 10.0),
        (Duration::ZERO, 30, 14.0),
        (Duration::from_secs(5), 20, 15.2),
        (Duration::from_secs(5) + Duration::from_nanos(1), 3, 3.0),
    ] {
        app.world().resource::<ManualRunClock>().advance(gap);
        recorder.record_duration_with(|| Duration::from_millis(millis));
        app.update();
        let smoothed = smoothed(app.world(), &test_support::path("test/work"))
            .expect("accepted span seeds the smoothing state");
        assert!(
            smoothed.mul_add(1000.0, -expected).abs() < 0.000_01,
            "expected {expected} ms, got {} ms",
            smoothed * 1000.0
        );
    }
}

#[test]
fn recording_time_based_smoothing_uses_elapsed_time_and_reseeds_on_new_epoch() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        test_support::gauge_definition("test/level").with_smoothing(
            SmoothingPolicy::time_based(Duration::from_secs(1), Duration::from_secs(5))
                .expect("time-based policy"),
        ),
    )
    .expect("smoothed gauge");
    let selection = MetricSelection {
        path:      test_support::path("test/level"),
        statistic: MetricStatistic::Smoothed,
    };
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))
        .expect("subscribe to declared smoothing");
    app.update();
    recorder.record_with(|| 10.0);
    app.update();
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_with(|| 30.0);
    app.update();
    let expected = 30.0 - 20.0 / std::f64::consts::E;
    let smoothed = smoothed(app.world(), &test_support::path("test/level"))
        .expect("time-based smoothing result");
    assert!((smoothed - expected).abs() < 1e-12);
    recorder.record_with(|| 99.0);
    app.update();
    let repeated_time = self::smoothed(app.world(), &test_support::path("test/level"))
        .expect("repeated-time smoothing result");
    assert!((repeated_time - expected).abs() < 1e-12);

    drop(subscription);
    app.update();
    let _reactivated = subscribe_metrics(app.world_mut(), &[selection]).expect("new epoch");
    app.update();
    recorder.record_with(|| 4.0);
    app.update();
    let reseeded = self::smoothed(app.world(), &test_support::path("test/level"))
        .expect("new epoch seeds smoothing");
    assert!((reseeded - 4.0).abs() < 1e-12);
}

#[test]
fn recording_measured_spans_keep_endpoints_and_duration_only_spans_are_completion_only() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(
            test_support::path("test/work"),
            "Work",
            Freshness::CompletedEvent,
        ),
    )
    .expect("timing span");
    let _subscription = subscribe_metrics(app.world_mut(), &[test_support::retained("test/work")])
        .expect("retain completed spans");
    app.update();
    let clock = app.world().resource::<ManualRunClock>().clone();
    clock.advance(Duration::from_secs(2));
    let calls = Cell::new(0);
    let result = recorder.measure(|| {
        calls.set(calls.get() + 1);
        clock.advance(Duration::from_millis(25));
        String::from("completed")
    });
    assert_eq!(result, "completed");
    assert_eq!(calls.get(), 1);
    recorder.record_duration_with(|| Duration::from_millis(7));
    app.update();

    let retained = retained_samples(app.world(), &test_support::path("test/work"));
    assert_eq!(retained.len(), 2);
    let end = RunOffset::new(Duration::from_millis(2025));
    assert_eq!(
        retained[0].extent,
        ObservationExtent::Interval(
            ObservationInterval::new(RunOffset::new(Duration::from_secs(2)), end)
                .expect("ordered measured span"),
        )
    );
    assert_eq!(retained[1].extent, ObservationExtent::CompletionOnly(end));
    assert_eq!(test_support::values(&app, "test/work"), [0.025, 0.007]);
}

#[test]
fn recording_out_of_order_appends_are_ingested_in_endpoint_order() {
    let mut app = test_support::metrics_app();
    let recorder =
        register_metric(&mut app, test_support::gauge_definition("test/level")).expect("gauge");
    let _subscription = subscribe_metrics(app.world_mut(), &[test_support::retained("test/level")])
        .expect("retain both producer observations");
    app.update();
    let clock = app.world().resource::<ManualRunClock>();
    clock.advance(Duration::from_secs(1));
    recorder.record_with(|| 11.0);
    clock.advance(Duration::from_secs(1));
    recorder.record_with(|| 22.0);
    // Model two producers taking ordered timestamps but acquiring the append lock in reverse.
    super::lock(&recorder.shared.pending).samples.reverse();
    app.update();

    assert_eq!(
        quality(app.world(), &test_support::path("test/level")),
        Some(MetricQuality::Complete)
    );
    let observations = retained_samples(app.world(), &test_support::path("test/level"));
    assert_eq!(observations.len(), 2);
    assert_eq!(
        observations[0].extent,
        ObservationExtent::Instant(RunOffset::new(Duration::from_secs(1)))
    );
    assert_eq!(
        observations[1].extent,
        ObservationExtent::Instant(RunOffset::new(Duration::from_secs(2)))
    );
    assert_eq!(observations[0].number, MetricNumber::Scalar(11.0));
    assert_eq!(observations[1].number, MetricNumber::Scalar(22.0));
    assert_eq!(test_support::values(&app, "test/level"), [11.0, 22.0]);
}

#[test]
fn recording_endpoint_before_previous_drain_is_a_clock_domain_failure() {
    let mut app = test_support::metrics_app();
    let recorder =
        register_metric(&mut app, test_support::gauge_definition("test/level")).expect("gauge");
    let _subscription = subscribe_metrics(app.world_mut(), &[test_support::retained("test/level")])
        .expect("retain producer observations");
    app.update();
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(2));
    recorder.record_with(|| 22.0);
    app.update();

    recorder.record_with(|| 11.0);
    super::lock(&recorder.shared.pending)
        .samples
        .last_mut()
        .expect("pending observation")
        .extent = ObservationExtent::Instant(RunOffset::new(Duration::from_secs(1)));
    app.update();

    assert_eq!(
        quality(app.world(), &test_support::path("test/level")),
        Some(MetricQuality::Incomplete {
            invalid_samples: 1,
            first_failure:   MeasurementFailure::ClockDomain,
        })
    );
    assert_eq!(
        retained_samples(app.world(), &test_support::path("test/level")).len(),
        1
    );
    assert_eq!(test_support::values(&app, "test/level"), [22.0]);
}

#[test]
fn recording_cumulative_counter_keeps_external_epoch_across_demand_reactivation() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            test_support::path("test/totals"),
            "External total",
            CounterUnit::Count,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )
    .expect("cumulative counter");
    let subscription = subscribe_metrics(app.world_mut(), &[test_support::latest("test/totals")])
        .expect("observe the first source total");
    app.update();
    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 100,
    });
    app.update();
    assert_eq!(
        quality(app.world(), &test_support::path("test/totals")),
        Some(MetricQuality::Complete)
    );
    assert_eq!(
        retained_samples(app.world(), &test_support::path("test/totals"))[0].number,
        MetricNumber::Count(100)
    );

    drop(subscription);
    app.update();
    let _reactivated = subscribe_metrics(app.world_mut(), &[test_support::latest("test/totals")])
        .expect("observe the same source again");
    app.update();
    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 50,
    });
    app.update();

    assert_eq!(
        quality(app.world(), &test_support::path("test/totals")),
        Some(MetricQuality::Incomplete {
            invalid_samples: 1,
            first_failure:   MeasurementFailure::FallingTotalInEpoch,
        })
    );
    assert_eq!(
        retained_samples(app.world(), &test_support::path("test/totals")),
        [] as [RetainedObservation; 0]
    );
    assert_eq!(test_support::values(&app, "test/totals"), [100.0]);

    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(2),
        total: 7,
    });
    app.update();
    let observations = retained_samples(app.world(), &test_support::path("test/totals"));
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].number, MetricNumber::Count(7));
    assert_eq!(test_support::values(&app, "test/totals"), [100.0, 7.0]);
    assert_eq!(
        quality(app.world(), &test_support::path("test/totals")),
        Some(MetricQuality::Complete)
    );
}

#[test]
fn recording_increment_total_restarts_in_each_observation_epoch() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/increments"),
            "Events",
            CounterUnit::Count,
        ),
    )
    .expect("increment counter");
    let subscription =
        subscribe_metrics(app.world_mut(), &[test_support::latest("test/increments")])
            .expect("first observation epoch");
    app.update();
    recorder.increment_with(|| 13);
    recorder.increment_with(|| 7);
    app.update();
    let previous = retained_samples(app.world(), &test_support::path("test/increments"));
    assert_eq!(previous.len(), 1);
    assert_eq!(previous[0].number, MetricNumber::Count(20));

    drop(subscription);
    app.update();
    let _reactivated =
        subscribe_metrics(app.world_mut(), &[test_support::latest("test/increments")])
            .expect("second observation epoch");
    app.update();
    assert_eq!(
        retained_samples(app.world(), &test_support::path("test/increments")),
        [] as [RetainedObservation; 0]
    );
    recorder.increment_with(|| 3);
    app.update();
    let current = retained_samples(app.world(), &test_support::path("test/increments"));
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].number, MetricNumber::Count(3));
    assert_ne!(current[0].epoch, previous[0].epoch);
    assert_eq!(
        quality(app.world(), &test_support::path("test/increments")),
        Some(MetricQuality::Complete)
    );
}

#[test]
fn recording_unit_alpha_stays_finite_for_opposite_extreme_values() {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        test_support::gauge_definition("test/level").with_smoothing(
            SmoothingPolicy::fixed_alpha(1.0, Duration::from_secs(5)).expect("unit alpha"),
        ),
    )
    .expect("smoothed gauge");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[MetricSelection {
            path:      test_support::path("test/level"),
            statistic: MetricStatistic::Smoothed,
        }],
    )
    .expect("observe the configured filter");
    app.update();
    for value in [f64::MAX, -f64::MAX] {
        recorder.record_with(|| value);
        app.update();
        let filtered =
            smoothed(app.world(), &test_support::path("test/level")).expect("filter value");
        assert!(filtered.is_finite());
        assert_eq!(filtered.to_bits(), value.to_bits());
    }
    assert_eq!(
        quality(app.world(), &test_support::path("test/level")),
        Some(MetricQuality::Complete)
    );
}

#[test]
fn recording_registration_without_service_does_not_add_a_diagnostic() {
    let mut app = App::new();
    app.add_plugins(DiagnosticsPlugin);
    let before = test_support::diagnostic_paths(&app);
    assert!(register_metric(&mut app, test_support::gauge_definition("test/level")).is_err());
    assert_eq!(test_support::diagnostic_paths(&app), before);
    assert!(
        app.world()
            .resource::<DiagnosticsStore>()
            .get(&test_support::path("test/level"))
            .is_none()
    );

    app.insert_resource(ManualRunClock::default())
        .add_plugins(MetricsPlugin);
    assert!(register_metric(&mut app, test_support::gauge_definition("test/level")).is_ok());
}

#[test]
fn recording_dormant_recorders_skip_all_value_closures_and_measure_work_once() {
    let mut app = test_support::metrics_app();
    let gauge =
        register_metric(&mut app, test_support::gauge_definition("test/level")).expect("gauge");
    let increments = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/increments"),
            "Increments",
            CounterUnit::Count,
        ),
    )
    .expect("increment counter");
    let totals = register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            test_support::path("test/totals"),
            "Totals",
            CounterUnit::Bytes,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )
    .expect("cumulative counter");
    let span = register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(
            test_support::path("test/work"),
            "Work",
            Freshness::CompletedEvent,
        ),
    )
    .expect("timing span");
    let value_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    for _ in 0..2 {
        gauge.record_with(|| {
            value_calls.set(value_calls.get() + 1);
            5.0
        });
        gauge.record_interval_with(|_| {
            value_calls.set(value_calls.get() + 1);
            Err(InvalidMetricSample::MissingInterval)
        });
        increments.increment_with(|| {
            value_calls.set(value_calls.get() + 1);
            5
        });
        totals.record_total_with(|| {
            value_calls.set(value_calls.get() + 1);
            CumulativeCounterSample {
                epoch: ExternalCounterEpoch::new(1),
                total: 19,
            }
        });
        span.record_duration_with(|| {
            value_calls.set(value_calls.get() + 1);
            Duration::from_millis(7)
        });
        let result = span.measure(|| {
            work_calls.set(work_calls.get() + 1);
            String::from("application result")
        });
        assert_eq!(result, "application result");
        app.update();
    }
    assert_eq!(value_calls.get(), 0);
    assert_eq!(work_calls.get(), 2);
    for metric in ["test/level", "test/increments", "test/totals", "test/work"] {
        assert_eq!(test_support::values(&app, metric), [] as [f64; 0]);
    }
}

#[test]
fn recording_repeated_samples_in_one_update_keep_values_and_source_timestamps() {
    let mut app = test_support::metrics_app();
    let recorder =
        register_metric(&mut app, test_support::gauge_definition("test/level")).expect("gauge");
    let anchor = register_metric(&mut app, test_support::gauge_definition("test/anchor"))
        .expect("clock anchor");
    let _subscription = subscribe_metrics(
        app.world_mut(),
        &[
            test_support::retained("test/level"),
            test_support::latest("test/anchor"),
        ],
    )
    .expect("subscribe to both metrics");
    app.update();
    anchor.record_with(|| 1.0);
    app.update();
    let origin = app
        .world()
        .resource::<DiagnosticsStore>()
        .get(&test_support::path("test/anchor"))
        .expect("anchor diagnostic")
        .measurement()
        .expect("anchor sample")
        .time;
    app.add_systems(Update, move |world: &mut World| {
        world
            .resource_mut::<ManualRunClock>()
            .advance(Duration::from_millis(1));
        recorder.record_with(|| 1.25);
        world
            .resource_mut::<ManualRunClock>()
            .advance(Duration::from_millis(2));
        recorder.record_with(|| 3.5);
        world
            .resource_mut::<ManualRunClock>()
            .advance(Duration::from_millis(5));
        recorder.record_with(|| 8.75);
        // Make drain time distinct from every observation time.
        world
            .resource_mut::<ManualRunClock>()
            .advance(Duration::from_secs(5));
    });
    app.update();

    let store = app.world().resource::<DiagnosticsStore>();
    let samples = store
        .get(&test_support::path("test/level"))
        .expect("gauge diagnostic")
        .measurements()
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 3, "append ingest must not coalesce a path");
    for (sample, (offset_ms, value)) in samples.iter().zip([(1, 1.25_f64), (3, 3.5), (8, 8.75)]) {
        assert_eq!(sample.value.to_bits(), value.to_bits());
        assert_eq!(
            sample.time.duration_since(origin),
            Duration::from_millis(offset_ms)
        );
    }
}

fn descriptor<'world>(
    world: &'world World,
    path: &DiagnosticPath,
) -> Option<&'world MetricDescriptor> {
    world
        .resource::<MetricRegistry>()
        .entries
        .get(path)
        .map(|entry| &entry.descriptor)
}

fn quality(world: &World, path: &DiagnosticPath) -> Option<MetricQuality> {
    match &world.resource::<MetricRegistry>().entries.get(path)?.source {
        MetricSource::Owned(owned) => Some(owned.quality),
        MetricSource::AdoptedNative | MetricSource::DiscoveredNative => None,
    }
}

fn retained_samples(world: &World, path: &DiagnosticPath) -> Vec<RetainedObservation> {
    let entry = world
        .resource::<MetricRegistry>()
        .entries
        .get(path)
        .expect("registered metric");
    match &entry.source {
        MetricSource::Owned(owned) => owned.retained.iter().copied().collect(),
        MetricSource::AdoptedNative | MetricSource::DiscoveredNative => Vec::new(),
    }
}

fn smoothed(world: &World, path: &DiagnosticPath) -> Option<f64> {
    match &world.resource::<MetricRegistry>().entries.get(path)?.source {
        MetricSource::Owned(owned) => match &owned.smoothing {
            FilterState::Unseeded => None,
            FilterState::Seeded { value, .. } => Some(*value),
        },
        MetricSource::AdoptedNative | MetricSource::DiscoveredNative => None,
    }
}
