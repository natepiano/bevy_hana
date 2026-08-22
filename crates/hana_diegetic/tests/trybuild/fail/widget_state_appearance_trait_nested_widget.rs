use hana_diegetic::AcceptsElement;
use hana_diegetic::Button;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutContentBuilder;
use hana_diegetic::WidgetElement;

fn add_nested_widget(
    builder: &mut (impl LayoutContentBuilder + AcceptsElement<WidgetElement<Button>>),
) {
    builder.with(El::new().button("outer"), |builder| {
        builder.with(El::new().button("inner"), |_| {});
    });
}

fn main() {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    add_nested_widget(&mut builder);
}
