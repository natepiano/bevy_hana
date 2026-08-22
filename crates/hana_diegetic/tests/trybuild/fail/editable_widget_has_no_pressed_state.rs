use bevy::color::Color;
use hana_diegetic::Appearance;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;

fn main() {
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
    let mut builder = LayoutBuilder::new(100.0, 50.0);
    builder.with(El::new().editable_field("editable", field), |builder| {
        builder.with(
            builder
                .child(El::new().background(Color::WHITE))
                .pressed(Appearance::new().background(Color::BLACK)),
            |_| {},
        );
    });
}
