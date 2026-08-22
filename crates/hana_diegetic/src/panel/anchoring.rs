//! Panel-source anchoring authoring types.
//!
//! Anchoring is a per-frame resolved attachment, not `ChildOf` parenting.
//! The pin position depends on the source and target geometry: a `Fit` panel
//! target that remeasures or a widget target whose rectangle changes moves the
//! target anchor point, so a parent-relative transform captured once would go
//! stale. Screen panels also get window-absolute translations written every
//! frame, which a parent transform would double-apply. Reparenting would
//! further couple lifetimes (target despawn despawns dependents), inherit the
//! target's scale chain, and turn an attachment cycle into a hierarchy cycle.
//! The resolvers instead keep diegetic authoring separate from the
//! coordinate-space positioners. World sources may target world panels or
//! reified widgets in the same coordinate space.

use bevy::prelude::Camera;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::In;
use bevy::prelude::Insert;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Remove;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::warn;
use bevy::transform::helper::TransformHelper;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo as ValenceAnchoredTo;
use hana_valence::Displacement;
use hana_valence::ResolvedAnchorGeometry;
use hana_valence::ResolvedAnchorOffset;

use super::CoordinateSpace;
use super::DiegeticPanel;
use super::PanelEntity;
use super::WidgetEntity;
use super::coordinate_space::PanelSpace;
use super::lifecycle;
use super::lifecycle::PanelComponentOwnership;
use crate::layout::Anchor;
use crate::layout::Dimension;
use crate::layout::Unit;
use crate::widgets::MaterializedTooltip;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgetIndex;
use crate::widgets::ScreenWidgetAnchoredTo;
use crate::widgets::WidgetOf;

/// Same-space panel attachment authoring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelAttachment {
    source: Anchor,
    target: Anchor,
    offset: PanelAnchorOffset,
}

impl PanelAttachment {
    /// Creates attachment authoring with zero offset.
    #[must_use]
    pub const fn new(source: Anchor, target: Anchor) -> Self {
        Self {
            source,
            target,
            offset: PanelAnchorOffset::ZERO,
        }
    }

    /// Sets the offset from the resolved target anchor point.
    #[must_use]
    pub const fn with_offset(mut self, offset: PanelAnchorOffset) -> Self {
        self.offset = offset;
        self
    }

    /// Anchor point on the source panel.
    #[must_use]
    pub const fn source_anchor(self) -> Anchor { self.source }

    /// Anchor point on the target panel or widget.
    #[must_use]
    pub const fn target_anchor(self) -> Anchor { self.target }

    /// Offset from the target anchor point.
    #[must_use]
    pub const fn offset(self) -> PanelAnchorOffset { self.offset }
}

pub(super) fn queue_attach_to_panel<Space>(
    commands: &mut Commands<'_, '_>,
    source: PanelEntity<Space>,
    target: PanelEntity<Space>,
    authored: PanelAttachment,
) {
    queue_attachment_operation(
        commands,
        PanelAttachmentOperation::Attach {
            source: PanelEndpoint::from_panel(&source),
            target: PanelAttachmentTarget::from_panel(&target),
            authored,
        },
    );
}

pub(super) fn queue_attach_to_widget<Space>(
    commands: &mut Commands<'_, '_>,
    source: PanelEntity<Space>,
    target: WidgetEntity<Space>,
    authored: PanelAttachment,
) {
    queue_attachment_operation(
        commands,
        PanelAttachmentOperation::Attach {
            source: PanelEndpoint::from_panel(&source),
            target: PanelAttachmentTarget::from_widget(&target),
            authored,
        },
    );
}

pub(super) fn queue_retarget_to_panel<Space>(
    commands: &mut Commands<'_, '_>,
    source: PanelEntity<Space>,
    target: PanelEntity<Space>,
) {
    queue_attachment_operation(
        commands,
        PanelAttachmentOperation::Retarget {
            source: PanelEndpoint::from_panel(&source),
            target: PanelAttachmentTarget::from_panel(&target),
        },
    );
}

pub(super) fn queue_retarget_to_widget<Space>(
    commands: &mut Commands<'_, '_>,
    source: PanelEntity<Space>,
    target: WidgetEntity<Space>,
) {
    queue_attachment_operation(
        commands,
        PanelAttachmentOperation::Retarget {
            source: PanelEndpoint::from_panel(&source),
            target: PanelAttachmentTarget::from_widget(&target),
        },
    );
}

pub(super) fn queue_detach<Space>(commands: &mut Commands<'_, '_>, source: PanelEntity<Space>) {
    queue_attachment_operation(
        commands,
        PanelAttachmentOperation::Detach {
            source: PanelEndpoint::from_panel(&source),
        },
    );
}

fn queue_attachment_operation(
    commands: &mut Commands<'_, '_>,
    operation: PanelAttachmentOperation,
) {
    commands.run_system_cached_with(apply_panel_attachment_operation, operation);
}

#[derive(Clone, Copy)]
struct PanelEndpoint {
    entity: Entity,
    space:  PanelSpace,
}

impl PanelEndpoint {
    const fn from_panel<Space>(panel: &PanelEntity<Space>) -> Self {
        Self {
            entity: panel.entity(),
            space:  panel.expected_space(),
        }
    }
}

#[derive(Clone, Copy)]
enum PanelAttachmentTarget {
    Panel(PanelEndpoint),
    Widget {
        entity: Entity,
        owner:  Entity,
        space:  PanelSpace,
    },
}

impl PanelAttachmentTarget {
    const fn from_panel<Space>(panel: &PanelEntity<Space>) -> Self {
        Self::Panel(PanelEndpoint::from_panel(panel))
    }

    const fn from_widget<Space>(widget: &WidgetEntity<Space>) -> Self {
        Self::Widget {
            entity: widget.entity(),
            owner:  widget.owner(),
            space:  widget.expected_space(),
        }
    }

    const fn entity(self) -> Entity {
        match self {
            Self::Panel(panel) => panel.entity,
            Self::Widget { entity, .. } => entity,
        }
    }

    const fn screen_widget(self) -> Option<Entity> {
        match self {
            Self::Widget {
                entity,
                space: PanelSpace::Screen,
                ..
            } => Some(entity),
            Self::Panel(_) | Self::Widget { .. } => None,
        }
    }
}

#[derive(Clone, Copy)]
enum PanelAttachmentOperation {
    Attach {
        source:   PanelEndpoint,
        target:   PanelAttachmentTarget,
        authored: PanelAttachment,
    },
    Retarget {
        source: PanelEndpoint,
        target: PanelAttachmentTarget,
    },
    Detach {
        source: PanelEndpoint,
    },
}

impl PanelAttachmentOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Attach { .. } => "attach",
            Self::Retarget { .. } => "retarget",
            Self::Detach { .. } => "detach",
        }
    }

    const fn source(self) -> PanelEndpoint {
        match self {
            Self::Attach { source, .. }
            | Self::Retarget { source, .. }
            | Self::Detach { source } => source,
        }
    }

    const fn target(self) -> Option<PanelAttachmentTarget> {
        match self {
            Self::Attach { target, .. } | Self::Retarget { target, .. } => Some(target),
            Self::Detach { .. } => None,
        }
    }
}

