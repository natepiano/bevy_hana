//! Placement of edge panels without overlap.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::camera::RenderTarget;
use bevy::ecs::change_detection::Tick;
use bevy::prelude::Add;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::InheritedVisibility;
use bevy::prelude::Local;
use bevy::prelude::Mut;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::ParamSet;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::CoordinateSpace;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::PanelAnchorGeometryParam;
use hana_diegetic::PanelAnchorPoints;
use hana_diegetic::PanelAttachmentAuthored;
use hana_diegetic::PanelPlane;
use hana_diegetic::PanelScreenBounds;
use hana_diegetic::ScreenPosition;
use hana_diegetic::Sizing;
use hana_diegetic::WidgetOf;
use hana_lagrange::FreeCam;

use crate::camera_home::CameraHomeTarget;
use crate::constants::LABEL_SIZE;
use crate::orbit_cam::FairyDustOrbitCam;

mod constants;
use constants::FULL_SCREEN_CLEARANCE_MINIMUM;

/// Minimum clear space between visible screen panels, in logical pixels.
pub const SCREEN_PANEL_GAP: f32 = 12.0;
/// Four units between an edge panel and its nearest window edge.
pub(crate) const SCREEN_PANEL_WINDOW_INSET: f32 = LABEL_SIZE.0 * 4.0 / 3.0;
const PLACEMENT_EPSILON: f32 = 0.001;
const AUXILIARY_PANEL_MIN_WINDOW: Vec2 = Vec2::new(1000.0, 900.0);

pub(crate) fn inset_anchor_top_left(window: Vec2, size: Vec2, anchor: Anchor) -> Vec2 {
    let (fx, fy) = anchor.offset_fraction();
    let inset = if anchor == Anchor::TopLeft {
        Vec2::new(
            SCREEN_PANEL_WINDOW_INSET.min((window.x - size.x).max(0.0)),
            SCREEN_PANEL_WINDOW_INSET.min((window.y - size.y).max(0.0)),
        )
    } else {
        Vec2::splat(SCREEN_PANEL_WINDOW_INSET)
    };
    inset + (window - size - Vec2::splat(SCREEN_PANEL_WINDOW_INSET * 2.0)) * Vec2::new(fx, fy)
}

/// One panel's desired corner or edge and its solved logical pixel size.
#[derive(Clone, Copy)]
struct EdgePanel {
    entity:       Entity,
    anchor:       Anchor,
    size:         Vec2,
    precedence:   EdgePanelPrecedence,
    width_policy: EdgePanelWidth,
}

#[derive(Component, Clone, Copy, Debug)]
pub(super) enum EdgePanelWidth {
    Fixed,
    Resizable { minimum: f32 },
}

/// The result of placing one panel against higher precedence panels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PanelPlacement {
    AtAnchor,
    Moved { top_left: Vec2 },
    Hidden,
}

/// The authored fraction survives while separation applies a pixel width.
#[derive(Component, Clone, Copy, Debug)]
pub(super) struct EdgePanelAuthoredWidth {
    fraction:      f32,
    height:        Sizing,
    last_resolved: f32,
}

#[derive(Clone, Copy, Debug)]
struct ResolvedEdgePanel {
    entity:    Entity,
    placement: PanelPlacement,
    width:     f32,
}

/// Last placement chosen by Fairy Dust, including panels it hid.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgePanelPlacementRecord {
    pub placement: PanelPlacement,
}

/// The edge placement Home fits around, computed without camera-dependent
/// world-panel obstacles.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgePanelHomePlacement(pub PanelPlacement, pub f32);

/// Whether the latest edge-panel placement still needs position verification.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EdgePanelPlacementState {
    /// Placement is awaiting position verification.
    #[default]
    Pending,
    /// Every managed panel holds its solved position.
    Placed,
}

/// The current placement state and the revision it belongs to.
#[derive(Resource, Default)]
pub struct EdgePanelPlacementStatus {
    /// Whether every managed edge panel holds its current solved position.
    pub state:          EdgePanelPlacementState,
    /// Increments whenever a panel's solved placement changes.
    pub revision:       u64,
    expected_panels:    usize,
    current_placements: HashMap<Entity, PanelPlacement>,
    resolved_widths:    HashMap<Entity, f32>,
}

