//! Screen-space panel attachment resolution.

mod candidate;
mod placement;
mod projection;
mod rect;
mod resolve;
mod window;

use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
pub(crate) use candidate::ScreenAttachmentResolveSkip;
use hana_valence::AnchorPose;
pub use rect::ScreenAnchorTarget;
pub(crate) use rect::ScreenPanelRect;
pub(crate) use rect::screen_panel_rect;
pub(crate) use resolve::ScreenAttachmentResolveDiagnostics;
use resolve::ScreenAttachmentResolverScratch;
pub(crate) use resolve::attachment_is_ready;

use super::CandidateQueries;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::ResolvedScreenPanelPosition;

pub(crate) fn resolve_screen_space_panel_attachments(
    windows: Query<(Entity, &Window)>,
    attachments: Query<(Entity, &PanelAttachmentAuthored, &PanelAnchorOffset)>,
    anchor_poses: Query<(Entity, &AnchorPose)>,
    candidate_queries: CandidateQueries,
    resolved_positions: Query<&mut ResolvedScreenPanelPosition>,
    diagnostics: ResMut<ScreenAttachmentResolveDiagnostics>,
    scratch: Local<ScreenAttachmentResolverScratch>,
) {
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
