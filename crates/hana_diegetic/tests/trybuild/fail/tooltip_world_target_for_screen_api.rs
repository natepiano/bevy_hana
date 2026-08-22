use hana_diegetic::Screen;
use hana_diegetic::TooltipTarget;
use hana_diegetic::TooltipTargetEntity;
use hana_diegetic::World;

fn requires_screen_target<Target>(_target: Target)
where
    Target: TooltipTarget<Space = Screen>,
{
}

fn reject_world_target(target: TooltipTargetEntity<World>) {
    requires_screen_target(target);
}

fn main() {}
