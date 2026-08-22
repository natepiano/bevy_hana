use bevy::color::Color;
use hana_diegetic::EditorStateColors;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;

fn main() {
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
    let _field = El::new().editable_field("editable", field).editor_caret(
        EditorStateColors::new()
            .focused(Color::BLACK)
            .text_color(Color::WHITE),
    );
}