impl EdgePanelPlacementStatus {
    fn record_solved_placements(
        &mut self,
        expected_panels: usize,
        placements: HashMap<Entity, PanelPlacement>,
        resolved_widths: HashMap<Entity, f32>,
        changed: bool,
    ) {
        if changed
            || self.expected_panels != expected_panels
            || self.current_placements != placements
            || self.resolved_widths != resolved_widths
        {
            self.state = EdgePanelPlacementState::Pending;
        }
        self.expected_panels = expected_panels;
        self.current_placements = placements;
        self.resolved_widths = resolved_widths;
        if changed {
            self.revision += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlacementPendingReason {
    NoExpectedPanels,
    MissingPlacements { expected: usize, placed: usize },
    PanelMissing(Entity),
    HiddenPanelVisible(Entity),
    UnresolvedGeometry(Entity),
    WrongCoordinateSpace(Entity),
    WindowMissing(Entity),
    WrongPosition(Entity),
    WrongWidth(Entity),
}

#[derive(Default)]
pub(super) struct LastPlacementDiagnostic {
    state: PlacementDiagnosticState,
}

#[derive(Default)]
enum PlacementDiagnosticState {
    #[default]
    None,
    Reported {
        revision: u64,
        reason:   PlacementPendingReason,
    },
}

/// An attachment hidden until its target has a visible, resolved position.
#[derive(Component)]
pub(super) struct SeparationHiddenAttachment {
    visibility_written_tick: Tick,
}

/// Whether an edge panel has a placement for Home, a separation record, or neither.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EdgePanelManagement {
    Unmanaged,
    Recorded(PanelPlacement),
    ForHome(PanelPlacement),
}

impl EdgePanelManagement {
    const fn from_record(record: Option<&EdgePanelPlacementRecord>) -> Self {
        match record {
            Some(record) => Self::Recorded(record.placement),
            None => Self::Unmanaged,
        }
    }

    pub(crate) const fn for_home(
        home_placement: Option<&EdgePanelHomePlacement>,
        record: Option<&EdgePanelPlacementRecord>,
    ) -> Self {
        match home_placement {
            Some(home_placement) => Self::ForHome(home_placement.0),
            None => Self::from_record(record),
        }
    }
}

/// Fairy Dust's precedence below panels supplied by the host application.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgePanelPrecedence {
    TitleBar,
    HostPanel,
    CameraControl,
    Description,
}

impl EdgePanelPrecedence {
    const fn rank(self) -> u8 {
        match self {
            Self::TitleBar => 0,
            Self::HostPanel => 1,
            Self::CameraControl => 2,
            Self::Description => 3,
        }
    }
}

#[derive(Clone, Copy)]
struct PanelFootprint {
    top_left: Vec2,
    size:     Vec2,
}

impl PanelFootprint {
    fn overlaps(self, other: Self) -> bool {
        self.top_left.x < other.top_left.x + other.size.x
            && other.top_left.x < self.top_left.x + self.size.x
            && self.top_left.y < other.top_left.y + other.size.y
            && other.top_left.y < self.top_left.y + self.size.y
    }

    fn fits(self, window: Vec2) -> bool {
        self.top_left.x >= SCREEN_PANEL_WINDOW_INSET - PLACEMENT_EPSILON
            && self.top_left.y >= SCREEN_PANEL_WINDOW_INSET - PLACEMENT_EPSILON
            && self.top_left.x + self.size.x
                <= window.x - SCREEN_PANEL_WINDOW_INSET + PLACEMENT_EPSILON
            && self.top_left.y + self.size.y
                <= window.y - SCREEN_PANEL_WINDOW_INSET + PLACEMENT_EPSILON
    }

