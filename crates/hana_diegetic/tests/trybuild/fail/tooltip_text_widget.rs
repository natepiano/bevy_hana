use hana_diegetic::El;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;

fn main() {
    let widget_text = Text::new("nested", TextStyle::default())
        .layout(El::new().button("nested"));
    let mut tooltip = Tooltip::new(El::new());
    tooltip.text(widget_text);
}
