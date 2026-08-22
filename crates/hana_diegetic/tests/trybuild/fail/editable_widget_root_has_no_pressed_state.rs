use bevy::color::Color;
use hana_diegetic::Appearance;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;

fn main() {
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
    let _field = El::new()
        .editable_field("editable", field)
        .pressed(Appearance::new().background(Color::BLACK));
}