    fn clears(self, other: Self) -> bool {
        self.top_left.x >= other.top_left.x + other.size.x + SCREEN_PANEL_GAP
            || other.top_left.x >= self.top_left.x + self.size.x + SCREEN_PANEL_GAP
            || self.top_left.y >= other.top_left.y + other.size.y + SCREEN_PANEL_GAP
            || other.top_left.y >= self.top_left.y + self.size.y + SCREEN_PANEL_GAP
    }
}

#[derive(Clone, Copy)]
enum AttachmentPlacement {
    Independent,
    Carried { owner: Entity, offset: Vec2 },
}

#[derive(Clone, Copy)]
struct AttachedPanelFootprint {
    bounds:    PanelFootprint,
    placement: AttachmentPlacement,
}

impl AttachedPanelFootprint {
    fn at(self, owner: Entity, owner_rect: PanelFootprint) -> Option<PanelFootprint> {
        match self.placement {
            AttachmentPlacement::Carried {
                owner: target,
                offset,
            } if target == owner => Some(PanelFootprint {
                top_left: owner_rect.top_left + offset,
                size:     self.bounds.size,
            }),
            AttachmentPlacement::Independent | AttachmentPlacement::Carried { .. } => None,
        }
    }
}

impl From<PanelScreenBounds> for PanelFootprint {
    fn from(bounds: PanelScreenBounds) -> Self {
        Self {
            top_left: bounds.top_left(),
            size:     bounds.size(),
        }
    }
}

/// Places panels in precedence order, moving a blocked panel only along its edge.
fn auxiliary_panel_has_room(window: Vec2, precedence: EdgePanelPrecedence) -> bool {
    !matches!(
        precedence,
        EdgePanelPrecedence::CameraControl | EdgePanelPrecedence::Description
    ) || (window.x >= AUXILIARY_PANEL_MIN_WINDOW.x && window.y >= AUXILIARY_PANEL_MIN_WINDOW.y)
}

fn edge_panel_fits_window(window: Vec2, panel: &EdgePanel) -> bool {
    panel.precedence == EdgePanelPrecedence::TitleBar
        || (window.is_finite()
            && panel.size.is_finite()
            && window.x > 0.0
            && window.y > 0.0
            && panel.size.x > 0.0
            && panel.size.y > 0.0
            && panel.size.x <= window.x
            && panel.size.y <= window.y)
}

#[cfg(test)]
fn separate_edge_panels(
    window: Vec2,
    panels: &[EdgePanel],
    attachments: &[AttachedPanelFootprint],
    world_panels: &[PanelFootprint],
) -> Vec<(Entity, PanelPlacement)> {
    separate_edge_panels_with_widths(window, panels, attachments, world_panels)
        .into_iter()
        .map(|resolved| (resolved.entity, resolved.placement))
        .collect()
}

fn separate_edge_panels_with_widths(
    window: Vec2,
    panels: &[EdgePanel],
    attachments: &[AttachedPanelFootprint],
    world_panels: &[PanelFootprint],
) -> Vec<ResolvedEdgePanel> {
    let mut placed: Vec<PanelFootprint> = Vec::new();
    let mut title_footprints = Vec::new();
    let mut results: Vec<_> = panels
        .iter()
        .map(|panel| ResolvedEdgePanel {
            entity:    panel.entity,
            placement: PanelPlacement::Hidden,
            width:     panel.size.x,
        })
        .collect();
    let mut order: Vec<_> = (0..panels.len()).collect();
    order.sort_by_key(|&index| panels[index].precedence.rank());
    let independent: Vec<_> = attachments
        .iter()
        .filter_map(|attachment| match attachment.placement {
            AttachmentPlacement::Independent => Some(attachment.bounds),
            AttachmentPlacement::Carried { .. } => None,
        })
        .collect();

    for index in order {
        let original = &panels[index];
        let placement = try_place_edge_panel(
            window,
            original,
            attachments,
            &independent,
            world_panels,
            &mut placed,
            &mut title_footprints,
        );
        if placement != PanelPlacement::Hidden {
            results[index].placement = placement;
            continue;
        }
        for width in candidate_widths(window, original, &placed, &independent)
            .into_iter()
            .rev()
        {
            let panel = EdgePanel {
                size: Vec2::new(width, original.size.y),
                ..*original
            };
            let placement = try_place_edge_panel(
                window,
                &panel,
                attachments,
                &independent,
                world_panels,
                &mut placed,
                &mut title_footprints,
            );
            if placement != PanelPlacement::Hidden {
                results[index].placement = placement;
                results[index].width = width;
                break;
            }
        }
    }

    results
}

fn candidate_widths(
    window: Vec2,
    panel: &EdgePanel,
    placed: &[PanelFootprint],
    independent: &[PanelFootprint],
) -> Vec<f32> {
    let mut widths = Vec::new();
    if let EdgePanelWidth::Resizable { minimum } = panel.width_policy {
        let anchored_x = inset_anchor_top_left(window, panel.size, panel.anchor).x;
        for other in placed.iter().chain(independent) {
            let clear_width = other.top_left.x - SCREEN_PANEL_GAP - anchored_x - PLACEMENT_EPSILON;
            if clear_width >= minimum && clear_width < panel.size.x {
                widths.push(clear_width);
            }
        }
        widths.sort_by(f32::total_cmp);
        widths.dedup();
    }
    widths
}

fn try_place_edge_panel(
    window: Vec2,
    panel: &EdgePanel,
    attachments: &[AttachedPanelFootprint],
    independent: &[PanelFootprint],
    world_panels: &[PanelFootprint],
    placed: &mut Vec<PanelFootprint>,
    title_footprints: &mut Vec<PanelFootprint>,
) -> PanelPlacement {
    let auxiliary = matches!(
        panel.precedence,
        EdgePanelPrecedence::CameraControl | EdgePanelPrecedence::Description
    );
    if !auxiliary_panel_has_room(window, panel.precedence) || !edge_panel_fits_window(window, panel)
    {
        return PanelPlacement::Hidden;
    }
    let anchored = inset_anchor_top_left(window, panel.size, panel.anchor);
    let anchored_rect = PanelFootprint {
        top_left: anchored,
        size:     panel.size,
    };
    let group_at = |rect: PanelFootprint| {
        std::iter::once(rect)
            .chain(
                attachments
                    .iter()
                    .filter_map(|attachment| attachment.at(panel.entity, rect)),
            )
            .collect::<Vec<_>>()
    };
    let fits_and_clears = |group: &[PanelFootprint]| {
        group.iter().all(|&rect| {
            rect.fits(window)
                && placed.iter().all(|&other| rect.clears(other))
                && independent.iter().all(|&other| rect.clears(other))
                && (!auxiliary || auxiliary_corner_clears_popups(panel, rect, attachments))
                && (auxiliary || world_panels.iter().all(|&other| rect.clears(other)))
        })
    };
    let anchored_group = group_at(anchored_rect);
    if panel.precedence == EdgePanelPrecedence::TitleBar {
        title_footprints.extend(anchored_group.iter().copied());
        placed.extend(anchored_group);
        return PanelPlacement::AtAnchor;
    }
    if !auxiliary
        && anchored_group
            .iter()
            .any(|&rect| title_footprints.iter().any(|&title| !rect.clears(title)))
    {
        return PanelPlacement::Hidden;
    }
    if fits_and_clears(&anchored_group) {
        placed.extend(anchored_group);
        return PanelPlacement::AtAnchor;
    }
    let moved = candidate_rows(
        panel,
        window,
        anchored.y,
        attachments,
        placed,
        independent,
        world_panels,
    )
    .into_iter()
    .find_map(|y| {
        let rect = PanelFootprint {
            top_left: Vec2::new(anchored.x, y),
            size:     panel.size,
        };
        fits_and_clears(&group_at(rect)).then_some(rect)
    });
    if let Some(rect) = moved {
        placed.extend(group_at(rect));
        return PanelPlacement::Moved {
            top_left: rect.top_left,
        };
    }
    if auxiliary
        && (window.x < FULL_SCREEN_CLEARANCE_MINIMUM.x
            || window.y < FULL_SCREEN_CLEARANCE_MINIMUM.y)
        && auxiliary_corner_clears_popups(panel, anchored_rect, attachments)
    {
        placed.extend(anchored_group);
        return PanelPlacement::AtAnchor;
    }
    PanelPlacement::Hidden
}

fn auxiliary_corner_clears_popups(
    panel: &EdgePanel,
    corner: PanelFootprint,
    attachments: &[AttachedPanelFootprint],
) -> bool {
    attachments.iter().all(|attachment| {
        matches!(attachment.placement, AttachmentPlacement::Carried { owner, .. } if owner == panel.entity)
            || !corner.overlaps(attachment.bounds)
    })
}

fn candidate_rows(
    panel: &EdgePanel,
    window: Vec2,
    anchored_y: f32,
    attachments: &[AttachedPanelFootprint],
    placed: &[PanelFootprint],
    independent: &[PanelFootprint],
    world_panels: &[PanelFootprint],
) -> Vec<f32> {
    let mut rows = vec![
        SCREEN_PANEL_WINDOW_INSET,
        window.y - panel.size.y - SCREEN_PANEL_WINDOW_INSET,
    ];
    let members =
        std::iter::once((0.0, panel.size.y)).chain(attachments.iter().filter_map(|attachment| {
            match attachment.placement {
                AttachmentPlacement::Carried { owner, offset } if owner == panel.entity => {
                    Some((offset.y, attachment.bounds.size.y))
                },
                AttachmentPlacement::Independent | AttachmentPlacement::Carried { .. } => None,
            }
        }));
    for (offset, member_height) in members {
        for other in placed.iter().chain(independent).chain(world_panels) {
            rows.push(other.top_left.y + other.size.y + SCREEN_PANEL_GAP - offset);
            rows.push(other.top_left.y - SCREEN_PANEL_GAP - member_height - offset);
        }
    }
    rows.retain(|&y| match panel.anchor {
        Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => y >= anchored_y,
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => y <= anchored_y,
        Anchor::CenterLeft | Anchor::Center | Anchor::CenterRight => true,
    });
    rows.sort_by(|left, right| {
        let distance = match panel.anchor {
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => left.total_cmp(right),
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
                right.total_cmp(left)
            },
            Anchor::CenterLeft | Anchor::Center | Anchor::CenterRight => (left - anchored_y)
                .abs()
                .total_cmp(&(right - anchored_y).abs()),
        };
        distance.then_with(|| left.total_cmp(right))
    });
    rows.dedup();
    rows
}

