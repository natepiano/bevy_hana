//! Consumer-facing windows, catalogue metadata and named reading states.

use std::error::Error;
use std::time::Duration;

use bevy_diagnostic::Diagnostic;
use bevy_diagnostic::DiagnosticMeasurement;
use bevy_diagnostic::DiagnosticsStore;

use crate::BoundaryCoverage;
use crate::Counter;
use crate::CounterUnit;
use crate::CumulativeCounterSample;
use crate::ExternalCounterEpoch;
use crate::Freshness;
use crate::GaugeSemantics;
use crate::ManualRunClock;
use crate::MeasurementCadence;
use crate::MeasurementFailure;
use crate::MetricDefinition;
use crate::MetricLookup;
use crate::MetricNumber;
use crate::MetricReading;
use crate::MetricSelection;
use crate::MetricSeries;
use crate::MetricStatistic;
use crate::MetricUnit;
use crate::NativeDiagnosticDescription;
use crate::Percentile;
use crate::RunOffset;
use crate::SampleWindow;
use crate::SmoothingPolicy;
use crate::SpanSample;
use crate::StatisticKind;
use crate::SubscriptionStatus;
use crate::TimingSpan;
use crate::UnavailableReason;
use crate::WindowReduction;
use crate::describe_diagnostic;
use crate::recording::MetricRegistry;
use crate::recording::registered_metric::SourceActivity;
use crate::register_metric;
use crate::subscribe_metrics;
use crate::test_support;

#[test]
fn reading_trailing_membership_is_open_at_start_and_closed_at_now() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let window = SampleWindow::Trailing {
        duration: Duration::from_secs(2),
        capacity: test_support::nonzero(8),
    };
    let selection = test_support::reduction("test/level", window, WindowReduction::Maximum);
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    recorder.record_with(|| 100.0);
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_with(|| 7.0);
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_with(|| 3.0);
    app.update();
    let estimate =
        test_support::available(test_support::read(&mut app, &subscription, &selection)?)?;
    assert_eq!(test_support::scalar(&estimate)?, 7.0);
    assert_eq!(estimate.coverage.sample_count, 2);
    assert_eq!(estimate.coverage.boundary, BoundaryCoverage::Complete);
    let snapshot = match test_support::series(&mut app, &subscription, "test/level", &window)? {
        MetricSeries::Available(snapshot) => snapshot,
        other => return Err(format!("expected available series: {other:?}").into()),
    };
    assert_eq!(
        snapshot
            .samples
            .iter()
            .map(|sample| sample.number)
            .collect::<Vec<_>>(),
        [MetricNumber::Scalar(7.0), MetricNumber::Scalar(3.0)]
    );
    assert_eq!(snapshot.unit, MetricUnit::Count);

    // A read-time clock advance expires the peak even without another producer call or update.
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &subscription,
            &selection
        )?)?)?,
        3.0
    );
    assert_eq!(
        snapshot.samples.len(),
        2,
        "an earlier snapshot is immutable"
    );
    Ok(())
}

#[test]
fn reading_windows_warm_up_until_the_requested_boundary_is_covered() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let selection = test_support::reduction(
        "test/level",
        SampleWindow::Trailing {
            duration: Duration::from_secs(2),
            capacity: test_support::nonzero(8),
        },
        WindowReduction::Mean,
    );
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_with(|| 4.0);
    app.update();
    match test_support::read(&mut app, &subscription, &selection)? {
        MetricReading::Unavailable(UnavailableReason::WarmingUp(coverage)) => {
            assert_eq!(coverage.boundary, BoundaryCoverage::WarmingUp);
            assert_eq!(coverage.sample_count, 1);
        },
        other => return Err(format!("epoch start must report warm-up: {other:?}").into()),
    }
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    recorder.record_with(|| 6.0);
    app.update();
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &subscription,
            &selection
        )?)?)?,
        5.0
    );
    Ok(())
}

