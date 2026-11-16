//! Consumer lifetimes control collection at the next First boundary.
#![allow(
    clippy::expect_used,
    reason = "tests stop when valid fixtures or active sources are absent"
)]

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bevy_app::App;
use bevy_app::Update;
use bevy_diagnostic::Diagnostic;
use bevy_diagnostic::DiagnosticPath;
use bevy_diagnostic::Diagnostics;
use bevy_diagnostic::DiagnosticsStore;

use crate::ActiveMetricSource;
use crate::Counter;
use crate::CounterUnit;
use crate::CumulativeCounterSample;
use crate::ExternalCounterEpoch;
use crate::Freshness;
use crate::Gauge;
use crate::GaugeSemantics;
use crate::ManualRunClock;
use crate::MeasurementCadence;
use crate::MetricDefinition;
use crate::MetricNumber;
use crate::MetricRecorder;
use crate::MetricSelection;
use crate::MetricStatistic;
use crate::MetricSubscription;
use crate::MetricUnit;
use crate::MetricsSettings;
use crate::RunOffset;
use crate::SampleWindow;
use crate::SubscriptionStatus;
use crate::WindowReduction;
use crate::register_metric;
use crate::subscribe_metrics;
use crate::test_support;

#[test]
fn demand_pending_activation_prevents_recording_until_first_before_update() {
    let mut app = test_support::metrics_app();
    let recorder = register_gauge(&mut app, "test/level");
    let calls = Arc::new(AtomicUsize::new(0));
    let producer_calls = Arc::clone(&calls);
    let producer = recorder.clone();
    app.add_systems(Update, move || {
        producer.record_with(|| {
            producer_calls.fetch_add(1, Ordering::Relaxed);
            7.0
        });
    });
    let subscription = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("subscribe to a registered gauge");
    assert!(matches!(
        subscription.status(),
        SubscriptionStatus::PendingActivation
    ));
    assert!(!test_support::enabled(&app, "test/level"));
    recorder.record_with(|| {
        calls.fetch_add(1, Ordering::Relaxed);
        3.0
    });
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    advance(&mut app, Duration::from_secs(2));
    app.update();

    let source = active_source(&subscription);
    assert_eq!(source.effective_at, RunOffset::new(Duration::from_secs(2)));
    assert!(test_support::enabled(&app, "test/level"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(diagnostic_value(&app, "test/level"), Some(7.0));
}

#[test]
fn demand_drop_releases_collection_at_next_first() {
    let mut app = test_support::metrics_app();
    let recorder = register_gauge(&mut app, "test/level");
    let subscription = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("subscribe to a registered gauge");
    app.update();
    let calls = Cell::new(0);

    drop(subscription);
    assert!(test_support::enabled(&app, "test/level"));
    recorder.record_with(|| {
        calls.set(calls.get() + 1);
        3.0
    });
    assert_eq!(calls.get(), 1, "drop must queue release until First");
    app.update();
    assert!(!test_support::enabled(&app, "test/level"));
    recorder.record_with(|| {
        calls.set(calls.get() + 1);
        4.0
    });
    assert_eq!(
        calls.get(),
        1,
        "released source must skip value construction"
    );
}

#[test]
fn demand_two_subscribers_share_epoch_and_one_drop_keeps_source_active() {
    let mut app = test_support::metrics_app();
    let recorder = register_gauge(&mut app, "test/level");
    let first = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("first subscriber");
    app.update();
    let original = active_source(&first);
    advance(&mut app, Duration::from_secs(1));
    let second = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("second subscriber");
    app.update();
    assert_eq!(active_source(&second).epoch, original.epoch);
    assert_eq!(active_source(&first).epoch, original.epoch);

    drop(first);
    app.update();
    assert_eq!(active_source(&second).epoch, original.epoch);
    assert!(test_support::enabled(&app, "test/level"));
    recorder.record_with(|| 19.0);
    app.update();
    assert_eq!(diagnostic_value(&app, "test/level"), Some(19.0));

    drop(second);
    app.update();
    assert!(!test_support::enabled(&app, "test/level"));
}

#[test]
fn demand_reactivation_after_release_starts_a_new_epoch() {
    let mut app = test_support::metrics_app();
    register_gauge(&mut app, "test/level");
    let first = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("first subscriber");
    app.update();
    let original = active_source(&first);
    drop(first);
    app.update();
    advance(&mut app, Duration::from_secs(3));

    let next = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])
        .expect("reactivation subscriber");
    assert!(matches!(
        next.status(),
        SubscriptionStatus::PendingActivation
    ));
    app.update();
    let reactivated = active_source(&next);
    assert_ne!(reactivated.epoch, original.epoch);
    assert_eq!(
        reactivated.effective_at,
        RunOffset::new(Duration::from_secs(3))
    );
}

