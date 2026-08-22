use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;

fn main() {
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(El::new().button("outer"), |builder| {
        builder.with(El::new().button("inner"), |_| {});
    });
}