#[test]
fn reading_capacity_overflow_is_incomplete_until_the_lost_sample_expires()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let selection = test_support::reduction(
        "test/level",
        SampleWindow::Trailing {
            duration: Duration::from_secs(3),
            capacity: test_support::nonzero(2),
        },
        WindowReduction::Mean,
    );
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    for value in [2.0, 4.0, 6.0] {
        app.world()
            .resource::<ManualRunClock>()
            .advance(Duration::from_secs(1));
        recorder.record_with(|| value);
    }
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::Incomplete(
            MeasurementFailure::CapacityLoss
        ))
    ));
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    // t=1 is now the excluded opening boundary. No current member was lost.
    match test_support::read(&mut app, &subscription, &selection)? {
        MetricReading::Available(estimate) | MetricReading::Stale { last: estimate, .. } => {
            assert_eq!(test_support::scalar(&estimate)?, 5.0);
            assert_eq!(estimate.coverage.boundary, BoundaryCoverage::Complete);
        },
        other @ MetricReading::Unavailable(_) => {
            return Err(format!("expired loss must stop poisoning the window: {other:?}").into());
        },
    }
    Ok(())
}

#[test]
fn reading_retention_shrink_records_loss_inside_a_trailing_window() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let narrow = test_support::reduction(
        "test/level",
        SampleWindow::Trailing {
            duration: Duration::from_secs(3),
            capacity: test_support::nonzero(2),
        },
        WindowReduction::Mean,
    );
    let wide = test_support::reduction(
        "test/level",
        SampleWindow::LastSamples(test_support::nonzero(8)),
        WindowReduction::Mean,
    );
    let narrow_subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&narrow))?;
    let wide_subscription = subscribe_metrics(app.world_mut(), &[wide])?;
    app.update();
    for value in [2.0, 4.0, 6.0] {
        app.world()
            .resource::<ManualRunClock>()
            .advance(Duration::from_secs(1));
        recorder.record_with(|| value);
    }
    app.update();
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &narrow_subscription,
            &narrow
        )?)?)?,
        4.0
    );
    drop(wide_subscription);
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &narrow_subscription, &narrow)?,
        MetricReading::Unavailable(UnavailableReason::Incomplete(
            MeasurementFailure::CapacityLoss
        ))
    ));
    Ok(())
}

#[test]
fn reading_even_median_and_nearest_rank_percentiles_use_the_sorted_distribution()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let window = SampleWindow::LastSamples(test_support::nonzero(4));
    let mut selections = vec![
        test_support::reduction("test/level", window, WindowReduction::Median),
        test_support::reduction("test/level", window, WindowReduction::Mean),
    ];
    for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        selections.push(test_support::reduction(
            "test/level",
            window,
            WindowReduction::Percentile(Percentile::try_new(fraction)?),
        ));
    }
    let subscription = subscribe_metrics(app.world_mut(), &selections)?;
    app.update();
    for value in [20.0, 2.0, 8.0, 4.0] {
        recorder.record_with(|| value);
    }
    app.update();
    for (selection, expected) in selections.iter().zip([6.0, 8.5, 2.0, 2.0, 4.0, 8.0, 20.0]) {
        let estimate =
            test_support::available(test_support::read(&mut app, &subscription, selection)?)?;
        assert_eq!(test_support::scalar(&estimate)?, expected);
        assert_eq!(estimate.coverage.sample_count, 4);
    }
    Ok(())
}

#[test]
fn reading_latest_counter_keeps_integer_precision_above_two_to_the_fifty_third()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/events"),
            "Events",
            CounterUnit::Count,
        ),
    )?;
    let selection = test_support::latest("test/events");
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    let total = (1_u64 << 53) + 1;
    recorder.increment_with(|| total);
    app.update();
    let estimate =
        test_support::available(test_support::read(&mut app, &subscription, &selection)?)?;
    assert_eq!(estimate.value, MetricNumber::Count(total));
    assert_eq!(estimate.unit, MetricUnit::Count);
    Ok(())
}

