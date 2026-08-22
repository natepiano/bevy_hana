use bevy::asset::Handle;
use bevy::color::Color;
use bevy::image::Image;
use hana_diegetic::El;
use hana_diegetic::Tooltip;

fn main() {
    let mut tooltip = Tooltip::new(El::new());
    tooltip.image(
        El::new().button("nested"),
        Handle::<Image>::default(),
        Color::WHITE,
    );
}