struct RankedEdgePanel {
    edge:        EdgePanel,
    precedence:  u8,
    spawn_order: u64,
}

#[derive(Clone, Copy)]
enum AttachedPanelOwner {
    Independent,
    EdgePanel(Entity),
}

struct AttachedPanel {
    entity: Entity,
    target: Entity,
    owner:  AttachedPanelOwner,
}

#[derive(Resource, Default)]
pub(super) struct EdgePanelSpawnOrder(u64);

/// Stable order and initial edge intent survive a temporary attachment.
#[derive(Component)]
pub(super) struct EdgePanelSpawnOrdinal {
    order:         u64,
    authored_edge: bool,
}

pub(super) fn record_edge_panel_spawn(
    trigger: On<Add<DiegeticPanel>>,
    panels: Query<&DiegeticPanel>,
    mut order: ResMut<EdgePanelSpawnOrder>,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    let Ok(panel) = panels.get(entity) else {
        return;
    };
    let authored_edge = matches!(
        panel.coordinate_space(),
        CoordinateSpace::Screen {
            position: ScreenPosition::Screen,
            ..
        }
    ) && panel.anchor() != Anchor::Center;
    commands.entity(entity).insert(EdgePanelSpawnOrdinal {
        order: order.0,
        authored_edge,
    });
    order.0 += 1;
}

pub(super) type EdgePanelQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut DiegeticPanel,
        &'static ComputedDiegeticPanel,
        &'static mut Visibility,
        Option<&'static InheritedVisibility>,
        Option<&'static EdgePanelPlacementRecord>,
        Option<&'static EdgePanelPrecedence>,
        Option<&'static PanelAttachmentAuthored>,
        Option<&'static EdgePanelSpawnOrdinal>,
        Option<&'static SeparationHiddenAttachment>,
        Option<&'static EdgePanelWidth>,
        Option<&'static EdgePanelAuthoredWidth>,
    ),
>;

/// Recomputes edge placements from the final screen panel dimensions each frame.
pub(super) fn place_edge_panels(
    mut commands: Commands,
    mut placement_status: ResMut<EdgePanelPlacementStatus>,
    windows: Query<(Entity, &Window)>,
    primary: Query<Entity, With<PrimaryWindow>>,
    widget_owners: Query<&WidgetOf>,
    home_targets: Query<Entity, With<CameraHomeTarget>>,
    children: Query<&Children>,
    cameras: Query<
        (&Camera, &RenderTarget, &GlobalTransform),
        Or<(With<FairyDustOrbitCam>, With<FreeCam>)>,
    >,
    mut panel_params: ParamSet<(
        PanelAnchorGeometryParam,
        EdgePanelQuery,
        Query<(&DiegeticPanel, &GlobalTransform)>,
    )>,
) {
    let primary = primary.single().ok();
    let window_sizes: HashMap<Entity, Vec2> = windows
        .iter()
        .map(|(entity, window)| (entity, Vec2::new(window.width(), window.height())))
        .collect();
    let (by_window, mut attached_panels) = collect_edge_panels(
        &mut panel_params.p1(),
        &mut commands,
        primary,
        &window_sizes,
    );
    let expected_panels: usize = by_window.values().map(Vec::len).sum();
    let mut changed = false;

    let obstacles_by_window = attached_panel_obstacles(
        &mut attached_panels,
        &by_window,
        &window_sizes,
        &widget_owners,
        &panel_params.p0(),
    );
    // Placement runs before Home recalculates insets. Project against the
    // current camera pose, then let Home respond to the resulting placements.
    let framed_panels_by_window = framed_world_panel_footprints(
        &home_targets,
        &children,
        &panel_params.p2(),
        &cameras,
        primary,
    );

    let mut panels = panel_params.p1();
    let mut placed_by_entity = HashMap::new();
    let mut resolved_widths = HashMap::new();
    let mut attachments_waiting_for_target = HashSet::new();
    for (window, candidates) in by_window {
        let obstacles = obstacles_by_window
            .get(&window)
            .map_or(&[][..], Vec::as_slice);
        let framed_panels = framed_panels_by_window
            .get(&window)
            .map_or(&[][..], Vec::as_slice);
        for (resolved, home) in
            resolve_window_edge_panels(window_sizes[&window], candidates, obstacles, framed_panels)
        {
            placed_by_entity.insert(resolved.entity, resolved.placement);
            apply_resolved_edge_panel(
                (resolved, home),
                window_sizes[&window],
                &mut panels,
                &mut commands,
                &mut resolved_widths,
                &mut attachments_waiting_for_target,
                &mut changed,
            );
        }
    }
    sync_attachment_visibility(
        &mut panels,
        &mut commands,
        attached_panels,
        &placed_by_entity,
        &attachments_waiting_for_target,
    );
    placement_status.record_solved_placements(
        expected_panels,
        placed_by_entity,
        resolved_widths,
        changed,
    );
}