#[test]
fn demand_invalid_statistic_and_unknown_path_do_not_partially_activate_sources() {
    let mut app = test_support::metrics_app();
    register_gauge(&mut app, "test/level");
    for invalid in [
        test_support::latest("test/unknown"),
        MetricSelection {
            path:      DiagnosticPath::const_new("test/level"),
            statistic: MetricStatistic::Smoothed,
        },
        MetricSelection {
            path:      DiagnosticPath::const_new("test/level"),
            statistic: MetricStatistic::Window {
                window:    SampleWindow::LastSamples(test_support::nonzero(2)),
                reduction: WindowReduction::Sum,
            },
        },
    ] {
        assert!(
            subscribe_metrics(
                app.world_mut(),
                &[test_support::latest("test/level"), invalid]
            )
            .is_err()
        );
        app.update();
        assert!(!test_support::enabled(&app, "test/level"));
    }
}

#[test]
fn demand_native_adoption_preserves_producer_enablement_after_release() {
    for native_enabled in [true, false] {
        let mut app = test_support::metrics_app();
        let mut native = Diagnostic::new(test_support::path("native/level"));
        native.is_enabled = native_enabled;
        app.world_mut()
            .resource_mut::<DiagnosticsStore>()
            .add(native);
        crate::describe_diagnostic(
            &mut app,
            crate::NativeDiagnosticDescription::new(
                test_support::path("native/level"),
                "Native level",
                MetricUnit::Count,
                GaugeSemantics::Level,
            ),
        )
        .expect("adopt an existing native diagnostic");
        assert_eq!(test_support::enabled(&app, "native/level"), native_enabled);

        let subscription =
            subscribe_metrics(app.world_mut(), &[test_support::latest("native/level")])
                .expect("subscribe to an adopted native diagnostic");
        app.update();
        assert!(matches!(
            subscription.status(),
            SubscriptionStatus::Active { .. }
        ));
        assert_eq!(test_support::enabled(&app, "native/level"), native_enabled);
        drop(subscription);
        app.add_systems(Update, |mut diagnostics: Diagnostics| {
            diagnostics.add_measurement(&test_support::path("native/level"), || 41.0);
        });
        app.update();

        assert_eq!(test_support::enabled(&app, "native/level"), native_enabled);
        let expected = if native_enabled { Some(41.0) } else { None };
        assert_eq!(diagnostic_value(&app, "native/level"), expected);
    }
}

/// Registers a level gauge in the fixture app.
///
/// # Panics
/// Panics when a required fixture or subscription state is absent.
fn register_gauge(app: &mut App, path: &'static str) -> MetricRecorder<Gauge> {
    register_metric(app, test_support::gauge_definition(path)).expect("register a valid gauge")
}

/// Advances the fixture app clock by the supplied duration.
fn advance(app: &mut App, elapsed: Duration) {
    app.world_mut()
        .resource_mut::<ManualRunClock>()
        .advance(elapsed);
}

/// Reads the single source named by an active fixture subscription.
///
/// # Panics
/// Panics when a required fixture or subscription state is absent.
fn active_source(subscription: &MetricSubscription) -> ActiveMetricSource {
    let sources = match subscription.status() {
        SubscriptionStatus::Active { sources } => Some(sources),
        _ => None,
    }
    .expect("subscription is active");
    assert_eq!(sources.len(), 1, "one selection names one ordinary source");
    sources.into_iter().next().expect("one active source")
}

/// Reads the latest scalar value of a registered fixture diagnostic.
///
/// # Panics
/// Panics when a required fixture or subscription state is absent.
fn diagnostic_value(app: &App, path: &'static str) -> Option<f64> {
    app.world()
        .resource::<DiagnosticsStore>()
        .get(&DiagnosticPath::const_new(path))
        .expect("registered diagnostic")
        .value()
}

