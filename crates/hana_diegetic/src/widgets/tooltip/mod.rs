mod attachment;
mod controller;
mod declaration;
mod materialization;
mod placement;
mod readiness;
mod target;
mod visibility;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::transform::TransformSystems;
use declaration::TooltipPhase;
use hana_valence::AnchorSystems;
#[cfg(test)]
pub(crate) use visibility::TooltipPointerCamera;

use self::attachment::ScreenTooltipAttachmentCorrection;
use self::attachment::attach_materialized_tooltips;
use self::attachment::clear_screen_tooltip_attachment_corrections;
use self::attachment::screen_tooltip_attachment_correction_requested;
pub(crate) use self::controller::ComputedTooltipRecord;
pub use self::controller::TooltipCommandsExt;
pub(crate) use self::controller::TooltipControllerIndex;
pub use self::controller::TooltipFor;
pub use self::controller::Tooltips;
pub(super) use self::controller::reify_tooltip_controllers;
pub(crate) use self::controller::remove_materialized_state;
#[cfg(test)]
use self::declaration::PrepareTooltip;
pub use self::declaration::Tooltip;
pub use self::declaration::TooltipDisabledPolicy;
pub(crate) use self::materialization::MaterializedTooltip;
pub(crate) use self::materialization::TooltipPaintInsideBorder;
use self::materialization::TooltipPresentationCamera;
use self::materialization::TooltipPresentationSpace;
#[cfg(test)]
use self::materialization::TooltipReadiness;
use self::materialization::TooltipSurfaceMaterial;
use self::materialization::TooltipWidthConstraint;
use self::materialization::TooltipWidthConstraintRequest;
use self::materialization::apply_tooltip_width_constraints;
use self::materialization::invalidate_stale_materialized_tooltips;
use self::materialization::invalidate_stale_world_tooltip_frames;
use self::materialization::live_window;
use self::materialization::materialize_requested_tooltips;
use self::materialization::zero_origin_viewport;
pub use self::placement::TooltipCardOccupiedRegions;
use self::placement::resolve_screen_tooltip_placements;
use self::placement::resolve_world_tooltip_placements;
use self::readiness::finalize_tooltip_readiness;
use self::readiness::tooltip_readiness_inputs_changed;
use self::readiness::tooltip_readiness_or_reveal;
pub use self::target::MeshAnchorCommandsExt;
pub(crate) use self::target::MeshAnchorWarnings;
pub use self::target::MeshFace;
pub use self::target::ScreenAnchorCommandsExt;
pub use self::target::TooltipTarget;
pub use self::target::TooltipTargetEntity;
pub use self::target::TooltipTargetSpace;
pub(super) use self::target::remove_mesh_anchor_geometry;
pub(super) use self::target::update_mesh_anchor_geometry;
pub(super) use self::target::warn_pending_mesh_anchor_geometry;
pub use self::visibility::TooltipHidden;
use self::visibility::TooltipHiddenEventKey;
pub use self::visibility::TooltipShown;
use self::visibility::advance_tooltip_visibility;
use self::visibility::emit_tooltip_shown;
use self::visibility::finalize_despawned_tooltip;
use self::visibility::finalize_removed_tooltip;
use self::visibility::finalize_tooltip_panel_role;
use self::visibility::remember_tooltip_camera_from_move;
use self::visibility::remember_tooltip_camera_from_over;
use self::visibility::remember_tooltip_camera_from_press;
use self::visibility::remove_tooltip_pointer_camera;
use self::visibility::retire_superseded_help_panels;
use self::visibility::reveal_ready_tooltips;
pub(crate) use self::visibility::select_window_presentation_camera;
use super::TooltipSystems;
use super::WidgetSystems;
use crate::PanelSystems;
use crate::panel;
use crate::panel::PanelSpace;
use crate::render::MaterialTableAppendReady;
use crate::render::PanelChildSystems;
use crate::screen_space::ScreenSpaceSystems;
use crate::screen_space::{self};

pub(super) struct TooltipPlugin;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
enum TooltipPlacementSystems {
    ScreenDecide,
    ScreenCommandsApplied,
    ScreenCorrection,
    WorldDecide,
    WorldCommandsApplied,
    OccludersResolved,
}

fn world_tooltip_placement_requested(
    tooltips: Query<(&TooltipPhase, &MaterializedTooltip)>,
) -> bool {
    tooltips.iter().any(|(phase, materialized)| {
        matches!(phase, declaration::TooltipPhase::WaitingToShow(_))
            && materialized.space() == PanelSpace::World
    })
}