fn apply_resolved_edge_panel(
    (resolved, home): (ResolvedEdgePanel, ResolvedEdgePanel),
    window_size: Vec2,
    panels: &mut EdgePanelQuery<'_, '_>,
    commands: &mut Commands,
    resolved_widths: &mut HashMap<Entity, f32>,
    attachments_waiting_for_target: &mut HashSet<Entity>,
    changed: &mut bool,
) {
    let entity = resolved.entity;
    let placement = resolved.placement;
    let Ok((_, mut panel, _, mut visibility, _, record, _, _, _, _, width_policy, authored_width)) =
        panels.get_mut(entity)
    else {
        return;
    };
    let previous = EdgePanelManagement::from_record(record);
    let owner_changed_visibility = previous
        == EdgePanelManagement::Recorded(PanelPlacement::Hidden)
        && visibility.is_changed();
    let width_intent = panel_authored_width(
        &panel,
        width_policy.copied().unwrap_or(EdgePanelWidth::Fixed),
        authored_width,
    );
    let width_changed = width_intent.is_some_and(|authored| {
        set_resolved_panel_width(&mut panel, resolved.width, authored.height)
    });
    if width_intent.is_some() {
        resolved_widths.insert(entity, resolved.width);
    }
    let position_changed = apply_edge_panel_placement(
        &mut panel,
        &mut visibility,
        if owner_changed_visibility {
            EdgePanelManagement::Unmanaged
        } else {
            previous
        },
        placement,
        resolved.width,
        window_size,
    );
    if position_changed || width_changed {
        attachments_waiting_for_target.insert(entity);
        *changed = true;
    }
    if previous != EdgePanelManagement::Recorded(placement) {
        *changed = true;
        commands
            .entity(entity)
            .insert(EdgePanelPlacementRecord { placement });
    }
    if let Some(mut intent) = width_intent {
        intent.last_resolved = resolved.width;
        commands.entity(entity).insert(intent);
    }
    commands
        .entity(entity)
        .insert(EdgePanelHomePlacement(home.placement, home.width));
}

fn resolve_window_edge_panels(
    window: Vec2,
    mut candidates: Vec<RankedEdgePanel>,
    attachments: &[AttachedPanelFootprint],
    world_panels: &[PanelFootprint],
) -> Vec<(ResolvedEdgePanel, ResolvedEdgePanel)> {
    candidates.sort_by_key(|candidate| (candidate.precedence, candidate.spawn_order));
    let edges: Vec<_> = candidates
        .into_iter()
        .map(|candidate| candidate.edge)
        .collect();
    let home = separate_edge_panels_with_widths(window, &edges, attachments, &[]);
    separate_edge_panels_with_widths(window, &edges, attachments, world_panels)
        .into_iter()
        .zip(home)
        .collect()
}

fn panel_authored_width(
    panel: &DiegeticPanel,
    policy: EdgePanelWidth,
    saved: Option<&EdgePanelAuthoredWidth>,
) -> Option<EdgePanelAuthoredWidth> {
    if matches!(policy, EdgePanelWidth::Fixed) {
        return None;
    }
    if let Some(saved) = saved {
        return match panel.coordinate_space() {
            CoordinateSpace::Screen {
                width: Sizing::Fixed(dimension),
                ..
            } if (dimension.value - saved.last_resolved).abs() <= PLACEMENT_EPSILON => Some(*saved),
            _ => None,
        };
    }
    {
        let CoordinateSpace::Screen {
            width: Sizing::Percent(fraction),
            height,
            ..
        } = panel.coordinate_space()
        else {
            return None;
        };
        Some(EdgePanelAuthoredWidth {
            fraction:      *fraction,
            height:        *height,
            last_resolved: panel.width(),
        })
    }
}

/// Confirms screen positioning has applied each edge panel's latest solved spot.
pub(super) fn verify_edge_panel_positions(
    mut status: ResMut<EdgePanelPlacementStatus>,
    panels: Query<(&DiegeticPanel, &Transform, &Visibility)>,
    geometry: PanelAnchorGeometryParam,
    windows: Query<&Window>,
    mut last_diagnostic: Local<LastPlacementDiagnostic>,
) {
    if status.state == EdgePanelPlacementState::Placed {
        return;
    }
    match placement_pending_reason(&status, &panels, &geometry, &windows) {
        Ok(()) => {
            status.state = EdgePanelPlacementState::Placed;
            *last_diagnostic = LastPlacementDiagnostic::default();
        },
        Err(reason) => {
            status.state = EdgePanelPlacementState::Pending;
            if !matches!(&last_diagnostic.state, PlacementDiagnosticState::Reported { revision, reason: previous } if *revision == status.revision && *previous == reason)
            {
                bevy::log::info!(
                    "edge panel placement pending at revision {}: {reason:?}",
                    status.revision
                );
                last_diagnostic.state = PlacementDiagnosticState::Reported {
                    revision: status.revision,
                    reason,
                };
            }
        },
    }
}

