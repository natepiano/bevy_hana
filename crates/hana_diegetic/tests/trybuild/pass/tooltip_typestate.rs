use bevy::asset::Handle;
use bevy::color::Color;
use bevy::ecs::system::Commands;
use bevy::image::Image;
use bevy::prelude::Entity;
use hana_diegetic::ChildDivider;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::MeshAnchorCommandsExt;
use hana_diegetic::MeshFace;
use hana_diegetic::PanelEntity;
use hana_diegetic::Slider;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipCommandsExt;
use hana_diegetic::TooltipTarget;
use hana_diegetic::TooltipTargetEntity;
use hana_diegetic::WidgetEntity;
use hana_diegetic::World;

struct ApplicationWorldTarget(Entity);

impl TooltipTarget for ApplicationWorldTarget {
    type Space = World;

    fn tooltip_target_entity(&self) -> Entity { self.0 }
}

fn associated_tooltips(slider: Slider) {
    let tooltip = Tooltip::new(El::new());
    let _ = El::new().button("button").tooltip(tooltip.clone());
    let _ = El::new().widget("slider", slider).tooltip(tooltip);
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
    let _ = El::new()
        .editable_field("editable", field)
        .tooltip(Tooltip::new(El::new()));
}

fn post_widget_row_and_column_builders(slider: Slider) {
    let divider = ChildDivider::new(1.0, Color::WHITE);
    let _ = El::row()
        .button("row-button")
        .gap(2.0)
        .child_divider(divider);
    let _ = El::column()
        .widget("column-slider", slider)
        .gap(3.0)
        .child_divider(divider);
}

fn typed_targets(
    commands: &mut Commands,
    panel: PanelEntity<World>,
    widget: WidgetEntity<World>,
    general: TooltipTargetEntity<World>,
    application: ApplicationWorldTarget,
) {
    commands.spawn_tooltip(panel, Tooltip::new(El::new()));
    commands.spawn_tooltip(widget, Tooltip::new(El::new()));
    commands.spawn_tooltip(general, Tooltip::new(El::new()));
    commands.spawn_tooltip(application, Tooltip::new(El::new()));
}

fn checked_mesh_target(commands: &mut Commands, entity: Entity) {
    let target = commands.mesh_anchor_target(entity, MeshFace::PositiveZ);
    commands.spawn_tooltip(target, Tooltip::new(El::new()));
}

fn main() {
    let mut tooltip = Tooltip::new(El::column());
    tooltip.with(El::new(), |tooltip| {
        tooltip.text("plain");
        tooltip.text(("styled", TextStyle::default()));
    });
    tooltip.image(El::new(), Handle::<Image>::default(), Color::WHITE);

    let mut panel = LayoutBuilder::new(100.0, 50.0);
    panel.text(
        Text::new("button text", TextStyle::default()).layout(El::new().button("text-button")),
    );

    let _ = associated_tooltips;
    let _ = post_widget_row_and_column_builders;
    let _ = typed_targets;
    let _ = checked_mesh_target;
}
