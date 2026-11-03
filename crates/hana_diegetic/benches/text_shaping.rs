//! Production Parley measurement, with the panel layout's measurement cache.
//!
//! Every iteration measures one complete status or sibling-footer workload.
//! `cold_shape` starts with an empty cache; `cached_shape` primes every text;
//! `measure_style_change` primes the original size outside the timed portion,
//! then measures a different size, alternating that size each iteration.
//! Font registration and reusable Parley buffers are outside all timed loops.
//! Batched cases return their cache so Criterion drops it outside the timing.

mod fixtures;

use std::hint::black_box;

use criterion::BatchSize;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use fixtures::measurement::FONT_SIZE;
use fixtures::rows::ROW_COUNTS;
use fixtures::rows::generate_status_rows;
use hana_diegetic::FontRegistry;
use hana_diegetic::MeasureTextFn;
use hana_diegetic::TextDimensions;
use hana_diegetic::TextMeasure;
use hana_diegetic::TextStyle;
use hana_diegetic::bench_support::ShapedTextCache;
use hana_diegetic::bench_support::create_parley_measurer;

fn bench_text_shaping(c: &mut Criterion) {
    let registry = FontRegistry::new().unwrap_or_else(|error| {
        eprintln!("could not register benchmark fonts: {error}");
        std::process::exit(1);
    });
    let measurer = create_parley_measurer(registry.font_context(), registry.family_names());
    for row_count in ROW_COUNTS {
        let rows = generate_status_rows(row_count);
        let status: Vec<&str> = ["STATUS", "BENCH"]
            .into_iter()
            .chain(rows.iter().flat_map(|(label, value)| [*label, *value]))
            .collect();
        bench_workload(
            c,
            &format!("shape_status_{row_count}_rows"),
            &status,
            &measurer,
        );

        let counts: Vec<String> = (1..=row_count)
            .map(|row| format!("{row} of {row_count}"))
            .collect();
        let footer: Vec<&str> = counts
            .iter()
            .flat_map(|count| {
                [
                    "↑↓",
                    "to navigate",
                    "-",
                    "⏎",
                    "to use",
                    "-",
                    "esc",
                    "to dismiss",
                    count.as_str(),
                ]
            })
            .collect();
        bench_workload(
            c,
            &format!("shape_footer_sibling_{row_count}_rows"),
            &footer,
            &measurer,
        );
        bench_rich_footer(c, row_count, &registry);
    }
}