fn placement_pending_reason(
    status: &EdgePanelPlacementStatus,
    panels: &Query<(&DiegeticPanel, &Transform, &Visibility)>,
    geometry: &PanelAnchorGeometryParam<'_, '_>,
    windows: &Query<&Window>,
) -> Result<(), PlacementPendingReason> {
    if status.expected_panels == 0 {
        return Err(PlacementPendingReason::NoExpectedPanels);
    }
    if status.current_placements.len() != status.expected_panels {
        return Err(PlacementPendingReason::MissingPlacements {
            expected: status.expected_panels,
            placed:   status.current_placements.len(),
        });
    }
    for (&entity, &placement) in &status.current_placements {
        let (panel, transform, visibility) = panels
            .get(entity)
            .map_err(|_| PlacementPendingReason::PanelMissing(entity))?;
        if placement == PanelPlacement::Hidden {
            if *visibility != Visibility::Hidden {
                return Err(PlacementPendingReason::HiddenPanelVisible(entity));
            }
            continue;
        }
        if let Some(&width) = status.resolved_widths.get(&entity)
            && (panel.width() - width).abs() > PLACEMENT_EPSILON
        {
            return Err(PlacementPendingReason::WrongWidth(entity));
        }
        let resolved = geometry
            .get(entity)
            .map_err(|_| PlacementPendingReason::UnresolvedGeometry(entity))?;
        let PanelAnchorPoints::Screen { window, bounds } = *resolved.points() else {
            return Err(PlacementPendingReason::WrongCoordinateSpace(entity));
        };
        let window = windows
            .get(window)
            .map_err(|_| PlacementPendingReason::WindowMissing(entity))?;
        let position = bounds.point(panel.anchor());
        let expected = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        );
        let actual = transform.translation.truncate();
        if !actual.is_finite() || actual.distance(expected) > PLACEMENT_EPSILON {
            return Err(PlacementPendingReason::WrongPosition(entity));
        }
    }
    Ok(())
}

fn framed_world_panel_footprints(
    targets: &Query<Entity, With<CameraHomeTarget>>,
    children: &Query<&Children>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    cameras: &Query<
        (&Camera, &RenderTarget, &GlobalTransform),
        Or<(With<FairyDustOrbitCam>, With<FreeCam>)>,
    >,
    primary: Option<Entity>,
) -> HashMap<Entity, Vec<PanelFootprint>> {
    let mut by_window = HashMap::new();
    for (camera, target, camera_transform) in cameras {
        if !camera.is_active {
            continue;
        }
        let window = match target {
            RenderTarget::Window(WindowRef::Primary) => primary,
            RenderTarget::Window(WindowRef::Entity(entity)) => Some(*entity),
            _ => None,
        };
        let Some(window) = window else {
            continue;
        };
        let footprints = by_window.entry(window).or_insert_with(Vec::new);
        for target in targets {
            for entity in std::iter::once(target).chain(children.iter_descendants(target)) {
                let Ok((panel, transform)) = panels.get(entity) else {
                    continue;
                };
                if !matches!(panel.coordinate_space(), CoordinateSpace::World { .. }) {
                    continue;
                }
                let Ok(plane) = PanelPlane::from_panel(panel, transform) else {
                    continue;
                };
                // Back-facing panels on a multi-face target do not occupy the
                // visible scene from this camera.
                if (camera_transform.translation() - plane.origin()).dot(plane.normal()) <= 0.0 {
                    continue;
                }
                if let Some(footprint) = project_world_panel(camera, camera_transform, plane) {
                    footprints.push(footprint);
                }
            }
        }
    }
    by_window
}

fn project_world_panel(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    plane: PanelPlane,
) -> Option<PanelFootprint> {
    let mut minimum = Vec2::splat(f32::INFINITY);
    let mut maximum = Vec2::splat(f32::NEG_INFINITY);
    for corner in [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomLeft,
        Anchor::BottomRight,
    ] {
        let point: Vec3 = plane.point(corner);
        let Ok(projected) = camera.world_to_viewport(camera_transform, point) else {
            return None;
        };
        minimum = minimum.min(projected);
        maximum = maximum.max(projected);
    }
    let size = maximum - minimum;
    (minimum.is_finite() && size.is_finite() && size.x > 0.0 && size.y > 0.0).then_some(
        PanelFootprint {
            top_left: minimum,
            size,
        },
    )
}