fn register_tooltip_observers(app: &mut App) {
    app.add_observer(remember_tooltip_camera_from_over)
        .add_observer(remember_tooltip_camera_from_move)
        .add_observer(remember_tooltip_camera_from_press)
        .add_observer(remove_tooltip_pointer_camera)
        .add_observer(finalize_removed_tooltip)
        .add_observer(finalize_despawned_tooltip)
        .add_observer(finalize_tooltip_panel_role);
}

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TooltipSurfaceMaterial>();
        let hidden_event = app.world_mut().register_event_key::<TooltipHidden>();
        app.insert_resource(TooltipHiddenEventKey(hidden_event));
        register_tooltip_observers(app);
        app.configure_sets(
            Update,
            (
                TooltipSystems::Eligibility
                    .after(WidgetSystems::FocusCommandsApplied)
                    .after(TooltipSystems::ControllerCommandsApplied),
                TooltipSystems::EligibilityCommandsApplied.after(TooltipSystems::Eligibility),
                TooltipSystems::Materialize.after(TooltipSystems::EligibilityCommandsApplied),
                TooltipSystems::MaterializationCommandsApplied.after(TooltipSystems::Materialize),
                TooltipSystems::Attach.after(TooltipSystems::MaterializationCommandsApplied),
                TooltipSystems::AttachmentCommandsApplied
                    .after(TooltipSystems::Attach)
                    .before(ScreenSpaceSystems::WidgetDemandCommandsApplied)
                    .before(PanelSystems::ResolvePanelAttachments),
                TooltipPlacementSystems::ScreenDecide
                    .after(PanelSystems::ResolvePanelAttachments)
                    .after(WidgetSystems::ReifyEdited),
                TooltipPlacementSystems::ScreenCommandsApplied
                    .after(TooltipPlacementSystems::ScreenDecide),
                TooltipPlacementSystems::ScreenCorrection
                    .after(TooltipPlacementSystems::ScreenCommandsApplied)
                    .before(PanelSystems::PositionScreenSpace),
            ),
        )
        .add_systems(
            Update,
            (
                (advance_tooltip_visibility, retire_superseded_help_panels)
                    .chain()
                    .in_set(TooltipSystems::Eligibility),
                ApplyDeferred.in_set(TooltipSystems::EligibilityCommandsApplied),
                apply_tooltip_width_constraints.before(PanelSystems::ComputeLayout),
                (
                    invalidate_stale_world_tooltip_frames,
                    materialize_requested_tooltips,
                )
                    .chain()
                    .in_set(TooltipSystems::Materialize),
                ApplyDeferred.in_set(TooltipSystems::MaterializationCommandsApplied),
                (
                    invalidate_stale_materialized_tooltips,
                    attach_materialized_tooltips,
                )
                    .chain()
                    .run_if(tooltip_readiness_inputs_changed)
                    .in_set(TooltipSystems::Attach),
                ApplyDeferred.in_set(TooltipSystems::AttachmentCommandsApplied),
                resolve_screen_tooltip_placements
                    .in_set(TooltipPlacementSystems::ScreenDecide)
                    .run_if(tooltip_readiness_or_reveal),
                ApplyDeferred.in_set(TooltipPlacementSystems::ScreenCommandsApplied),
                (
                    screen_space::resolve_screen_space_panel_attachments,
                    clear_screen_tooltip_attachment_corrections,
                )
                    .chain()
                    .in_set(TooltipPlacementSystems::ScreenCorrection)
                    .run_if(screen_tooltip_attachment_correction_requested),
            ),
        );
        register_tooltip_post_update_systems(app);
    }
}

fn register_tooltip_post_update_systems(app: &mut App) {
    app.configure_sets(
        PostUpdate,
        (
            TooltipSystems::Reveal
                .after(TooltipSystems::Readiness)
                .before(PanelChildSystems::Build)
                .before(MaterialTableAppendReady),
            TooltipPlacementSystems::WorldDecide
                .after(panel::refresh_world_anchor_globals)
                .after(placement::refresh_occluder_globals),
            TooltipPlacementSystems::WorldCommandsApplied
                .after(TooltipPlacementSystems::WorldDecide)
                .before(panel::write_panel_anchor_offsets)
                .before(AnchorSystems::Resolve),
            TooltipSystems::Readiness
                .after(AnchorSystems::Resolve)
                .after(TooltipPlacementSystems::OccludersResolved)
                .before(MaterialTableAppendReady),
            TooltipPlacementSystems::OccludersResolved.after(AnchorSystems::Resolve),
            TooltipSystems::VisibilityEvents
                .after(TooltipSystems::Readiness)
                .after(TransformSystems::Propagate),
        ),
    )
    .add_systems(
        PostUpdate,
        (
            reveal_ready_tooltips.in_set(TooltipSystems::Reveal),
            resolve_world_tooltip_placements
                .in_set(TooltipPlacementSystems::WorldDecide)
                .run_if(tooltip_readiness_or_reveal),
            placement::refresh_occluder_globals
                .after(panel::refresh_world_anchor_globals)
                .before(TooltipPlacementSystems::WorldDecide)
                .run_if(world_tooltip_placement_requested),
            ApplyDeferred.in_set(TooltipPlacementSystems::WorldCommandsApplied),
            placement::refresh_occluder_globals_after_resolve
                .in_set(TooltipPlacementSystems::OccludersResolved)
                .run_if(world_tooltip_placement_requested),
            finalize_tooltip_readiness
                .in_set(TooltipSystems::Readiness)
                .run_if(tooltip_readiness_or_reveal),
            emit_tooltip_shown.in_set(TooltipSystems::VisibilityEvents),
        ),
    );
}