#[test]
fn reading_cumulative_totals_keep_nonzero_baselines_equal_samples_and_source_resets()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::cumulative(
            test_support::path("test/bytes"),
            "Bytes",
            CounterUnit::Bytes,
            MeasurementCadence::EveryUpdate,
            Freshness::CompletedEvent,
        ),
    )?;
    let delta = test_support::reduction(
        "test/bytes",
        SampleWindow::LastSamples(test_support::nonzero(2)),
        WindowReduction::Delta,
    );
    let latest = test_support::latest("test/bytes");
    let subscription = subscribe_metrics(app.world_mut(), &[delta.clone(), latest.clone()])?;
    app.update();
    for total in [900, 900] {
        recorder.record_total_with(|| CumulativeCounterSample {
            epoch: ExternalCounterEpoch::new(1),
            total,
        });
        app.world()
            .resource::<ManualRunClock>()
            .advance(Duration::from_secs(1));
        app.update();
    }
    assert_eq!(
        test_support::available(test_support::read(&mut app, &subscription, &latest)?)?.value,
        MetricNumber::Count(900)
    );
    let estimate = test_support::available(test_support::read(&mut app, &subscription, &delta)?)?;
    assert_eq!(estimate.value, MetricNumber::Count(0));
    assert_eq!(estimate.unit, MetricUnit::Bytes);
    recorder.record_total_with(|| CumulativeCounterSample {
        epoch: ExternalCounterEpoch::new(1),
        total: 899,
    });
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &latest)?,
        MetricReading::Unavailable(UnavailableReason::Incomplete(
            MeasurementFailure::FallingTotalInEpoch
        ))
    ));
    for total in [7, 12] {
        recorder.record_total_with(|| CumulativeCounterSample {
            epoch: ExternalCounterEpoch::new(2),
            total,
        });
        app.world()
            .resource::<ManualRunClock>()
            .advance(Duration::from_secs(1));
        app.update();
    }
    assert_eq!(
        test_support::available(test_support::read(&mut app, &subscription, &latest)?)?.value,
        MetricNumber::Count(12)
    );
    assert_eq!(
        test_support::available(test_support::read(&mut app, &subscription, &delta)?)?.value,
        MetricNumber::Count(5)
    );
    Ok(())
}

#[test]
fn reading_fixed_alpha_matches_the_compatibility_span_sequence() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(
            test_support::path("test/work"),
            "Work",
            Freshness::CompletedEvent,
        )
        .with_smoothing(SmoothingPolicy::fixed_alpha(0.2, Duration::from_secs(5))?),
    )?;
    let selection = MetricSelection {
        path:      test_support::path("test/work"),
        statistic: MetricStatistic::Smoothed,
    };
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    let mut compatibility = crate::SpanTiming::default();
    let mut peaks = crate::PeakDeque::default();
    let mut time_ms = 0;
    for (frame, gap_ms, millis, compatibility_millis) in [
        (1, 0, 10, 10.0_f32),
        (2, 0, 30, 30.0),
        (3, 5000, 20, 20.0),
        (4, 5001, 3, 3.0),
    ] {
        time_ms += gap_ms;
        app.world()
            .resource::<ManualRunClock>()
            .advance(Duration::from_millis(gap_ms));
        compatibility.observe(frame, compatibility_millis, time_ms, &mut peaks);
        recorder.record_duration_with(|| Duration::from_millis(millis));
        app.update();
        let expected = match compatibility.sample {
            SpanSample::Measured { smoothed_ms, .. } => f64::from(smoothed_ms),
            SpanSample::NeverMeasured => return Err("compatibility span did not observe".into()),
        };
        let estimate =
            test_support::available(test_support::read(&mut app, &subscription, &selection)?)?;
        assert_eq!(estimate.unit, MetricUnit::Seconds);
        assert!(
            test_support::scalar(&estimate)?
                .mul_add(1000.0, -expected)
                .abs()
                < 0.000_01
        );
    }
    Ok(())
}

#[test]
fn reading_unavailable_precedence_does_not_leak_values_or_acquire_demand()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    register_metric(&mut app, test_support::gauge_definition("test/other"))?;
    let selection = test_support::latest("test/level");
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    assert!(matches!(
        test_support::read(
            &mut app,
            &subscription,
            &test_support::latest("test/missing")
        )?,
        MetricReading::Unavailable(UnavailableReason::UnknownPath)
    ));
    let unsupported = test_support::reduction(
        "test/other",
        SampleWindow::LastSamples(test_support::nonzero(1)),
        WindowReduction::Sum,
    );
    assert!(matches!(
        test_support::read(&mut app, &subscription, &unsupported)?,
        MetricReading::Unavailable(UnavailableReason::Unsupported(_))
    ));
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::Dormant)
    ));
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::NeverObserved)
    ));
    assert!(matches!(
        test_support::read(&mut app, &subscription, &test_support::latest("test/other"))?,
        MetricReading::Unavailable(UnavailableReason::Dormant)
    ));
    assert!(!test_support::enabled(&app, "test/other"));
    recorder.record_with(|| f64::NAN);
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::Incomplete(
            MeasurementFailure::NonFiniteValue
        ))
    ));
    Ok(())
}