fn collect_edge_panels(
    panels: &mut EdgePanelQuery<'_, '_>,
    commands: &mut Commands,
    primary: Option<Entity>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> (HashMap<Entity, Vec<RankedEdgePanel>>, Vec<AttachedPanel>) {
    let mut by_window: HashMap<Entity, Vec<RankedEdgePanel>> = HashMap::new();
    let mut attached_panels = Vec::new();
    for (
        entity,
        panel,
        computed,
        mut visibility,
        inherited,
        record,
        precedence,
        attachment,
        ordinal,
        hidden_attachment,
        width_policy,
        authored_width,
    ) in panels.iter_mut()
    {
        let management = EdgePanelManagement::from_record(record);
        let was_hidden = management == EdgePanelManagement::Recorded(PanelPlacement::Hidden);
        let owner_changed_visibility = was_hidden && visibility.is_changed();
        if let Some(attachment) = attachment {
            commands.entity(entity).remove::<EdgePanelHomePlacement>();
            let attachment_visibility_changed = hidden_attachment
                .is_some_and(|hidden| visibility.last_changed() != hidden.visibility_written_tick);
            if attachment_visibility_changed {
                commands
                    .entity(entity)
                    .remove::<SeparationHiddenAttachment>();
            }
            if release_attached_panel(
                entity,
                &panel,
                &mut visibility,
                inherited,
                management,
                owner_changed_visibility,
                commands,
            ) || (hidden_attachment.is_some() && !attachment_visibility_changed)
            {
                attached_panels.push(AttachedPanel {
                    entity,
                    target: attachment.target(),
                    owner: AttachedPanelOwner::Independent,
                });
            }
            continue;
        }
        let CoordinateSpace::Screen { window, .. } = panel.coordinate_space() else {
            commands.entity(entity).remove::<EdgePanelHomePlacement>();
            continue;
        };
        let Some(ordinal) = ordinal else {
            commands.entity(entity).remove::<EdgePanelHomePlacement>();
            continue;
        };
        if !ordinal.authored_edge {
            commands.entity(entity).remove::<EdgePanelHomePlacement>();
            continue;
        }
        if owner_changed_visibility && *visibility == Visibility::Hidden {
            commands
                .entity(entity)
                .remove::<(EdgePanelPlacementRecord, EdgePanelHomePlacement)>();
            continue;
        }
        if !edge_panel_is_active(*visibility, inherited, was_hidden, owner_changed_visibility) {
            commands.entity(entity).remove::<EdgePanelHomePlacement>();
            continue;
        }
        let Some(window) = target_window(*window, primary, window_sizes) else {
            continue;
        };
        let precedence = precedence
            .copied()
            .unwrap_or(EdgePanelPrecedence::HostPanel);
        let width_policy = width_policy.copied().unwrap_or(EdgePanelWidth::Fixed);
        let width_intent = panel_authored_width(&panel, width_policy, authored_width);
        if authored_width.is_some() && width_intent.is_none() {
            commands.entity(entity).remove::<EdgePanelAuthoredWidth>();
        }
        by_window.entry(window).or_default().push(RankedEdgePanel {
            edge:        EdgePanel {
                entity,
                anchor: panel.anchor(),
                size: edge_panel_size(&panel, computed, width_intent, window_sizes[&window]),
                precedence,
                width_policy: if width_intent.is_some() {
                    width_policy
                } else {
                    EdgePanelWidth::Fixed
                },
            },
            precedence:  precedence.rank(),
            spawn_order: ordinal.order,
        });
    }
    (by_window, attached_panels)
}

fn target_window(
    reference: WindowRef,
    primary: Option<Entity>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Option<Entity> {
    match reference {
        WindowRef::Primary => primary,
        WindowRef::Entity(entity) => Some(entity),
    }
    .filter(|entity| window_sizes.contains_key(entity))
}

fn edge_panel_size(
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    width_intent: Option<EdgePanelAuthoredWidth>,
    window: Vec2,
) -> Vec2 {
    // Screen Fit can clamp the visible surface to the window while the
    // solved content still needs more room.
    Vec2::new(
        width_intent.map_or_else(
            || panel.width().max(computed.content_width()),
            |authored| window.x * authored.fraction,
        ),
        panel.height().max(computed.content_height()),
    )
}

fn sync_attachment_visibility(
    panels: &mut EdgePanelQuery<'_, '_>,
    commands: &mut Commands,
    attachments: Vec<AttachedPanel>,
    placed_by_entity: &HashMap<Entity, PanelPlacement>,
    waiting_for_target: &HashSet<Entity>,
) {
    for attachment in attachments {
        let target_hidden = match attachment.owner {
            AttachedPanelOwner::EdgePanel(owner) => {
                placed_by_entity.get(&owner) == Some(&PanelPlacement::Hidden)
                    || waiting_for_target.contains(&owner)
            },
            AttachedPanelOwner::Independent => false,
        };
        let Ok((_, _, _, mut visibility, _, _, _, _, _, hidden_attachment, _, _)) =
            panels.get_mut(attachment.entity)
        else {
            continue;
        };
        if target_hidden {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
                commands
                    .entity(attachment.entity)
                    .insert(SeparationHiddenAttachment {
                        visibility_written_tick: visibility.last_changed(),
                    });
            }
        } else if hidden_attachment.is_some() {
            show_panel(&mut visibility);
            commands
                .entity(attachment.entity)
                .remove::<SeparationHiddenAttachment>();
        }
    }
}

fn edge_panel_is_active(
    visibility: Visibility,
    inherited: Option<&InheritedVisibility>,
    was_hidden: bool,
    owner_changed_visibility: bool,
) -> bool {
    was_hidden
        || match visibility {
            Visibility::Hidden => false,
            Visibility::Visible => true,
            Visibility::Inherited => {
                owner_changed_visibility || inherited.is_none_or(|inherited| inherited.get())
            },
        }
}

fn release_attached_panel(
    entity: Entity,
    panel: &DiegeticPanel,
    visibility: &mut Mut<'_, Visibility>,
    inherited: Option<&InheritedVisibility>,
    management: EdgePanelManagement,
    owner_changed_visibility: bool,
    commands: &mut Commands,
) -> bool {
    if management == EdgePanelManagement::Recorded(PanelPlacement::Hidden)
        && !owner_changed_visibility
    {
        show_panel(visibility);
    }
    if management != EdgePanelManagement::Unmanaged {
        commands.entity(entity).remove::<EdgePanelPlacementRecord>();
    }
    matches!(panel.coordinate_space(), CoordinateSpace::Screen { .. })
        && match **visibility {
            Visibility::Hidden => false,
            Visibility::Visible => true,
            Visibility::Inherited => inherited.is_none_or(|inherited| inherited.get()),
        }
}

fn attached_panel_obstacles(
    attached_panels: &mut [AttachedPanel],
    by_window: &HashMap<Entity, Vec<RankedEdgePanel>>,
    window_sizes: &HashMap<Entity, Vec2>,
    widget_owners: &Query<&WidgetOf>,
    geometry: &PanelAnchorGeometryParam<'_, '_>,
) -> HashMap<Entity, Vec<AttachedPanelFootprint>> {
    let mut obstacles_by_window = HashMap::new();
    for attachment in attached_panels {
        let Ok(panel_geometry) = geometry.get(attachment.entity) else {
            continue;
        };
        let PanelAnchorPoints::Screen { window, bounds } = *panel_geometry.points() else {
            continue;
        };
        let owner = widget_owners
            .get(attachment.target)
            .map_or(attachment.target, WidgetOf::panel);
        let footprint = PanelFootprint::from(bounds);
        let placement = if by_window
            .get(&window)
            .is_some_and(|panels: &Vec<RankedEdgePanel>| {
                panels.iter().any(|panel| panel.edge.entity == owner)
            }) {
            attachment.owner = AttachedPanelOwner::EdgePanel(owner);
            let Some(&window_size) = window_sizes.get(&window) else {
                continue;
            };
            let Some(owner_bounds) = geometry
                .get(owner)
                .ok()
                .and_then(|geometry| geometry.points().screen_bounds())
            else {
                continue;
            };
            // Initial attachment positions can be placeholders. Wait until
            // both panels have in-window bounds before carrying their offset.
            if !footprint.fits(window_size) || !PanelFootprint::from(owner_bounds).fits(window_size)
            {
                continue;
            }
            AttachmentPlacement::Carried {
                owner,
                offset: bounds.top_left() - owner_bounds.top_left(),
            }
        } else {
            AttachmentPlacement::Independent
        };
        obstacles_by_window
            .entry(window)
            .or_insert_with(Vec::new)
            .push(AttachedPanelFootprint {
                bounds: footprint,
                placement,
            });
    }
    obstacles_by_window
}

fn apply_edge_panel_placement(
    panel: &mut Mut<'_, DiegeticPanel>,
    visibility: &mut Mut<'_, Visibility>,
    previous: EdgePanelManagement,
    placement: PanelPlacement,
    resolved_width: f32,
    window_size: Vec2,
) -> bool {
    match placement {
        PanelPlacement::AtAnchor => {
            if previous == EdgePanelManagement::Recorded(PanelPlacement::Hidden) {
                show_panel(visibility);
            }
            let top_left = inset_anchor_top_left(
                window_size,
                Vec2::new(resolved_width, panel.height()),
                panel.anchor(),
            );
            let (offset_x, offset_y) = panel.anchor().offset(resolved_width, panel.height());
            set_screen_position_if_changed(panel, top_left + Vec2::new(offset_x, offset_y))
        },
        PanelPlacement::Moved { top_left } => {
            if previous == EdgePanelManagement::Recorded(PanelPlacement::Hidden) {
                show_panel(visibility);
            }
            let (offset_x, offset_y) = panel.anchor().offset(resolved_width, panel.height());
            let anchor_position = top_left + Vec2::new(offset_x, offset_y);
            set_screen_position_if_changed(panel, anchor_position)
        },
        PanelPlacement::Hidden => {
            if **visibility != Visibility::Hidden {
                **visibility = Visibility::Hidden;
            }
            false
        },
    }
}

fn show_panel(visibility: &mut Mut<'_, Visibility>) {
    if **visibility == Visibility::Hidden {
        **visibility = Visibility::Inherited;
    }
}

fn set_resolved_panel_width(
    panel: &mut Mut<'_, DiegeticPanel>,
    width: f32,
    height: Sizing,
) -> bool {
    let CoordinateSpace::Screen { width: current, .. } = panel.coordinate_space() else {
        return false;
    };
    let sizing_changed = !matches!(current, Sizing::Fixed(dimension) if (dimension.value - width).abs() <= PLACEMENT_EPSILON);
    let width_changed = (panel.width() - width).abs() > PLACEMENT_EPSILON;
    if sizing_changed {
        let _ = panel.set_screen_size(Sizing::fixed(width), height);
    }
    if width_changed {
        panel.set_width(width);
    }
    sizing_changed || width_changed
}

fn set_screen_position_if_changed(panel: &mut Mut<'_, DiegeticPanel>, position: Vec2) -> bool {
    if !matches!(panel.coordinate_space(), CoordinateSpace::Screen { position: ScreenPosition::At(current), .. } if *current == position)
    {
        let _ = panel.set_screen_position(position);
        return true;
    }
    false
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod placement_status_tests {
    use super::*;

    #[test]
    fn changed_edge_panel_placement_becomes_pending_again() {
        let panel = Entity::from_bits(1);
        let placements = HashMap::from([(panel, PanelPlacement::AtAnchor)]);
        let mut status = EdgePanelPlacementStatus::default();
        status.record_solved_placements(1, placements.clone(), HashMap::new(), true);
        status.state = EdgePanelPlacementState::Placed;
        status.record_solved_placements(1, placements, HashMap::new(), false);
        assert_eq!(status.state, EdgePanelPlacementState::Placed);
        status.record_solved_placements(
            1,
            HashMap::from([(panel, PanelPlacement::Hidden)]),
            HashMap::new(),
            true,
        );
        assert_eq!(status.state, EdgePanelPlacementState::Pending);
    }
}
