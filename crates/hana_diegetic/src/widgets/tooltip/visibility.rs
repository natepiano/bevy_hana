use std::time::Duration;

use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
#[cfg(test)]
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::change_detection::MaybeLocation;
use bevy::ecs::event::EntityTrigger;
use bevy::ecs::event::EventKey;
#[cfg(test)]
use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::system::SystemParam;
use bevy::ecs::world::DeferredWorld;
#[cfg(test)]
use bevy::mesh::Mesh3d;
use bevy::picking::events::PointerMove;
use bevy::picking::events::PointerOver;
use bevy::picking::events::PointerPress;
use bevy::picking::hover::HoverMap;
use bevy::picking::hover::PickingInteraction;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::ContainsEntity;
use bevy::prelude::Despawn;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Has;
use bevy::prelude::Mut;
use bevy::prelude::On;
use bevy::prelude::Or;
#[cfg(test)]
use bevy::prelude::Pointer;
#[cfg(test)]
use bevy::prelude::PostUpdate;
#[cfg(test)]
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
#[cfg(test)]
use bevy::prelude::Transform;
#[cfg(test)]
use bevy::prelude::Update;
#[cfg(test)]
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;
#[cfg(test)]
use bevy::window::WindowRef;
#[cfg(test)]
use hana_valence::AnchoredHere;
use hana_valence::ResolvedAnchorGeometry;

use super::controller;
use super::controller::TooltipFor;
use super::controller::Tooltips;
use super::declaration::AuthoredTooltipTargetSpace;
use super::declaration::PrepareTooltip;
use super::declaration::Tooltip;
use super::declaration::TooltipDisabledPolicy;
use super::declaration::TooltipExclusivity;
use super::declaration::TooltipPhase;
use super::materialization::MaterializedTooltip;
use super::materialization::TooltipPresentationCamera;
use super::materialization::TooltipReadiness;
use super::materialization::TooltipWidthConstraint;
use super::materialization::TooltipWidthConstraintRequest;
use super::materialization::WorldTooltipFrameProjection;
use super::placement::TooltipPlacementState;
use super::target::MeshAnchorTarget;
#[cfg(test)]
use crate::layout::Anchor;
#[cfg(test)]
use crate::layout::Unit;
use crate::panel::DiegeticPanel;
#[cfg(test)]
use crate::panel::PanelAttachment;
use crate::panel::PanelSpace;
#[cfg(test)]
use crate::panel::World;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetFocusVisible;
use crate::widgets::WidgetOf;

/// Reports that a tooltip became visible.
#[derive(Clone, Copy, Debug, EntityEvent)]
pub struct TooltipShown {
    /// Tooltip controller that became visible.
    #[event_target]
    pub entity: Entity,
}

/// Reports that a tooltip became hidden.
#[derive(Clone, Copy, Debug, EntityEvent)]
pub struct TooltipHidden {
    /// Tooltip controller that became hidden.
    #[event_target]
    pub entity: Entity,
}

#[derive(Clone, Copy, Component)]
pub(crate) struct TooltipPointerCamera {
    camera: Entity,
}

impl TooltipPointerCamera {
    #[cfg(test)]
    pub(crate) const fn new(camera: Entity) -> Self { Self { camera } }
}

// This belongs to the target: declarations and their controllers may be replaced
// while the pointer remains over the same widget.
#[derive(Component)]
pub(super) struct TooltipPressSuppressed;

#[derive(Clone, Copy, Component)]
pub(super) struct TooltipShownPending;

#[derive(Resource)]
pub(super) struct TooltipHiddenEventKey(pub(super) EventKey);

fn remember_tooltip_camera(
    target: Entity,
    camera: Entity,
    tooltips: &Query<(), With<Tooltips>>,
    commands: &mut Commands<'_, '_>,
) {
    if tooltips.contains(target) {
        commands
            .entity(target)
            .insert(TooltipPointerCamera { camera });
    }
}

pub(super) fn remember_tooltip_camera_from_over(
    event: On<PointerOver>,
    tooltips: Query<(), With<Tooltips>>,
    mut commands: Commands,
) {
    remember_tooltip_camera(
        event.event_target(),
        event.event().hit.camera,
        &tooltips,
        &mut commands,
    );
}

pub(super) fn remember_tooltip_camera_from_move(
    event: On<PointerMove>,
    tooltips: Query<(), With<Tooltips>>,
    mut commands: Commands,
) {
    remember_tooltip_camera(
        event.event_target(),
        event.event().hit.camera,
        &tooltips,
        &mut commands,
    );
}

pub(super) fn remember_tooltip_camera_from_press(
    event: On<PointerPress>,
    tooltips: Query<&Tooltips>,
    mut controllers: Query<(&mut TooltipPhase, Option<&mut Visibility>)>,
    mut commands: Commands,
) {
    let target = event.event_target();
    let Ok(tooltips) = tooltips.get(target) else {
        return;
    };
    commands.entity(target).insert((
        TooltipPointerCamera {
            camera: event.hit.camera,
        },
        TooltipPressSuppressed,
    ));
    // Interaction may already be Hovered after a same-update release. The
    // press event itself owns suppression, before that transient state is lost.
    for entity in tooltips.iter() {
        let Ok((mut phase, visibility)) = controllers.get_mut(entity) else {
            continue;
        };
        let was_visible = phase.is_visible();
        *phase = TooltipPhase::HiddenUntilLeave;
        if let Some(mut visibility) = visibility {
            *visibility = Visibility::Hidden;
        }
        if was_visible {
            commands.trigger(TooltipHidden { entity });
        }
    }
}

pub(super) fn remove_tooltip_pointer_camera(event: On<Remove<Tooltips>>, mut commands: Commands) {
    commands
        .entity(event.entity)
        .try_remove::<TooltipPointerCamera>();
}

fn finalize_visible_tooltip_now(entity: Entity, world: &mut DeferredWorld<'_>) {
    let was_visible = world
        .get_mut::<TooltipPhase>(entity)
        .is_some_and(|mut phase| {
            if !phase.is_visible() {
                return false;
            }
            *phase = TooltipPhase::Hidden;
            true
        });
    if !was_visible {
        return;
    }
    if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
        *visibility = Visibility::Hidden;
    }
    let Some(event_key) = world
        .get_resource::<TooltipHiddenEventKey>()
        .map(|key| key.0)
    else {
        return;
    };
    let mut event = TooltipHidden { entity };
    let mut trigger = EntityTrigger;
    // SAFETY: `hidden_event` was registered for `TooltipHidden` in this same
    // world, and `trigger` is that event type's matching trigger.
    unsafe {
        world.trigger_raw(event_key, &mut event, &mut trigger, MaybeLocation::caller());
    }
}

pub(super) fn finalize_removed_tooltip(trigger: On<Remove<Tooltip>>, mut world: DeferredWorld<'_>) {
    finalize_visible_tooltip_now(trigger.entity, &mut world);
}

pub(super) fn finalize_despawned_tooltip(
    trigger: On<Despawn<Tooltip>>,
    mut world: DeferredWorld<'_>,
) {
    finalize_visible_tooltip_now(trigger.entity, &mut world);
}