#[test]
fn reading_live_values_and_series_become_stale_with_the_original_age() -> Result<(), Box<dyn Error>>
{
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let selection = test_support::latest("test/level");
    let window = SampleWindow::LastSamples(test_support::nonzero(1));
    let retained = test_support::reduction("test/level", window, WindowReduction::Mean);
    let subscription = subscribe_metrics(app.world_mut(), &[selection.clone(), retained])?;
    app.update();
    recorder.record_with(|| 23.0);
    app.update();
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(1));
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &subscription,
            &selection
        )?)?)?,
        23.0
    );
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_nanos(1));
    let expected_age = Duration::from_secs(1) + Duration::from_nanos(1);
    match test_support::read(&mut app, &subscription, &selection)? {
        MetricReading::Stale { last, age } => {
            assert_eq!(test_support::scalar(&last)?, 23.0);
            assert_eq!(last.measured_at, RunOffset::new(Duration::ZERO));
            assert_eq!(age, expected_age);
        },
        other => return Err(format!("expected stale scalar: {other:?}").into()),
    }
    match test_support::series(&mut app, &subscription, "test/level", &window)? {
        MetricSeries::Stale { last, age } => {
            assert_eq!(last.samples.len(), 1);
            assert_eq!(last.samples[0].number, MetricNumber::Scalar(23.0));
            assert_eq!(age, expected_age);
        },
        other => return Err(format!("expected stale series: {other:?}").into()),
    }
    Ok(())
}

#[test]
fn reading_completed_events_remain_available_without_a_freshness_deadline()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<TimingSpan>::new(
            test_support::path("test/startup"),
            "Startup",
            Freshness::CompletedEvent,
        ),
    )?;
    let selection = test_support::latest("test/startup");
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    recorder.record_duration_with(|| Duration::from_millis(12));
    app.update();
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(3600));
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &subscription,
            &selection
        )?)?)?,
        0.012
    );
    Ok(())
}

#[test]
fn reading_catalogue_queries_leave_owned_producers_dormant() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    test_support::with_catalogue(&mut app, |catalogue| {
        assert!(
            catalogue
                .descriptors()
                .any(|descriptor| descriptor.path() == &test_support::path("test/level"))
        );
        assert!(matches!(
            catalogue.lookup(&test_support::path("test/level")),
            MetricLookup::Registered(_)
        ));
        assert!(matches!(
            catalogue.lookup(&test_support::path("test/missing")),
            MetricLookup::Unregistered
        ));
        Ok(())
    })?;
    app.update();
    assert!(!test_support::enabled(&app, "test/level"));
    let called = std::cell::Cell::new(false);
    recorder.record_with(|| {
        called.set(true);
        9.0
    });
    assert!(!called.get());
    Ok(())
}

