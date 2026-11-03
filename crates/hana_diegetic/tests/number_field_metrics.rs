//! Numeric field outlines belong to their field metrics and layout unit.

use hana_diegetic::ControlMetrics;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutEngine;
use hana_diegetic::Px;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;

#[test]
fn number_field_metrics_outline_scales_in_the_fields_unit() -> Result<(), String> {
    for label_size in [13.0, 26.0] {
        let metrics = ControlMetrics::from_label_size(label_size);
        let builder = LayoutBuilder::with_widget_root(El::row().slider_value_field(
            "value",
            ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("value")),
            &metrics,
        ));
        let tree = builder.build();
        let layout = LayoutEngine::new(DiegeticTextMeasurer::default().measure_fn)
            .compute(&tree, 200.0, 50.0, 1.0);
        let border = layout
            .commands
            .iter()
            .find_map(|command| match command.kind {
                RenderCommandKind::Border { border } => Some(border),
                _ => None,
            })
            .ok_or("field outline")?;
        assert!((border.bottom.value - metrics.border_width).abs() < 1e-5);
        assert_eq!(border.left, border.right);
        assert_eq!(border.top, border.bottom);
        assert_eq!(border.left, border.top);
    }
    Ok(())
}

#[test]
fn number_field_metrics_outline_does_not_borrow_a_controls_changed_border() -> Result<(), String> {
    let mut metrics = ControlMetrics::from_label_size(Px(13.0));
    metrics.field.border_width = Px(2.0);
    metrics.border_width = Px(7.0);
    let builder = LayoutBuilder::with_widget_root(El::row().slider_value_field(
        "value",
        ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("value")),
        &metrics,
    ));
    let layout = LayoutEngine::new(DiegeticTextMeasurer::default().measure_fn).compute(
        &builder.build(),
        200.0,
        50.0,
        1.0,
    );
    let border = layout
        .commands
        .iter()
        .find_map(|command| match command.kind {
            RenderCommandKind::Border { border } => Some(border),
            _ => None,
        })
        .ok_or("field outline")?;
    assert_eq!(
        border.bottom.value, 2.0,
        "field supplies its own border width"
    );
    assert_eq!(border.left, border.right);
    assert_eq!(border.top, border.bottom);
    assert_eq!(border.left, border.top);
    Ok(())
}

#[test]
fn number_field_metrics_corners_do_not_borrow_a_buttons_changed_radius() {
    let mut metrics = ControlMetrics::from_label_size(Px(13.0));
    metrics.field.corner_radius = Px(7.0);
    let field_radius = metrics.field.corner_radius;
    metrics.button.corner_radius = Px(21.0);
    let mut builder = LayoutBuilder::with_widget_root(El::row().slider_value_field(
        "value",
        ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("value")),
        &metrics,
    ));
    builder.text(Text::new("0.5", TextStyle::new(metrics.label_size)));
    let tree = builder.build();
    let radii = tree.element_corner_radius(0).to_array();
    assert_eq!(radii, [field_radius.0; 4]);
}

#[test]
fn number_field_metrics_text_field_uses_the_same_frame_and_left_padding() -> Result<(), String> {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let spec = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("text"));
    let mut builder =
        LayoutBuilder::with_widget_root(El::row().text_value_field("text", spec, &metrics));
    builder.text(Text::new("hana", TextStyle::new(metrics.label_size)));
    let tree = builder.build();
    let layout = LayoutEngine::new(DiegeticTextMeasurer::default().measure_fn).compute(
        &tree,
        metrics.field.width,
        metrics.field.height,
        1.0,
    );
    let border = layout
        .commands
        .iter()
        .find_map(|command| match command.kind {
            RenderCommandKind::Border { border } => Some(border),
            _ => None,
        })
        .ok_or("text field outline")?;
    assert_eq!(border.left.value, metrics.field.border_width);
    assert_eq!(
        tree.element_corner_radius(0).to_array(),
        [metrics.field.corner_radius; 4]
    );
    let text = layout
        .commands
        .iter()
        .find(|command| matches!(command.kind, RenderCommandKind::Text { .. }))
        .ok_or("field text")?;
    assert!((text.bounds.x - metrics.field.side_padding - metrics.field.border_width).abs() < 1e-5);
    assert!((layout.computed[0].bounds.height - metrics.field.height).abs() < 1e-5);
    Ok(())
}
