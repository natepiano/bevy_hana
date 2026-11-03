//! Screen-space panel attachment resolution.

mod candidate;
mod placement;
mod projection;
mod rect;
mod resolve;
mod window;

use bevy::ecs::system::SystemParam;
use bevy::prelude::Changed;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Or;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
pub(crate) use candidate::ScreenAttachmentResolveSkip;
use hana_valence::AnchorPose;
use hana_valence::ResolvedAnchorGeometry;
pub use rect::ScreenAnchorTarget;
pub(crate) use rect::ScreenPanelRect;
pub(crate) use rect::screen_panel_rect;
pub(crate) use resolve::ScreenAttachmentResolveDiagnostics;
use resolve::ScreenAttachmentResolverScratch;
pub(crate) use resolve::attachment_is_ready;

use super::CandidateQueries;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::ResolvedScreenPanelPosition;
use crate::widgets::PanelWidget;
use crate::widgets::ScreenWidgetAnchorProxy;
use crate::widgets::ScreenWidgetAnchoredHere;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;

type ChangedScreenAttachmentInput = Or<(
    Changed<Window>,
    Changed<PanelAttachmentAuthored>,
    Changed<PanelAnchorOffset>,
    Changed<AnchorPose>,
    Changed<DiegeticPanel>,
    Changed<Transform>,
    Changed<ScreenAnchorTarget>,
    Changed<WidgetOf>,
    Changed<WidgetAnchorRect>,
    Changed<ScreenWidgetAnchoredHere>,
    Changed<ScreenWidgetAnchorProxy>,
    Changed<ResolvedAnchorGeometry>,
)>;

#[derive(SystemParam)]
pub(super) struct ScreenAttachmentWakeInputs<'w, 's> {
    changed:          Query<'w, 's, (), ChangedScreenAttachmentInput>,
    removed_windows:  RemovedComponents<'w, 's, Window>,
    removed_attached: RemovedComponents<'w, 's, PanelAttachmentAuthored>,
    removed_poses:    RemovedComponents<'w, 's, AnchorPose>,
    removed_panels:   RemovedComponents<'w, 's, DiegeticPanel>,
    removed_widgets:  RemovedComponents<'w, 's, PanelWidget>,
    removed_targets:  RemovedComponents<'w, 's, ScreenAnchorTarget>,
    removed_geometry: RemovedComponents<'w, 's, ResolvedAnchorGeometry>,
}

pub(super) fn screen_attachment_inputs_changed(mut wake: ScreenAttachmentWakeInputs) -> bool {
    let mut changed = !wake.changed.is_empty();
    changed |= wake.removed_windows.read().next().is_some();
    changed |= wake.removed_attached.read().next().is_some();
    changed |= wake.removed_poses.read().next().is_some();
    changed |= wake.removed_panels.read().next().is_some();
    changed |= wake.removed_widgets.read().next().is_some();
    changed |= wake.removed_targets.read().next().is_some();
    changed |= wake.removed_geometry.read().next().is_some();
    changed
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct ScreenAttachmentResolveRuns(usize);

#[cfg(test)]
pub(super) fn reset_screen_attachment_resolve_runs(mut runs: ResMut<ScreenAttachmentResolveRuns>) {
    runs.0 = 0;
}

pub(crate) fn resolve_screen_space_panel_attachments(
    windows: Query<(Entity, &Window)>,
    attachments: Query<(Entity, &PanelAttachmentAuthored, &PanelAnchorOffset)>,
    anchor_poses: Query<(Entity, &AnchorPose)>,
    candidate_queries: CandidateQueries,
    resolved_positions: Query<&mut ResolvedScreenPanelPosition>,
    diagnostics: ResMut<ScreenAttachmentResolveDiagnostics>,
    scratch: Local<ScreenAttachmentResolverScratch>,
    #[cfg(test)] mut runs: Option<ResMut<ScreenAttachmentResolveRuns>>,
) {
    #[cfg(test)]
    if let Some(runs) = runs.as_deref_mut() {
        runs.0 += 1;
    }
    resolve::resolve_screen_space_panel_attachments(
        windows,
        attachments,
        anchor_poses,
        candidate_queries,
        resolved_positions,
        diagnostics,
        scratch,
    );
}

pub(super) fn rotate_screen_offset(offset: Vec2, angle: f32) -> Vec2 {
    projection::rotate_screen_offset(offset, angle)
}

pub(crate) fn screen_in_plane_angle(rotation: Quat) -> f32 {
    projection::screen_in_plane_angle(rotation)
}