#[test]
fn reading_late_native_discovery_preserves_enablement_and_allows_explicit_adoption()
-> Result<(), Box<dyn Error>> {
    for enabled in [false, true] {
        let mut app = test_support::metrics_app();
        app.finish();
        app.cleanup();
        let mut diagnostic = Diagnostic::new(test_support::path("native/late")).with_suffix(" ms");
        diagnostic.is_enabled = enabled;
        // Discovery and adoption must preserve the native object and its existing history.
        diagnostic.add_measurement(DiagnosticMeasurement {
            time:  std::time::Instant::now(),
            value: 17.0,
        });
        app.world_mut()
            .resource_mut::<DiagnosticsStore>()
            .add(diagnostic);
        let selection = test_support::latest("native/late");
        let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
        assert!(matches!(
            subscription.status(),
            SubscriptionStatus::PendingActivation
        ));
        app.update();
        assert!(matches!(
            subscription.status(),
            SubscriptionStatus::Active { .. }
        ));
        assert_eq!(test_support::enabled(&app, "native/late"), enabled);
        test_support::with_catalogue(&mut app, |catalogue| {
            let descriptor = match catalogue.lookup(&test_support::path("native/late")) {
                MetricLookup::Registered(descriptor) => descriptor,
                MetricLookup::Unregistered => {
                    return Err("late diagnostic was not discovered".into());
                },
            };
            assert_eq!(
                descriptor.unit(),
                MetricUnit::Unspecified,
                "suffix is not parsed as a unit"
            );
            assert_eq!(descriptor.supported_statistics(), &[StatisticKind::Latest]);
            Ok(())
        })?;
        let before =
            test_support::available(test_support::read(&mut app, &subscription, &selection)?)?;
        assert_eq!(before.unit, MetricUnit::Unspecified);
        assert_eq!(test_support::scalar(&before)?, 17.0);
        describe_diagnostic(
            &mut app,
            NativeDiagnosticDescription::new(
                test_support::path("native/late"),
                "Adopted latency",
                MetricUnit::Milliseconds,
                GaugeSemantics::Level,
            ),
        )?;
        test_support::with_catalogue(&mut app, |catalogue| {
            let descriptor = match catalogue.lookup(&test_support::path("native/late")) {
                MetricLookup::Registered(descriptor) => descriptor,
                MetricLookup::Unregistered => return Err("adopted descriptor disappeared".into()),
            };
            assert_eq!(descriptor.label(), "Adopted latency");
            assert_eq!(descriptor.unit(), MetricUnit::Milliseconds);
            assert_eq!(descriptor.supported_statistics(), &[StatisticKind::Latest]);
            assert!(descriptor.result_unit(&MetricStatistic::Smoothed).is_err());
            Ok(())
        })?;
        assert_eq!(test_support::enabled(&app, "native/late"), enabled);
        assert_eq!(test_support::values(&app, "native/late"), [17.0]);
    }
    Ok(())
}

#[test]
fn reading_native_diagnostics_present_at_finish_are_catalogued_without_demand()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let mut diagnostic = Diagnostic::new(test_support::path("native/startup"));
    diagnostic.is_enabled = false;
    app.world_mut()
        .resource_mut::<DiagnosticsStore>()
        .add(diagnostic);
    app.finish();
    app.cleanup();
    test_support::with_catalogue(&mut app, |catalogue| {
        match catalogue.lookup(&test_support::path("native/startup")) {
            MetricLookup::Registered(descriptor) => {
                assert_eq!(descriptor.unit(), MetricUnit::Unspecified);
                assert_eq!(descriptor.capability(), crate::MetricCapability::LatestOnly);
            },
            MetricLookup::Unregistered => {
                return Err("finish must discover existing native diagnostics".into());
            },
        }
        Ok(())
    })?;
    assert!(!test_support::enabled(&app, "native/startup"));
    Ok(())
}

