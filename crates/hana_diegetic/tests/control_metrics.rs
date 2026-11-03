//! Integration tests for shared control proportions in pixels and panel units.

use bevy::prelude::App;
use bevy::prelude::MinimalPlugins;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::Slider;
use hana_diegetic::SliderMetrics;
use hana_diegetic::SliderReachableBand;

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn pixel_switch_metrics_match_system_settings_proportions() {
    let metrics = ControlMetrics::from_label_size(Px(13.0));

    close(metrics.button.height.0, 22.0);
    close(metrics.switch.track_width.0, 33.0);
    close(metrics.switch.thumb.width.0, 19.5);
    close(metrics.switch.track_height.0, 15.0);
    close(
        metrics.switch.thumb.height.0,
        2.0_f32.mul_add(
            -(metrics.switch.track_padding.0 + metrics.switch.border_width.0),
            metrics.switch.track_height.0,
        ),
    );
    close(metrics.slider.width.0, 133.0);
    close(metrics.slider.value_width.0, 79.0);
    assert_eq!(SliderMetrics::<Px>::VALUE_CHARACTER_CAPACITY, 8);
    close(metrics.slider.thumb.height.0, 12.0);
    close(metrics.slider.thumb.width.0, 19.5);
    close(metrics.slider.thumb.corner_radius().0, 6.0);
    close(metrics.slider.track_height.0, 6.0);
    close(metrics.row.height.0, 30.0);
    close(metrics.row.gap.0, 6.5);
    close(metrics.field.width.0, 165.0);
    close(metrics.field.height.0, 18.0);

    close(
        2.0_f32.mul_add(
            metrics.switch.track_padding.0 + metrics.switch.border_width.0,
            metrics.switch.thumb.height.0,
        ),
        metrics.switch.track_height.0,
    );
    assert!(metrics.slider.thumb.height.0 < metrics.switch.track_height.0);
    assert!(metrics.field.height.0 < metrics.button.height.0);
    assert!(
        2.0_f32.mul_add(
            metrics.switch.track_padding.0 + metrics.switch.border_width.0,
            metrics.switch.thumb.width.0,
        ) <= metrics.switch.track_width.0
    );
    assert!(
        2.0_f32.mul_add(metrics.border_width.0, metrics.slider.thumb.height.0)
            <= metrics.row.height.0
    );
    assert!(
        2.0_f32.mul_add(metrics.border_width.0, metrics.label_size.0) <= metrics.button.height.0
    );
    assert!(
        2.0_f32.mul_add(
            metrics.field.border_width.0 + metrics.field.vertical_padding().0,
            metrics.label_size.0,
        ) <= metrics.field.height.0
    );
}

#[test]
fn panel_unit_switch_metrics_scale_from_label_size() {
    let metrics = ControlMetrics::from_label_size(0.013_f32);

    close(metrics.button.height, 0.022);
    close(metrics.switch.track_width, 0.033);
    close(metrics.switch.thumb.width, 0.0195);
    close(metrics.switch.track_height, 0.015);
    close(metrics.slider.width, 0.133);
    close(metrics.slider.value_width, 0.079);
    close(metrics.slider.thumb.width, 0.0195);
    close(metrics.slider.thumb.height, 0.012);
    close(metrics.slider.thumb.corner_radius(), 0.006);
    close(metrics.slider.track_height, 0.006);
    close(metrics.row.height, 0.030);
    close(metrics.field.width, 0.165);
    close(
        2.0_f32.mul_add(
            metrics.switch.track_padding + metrics.switch.border_width,
            metrics.switch.thumb.height,
        ),
        metrics.switch.track_height,
    );
}

#[test]
fn slider_thumb_and_switch_knob_share_one_capsule() -> Result<(), String> {
    for label in [9.0, 13.0, 26.0] {
        let metrics = ControlMetrics::from_label_size(Px(label));
        let thumb = metrics.slider.thumb;
        close(thumb.width.0 / metrics.switch.track_height.0, 52.0 / 40.0);
        close(thumb.height.0 / metrics.switch.track_height.0, 32.0 / 40.0);
        close(thumb.corner_radius().0, thumb.height.0 * 0.5);
        close(metrics.slider.thumb_hit_width.0, label * 20.0 / 13.0);
        close(metrics.slider.thumb_hit_height.0, label * 16.0 / 13.0);
        let fitted = ControlMetrics::from_label_size(label).with_painted_row_height();
        close(fitted.row.height, label * 18.0 / 13.0);

        let metrics = ControlMetrics::from_label_size(label);
        assert_eq!(metrics.slider.thumb, metrics.switch.thumb);
        let mut builder = LayoutBuilder::new(600.0, 200.0);
        builder.with(El::row().size(600.0, 200.0), |builder| {
            builder.slider(
                "level",
                Slider::new(0.0..=1.0),
                &metrics,
                SliderReachableBand::Hidden,
            );
            builder.with(
                El::new()
                    .size(metrics.switch.track_width, metrics.switch.track_height)
                    .switch("power"),
                |builder| {
                    builder.switch_knob(&metrics.switch);
                },
            );
        });
        let tree = builder.build();
        let panel = DiegeticPanel::world()
            .size(Pt(600.0), Pt(200.0))
            .with_tree(tree.clone())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let computed = app.world().get::<ComputedDiegeticPanel>(panel);
        let layout = computed
            .and_then(|computed| computed.layout().solved())
            .ok_or("capsules should solve")?;
        let capsules = layout.commands.iter().filter(|command| {
            matches!(command.kind, RenderCommandKind::Rectangle { color, .. } if color == metrics.slider.thumb.fill)
        }).collect::<Vec<_>>();
        assert_eq!(capsules.len(), 2, "one painted capsule per control");
        for capsule in capsules {
            close(capsule.bounds.width, metrics.slider.thumb.width);
            close(capsule.bounds.height, metrics.slider.thumb.height);
            close(
                tree.element_corner_radius(capsule.element_idx)
                    .top_left
                    .value,
                metrics.slider.thumb.corner_radius(),
            );
        }
    }
    Ok(())
}

#[test]
fn spacing_uses_one_text_relative_scale_for_panels_and_cards() {
    for label_size in [Px(13.0), Px(14.667), Px(20.0)] {
        let spacing = ControlMetrics::from_label_size(label_size).spacing;
        let unit = label_size.0 / 4.0;
        close(spacing.step().0, unit);
        close(spacing.panel_padding.0, 6.0 * unit);
        close(spacing.panel_bottom.0, 8.0 * unit);
        close(spacing.title_gap.0, 4.0 * unit);
        close(spacing.card_gap.0, spacing.title_gap.0);
        close(spacing.card_padding.0, 4.0 * unit);
        close(spacing.row_gap.0, 2.0 * unit);
        assert!(spacing.panel_bottom.0 >= spacing.panel_padding.0);
        assert!(spacing.row_gap.0 < spacing.card_padding.0);
    }
}

#[test]
fn panel_unit_spacing_keeps_face_edges_and_grouping_in_proportion() {
    let spacing = ControlMetrics::from_label_size(0.013_f32).spacing;
    close(spacing.panel_padding, 0.0195);
    close(spacing.panel_bottom, 0.026);
    close(spacing.title_gap, spacing.card_gap);
    close(spacing.card_padding, spacing.card_gap);
    close(spacing.row_gap, 0.0065);
    assert!(spacing.panel_bottom >= spacing.panel_padding);
    assert!(spacing.row_gap < spacing.card_padding);
}
