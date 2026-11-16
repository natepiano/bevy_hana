//! Constructor validation and the saved metric-selection contract.
#![allow(
    clippy::expect_used,
    reason = "tests stop when a valid fixture is rejected"
)]

use std::num::NonZeroUsize;
use std::time::Duration;

use bevy_diagnostic::DiagnosticPath;
use hana_metrics::IntervalGaugeSample;
use hana_metrics::MetricSelection;
use hana_metrics::MetricStatistic;
use hana_metrics::ObservationInterval;
use hana_metrics::Percentile;
use hana_metrics::RunOffset;
use hana_metrics::SampleWindow;
use hana_metrics::SmoothingPolicy;
use hana_metrics::WindowReduction;

#[test]
fn fixed_alpha_requires_a_finite_fraction_greater_than_zero() {
    for alpha in [
        f64::NAN,
        f64::NEG_INFINITY,
        f64::INFINITY,
        -0.1,
        -0.0,
        0.0,
        1.000_001,
    ] {
        assert!(
            SmoothingPolicy::fixed_alpha(alpha, Duration::from_secs(5)).is_err(),
            "accepted invalid smoothing alpha {alpha}"
        );
    }
    for alpha in [f64::MIN_POSITIVE, 0.2, 1.0] {
        assert!(
            SmoothingPolicy::fixed_alpha(alpha, Duration::from_secs(5)).is_ok(),
            "rejected valid smoothing alpha {alpha}"
        );
    }
}

#[test]
fn smoothing_requires_nonzero_time_constants_and_reset_intervals() {
    let positive = Duration::from_nanos(1);
    assert!(SmoothingPolicy::fixed_alpha(0.2, Duration::ZERO).is_err());
    assert!(SmoothingPolicy::time_based(Duration::ZERO, positive).is_err());
    assert!(SmoothingPolicy::time_based(positive, Duration::ZERO).is_err());
    assert!(SmoothingPolicy::time_based(positive, positive).is_ok());
}

#[test]
fn percentile_requires_a_finite_fraction_in_the_closed_unit_interval() {
    for fraction in [f64::NEG_INFINITY, f64::NAN, f64::INFINITY, -0.001, 1.001] {
        assert!(
            Percentile::try_new(fraction).is_err(),
            "accepted invalid percentile {fraction}"
        );
    }
    for fraction in [0.0, 0.5, 1.0] {
        assert!(
            Percentile::try_new(fraction).is_ok(),
            "rejected percentile {fraction}"
        );
    }
}

#[test]
fn observation_intervals_accept_ordered_and_equal_endpoints() {
    for (start, end) in [(0, 0), (7, 7), (0, 7), (7, 19)] {
        assert!(
            ObservationInterval::new(
                RunOffset::new(Duration::from_nanos(start)),
                RunOffset::new(Duration::from_nanos(end)),
            )
            .is_ok(),
            "rejected ordered interval {start}..{end}"
        );
    }
    assert!(
        ObservationInterval::new(
            RunOffset::new(Duration::from_nanos(19)),
            RunOffset::new(Duration::from_nanos(7)),
        )
        .is_err()
    );
}

#[test]
fn interval_gauges_reject_nonfinite_values_but_accept_finite_levels() {
    for value in [f64::NAN, f64::NEG_INFINITY, f64::INFINITY] {
        assert!(
            IntervalGaugeSample::new(value, source_interval()).is_err(),
            "accepted nonfinite gauge value {value}"
        );
    }
    for value in [-f64::MAX, -1.0, 0.0, 1.0, f64::MAX] {
        assert!(
            IntervalGaugeSample::new(value, source_interval()).is_ok(),
            "rejected finite gauge value {value}"
        );
    }
}

#[test]
fn saved_selections_store_the_path_string_and_preserve_instant_statistics() {
    for statistic in [MetricStatistic::Latest, MetricStatistic::Smoothed] {
        let selection = selection(statistic);
        let encoded = serde_json::to_value(&selection).expect("serialize a valid selection");
        assert_eq!(encoded["path"], "hana/frame/interval");
        let decoded: MetricSelection =
            serde_json::from_value(encoded).expect("deserialize a valid selection");
        assert_eq!(decoded.path.as_str(), "hana/frame/interval");
        assert!(matches!(
            (&selection.statistic, decoded.statistic),
            (MetricStatistic::Latest, MetricStatistic::Latest)
                | (MetricStatistic::Smoothed, MetricStatistic::Smoothed)
        ));
    }
}

