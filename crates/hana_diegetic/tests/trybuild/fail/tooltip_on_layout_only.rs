use hana_diegetic::El;
use hana_diegetic::Tooltip;

fn main() {
    let tooltip = Tooltip::new(El::new());
    let _ = El::new().tooltip(tooltip);
}