#[test]
fn reading_descriptor_statistics_and_units_follow_each_declaration() -> Result<(), Box<dyn Error>> {
    let mut app = test_support::descriptor_app()?;
    let distribution = [
        StatisticKind::Latest,
        StatisticKind::Mean,
        StatisticKind::Median,
        StatisticKind::Maximum,
        StatisticKind::Percentile,
    ];
    let cases = [
        ("test/level", MetricUnit::Points, false, false, false),
        ("test/additive", MetricUnit::Count, true, false, false),
        ("test/mean", MetricUnit::Microseconds, false, false, false),
        ("test/rate", MetricUnit::Hertz, false, false, false),
        ("test/smoothed", MetricUnit::Count, false, false, true),
        ("test/count", MetricUnit::Count, false, true, false),
        ("test/bytes", MetricUnit::Bytes, false, true, false),
        ("test/span", MetricUnit::Seconds, true, false, false),
    ];
    test_support::with_catalogue(&mut app, |catalogue| {
        for (metric, base_unit, additive, counter, smoothing) in cases {
            let descriptor = match catalogue.lookup(&test_support::path(metric)) {
                MetricLookup::Registered(descriptor) => descriptor,
                MetricLookup::Unregistered => {
                    return Err(format!("missing descriptor {metric}").into());
                },
            };
            let mut expected = if counter {
                vec![
                    StatisticKind::Latest,
                    StatisticKind::Delta,
                    StatisticKind::Rate,
                ]
            } else {
                distribution.to_vec()
            };
            if additive {
                expected.push(StatisticKind::Sum);
            }
            if smoothing {
                expected.push(StatisticKind::Smoothed);
            }
            assert_eq!(
                descriptor.supported_statistics().len(),
                expected.len(),
                "{metric}"
            );
            for kind in expected {
                assert!(
                    descriptor.supported_statistics().contains(&kind),
                    "{metric}: {kind:?}"
                );
            }
            assert_eq!(descriptor.result_unit(&MetricStatistic::Latest)?, base_unit);
            assert_eq!(
                descriptor.result_unit(&MetricStatistic::Smoothed).is_ok(),
                smoothing
            );
            for reduction in [
                WindowReduction::Mean,
                WindowReduction::Median,
                WindowReduction::Maximum,
                WindowReduction::Sum,
                WindowReduction::Delta,
                WindowReduction::Rate,
                WindowReduction::Percentile(Percentile::try_new(0.95)?),
            ] {
                let supported = match reduction {
                    WindowReduction::Sum => additive,
                    WindowReduction::Delta | WindowReduction::Rate => counter,
                    _ => !counter,
                };
                let result = descriptor.result_unit(&MetricStatistic::Window {
                    window: SampleWindow::LastSamples(test_support::nonzero(2)),
                    reduction,
                });
                if supported {
                    let expected_unit = if reduction == WindowReduction::Rate {
                        if base_unit == MetricUnit::Bytes {
                            MetricUnit::BytesPerSecond
                        } else {
                            MetricUnit::CountPerSecond
                        }
                    } else {
                        base_unit
                    };
                    assert_eq!(result?, expected_unit, "{metric}: {reduction:?}");
                } else {
                    assert!(result.is_err(), "{metric}: {reduction:?}");
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn reading_last_samples_warms_up_and_reports_ordered_snapshot_coverage()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let window = SampleWindow::LastSamples(test_support::nonzero(2));
    let selection = test_support::reduction("test/level", window, WindowReduction::Mean);
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    recorder.record_with(|| 2.0);
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::WarmingUp(_))
    ));
    assert!(matches!(
        test_support::series(&mut app, &subscription, "test/level", &window)?,
        MetricSeries::Unavailable(UnavailableReason::WarmingUp(_))
    ));
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_millis(1));
    recorder.record_with(|| 6.0);
    app.update();
    let snapshot = match test_support::series(&mut app, &subscription, "test/level", &window)? {
        MetricSeries::Available(snapshot) => snapshot,
        other => return Err(format!("expected complete sample window: {other:?}").into()),
    };
    assert_eq!(
        snapshot.coverage.requested,
        crate::RequestedWindow::Samples(2)
    );
    assert_eq!(snapshot.coverage.sample_count, 2);
    assert_eq!(snapshot.coverage.boundary, BoundaryCoverage::Complete);
    assert_eq!(
        snapshot.coverage.observed,
        crate::ObservedBoundaries::Observed {
            start: RunOffset::new(Duration::ZERO),
            end:   RunOffset::new(Duration::from_millis(1)),
        }
    );
    assert_eq!(
        snapshot.coverage.sequences,
        crate::SequenceRange::Inclusive {
            first: snapshot.samples[0].sequence,
            last:  snapshot.samples[1].sequence,
        }
    );
    assert!(snapshot.samples[0].sequence < snapshot.samples[1].sequence);
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app,
            &subscription,
            &selection
        )?)?)?,
        4.0
    );
    Ok(())
}

#[test]
fn reading_a_selection_or_path_outside_the_subscription_is_dormant_even_when_active()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let window = SampleWindow::LastSamples(test_support::nonzero(1));
    let mean = test_support::reduction("test/level", window, WindowReduction::Mean);
    let owner = subscribe_metrics(
        app.world_mut(),
        &[test_support::latest("test/level"), mean.clone()],
    )?;
    let latest_only = subscribe_metrics(app.world_mut(), &[test_support::latest("test/level")])?;
    let empty = subscribe_metrics(app.world_mut(), &[])?;
    app.update();
    recorder.record_with(|| 5.0);
    app.update();
    assert_eq!(
        test_support::scalar(&test_support::available(test_support::read(
            &mut app, &owner, &mean
        )?)?)?,
        5.0
    );
    assert!(matches!(
        test_support::read(&mut app, &latest_only, &mean)?,
        MetricReading::Unavailable(UnavailableReason::Dormant)
    ));
    assert!(matches!(
        test_support::read(&mut app, &empty, &test_support::latest("test/level"))?,
        MetricReading::Unavailable(UnavailableReason::Dormant)
    ));
    assert!(matches!(
        test_support::series(&mut app, &empty, "test/level", &window)?,
        MetricSeries::Unavailable(UnavailableReason::Dormant)
    ));
    Ok(())
}

