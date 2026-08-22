use bevy::color::Color;
use hana_diegetic::AcceptsElement;
use hana_diegetic::Appearance;
use hana_diegetic::Button;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::WidgetElement;

fn main() {
    let mut panel = LayoutBuilder::new(100.0, 50.0);

    <LayoutBuilder as AcceptsElement<WidgetElement<Button>>>::with_child_builder(
        &mut panel,
        |widget| {
            widget.with(
                widget
                    .child(El::new().background(Color::WHITE))
                    .disabled(Appearance::new().background(Color::BLACK)),
                |_| {},
            );
        },
    );

    let _ = panel.build();
}