pub(super) fn finalize_tooltip_panel_role(
    trigger: On<Remove<DiegeticPanel>>,
    mut world: DeferredWorld<'_>,
) {
    if world.get::<Tooltip>(trigger.entity).is_some() {
        finalize_visible_tooltip_now(trigger.entity, &mut world);
    }
    let targeted_tooltips = world
        .get::<Tooltips>(trigger.entity)
        .map(|tooltips| tooltips.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    for tooltip in targeted_tooltips {
        finalize_visible_tooltip_now(tooltip, &mut world);
    }
}

fn advance_one_tooltip_phase(
    phase: &mut TooltipPhase,
    tooltip: &Tooltip,
    eligible: bool,
    suppressed: bool,
    delta: Duration,
    contact: TooltipPointerContact,
) -> bool {
    if contact == TooltipPointerContact::Pressed {
        let emit_hidden = phase.is_visible();
        *phase = TooltipPhase::HiddenUntilLeave;
        return emit_hidden;
    }
    let mut next = None;
    let mut emit_hidden = false;
    match phase {
        TooltipPhase::HiddenUntilLeave if contact == TooltipPointerContact::Away => {
            next = Some(TooltipPhase::Hidden);
        },
        TooltipPhase::Hidden if eligible => {
            let mut timer = Timer::new(tooltip.show_after, TimerMode::Once);
            timer.tick(delta);
            next = Some(TooltipPhase::WaitingToShow(timer));
        },
        TooltipPhase::Hidden | TooltipPhase::HiddenUntilLeave => {},
        TooltipPhase::WaitingToShow(timer) if eligible => {
            timer.tick(delta);
        },
        TooltipPhase::WaitingToShow(_) => {
            next = Some(TooltipPhase::Hidden);
        },
        TooltipPhase::Visible if eligible => {},
        TooltipPhase::Visible => {
            if suppressed || tooltip.hide_after.is_zero() {
                next = Some(TooltipPhase::Hidden);
                emit_hidden = true;
            } else {
                let mut timer = Timer::new(tooltip.hide_after, TimerMode::Once);
                timer.tick(delta);
                if timer.is_finished() {
                    next = Some(TooltipPhase::Hidden);
                    emit_hidden = true;
                } else {
                    next = Some(TooltipPhase::WaitingToHide(timer));
                }
            }
        },
        TooltipPhase::WaitingToHide(_) if eligible => {
            next = Some(TooltipPhase::Visible);
        },
        TooltipPhase::WaitingToHide(timer) => {
            timer.tick(delta);
            if suppressed || timer.is_finished() {
                next = Some(TooltipPhase::Hidden);
                emit_hidden = true;
            }
        },
    }
    if let Some(next) = next {
        *phase = next;
    }
    emit_hidden
}

#[derive(SystemParam)]
pub(super) struct GeometryTooltipHover<'w, 's> {
    hover_map:       Option<Res<'w, HoverMap>>,
    parents:         Query<'w, 's, &'static ChildOf>,
    anchor_geometry: Query<'w, 's, (), Or<(With<ResolvedAnchorGeometry>, With<MeshAnchorTarget>)>>,
    tooltip_owners:  Query<'w, 's, &'static Tooltips>,
    controllers:     Query<'w, 's, (), With<Tooltip>>,
    pressed_targets: Query<'w, 's, Entity, With<TooltipPressSuppressed>>,
}

impl GeometryTooltipHover<'_, '_> {
    fn clear_departed_presses(&self, hovered: &[Entity], commands: &mut Commands) {
        for target in &self.pressed_targets {
            let over_target = if self.hover_map.is_some() {
                self.hit_path_contains(target)
            } else {
                hovered.iter().any(|&entity| {
                    entity == target
                        || self
                            .parents
                            .iter_ancestors(entity)
                            .any(|ancestor| ancestor == target)
                })
            };
            if !over_target {
                commands.entity(target).remove::<TooltipPressSuppressed>();
            }
        }
    }

    fn hit_path_contains(&self, entity: Entity) -> bool {
        self.hover_map.as_ref().is_none_or(|map| {
            map.values()
                .filter_map(|hits| {
                    hits.iter()
                        .min_by(|(_, left), (_, right)| left.depth.total_cmp(&right.depth))
                        .map(|(&hit, _)| hit)
                })
                .any(|hit| {
                    let mut ancestor = hit;
                    while let Ok(parent) = self.parents.get(ancestor) {
                        if ancestor == entity {
                            return true;
                        }
                        ancestor = parent.parent();
                    }
                    ancestor == entity
                })
        })
    }

    fn camera_for(
        &self,
        target: Entity,
        controls: &Query<(), With<PanelWidget>>,
        show_over_descendant_controls: bool,
    ) -> Option<Entity> {
        if !self.anchor_geometry.contains(target) {
            return None;
        }
        self.hover_map
            .iter()
            .flat_map(|map| map.values())
            .filter_map(|hits| {
                hits.iter()
                    .min_by(|(_, left), (_, right)| left.depth.total_cmp(&right.depth))
            })
            .find_map(|(&entity, hit)| {
                let mut ancestor = entity;
                while let Ok(parent) = self.parents.get(ancestor) {
                    if ancestor == target {
                        return Some(hit.camera);
                    }
                    if !show_over_descendant_controls && controls.contains(ancestor) {
                        return None;
                    }
                    if self.tooltip_owners.get(ancestor).is_ok_and(|owner| {
                        owner
                            .iter()
                            .any(|controller| self.controllers.contains(controller))
                    }) {
                        return None;
                    }
                    ancestor = parent.parent();
                }
                (ancestor == target).then_some(hit.camera)
            })
    }
}

pub(super) fn advance_tooltip_visibility(
    time: Option<Res<Time>>,
    mut controllers: Query<(
        Entity,
        &Tooltip,
        &TooltipFor,
        Option<&AuthoredTooltipTargetSpace>,
        Option<&MaterializedTooltip>,
        Option<&TooltipPresentationCamera>,
        Has<PrepareTooltip>,
        &mut TooltipPhase,
        Option<&mut Visibility>,
        Option<&mut TooltipPlacementState>,
        Option<&TooltipWidthConstraint>,
    )>,
    hovered_entities: Query<(Entity, &PickingInteraction)>,
    geometry_hover: GeometryTooltipHover,
    controls: Query<(), With<PanelWidget>>,
    targets: Query<(
        Option<&PickingInteraction>,
        Has<WidgetFocusVisible>,
        Has<WidgetDisabled>,
        Option<&TooltipPointerCamera>,
        Has<TooltipPressSuppressed>,
    )>,
    panels: Query<&DiegeticPanel>,
    widgets: Query<&WidgetOf, With<PanelWidget>>,
    render_layers: Query<&RenderLayers>,
    cameras: Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: Query<(), With<Window>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    focus_authority: Res<WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let delta = time.as_deref().map_or(Duration::ZERO, Time::delta);
    let (hovered_descendants, pressed_descendants) = collect_hovered_descendants(&hovered_entities);
    geometry_hover.clear_departed_presses(&hovered_descendants, &mut commands);
    for (
        entity,
        tooltip,
        tooltip_for,
        authored_space,
        materialized,
        presentation_camera,
        preparing,
        mut phase,
        mut visibility,
        placement,
        width_constraint,
    ) in &mut controllers
    {
        let target = tooltip_for.target();
        let Ok((interaction, focus_visible, disabled, pointer_camera, press_suppressed)) =
            targets.get(target)
        else {
            continue;
        };
        let pointer = tooltip_pointer_state(
            target,
            interaction,
            (&hovered_descendants, &pressed_descendants),
            &geometry_hover,
            &controls,
            tooltip.show_over_descendant_controls,
        );
        if press_suppressed && pointer.contact != TooltipPointerContact::Away {
            *phase = TooltipPhase::HiddenUntilLeave;
        }
        let suppressed = disabled && tooltip.disabled_policy == TooltipDisabledPolicy::Suppress;
        let hovered = pointer.contact != TooltipPointerContact::Away;
        let eligible = pointer.eligible(&phase, focus_visible, suppressed);

        if matches!(*phase, TooltipPhase::Hidden) && eligible {
            reset_new_show(entity, placement, width_constraint, &mut commands);
        }
        if advance_one_tooltip_phase(
            &mut phase,
            tooltip,
            eligible,
            suppressed,
            delta,
            pointer.contact,
        ) {
            if let Some(visibility) = visibility.as_deref_mut() {
                *visibility = Visibility::Hidden;
            }
            commands.trigger(TooltipHidden { entity });
        }

        if !eligible && !focus_visible {
            continue;
        }
        let Some((space, target_layers)) = tooltip_target_presentation(
            target,
            authored_space,
            materialized,
            &panels,
            &widgets,
            &render_layers,
        ) else {
            continue;
        };
        let geometry_camera = pointer.camera.pointer_camera();
        let camera = if space == PanelSpace::World {
            select_tooltip_presentation_camera(
                target,
                geometry_camera
                    .as_ref()
                    .or_else(|| hovered.then_some(pointer_camera).flatten()),
                focus_visible,
                &target_layers,
                &focus_authority,
                &cameras,
                &windows,
                &primary_window,
            )
        } else {
            None
        };
        if space == PanelSpace::World
            && !phase.is_visible()
            && remember_presentation_camera(
                entity,
                presentation_camera.map(|current| current.camera).into(),
                camera.into(),
                &mut commands,
            ) == TooltipPresentationDecision::WaitForCamera
        {
            continue;
        }
        if eligible && materialized.is_none() && !preparing {
            commands.entity(entity).insert(PrepareTooltip);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TooltipCameraAvailability {
    Unavailable,
    Available(Entity),
}

impl From<Option<Entity>> for TooltipCameraAvailability {
    fn from(camera: Option<Entity>) -> Self { camera.map_or(Self::Unavailable, Self::Available) }
}

#[derive(PartialEq, Eq)]
enum TooltipPresentationDecision {
    WaitForCamera,
    Ready,
}

fn remember_presentation_camera(
    entity: Entity,
    current: TooltipCameraAvailability,
    camera: TooltipCameraAvailability,
    commands: &mut Commands,
) -> TooltipPresentationDecision {
    let TooltipCameraAvailability::Available(camera) = camera else {
        if current != TooltipCameraAvailability::Unavailable {
            commands
                .entity(entity)
                .remove::<TooltipPresentationCamera>();
        }
        return TooltipPresentationDecision::WaitForCamera;
    };
    if current != TooltipCameraAvailability::Available(camera) {
        commands
            .entity(entity)
            .insert(TooltipPresentationCamera { camera });
    }
    TooltipPresentationDecision::Ready
}

/// An active jack help retires older help surfaces while preserving their declarations.
pub(super) fn retire_superseded_help_panels(
    controllers: Query<(Entity, &Tooltip, &TooltipPhase)>,
    materialized: Query<(Entity, &MaterializedTooltip), With<DiegeticPanel>>,
    mut commands: Commands,
) {
    let active_help = controllers.iter().filter_map(|(entity, tooltip, phase)| {
        (tooltip.exclusivity == TooltipExclusivity::SoleHelp
            && matches!(
                phase,
                TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible
            ))
        .then_some(entity)
    });
    for sole_help in active_help.take(1) {
        for (entity, state) in &materialized {
            if entity != sole_help {
                let mut controller = commands.entity(entity);
                controller.remove::<DiegeticPanel>();
                controller::remove_materialized_state(&mut controller, state);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TooltipPointerContact {
    Away,
    Hovering,
    Pressed,
}

enum TooltipPointerCameraContact {
    NoGeometryHit,
    OverGeometry(Entity),
}

impl TooltipPointerCameraContact {
    // Convert to the picking camera component at the presentation API boundary.
    const fn pointer_camera(self) -> Option<TooltipPointerCamera> {
        match self {
            Self::OverGeometry(camera) => Some(TooltipPointerCamera { camera }),
            Self::NoGeometryHit => None,
        }
    }
}

struct TooltipPointerState {
    contact: TooltipPointerContact,
    camera:  TooltipPointerCameraContact,
}

impl TooltipPointerState {
    fn eligible(&self, phase: &TooltipPhase, focus_visible: bool, suppressed: bool) -> bool {
        (self.contact != TooltipPointerContact::Away || focus_visible)
            && !suppressed
            && self.contact != TooltipPointerContact::Pressed
            && !matches!(phase, TooltipPhase::HiddenUntilLeave)
    }
}

fn tooltip_pointer_state(
    target: Entity,
    interaction: Option<&PickingInteraction>,
    descendants: (&[Entity], &[Entity]),
    geometry: &GeometryTooltipHover,
    controls: &Query<(), With<PanelWidget>>,
    show_descendants: bool,
) -> TooltipPointerState {
    let picking_hovered = tooltip_target_hovered(
        target,
        matches!(
            interaction,
            Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
        ),
        descendants.0,
        geometry,
        controls,
        show_descendants,
    );
    let camera = geometry.camera_for(target, controls, show_descendants);
    let pressed = tooltip_target_hovered(
        target,
        interaction == Some(&PickingInteraction::Pressed),
        descendants.1,
        geometry,
        controls,
        show_descendants,
    );
    TooltipPointerState {
        contact: if pressed {
            TooltipPointerContact::Pressed
        } else if picking_hovered || camera.is_some() {
            TooltipPointerContact::Hovering
        } else {
            TooltipPointerContact::Away
        },
        camera:  match camera {
            Some(camera) => TooltipPointerCameraContact::OverGeometry(camera),
            None => TooltipPointerCameraContact::NoGeometryHit,
        },
    }
}

fn collect_hovered_descendants(
    interactions: &Query<(Entity, &PickingInteraction)>,
) -> (Vec<Entity>, Vec<Entity>) {
    let mut hovered = Vec::new();
    let mut pressed = Vec::new();
    for (entity, state) in interactions {
        if matches!(
            state,
            PickingInteraction::Hovered | PickingInteraction::Pressed
        ) {
            hovered.push(entity);
        }
        if *state == PickingInteraction::Pressed {
            pressed.push(entity);
        }
    }
    (hovered, pressed)
}

fn reset_new_show(
    entity: Entity,
    placement: Option<Mut<'_, TooltipPlacementState>>,
    width_constraint: Option<&TooltipWidthConstraint>,
    commands: &mut Commands<'_, '_>,
) {
    if let Some(mut placement) = placement {
        placement.reset_new_show();
    }
    if width_constraint.is_some() {
        commands
            .entity(entity)
            .insert(TooltipWidthConstraintRequest::Restore);
    }
}

fn tooltip_target_hovered(
    target: Entity,
    target_hovered: bool,
    hovered_entities: &[Entity],
    hover: &GeometryTooltipHover<'_, '_>,
    controls: &Query<(), With<PanelWidget>>,
    show_over_descendant_controls: bool,
) -> bool {
    // Bevy may keep an ancestor's interaction hovered while another surface
    // wins the current ray. Only the live hit path may own help for that ray.
    let mut hovered = target_hovered && hover.hit_path_contains(target);
    let mut claimed_by_descendant = false;
    for &entity in hovered_entities {
        if !hover.hit_path_contains(entity) {
            continue;
        }
        let mut ancestor = entity;
        let live_owner = |entity| {
            hover.tooltip_owners.get(entity).is_ok_and(|owner| {
                owner
                    .iter()
                    .any(|controller| hover.controllers.contains(controller))
            })
        };
        let mut claims_hover =
            live_owner(entity) || (!show_over_descendant_controls && controls.contains(entity));
        while let Ok(parent) = hover.parents.get(ancestor) {
            ancestor = parent.parent();
            if ancestor == target {
                hovered = true;
                claimed_by_descendant |= claims_hover;
                break;
            }
            claims_hover |= live_owner(ancestor)
                || (!show_over_descendant_controls && controls.contains(ancestor));
        }
    }
    hovered && !claimed_by_descendant
}

fn tooltip_target_presentation(
    target: Entity,
    authored_space: Option<&AuthoredTooltipTargetSpace>,
    materialized: Option<&MaterializedTooltip>,
    panels: &Query<&DiegeticPanel>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    render_layers: &Query<&RenderLayers>,
) -> Option<(PanelSpace, RenderLayers)> {
    if let Some(materialized) = materialized {
        return Some((materialized.space(), materialized.render_layers.clone()));
    }
    let presentation_entity = if let Ok(panel) = panels.get(target) {
        return Some((
            PanelSpace::from(panel.coordinate_space()),
            render_layers
                .get(target)
                .cloned()
                .unwrap_or_else(|_| RenderLayers::layer(0)),
        ));
    } else if let Ok(widget_of) = widgets.get(target) {
        widget_of.panel()
    } else {
        return authored_space.map(|space| {
            (
                space.0,
                render_layers
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|_| RenderLayers::layer(0)),
            )
        });
    };
    let panel = panels.get(presentation_entity).ok()?;
    Some((
        PanelSpace::from(panel.coordinate_space()),
        render_layers
            .get(presentation_entity)
            .cloned()
            .unwrap_or_else(|_| RenderLayers::layer(0)),
    ))
}

fn select_tooltip_presentation_camera(
    target: Entity,
    pointer_camera: Option<&TooltipPointerCamera>,
    focus_visible: bool,
    target_layers: &RenderLayers,
    focus_authority: &WidgetFocusAuthority,
    cameras: &Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: &Query<(), With<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    if let Some(camera) = pointer_camera.map(|camera| camera.camera)
        && compatible_presentation_camera(
            camera,
            None,
            target_layers,
            cameras,
            windows,
            primary_window,
        )
    {
        return Some(camera);
    }
    let (window, _, preferred) = focus_visible
        .then(|| focus_authority.tooltip_focus_context(target))
        .flatten()?;
    select_window_presentation_camera(
        preferred,
        window,
        target_layers,
        cameras,
        windows,
        primary_window,
    )
}

pub(crate) fn select_window_presentation_camera(
    preferred: Option<Entity>,
    window: Entity,
    target_layers: &RenderLayers,
    cameras: &Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: &Query<(), With<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    if let Some(camera) = preferred
        && compatible_presentation_camera(
            camera,
            Some(window),
            target_layers,
            cameras,
            windows,
            primary_window,
        )
    {
        return Some(camera);
    }
    cameras
        .iter()
        .filter(|(entity, ..)| {
            compatible_presentation_camera(
                *entity,
                Some(window),
                target_layers,
                cameras,
                windows,
                primary_window,
            )
        })
        .max_by(|(left_entity, left, ..), (right_entity, right, ..)| {
            left.order
                .cmp(&right.order)
                .then_with(|| right_entity.to_bits().cmp(&left_entity.to_bits()))
        })
        .map(|(entity, ..)| entity)
}

fn compatible_presentation_camera(
    entity: Entity,
    required_window: Option<Entity>,
    target_layers: &RenderLayers,
    cameras: &Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: &Query<(), With<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> bool {
    let Ok((_, camera, global_transform, render_target, camera_layers)) = cameras.get(entity)
    else {
        return false;
    };
    if !camera.is_active || !global_transform.affine().is_finite() {
        return false;
    }
    let primary = primary_window.single().ok();
    let Some(NormalizedRenderTarget::Window(window)) =
        render_target.and_then(|target| target.normalize(primary))
    else {
        return false;
    };
    let window = window.entity();
    if windows.get(window).is_err() || required_window.is_some_and(|required| required != window) {
        return false;
    }
    camera_layers
        .cloned()
        .unwrap_or_else(|| RenderLayers::layer(0))
        .intersects(target_layers)
}

pub(super) fn reveal_ready_tooltips(
    mut controllers: Query<(
        Entity,
        &mut TooltipPhase,
        &TooltipReadiness,
        &mut Visibility,
        Option<&WorldTooltipFrameProjection>,
    )>,
    mut commands: Commands,
) {
    for (entity, mut phase, readiness, mut visibility, frame) in &mut controllers {
        let TooltipPhase::WaitingToShow(timer) = &*phase else {
            continue;
        };
        if !timer.is_finished() || *readiness != TooltipReadiness::Ready {
            continue;
        }
        if frame.is_some_and(|frame| !frame.allows_reveal()) {
            continue;
        }
        *phase = TooltipPhase::Visible;
        *visibility = Visibility::Inherited;
        commands.entity(entity).insert(TooltipShownPending);
    }
}

pub(super) fn emit_tooltip_shown(
    controllers: Query<(Entity, &TooltipPhase), With<TooltipShownPending>>,
    mut commands: Commands,
) {
    for (entity, phase) in &controllers {
        commands.entity(entity).remove::<TooltipShownPending>();
        if phase.is_visible() {
            commands.trigger(TooltipShown { entity });
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::backend::HitData;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PointerButton;
    use bevy::prelude::Vec3A;
    use bevy::prelude::World as BevyWorld;
    use bevy::time::TimeUpdateStrategy;
    use bevy::transform::TransformPlugin;
    use hana_valence::AnchoredTo;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::layout::El;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetFocusAuthority;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::controller::TooltipCommandsExt;
    use crate::widgets::tooltip::materialization::PreMaterializationComponent;
    use crate::widgets::tooltip::materialization::TooltipPresentationSpace;
    use crate::widgets::tooltip::target::MeshAnchorCommandsExt;
    use crate::widgets::tooltip::target::MeshFace;
    use crate::widgets::tooltip::target::TooltipTargetEntity;

    const MATERIALIZATION_UPDATES: usize = 5;

    const TEST_CAMERA_DISTANCE: f32 = 5.0;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    #[derive(Default, Resource)]
    struct TooltipVisibilityLog {
        shown:  Vec<Entity>,
        hidden: Vec<Entity>,
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn materialization_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app
    }

    #[test]
    fn sole_jack_help_retires_other_panels_and_preserves_their_rebuild_path() -> Result<(), String>
    {
        let mut world = BevyWorld::new();
        let target = world.spawn_empty().id();
        let prior = Tooltip::new(El::new());
        let prior_panel = DiegeticPanel::world()
            .size(Mm(1.0), Mm(1.0))
            .with_tree(LayoutBuilder::new(1.0, 1.0).build())
            .build()
            .map_err(|error| format!("prior help panel: {error}"))?;
        let materialized = MaterializedTooltip {
            target,
            presentation: TooltipPresentationSpace::World,
            layout_unit: Unit::Millimeters,
            render_layers: RenderLayers::layer(0),
            authored_attachment: prior.attachment(),
            previous_transform: PreMaterializationComponent::Absent,
            previous_global: PreMaterializationComponent::Absent,
            previous_visibility: PreMaterializationComponent::Absent,
        };
        let prior_entity = world
            .spawn((
                prior,
                TooltipPhase::Hidden,
                prior_panel,
                materialized.clone(),
            ))
            .id();
        let active_panel = DiegeticPanel::world()
            .size(Mm(1.0), Mm(1.0))
            .with_tree(LayoutBuilder::new(1.0, 1.0).build())
            .build()
            .map_err(|error| format!("active help panel: {error}"))?;
        let active_help = Tooltip::new(El::new()).one_help_at_a_time();
        let active_materialized = MaterializedTooltip {
            authored_attachment: active_help.attachment(),
            ..materialized
        };
        let active = world
            .spawn((
                active_help,
                TooltipPhase::Visible,
                active_panel,
                active_materialized,
            ))
            .id();
        world
            .run_system_once(retire_superseded_help_panels)
            .map_err(|error| format!("retire competing help: {error}"))?;

        assert!(world.get::<DiegeticPanel>(prior_entity).is_none());
        assert!(world.get::<MaterializedTooltip>(prior_entity).is_none());
        assert!(world.get::<Tooltip>(prior_entity).is_some());
        assert!(world.get::<DiegeticPanel>(active).is_some());
        Ok(())
    }

    fn update_materialization(app: &mut App) {
        for _ in 0..MATERIALIZATION_UPDATES {
            app.update();
        }
    }

    fn mesh_target(app: &mut App, half_extents: Vec3A) -> Entity {
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center: Vec3A::ZERO,
                    half_extents,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        target
    }

    fn standalone_world_tooltip(app: &mut App, target: Entity, tooltip: Tooltip) -> Entity {
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        app.world_mut().commands().spawn_tooltip(handle, tooltip)
    }

    #[test]
    fn geometry_tooltip_child_with_its_own_help_claims_the_hover() {
        let mut app = materialization_app();
        app.init_resource::<HoverMap>();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = mesh_target(&mut app, Vec3A::splat(0.5));
        let child = mesh_target(&mut app, Vec3A::splat(0.25));
        app.world_mut().entity_mut(child).insert(ChildOf(target));
        let parent_controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(60.0, 30.0))
                .show_after(Duration::ZERO)
                .show_over_descendant_controls(),
        );
        let child_controller = standalone_world_tooltip(
            &mut app,
            child,
            fixed_size_tooltip(Vec2::new(60.0, 30.0)).show_after(Duration::ZERO),
        );
        app.world_mut()
            .resource_mut::<HoverMap>()
            .entry(PointerId::Mouse)
            .or_default()
            .insert(child, HitData::new(camera, 0.0, None, None));
        app.update();
        app.update();

        assert!(matches!(
            app.world().get::<TooltipPhase>(child_controller),
            Some(TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible)
        ));
        assert!(matches!(
            app.world().get::<TooltipPhase>(parent_controller),
            Some(TooltipPhase::Hidden)
        ));
        assert_eq!(
            app.world()
                .get::<TooltipPresentationCamera>(child_controller)
                .map(|view| view.camera),
            Some(camera),
        );
    }

    #[test]
    fn geometry_tooltip_empty_child_owner_keeps_parent_help() {
        let mut app = materialization_app();
        app.init_resource::<HoverMap>();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = mesh_target(&mut app, Vec3A::splat(0.5));
        let child = mesh_target(&mut app, Vec3A::splat(0.25));
        app.world_mut()
            .entity_mut(child)
            .insert((ChildOf(target), Tooltips::default()));
        let parent_controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(60.0, 30.0))
                .show_after(Duration::ZERO)
                .show_over_descendant_controls(),
        );
        app.world_mut()
            .resource_mut::<HoverMap>()
            .entry(PointerId::Mouse)
            .or_default()
            .insert(child, HitData::new(camera, 0.0, None, None));
        app.update();
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(parent_controller),
            Some(TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible)
        ));
    }

    fn prepare_tooltip(app: &mut App, controller: Entity) {
        app.world_mut()
            .entity_mut(controller)
            .insert(PrepareTooltip);
    }

    fn record_tooltip_shown(event: On<TooltipShown>, mut world: DeferredWorld<'_>) {
        assert!(
            world.get::<Tooltip>(event.entity).is_some()
                && world.get::<TooltipFor>(event.entity).is_some()
                && world.get::<GlobalTransform>(event.entity).is_some(),
            "shown observers should see complete tooltip data",
        );
        let mut log = world.resource_mut::<TooltipVisibilityLog>();
        log.shown.push(event.entity);
    }

    fn record_tooltip_hidden(event: On<TooltipHidden>, mut world: DeferredWorld<'_>) {
        assert!(
            world.get::<Tooltip>(event.entity).is_some()
                && world.get::<TooltipFor>(event.entity).is_some()
                && world.get::<GlobalTransform>(event.entity).is_some(),
            "hidden observers should see complete tooltip data",
        );
        let mut log = world.resource_mut::<TooltipVisibilityLog>();
        log.hidden.push(event.entity);
    }

    fn visibility_app(step: Duration) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(step))
            .init_resource::<WidgetFocusAuthority>()
            .init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_shown)
            .add_observer(record_tooltip_hidden)
            .add_systems(Update, advance_tooltip_visibility)
            .add_systems(
                PostUpdate,
                (reveal_ready_tooltips, ApplyDeferred, emit_tooltip_shown).chain(),
            );
        app
    }

    fn visibility_controller(app: &mut App, tooltip: Tooltip) -> (Entity, Entity) {
        let target = app.world_mut().spawn(PickingInteraction::None).id();
        let controller = app
            .world_mut()
            .spawn((
                tooltip,
                TooltipFor::new(target),
                AuthoredTooltipTargetSpace(PanelSpace::Screen),
                TooltipReadiness::Ready,
                Visibility::Hidden,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        (target, controller)
    }

    fn set_interaction(app: &mut App, target: Entity, interaction: PickingInteraction) {
        app.world_mut().entity_mut(target).insert(interaction);
    }

    fn descendant_hover_eligible(
        app: &mut App,
        target: Entity,
        show_over_descendant_controls: bool,
    ) -> bool {
        app.world_mut()
            .run_system_once(
                move |hovered_entities: Query<(Entity, &PickingInteraction)>,
                      hover: GeometryTooltipHover,
                      controls: Query<(), With<PanelWidget>>| {
                    tooltip_target_hovered(
                        target,
                        true,
                        &hovered_entities
                            .iter()
                            .filter_map(|(entity, state)| {
                                matches!(
                                    state,
                                    PickingInteraction::Hovered | PickingInteraction::Pressed
                                )
                                .then_some(entity)
                            })
                            .collect::<Vec<_>>(),
                        &hover,
                        &controls,
                        show_over_descendant_controls,
                    )
                },
            )
            .unwrap_or(false)
    }

    #[test]
    fn noninteractive_descendant_keeps_parent_hover_but_own_tooltip_or_control_takes_over() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let target = app.world_mut().spawn(PickingInteraction::Hovered).id();
        let child = app
            .world_mut()
            .spawn((PickingInteraction::Hovered, ChildOf(target)))
            .id();
        assert!(descendant_hover_eligible(&mut app, target, false));

        app.world_mut()
            .entity_mut(child)
            .insert(Tooltips::default());
        assert!(descendant_hover_eligible(&mut app, target, false));
        assert!(descendant_hover_eligible(&mut app, target, true));

        app.world_mut().entity_mut(child).remove::<Tooltips>();
        let controller = app
            .world_mut()
            .spawn((Tooltip::new(El::new()), TooltipFor::new(child)))
            .id();
        assert!(!descendant_hover_eligible(&mut app, target, false));
        assert!(!descendant_hover_eligible(&mut app, target, true));
        app.world_mut().despawn(controller);
        app.world_mut()
            .entity_mut(child)
            .insert(PanelWidget::new(PanelElementId::named("child")));
        assert!(!descendant_hover_eligible(&mut app, target, false));
        assert!(descendant_hover_eligible(&mut app, target, true));
    }

    #[test]
    fn a_pick_face_hit_keeps_parent_help_when_a_control_retains_stale_hover() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<HoverMap>();
        let target = app.world_mut().spawn(PickingInteraction::Hovered).id();
        let pick_face = app
            .world_mut()
            .spawn((PickingInteraction::Hovered, ChildOf(target)))
            .id();
        let row = app
            .world_mut()
            .spawn((PickingInteraction::Hovered, ChildOf(target)))
            .id();
        let control = app
            .world_mut()
            .spawn((PickingInteraction::Hovered, ChildOf(row)))
            .id();
        app.world_mut()
            .spawn((Tooltip::new(El::new()), TooltipFor::new(control)));
        app.world_mut()
            .resource_mut::<HoverMap>()
            .entry(PointerId::Mouse)
            .or_default()
            .insert(
                pick_face,
                HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            );
        assert!(
            descendant_hover_eligible(&mut app, target, true),
            "the picked face owns this hover even while the control retains old interaction state"
        );
    }

    fn add_synthetic_pointer(app: &mut App, pointer: PointerId, location: Location) {
        app.world_mut()
            .spawn((pointer, PointerLocation::new(location)));
    }

    fn seeded_perspective_camera() -> Camera { seeded_perspective_camera_for(TEST_VIEWPORT) }

    fn seeded_perspective_camera_for(viewport: Vec2) -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: viewport.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(viewport.x, viewport.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn spawn_presentation_camera(
        app: &mut App,
        window: Entity,
        render_layers: RenderLayers,
    ) -> Entity {
        let transform = Transform::from_xyz(0.0, 0.0, TEST_CAMERA_DISTANCE);
        app.world_mut()
            .spawn((
                seeded_perspective_camera(),
                RenderTarget::Window(WindowRef::Entity(window)),
                render_layers,
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
    }

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    fn tooltip_tree(tooltip: Option<Tooltip>) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let button = El::new().size(40.0, 20.0).button("action");
        match tooltip {
            Some(tooltip) => {
                builder.with(button.tooltip(tooltip), |_| {});
            },
            None => {
                builder.with(button, |_| {});
            },
        }
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: crate::LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let panel = result.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(panel, Entity::PLACEHOLDER);
        panel
    }

    fn widget(app: &mut App, panel: Entity) -> Entity {
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("action"))
            })
            .ok()
            .flatten()
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(widget, Entity::PLACEHOLDER);
        widget
    }

    fn controller(app: &App, widget: Entity) -> Entity {
        let controller = app
            .world()
            .get::<crate::widgets::Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(controller, Entity::PLACEHOLDER);
        controller
    }

    #[test]
    fn tooltip_short_click_hides_until_leave_with_real_pointer_input() {
        let mut app = visibility_app(Duration::from_millis(20));
        app.add_plugins(InteractionPlugin)
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .init_resource::<PointerMap>()
            .add_observer(remember_tooltip_camera_from_press);
        let window = app.world_mut().spawn(Window::default()).id();
        let normalized = RenderTarget::Window(WindowRef::Entity(window)).normalize(Some(window));
        assert!(normalized.is_some());
        let Some(normalized) = normalized else { return };
        let location = Location {
            target:   normalized,
            position: Vec2::splat(50.0),
        };
        let pointer = PointerId::Touch(92);
        add_synthetic_pointer(&mut app, pointer, location.clone());
        let tooltip = Tooltip::new(El::new()).show_after(Duration::ZERO);
        let (target, controller) = visibility_controller(&mut app, tooltip);
        let hit = HitData::new(Entity::PLACEHOLDER, 0.0, None, None);
        for _ in 0..3 {
            app.world_mut().write_message(PointerHits::new(
                pointer,
                vec![(target, hit.clone())],
                0.0,
            ));
            app.world_mut().write_message(PointerInput::new(
                pointer,
                location.clone(),
                PointerAction::Move { delta: Vec2::ZERO },
            ));
            app.update();
        }
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
        app.world_mut()
            .write_message(PointerHits::new(pointer, vec![(target, hit.clone())], 0.0));
        for action in [
            PointerAction::Press(PointerButton::Primary),
            PointerAction::Release(PointerButton::Primary),
        ] {
            app.world_mut()
                .write_message(PointerInput::new(pointer, location.clone(), action));
        }
        app.update();
        assert_eq!(
            app.world().get::<PickingInteraction>(target),
            Some(&PickingInteraction::Hovered)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden),
            "a press and release in one input batch must hide help"
        );
        for _ in 0..3 {
            app.world_mut().write_message(PointerHits::new(
                pointer,
                vec![(target, hit.clone())],
                0.0,
            ));
            app.update();
            assert_eq!(
                app.world().get::<Visibility>(controller),
                Some(&Visibility::Hidden)
            );
        }
        app.world_mut()
            .write_message(PointerHits::new(pointer, Vec::new(), 0.0));
        app.update();
        for _ in 0..3 {
            app.world_mut().write_message(PointerHits::new(
                pointer,
                vec![(target, hit.clone())],
                0.0,
            ));
            app.update();
        }
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn tooltip_declaration_replacement_preserves_press_suppression() {
        let mut app = materialization_app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            100,
        )));
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let mut help = Tooltip::new(El::new()).show_after(Duration::from_millis(200));
        help.text("Arrange jacks");
        let panel = spawn_panel(&mut app, tooltip_tree(Some(help)));
        update_materialization(&mut app);
        let target = widget(&mut app, panel);
        app.world_mut()
            .entity_mut(target)
            .insert(TooltipPointerCamera { camera });
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        update_materialization(&mut app);
        let previous = controller(&app, target);
        assert!(
            app.world()
                .get::<TooltipPhase>(previous)
                .is_some_and(TooltipPhase::is_visible),
            "phase={:?}, readiness={:?}, placement={:?}",
            app.world().get::<TooltipPhase>(previous),
            app.world().get::<TooltipReadiness>(previous),
            app.world().get::<TooltipPlacementState>(previous),
        );
        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::None {
                        width:  800,
                        height: 600,
                    },
                    position: Vec2::splat(50.0),
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
        let mut replacement = Tooltip::new(El::new()).show_after(Duration::from_millis(200));
        replacement.text("Put tools back");
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, tooltip_tree(Some(replacement)))
                .is_ok()
        );
        update_materialization(&mut app);
        assert_eq!(
            widget(&mut app, panel),
            target,
            "the pointer's widget is retained"
        );
        let replaced = controller(&app, target);
        assert_ne!(
            previous, replaced,
            "changing the declaration replaces its controller"
        );
        assert!(app.world().get_entity(previous).is_err());
        for _ in 0..10 {
            app.update();
            assert!(
                matches!(
                    app.world().get::<TooltipPhase>(replaced),
                    Some(TooltipPhase::HiddenUntilLeave)
                ),
                "replacement help stays hidden past show_after until the pointer leaves"
            );
            assert!(
                app.world()
                    .get::<Visibility>(replaced)
                    .is_none_or(|visibility| *visibility == Visibility::Hidden)
            );
        }
        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<Visibility>(replaced),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn tooltip_press_hides_until_the_pointer_leaves() {
        let mut app = visibility_app(Duration::from_millis(20));
        let tooltip = Tooltip::new(El::new())
            .show_after(Duration::ZERO)
            .hide_after(Duration::from_secs(10));
        let (target, controller) = visibility_controller(&mut app, tooltip);
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );

        for interaction in [
            PickingInteraction::Pressed,
            PickingInteraction::Pressed,
            PickingInteraction::Hovered,
            PickingInteraction::Hovered,
        ] {
            set_interaction(&mut app, target, interaction);
            app.update();
            assert_eq!(
                app.world().get::<Visibility>(controller),
                Some(&Visibility::Hidden),
                "press, hold and release over the control must stay hidden: {interaction:?}"
            );
        }
        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().shown,
            [controller, controller]
        );
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().hidden,
            [controller]
        );
    }

    #[test]
    fn show_wait_cancels_and_hide_grace_can_be_canceled() {
        let step = Duration::from_millis(100);
        let mut app = visibility_app(step);
        let tooltip = Tooltip::new(El::new())
            .show_after(Duration::from_millis(300))
            .hide_after(Duration::from_millis(200));
        let (target, controller) = visibility_controller(&mut app, tooltip);

        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::WaitingToShow(_))
        ));
        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Hidden)
        ));
        assert_eq!(app.world().resource::<TooltipVisibilityLog>().shown, []);

        set_interaction(&mut app, target, PickingInteraction::Hovered);
        for _ in 0..3 {
            app.update();
        }
        assert!(
            matches!(
                app.world().get::<TooltipPhase>(controller),
                Some(TooltipPhase::Visible)
            ),
            "phase={:?}, readiness={:?}, placement={:?}",
            app.world().get::<TooltipPhase>(controller),
            app.world().get::<TooltipReadiness>(controller),
            app.world()
                .get::<crate::widgets::tooltip::placement::TooltipPlacementState>(controller),
        );
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().shown,
            [controller]
        );

        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::WaitingToHide(_))
        ));
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Visible)
        ));
        assert_eq!(app.world().resource::<TooltipVisibilityLog>().hidden, []);

        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Hidden)
        ));
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().hidden,
            [controller]
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn zero_show_waits_for_readiness_and_suppress_hides_immediately() {
        let mut app = visibility_app(Duration::from_millis(16));
        let tooltip = Tooltip::new(El::new())
            .show_after(Duration::ZERO)
            .hide_after(Duration::from_secs(10));
        let (target, controller) = visibility_controller(&mut app, tooltip);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipReadiness::Pending);

        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::WaitingToShow(timer)) if timer.is_finished()
        ));
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        assert_eq!(app.world().resource::<TooltipVisibilityLog>().shown, []);

        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipReadiness::Ready);
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Visible)
        ));
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::widgets::WidgetDisabled::test_marker());
        app.update();
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Hidden)
        ));
        let log = app.world().resource::<TooltipVisibilityLog>();
        assert_eq!(log.shown, [controller]);
        assert_eq!(log.hidden, [controller]);
    }

    #[test]
    fn hiding_and_showing_reuses_the_materialized_controller() {
        let mut app = visibility_app(Duration::ZERO);
        let tooltip = Tooltip::new(El::new())
            .show_after(Duration::ZERO)
            .hide_after(Duration::ZERO);
        let (target, controller) = visibility_controller(&mut app, tooltip);
        let panel = DiegeticPanel::world()
            .size(Mm(1.0), Mm(1.0))
            .with_tree(LayoutBuilder::new(1.0, 1.0).build())
            .build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else {
            return;
        };
        app.world_mut().entity_mut(controller).insert((
            panel,
            MaterializedTooltip {
                target,
                presentation: TooltipPresentationSpace::World,
                layout_unit: Unit::Millimeters,
                render_layers: RenderLayers::layer(0),
                authored_attachment: PanelAttachment::new(Anchor::Center, Anchor::Center),
                previous_transform: PreMaterializationComponent::Absent,
                previous_global: PreMaterializationComponent::Absent,
                previous_visibility: PreMaterializationComponent::Absent,
            },
        ));
        let panel_revision = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::tree_revision);

        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();
        set_interaction(&mut app, target, PickingInteraction::None);
        app.update();
        set_interaction(&mut app, target, PickingInteraction::Hovered);
        app.update();

        assert!(app.world().get_entity(controller).is_ok());
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(DiegeticPanel::tree_revision),
            panel_revision
        );
        assert!(app.world().get::<MaterializedTooltip>(controller).is_some());
        let log = app.world().resource::<TooltipVisibilityLog>();
        assert_eq!(log.shown, [controller, controller]);
        assert_eq!(log.hidden, [controller]);
    }

    #[test]
    fn visible_lifecycle_finalizer_emits_once_while_data_is_queryable() {
        for operation in 0..2 {
            let mut app = test_app();
            app.init_resource::<TooltipVisibilityLog>()
                .add_observer(record_tooltip_hidden);
            let target = app.world_mut().spawn_empty().id();
            let controller = app
                .world_mut()
                .spawn((
                    Tooltip::new(El::new()),
                    TooltipFor::new(target),
                    TooltipPhase::Visible,
                    Visibility::Inherited,
                    Transform::default(),
                    GlobalTransform::default(),
                ))
                .id();

            match operation {
                0 => {
                    app.world_mut().despawn(controller);
                },
                _ => {
                    app.world_mut().despawn(target);
                },
            }

            let log = app.world().resource::<TooltipVisibilityLog>();
            assert_eq!(log.hidden, [controller]);
        }
    }

    #[test]
    fn visible_associated_declaration_removal_emits_before_controller_cleanup() {
        let mut app = test_app();
        app.init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_hidden);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        app.update();
        let widget = widget(&mut app, panel);
        let controller = controller(&app, widget);
        app.world_mut().entity_mut(controller).insert((
            TooltipPhase::Visible,
            Visibility::Inherited,
            Transform::default(),
            GlobalTransform::default(),
        ));
        app.world_mut()
            .entity_mut(widget)
            .insert(TooltipPointerCamera { camera: panel });

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, tooltip_tree(None));
        assert!(result.is_ok());
        app.update();

        let log = app.world().resource::<TooltipVisibilityLog>();
        assert_eq!(log.hidden, [controller]);
        assert!(app.world().get_entity(controller).is_err());
        assert!(app.world().get::<TooltipPointerCamera>(widget).is_none());
    }

    #[test]
    fn target_panel_role_removal_finalizes_a_visible_standalone_tooltip() {
        let mut app = test_app();
        app.init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_hidden);
        let target = spawn_panel(&mut app, LayoutBuilder::new(10.0, 10.0).build());
        let controller = app
            .world_mut()
            .spawn((
                Tooltip::new(El::new()),
                TooltipFor::new(target),
                TooltipPhase::Visible,
                Visibility::Inherited,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();

        app.world_mut().entity_mut(target).remove::<DiegeticPanel>();
        app.world_mut().flush();

        let log = app.world().resource::<TooltipVisibilityLog>();
        assert_eq!(log.hidden, [controller]);
    }

    #[test]
    fn synthetic_backend_hit_and_raw_pointer_input_start_tooltip_eligibility() {
        let pointer = PointerId::Touch(91);
        let mut app = materialization_app();
        app.add_plugins(InteractionPlugin)
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .init_resource::<PointerMap>()
            .init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_shown)
            .add_observer(record_tooltip_hidden);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let Some(normalized_target) =
            RenderTarget::Window(WindowRef::Entity(window)).normalize(Some(window))
        else {
            return;
        };
        let location = Location {
            target:   normalized_target,
            position: TEST_VIEWPORT * 0.5,
        };
        add_synthetic_pointer(&mut app, pointer, location.clone());

        let tooltip = fixed_size_tooltip(Vec2::new(30.0, 10.0)).show_after(Duration::ZERO);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(tooltip)));
        app.update();
        let widget = widget(&mut app, panel);
        let controller = controller(&app, widget);

        app.world_mut().write_message(PointerHits::new(
            pointer,
            vec![(widget, HitData::new(camera, 0.0, None, None))],
            0.0,
        ));
        app.world_mut().write_message(PointerInput::new(
            pointer,
            location.clone(),
            PointerAction::Move { delta: Vec2::ZERO },
        ));
        app.update();

        assert_eq!(
            app.world().get::<PickingInteraction>(widget),
            Some(&PickingInteraction::Hovered)
        );
        assert_eq!(
            app.world()
                .get::<TooltipPointerCamera>(widget)
                .map(|remembered| remembered.camera),
            Some(camera)
        );
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible)
        ));
        assert!(
            app.world().get::<PrepareTooltip>(controller).is_some()
                || app.world().get::<MaterializedTooltip>(controller).is_some()
        );

        // Backends emit current hits and pointer state every frame. Keep this
        // synthetic backend active until the PostUpdate readiness pass can be
        // observed by the following reveal pass.
        for _ in 0..=MATERIALIZATION_UPDATES {
            app.world_mut().write_message(PointerHits::new(
                pointer,
                vec![(widget, HitData::new(camera, 0.0, None, None))],
                0.0,
            ));
            app.world_mut().write_message(PointerInput::new(
                pointer,
                location.clone(),
                PointerAction::Move { delta: Vec2::ZERO },
            ));
            app.update();
        }
        assert!(
            matches!(
                app.world().get::<TooltipPhase>(controller),
                Some(TooltipPhase::Visible)
            ),
            "phase={:?}, readiness={:?}, placement={:?}",
            app.world().get::<TooltipPhase>(controller),
            app.world().get::<TooltipReadiness>(controller),
            app.world()
                .get::<crate::widgets::tooltip::placement::TooltipPlacementState>(controller),
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
        let log = app.world().resource::<TooltipVisibilityLog>();
        assert_eq!(log.shown, [controller]);
    }

    #[test]
    fn standalone_tooltip_uses_the_same_visibility_path() {
        let mut app = materialization_app();
        app.init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_shown)
            .add_observer(record_tooltip_hidden);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = mesh_target(&mut app, Vec3A::splat(0.5));
        let controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(0.03, 0.01))
                .show_after(Duration::ZERO)
                .hide_after(Duration::ZERO),
        );
        app.update();

        app.world_mut()
            .entity_mut(target)
            .insert((PickingInteraction::Hovered, TooltipPointerCamera { camera }));
        update_materialization(&mut app);

        assert!(
            matches!(
                app.world().get::<TooltipPhase>(controller),
                Some(TooltipPhase::Visible)
            ),
            "phase={:?}, readiness={:?}, placement={:?}",
            app.world().get::<TooltipPhase>(controller),
            app.world().get::<TooltipReadiness>(controller),
            app.world()
                .get::<crate::widgets::tooltip::placement::TooltipPlacementState>(controller),
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().shown,
            [controller]
        );

        app.world_mut()
            .entity_mut(target)
            .insert(PickingInteraction::None);
        app.update();

        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Hidden)
        ));
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().hidden,
            [controller]
        );
    }

    #[test]
    fn keyboard_focus_selects_and_then_reuses_the_interaction_camera() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let lower = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let higher = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let Some(mut lower_camera) = app.world_mut().get_mut::<Camera>(lower) else {
            return;
        };
        lower_camera.order = 1;
        let Some(mut higher_camera) = app.world_mut().get_mut::<Camera>(higher) else {
            return;
        };
        higher_camera.order = 5;
        let tooltip = Tooltip::new(El::new()).show_after(Duration::ZERO);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(tooltip)));
        app.update();
        let widget = widget(&mut app, panel);
        let controller = controller(&app, widget);

        app.world_mut()
            .trigger(crate::RequestWidgetFocus { window, widget });
        app.update();
        assert_eq!(
            app.world()
                .get::<TooltipPresentationCamera>(controller)
                .map(|camera| camera.camera),
            Some(higher),
            "initial keyboard-only focus chooses the highest-order compatible camera",
        );

        let Some(normalized_target) =
            RenderTarget::Window(WindowRef::Entity(window)).normalize(Some(window))
        else {
            return;
        };
        let location = Location {
            target:   normalized_target,
            position: TEST_VIEWPORT * 0.5,
        };
        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(PointerId::Mouse, location.clone()),
            button:  PointerButton::Primary,
            hit:     HitData::new(lower, 0.0, None, None),
            count:   1,
        });
        app.world_mut()
            .trigger(crate::RequestWidgetFocus { window, widget });
        app.update();
        assert_eq!(
            app.world()
                .get::<TooltipPresentationCamera>(controller)
                .map(|camera| camera.camera),
            Some(lower),
            "visible keyboard focus reuses the camera from the preceding interaction",
        );

        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(PointerId::Mouse, location),
            button:  PointerButton::Primary,
            hit:     HitData::new(higher, 0.0, None, None),
            count:   1,
        });
        app.world_mut()
            .trigger(crate::RequestWidgetFocus { window, widget });
        app.update();
        assert_eq!(
            app.world()
                .get::<TooltipPresentationCamera>(controller)
                .map(|camera| camera.camera),
            Some(higher),
            "a later interaction replaces the remembered camera",
        );
    }

    #[test]
    fn visible_associated_replacement_starts_a_fresh_show_wait() {
        let mut app = test_app();
        app.init_resource::<TooltipVisibilityLog>()
            .add_observer(record_tooltip_hidden);
        let panel = spawn_panel(
            &mut app,
            tooltip_tree(Some(Tooltip::new(El::new()).show_after(Duration::ZERO))),
        );
        app.update();
        let widget = widget(&mut app, panel);
        let original = controller(&app, widget);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.world_mut().entity_mut(original).insert((
            TooltipPhase::Visible,
            Visibility::Inherited,
            Transform::default(),
            GlobalTransform::default(),
        ));

        let replacement_delay = Duration::from_secs(30);
        let replacement = Tooltip::new(El::new()).show_after(replacement_delay);
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, tooltip_tree(Some(replacement.clone())));
        assert!(result.is_ok());
        app.update();

        let replacement_controller = controller(&app, widget);
        assert_ne!(replacement_controller, original);
        assert!(app.world().get_entity(original).is_err());
        assert_eq!(
            app.world().resource::<TooltipVisibilityLog>().hidden,
            [original]
        );
        assert_eq!(
            app.world().get::<Tooltip>(replacement_controller),
            Some(&replacement)
        );
        assert!(matches!(
            app.world().get::<TooltipPhase>(replacement_controller),
            Some(TooltipPhase::WaitingToShow(timer))
                if timer.duration() == replacement_delay && !timer.is_finished()
        ));
        assert!(
            app.world()
                .get::<MaterializedTooltip>(replacement_controller)
                .is_none()
        );
    }

    #[test]
    fn hidden_materialized_panel_role_cleanup_detaches_once() {
        let mut app = materialization_app();
        let target = mesh_target(&mut app, Vec3A::ONE);
        let tooltip = fixed_size_tooltip(Vec2::new(20.0, 10.0));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(target)
                .map(AnchoredHere::len),
            Some(1)
        );

        app.world_mut()
            .entity_mut(controller)
            .remove::<DiegeticPanel>();
        update_materialization(&mut app);

        assert!(app.world().get_entity(controller).is_ok());
        assert!(app.world().get::<Tooltip>(controller).is_some());
        assert!(app.world().get::<MaterializedTooltip>(controller).is_none());
        assert!(app.world().get::<TooltipReadiness>(controller).is_none());
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(app.world().get::<Transform>(controller).is_none());
        assert!(app.world().get::<GlobalTransform>(controller).is_none());
        assert!(app.world().get::<Visibility>(controller).is_none());
        assert!(
            app.world()
                .get::<AnchoredHere>(target)
                .is_none_or(AnchoredHere::is_empty)
        );
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
    }

    #[test]
    fn materialized_panel_role_cleanup_restores_controller_requirements() {
        let mut app = materialization_app();
        let target = mesh_target(&mut app, Vec3A::ONE);
        let tooltip = fixed_size_tooltip(Vec2::new(20.0, 10.0));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();

        let previous_transform =
            Transform::from_xyz(4.0, -2.0, 1.5).with_rotation(Quat::from_rotation_y(0.25));
        let previous_global = GlobalTransform::from(previous_transform);
        let previous_visibility = Visibility::Inherited;
        app.world_mut().entity_mut(controller).insert((
            previous_transform,
            previous_global,
            previous_visibility,
        ));
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .entity_mut(controller)
            .remove::<DiegeticPanel>();
        update_materialization(&mut app);

        assert_eq!(
            app.world().get::<Transform>(controller),
            Some(&previous_transform)
        );
        assert_eq!(
            app.world().get::<GlobalTransform>(controller),
            Some(&previous_global)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&previous_visibility)
        );
    }
}