#[test]
fn saved_selections_validate_paths_on_deserialization() {
    let valid = serde_json::to_value(selection(MetricStatistic::Latest))
        .expect("serialize a valid selection");
    for path in [
        "",
        "hana",
        "hana//interval",
        "/hana/frame",
        "hana/frame/",
        "Hana/frame",
        "hana/Frame",
        "hana/123",
        "hana/1frame",
        "hana/frame-time",
        "hana/frame time",
        "hana/fr\u{e4}me",
        "hana/_frame",
    ] {
        let mut encoded = valid.clone();
        encoded["path"] = serde_json::Value::from(path);
        assert!(
            serde_json::from_value::<MetricSelection>(encoded).is_err(),
            "accepted invalid saved path {path:?}"
        );
    }
    let mut encoded = valid;
    encoded["path"] = serde_json::Value::from("hana_2/frame_time9");
    let decoded: MetricSelection =
        serde_json::from_value(encoded).expect("accept letters, underscores and noninitial digits");
    assert_eq!(decoded.path.as_str(), "hana_2/frame_time9");
}

#[test]
fn saved_window_selections_preserve_both_window_variants_and_every_reduction()
-> Result<(), &'static str> {
    for trailing in [false, true] {
        for reduction in [
            WindowReduction::Mean,
            WindowReduction::Median,
            WindowReduction::Maximum,
            WindowReduction::Sum,
            WindowReduction::Delta,
            WindowReduction::Rate,
            WindowReduction::Percentile(Percentile::try_new(0.95).expect("valid percentile")),
        ] {
            let capacity = NonZeroUsize::new(37).expect("positive window capacity");
            let window = if trailing {
                SampleWindow::Trailing {
                    duration: Duration::new(5, 123),
                    capacity,
                }
            } else {
                SampleWindow::LastSamples(capacity)
            };
            let expected_reduction = serde_json::to_value(reduction).expect("serialize reduction");
            let encoded =
                serde_json::to_vec(&selection(MetricStatistic::Window { window, reduction }))
                    .expect("serialize a window selection");
            let decoded: MetricSelection =
                serde_json::from_slice(&encoded).expect("deserialize a window selection");
            assert_eq!(decoded.path.as_str(), "hana/frame/interval");
            let MetricStatistic::Window { window, reduction } = decoded.statistic else {
                return Err("window statistic changed during round trip");
            };
            match window {
                SampleWindow::LastSamples(samples) => {
                    assert!(!trailing);
                    assert_eq!(samples.get(), 37);
                },
                SampleWindow::Trailing { duration, capacity } => {
                    assert!(trailing);
                    assert_eq!(duration, Duration::new(5, 123));
                    assert_eq!(capacity.get(), 37);
                },
            }
            assert_eq!(
                serde_json::to_value(reduction).expect("serialize decoded reduction"),
                expected_reduction
            );
        }
    }
    Ok(())
}

#[test]
fn percentile_json_deserialization_applies_fraction_bounds() {
    for json in ["-0.001", "1.001", "-1e300", "1e300", "null"] {
        assert!(
            serde_json::from_str::<Percentile>(json).is_err(),
            "accepted percentile JSON {json}"
        );
    }
    for json in ["0.0", "0.5", "1.0"] {
        let percentile: Percentile = serde_json::from_str(json).expect("valid percentile JSON");
        let encoded = serde_json::to_value(percentile).expect("serialize a valid percentile");
        assert_eq!(
            encoded,
            serde_json::from_str::<serde_json::Value>(json).expect("JSON number")
        );
    }
}

fn source_interval() -> ObservationInterval {
    ObservationInterval::new(
        RunOffset::new(Duration::from_millis(500)),
        RunOffset::new(Duration::from_secs(1)),
    )
    .expect("ordered source interval")
}

fn selection(statistic: MetricStatistic) -> MetricSelection {
    MetricSelection {
        path: DiagnosticPath::new("hana/frame/interval"),
        statistic,
    }
}
