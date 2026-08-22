use bevy::color::Color;
use hana_diegetic::Appearance;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;

fn main() {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(
        El::new()
            .background(Color::WHITE)
            .disabled(Appearance::new().background(Color::BLACK)),
        |_| {},
    );
}
