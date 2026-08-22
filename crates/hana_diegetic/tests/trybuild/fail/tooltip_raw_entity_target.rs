use bevy::ecs::system::Commands;
use bevy::prelude::Entity;
use hana_diegetic::El;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipCommandsExt;

fn raw_entity_target(commands: &mut Commands, target: Entity) {
    commands.spawn_tooltip(target, Tooltip::new(El::new()));
}

fn main() {}
