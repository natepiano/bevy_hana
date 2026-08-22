use bevy::prelude::Color;
use hana_diegetic::El;

const RED: Color = Color::srgb(1.0, 0.0, 0.0);

fn main() {
    let _button = El::new().button("button").hovered(RED);
}