fn apply_panel_attachment_operation(
    In(operation): In<PanelAttachmentOperation>,
    panels: Query<&DiegeticPanel>,
    widget_indices: Query<&PanelWidgetIndex, With<DiegeticPanel>>,
    widgets: Query<(&PanelWidget, &WidgetOf)>,
    authored_attachments: Query<&PanelAttachmentAuthored>,
    mut commands: Commands,
) {
    let source = operation.source();
    let result = validate_panel_endpoint(source, &panels, PanelEndpointRole::Source)
        .and_then(|()| {
            operation.target().map_or(Ok(()), |target| {
                validate_attachment_target(target, &panels, &widget_indices, &widgets)
            })
        })
        .and_then(|()| match operation {
            PanelAttachmentOperation::Retarget { .. } | PanelAttachmentOperation::Detach { .. } => {
                authored_attachments
                    .get(source.entity)
                    .map(|_| ())
                    .map_err(|_| "source panel has no live attachment")
            },
            PanelAttachmentOperation::Attach { .. } => Ok(()),
        });
    if let Err(reason) = result {
        warn_rejected_attachment(operation, reason);
        return;
    }

    match operation {
        PanelAttachmentOperation::Attach {
            source,
            target,
            authored,
        } => {
            set_screen_widget_relation(&mut commands, source.entity, target);
            commands.entity(source.entity).insert((
                PanelAttachmentAuthored::new(target.entity(), authored.source, authored.target),
                authored.offset,
            ));
        },
        PanelAttachmentOperation::Retarget { source, target } => {
            let Ok(authored) = authored_attachments.get(source.entity) else {
                return;
            };
            set_screen_widget_relation(&mut commands, source.entity, target);
            commands
                .entity(source.entity)
                .insert(authored.retargeted(target.entity()));
        },
        PanelAttachmentOperation::Detach { source } => {
            commands.entity(source.entity).remove::<(
                ScreenWidgetAnchoredTo,
                PanelAttachmentAuthored,
                PanelAnchorOffset,
            )>();
        },
    }
}

fn validate_panel_endpoint(
    endpoint: PanelEndpoint,
    panels: &Query<&DiegeticPanel>,
    role: PanelEndpointRole,
) -> Result<(), &'static str> {
    let panel = panels.get(endpoint.entity).map_err(|_| role.missing())?;
    if PanelSpace::from(panel.coordinate_space()) != endpoint.space {
        return Err(role.space_changed());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PanelEndpointRole {
    Source,
    Target,
}

impl PanelEndpointRole {
    const fn missing(self) -> &'static str {
        match self {
            Self::Source => "source panel is missing",
            Self::Target => "target panel is missing",
        }
    }

    const fn space_changed(self) -> &'static str {
        match self {
            Self::Source => "source panel coordinate space changed",
            Self::Target => "target panel coordinate space changed",
        }
    }
}

fn validate_attachment_target(
    target: PanelAttachmentTarget,
    panels: &Query<&DiegeticPanel>,
    widget_indices: &Query<&PanelWidgetIndex, With<DiegeticPanel>>,
    widgets: &Query<(&PanelWidget, &WidgetOf)>,
) -> Result<(), &'static str> {
    match target {
        PanelAttachmentTarget::Panel(panel) => {
            validate_panel_endpoint(panel, panels, PanelEndpointRole::Target)
        },
        PanelAttachmentTarget::Widget {
            entity,
            owner,
            space,
        } => {
            let (widget, widget_of) = widgets
                .get(entity)
                .map_err(|_| "target widget is missing")?;
            if widget_of.panel() != owner {
                return Err("target widget owner changed");
            }
            let owner = panels
                .get(owner)
                .map_err(|_| "target widget owner panel is missing")?;
            if PanelSpace::from(owner.coordinate_space()) != space {
                return Err("target widget coordinate space changed");
            }
            let index = widget_indices
                .get(widget_of.panel())
                .map_err(|_| "target widget owner index is missing")?;
            if !index.maps_to(widget.id(), entity) {
                return Err("target widget is no longer indexed by its owner panel");
            }
            Ok(())
        },
    }
}

fn set_screen_widget_relation(
    commands: &mut Commands<'_, '_>,
    source: Entity,
    target: PanelAttachmentTarget,
) {
    if let Some(widget) = target.screen_widget() {
        commands
            .entity(source)
            .insert(ScreenWidgetAnchoredTo::new(widget));
    } else {
        commands.entity(source).remove::<ScreenWidgetAnchoredTo>();
    }
}

fn warn_rejected_attachment(operation: PanelAttachmentOperation, reason: &str) {
    let source = operation.source().entity;
    if let Some(target) = operation.target() {
        warn!(
            "panel attachment operation `{}` rejected for source {source:?} and target {:?}: \
             {reason}",
            operation.name(),
            target.entity(),
        );
    } else {
        warn!(
            "panel attachment operation `{}` rejected for source {source:?}: {reason}",
            operation.name(),
        );
    }
}

/// Shared panel attachment authoring read by screen and world positioners.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(immutable)]
pub(crate) struct PanelAttachmentAuthored {
    target:        Entity,
    source:        Anchor,
    target_anchor: Anchor,
}

impl PanelAttachmentAuthored {
    pub(crate) const fn new(target: Entity, source: Anchor, target_anchor: Anchor) -> Self {
        Self {
            target,
            source,
            target_anchor,
        }
    }

    /// Target panel or reified widget entity.
    #[must_use]
    pub(crate) const fn target(&self) -> Entity { self.target }

    /// Anchor point on the source panel.
    #[must_use]
    pub(crate) const fn source_anchor(&self) -> Anchor { self.source }

    /// Anchor point on the target panel or reified widget.
    #[must_use]
    pub(crate) const fn target_anchor(&self) -> Anchor { self.target_anchor }

    /// Returns a copy that points at `target`.
    #[must_use]
    pub(crate) const fn retargeted(mut self, target: Entity) -> Self {
        self.target = target;
        self
    }

    pub(crate) fn valence_relation(&self) -> ValenceAnchoredTo {
        ValenceAnchoredTo::new(
            self.target,
            AnchorSite::from(self.source),
            AnchorSite::from(self.target_anchor),
        )
    }
}

pub(crate) fn world_attachment_is_ready(
    source: Entity,
    target: Entity,
    relation: &ValenceAnchoredTo,
    diagnostics: &AnchorResolveDiagnostics,
) -> bool {
    relation.target() == target && diagnostics.current().all(|entry| entry.source != source)
}

/// Refreshes the stored world transforms consumed by the anchor resolver.
///
/// Bevy's ordinary transform propagation runs after
/// [`AnchorSystems::Resolve`](hana_valence::AnchorSystems::Resolve).
/// Without this pre-resolve refresh, a target whose local [`Transform`] changed
/// earlier in the frame still presents its previous-frame [`GlobalTransform`]
/// to Valence, making every dependent trail by one frame. Computing the current
/// value from the target's local hierarchy between animation and resolution
/// lets the resolver place dependents from the same-frame target pose. Only
/// roots without [`ValenceAnchoredTo`] are refreshed here; Valence computes
/// attached dependents in graph order during its following resolve pass. The
/// equality gate preserves quiet change ticks when the pose is unchanged.
pub(crate) fn refresh_world_anchor_globals(
    transforms: TransformHelper,
    mut participants: Query<
        (Entity, &mut GlobalTransform),
        (
            Or<(With<ResolvedAnchorGeometry>, With<Camera>)>,
            With<Transform>,
            Without<ValenceAnchoredTo>,
        ),
    >,
) {
    for (entity, mut global_transform) in &mut participants {
        let Ok(current) = transforms.compute_global_transform(entity) else {
            continue;
        };
        global_transform.set_if_neq(current);
    }
}

pub(super) fn world_anchor_transform_inputs_changed(
    changed_transforms: Query<(), Changed<Transform>>,
    changed_parents: Query<(), Changed<ChildOf>>,
    mut removed_transforms: RemovedComponents<Transform>,
    mut removed_parents: RemovedComponents<ChildOf>,
) -> bool {
    changed_transforms.iter().next().is_some()
        || changed_parents.iter().next().is_some()
        || removed_transforms.read().next().is_some()
        || removed_parents.read().next().is_some()
}

