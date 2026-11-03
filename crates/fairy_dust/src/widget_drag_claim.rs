//! Bridges widget-owned primary presses to camera drag routing in example apps.

use bevy::picking::PickingSystems;
use bevy::prelude::*;
use hana_diegetic::WidgetPrimaryPressOwnership;
use hana_lagrange::CameraDragClaim;
use hana_lagrange::CameraInputPhase;

/// Gives a held widget press sole ownership of pointer drag input.
pub struct WidgetDragClaimPlugin;

impl Plugin for WidgetDragClaimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            sync_widget_drag_claim
                .after(PickingSystems::Hover)
                .before(CameraInputPhase::PreInput),
        );
    }
}

fn sync_widget_drag_claim(
    ownership: Res<WidgetPrimaryPressOwnership>,
    mut claim: ResMut<CameraDragClaim>,
) {
    let next = match *ownership {
        WidgetPrimaryPressOwnership::Unclaimed => CameraDragClaim::Unclaimed,
        WidgetPrimaryPressOwnership::HeldByWidget => CameraDragClaim::Claimed,
    };
    if *claim != next {
        *claim = next;
    }
}