fn bench_workload(c: &mut Criterion, name: &str, texts: &[&str], measurer: &MeasureTextFn) {
    let base_measure = TextStyle::new(FONT_SIZE).as_measure();
    let mut group = c.benchmark_group(name);

    group.bench_function("cold_shape", |b| {
        b.iter_batched(
            ShapedTextCache::default,
            |cache| {
                measure_workload(black_box(texts), &base_measure, measurer, &cache);
                cache
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("cached_shape", |b| {
        let cache = ShapedTextCache::default();
        measure_workload(texts, &base_measure, measurer, &cache);
        for text in texts {
            assert!(cache.get_measurement(text, &base_measure).is_some());
        }
        b.iter(|| measure_workload(black_box(texts), &base_measure, measurer, &cache));
    });

    group.bench_function("measure_style_change", |b| {
        let mut larger = false;
        b.iter_batched(
            || {
                let cache = ShapedTextCache::default();
                measure_workload(texts, &base_measure, measurer, &cache);
                larger = !larger;
                let size = if larger {
                    FONT_SIZE + 2.0
                } else {
                    FONT_SIZE + 4.0
                };
                let changed_measure = TextStyle::new(size).as_measure();
                for text in texts {
                    assert!(cache.get_measurement(text, &changed_measure).is_none());
                }
                (cache, changed_measure)
            },
            |(cache, measure)| {
                measure_workload(black_box(texts), &measure, measurer, &cache);
                cache
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn measure_workload(
    texts: &[&str],
    measure: &TextMeasure,
    measurer: &MeasureTextFn,
    cache: &ShapedTextCache,
) {
    for text in texts {
        let dimensions = measure_cached(text, measure, measurer, cache);
        black_box(dimensions);
    }
}

// Matches `panel::compute_layout::build_cached_measure`: only a miss calls Parley.
fn measure_cached(
    text: &str,
    measure: &TextMeasure,
    measurer: &MeasureTextFn,
    cache: &ShapedTextCache,
) -> TextDimensions {
    if let Some(dimensions) = cache.get_measurement(text, measure) {
        return dimensions;
    }
    let dimensions = measurer(text, measure);
    cache.insert_measurement(text, measure, dimensions);
    dimensions
}

criterion_group!(benches, bench_text_shaping);
criterion_main!(benches);

fn rich_footer_paragraph() -> hana_diegetic::RichParagraph {
    use bevy::color::Color;
    use hana_diegetic::SpanStyle;
    use hana_diegetic::bench_support::rich_paragraph;
    let bright = SpanStyle::new().with_color(Color::WHITE);
    let separator = SpanStyle::new().with_word_spacing((-FONT_SIZE).mul_add(0.6, 4.0));
    rich_paragraph(
        TextStyle::new(FONT_SIZE).with_color(Color::srgb(0.6, 0.6, 0.6)),
        |rich| {
            rich.span(("↑↓", bright))
                .span((" ", separator))
                .span("to navigate")
                .span((" ", separator))
                .span("-")
                .span((" ", separator))
                .span(("⏎", bright))
                .span((" ", separator))
                .span("to use")
                .span((" ", separator))
                .span("-")
                .span((" ", separator))
                .span(("esc", bright))
                .span((" ", separator))
                .span("to dismiss");
        },
    )
}

fn measure_rich_footer(
    paragraphs: &[hana_diegetic::RichParagraph],
    counts: &[String],
    rich: &dyn Fn(&hana_diegetic::RichParagraph, f32) -> TextDimensions,
    plain: &MeasureTextFn,
    cache: &ShapedTextCache,
) {
    let measure = TextStyle::new(FONT_SIZE).as_measure();
    for (paragraph, count) in paragraphs.iter().zip(counts) {
        black_box(rich(black_box(paragraph), 1.0));
        black_box(measure_cached(black_box(count), &measure, plain, cache));
    }
}

fn bench_rich_footer(c: &mut Criterion, rows: usize, registry: &FontRegistry) {
    use hana_diegetic::bench_support::create_parley_rich_measurer;
    let paragraphs = vec![rich_footer_paragraph(); rows];
    let counts: Vec<String> = (1..=rows).map(|row| format!("{row} of {rows}")).collect();
    let plain = create_parley_measurer(registry.font_context(), registry.family_names());
    let mut group = c.benchmark_group(format!("shape_footer_rich_{rows}_rows"));
    group.bench_function("rich_footer_cold_shape", |b| {
        b.iter_batched(
            || {
                let cache = ShapedTextCache::default();
                let rich = create_parley_rich_measurer(registry, cache.clone());
                (cache, rich)
            },
            |(cache, rich)| {
                measure_rich_footer(&paragraphs, &counts, rich.as_ref(), &plain, &cache);
                cache
            },
            BatchSize::SmallInput,
        );
    });
    let cache = ShapedTextCache::default();
    let rich = create_parley_rich_measurer(registry, cache.clone());
    measure_rich_footer(&paragraphs, &counts, rich.as_ref(), &plain, &cache);
    group.bench_function("rich_footer_cached_shape", |b| {
        b.iter(|| measure_rich_footer(&paragraphs, &counts, rich.as_ref(), &plain, &cache));
    });
    group.bench_function("rich_footer_color_only", |b| {
        let mut recoloured = paragraphs.clone();
        let mut bright = false;
        b.iter(|| {
            bright = !bright;
            let color = if bright {
                bevy::color::Color::WHITE
            } else {
                bevy::color::Color::BLACK
            };
            for paragraph in &mut recoloured {
                for run in &mut paragraph.runs {
                    run.color = color;
                }
            }
            measure_rich_footer(&recoloured, &counts, rich.as_ref(), &plain, &cache);
        });
    });
    group.finish();
}