/// Offset from a target panel or reified widget anchor point.
///
/// Coordinates are authored in target-local layout space: positive x moves
/// right, positive y moves down, positive z moves the source in front of the
/// target — along the target plane normal for world targets, toward the screen
/// camera for screen panels. Bare `f32` values resolve against the target
/// panel's layout unit; a widget target uses its owning panel's layout unit.
/// [`Px`](crate::Px), [`Mm`](crate::Mm), [`Pt`](crate::Pt), and
/// [`In`](crate::In) carry explicit units.
///
/// Screen depth selects draw order under the shared orthographic camera and
/// never changes apparent size. The camera sits at z = 1000 with a far plane
/// of 2000, so resolved depths outside `(-1000, 1000)` clip rather than
/// clamp. Panel children are coplanar with their backing and order via
/// material sort biases, not z: batched text carries a 64-unit
/// `Transparent3d` sort bias, so on the sorted screen view a back panel's
/// text composites over a front panel's backing until the panels' depths
/// differ by more than 64 logical pixels.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(PartialEq, Debug, Default)]
pub struct PanelAnchorOffset {
    x: Dimension,
    y: Dimension,
    z: Dimension,
}

const ZERO_DIMENSION: Dimension = Dimension {
    value: 0.0,
    unit:  None,
};

impl PanelAnchorOffset {
    /// Zero offset.
    pub const ZERO: Self = Self {
        x: ZERO_DIMENSION,
        y: ZERO_DIMENSION,
        z: ZERO_DIMENSION,
    };

    /// Creates an offset from two authored dimensions, with zero depth.
    #[must_use]
    pub fn new(x: impl Into<Dimension>, y: impl Into<Dimension>) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
            z: ZERO_DIMENSION,
        }
    }

    /// Sets the depth offset dimension.
    #[must_use]
    pub fn with_z(mut self, z: impl Into<Dimension>) -> Self {
        self.z = z.into();
        self
    }

    /// Horizontal offset dimension.
    #[must_use]
    pub const fn x(self) -> Dimension { self.x }

    /// Vertical offset dimension.
    #[must_use]
    pub const fn y(self) -> Dimension { self.y }

    /// Depth offset dimension.
    #[must_use]
    pub const fn z(self) -> Dimension { self.z }

    pub(crate) fn to_layout_units(self, layout_unit: Unit) -> Vec3 {
        let layout_to_points = layout_unit.to_points();
        Vec3::new(
            self.x.to_points(layout_to_points) / layout_to_points,
            self.y.to_points(layout_to_points) / layout_to_points,
            self.z.to_points(layout_to_points) / layout_to_points,
        )
    }
}

/// Authored local transform for a world panel while it is panel-attached.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(super) struct AnchoredWorldPanelPose {
    authored_transform: Transform,
}

/// Inserts the valence world relation for world-space panel attachments.
pub(super) fn on_panel_attachment_inserted(
    inserted: On<Insert, PanelAttachmentAuthored>,
    attachments: Query<(
        &PanelAttachmentAuthored,
        Option<&DiegeticPanel>,
        Option<&Transform>,
        Option<&AnchoredWorldPanelPose>,
    )>,
    mut commands: Commands,
) {
    let entity = inserted.entity;
    let Ok((authored, panel, transform, pose)) = attachments.get(entity) else {
        return;
    };
    let Some(panel) = panel else {
        lifecycle::remove_owned_component::<ValenceAnchoredTo>(&mut commands, entity, entity);
        lifecycle::remove_owned_component::<ResolvedAnchorOffset>(&mut commands, entity, entity);
        return;
    };
    reconcile_panel_anchor(
        entity,
        PanelSpace::from(panel.coordinate_space()),
        authored,
        transform,
        pose,
        &mut commands,
    );
}

/// Sets a panel-attached entity's valence anchor state to match its space.
///
/// World: insert the valence relation, capturing the authored local transform
/// once so [`restore_inactive_world_panel_poses`] can restore it. Screen:
/// remove the world-only relation, resolved offset, and captured pose.
fn reconcile_panel_anchor(
    entity: Entity,
    space: PanelSpace,
    authored: &PanelAttachmentAuthored,
    transform: Option<&Transform>,
    pose: Option<&AnchoredWorldPanelPose>,
    commands: &mut Commands,
) {
    if space != PanelSpace::World {
        lifecycle::remove_owned_component::<ValenceAnchoredTo>(commands, entity, entity);
        lifecycle::remove_owned_component::<ResolvedAnchorOffset>(commands, entity, entity);
        commands.entity(entity).remove::<AnchoredWorldPanelPose>();
        return;
    }
    lifecycle::write_owned_component(commands, entity, entity, authored.valence_relation());
    if let (Some(transform), None) = (transform, pose) {
        commands.entity(entity).insert(AnchoredWorldPanelPose {
            authored_transform: *transform,
        });
    }
}

/// Removes valence world-only state when panel attachment authoring is removed.
pub(super) fn on_panel_attachment_removed(
    removed: On<Remove, PanelAttachmentAuthored>,
    mut commands: Commands,
) {
    commands
        .entity(removed.entity)
        .try_remove::<ScreenWidgetAnchoredTo>();
    lifecycle::remove_owned_component::<ValenceAnchoredTo>(
        &mut commands,
        removed.entity,
        removed.entity,
    );
    lifecycle::remove_owned_component::<ResolvedAnchorOffset>(
        &mut commands,
        removed.entity,
        removed.entity,
    );
}

/// Restores a world panel's authored local transform after panel anchoring stops.
pub(super) fn restore_inactive_world_panel_poses(
    mut commands: Commands,
    mut panels: Query<(
        Entity,
        &AnchoredWorldPanelPose,
        Option<&PanelAttachmentAuthored>,
        Option<&DiegeticPanel>,
        &mut Transform,
    )>,
) {
    for (entity, pose, attachment, panel, mut transform) in &mut panels {
        let is_world_panel = panel
            .is_some_and(|panel| matches!(panel.coordinate_space(), CoordinateSpace::World { .. }));
        if attachment.is_none() && is_world_panel {
            *transform = pose.authored_transform;
        }
        if attachment.is_none() || !is_world_panel {
            commands.entity(entity).remove::<AnchoredWorldPanelPose>();
        }
    }
}

