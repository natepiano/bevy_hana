use hana_diegetic::El;
use hana_diegetic::Tooltip;

fn main() {
    let mut tooltip = Tooltip::new(El::new());
    tooltip.with(El::new().button("nested"), |_| {});
}
