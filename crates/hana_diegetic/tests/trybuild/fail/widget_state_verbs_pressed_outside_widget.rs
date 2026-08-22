use bevy::color::Color;
use hana_diegetic::Appearance;
use hana_diegetic::El;

fn main() {
    let _ = El::new()
        .background(Color::WHITE)
        .pressed(Appearance::new().background(Color::BLACK));
}