#[derive(Clone, Copy)]
enum AnchorTargetMetrics {
    Panel {
        layout_unit: Unit,
        layout_size: Vec2,
        world_size:  Vec2,
    },
    Widget {
        owner_layout:          WidgetOwnerLayout,
        world_per_layout_unit: Vec2,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct WidgetOwnerLayout {
    layout_unit: Unit,
    panel_space: PanelSpace,
}

impl From<&DiegeticPanel> for WidgetOwnerLayout {
    fn from(panel: &DiegeticPanel) -> Self {
        Self {
            layout_unit: panel.layout_unit(),
            panel_space: PanelSpace::from(panel.coordinate_space()),
        }
    }
}

impl WidgetOwnerLayout {
    pub(crate) const fn layout_unit(self) -> Unit { self.layout_unit }

    pub(crate) const fn panel_space(self) -> PanelSpace { self.panel_space }
}

impl AnchorTargetMetrics {
    const fn layout_unit(self) -> Unit {
        match self {
            Self::Panel { layout_unit, .. } => layout_unit,
            Self::Widget { owner_layout, .. } => owner_layout.layout_unit(),
        }
    }

    fn world_per_layout_unit(self) -> Option<Vec2> {
        match self {
            Self::Panel {
                layout_size,
                world_size,
                ..
            } => Some(Vec2::new(
                world_size.x / layout_size.x,
                world_size.y / layout_size.y,
            )),
            Self::Widget {
                world_per_layout_unit,
                ..
            } => valid_offset_size(world_per_layout_unit).then_some(world_per_layout_unit),
        }
    }
}

/// Resolves diegetic world-panel offsets into valence resolver-frame offsets.
pub(crate) fn write_panel_anchor_offsets(
    mut commands: Commands,
    attachments: Query<(
        Entity,
        &PanelAttachmentAuthored,
        &PanelAnchorOffset,
        &DiegeticPanel,
        Ref<ValenceAnchoredTo>,
        Option<&PanelComponentOwnership<ValenceAnchoredTo>>,
        Option<&MaterializedTooltip>,
    )>,
    panel_targets: Query<(&DiegeticPanel, &GlobalTransform)>,
    widget_targets: Query<&WidgetOf, With<PanelWidget>>,
    general_targets: Query<(), With<ResolvedAnchorGeometry>>,
) {
    for (entity, authored, offset, source_panel, relation, ownership, materialized) in &attachments
    {
        if !ownership.is_some_and(|ownership| ownership.owns(entity, relation.last_changed())) {
            lifecycle::remove_owned_component::<ResolvedAnchorOffset>(
                &mut commands,
                entity,
                entity,
            );
            continue;
        }
        if !matches!(
            source_panel.coordinate_space(),
            CoordinateSpace::World { .. }
        ) {
            lifecycle::remove_owned_component::<ResolvedAnchorOffset>(
                &mut commands,
                entity,
                entity,
            );
            continue;
        }
        let Some(offset) = lowered_world_offset(
            authored.target(),
            *offset,
            materialized,
            &panel_targets,
            &widget_targets,
            &general_targets,
        ) else {
            lifecycle::remove_owned_component::<ResolvedAnchorOffset>(
                &mut commands,
                entity,
                entity,
            );
            continue;
        };
        lifecycle::write_owned_component(
            &mut commands,
            entity,
            entity,
            ResolvedAnchorOffset::from(Displacement::from(offset)),
        );
    }
}

fn lowered_world_offset(
    target: Entity,
    offset: PanelAnchorOffset,
    materialized: Option<&MaterializedTooltip>,
    panel_targets: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widget_targets: &Query<&WidgetOf, With<PanelWidget>>,
    general_targets: &Query<(), With<ResolvedAnchorGeometry>>,
) -> Option<Vec3> {
    let Some(metrics) = anchor_target_metrics(target, panel_targets, widget_targets) else {
        let materialized = materialized.filter(|materialized| {
            materialized.space() == PanelSpace::World && materialized.target() == target
        })?;
        general_targets.get(target).ok()?;
        return Some(Vec3::new(
            offset
                .x()
                .to_meters(materialized.layout_unit().meters_per_unit()),
            -offset
                .y()
                .to_meters(materialized.layout_unit().meters_per_unit()),
            offset
                .z()
                .to_meters(materialized.layout_unit().meters_per_unit()),
        ));
    };
    let offset = offset.to_layout_units(metrics.layout_unit());
    if !offset.is_finite() {
        return None;
    }
    let world_per_layout_unit = metrics.world_per_layout_unit()?;
    Some(Vec3::new(
        offset.x * world_per_layout_unit.x,
        -offset.y * world_per_layout_unit.y,
        offset.z * world_per_layout_unit.x,
    ))
}

fn anchor_target_metrics(
    target: Entity,
    panel_targets: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widget_targets: &Query<&WidgetOf, With<PanelWidget>>,
) -> Option<AnchorTargetMetrics> {
    if let Ok((target_panel, target_global)) = panel_targets.get(target) {
        return panel_target_metrics(target_panel, target_global);
    }
    let widget_of = widget_targets.get(target).ok()?;
    let (owner_panel, owner_global) = panel_targets.get(widget_of.panel()).ok()?;
    let owner_layout = WidgetOwnerLayout::from(owner_panel);
    if owner_layout.panel_space() != PanelSpace::World {
        return None;
    }
    // A reified widget's authored `Transform` is translation-only, so its
    // `WidgetOf` owner supplies the effective scale before child propagation.
    let owner_scale = transform_scale(owner_global)?;
    let layout_to_world = owner_panel.layout_unit().to_points() * owner_panel.points_to_world();
    let world_per_layout_unit = owner_scale * layout_to_world;
    Some(AnchorTargetMetrics::Widget {
        owner_layout,
        world_per_layout_unit,
    })
}

fn panel_target_metrics(
    target_panel: &DiegeticPanel,
    target_global: &GlobalTransform,
) -> Option<AnchorTargetMetrics> {
    if !matches!(
        target_panel.coordinate_space(),
        CoordinateSpace::World { .. }
    ) {
        return None;
    }
    let layout_size = Vec2::new(target_panel.width(), target_panel.height());
    if !valid_offset_size(layout_size) {
        return None;
    }
    let world_size = target_world_size(target_panel, target_global)?;
    Some(AnchorTargetMetrics::Panel {
        layout_unit: target_panel.layout_unit(),
        layout_size,
        world_size,
    })
}

fn target_world_size(panel: &DiegeticPanel, transform: &GlobalTransform) -> Option<Vec2> {
    let scale = transform_scale(transform)?;
    let size = Vec2::new(
        panel.world_width() * scale.x,
        panel.world_height() * scale.y,
    );
    valid_offset_size(size).then_some(size)
}

fn transform_scale(transform: &GlobalTransform) -> Option<Vec2> {
    let affine = transform.affine();
    let scale = Vec2::new(
        affine.transform_vector3(Vec3::X).length(),
        affine.transform_vector3(Vec3::Y).length(),
    );
    valid_offset_size(scale).then_some(scale)
}

fn valid_offset_size(size: Vec2) -> bool { size.is_finite() && size.x > 0.0 && size.y > 0.0 }

/// Resolver-owned screen pose override for a panel's configured anchor.
///
/// `depth` is the resolved `translation.z` when the screen resolver produced
/// one; `authored_depth` captures the pre-resolution z so removing the
/// attachment restores it. `rotation` and `authored_rotation` mirror that
/// capture-and-restore path for the resolved in-plane z angle.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ResolvedScreenPanelPosition {
    pub(crate) anchor_position:   Option<Vec2>,
    pub(crate) depth:             Option<f32>,
    pub(crate) authored_depth:    Option<f32>,
    pub(crate) rotation:          Option<f32>,
    pub(crate) authored_rotation: Option<f32>,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;
    use std::sync::Arc;

    use bevy::ecs::error;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::ecs::world::CommandQueue;
    use bevy::prelude::App;
    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::ChildOf;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Quat;
    use bevy::prelude::Query;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::World;
    use bevy::transform::TransformPlugin;
    use hana_valence::AnchorPose;
    use hana_valence::AnchorResolveDiagnostics;
    use hana_valence::AnchorResolveSkip;
    use hana_valence::AnchorSystems;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo as ValenceAnchoredTo;
    use hana_valence::Displacement;
    use hana_valence::ResolvedAnchorGeometry;
    use hana_valence::ResolvedAnchorOffset;

    use super::AnchoredWorldPanelPose;
    use super::PanelAnchorOffset;
    use super::PanelAttachment;
    use super::PanelAttachmentAuthored;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelEntityReader;
    use crate::PanelSpace;
    use crate::PanelSystems;
    use crate::PanelWidgetReader;
    use crate::Pt;
    use crate::Px;
    use crate::layout::Anchor;
    use crate::layout::Dimension;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::Unit;
    use crate::panel::DiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::PanelWidget;
    use crate::widgets::PanelWidgetIndex;
    use crate::widgets::PanelWidgets;
    use crate::widgets::ScreenWidgetAnchoredHere;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::WidgetOf;
    use crate::widgets::WidgetSystems;
    use crate::widgets::WidgetsPlugin;

    const DIAGNOSTIC_REPEAT_COUNT: u32 = 2;
    const TRANSFORM_SCALE: f32 = 2.0;
    const WIDGET_HEIGHT: f32 = 10.0;
    const WIDGET_OFFSET_MILLIMETERS: f32 = 5.0;
    const WIDGET_OFFSET_PIXELS: f32 = 10.0;
    const WIDGET_OFFSET_POINTS: f32 = 3.0;
    const WIDGET_OWNER_ROTATION: f32 = 0.4;
    const WIDGET_OWNER_TRANSLATION: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const WIDGET_WIDTH: f32 = 20.0;
    const WORLD_WIDGET_PANEL_HEIGHT: f32 = 100.0;
    const WORLD_WIDGET_PANEL_WIDTH: f32 = 200.0;

    fn test_attachment(
        target: Entity,
        source_anchor: Anchor,
        target_anchor: Anchor,
    ) -> (PanelAttachmentAuthored, PanelAnchorOffset) {
        (
            PanelAttachmentAuthored::new(target, source_anchor, target_anchor),
            PanelAnchorOffset::ZERO,
        )
    }

    fn test_attachment_with_offset(
        target: Entity,
        source_anchor: Anchor,
        target_anchor: Anchor,
        offset: PanelAnchorOffset,
    ) -> (PanelAttachmentAuthored, PanelAnchorOffset) {
        (
            PanelAttachmentAuthored::new(target, source_anchor, target_anchor),
            offset,
        )
    }

    #[test]
    fn attachment_cleanup_tolerates_source_despawn_in_the_same_flush() {
        let mut app = App::new();
        app.set_error_handler(error::panic)
            .add_observer(super::on_panel_attachment_removed);
        let target = app.world_mut().spawn_empty().id();
        let source = app
            .world_mut()
            .spawn((
                PanelAttachmentAuthored::new(target, Anchor::Center, Anchor::Center),
                ScreenWidgetAnchoredTo::new(target),
            ))
            .id();

        app.world_mut().commands().entity(source).despawn();
        app.update();

        assert!(app.world().get_entity(source).is_err());
    }

    #[test]
    fn queued_attachment_rejects_stale_handle_without_mutation() {
        let mut app = App::new();
        let source = app.world_mut().spawn(world_panel(Anchor::Center)).id();
        let target = app.world_mut().spawn(world_panel(Anchor::Center)).id();
        let handles = app
            .world_mut()
            .run_system_once(move |reader: PanelEntityReader| {
                (reader.world(source), reader.world(target))
            })
            .expect("panel reader system runs");
        let source_handle = handles.0.expect("source world panel handle is minted");
        let target_handle = handles.1.expect("target world panel handle is minted");
        app.world_mut().entity_mut(target).insert(screen_panel());

        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.attach_to_panel(
                    source_handle,
                    target_handle,
                    PanelAttachment::new(Anchor::Center, Anchor::Center),
                );
            })
            .expect("attachment system runs");

