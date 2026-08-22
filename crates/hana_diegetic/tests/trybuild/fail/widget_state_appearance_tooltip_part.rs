use bevy::color::Color;
use hana_diegetic::Appearance;
use hana_diegetic::El;
use hana_diegetic::Tooltip;

fn main() {
    let _ = Tooltip::new(
        El::new()
            .background(Color::WHITE)
            .disabled(Appearance::new().background(Color::BLACK)),
    );
}
