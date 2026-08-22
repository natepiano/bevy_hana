use bevy::color::Color;
use hana_diegetic::ChildDivider;
use hana_diegetic::El;
use hana_diegetic::In;

fn main() {
    let _ = El::overlay().child_divider(ChildDivider::new(In(0.01), Color::WHITE));
}