        assert!(app.world().get::<PanelAttachmentAuthored>(source).is_none(),);
    }

    #[test]
    fn queued_attachment_rejects_widget_handle_invalidated_by_earlier_tree_replacement() {
        let mut app = App::new();
        let owner = app
            .world_mut()
            .spawn(screen_panel_with_widget("removed-target"))
            .id();
        let source = app.world_mut().spawn(screen_panel()).id();
        let widget = spawn_indexed_widget(&mut app, owner, "removed-target");
        let source_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(source, PanelSpace::Screen);
        let widget_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(widget, owner, PanelSpace::Screen);
        let replacement = LayoutBuilder::new(100.0, 40.0).build();

        apply_attachment_commands(&mut app, move |commands| {
            commands
                .set_tree(owner, replacement)
                .expect("replacement tree is valid");
            commands.attach_to_widget(
                source_handle,
                widget_handle,
                PanelAttachment::new(Anchor::Center, Anchor::Center),
            );
        });

        assert!(app.world().get_entity(widget).is_ok());
        assert!(app.world().get::<PanelAttachmentAuthored>(source).is_none());
        assert!(app.world().get::<ScreenWidgetAnchoredTo>(source).is_none());
        assert!(screen_reverse_sources(app.world(), widget).is_empty());
    }

    #[test]
    fn queued_retarget_preserves_attachment_when_tree_replacement_invalidates_widget_handle() {
        let mut app = App::new();
        let owner = app
            .world_mut()
            .spawn(screen_panel_with_widget("removed-target"))
            .id();
        let original_target = app.world_mut().spawn(screen_panel()).id();
        let source = app.world_mut().spawn(screen_panel()).id();
        let widget = spawn_indexed_widget(&mut app, owner, "removed-target");
        let source_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(source, PanelSpace::Screen);
        let original_target_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(original_target, PanelSpace::Screen);
        let widget_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(widget, owner, PanelSpace::Screen);

        apply_attachment_commands(&mut app, |commands| {
            commands.attach_to_panel(
                source_handle,
                original_target_handle,
                PanelAttachment::new(Anchor::Center, Anchor::Center),
            );
        });
        let replacement = LayoutBuilder::new(100.0, 40.0).build();
        apply_attachment_commands(&mut app, move |commands| {
            commands
                .set_tree(owner, replacement)
                .expect("replacement tree is valid");
            commands.retarget_to_widget(source_handle, widget_handle);
        });

        assert!(app.world().get_entity(widget).is_ok());
        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(source)
                .map(PanelAttachmentAuthored::target),
            Some(original_target),
        );
        assert!(app.world().get::<ScreenWidgetAnchoredTo>(source).is_none());
        assert!(screen_reverse_sources(app.world(), widget).is_empty());
    }

    #[test]
    fn queued_screen_attachment_mutates_only_when_commands_apply() {
        let mut app = App::new();
        let owner = app.world_mut().spawn(screen_panel()).id();
        let source = app.world_mut().spawn(screen_panel()).id();
        let widget = spawn_indexed_widget(&mut app, owner, "target");
        let source_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(source, PanelSpace::Screen);
        let widget_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(widget, owner, PanelSpace::Screen);
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, app.world());
            commands.attach_to_widget(
                source_handle,
                widget_handle,
                PanelAttachment::new(Anchor::Center, Anchor::Center),
            );
        }

        assert!(app.world().get::<PanelAttachmentAuthored>(source).is_none());
        assert!(app.world().get::<ScreenWidgetAnchoredTo>(source).is_none());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(widget)
                .is_none()
        );

        queue.apply(app.world_mut());

        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(source)
                .map(PanelAttachmentAuthored::target),
            Some(widget),
        );
        assert_eq!(
            app.world()
                .get::<ScreenWidgetAnchoredTo>(source)
                .map(|relation| relation.target()),
            Some(widget),
        );
        assert_eq!(screen_reverse_sources(app.world(), widget), vec![source]);
    }

    #[test]
    fn screen_retarget_and_detach_update_reverse_membership_in_one_application() {
        let mut app = App::new();
        let owner = app.world_mut().spawn(screen_panel()).id();
        let panel_target = app.world_mut().spawn(screen_panel()).id();
        let source = app.world_mut().spawn(screen_panel()).id();
        let first_widget = spawn_indexed_widget(&mut app, owner, "first");
        let second_widget = spawn_indexed_widget(&mut app, owner, "second");
        let source_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(source, PanelSpace::Screen);
        let first_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(first_widget, owner, PanelSpace::Screen);
        let second_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(second_widget, owner, PanelSpace::Screen);
        let panel_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(panel_target, PanelSpace::Screen);

        apply_attachment_commands(&mut app, |attachments| {
            attachments.attach_to_widget(
                source_handle,
                first_handle,
                PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
            );
        });
        assert_eq!(
            screen_reverse_sources(app.world(), first_widget),
            vec![source]
        );

        apply_attachment_commands(&mut app, |attachments| {
            attachments.retarget_to_widget(source_handle, second_handle);
        });
        assert!(screen_reverse_sources(app.world(), first_widget).is_empty());
        assert_eq!(
            screen_reverse_sources(app.world(), second_widget),
            vec![source]
        );

        apply_attachment_commands(&mut app, |attachments| {
            attachments.retarget_to_panel(source_handle, panel_handle);
        });
        assert!(screen_reverse_sources(app.world(), second_widget).is_empty());
        assert!(app.world().get::<ScreenWidgetAnchoredTo>(source).is_none());
        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(source)
                .map(PanelAttachmentAuthored::target),
            Some(panel_target),
        );

        apply_attachment_commands(&mut app, |attachments| {
            attachments.retarget_to_widget(source_handle, first_handle);
        });
        apply_attachment_commands(&mut app, |attachments| {
            attachments.detach(source_handle);
        });
        assert!(screen_reverse_sources(app.world(), first_widget).is_empty());
        assert!(app.world().get::<ScreenWidgetAnchoredTo>(source).is_none());
        assert!(app.world().get::<PanelAttachmentAuthored>(source).is_none());
    }

    #[test]
    fn missing_live_attachment_preserves_existing_screen_relationship() {
        let mut app = App::new();
        let owner = app.world_mut().spawn(screen_panel()).id();
        let source = app.world_mut().spawn(screen_panel()).id();
        let first_widget = spawn_indexed_widget(&mut app, owner, "first");
        let second_widget = spawn_indexed_widget(&mut app, owner, "second");
        app.world_mut()
            .entity_mut(source)
            .insert(ScreenWidgetAnchoredTo::new(first_widget));
        let source_handle: crate::PanelEntity<crate::Screen> =
            crate::PanelEntity::from_validated(source, PanelSpace::Screen);
        let second_handle: crate::WidgetEntity<crate::Screen> =
            crate::WidgetEntity::from_validated(second_widget, owner, PanelSpace::Screen);

        apply_attachment_commands(&mut app, |attachments| {
            attachments.retarget_to_widget(source_handle, second_handle);
            attachments.detach(source_handle);
        });

        assert_eq!(
            screen_reverse_sources(app.world(), first_widget),
            vec![source]
        );
        assert!(screen_reverse_sources(app.world(), second_widget).is_empty());
        assert_eq!(
            app.world()
                .get::<ScreenWidgetAnchoredTo>(source)
                .map(|relation| relation.target()),
            Some(first_widget),
        );
    }

    fn apply_attachment_commands(
        app: &mut App,
        queue_operations: impl FnOnce(&mut Commands<'_, '_>),
    ) {
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, app.world());
            queue_operations(&mut commands);
        }
        queue.apply(app.world_mut());
    }

    fn spawn_indexed_widget(app: &mut App, owner: Entity, id: &'static str) -> Entity {
        let id = crate::PanelElementId::named(id);
        let widget = app
            .world_mut()
            .spawn((PanelWidget::new(id.clone()), WidgetOf::new(owner)))
            .id();
        app.world_mut()
            .get_mut::<PanelWidgetIndex>(owner)
            .expect("panel has a widget index")
            .insert(id, widget);
        widget
    }

    fn screen_reverse_sources(world: &World, widget: Entity) -> Vec<Entity> {
        world
            .get::<ScreenWidgetAnchoredHere>(widget)
            .map(ScreenWidgetAnchoredHere::iter)
            .into_iter()
            .flatten()
            .collect()
    }

    fn reverse_targets(world: &World, target: Entity) -> Vec<Entity> {
        world
            .get::<AnchoredHere>(target)
            .map(|targets| targets.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn world_bundle_insert_creates_valence_relation_and_reverse_index() {
        let mut app = app_with_world_anchoring();
        let target = app
            .world_mut()
            .spawn((world_panel(Anchor::TopLeft), Transform::default()))
            .id();
        let authored_transform = Transform::from_xyz(0.25, 0.5, 0.75);
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                authored_transform,
                test_attachment(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();

        let relation = app
            .world()
            .get::<ValenceAnchoredTo>(source)
            .expect("world attachment has valence relation");
        assert_eq!(relation.target(), target);
        assert_eq!(reverse_targets(app.world(), target), vec![source]);
        assert_eq!(
            app.world()
                .get::<AnchoredWorldPanelPose>(source)
                .map(|pose| pose.authored_transform),
            Some(authored_transform),
        );
    }

    #[test]
    fn new_defaults_depth_to_zero_and_with_z_sets_it() {
        let offset = PanelAnchorOffset::new(Px(1.0), Px(2.0));
        assert_eq!(
            offset.z(),
            Dimension {
                value: 0.0,
                unit:  None,
            }
        );

        let offset = offset.with_z(Mm(3.0));
        assert_eq!(offset.x(), Px(1.0).into());
        assert_eq!(offset.y(), Px(2.0).into());
        assert_eq!(offset.z(), Mm(3.0).into());
    }

    #[test]
    fn to_layout_units_converts_depth_like_x_and_y() {
        let offset = PanelAnchorOffset::new(Mm(10.0), Mm(20.0)).with_z(Mm(30.0));
        let resolved = offset.to_layout_units(Unit::Millimeters);

        assert!(
            resolved.abs_diff_eq(Vec3::new(10.0, 20.0, 30.0), 1e-4),
            "expected (10, 20, 30), got {resolved:?}",
        );
    }

    #[test]
    fn anchor_types_are_registered_with_expected_reflect_component_data() {
        let mut app = App::new();
        app.add_plugins(HeadlessLayoutPlugin);

        let registry = app.world().resource::<AppTypeRegistry>().read();
        let authored_registered = registry
            .get(TypeId::of::<PanelAttachmentAuthored>())
            .is_some();
        let offset_registered = registry.get(TypeId::of::<PanelAnchorOffset>()).is_some();
        let valence_source_has_reflect_component = registry
            .get(TypeId::of::<ValenceAnchoredTo>())
            .expect("AnchoredTo is registered")
            .data::<ReflectComponent>()
            .is_some();
        let reverse_has_reflect_component = registry
            .get(TypeId::of::<AnchoredHere>())
            .expect("AnchoredHere is registered")
            .data::<ReflectComponent>()
            .is_some();
        let pose_has_reflect_component = registry
            .get(TypeId::of::<AnchorPose>())
            .expect("AnchorPose is registered")
            .data::<ReflectComponent>()
            .is_some();
        let geometry_has_reflect_component = registry
            .get(TypeId::of::<ResolvedAnchorGeometry>())
            .expect("ResolvedAnchorGeometry is registered")
            .data::<ReflectComponent>()
            .is_some();
        let resolved_offset_has_reflect_component = registry
            .get(TypeId::of::<ResolvedAnchorOffset>())
            .expect("ResolvedAnchorOffset is registered")
            .data::<ReflectComponent>()
            .is_some();
        drop(registry);

        assert!(!authored_registered);
        assert!(offset_registered);
        assert!(!valence_source_has_reflect_component);
        assert!(!reverse_has_reflect_component);
        assert!(pose_has_reflect_component);
        assert!(geometry_has_reflect_component);
        assert!(resolved_offset_has_reflect_component);
    }

    #[test]
    fn world_anchoring_respects_source_scale_and_parent_rotation() {
        let mut app = app_with_world_anchoring();
        let parent = app
            .world_mut()
            .spawn(Transform::from_rotation(Quat::from_rotation_z(
                std::f32::consts::FRAC_PI_2,
            )))
            .id();
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::from_xyz(2.0, 1.0, 0.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(0.5)),
                ChildOf(parent),
                test_attachment(target, Anchor::BottomRight, Anchor::TopLeft),
            ))
            .id();

        app.update();

        let source_global = global_transform(&app, source);
        let (scale, rotation, translation) = source_global.to_scale_rotation_translation();
        assert_close_3d(translation, Vec3::new(1.5, 1.25, 0.0));
        assert_close_quat(rotation, Quat::IDENTITY);
        assert_close_3d(scale, Vec3::splat(0.5));
    }

    #[test]
    fn pose_written_in_animation_set_lands_this_frame() {
        let mut app = app_with_world_anchoring();
        app.insert_resource(PoseLift(0.5));
        app.add_systems(
            PostUpdate,
            lift_anchored_pose.in_set(AnchorSystems::AnimatePose),
        );
        let (target, source) = spawn_lift_scene(&mut app);

        app.update();

        let target_pin = panel_anchor_point(&app, target, Anchor::Center);
        assert_close_3d(
            panel_anchor_point(&app, source, Anchor::Center),
            target_pin + Vec3::Z * 0.5,
        );
    }

    #[test]
    fn world_offset_lowering_tracks_target_size_and_layout_unit() {
        let mut app = app_with_world_anchoring();
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::from_scale(Vec3::splat(2.0)),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::default(),
                test_attachment_with_offset(
                    target,
                    Anchor::TopLeft,
                    Anchor::BottomLeft,
                    PanelAnchorOffset::new(Mm(50.0), Pt(72.0)).with_z(Mm(25.0)),
                ),
            ))
            .id();

        app.update();
        assert_close_3d(resolved_offset(&app, source), Vec3::new(1.0, -0.508, 0.5));

        app.world_mut()
            .get_mut::<DiegeticPanel>(target)
            .expect("target panel exists")
            .set_width(400.0);
        app.world_mut()
            .get_mut::<DiegeticPanel>(target)
            .expect("target panel exists")
            .set_height(200.0);
        app.update();

        assert_close_3d(resolved_offset(&app, source), Vec3::new(0.5, -0.254, 0.25));
    }

    #[test]
    fn widget_target_offset_uses_owner_units_and_transformed_scale() {
        let mut app = app_with_world_anchoring();
        app.add_plugins(WidgetsPlugin);
        let mut tree = LayoutBuilder::new(WORLD_WIDGET_PANEL_WIDTH, WORLD_WIDGET_PANEL_HEIGHT);
        tree.with(
            El::new()
                .size(WIDGET_WIDTH, WIDGET_HEIGHT)
                .button("offset-target"),
            |_| {},
        );
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::from_translation(WIDGET_OWNER_TRANSLATION)
                    .with_rotation(Quat::from_rotation_z(WIDGET_OWNER_ROTATION))
                    .with_scale(Vec3::splat(TRANSFORM_SCALE)),
            ))
            .id();
        app.world_mut()
            .commands()
            .set_tree(target, tree.build())
            .expect("widget target tree should be accepted");
        app.update();
        let widget = app
            .world()
            .get::<PanelWidgets>(target)
            .and_then(|widgets| widgets.first().copied())
            .expect("target widget should be reified");
        let widget_local = transform(&app, widget);
        assert_ne!(
            widget_local.translation,
            Vec3::ZERO,
            "widget target should be off the owner panel origin",
        );
        let owner_global = global_transform(&app, target);
        let expected_widget_position = owner_global.transform_point(widget_local.translation);
        assert_close_3d(
            global_transform(&app, widget).translation(),
            expected_widget_position,
        );
        let offset =
            PanelAnchorOffset::new(Px(WIDGET_OFFSET_PIXELS), Mm(WIDGET_OFFSET_MILLIMETERS))
                .with_z(Pt(WIDGET_OFFSET_POINTS));
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::default(),
                test_attachment_with_offset(widget, Anchor::Center, Anchor::TopRight, offset),
            ))
            .id();

        app.update();

        assert_close_3d(
            global_transform(&app, widget).translation(),
            expected_widget_position,
        );
        let target_panel = app
            .world()
            .get::<DiegeticPanel>(target)
            .expect("target panel should remain live");
        let world_per_layout_unit =
            target_panel.world_width() / target_panel.width() * TRANSFORM_SCALE;
        let expected = Vec3::new(
            WIDGET_OFFSET_PIXELS * Unit::Pixels.to_points() / Unit::Millimeters.to_points()
                * world_per_layout_unit,
            -WIDGET_OFFSET_MILLIMETERS * world_per_layout_unit,
            WIDGET_OFFSET_POINTS * Unit::Points.to_points() / Unit::Millimeters.to_points()
                * world_per_layout_unit,
        );
        assert_close_3d(resolved_offset(&app, source), expected);
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(widget)
                .map(AnchoredHere::len),
            Some(1),
        );
        assert_eq!(
            app.world()
                .get::<ValenceAnchoredTo>(widget)
                .map(ValenceAnchoredTo::target),
            Some(target),
        );
        assert!(
            app.world().get::<ResolvedAnchorGeometry>(source).is_some(),
            "source panel should publish anchor geometry",
        );
        assert!(
            app.world().get::<ResolvedAnchorGeometry>(widget).is_some(),
            "demanded widget should publish anchor geometry",
        );
        let widget_rotation = global_transform(&app, widget).rotation();
        assert_close_3d(
            resolved_anchor_point(&app, source, Anchor::Center),
            resolved_anchor_point(&app, widget, Anchor::TopRight) + widget_rotation * expected,
        );
    }

    #[test]
    fn newly_reified_widget_uses_owner_scale_for_offset_on_first_resolver_pass() {
        let mut app = app_with_world_anchoring();
        app.add_plugins(WidgetsPlugin);
        app.add_systems(
            Update,
            attach_source_to_new_widget
                .after(WidgetSystems::ReifyCommandsApplied)
                .before(PanelSystems::ResolvePanelAttachments),
        );
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::from_translation(WIDGET_OWNER_TRANSLATION)
                    .with_rotation(Quat::from_rotation_z(WIDGET_OWNER_ROTATION))
                    .with_scale(Vec3::splat(TRANSFORM_SCALE)),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((world_panel(Anchor::Center), Transform::default()))
            .id();
        app.insert_resource(PendingWidgetTarget {
            owner_panel:  target,
            source_panel: source,
            widget:       None,
        });

        app.update();

        let owner_scale = global_transform(&app, target)
            .to_scale_rotation_translation()
            .0;
        assert_close_3d(owner_scale, Vec3::splat(TRANSFORM_SCALE));
        let mut tree = LayoutBuilder::new(WORLD_WIDGET_PANEL_WIDTH, WORLD_WIDGET_PANEL_HEIGHT);
        tree.with(
            El::row().size(WORLD_WIDGET_PANEL_WIDTH, WORLD_WIDGET_PANEL_HEIGHT),
            |tree| {
                tree.with(El::new().size(WIDGET_WIDTH, WIDGET_HEIGHT), |_| {});
                tree.with(
                    El::new()
                        .size(WIDGET_WIDTH, WIDGET_HEIGHT)
                        .button("same-frame-offset-target"),
                    |_| {},
                );
            },
        );
        app.world_mut()
            .commands()
            .set_tree(target, tree.build())
            .expect("widget target tree should be accepted");

        app.update();

        let widget = app
            .world()
            .resource::<PendingWidgetTarget>()
            .widget
            .expect("new widget should be targeted in its reification frame");
        let widget_local = transform(&app, widget);
        assert_ne!(widget_local.translation, Vec3::ZERO);
        assert_close_3d(widget_local.scale, Vec3::ONE);
        let target_panel = app
            .world()
            .get::<DiegeticPanel>(target)
            .expect("target panel should remain live");
        let owner_global = global_transform(&app, target);
        let (owner_scale, owner_rotation, _) = owner_global.to_scale_rotation_translation();
        let (expected_widget_center, expected_widget_corner) =
            expected_widget_center_and_corner(target_panel, &owner_global);
        let widget_global = global_transform(&app, widget);
        let widget_scale = widget_global.to_scale_rotation_translation().0;
        assert_close_3d(widget_global.translation(), expected_widget_center);
        assert_close_3d(widget_scale, owner_scale);
        let world_per_layout_unit =
            target_panel.world_width() / target_panel.width() * owner_scale.x;
        let expected_offset = Vec3::new(
            WIDGET_OFFSET_PIXELS * Unit::Pixels.to_points() / Unit::Millimeters.to_points()
                * world_per_layout_unit,
            -WIDGET_OFFSET_MILLIMETERS * world_per_layout_unit,
            WIDGET_OFFSET_POINTS * Unit::Points.to_points() / Unit::Millimeters.to_points()
                * world_per_layout_unit,
        );
        assert_close_3d(resolved_offset(&app, source), expected_offset);
        assert_close_3d(
            global_transform(&app, source).translation(),
            expected_widget_corner + owner_rotation * expected_offset,
        );
    }

    #[test]
    fn missing_geometry_diagnostics_deduplicate_by_attachment_key() {
        let mut app = app_with_world_anchoring();
        let target = app
            .world_mut()
            .spawn((Transform::default(), GlobalTransform::default()))
            .id();
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::default(),
                test_attachment(target, Anchor::Center, Anchor::Center),
            ))
            .id();

        app.update();
        app.update();

        let diagnostics = app.world().resource::<AnchorResolveDiagnostics>();
        let entries = diagnostics
            .entries()
            .filter(|entry| {
                entry.source == source
                    && entry.target == target
                    && entry.reason == AnchorResolveSkip::MissingTargetGeometry
            })
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert!(
            entries
                .first()
                .is_some_and(|entry| entry.count == DIAGNOSTIC_REPEAT_COUNT),
        );
    }

    #[test]
    fn removing_panel_attachment_restores_authored_world_pose() {
        let mut app = app_with_world_anchoring();
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::from_xyz(2.0, 1.0, 0.0),
            ))
            .id();
        let authored = Transform::from_xyz(-1.0, -2.0, 0.0);
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                authored,
                test_attachment(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        assert_ne!(transform(&app, source), authored);

        app.world_mut()
            .entity_mut(source)
            .remove::<(PanelAttachmentAuthored, PanelAnchorOffset)>();
        app.update();

        assert_eq!(transform(&app, source), authored);
        assert!(app.world().get::<AnchoredWorldPanelPose>(source).is_none());
        assert!(app.world().get::<ValenceAnchoredTo>(source).is_none());
    }

    #[derive(Resource)]
    struct PoseLift(f32);

    #[derive(Resource)]
    struct PendingWidgetTarget {
        owner_panel:  Entity,
        source_panel: Entity,
        widget:       Option<Entity>,
    }

    fn attach_source_to_new_widget(
        mut pending: ResMut<PendingWidgetTarget>,
        panel_entities: PanelEntityReader,
        panel_widgets: PanelWidgetReader,
        mut attachments: Commands,
    ) {
        if pending.widget.is_some() {
            return;
        }
        let Some(owner) = panel_entities.world(pending.owner_panel) else {
            return;
        };
        let Some(source) = panel_entities.world(pending.source_panel) else {
            return;
        };
        let id = crate::PanelElementId::named("same-frame-offset-target");
        let Some(widget) = panel_widgets.typed_entity(owner, &id) else {
            return;
        };
        let offset =
            PanelAnchorOffset::new(Px(WIDGET_OFFSET_PIXELS), Mm(WIDGET_OFFSET_MILLIMETERS))
                .with_z(Pt(WIDGET_OFFSET_POINTS));
        let authored = PanelAttachment::new(Anchor::Center, Anchor::TopRight).with_offset(offset);
        attachments.attach_to_widget(source, widget, authored);
        pending.widget = Some(widget.entity());
    }

    fn expected_widget_center_and_corner(
        target_panel: &DiegeticPanel,
        owner_global: &GlobalTransform,
    ) -> (Vec3, Vec3) {
        let authored_widget_min = Vec2::new(WIDGET_WIDTH, 0.0);
        let authored_widget_size = Vec2::new(WIDGET_WIDTH, WIDGET_HEIGHT);
        let authored_widget_center = authored_widget_min + authored_widget_size / 2.0;
        let widget_layout_to_world =
            target_panel.layout_unit().to_points() * target_panel.points_to_world();
        let (panel_anchor_x, panel_anchor_y) = target_panel.anchor_offsets();
        let widget_center_local = Vec3::new(
            authored_widget_center
                .x
                .mul_add(widget_layout_to_world, -panel_anchor_x),
            (-authored_widget_center.y).mul_add(widget_layout_to_world, panel_anchor_y),
            0.0,
        );
        let widget_half_extents_local = Vec3::new(
            authored_widget_size.x * widget_layout_to_world / 2.0,
            authored_widget_size.y * widget_layout_to_world / 2.0,
            0.0,
        );
        (
            owner_global.transform_point(widget_center_local),
            owner_global.transform_point(widget_center_local + widget_half_extents_local),
        )
    }

    fn app_with_world_anchoring() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(TransformPlugin);
        app.insert_resource(DiegeticTextMeasurer {
            measure_fn: Arc::new(|_text: &str, measure: &TextMeasure| TextDimensions {
                width:       measure.size,
                height:      measure.size,
                line_height: measure.size,
            }),
        });
        app.add_plugins(HeadlessLayoutPlugin);
        app
    }

    fn world_panel(anchor: Anchor) -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Mm(200.0), Mm(100.0))
            .world_width(2.0)
            .anchor(anchor)
            .layout(|_| {})
            .build()
            .expect("world panel builds")
    }

    fn transform(app: &App, entity: Entity) -> Transform {
        app.world()
            .get::<Transform>(entity)
            .copied()
            .expect("entity has Transform")
    }

    fn global_transform(app: &App, entity: Entity) -> GlobalTransform {
        app.world()
            .get::<GlobalTransform>(entity)
            .copied()
            .expect("entity has GlobalTransform")
    }

    fn resolved_offset(app: &App, entity: Entity) -> Vec3 {
        app.world()
            .get::<ResolvedAnchorOffset>(entity)
            .copied()
            .expect("entity has ResolvedAnchorOffset")
            .offset()
            .into_inner()
    }

    fn panel_anchor_point(app: &App, entity: Entity, anchor: Anchor) -> Vec3 {
        let transform = transform(app, entity);
        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("entity has DiegeticPanel");
        let size = Vec2::new(panel.world_width(), panel.world_height());
        let panel_offset = anchor_offset(panel.anchor(), size);
        let source_offset = anchor_offset(anchor, size);
        transform.translation
            + transform.rotation
                * Vec3::new(
                    source_offset.x - panel_offset.x,
                    panel_offset.y - source_offset.y,
                    0.0,
                )
    }

    fn resolved_anchor_point(app: &App, entity: Entity, anchor: Anchor) -> Vec3 {
        let geometry = app
            .world()
            .get::<ResolvedAnchorGeometry>(entity)
            .expect("entity has ResolvedAnchorGeometry");
        let point = geometry
            .frame(hana_valence::AnchorSite::from(anchor))
            .expect("geometry contains requested anchor");
        global_transform(app, entity).transform_point(point.position().into_inner())
    }

    fn anchor_offset(anchor: Anchor, size: Vec2) -> Vec2 {
        let (x, y) = anchor.offset(size.x, size.y);
        Vec2::new(x, y)
    }

    fn assert_close_3d(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn assert_close_quat(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4) || actual.abs_diff_eq(-expected, 1e-4),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn lift_anchored_pose(lift: Res<PoseLift>, mut poses: Query<&mut AnchorPose>) {
        for mut pose in &mut poses {
            let mut translation = pose.translation.into_inner();
            translation.z = lift.0;
            pose.translation = Displacement::from(translation);
        }
    }

    fn spawn_lift_scene(app: &mut App) -> (Entity, Entity) {
        let target = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::from_xyz(1.0, 2.0, 0.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                world_panel(Anchor::TopLeft),
                Transform::default(),
                test_attachment(target, Anchor::Center, Anchor::Center),
                AnchorPose::default(),
            ))
            .id();
        (target, source)
    }

    #[test]
    fn screen_attachment_does_not_enter_valence_resolve_diagnostics() {
        let mut app = app_with_world_anchoring();
        let target = app
            .world_mut()
            .spawn((screen_panel(), Transform::default()))
            .id();
        let source = app
            .world_mut()
            .spawn((
                screen_panel(),
                Transform::default(),
                test_attachment(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();

        assert!(app.world().get::<ValenceAnchoredTo>(source).is_none());
        let diagnostics = app.world().resource::<AnchorResolveDiagnostics>();
        assert!(diagnostics.current().next().is_none());
    }

    fn screen_panel() -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(100.0), Px(40.0))
            .screen_position(10.0, 10.0)
            .layout(|_| {})
            .build()
            .expect("screen panel builds")
    }

    fn screen_panel_with_widget(id: &'static str) -> DiegeticPanel {
        let mut tree = LayoutBuilder::new(100.0, 40.0);
        tree.with(El::new().button(id), |_| {});
        DiegeticPanel::screen()
            .size(Px(100.0), Px(40.0))
            .screen_position(10.0, 10.0)
            .with_tree(tree.build())
            .build()
            .expect("screen widget panel builds")
    }
}