#[test]
fn demand_retention_limits_reject_the_whole_request_before_acquiring_demand()
-> Result<(), Box<dyn std::error::Error>> {
    assert_retention_limits(test_support::metrics_app())
}

fn assert_retention_limits(mut app: App) -> Result<(), Box<dyn std::error::Error>> {
    let settings = app.world().resource::<MetricsSettings>();
    let max_samples = settings.max_retained_samples.get();
    let max_trailing = settings.max_trailing;
    let recorder = crate::register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    for window in [
        SampleWindow::LastSamples(test_support::nonzero(max_samples + 1)),
        SampleWindow::Trailing {
            duration: max_trailing,
            capacity: test_support::nonzero(max_samples + 1),
        },
        SampleWindow::Trailing {
            duration: max_trailing + Duration::from_nanos(1),
            capacity: test_support::nonzero(2),
        },
    ] {
        let selections = [
            test_support::latest("test/level"),
            test_support::window_selection("test/level", window),
        ];
        assert!(subscribe_metrics(app.world_mut(), &selections).is_err());
        app.update();
        assert!(!test_support::enabled(&app, "test/level"));
        let called = Cell::new(false);
        recorder.record_with(|| {
            called.set(true);
            5.0
        });
        assert!(!called.get(), "failed subscription acquired demand");
    }

    let at_limit = test_support::window_selection(
        "test/level",
        SampleWindow::Trailing {
            duration: max_trailing,
            capacity: test_support::nonzero(max_samples),
        },
    );
    let subscription = subscribe_metrics(app.world_mut(), &[at_limit])?;
    app.update();
    assert!(matches!(
        subscription.status(),
        SubscriptionStatus::Active { .. }
    ));
    Ok(())
}

#[test]
fn demand_later_counter_subscription_has_its_own_delta_and_rate_baseline()
-> Result<(), Box<dyn std::error::Error>> {
    for cumulative in [false, true] {
        let mut app = test_support::metrics_app();
        let definition = if cumulative {
            MetricDefinition::<Counter>::cumulative(
                test_support::path("test/events"),
                "Events",
                CounterUnit::Count,
                MeasurementCadence::EveryUpdate,
                Freshness::CompletedEvent,
            )
        } else {
            MetricDefinition::<Counter>::new(
                test_support::path("test/events"),
                "Events",
                CounterUnit::Count,
            )
        };
        let recorder = register_metric(&mut app, definition)?;
        let window = SampleWindow::Trailing {
            duration: Duration::from_secs(3),
            capacity: test_support::nonzero(8),
        };
        let selections = [
            test_support::latest("test/events"),
            test_support::reduction("test/events", window, WindowReduction::Delta),
            test_support::reduction("test/events", window, WindowReduction::Rate),
        ];
        let first = subscribe_metrics(app.world_mut(), &selections)?;
        app.update();
        if cumulative {
            recorder.record_total_with(|| CumulativeCounterSample {
                epoch: ExternalCounterEpoch::new(1),
                total: 100,
            });
            app.update();
        }
        for (increment, total) in [(10, 110), (20, 130)] {
            advance(&mut app, Duration::from_secs(1));
            if cumulative {
                recorder.record_total_with(|| CumulativeCounterSample {
                    epoch: ExternalCounterEpoch::new(1),
                    total,
                });
            } else {
                recorder.increment_with(|| increment);
            }
            app.update();
        }
        let first_source = active_source(&first);
        let second = subscribe_metrics(app.world_mut(), &selections)?;
        assert!(matches!(
            second.status(),
            SubscriptionStatus::PendingActivation
        ));
        app.update();
        assert_eq!(active_source(&second).epoch, first_source.epoch);
        advance(&mut app, Duration::from_secs(1));
        if cumulative {
            recorder.record_total_with(|| CumulativeCounterSample {
                epoch: ExternalCounterEpoch::new(1),
                total: 160,
            });
        } else {
            recorder.increment_with(|| 30);
        }
        app.update();
        let expected_total = if cumulative { 160 } else { 60 };
        for subscription in [&first, &second] {
            assert_eq!(
                test_support::available(test_support::read(
                    &mut app,
                    subscription,
                    &selections[0]
                )?)?
                .value,
                MetricNumber::Count(expected_total)
            );
            assert_eq!(active_source(subscription).epoch, first_source.epoch);
        }
        assert_eq!(
            test_support::available(test_support::read(&mut app, &first, &selections[1])?)?.value,
            MetricNumber::Count(60)
        );
        assert_eq!(
            test_support::available(test_support::read(&mut app, &second, &selections[1])?)?.value,
            MetricNumber::Count(30)
        );
        let first_rate =
            test_support::available(test_support::read(&mut app, &first, &selections[2])?)?;
        let second_rate =
            test_support::available(test_support::read(&mut app, &second, &selections[2])?)?;
        assert_eq!(test_support::scalar(&first_rate)?, 20.0);
        assert_eq!(test_support::scalar(&second_rate)?, 30.0);
        assert_eq!(first_rate.unit, MetricUnit::CountPerSecond);
        assert_eq!(second_rate.unit, MetricUnit::CountPerSecond);
    }
    Ok(())
}