#[test]
fn reading_counter_rate_with_no_positive_baseline_elapsed_time_is_warming_up()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(
        &mut app,
        MetricDefinition::<Counter>::new(
            test_support::path("test/events"),
            "Events",
            CounterUnit::Count,
        ),
    )?;
    let selection = test_support::reduction(
        "test/events",
        SampleWindow::LastSamples(test_support::nonzero(1)),
        WindowReduction::Rate,
    );
    let subscription = subscribe_metrics(app.world_mut(), std::slice::from_ref(&selection))?;
    app.update();
    recorder.increment_with(|| 5);
    app.update();
    assert!(matches!(
        test_support::read(&mut app, &subscription, &selection)?,
        MetricReading::Unavailable(UnavailableReason::WarmingUp(_))
    ));
    app.world()
        .resource::<ManualRunClock>()
        .advance(Duration::from_secs(2));
    recorder.increment_with(|| 3);
    app.update();
    let estimate =
        test_support::available(test_support::read(&mut app, &subscription, &selection)?)?;
    assert_eq!(test_support::scalar(&estimate)?, 1.5);
    Ok(())
}

#[test]
fn reading_an_epoch_marked_ended_reports_source_ended_for_scalar_and_series()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    let recorder = register_metric(&mut app, test_support::gauge_definition("test/level"))?;
    let window = SampleWindow::LastSamples(test_support::nonzero(1));
    let subscription = subscribe_metrics(
        app.world_mut(),
        &[
            test_support::latest("test/level"),
            test_support::reduction("test/level", window, WindowReduction::Mean),
        ],
    )?;
    app.update();
    recorder.record_with(|| 5.0);
    app.update();
    let epoch = {
        let mut registry = app.world_mut().resource_mut::<MetricRegistry>();
        let entry = registry
            .entries
            .get_mut(&test_support::path("test/level"))
            .ok_or("registered source")?;
        entry.activity = SourceActivity::Ended;
        entry.epoch
    };
    assert!(
        matches!(test_support::read(&mut app, &subscription, &test_support::latest("test/level"))?,
        MetricReading::Unavailable(UnavailableReason::SourceEnded(ended)) if ended == epoch)
    );
    assert!(
        matches!(test_support::series(&mut app, &subscription, "test/level", &window)?,
        MetricSeries::Unavailable(UnavailableReason::SourceEnded(ended)) if ended == epoch)
    );
    Ok(())
}

#[test]
fn reading_discovered_native_selection_round_trips_its_single_segment_path()
-> Result<(), Box<dyn Error>> {
    let mut app = test_support::metrics_app();
    app.world_mut()
        .resource_mut::<DiagnosticsStore>()
        .add(Diagnostic::new(test_support::path("fps")));
    app.finish();
    app.cleanup();
    let selection = test_support::with_catalogue(&mut app, |catalogue| {
        match catalogue.lookup(&test_support::path("fps")) {
            MetricLookup::Registered(descriptor) => Ok(MetricSelection {
                path:      descriptor.path().clone(),
                statistic: MetricStatistic::Latest,
            }),
            MetricLookup::Unregistered => Err("native fps was not discovered".into()),
        }
    })?;
    let saved = serde_json::to_value(&selection)?;
    assert_eq!(saved["path"], "fps");
    assert_eq!(saved["native"], true);
    let restored: MetricSelection = serde_json::from_value(saved)?;
    assert_eq!(restored, selection);
    let subscription = subscribe_metrics(app.world_mut(), &[restored])?;
    app.update();
    assert!(matches!(
        subscription.status(),
        SubscriptionStatus::Active { .. }
    ));
    Ok(())
}
