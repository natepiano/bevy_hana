use hana_diegetic::El;
use hana_diegetic::Tooltip;

fn main() {
    let widget = El::new().button("nested");
    let _ = Tooltip::new(widget);
}