#[test]
fn demand_repeated_timestamp_delta_starts_after_its_activation_sequence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/events"),
            "Events",
            CounterUnit::Count,
        ),
    )?;
    let first_selection = test_support::reduction(
        "test/events",
        SampleWindow::LastSamples(test_support::nonzero(3)),
        WindowReduction::Delta,
    );
    let first = subscribe_metrics(app.world_mut(), &[first_selection])?;
    app.update();
    for increment in [10, 20, 30] {
        recorder.increment_with(|| increment);
    }
    app.update();
    let second_selection = test_support::reduction(
        "test/events",
        SampleWindow::LastSamples(test_support::nonzero(2)),
        WindowReduction::Delta,
    );
    let second = subscribe_metrics(app.world_mut(), std::slice::from_ref(&second_selection))?;
    app.update();
    assert_eq!(active_source(&first).epoch, active_source(&second).epoch);
    recorder.increment_with(|| 40);
    app.update();
    let estimate =
        test_support::available(test_support::read(&mut app, &second, &second_selection)?)?;
    assert_eq!(estimate.value, MetricNumber::Count(40));
    assert_eq!(estimate.measured_at, RunOffset::new(Duration::ZERO));
    Ok(())
}

#[test]
fn demand_cumulative_rate_keeps_the_baseline_snapshots_real_timestamp()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            test_support::path("test/events"),
            "Events",
            CounterUnit::Count,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )?;
    let first = subscribe_metrics(app.world_mut(), &[test_support::latest("test/events")])?;
    app.update();
    advance(&mut app, Duration::from_secs(1));
    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 100,
    });
    app.update();
    advance(&mut app, Duration::from_secs(9));
    let selection = test_support::reduction(
        "test/events",
        SampleWindow::Trailing {
            duration: Duration::from_secs(10),
            capacity: test_support::nonzero(4),
        },
        WindowReduction::Rate,
    );
    let second = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    assert_eq!(active_source(&first).epoch, active_source(&second).epoch);
    assert_eq!(
        active_source(&second).effective_at,
        RunOffset::new(Duration::from_secs(10))
    );
    advance(&mut app, Duration::from_secs(1));
    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 160,
    });
    app.update();
    let estimate = test_support::available(test_support::read(&mut app, &second, &selection)?)?;
    assert_eq!(test_support::scalar(&estimate)?, 6.0);
    assert_eq!(estimate.unit, MetricUnit::CountPerSecond);
    assert_eq!(
        estimate.measured_at,
        RunOffset::new(Duration::from_secs(11))
    );
    Ok(())
}

#[test]
fn demand_runtime_settings_bound_samples_and_duration_before_acquiring_demand()
-> Result<(), Box<dyn std::error::Error>> {
    let sample_limit = test_support::nonzero(3);
    let time_limit = Duration::from_secs(2);
    let app = test_support::metrics_app_with_settings(MetricsSettings {
        max_retained_samples: sample_limit,
        max_trailing:         time_limit,
    });
    let settings = app.world().resource::<MetricsSettings>();
    assert_eq!(settings.max_retained_samples, sample_limit);
    assert_eq!(settings.max_trailing, time_limit);
    assert_retention_limits(app)
}
