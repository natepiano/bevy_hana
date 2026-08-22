use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;

fn main() {
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
    let _ = El::new()
        .editable_field("editable", field)
        .button("button");
}
