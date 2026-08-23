//! Authoring steps both built-in sheets perform identically.
//!
//! [`QuadSheet`](crate::QuadSheet) and [`TriangleSheet`](crate::TriangleSheet)
//! differ only in their cell geometry and connection forest. Placing a cell
//! frame, turning authored lines into fold groups, and giving each group member
//! one layer of winding clearance are the same three steps for both.

use bevy_ecs::entity::Entity;
use bevy_math::Vec3;
use hana_kana::Displacement;
use hana_kana::Orientation;
use hana_kana::Position;

use crate::AnchorFrame;
use crate::ArrangementConnection;
use crate::ArrangementError;
use crate::FoldGroup;
use crate::FoldGroups;
use crate::GeometryError;
use crate::WindingClearance;

/// Creates one identity-oriented cell frame at `position`.
///
/// # Errors
///
/// Returns [`GeometryError::NonFiniteAnchorPosition`] when `position` contains
/// a NaN or infinite coordinate.
pub(super) fn anchor_frame(position: Vec3) -> Result<AnchorFrame, GeometryError> {
    AnchorFrame::try_new(Position::from(position), Orientation::default())
}

/// Builds one fold group per supplied line of cells.
///
/// Each line is one row or one column in that line's own cell order. A cell
/// only enters a group when it is the source of one of `connections`, so a
/// forest root — which has no connection and therefore no hinge — is dropped
/// and a line made entirely of roots contributes no group at all. Line order
/// becomes group order, which runs outward from the sheet's fixed root cell.
///
/// # Errors
///
/// Returns [`ArrangementError::Provider`] wrapping a
/// [`FoldAuthorError`](crate::FoldAuthorError) when a line repeats a cell or
/// when no line contains a connection source. Callers skip the alternative
/// entirely for a sheet with no connections, so the empty case means the caller
/// authored inconsistent lines.
pub(super) fn fold_groups_from_lines(
    lines: impl IntoIterator<Item = Vec<Entity>>,
    connections: &[ArrangementConnection],
) -> Result<FoldGroups, ArrangementError> {
    let groups = lines
        .into_iter()
        .filter_map(|line| {
            let hinged = line
                .into_iter()
                .filter(|member_entity| {
                    connections
                        .iter()
                        .any(|connection| connection.member_entity == *member_entity)
                })
                .collect::<Vec<_>>();
            (!hinged.is_empty()).then_some(hinged)
        })
        .map(FoldGroup::try_from_iter)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ArrangementError::provider)?;

    FoldGroups::try_from_iter(groups).map_err(ArrangementError::provider)
}

/// Produces the wound-layer clearance covering exactly the members of `groups`.
///
/// Both shipped sheets tile planar cells whose local frames share one `+Z`
/// normal, so one member's winding displacement is that normal scaled by the
/// thickness of the member itself: every member of every group receives
/// `layer_thickness` along `+Z`, in its own source-local frame. Where a layer
/// lands in the stack is a property of the wrapping recipe, which reads group
/// order for it, not of this source-local value. Every group member receives a
/// displacement, so [`WindingClearance::try_new`] sees complete coverage and no
/// foreign member.
///
/// # Errors
///
/// Returns [`ArrangementError::Provider`] wrapping a
/// [`FoldAuthorError`](crate::FoldAuthorError) when `layer_thickness` is not
/// finite or when a member appears in more than one group of the same
/// alternative.
pub(super) fn wound_layer_clearance(
    groups: &FoldGroups,
    layer_thickness: f32,
) -> Result<WindingClearance, ArrangementError> {
    let layer = Displacement::new(0.0, 0.0, layer_thickness);
    let clearances = groups.iter().flat_map(|group| {
        group
            .iter()
            .map(move |member_entity| (*member_entity, layer))
    });

    WindingClearance::try_new(groups, clearances).map_err(ArrangementError::provider)
}
