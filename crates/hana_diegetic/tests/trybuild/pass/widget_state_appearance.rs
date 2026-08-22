use bevy::asset::Handle;
use bevy::color::Color;
use bevy::pbr::StandardMaterial;
use hana_diegetic::Appearance;
use hana_diegetic::Border;
use hana_diegetic::El;
use hana_diegetic::Px;
use hana_diegetic::Slider;

fn appearance() -> Appearance {
    Appearance::new()
        .background(Color::BLACK)
        .border_color(Color::WHITE)
        .border_width(Px(2.0))
        .material(Handle::<StandardMaterial>::default())
}

fn button() {
    let _ = El::new()
        .background(Color::WHITE)
        .border(Border::all(Px(0.0), Color::WHITE))
        .button("action")
        .hovered(appearance())
        .focused(appearance())
        .pressed(appearance())
        .disabled(appearance());
}

fn slider() {
    let _ = El::new()
        .background(Color::WHITE)
        .border(Border::all(Px(0.0), Color::WHITE))
        .widget("level", Slider::new(0.0..=1.0))
        .hovered(appearance())
        .focused(appearance())
        .pressed(appearance())
        .disabled(appearance());
}

fn main() {
    let _ = button;
    let _ = slider;
}
