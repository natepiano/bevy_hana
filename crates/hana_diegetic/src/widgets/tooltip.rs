use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use bevy::asset::Handle;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::color::Color;
use bevy::ecs::change_detection::MaybeLocation;
use bevy::ecs::event::EntityTrigger;
use bevy::ecs::event::EventKey;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::system::SystemParam;
use bevy::ecs::world::DeferredWorld;
use bevy::image::Image;
use bevy::mesh::Mesh3d;
use bevy::picking::events::Move as PointerMove;
use bevy::picking::events::Over;
use bevy::picking::events::Press;
use bevy::picking::hover::PickingInteraction;
use bevy::platform::collections::HashSet;
use bevy::prelude::Added;
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::ContainsEntity;
use bevy::prelude::Despawn;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::EntityCommands;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Has;
use bevy::prelude::In;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mut;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Plugin;
use bevy::prelude::Pointer;
use bevy::prelude::PostUpdate;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Rect;
use bevy::prelude::Ref;
use bevy::prelude::Remove;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SystemSet;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_valence::AnchorFrame;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSite;
use hana_valence::AnchorSystems;
use hana_valence::AnchoredHere;
use hana_valence::AnchoredTo;
use hana_valence::Edge;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;

use super::TooltipFor;
use super::TooltipSystems;
use super::Tooltips;
use super::WidgetSystems;
use crate::PanelSystems;
use crate::layout::Anchor;
use crate::layout::ChildLayoutState;
use crate::layout::Dimension;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutOnly;
use crate::layout::LayoutTree;
use crate::layout::Px;
use crate::layout::Sizing;
use crate::layout::Text;
use crate::layout::Unit;
use crate::panel;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPanelCommands;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelEntity;
use crate::panel::PanelEntityReader;
use crate::panel::PanelLayout;
use crate::panel::PanelPlane;
use crate::panel::PanelScreenBounds;
use crate::panel::PanelSpace;
use crate::panel::ResolvedScreenPanelPosition;
use crate::panel::Screen;
use crate::panel::WidgetEntity;
use crate::panel::World;
use crate::render::MaterialTableAppendReady;
use crate::render::PanelChildSystems;
use crate::screen_space;
use crate::screen_space::ScreenAnchorTarget;
use crate::screen_space::ScreenAttachmentResolveDiagnostics;
use crate::screen_space::ScreenPanelRect;
use crate::screen_space::ScreenSpaceSystems;

const DEFAULT_SHOW_DELAY: Duration = Duration::from_millis(500);
const DEFAULT_TOOLTIP_GAP: f32 = 8.0;
const QUAD_ANCHOR_COUNT: usize = 9;
const QUAD_EDGE_COUNT: usize = 4;
const TOOLTIP_VIEWPORT_MARGIN: f32 = 8.0;

pub(super) struct TooltipPlugin;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
enum TooltipPlacementSystems {
    ScreenDecide,
    ScreenCommandsApplied,
    ScreenCorrection,
    WorldDecide,
    WorldCommandsApplied,
}

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        let hidden_event = app.world_mut().register_event_key::<TooltipHidden>();
        app.insert_resource(TooltipHiddenEventKey(hidden_event))
            .add_observer(remove_mesh_anchor_geometry)
            .add_observer(remember_tooltip_camera_from_over)
            .add_observer(remember_tooltip_camera_from_move)
            .add_observer(remember_tooltip_camera_from_press)
            .add_observer(remove_tooltip_pointer_camera)
            .add_observer(finalize_removed_tooltip)
            .add_observer(finalize_despawned_tooltip)
            .add_observer(finalize_tooltip_panel_role)
            .configure_sets(
                Update,
                (
                    TooltipSystems::Eligibility
                        .after(WidgetSystems::FocusCommandsApplied)
                        .after(TooltipSystems::ControllerCommandsApplied),
                    TooltipSystems::EligibilityCommandsApplied.after(TooltipSystems::Eligibility),
                    TooltipSystems::Materialize.after(TooltipSystems::EligibilityCommandsApplied),
                    TooltipSystems::MaterializationCommandsApplied
                        .after(TooltipSystems::Materialize),
                    TooltipSystems::Attach.after(TooltipSystems::MaterializationCommandsApplied),
                    TooltipSystems::AttachmentCommandsApplied
                        .after(TooltipSystems::Attach)
                        .before(ScreenSpaceSystems::WidgetDemandCommandsApplied)
                        .before(PanelSystems::ResolvePanelAttachments),
                    TooltipPlacementSystems::ScreenDecide
                        .after(PanelSystems::ResolvePanelAttachments),
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
                    advance_tooltip_visibility.in_set(TooltipSystems::Eligibility),
                    ApplyDeferred.in_set(TooltipSystems::EligibilityCommandsApplied),
                    apply_tooltip_width_constraints.before(PanelSystems::ComputeLayout),
                    materialize_requested_tooltips.in_set(TooltipSystems::Materialize),
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
                        .run_if(tooltip_readiness_inputs_changed),
                    ApplyDeferred.in_set(TooltipPlacementSystems::ScreenCommandsApplied),
                    (
                        screen_space::resolve_screen_space_panel_attachments,
                        clear_screen_tooltip_attachment_corrections,
                    )
                        .chain()
                        .in_set(TooltipPlacementSystems::ScreenCorrection)
                        .run_if(screen_tooltip_attachment_correction_requested),
                ),
            )
            .configure_sets(
                PostUpdate,
                (
                    TooltipSystems::Reveal
                        .before(PanelChildSystems::Build)
                        .before(MaterialTableAppendReady),
                    TooltipPlacementSystems::WorldDecide.after(panel::refresh_world_anchor_globals),
                    TooltipPlacementSystems::WorldCommandsApplied
                        .after(TooltipPlacementSystems::WorldDecide)
                        .before(panel::write_panel_anchor_offsets)
                        .before(AnchorSystems::Resolve),
                    TooltipSystems::Readiness.after(TransformSystems::Propagate),
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
                        .run_if(tooltip_readiness_inputs_changed),
                    ApplyDeferred.in_set(TooltipPlacementSystems::WorldCommandsApplied),
                    finalize_tooltip_readiness
                        .in_set(TooltipSystems::Readiness)
                        .run_if(tooltip_readiness_inputs_changed),
                    emit_tooltip_shown.in_set(TooltipSystems::VisibilityEvents),
                ),
            );
    }
}

/// Behavior when the described widget is disabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TooltipDisabledPolicy {
    /// Disabled targets may show their tooltip.
    Show,
    /// Disabled targets suppress their tooltip.
    Suppress,
}

/// Placement behavior when a tooltip would extend beyond its presentation area.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TooltipPlacementPolicy {
    /// Adjust placement to keep the tooltip visible.
    KeepVisible,
    /// Preserve the authored anchors and offset.
    Fixed,
}

/// Deferred visual and behavior declaration for a tooltip.
///
/// Replace a standalone tooltip by despawning its controller and calling
/// [`TooltipCommandsExt::spawn_tooltip`] again. Replacing this component on a
/// materialized controller is not an in-place content update.
#[derive(Component, Debug)]
#[require(TooltipPhase)]
pub struct Tooltip {
    blueprint:        Arc<LayoutTree>,
    authoring:        TooltipAuthoring,
    show_after:       Duration,
    hide_after:       Duration,
    disabled_policy:  TooltipDisabledPolicy,
    source_anchor:    Anchor,
    target_anchor:    Anchor,
    offset:           PanelAnchorOffset,
    placement_policy: TooltipPlacementPolicy,
}

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

/// Sole authority for one tooltip controller's visibility lifecycle.
///
/// A wait owns the timer duration captured when that transition began. The
/// panel remains materialized while hidden; only `Visible` and
/// `WaitingToHide` represent an on-screen tooltip.
#[derive(Component, Default)]
enum TooltipPhase {
    #[default]
    Hidden,
    WaitingToShow(Timer),
    Visible,
    WaitingToHide(Timer),
}

impl TooltipPhase {
    const fn is_visible(&self) -> bool { matches!(self, Self::Visible | Self::WaitingToHide(_)) }
}

#[derive(Clone, Copy, Component)]
struct TooltipPointerCamera {
    camera: Entity,
}

#[derive(Clone, Copy, Component)]
struct TooltipShownPending;

#[derive(Resource)]
struct TooltipHiddenEventKey(EventKey);

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

fn remember_tooltip_camera_from_over(
    event: On<Pointer<Over>>,
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

fn remember_tooltip_camera_from_move(
    event: On<Pointer<PointerMove>>,
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

fn remember_tooltip_camera_from_press(
    event: On<Pointer<Press>>,
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

fn remove_tooltip_pointer_camera(event: On<Remove, Tooltips>, mut commands: Commands) {
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

fn finalize_removed_tooltip(trigger: On<Remove, Tooltip>, mut world: DeferredWorld<'_>) {
    finalize_visible_tooltip_now(trigger.entity, &mut world);
}

fn finalize_despawned_tooltip(trigger: On<Despawn, Tooltip>, mut world: DeferredWorld<'_>) {
    finalize_visible_tooltip_now(trigger.entity, &mut world);
}

fn finalize_tooltip_panel_role(trigger: On<Remove, DiegeticPanel>, mut world: DeferredWorld<'_>) {
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

#[derive(Debug)]
struct TooltipAuthoring {
    parent_stack: Vec<usize>,
    next_auto_id: u32,
}

impl Clone for Tooltip {
    fn clone(&self) -> Self {
        Self {
            blueprint:        Arc::clone(&self.blueprint),
            // `parent_stack` exists only for the active `Tooltip::with` call;
            // an independent blueprint clone resumes authoring at the tree root.
            authoring:        TooltipAuthoring {
                parent_stack: vec![0],
                next_auto_id: self.authoring.next_auto_id,
            },
            show_after:       self.show_after,
            hide_after:       self.hide_after,
            disabled_policy:  self.disabled_policy,
            source_anchor:    self.source_anchor,
            target_anchor:    self.target_anchor,
            offset:           self.offset,
            placement_policy: self.placement_policy,
        }
    }
}

impl PartialEq for Tooltip {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.blueprint, &other.blueprint)
            && self.show_after == other.show_after
            && self.hide_after == other.hide_after
            && self.disabled_policy == other.disabled_policy
            && self.source_anchor == other.source_anchor
            && self.target_anchor == other.target_anchor
            && self.offset == other.offset
            && self.placement_policy == other.placement_policy
    }
}

impl Tooltip {
    /// Creates a deferred `Fit` by `Fit` tooltip with `root` as its visible root.
    ///
    /// ```compile_fail
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let widget = El::new().button("nested");
    /// let _ = Tooltip::new(widget);
    /// ```
    pub fn new<L>(root: El<L, LayoutOnly>) -> Self
    where
        L: ChildLayoutState,
    {
        Self {
            blueprint:        Arc::new(LayoutBuilder::with_root(root).build()),
            authoring:        TooltipAuthoring {
                parent_stack: vec![0],
                next_auto_id: 0,
            },
            show_after:       DEFAULT_SHOW_DELAY,
            hide_after:       Duration::ZERO,
            disabled_policy:  TooltipDisabledPolicy::Suppress,
            source_anchor:    Anchor::TopCenter,
            target_anchor:    Anchor::BottomCenter,
            offset:           PanelAnchorOffset::new(Px(0.0), Px(DEFAULT_TOOLTIP_GAP)),
            placement_policy: TooltipPlacementPolicy::KeepVisible,
        }
    }

    /// Adds a visual container and authors its descendants through this tooltip.
    ///
    /// ```compile_fail
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let mut tooltip = Tooltip::new(El::new());
    /// tooltip.with(El::new().button("nested"), |_| {});
    /// ```
    pub fn with<L>(
        &mut self,
        element: El<L, LayoutOnly>,
        children: impl FnOnce(&mut Self),
    ) -> &mut Self
    where
        L: ChildLayoutState,
    {
        let parent = self.current_parent();
        let child = Arc::make_mut(&mut self.blueprint).tooltip_add_container(parent, element);
        self.authoring.parent_stack.push(child);
        children(self);
        self.authoring.parent_stack.pop();
        self
    }

    /// Adds a text leaf under the current tooltip container.
    pub fn text(&mut self, text: impl Into<Text<LayoutOnly>>) -> &mut Self {
        let parent = self.current_parent();
        Arc::make_mut(&mut self.blueprint).tooltip_add_text(
            parent,
            text,
            &mut self.authoring.next_auto_id,
        );
        self
    }

    /// Adds an image leaf under the current tooltip container.
    ///
    /// ```compile_fail
    /// use bevy::asset::Handle;
    /// use bevy::color::Color;
    /// use bevy::image::Image;
    /// use hana_diegetic::{Button, El, Tooltip};
    ///
    /// let mut tooltip = Tooltip::new(El::new());
    /// tooltip.image(
    ///     El::new().button("nested"),
    ///     Handle::<Image>::default(),
    ///     Color::WHITE,
    /// );
    /// ```
    pub fn image<L>(
        &mut self,
        element: El<L, LayoutOnly>,
        handle: Handle<Image>,
        tint: Color,
    ) -> &mut Self
    where
        L: ChildLayoutState,
    {
        let parent = self.current_parent();
        Arc::make_mut(&mut self.blueprint).tooltip_add_image(parent, element, handle, tint);
        self
    }

    /// Sets the delay before an eligible tooltip is shown.
    #[must_use]
    pub const fn show_after(mut self, delay: Duration) -> Self {
        self.show_after = delay;
        self
    }

    /// Sets the delay before an ineligible tooltip is hidden.
    #[must_use]
    pub const fn hide_after(mut self, delay: Duration) -> Self {
        self.hide_after = delay;
        self
    }

    /// Sets disabled-target behavior.
    #[must_use]
    pub const fn disabled_policy(mut self, policy: TooltipDisabledPolicy) -> Self {
        self.disabled_policy = policy;
        self
    }

    /// Sets the tooltip anchor used for placement.
    #[must_use]
    pub const fn source_anchor(mut self, anchor: Anchor) -> Self {
        self.source_anchor = anchor;
        self
    }

    /// Sets the described target anchor used for placement.
    #[must_use]
    pub const fn target_anchor(mut self, anchor: Anchor) -> Self {
        self.target_anchor = anchor;
        self
    }

    /// Sets the offset from the described target.
    #[must_use]
    pub const fn offset(mut self, offset: PanelAnchorOffset) -> Self {
        self.offset = offset;
        self
    }

    /// Sets the placement policy.
    #[must_use]
    pub const fn placement_policy(mut self, policy: TooltipPlacementPolicy) -> Self {
        self.placement_policy = policy;
        self
    }

    #[cfg(test)]
    fn tree(&self) -> &LayoutTree { &self.blueprint }

    const fn blueprint(&self) -> &Arc<LayoutTree> { &self.blueprint }

    const fn attachment(&self) -> PanelAttachment {
        PanelAttachment::new(self.source_anchor, self.target_anchor).with_offset(self.offset)
    }

    fn current_parent(&self) -> usize { self.authoring.parent_stack.last().copied().unwrap_or(0) }
}

#[derive(Clone, Copy, Component)]
pub(super) struct PrepareTooltip;

#[derive(Clone, Copy, Component)]
pub(super) struct AuthoredTooltipTargetSpace(PanelSpace);

fn advance_one_tooltip_phase(
    phase: &mut TooltipPhase,
    tooltip: &Tooltip,
    eligible: bool,
    suppressed: bool,
    delta: Duration,
) -> bool {
    let mut next = None;
    let mut emit_hidden = false;
    match phase {
        TooltipPhase::Hidden if eligible => {
            let mut timer = Timer::new(tooltip.show_after, TimerMode::Once);
            timer.tick(delta);
            next = Some(TooltipPhase::WaitingToShow(timer));
        },
        TooltipPhase::Hidden => {},
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

fn advance_tooltip_visibility(
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
    )>,
    targets: Query<(
        Option<&PickingInteraction>,
        Has<super::WidgetFocusVisible>,
        Has<super::WidgetDisabled>,
        Option<&TooltipPointerCamera>,
    )>,
    panels: Query<&DiegeticPanel>,
    widgets: Query<&super::WidgetOf, With<super::PanelWidget>>,
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
    focus_authority: Res<super::WidgetFocusAuthority>,
    mut commands: Commands,
) {
    let delta = time.as_deref().map_or(Duration::ZERO, Time::delta);
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
    ) in &mut controllers
    {
        let target = tooltip_for.target();
        let Ok((interaction, focus_visible, disabled, pointer_camera)) = targets.get(target) else {
            continue;
        };
        let hovered = matches!(
            interaction,
            Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
        );
        let suppressed = disabled && tooltip.disabled_policy == TooltipDisabledPolicy::Suppress;
        let eligible = (hovered || focus_visible) && !suppressed;

        let emit_hidden =
            advance_one_tooltip_phase(&mut phase, tooltip, eligible, suppressed, delta);
        if emit_hidden {
            if let Some(visibility) = visibility.as_deref_mut() {
                *visibility = Visibility::Hidden;
            }
            commands.trigger(TooltipHidden { entity });
        }

        if !eligible {
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
        let camera = if space == PanelSpace::World
            && tooltip.placement_policy == TooltipPlacementPolicy::KeepVisible
        {
            select_tooltip_presentation_camera(
                target,
                hovered.then_some(pointer_camera).flatten(),
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
        let needs_camera = space == PanelSpace::World
            && tooltip.placement_policy == TooltipPlacementPolicy::KeepVisible;
        if needs_camera {
            let Some(camera) = camera else {
                if presentation_camera.is_some() {
                    commands
                        .entity(entity)
                        .remove::<TooltipPresentationCamera>();
                }
                continue;
            };
            if presentation_camera.is_none_or(|current| current.camera != camera) {
                commands
                    .entity(entity)
                    .insert(TooltipPresentationCamera { camera });
            }
        }
        if materialized.is_none() && !preparing {
            commands.entity(entity).insert(PrepareTooltip);
        }
    }
}

fn tooltip_target_presentation(
    target: Entity,
    authored_space: Option<&AuthoredTooltipTargetSpace>,
    materialized: Option<&MaterializedTooltip>,
    panels: &Query<&DiegeticPanel>,
    widgets: &Query<&super::WidgetOf, With<super::PanelWidget>>,
    render_layers: &Query<&RenderLayers>,
) -> Option<(PanelSpace, RenderLayers)> {
    if let Some(materialized) = materialized {
        return Some((materialized.space, materialized.render_layers.clone()));
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
    focus_authority: &super::WidgetFocusAuthority,
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

fn reveal_ready_tooltips(
    mut controllers: Query<(
        Entity,
        &mut TooltipPhase,
        &TooltipReadiness,
        &mut Visibility,
    )>,
    mut commands: Commands,
) {
    for (entity, mut phase, readiness, mut visibility) in &mut controllers {
        let TooltipPhase::WaitingToShow(timer) = &*phase else {
            continue;
        };
        if !timer.is_finished() || *readiness != TooltipReadiness::Ready {
            continue;
        }
        *phase = TooltipPhase::Visible;
        *visibility = Visibility::Inherited;
        commands.entity(entity).insert(TooltipShownPending);
    }
}

fn emit_tooltip_shown(
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

#[derive(Clone, Component)]
pub(crate) struct MaterializedTooltip {
    target:              Entity,
    space:               PanelSpace,
    layout_unit:         Unit,
    window:              Option<Entity>,
    camera_order:        Option<isize>,
    render_layers:       RenderLayers,
    blueprint:           Arc<LayoutTree>,
    authored_attachment: PanelAttachment,
    placement_policy:    TooltipPlacementPolicy,
    previous_transform:  Option<Transform>,
    previous_global:     Option<GlobalTransform>,
    previous_visibility: Option<Visibility>,
}

impl MaterializedTooltip {
    pub(crate) const fn target(&self) -> Entity { self.target }

    pub(crate) const fn space(&self) -> PanelSpace { self.space }

    pub(crate) const fn layout_unit(&self) -> Unit { self.layout_unit }
}

#[derive(Clone, Copy, Component)]
pub(super) struct TooltipPresentationCamera {
    camera: Entity,
}

impl TooltipPresentationCamera {
    #[cfg(test)]
    const fn new(camera: Entity) -> Self { Self { camera } }
}

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(super) enum TooltipReadiness {
    Pending,
    Ready,
}

#[derive(Clone, Copy, Component, Debug, PartialEq)]
pub(super) struct TooltipPlacementState {
    attachment:   PanelAttachment,
    result:       TooltipPlacementResult,
    world_target: Option<WorldTooltipTargetSnapshot>,
}

fn set_tooltip_placement(
    placement: &mut Mut<'_, TooltipPlacementState>,
    attachment: PanelAttachment,
    result: TooltipPlacementResult,
) {
    let next = TooltipPlacementState {
        attachment,
        result,
        world_target: placement.world_target,
    };
    if **placement != next {
        **placement = next;
    }
}

fn set_tooltip_placement_result(
    placement: &mut Mut<'_, TooltipPlacementState>,
    result: TooltipPlacementResult,
) {
    set_tooltip_placement(placement, placement.attachment, result);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TooltipPlacementResult {
    Pending,
    Fits,
    Unavailable,
}

#[derive(Clone, Copy)]
enum TooltipPlacementDecision {
    ConstrainWidth(f32),
    RestoreWidth,
    Move(PanelAttachment),
    Fits,
    Unavailable,
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(super) struct TooltipPlacementRunCount {
    screen: usize,
    world:  usize,
}

#[derive(Clone, Copy, Component)]
pub(super) struct TooltipWidthConstraint(f32);

#[derive(Clone, Copy, Component)]
pub(super) enum TooltipWidthConstraintRequest {
    Apply(f32),
    Restore,
}

#[derive(Component)]
struct ScreenTooltipAttachmentCorrection;

fn screen_tooltip_attachment_correction_requested(
    corrections: Query<(), With<ScreenTooltipAttachmentCorrection>>,
) -> bool {
    !corrections.is_empty()
}

fn clear_screen_tooltip_attachment_corrections(
    corrections: Query<Entity, With<ScreenTooltipAttachmentCorrection>>,
    mut commands: Commands,
) {
    for entity in &corrections {
        commands
            .entity(entity)
            .remove::<ScreenTooltipAttachmentCorrection>();
    }
}

#[derive(SystemParam)]
struct TooltipReadinessInputChanges<'w, 's> {
    controllers: Query<
        'w,
        's,
        (),
        (
            With<MaterializedTooltip>,
            Or<(
                Added<MaterializedTooltip>,
                Changed<TooltipFor>,
                Changed<DiegeticPanel>,
                Changed<ComputedDiegeticPanel>,
                Changed<TooltipPresentationCamera>,
                Changed<TooltipWidthConstraint>,
                Changed<TooltipPlacementState>,
                Changed<ResolvedScreenPanelPosition>,
                Changed<AnchoredTo>,
                Changed<PanelAttachmentAuthored>,
            )>,
        ),
    >,
    panels: Query<
        'w,
        's,
        (
            Ref<'static, DiegeticPanel>,
            Ref<'static, Transform>,
            Ref<'static, GlobalTransform>,
            Ref<'static, ResolvedScreenPanelPosition>,
        ),
        (
            Without<MaterializedTooltip>,
            Or<(
                Changed<DiegeticPanel>,
                Changed<Transform>,
                Changed<GlobalTransform>,
                Changed<ResolvedScreenPanelPosition>,
            )>,
        ),
    >,
    geometry_targets: Query<
        'w,
        's,
        (),
        (
            Without<MaterializedTooltip>,
            With<ResolvedAnchorGeometry>,
            Or<(
                Changed<ResolvedAnchorGeometry>,
                Changed<GlobalTransform>,
                Changed<super::WidgetOf>,
                Changed<super::WidgetAnchorRect>,
            )>,
        ),
    >,
    cameras: Query<
        'w,
        's,
        (),
        (
            With<Camera>,
            Or<(
                Changed<Camera>,
                Changed<GlobalTransform>,
                Changed<RenderTarget>,
                Changed<RenderLayers>,
            )>,
        ),
    >,
    windows:                     Query<'w, 's, (), Changed<Window>>,
    screen_targets:              Query<'w, 's, (), Changed<ScreenAnchorTarget>>,
    authored_attachments:        Query<'w, 's, (), Changed<PanelAttachmentAuthored>>,
    world_attachments:           Query<'w, 's, (), Changed<AnchoredTo>>,
    removed_geometry:            RemovedComponents<'w, 's, ResolvedAnchorGeometry>,
    removed_presentation_camera: RemovedComponents<'w, 's, TooltipPresentationCamera>,
    removed_camera:              RemovedComponents<'w, 's, Camera>,
    removed_render_target:       RemovedComponents<'w, 's, RenderTarget>,
    removed_render_layers:       RemovedComponents<'w, 's, RenderLayers>,
    removed_window:              RemovedComponents<'w, 's, Window>,
    removed_screen_target:       RemovedComponents<'w, 's, ScreenAnchorTarget>,
    removed_attachment:          RemovedComponents<'w, 's, PanelAttachmentAuthored>,
    removed_world_attachment:    RemovedComponents<'w, 's, AnchoredTo>,
    removed_transform:           RemovedComponents<'w, 's, Transform>,
    removed_global_transform:    RemovedComponents<'w, 's, GlobalTransform>,
}

fn tooltip_readiness_inputs_changed(mut inputs: TooltipReadinessInputChanges<'_, '_>) -> bool {
    inputs.controllers.iter().next().is_some()
        || inputs
            .panels
            .iter()
            .any(|(panel, transform, global_transform, resolved_position)| {
                panel.is_changed()
                    || match PanelSpace::from(panel.coordinate_space()) {
                        PanelSpace::World => global_transform.is_changed(),
                        PanelSpace::Screen => {
                            transform.is_changed() || resolved_position.is_changed()
                        },
                    }
            })
        || inputs.geometry_targets.iter().next().is_some()
        || inputs.cameras.iter().next().is_some()
        || inputs.windows.iter().next().is_some()
        || inputs.screen_targets.iter().next().is_some()
        || inputs.authored_attachments.iter().next().is_some()
        || inputs.world_attachments.iter().next().is_some()
        || inputs.removed_geometry.read().next().is_some()
        || inputs.removed_presentation_camera.read().next().is_some()
        || inputs.removed_camera.read().next().is_some()
        || inputs.removed_render_target.read().next().is_some()
        || inputs.removed_render_layers.read().next().is_some()
        || inputs.removed_window.read().next().is_some()
        || inputs.removed_screen_target.read().next().is_some()
        || inputs.removed_attachment.read().next().is_some()
        || inputs.removed_world_attachment.read().next().is_some()
        || inputs.removed_transform.read().next().is_some()
        || inputs.removed_global_transform.read().next().is_some()
}

/// A typed identity for a general tooltip target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooltipTargetEntity<Space> {
    entity: Entity,
    marker: PhantomData<fn() -> Space>,
}

impl<Space> TooltipTargetEntity<Space> {
    /// Returns the underlying Bevy entity for unrelated ECS work.
    #[must_use]
    pub const fn entity(&self) -> Entity { self.entity }

    const fn from_validated(entity: Entity) -> Self {
        Self {
            entity,
            marker: PhantomData,
        }
    }
}

/// Sealed coordinate-space marker accepted by [`TooltipTarget`].
pub trait TooltipTargetSpace: private::SealedSpace {
    #[doc(hidden)]
    const PANEL_SPACE: PanelSpace;
}

impl TooltipTargetSpace for World {
    const PANEL_SPACE: PanelSpace = PanelSpace::World;
}

impl TooltipTargetSpace for Screen {
    const PANEL_SPACE: PanelSpace = PanelSpace::Screen;
}

/// Typed entity handle that can be described by a tooltip.
pub trait TooltipTarget {
    /// Coordinate space supplied by the target's live placement data.
    type Space: TooltipTargetSpace;

    /// Returns the target entity.
    fn tooltip_target_entity(&self) -> Entity;

    /// Returns the captured panel entity for Hana's typed panel handle.
    #[doc(hidden)]
    fn tooltip_target_panel(&self) -> Option<Entity> { None }

    /// Returns the captured panel owner for Hana's typed widget handle.
    #[doc(hidden)]
    fn tooltip_target_widget_owner(&self) -> Option<Entity> { None }
}

impl<Space: TooltipTargetSpace> TooltipTarget for TooltipTargetEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity }
}

impl<Space: TooltipTargetSpace> TooltipTarget for PanelEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity() }

    fn tooltip_target_panel(&self) -> Option<Entity> { Some(self.entity()) }
}

impl<Space: TooltipTargetSpace> TooltipTarget for WidgetEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity() }

    fn tooltip_target_widget_owner(&self) -> Option<Entity> { Some(self.owner()) }
}

/// Extension methods for checked standalone tooltip authoring.
pub trait TooltipCommandsExt {
    /// Reserves and returns a tooltip controller entity.
    fn spawn_tooltip<Target>(&mut self, target: Target, tooltip: Tooltip) -> Entity
    where
        Target: TooltipTarget;
}

/// Axis-aligned face selected as a rectangular anchor provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshFace {
    /// Face with a positive local X normal.
    PositiveX,
    /// Face with a negative local X normal.
    NegativeX,
    /// Face with a positive local Y normal.
    PositiveY,
    /// Face with a negative local Y normal.
    NegativeY,
    /// Face with a positive local Z normal.
    PositiveZ,
    /// Face with a negative local Z normal.
    NegativeZ,
}

/// Extension methods for checked general mesh anchor authoring.
pub trait MeshAnchorCommandsExt {
    /// Authors a persistent mesh-face anchor target.
    fn mesh_anchor_target(&mut self, entity: Entity, face: MeshFace) -> TooltipTargetEntity<World>;
}

/// Extension methods for checked general screen anchor authoring.
pub trait ScreenAnchorCommandsExt {
    /// Publishes screen placement data and returns a typed target handle.
    fn screen_anchor_target(
        &mut self,
        entity: Entity,
        data: ScreenAnchorTarget,
    ) -> TooltipTargetEntity<Screen>;
}

impl ScreenAnchorCommandsExt for Commands<'_, '_> {
    fn screen_anchor_target(
        &mut self,
        entity: Entity,
        data: ScreenAnchorTarget,
    ) -> TooltipTargetEntity<Screen> {
        self.queue(move |world: &mut bevy::ecs::world::World| {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                entity_mut.insert(data);
            }
        });
        TooltipTargetEntity::from_validated(entity)
    }
}

impl MeshAnchorCommandsExt for Commands<'_, '_> {
    fn mesh_anchor_target(&mut self, entity: Entity, face: MeshFace) -> TooltipTargetEntity<World> {
        self.run_system_cached_with(author_mesh_anchor_target, (entity, face));
        TooltipTargetEntity::from_validated(entity)
    }
}

impl TooltipCommandsExt for Commands<'_, '_> {
    fn spawn_tooltip<Target>(&mut self, target: Target, tooltip: Tooltip) -> Entity
    where
        Target: TooltipTarget,
    {
        let controller = self.spawn_empty().id();
        let operation = StandaloneTooltipOperation {
            controller,
            target: target.tooltip_target_entity(),
            space: Target::Space::PANEL_SPACE,
            provenance: target.tooltip_target_panel().map_or_else(
                || {
                    target
                        .tooltip_target_widget_owner()
                        .map_or(StandaloneTargetProvenance::General, |owner| {
                            StandaloneTargetProvenance::Widget { owner }
                        })
                },
                |_| StandaloneTargetProvenance::Panel,
            ),
            tooltip,
        };
        self.run_system_cached_with(apply_standalone_tooltip, operation);
        controller
    }
}

#[derive(Clone, Copy)]
enum StandaloneTargetProvenance {
    General,
    Panel,
    Widget { owner: Entity },
}

#[derive(Clone)]
struct StandaloneTooltipOperation {
    controller: Entity,
    target:     Entity,
    space:      PanelSpace,
    provenance: StandaloneTargetProvenance,
    tooltip:    Tooltip,
}

fn apply_standalone_tooltip(
    In(operation): In<StandaloneTooltipOperation>,
    targets: Query<(
        Option<&DiegeticPanel>,
        Option<&super::PanelWidget>,
        Option<&super::WidgetOf>,
        Option<&ResolvedAnchorGeometry>,
        Option<&super::MeshAnchorTarget>,
        Option<&ScreenAnchorTarget>,
    )>,
    mut commands: Commands,
) {
    let Ok((panel, widget, widget_of, geometry, mesh_target, screen_target)) =
        targets.get(operation.target)
    else {
        commands.entity(operation.controller).despawn();
        return;
    };
    let owner = match operation.provenance {
        StandaloneTargetProvenance::General => None,
        StandaloneTargetProvenance::Panel => panel
            .filter(|panel| PanelSpace::from(panel.coordinate_space()) == operation.space)
            .map(|_| operation.target),
        StandaloneTargetProvenance::Widget { owner } => widget
            .zip(widget_of)
            .filter(|(_, widget_of)| widget_of.panel() == owner)
            .and_then(|_| {
                targets
                    .get(owner)
                    .ok()
                    .and_then(|(panel, ..)| panel)
                    .filter(|panel| PanelSpace::from(panel.coordinate_space()) == operation.space)
                    .map(|_| owner)
            }),
    };
    let valid_target = match operation.provenance {
        StandaloneTargetProvenance::General => match operation.space {
            PanelSpace::World => geometry.is_some() || mesh_target.is_some(),
            PanelSpace::Screen => screen_target.is_some(),
        },
        StandaloneTargetProvenance::Panel | StandaloneTargetProvenance::Widget { .. } => {
            owner.is_some()
        },
    };
    if !valid_target {
        commands.entity(operation.controller).despawn();
        return;
    }

    let mut controller = commands.entity(operation.controller);
    controller.insert((
        operation.tooltip,
        TooltipFor::new(operation.target),
        AuthoredTooltipTargetSpace(operation.space),
    ));
    if let Some(owner) = owner {
        controller.insert(crate::panel::PanelOwned::from(owner));
    }
}

#[derive(Clone, Copy, Component)]
pub(crate) struct MeshAnchorTarget {
    face: MeshFace,
}

#[derive(Clone, Copy, Component)]
pub(super) struct MeshAnchorGeometry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MeshAnchorPendingCause {
    WaitingForMesh,
    WaitingForBounds,
    UnusableBounds,
}

#[derive(Clone, Copy, Component)]
pub(super) struct MeshAnchorGeometryPending {
    cause: MeshAnchorPendingCause,
}

#[derive(Default, Resource)]
pub(crate) struct MeshAnchorWarnings(HashSet<Entity>);

fn author_mesh_anchor_target(
    In((entity, face)): In<(Entity, MeshFace)>,
    meshes: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    if meshes.contains(entity) {
        commands.entity(entity).insert(MeshAnchorTarget { face });
    }
}

pub(super) fn update_mesh_anchor_geometry(
    changed_targets: Query<
        Entity,
        (
            With<MeshAnchorTarget>,
            With<AnchoredHere>,
            Or<(
                Changed<AnchoredHere>,
                Changed<MeshAnchorTarget>,
                Changed<Mesh3d>,
                Changed<Aabb>,
            )>,
        ),
    >,
    demanded_targets: Query<
        (
            Entity,
            Ref<MeshAnchorTarget>,
            Option<Ref<Mesh3d>>,
            Option<Ref<Aabb>>,
            Option<&MeshAnchorGeometry>,
            Option<&MeshAnchorGeometryPending>,
        ),
        (With<MeshAnchorTarget>, With<AnchoredHere>),
    >,
    mut removed_meshes: RemovedComponents<Mesh3d>,
    mut removed_bounds: RemovedComponents<Aabb>,
    mut warnings: ResMut<MeshAnchorWarnings>,
    mut commands: Commands,
) {
    let mut candidates = changed_targets.iter().collect::<HashSet<_>>();
    candidates.extend(removed_meshes.read());
    candidates.extend(removed_bounds.read());

    for entity in candidates {
        let Ok((_, target, mesh, aabb, geometry, pending)) = demanded_targets.get(entity) else {
            continue;
        };
        if mesh.is_none() {
            mark_mesh_anchor_pending(
                entity,
                MeshAnchorPendingCause::WaitingForMesh,
                &mut commands,
            );
            continue;
        }
        let Some(aabb) = aabb else {
            mark_mesh_anchor_pending(
                entity,
                MeshAnchorPendingCause::WaitingForBounds,
                &mut commands,
            );
            continue;
        };
        if geometry.is_none() || pending.is_some() || target.is_changed() || aabb.is_changed() {
            write_mesh_anchor_geometry(entity, target.face, *aabb, &mut warnings, &mut commands);
        }
    }
}

fn mark_mesh_anchor_pending(
    entity: Entity,
    cause: MeshAnchorPendingCause,
    commands: &mut Commands,
) {
    commands
        .entity(entity)
        .remove::<(ResolvedAnchorGeometry, MeshAnchorGeometry)>()
        .insert(MeshAnchorGeometryPending { cause });
}

fn write_mesh_anchor_geometry(
    entity: Entity,
    face: MeshFace,
    aabb: Aabb,
    warnings: &mut MeshAnchorWarnings,
    commands: &mut Commands,
) {
    let Some(geometry) = mesh_face_geometry(aabb, face) else {
        mark_mesh_anchor_pending(entity, MeshAnchorPendingCause::UnusableBounds, commands);
        return;
    };
    warnings.0.remove(&entity);
    commands
        .entity(entity)
        .remove::<MeshAnchorGeometryPending>()
        .insert((geometry, MeshAnchorGeometry));
}

pub(super) fn warn_pending_mesh_anchor_geometry(
    targets: Query<
        (Entity, &MeshAnchorTarget, &MeshAnchorGeometryPending),
        (With<AnchoredHere>, Changed<MeshAnchorGeometryPending>),
    >,
    mut warnings: ResMut<MeshAnchorWarnings>,
) {
    for (entity, target, pending) in &targets {
        if warnings.0.insert(entity) {
            match pending.cause {
                MeshAnchorPendingCause::WaitingForMesh => {
                    warn!("mesh anchor target {entity:?} is waiting for Mesh3d");
                },
                MeshAnchorPendingCause::WaitingForBounds => {
                    warn!("mesh anchor target {entity:?} is waiting for Aabb");
                },
                MeshAnchorPendingCause::UnusableBounds => {
                    warn!(
                        "mesh anchor target {entity:?} has no usable bounds for face {:?}",
                        target.face
                    );
                },
            }
        }
    }
}

pub(super) fn remove_mesh_anchor_geometry(
    removed: On<Remove, AnchoredHere>,
    targets: Query<
        (
            Option<&MeshAnchorGeometry>,
            Option<&MeshAnchorGeometryPending>,
        ),
        With<MeshAnchorTarget>,
    >,
    mut warnings: ResMut<MeshAnchorWarnings>,
    mut commands: Commands,
) {
    let entity = removed.entity;
    warnings.0.remove(&entity);
    if targets
        .get(entity)
        .is_ok_and(|(geometry, pending)| geometry.is_some() || pending.is_some())
    {
        commands.entity(entity).remove::<(
            ResolvedAnchorGeometry,
            MeshAnchorGeometry,
            MeshAnchorGeometryPending,
        )>();
    }
}

fn mesh_face_geometry(aabb: Aabb, face: MeshFace) -> Option<ResolvedAnchorGeometry> {
    let center = Vec3::from(aabb.center);
    let half_extents = Vec3::from(aabb.half_extents);
    if !center.is_finite() || !half_extents.is_finite() {
        return None;
    }
    let frame = mesh_face_frame(face);
    let right = frame * Vec3::X;
    let up = frame * Vec3::Y;
    let normal = frame * Vec3::Z;
    let right_extent = right.abs().dot(half_extents);
    let up_extent = up.abs().dot(half_extents);
    let normal_extent = normal.abs().dot(half_extents);
    if right_extent <= 0.0 || up_extent <= 0.0 {
        return None;
    }
    let face_center = center + normal * normal_extent;
    let mut frames = Vec::with_capacity(QUAD_ANCHOR_COUNT);
    let orientation = Orientation::try_from(frame).ok()?;
    for anchor in [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
        Anchor::TopCenter,
        Anchor::CenterRight,
        Anchor::BottomCenter,
        Anchor::CenterLeft,
        Anchor::Center,
    ] {
        let (x_fraction, y_fraction) = anchor.offset_fraction();
        let x = x_fraction.mul_add(2.0, -1.0) * right_extent;
        let y = y_fraction.mul_add(-2.0, 1.0) * up_extent;
        let anchor_frame = AnchorFrame::try_new(
            Position::from(face_center + right * x + up * y),
            orientation,
        )
        .ok()?;
        frames.push((AnchorSite::from(anchor), anchor_frame));
    }
    let edges = vec![
        Edge {
            start: AnchorSite::from(Anchor::TopLeft),
            end:   AnchorSite::from(Anchor::TopRight),
        },
        Edge {
            start: AnchorSite::from(Anchor::TopRight),
            end:   AnchorSite::from(Anchor::BottomRight),
        },
        Edge {
            start: AnchorSite::from(Anchor::BottomRight),
            end:   AnchorSite::from(Anchor::BottomLeft),
        },
        Edge {
            start: AnchorSite::from(Anchor::BottomLeft),
            end:   AnchorSite::from(Anchor::TopLeft),
        },
    ];
    debug_assert_eq!(edges.len(), QUAD_EDGE_COUNT);
    ResolvedAnchorGeometry::try_new(frames, edges).ok()
}

fn mesh_face_frame(face: MeshFace) -> Quat {
    match face {
        MeshFace::PositiveX => Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        MeshFace::NegativeX => Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        MeshFace::PositiveY => Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        MeshFace::NegativeY => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        MeshFace::PositiveZ => Quat::IDENTITY,
        MeshFace::NegativeZ => Quat::from_rotation_y(std::f32::consts::PI),
    }
}

#[derive(Clone)]
struct TooltipMaterializationContext {
    target:        Entity,
    space:         PanelSpace,
    layout_unit:   Unit,
    window:        Option<Entity>,
    camera_order:  Option<isize>,
    render_layers: RenderLayers,
}

pub(super) fn materialize_requested_tooltips(
    controllers: Query<
        (
            Entity,
            &Tooltip,
            &TooltipFor,
            Option<&AuthoredTooltipTargetSpace>,
            Has<DiegeticPanel>,
            Option<&Transform>,
            Option<&GlobalTransform>,
            Option<&Visibility>,
        ),
        With<PrepareTooltip>,
    >,
    panels: Query<&DiegeticPanel>,
    widgets: Query<&super::WidgetOf, With<super::PanelWidget>>,
    anchor_geometry: Query<(), With<ResolvedAnchorGeometry>>,
    mesh_targets: Query<(), With<MeshAnchorTarget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    target_layers: Query<&RenderLayers>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    defaults: Res<crate::cascade::PanelDefaults>,
    mut commands: Commands,
) {
    for (
        controller,
        tooltip,
        tooltip_for,
        authored_space,
        has_panel,
        previous_transform,
        previous_global,
        previous_visibility,
    ) in &controllers
    {
        if has_panel {
            commands.entity(controller).remove::<PrepareTooltip>();
            continue;
        }
        let Some(context) = tooltip_materialization_context(
            tooltip_for.target(),
            authored_space,
            &panels,
            &widgets,
            &anchor_geometry,
            &mesh_targets,
            &screen_targets,
            &target_layers,
            &windows,
            &primary_window,
            &defaults,
        ) else {
            continue;
        };
        let Some(panel) = build_tooltip_panel(tooltip, &context, &windows) else {
            continue;
        };
        let width_constraint = initial_tooltip_width_constraint(tooltip, &context, &windows);
        let authored_attachment = tooltip.attachment();
        let materialized = MaterializedTooltip {
            target: context.target,
            space: context.space,
            layout_unit: context.layout_unit,
            window: context.window,
            camera_order: context.camera_order,
            render_layers: context.render_layers.clone(),
            blueprint: Arc::clone(tooltip.blueprint()),
            authored_attachment,
            placement_policy: tooltip.placement_policy,
            previous_transform: previous_transform.copied(),
            previous_global: previous_global.copied(),
            previous_visibility: previous_visibility.copied(),
        };
        commands
            .entity(controller)
            .remove::<(PrepareTooltip, super::PanelPicking)>()
            .insert((
                panel,
                Visibility::Hidden,
                materialized,
                TooltipReadiness::Pending,
                TooltipPlacementState {
                    attachment:   authored_attachment,
                    result:       TooltipPlacementResult::Pending,
                    world_target: None,
                },
            ));
        if let Some(width_constraint) = width_constraint {
            commands.entity(controller).insert(width_constraint);
        }
        panel::write_owned_render_layers(
            &mut commands,
            controller,
            controller,
            Some(context.render_layers),
        );
    }
}

fn initial_tooltip_width_constraint(
    tooltip: &Tooltip,
    context: &TooltipMaterializationContext,
    windows: &Query<&Window>,
) -> Option<TooltipWidthConstraint> {
    if context.space != PanelSpace::Screen
        || tooltip.placement_policy != TooltipPlacementPolicy::KeepVisible
    {
        return None;
    }
    let window = context.window?;
    let max_width = usable_viewport_axis(windows.get(window).ok()?.width())?;
    Some(TooltipWidthConstraint(max_width))
}

fn tooltip_materialization_context(
    target: Entity,
    authored_space: Option<&AuthoredTooltipTargetSpace>,
    panels: &Query<&DiegeticPanel>,
    widgets: &Query<&super::WidgetOf, With<super::PanelWidget>>,
    anchor_geometry: &Query<(), With<ResolvedAnchorGeometry>>,
    mesh_targets: &Query<(), With<MeshAnchorTarget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    target_layers: &Query<&RenderLayers>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    defaults: &crate::cascade::PanelDefaults,
) -> Option<TooltipMaterializationContext> {
    if let Ok(panel) = panels.get(target) {
        return panel_tooltip_context(
            target,
            panel,
            target_layers.get(target).ok(),
            windows,
            primary_window,
        );
    }
    if let Ok(widget_of) = widgets.get(target) {
        let panel = panels.get(widget_of.panel()).ok()?;
        return panel_tooltip_context(
            target,
            panel,
            target_layers.get(widget_of.panel()).ok(),
            windows,
            primary_window,
        );
    }
    let space = authored_space?.0;
    match space {
        PanelSpace::World => {
            if anchor_geometry.get(target).is_err() && mesh_targets.get(target).is_err() {
                return None;
            }
            Some(TooltipMaterializationContext {
                target,
                space,
                layout_unit: defaults.layout_unit,
                window: None,
                camera_order: None,
                render_layers: target_layers
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|_| RenderLayers::layer(0)),
            })
        },
        PanelSpace::Screen => {
            let screen_target = screen_targets.get(target).ok()?;
            windows.get(screen_target.window()).ok()?;
            Some(TooltipMaterializationContext {
                target,
                space,
                layout_unit: screen_target.layout_unit(),
                window: Some(screen_target.window()),
                camera_order: Some(screen_target.camera_order()),
                render_layers: screen_target.render_layers().clone(),
            })
        },
    }
}

fn panel_tooltip_context(
    target: Entity,
    panel: &DiegeticPanel,
    target_layers: Option<&RenderLayers>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<TooltipMaterializationContext> {
    match panel.coordinate_space() {
        CoordinateSpace::World { .. } => Some(TooltipMaterializationContext {
            target,
            space: PanelSpace::World,
            layout_unit: panel.layout_unit(),
            window: None,
            camera_order: None,
            render_layers: target_layers
                .cloned()
                .unwrap_or_else(|| RenderLayers::layer(0)),
        }),
        CoordinateSpace::Screen {
            camera_order,
            render_layers,
            window,
            ..
        } => {
            let window = live_window(*window, windows, primary_window)?;
            Some(TooltipMaterializationContext {
                target,
                space: PanelSpace::Screen,
                layout_unit: panel.layout_unit(),
                window: Some(window),
                camera_order: Some(*camera_order),
                render_layers: render_layers.clone(),
            })
        },
    }
}

fn live_window(
    window_ref: WindowRef,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    let entity = match window_ref {
        WindowRef::Primary => primary_window.single().ok()?,
        WindowRef::Entity(entity) => entity,
    };
    windows.get(entity).ok()?;
    Some(entity)
}

fn build_tooltip_panel(
    tooltip: &Tooltip,
    context: &TooltipMaterializationContext,
    windows: &Query<&Window>,
) -> Option<DiegeticPanel> {
    let tree = tooltip.blueprint().as_ref().clone();
    match context.space {
        PanelSpace::World => DiegeticPanel::world()
            .size(
                fit_sizing(context.layout_unit, f32::MAX),
                fit_sizing(context.layout_unit, f32::MAX),
            )
            .anchor(tooltip.source_anchor)
            .picking(super::PanelPicking::PASS_THROUGH)
            .with_tree(tree)
            .build()
            .map_err(|error| {
                error!("failed to materialize world tooltip panel: {error}");
            })
            .ok(),
        PanelSpace::Screen => {
            let window = context.window?;
            let max_width = initial_tooltip_width_constraint(tooltip, context, windows)
                .map_or(f32::MAX, |constraint| constraint.0);
            DiegeticPanel::screen()
                .size(
                    fit_sizing(Unit::Pixels, max_width),
                    fit_sizing(Unit::Pixels, f32::MAX),
                )
                .anchor(tooltip.source_anchor)
                .window_entity(window)
                .camera_order(context.camera_order?)
                .render_layers(context.render_layers.clone())
                .picking(super::PanelPicking::PASS_THROUGH)
                .with_tree(tree)
                .build()
                .map_err(|error| {
                    error!("failed to materialize screen tooltip panel: {error}");
                })
                .ok()
        },
    }
}

const fn fit_sizing(unit: Unit, max: f32) -> Sizing {
    Sizing::Fit {
        min: Dimension {
            value: 0.0,
            unit:  Some(unit),
        },
        max: Dimension {
            value: max,
            unit:  Some(unit),
        },
    }
}

fn usable_viewport_axis(axis: f32) -> Option<f32> {
    let usable = TOOLTIP_VIEWPORT_MARGIN.mul_add(-2.0, axis);
    (usable.is_finite() && usable > 0.0).then_some(usable)
}

const fn zero_origin_viewport(size: Vec2) -> Rect { Rect::from_corners(Vec2::ZERO, size) }

pub(super) fn invalidate_stale_materialized_tooltips(
    mut controllers: Query<
        (
            Entity,
            &TooltipFor,
            &MaterializedTooltip,
            &mut TooltipReadiness,
            &mut TooltipPlacementState,
            &mut Visibility,
        ),
        Changed<TooltipFor>,
    >,
    mut commands: Commands,
) {
    for (controller, tooltip_for, materialized, mut readiness, mut placement, mut visibility) in
        &mut controllers
    {
        if tooltip_for.target() == materialized.target {
            continue;
        }
        *readiness = TooltipReadiness::Pending;
        set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Unavailable);
        *visibility = Visibility::Hidden;
        commands.entity(controller).remove::<(
            PanelAttachmentAuthored,
            PanelAnchorOffset,
            TooltipWidthConstraintRequest,
        )>();
    }
}

pub(super) fn attach_materialized_tooltips(
    controllers: Query<
        (
            Entity,
            &TooltipFor,
            &MaterializedTooltip,
            &TooltipPlacementState,
        ),
        Without<PanelAttachmentAuthored>,
    >,
    panel_reader: PanelEntityReader,
    widget_reader: super::PanelWidgetReader,
    widgets: Query<(&super::PanelWidget, &super::WidgetOf)>,
    mut commands: Commands,
) {
    for (controller, tooltip_for, materialized, placement) in &controllers {
        if tooltip_for.target() != materialized.target {
            continue;
        }
        match materialized.space {
            PanelSpace::World => {
                let Some(source) = panel_reader.world(controller) else {
                    continue;
                };
                if let Some(target) = panel_reader.world(materialized.target) {
                    commands.attach_to_panel(source, target, placement.attachment);
                    continue;
                }
                if let Ok((widget, widget_of)) = widgets.get(materialized.target)
                    && let Some(owner) = panel_reader.world(widget_of.panel())
                    && let Some(target) = widget_reader.typed_entity(owner, widget.id())
                    && target.entity() == materialized.target
                {
                    commands.attach_to_widget(source, target, placement.attachment);
                    continue;
                }
                queue_general_tooltip_attachment(
                    &mut commands,
                    controller,
                    materialized,
                    placement.attachment,
                );
            },
            PanelSpace::Screen => {
                let Some(source) = panel_reader.screen(controller) else {
                    continue;
                };
                if let Some(target) = panel_reader.screen(materialized.target) {
                    commands.attach_to_panel(source, target, placement.attachment);
                    continue;
                }
                if let Ok((widget, widget_of)) = widgets.get(materialized.target)
                    && let Some(owner) = panel_reader.screen(widget_of.panel())
                    && let Some(target) = widget_reader.typed_entity(owner, widget.id())
                    && target.entity() == materialized.target
                {
                    commands.attach_to_widget(source, target, placement.attachment);
                    continue;
                }
                queue_general_tooltip_attachment(
                    &mut commands,
                    controller,
                    materialized,
                    placement.attachment,
                );
            },
        }
    }
}

#[derive(Clone, Copy)]
struct GeneralTooltipAttachmentOperation {
    controller: Entity,
    target:     Entity,
    space:      PanelSpace,
    window:     Option<Entity>,
    attachment: PanelAttachment,
}

fn queue_general_tooltip_attachment(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    materialized: &MaterializedTooltip,
    attachment: PanelAttachment,
) {
    commands.run_system_cached_with(
        apply_general_tooltip_attachment,
        GeneralTooltipAttachmentOperation {
            controller,
            target: materialized.target,
            space: materialized.space,
            window: materialized.window,
            attachment,
        },
    );
}

fn apply_general_tooltip_attachment(
    In(operation): In<GeneralTooltipAttachmentOperation>,
    controllers: Query<(&DiegeticPanel, &TooltipFor, &MaterializedTooltip)>,
    anchor_geometry: Query<(), With<ResolvedAnchorGeometry>>,
    mesh_targets: Query<(), With<MeshAnchorTarget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<(), With<Window>>,
    mut commands: Commands,
) {
    let Ok((panel, tooltip_for, materialized)) = controllers.get(operation.controller) else {
        return;
    };
    if PanelSpace::from(panel.coordinate_space()) != operation.space
        || tooltip_for.target() != operation.target
        || materialized.target != operation.target
        || materialized.space != operation.space
    {
        return;
    }
    let target_is_current = match operation.space {
        PanelSpace::World => {
            anchor_geometry.contains(operation.target) || mesh_targets.contains(operation.target)
        },
        PanelSpace::Screen => screen_targets.get(operation.target).is_ok_and(|target| {
            Some(target.window()) == operation.window && windows.contains(target.window())
        }),
    };
    if !target_is_current {
        return;
    }
    commands.entity(operation.controller).insert((
        crate::panel::PanelAttachmentAuthored::new(
            operation.target,
            operation.attachment.source_anchor(),
            operation.attachment.target_anchor(),
        ),
        operation.attachment.offset(),
    ));
}

#[derive(Clone)]
struct ScreenTooltipTarget {
    bounds:        PanelScreenBounds,
    rect:          ScreenPanelRect,
    window:        Entity,
    camera_order:  isize,
    render_layers: RenderLayers,
    layout_unit:   Unit,
    layout_scale:  Vec2,
}

impl ScreenTooltipTarget {
    fn presentation_matches(&self, materialized: &MaterializedTooltip) -> bool {
        materialized.window == Some(self.window)
            && materialized.camera_order == Some(self.camera_order)
            && materialized.render_layers == self.render_layers
            && materialized.layout_unit == self.layout_unit
    }
}

pub(super) fn resolve_screen_tooltip_placements(
    mut controllers: Query<(
        Entity,
        &TooltipFor,
        &MaterializedTooltip,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &Transform,
        &ResolvedScreenPanelPosition,
        Option<&TooltipWidthConstraint>,
        &mut TooltipPlacementState,
    )>,
    panels: Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: Query<(&super::WidgetOf, &super::WidgetAnchorRect), With<super::PanelWidget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
    #[cfg(test)] mut run_count: Option<ResMut<TooltipPlacementRunCount>>,
) {
    #[cfg(test)]
    if let Some(run_count) = run_count.as_mut() {
        run_count.screen += 1;
    }
    for (
        controller,
        tooltip_for,
        materialized,
        panel,
        computed,
        transform,
        resolved_position,
        width_constraint,
        mut placement,
    ) in &mut controllers
    {
        if materialized.space != PanelSpace::Screen {
            continue;
        }
        if tooltip_for.target() != materialized.target {
            set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Unavailable);
            continue;
        }
        let decision = screen_tooltip_placement(
            ScreenTooltipPlacementInput {
                materialized,
                panel,
                computed,
                transform,
                resolved_position,
                width_constraint,
                current_attachment: placement.attachment,
            },
            &panels,
            &widgets,
            &screen_targets,
            &windows,
            &primary_window,
        );
        match decision {
            TooltipPlacementDecision::ConstrainWidth(width) => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Pending);
                commands
                    .entity(controller)
                    .insert(TooltipWidthConstraintRequest::Apply(width));
            },
            TooltipPlacementDecision::RestoreWidth => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Pending);
                commands
                    .entity(controller)
                    .insert(TooltipWidthConstraintRequest::Restore);
            },
            TooltipPlacementDecision::Move(attachment) => {
                set_tooltip_placement(&mut placement, attachment, TooltipPlacementResult::Fits);
                commands.entity(controller).insert((
                    crate::panel::PanelAttachmentAuthored::new(
                        materialized.target,
                        attachment.source_anchor(),
                        attachment.target_anchor(),
                    ),
                    attachment.offset(),
                    ScreenTooltipAttachmentCorrection,
                ));
            },
            TooltipPlacementDecision::Fits => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Fits);
            },
            TooltipPlacementDecision::Unavailable => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Unavailable);
            },
        }
    }
}

struct ScreenTooltipPlacementInput<'a> {
    materialized:       &'a MaterializedTooltip,
    panel:              &'a DiegeticPanel,
    computed:           &'a ComputedDiegeticPanel,
    transform:          &'a Transform,
    resolved_position:  &'a ResolvedScreenPanelPosition,
    width_constraint:   Option<&'a TooltipWidthConstraint>,
    current_attachment: PanelAttachment,
}

fn screen_tooltip_placement(
    input: ScreenTooltipPlacementInput<'_>,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: &Query<(&super::WidgetOf, &super::WidgetAnchorRect), With<super::PanelWidget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> TooltipPlacementDecision {
    let Some(target) = screen_tooltip_target(
        input.materialized.target,
        panels,
        widgets,
        screen_targets,
        windows,
        primary_window,
    ) else {
        return TooltipPlacementDecision::Unavailable;
    };
    if !target.presentation_matches(input.materialized) {
        return TooltipPlacementDecision::Unavailable;
    }
    let Ok(window) = windows.get(target.window) else {
        return TooltipPlacementDecision::Unavailable;
    };
    let viewport = zero_origin_viewport(Vec2::new(window.width(), window.height()));
    match input.materialized.placement_policy {
        TooltipPlacementPolicy::KeepVisible => {
            let Some(max_width) = usable_viewport_axis(viewport.width()) else {
                return TooltipPlacementDecision::Unavailable;
            };
            if input
                .width_constraint
                .is_none_or(|current| (current.0 - max_width).abs() > f32::EPSILON)
            {
                return TooltipPlacementDecision::ConstrainWidth(max_width);
            }
        },
        TooltipPlacementPolicy::Fixed if input.width_constraint.is_some() => {
            return TooltipPlacementDecision::RestoreWidth;
        },
        TooltipPlacementPolicy::Fixed => {},
    }
    let source_size = Vec2::new(input.panel.width(), input.panel.height());
    if !valid_screen_size(source_size) {
        return TooltipPlacementDecision::Unavailable;
    }
    let Some(source_rect) = screen_space::screen_panel_rect(
        input.panel,
        Some(input.resolved_position),
        Some(input.transform),
        viewport.size(),
    ) else {
        return TooltipPlacementDecision::Unavailable;
    };
    let attachment = match input.materialized.placement_policy {
        TooltipPlacementPolicy::Fixed => Some(input.materialized.authored_attachment),
        TooltipPlacementPolicy::KeepVisible => {
            let Some(usable_height) = usable_viewport_axis(viewport.height()) else {
                return TooltipPlacementDecision::Unavailable;
            };
            if !tooltip_layout_fits_panel(input.panel, input.computed)
                || input.computed.content_height() > usable_height
            {
                return TooltipPlacementDecision::Unavailable;
            }
            fit_tooltip_in_viewport(
                input.materialized.authored_attachment,
                source_rect,
                &target,
                viewport,
            )
        },
    };
    match attachment {
        Some(attachment) if attachment == input.current_attachment => {
            TooltipPlacementDecision::Fits
        },
        Some(attachment) => TooltipPlacementDecision::Move(attachment),
        None => TooltipPlacementDecision::Unavailable,
    }
}

fn screen_tooltip_target(
    target: Entity,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: &Query<(&super::WidgetOf, &super::WidgetAnchorRect), With<super::PanelWidget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<ScreenTooltipTarget> {
    if let Ok((panel, transform, resolved)) = panels.get(target) {
        return screen_panel_target(panel, transform, resolved, windows, primary_window);
    }
    if let Ok((widget_of, anchor_rect)) = widgets.get(target) {
        let (panel, transform, resolved) = panels.get(widget_of.panel()).ok()?;
        let owner = screen_panel_target(panel, transform, resolved, windows, primary_window)?;
        let rect =
            crate::screen_space::ScreenPanelRect::from_widget(owner.rect, *anchor_rect, transform)?;
        return Some(ScreenTooltipTarget {
            bounds: rect.projected_bounds()?,
            rect,
            layout_scale: rect.layout_scale(),
            ..owner
        });
    }
    let target = screen_targets.get(target).ok()?;
    windows.get(target.window()).ok()?;
    let rect = crate::screen_space::ScreenPanelRect::from_screen_target(target);
    Some(ScreenTooltipTarget {
        bounds: rect.projected_bounds()?,
        rect,
        window: target.window(),
        camera_order: target.camera_order(),
        render_layers: target.render_layers().clone(),
        layout_unit: target.layout_unit(),
        layout_scale: Vec2::ONE,
    })
}

fn screen_panel_target(
    panel: &DiegeticPanel,
    transform: &Transform,
    resolved: Option<&ResolvedScreenPanelPosition>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<ScreenTooltipTarget> {
    let CoordinateSpace::Screen {
        camera_order,
        render_layers,
        window,
        ..
    } = panel.coordinate_space()
    else {
        return None;
    };
    let window = live_window(*window, windows, primary_window)?;
    let window_component = windows.get(window).ok()?;
    let viewport = Vec2::new(window_component.width(), window_component.height());
    let rect = screen_space::screen_panel_rect(panel, resolved, Some(transform), viewport)?;
    Some(ScreenTooltipTarget {
        bounds: rect.projected_bounds()?,
        rect,
        window,
        camera_order: *camera_order,
        render_layers: render_layers.clone(),
        layout_unit: panel.layout_unit(),
        layout_scale: Vec2::ONE,
    })
}

fn valid_screen_size(size: Vec2) -> bool { size.is_finite() && size.x > 0.0 && size.y > 0.0 }

fn tooltip_layout_fits_panel(panel: &DiegeticPanel, computed: &ComputedDiegeticPanel) -> bool {
    if computed.content_width() > panel.width() + f32::EPSILON
        || computed.content_height() > panel.height() + f32::EPSILON
    {
        return false;
    }
    let PanelLayout::Solved(result) = computed.layout() else {
        return false;
    };
    let Some(root) = result.computed.first().map(|layout| layout.bounds) else {
        return false;
    };
    result.computed.iter().skip(1).all(|layout| {
        let bounds = layout.bounds;
        bounds.x + f32::EPSILON >= root.x
            && bounds.y + f32::EPSILON >= root.y
            && bounds.x + bounds.width <= root.x + root.width + f32::EPSILON
            && bounds.y + bounds.height <= root.y + root.height + f32::EPSILON
    })
}

fn valid_screen_scale(scale: Vec2) -> bool {
    scale.is_finite() && scale.x.abs() > f32::EPSILON && scale.y.abs() > f32::EPSILON
}

fn rotate_screen_vector(vector: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(
        vector.y.mul_add(sin, vector.x * cos),
        vector.y.mul_add(cos, -vector.x * sin),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TooltipSide {
    Above,
    Right,
    Below,
    Left,
}

impl TooltipSide {
    const fn opposite(self) -> Self {
        match self {
            Self::Above => Self::Below,
            Self::Right => Self::Left,
            Self::Below => Self::Above,
            Self::Left => Self::Right,
        }
    }

    const fn is_vertical(self) -> bool { matches!(self, Self::Above | Self::Below) }

    const fn anchors(self) -> (Anchor, Anchor) {
        match self {
            Self::Above => (Anchor::BottomCenter, Anchor::TopCenter),
            Self::Right => (Anchor::CenterLeft, Anchor::CenterRight),
            Self::Below => (Anchor::TopCenter, Anchor::BottomCenter),
            Self::Left => (Anchor::CenterRight, Anchor::CenterLeft),
        }
    }
}

fn fit_tooltip_in_viewport(
    authored: PanelAttachment,
    source: ScreenPanelRect,
    target: &ScreenTooltipTarget,
    viewport: Rect,
) -> Option<PanelAttachment> {
    usable_viewport_axis(viewport.width())?;
    usable_viewport_axis(viewport.height())?;
    let authored_layout = authored.offset().to_layout_units(target.layout_unit);
    let authored_local_pixels = Vec2::new(
        authored_layout.x * target.layout_scale.x,
        authored_layout.y * target.layout_scale.y,
    );
    let preferred = preferred_side(authored, authored_local_pixels);
    let order = tooltip_side_order(preferred, target.bounds, viewport);
    for side in order {
        let (source_anchor, target_anchor, mut local_pixels) = if side == preferred {
            (
                authored.source_anchor(),
                authored.target_anchor(),
                authored_local_pixels,
            )
        } else {
            let (source_anchor, target_anchor) = side.anchors();
            let gap = if preferred.is_vertical() {
                authored_local_pixels.y.abs()
            } else {
                authored_local_pixels.x.abs()
            };
            let offset = match side {
                TooltipSide::Above => Vec2::new(0.0, -gap),
                TooltipSide::Right => Vec2::new(gap, 0.0),
                TooltipSide::Below => Vec2::new(0.0, gap),
                TooltipSide::Left => Vec2::new(-gap, 0.0),
            };
            (source_anchor, target_anchor, offset)
        };
        let target_point = target.rect.oriented_anchor_point(target_anchor)?;
        let source_anchor_position =
            target_point + rotate_screen_vector(local_pixels, target.rect.angle());
        let natural_bounds = source.placed_bounds(source_anchor, source_anchor_position)?;
        let shift = limited_along_edge_shift(
            side,
            natural_bounds.top_left(),
            natural_bounds.size(),
            target_point,
            viewport,
        );
        local_pixels += rotate_screen_vector(shift, -target.rect.angle());
        let source_anchor_position =
            target_point + rotate_screen_vector(local_pixels, target.rect.angle());
        let bounds = source.placed_bounds(source_anchor, source_anchor_position)?;
        if bounds_fit_viewport(bounds.top_left(), bounds.size(), viewport) {
            let layout_offset = Vec3::new(
                local_pixels.x / target.layout_scale.x,
                local_pixels.y / target.layout_scale.y,
                authored_layout.z,
            );
            return Some(
                PanelAttachment::new(source_anchor, target_anchor).with_offset(
                    PanelAnchorOffset::new(layout_offset.x, layout_offset.y)
                        .with_z(layout_offset.z),
                ),
            );
        }
    }
    None
}

fn preferred_side(authored: PanelAttachment, offset: Vec2) -> TooltipSide {
    let (source_x, source_y) = authored.source_anchor().offset_fraction();
    let (target_x, target_y) = authored.target_anchor().offset_fraction();
    let delta = Vec2::new(target_x - source_x, target_y - source_y);
    if delta.y.abs() >= delta.x.abs() && delta.y != 0.0 {
        if delta.y > 0.0 {
            TooltipSide::Below
        } else {
            TooltipSide::Above
        }
    } else if delta.x != 0.0 {
        if delta.x > 0.0 {
            TooltipSide::Right
        } else {
            TooltipSide::Left
        }
    } else if offset.y.abs() >= offset.x.abs() {
        if offset.y >= 0.0 {
            TooltipSide::Below
        } else {
            TooltipSide::Above
        }
    } else if offset.x >= 0.0 {
        TooltipSide::Right
    } else {
        TooltipSide::Left
    }
}

fn tooltip_side_order(
    preferred: TooltipSide,
    target: PanelScreenBounds,
    viewport: Rect,
) -> [TooltipSide; 4] {
    let top_left = target.top_left();
    let bottom_right = top_left + target.size();
    let remaining = if preferred.is_vertical() {
        let left_room = top_left.x - (viewport.min.x + TOOLTIP_VIEWPORT_MARGIN);
        let right_room = viewport.max.x - TOOLTIP_VIEWPORT_MARGIN - bottom_right.x;
        // Equal horizontal room checks the right side before the left side.
        if right_room >= left_room {
            [TooltipSide::Right, TooltipSide::Left]
        } else {
            [TooltipSide::Left, TooltipSide::Right]
        }
    } else {
        let above_room = top_left.y - (viewport.min.y + TOOLTIP_VIEWPORT_MARGIN);
        let below_room = viewport.max.y - TOOLTIP_VIEWPORT_MARGIN - bottom_right.y;
        // Equal vertical room checks the lower side before the upper side.
        if below_room >= above_room {
            [TooltipSide::Below, TooltipSide::Above]
        } else {
            [TooltipSide::Above, TooltipSide::Below]
        }
    };
    [preferred, preferred.opposite(), remaining[0], remaining[1]]
}

fn limited_along_edge_shift(
    side: TooltipSide,
    top_left: Vec2,
    source_size: Vec2,
    target_point: Vec2,
    viewport: Rect,
) -> Vec2 {
    if side.is_vertical() {
        let desired = containment_shift(
            top_left.x,
            source_size.x,
            viewport.min.x + TOOLTIP_VIEWPORT_MARGIN,
            viewport.max.x - TOOLTIP_VIEWPORT_MARGIN,
        );
        let min = target_point.x - (top_left.x + source_size.x);
        let max = target_point.x - top_left.x;
        Vec2::new(desired.clamp(min, max), 0.0)
    } else {
        let desired = containment_shift(
            top_left.y,
            source_size.y,
            viewport.min.y + TOOLTIP_VIEWPORT_MARGIN,
            viewport.max.y - TOOLTIP_VIEWPORT_MARGIN,
        );
        let min = target_point.y - (top_left.y + source_size.y);
        let max = target_point.y - top_left.y;
        Vec2::new(0.0, desired.clamp(min, max))
    }
}

fn containment_shift(start: f32, length: f32, minimum: f32, maximum: f32) -> f32 {
    if start < minimum {
        minimum - start
    } else if start + length > maximum {
        maximum - start - length
    } else {
        0.0
    }
}

fn bounds_fit_viewport(top_left: Vec2, size: Vec2, viewport: Rect) -> bool {
    let bottom_right = top_left + size;
    top_left.x >= viewport.min.x + TOOLTIP_VIEWPORT_MARGIN
        && top_left.y >= viewport.min.y + TOOLTIP_VIEWPORT_MARGIN
        && bottom_right.x <= viewport.max.x - TOOLTIP_VIEWPORT_MARGIN
        && bottom_right.y <= viewport.max.y - TOOLTIP_VIEWPORT_MARGIN
}

pub(super) fn apply_tooltip_width_constraints(
    mut controllers: Query<(
        Entity,
        &mut DiegeticPanel,
        &MaterializedTooltip,
        &TooltipWidthConstraintRequest,
        Option<&TooltipWidthConstraint>,
        &mut TooltipReadiness,
    )>,
    mut commands: Commands,
) {
    for (entity, mut panel, materialized, request, current, mut readiness) in &mut controllers {
        match *request {
            TooltipWidthConstraintRequest::Apply(max_width)
                if current.is_some_and(|current| (current.0 - max_width).abs() <= f32::EPSILON) =>
            {
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>();
            },
            TooltipWidthConstraintRequest::Apply(max_width) => {
                panel::constrain_fit_width(&mut panel, &materialized.blueprint, max_width);
                *readiness = TooltipReadiness::Pending;
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>()
                    .insert(TooltipWidthConstraint(max_width));
            },
            TooltipWidthConstraintRequest::Restore if current.is_none() => {
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>();
            },
            TooltipWidthConstraintRequest::Restore => {
                panel::constrain_fit_width(&mut panel, &materialized.blueprint, f32::MAX);
                *readiness = TooltipReadiness::Pending;
                commands
                    .entity(entity)
                    .remove::<(TooltipWidthConstraintRequest, TooltipWidthConstraint)>();
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorldAnchorFrame {
    position: Vec3,
    right:    Vec3,
    up:       Vec3,
    normal:   Vec3,
}

#[derive(Clone, Copy)]
struct WorldTooltipTarget {
    bounds:                  PanelScreenBounds,
    anchors:                 [WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    layout_unit:             Unit,
    world_per_layout_unit:   Vec2,
    world_per_layout_unit_z: f32,
}

impl WorldTooltipTarget {
    const fn anchor(&self, anchor: Anchor) -> WorldAnchorFrame {
        self.anchors[anchor_index(anchor)]
    }

    const fn snapshot(&self) -> WorldTooltipTargetSnapshot {
        WorldTooltipTargetSnapshot {
            anchors:                 self.anchors,
            layout_unit:             self.layout_unit,
            world_per_layout_unit:   self.world_per_layout_unit,
            world_per_layout_unit_z: self.world_per_layout_unit_z,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorldTooltipTargetSnapshot {
    anchors:                 [WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    layout_unit:             Unit,
    world_per_layout_unit:   Vec2,
    world_per_layout_unit_z: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorldTargetChange {
    Initial,
    Stable,
    Changed,
}

fn update_world_target_snapshot(
    placement: &mut Mut<'_, TooltipPlacementState>,
    snapshot: &WorldTooltipTargetSnapshot,
) -> WorldTargetChange {
    match placement.world_target {
        None => {
            placement.world_target = Some(*snapshot);
            WorldTargetChange::Initial
        },
        Some(current) if current == *snapshot => WorldTargetChange::Stable,
        Some(_) => {
            placement.world_target = Some(*snapshot);
            WorldTargetChange::Changed
        },
    }
}

struct TooltipCameraView<'a> {
    camera:           &'a Camera,
    global_transform: &'a GlobalTransform,
    viewport:         Rect,
    inputs_changed:   bool,
}

pub(super) fn resolve_world_tooltip_placements(
    mut controllers: Query<(
        Entity,
        &TooltipFor,
        &MaterializedTooltip,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &GlobalTransform,
        Option<Ref<TooltipPresentationCamera>>,
        Option<&TooltipWidthConstraint>,
        &mut TooltipPlacementState,
    )>,
    panels: Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: Query<&super::WidgetOf, With<super::PanelWidget>>,
    anchor_geometry: Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    cameras: Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: Query<Ref<Window>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
    #[cfg(test)] mut run_count: Option<ResMut<TooltipPlacementRunCount>>,
) {
    #[cfg(test)]
    if let Some(run_count) = run_count.as_mut() {
        run_count.world += 1;
    }
    for (
        controller,
        tooltip_for,
        materialized,
        panel,
        computed,
        panel_global,
        presentation_camera,
        width_constraint,
        mut placement,
    ) in &mut controllers
    {
        if materialized.space != PanelSpace::World {
            continue;
        }
        if tooltip_for.target() != materialized.target {
            set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Unavailable);
            continue;
        }
        let decision = match materialized.placement_policy {
            TooltipPlacementPolicy::Fixed
                if placement.attachment == materialized.authored_attachment =>
            {
                TooltipPlacementDecision::Fits
            },
            TooltipPlacementPolicy::Fixed => {
                TooltipPlacementDecision::Move(materialized.authored_attachment)
            },
            TooltipPlacementPolicy::KeepVisible => keep_visible_world_tooltip_placement(
                materialized,
                panel,
                computed,
                panel_global,
                presentation_camera,
                width_constraint,
                &mut placement,
                &panels,
                &widgets,
                &anchor_geometry,
                &cameras,
                &windows,
                &primary_window,
            ),
        };
        match decision {
            TooltipPlacementDecision::ConstrainWidth(width) => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Pending);
                commands
                    .entity(controller)
                    .insert(TooltipWidthConstraintRequest::Apply(width));
            },
            TooltipPlacementDecision::RestoreWidth => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Pending);
                commands
                    .entity(controller)
                    .insert(TooltipWidthConstraintRequest::Restore);
            },
            TooltipPlacementDecision::Move(attachment) => {
                set_tooltip_placement(&mut placement, attachment, TooltipPlacementResult::Fits);
                commands.entity(controller).insert((
                    crate::panel::PanelAttachmentAuthored::new(
                        materialized.target,
                        attachment.source_anchor(),
                        attachment.target_anchor(),
                    ),
                    attachment.offset(),
                ));
            },
            TooltipPlacementDecision::Fits => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Fits);
            },
            TooltipPlacementDecision::Unavailable => {
                set_tooltip_placement_result(&mut placement, TooltipPlacementResult::Unavailable);
            },
        }
    }
}

fn keep_visible_world_tooltip_placement(
    materialized: &MaterializedTooltip,
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    panel_global: &GlobalTransform,
    presentation_camera: Option<Ref<'_, TooltipPresentationCamera>>,
    width_constraint: Option<&TooltipWidthConstraint>,
    placement: &mut Mut<'_, TooltipPlacementState>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&super::WidgetOf, With<super::PanelWidget>>,
    anchor_geometry: &Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    cameras: &Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: &Query<Ref<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> TooltipPlacementDecision {
    let Some(camera_view) = presentation_camera.and_then(|input| {
        compatible_tooltip_camera(input, materialized, cameras, windows, primary_window)
    }) else {
        return TooltipPlacementDecision::Unavailable;
    };
    let Some(target) =
        world_tooltip_target(materialized, &camera_view, panels, widgets, anchor_geometry)
    else {
        return TooltipPlacementDecision::Unavailable;
    };
    let target_change = update_world_target_snapshot(placement, &target.snapshot());
    if width_constraint.is_some()
        && (camera_view.inputs_changed || target_change == WorldTargetChange::Changed)
    {
        return TooltipPlacementDecision::RestoreWidth;
    }
    let current_attachment = placement.attachment;
    let Some(source_bounds) = projected_world_tooltip_bounds(
        panel,
        panel_global,
        &target,
        current_attachment,
        &camera_view,
    ) else {
        return TooltipPlacementDecision::Unavailable;
    };
    let Some(usable_width) = usable_viewport_axis(camera_view.viewport.width()) else {
        return TooltipPlacementDecision::Unavailable;
    };
    if source_bounds.size().x > usable_width {
        let constrained_width = panel.width() * usable_width / source_bounds.size().x;
        return if constrained_width.is_finite()
            && constrained_width > 0.0
            && width_constraint.is_none_or(|current| constrained_width < current.0)
        {
            TooltipPlacementDecision::ConstrainWidth(constrained_width)
        } else {
            TooltipPlacementDecision::Unavailable
        };
    }
    if !tooltip_layout_fits_panel(panel, computed) {
        return TooltipPlacementDecision::Unavailable;
    }
    projected_world_tooltip_placement(
        materialized,
        &target,
        panel,
        panel_global,
        &camera_view,
        current_attachment,
    )
}

fn projected_world_tooltip_placement(
    materialized: &MaterializedTooltip,
    target: &WorldTooltipTarget,
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    camera: &TooltipCameraView<'_>,
    current_attachment: PanelAttachment,
) -> TooltipPlacementDecision {
    let authored = materialized.authored_attachment;
    let authored_layout = authored.offset().to_layout_units(target.layout_unit);
    let preferred = preferred_side(authored, authored_layout.truncate());
    for side in tooltip_side_order(preferred, target.bounds, camera.viewport) {
        let (source_anchor, target_anchor, layout_offset) =
            world_candidate_layout(authored_layout, authored, preferred, side);
        let attachment = panel_attachment(source_anchor, target_anchor, layout_offset);
        let Some(natural_bounds) =
            projected_world_tooltip_bounds(panel, panel_global, target, attachment, camera)
        else {
            continue;
        };
        let target_anchor_frame = target.anchor(target_anchor);
        let Some(target_point) = project_world_point(target_anchor_frame.position, camera) else {
            continue;
        };
        let viewport_shift = limited_along_edge_shift(
            side,
            natural_bounds.top_left(),
            natural_bounds.size(),
            target_point,
            camera.viewport,
        );
        let Some(layout_offset) = shifted_world_layout_offset(
            layout_offset,
            viewport_shift,
            target_anchor_frame,
            target,
            camera,
        ) else {
            continue;
        };
        let attachment = panel_attachment(source_anchor, target_anchor, layout_offset);
        let Some(bounds) =
            projected_world_tooltip_bounds(panel, panel_global, target, attachment, camera)
        else {
            continue;
        };
        if !bounds_fit_viewport(bounds.top_left(), bounds.size(), camera.viewport) {
            continue;
        }
        return if current_attachment == attachment {
            TooltipPlacementDecision::Fits
        } else {
            TooltipPlacementDecision::Move(attachment)
        };
    }
    TooltipPlacementDecision::Unavailable
}

fn world_candidate_layout(
    authored_layout: Vec3,
    authored: PanelAttachment,
    preferred: TooltipSide,
    side: TooltipSide,
) -> (Anchor, Anchor, Vec3) {
    if side == preferred {
        return (
            authored.source_anchor(),
            authored.target_anchor(),
            authored_layout,
        );
    }
    let (source_anchor, target_anchor) = side.anchors();
    let gap = if preferred.is_vertical() {
        authored_layout.y.abs()
    } else {
        authored_layout.x.abs()
    };
    let layout_offset = match side {
        TooltipSide::Above => Vec3::new(0.0, -gap, authored_layout.z),
        TooltipSide::Right => Vec3::new(gap, 0.0, authored_layout.z),
        TooltipSide::Below => Vec3::new(0.0, gap, authored_layout.z),
        TooltipSide::Left => Vec3::new(-gap, 0.0, authored_layout.z),
    };
    (source_anchor, target_anchor, layout_offset)
}

fn panel_attachment(
    source_anchor: Anchor,
    target_anchor: Anchor,
    layout_offset: Vec3,
) -> PanelAttachment {
    PanelAttachment::new(source_anchor, target_anchor).with_offset(
        PanelAnchorOffset::new(layout_offset.x, layout_offset.y).with_z(layout_offset.z),
    )
}

fn compatible_tooltip_camera<'a>(
    input: Ref<'_, TooltipPresentationCamera>,
    materialized: &MaterializedTooltip,
    cameras: &'a Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: &Query<Ref<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<TooltipCameraView<'a>> {
    let (camera, global_transform, render_target, camera_layers) =
        cameras.get(input.camera).ok()?;
    let inputs_changed = input.is_changed()
        || camera.is_changed()
        || global_transform.is_changed()
        || render_target
            .as_ref()
            .is_some_and(bevy::ecs::change_detection::DetectChanges::is_changed)
        || camera_layers
            .as_ref()
            .is_some_and(bevy::ecs::change_detection::DetectChanges::is_changed);
    let camera = camera.into_inner();
    let global_transform = global_transform.into_inner();
    if !camera.is_active {
        return None;
    }
    let primary = primary_window.single().ok();
    let window = match render_target?.into_inner().normalize(primary)? {
        NormalizedRenderTarget::Window(window) => window.entity(),
        NormalizedRenderTarget::Image(_)
        | NormalizedRenderTarget::TextureView(_)
        | NormalizedRenderTarget::None { .. } => return None,
    };
    let window = windows.get(window).ok()?;
    let inputs_changed = inputs_changed || window.is_changed();
    let camera_layers = camera_layers.map_or_else(
        || RenderLayers::layer(0),
        |render_layers| render_layers.into_inner().clone(),
    );
    if !camera_layers.intersects(&materialized.render_layers) {
        return None;
    }
    let viewport = camera
        .logical_viewport_rect()
        .unwrap_or_else(|| zero_origin_viewport(Vec2::new(window.width(), window.height())));
    valid_screen_size(viewport.size()).then_some(TooltipCameraView {
        camera,
        global_transform,
        viewport,
        inputs_changed,
    })
}

fn projected_world_tooltip_bounds(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    camera: &TooltipCameraView<'_>,
) -> Option<PanelScreenBounds> {
    let source_scale = panel_global.to_scale_rotation_translation().0;
    if !source_scale.is_finite() {
        return None;
    }
    let target_anchor = target.anchor(attachment.target_anchor());
    let layout_offset = attachment.offset().to_layout_units(target.layout_unit);
    let source_anchor = panel_local_anchor(panel, attachment.source_anchor());
    let target_position = target_anchor.position
        + target_anchor.right * (layout_offset.x * target.world_per_layout_unit.x)
        - target_anchor.up * (layout_offset.y * target.world_per_layout_unit.y)
        + target_anchor.normal * (layout_offset.z * target.world_per_layout_unit_z);
    let corners = [
        panel_local_anchor(panel, Anchor::TopLeft),
        panel_local_anchor(panel, Anchor::TopRight),
        panel_local_anchor(panel, Anchor::BottomRight),
        panel_local_anchor(panel, Anchor::BottomLeft),
    ];
    projected_points_bounds(
        corners.map(|corner| {
            let offset = source_scale * (corner - source_anchor);
            target_position
                + target_anchor.right * offset.x
                + target_anchor.up * offset.y
                + target_anchor.normal * offset.z
        }),
        camera,
    )
}

fn panel_local_anchor(panel: &DiegeticPanel, anchor: Anchor) -> Vec3 {
    let size = Vec2::new(panel.world_width(), panel.world_height());
    let panel_anchor = Vec2::from(panel.anchor().offset(size.x, size.y));
    let anchor = Vec2::from(anchor.offset(size.x, size.y));
    Vec3::new(anchor.x - panel_anchor.x, panel_anchor.y - anchor.y, 0.0)
}

fn project_world_point(point: Vec3, camera: &TooltipCameraView<'_>) -> Option<Vec2> {
    camera
        .camera
        .world_to_viewport(camera.global_transform, point)
        .ok()
}

fn projected_points_bounds<const N: usize>(
    points: [Vec3; N],
    camera: &TooltipCameraView<'_>,
) -> Option<PanelScreenBounds> {
    let mut minimum = Vec2::splat(f32::INFINITY);
    let mut maximum = Vec2::splat(f32::NEG_INFINITY);
    for point in points {
        let projected = camera
            .camera
            .world_to_viewport(camera.global_transform, point)
            .ok()?;
        minimum = minimum.min(projected);
        maximum = maximum.max(projected);
    }
    PanelScreenBounds::new(minimum, maximum - minimum).ok()
}

fn world_tooltip_target(
    materialized: &MaterializedTooltip,
    camera: &TooltipCameraView<'_>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&super::WidgetOf, With<super::PanelWidget>>,
    anchor_geometry: &Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
) -> Option<WorldTooltipTarget> {
    if let Ok((panel, global_transform)) = panels.get(materialized.target) {
        let plane = crate::panel::PanelPlane::from_panel(panel, global_transform).ok()?;
        let anchors = world_panel_anchor_frames(plane);
        let world_per_layout_unit = Vec2::new(
            plane.size().x / panel.width(),
            plane.size().y / panel.height(),
        );
        return Some(WorldTooltipTarget {
            bounds: projected_world_anchors(&anchors, camera)?,
            anchors,
            layout_unit: panel.layout_unit(),
            world_per_layout_unit,
            world_per_layout_unit_z: world_per_layout_unit.x,
        });
    }
    let (geometry, global_transform) = anchor_geometry.get(materialized.target).ok()?;
    let anchors = world_geometry_anchor_frames(geometry, global_transform)?;
    let world_per_layout_unit = widgets
        .get(materialized.target)
        .ok()
        .and_then(|widget_of| panels.get(widget_of.panel()).ok())
        .and_then(|(panel, global_transform)| {
            let plane = crate::panel::PanelPlane::from_panel(panel, global_transform).ok()?;
            Some(Vec2::new(
                plane.size().x / panel.width(),
                plane.size().y / panel.height(),
            ))
        })
        .unwrap_or_else(|| Vec2::splat(materialized.layout_unit.meters_per_unit()));
    valid_screen_scale(world_per_layout_unit).then_some(WorldTooltipTarget {
        bounds: projected_world_anchors(&anchors, camera)?,
        anchors,
        layout_unit: materialized.layout_unit,
        world_per_layout_unit,
        world_per_layout_unit_z: world_per_layout_unit.x,
    })
}

fn world_panel_anchor_frames(plane: PanelPlane) -> [WorldAnchorFrame; QUAD_ANCHOR_COUNT] {
    all_anchors().map(|anchor| WorldAnchorFrame {
        position: plane.point(anchor),
        right:    plane.right(),
        up:       plane.up(),
        normal:   plane.normal(),
    })
}

fn world_geometry_anchor_frames(
    geometry: &ResolvedAnchorGeometry,
    global_transform: &GlobalTransform,
) -> Option<[WorldAnchorFrame; QUAD_ANCHOR_COUNT]> {
    let rotation = global_transform.rotation();
    all_anchors()
        .map(|anchor| {
            let frame = geometry.frame(AnchorSite::from(anchor)).ok()?;
            let orientation = rotation * frame.orientation().into_inner();
            Some(WorldAnchorFrame {
                position: global_transform.transform_point(frame.position().into_inner()),
                right:    orientation * Vec3::X,
                up:       orientation * Vec3::Y,
                normal:   orientation * Vec3::Z,
            })
        })
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

fn projected_world_anchors(
    anchors: &[WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    camera: &TooltipCameraView<'_>,
) -> Option<PanelScreenBounds> {
    projected_points_bounds(anchors.map(|anchor| anchor.position), camera)
}

const fn all_anchors() -> [Anchor; QUAD_ANCHOR_COUNT] {
    [
        Anchor::TopLeft,
        Anchor::TopCenter,
        Anchor::TopRight,
        Anchor::CenterLeft,
        Anchor::Center,
        Anchor::CenterRight,
        Anchor::BottomLeft,
        Anchor::BottomCenter,
        Anchor::BottomRight,
    ]
}

const fn anchor_index(anchor: Anchor) -> usize {
    match anchor {
        Anchor::TopLeft => 0,
        Anchor::TopCenter => 1,
        Anchor::TopRight => 2,
        Anchor::CenterLeft => 3,
        Anchor::Center => 4,
        Anchor::CenterRight => 5,
        Anchor::BottomLeft => 6,
        Anchor::BottomCenter => 7,
        Anchor::BottomRight => 8,
    }
}

fn shifted_world_layout_offset(
    layout_offset: Vec3,
    viewport_shift: Vec2,
    anchor: WorldAnchorFrame,
    target: &WorldTooltipTarget,
    camera: &TooltipCameraView<'_>,
) -> Option<Vec3> {
    let depth_offset = layout_offset.z * target.world_per_layout_unit_z;
    let plane_origin = anchor.position + anchor.normal * depth_offset;
    let source_anchor_position = plane_origin
        + anchor.right * (layout_offset.x * target.world_per_layout_unit.x)
        - anchor.up * (layout_offset.y * target.world_per_layout_unit.y);
    let viewport_anchor = project_world_point(source_anchor_position, camera)?;
    let ray = camera
        .camera
        .viewport_to_world(camera.global_transform, viewport_anchor + viewport_shift)
        .ok()?;
    let denominator = ray.direction.dot(anchor.normal);
    if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
        return None;
    }
    let distance = (plane_origin - ray.origin).dot(anchor.normal) / denominator;
    if !distance.is_finite() || distance < 0.0 {
        return None;
    }
    let point = ray.get_point(distance);
    let delta = point - plane_origin;
    Some(Vec3::new(
        delta.dot(anchor.right) / target.world_per_layout_unit.x,
        -delta.dot(anchor.up) / target.world_per_layout_unit.y,
        layout_offset.z,
    ))
}

pub(super) fn finalize_tooltip_readiness(
    mut controllers: Query<(
        Entity,
        &TooltipFor,
        &MaterializedTooltip,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &TooltipPlacementState,
        &GlobalTransform,
        &mut TooltipReadiness,
        Option<&ResolvedScreenPanelPosition>,
        Option<&AnchoredTo>,
        Option<&PanelAttachmentAuthored>,
    )>,
    panels: Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: Query<(&super::WidgetOf, &super::WidgetAnchorRect), With<super::PanelWidget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    screen_diagnostics: Option<Res<ScreenAttachmentResolveDiagnostics>>,
    world_diagnostics: Option<Res<AnchorResolveDiagnostics>>,
) {
    for (
        entity,
        tooltip_for,
        materialized,
        panel,
        computed,
        placement,
        global_transform,
        mut readiness,
        screen_position,
        world_attachment,
        authored_attachment,
    ) in &mut controllers
    {
        let placement_ready = materialized.placement_policy == TooltipPlacementPolicy::Fixed
            || placement.result == TooltipPlacementResult::Fits;
        let target_matches = authored_attachment
            .is_some_and(|attachment| attachment.target() == materialized.target)
            && tooltip_for.target() == materialized.target;
        let layout_ready = computed.is_solved();
        let resolved = match materialized.space {
            PanelSpace::World => world_attachment.is_some_and(|attachment| {
                let Some(world_diagnostics) = world_diagnostics.as_deref() else {
                    return false;
                };
                panel::world_attachment_is_ready(
                    entity,
                    materialized.target,
                    attachment,
                    world_diagnostics,
                )
            }),
            PanelSpace::Screen => screen_position.is_some_and(|position| {
                let Some(screen_diagnostics) = screen_diagnostics.as_deref() else {
                    return false;
                };
                screen_space::screen_attachment_is_ready(entity, position, screen_diagnostics)
            }),
        };
        let presentation_matches =
            materialized_panel_presentation_matches(panel, materialized, &windows, &primary_window);
        let target_presentation_matches = match materialized.space {
            PanelSpace::World => true,
            PanelSpace::Screen => screen_tooltip_target(
                materialized.target,
                &panels,
                &widgets,
                &screen_targets,
                &windows,
                &primary_window,
            )
            .is_some_and(|target| target.presentation_matches(materialized)),
        };
        let next = if layout_ready
            && placement_ready
            && target_matches
            && resolved
            && presentation_matches
            && target_presentation_matches
            && global_transform.affine().is_finite()
        {
            TooltipReadiness::Ready
        } else {
            TooltipReadiness::Pending
        };
        if *readiness != next {
            *readiness = next;
        }
    }
}

fn materialized_panel_presentation_matches(
    panel: &DiegeticPanel,
    materialized: &MaterializedTooltip,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> bool {
    if panel.layout_unit() != materialized.layout_unit {
        return false;
    }
    match panel.coordinate_space() {
        CoordinateSpace::World { .. } => {
            materialized.space == PanelSpace::World
                && materialized.window.is_none()
                && materialized.camera_order.is_none()
        },
        CoordinateSpace::Screen {
            camera_order,
            render_layers,
            window,
            ..
        } => {
            materialized.space == PanelSpace::Screen
                && live_window(*window, windows, primary_window) == materialized.window
                && materialized.camera_order == Some(*camera_order)
                && materialized.render_layers == *render_layers
        },
    }
}

pub(crate) fn remove_materialized_state(
    entity: &mut EntityCommands<'_>,
    materialized: &MaterializedTooltip,
) {
    let previous_transform = materialized.previous_transform;
    let previous_global = materialized.previous_global;
    let previous_visibility = materialized.previous_visibility;
    entity.remove::<(
        MaterializedTooltip,
        TooltipReadiness,
        TooltipPlacementState,
        TooltipWidthConstraint,
        TooltipWidthConstraintRequest,
        TooltipPresentationCamera,
        TooltipShownPending,
        ScreenTooltipAttachmentCorrection,
        PrepareTooltip,
    )>();
    entity.remove::<(Transform, GlobalTransform, Visibility)>();
    if let Some(transform) = previous_transform {
        entity.insert(transform);
    }
    if let Some(global_transform) = previous_global {
        entity.insert(global_transform);
    }
    if let Some(visibility) = previous_visibility {
        entity.insert(visibility);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComputedTooltipRecord {
    widget_id: crate::PanelElementId,
    tooltip:   Tooltip,
}

impl ComputedTooltipRecord {
    pub(crate) const fn new(widget_id: crate::PanelElementId, tooltip: Tooltip) -> Self {
        Self { widget_id, tooltip }
    }

    pub(crate) const fn widget_id(&self) -> &crate::PanelElementId { &self.widget_id }

    pub(crate) const fn tooltip(&self) -> &Tooltip { &self.tooltip }
}

#[derive(Component, Default)]
pub(crate) struct TooltipControllerIndex(HashMap<crate::PanelElementId, Entity>);

impl TooltipControllerIndex {
    pub(crate) fn entity(&self, id: &crate::PanelElementId) -> Option<Entity> {
        self.0.get(id).copied()
    }

    pub(crate) fn entities(&self) -> impl Iterator<Item = Entity> + '_ { self.0.values().copied() }

    pub(crate) fn replace(&mut self, index: HashMap<crate::PanelElementId, Entity>) {
        self.0 = index;
    }
}

mod private {
    use crate::panel::Screen;
    pub trait SealedSpace {}

    impl SealedSpace for crate::panel::World {}

    impl SealedSpace for Screen {}
}

#[cfg(test)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTargetInfo;
    use bevy::camera::Viewport;
    use bevy::ecs::error;
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
    use bevy::picking::pointer::update_pointer_map;
    use bevy::prelude::DetectChangesMut;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PointerButton;
    use bevy::prelude::Vec3A;
    use bevy::time::TimeUpdateStrategy;
    use bevy::transform::TransformPlugin;
    use hana_valence::AnchoredTo;
    use hana_valence::ResolvedAnchorOffset;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelDefaults;
    use crate::PanelElementId;
    use crate::PanelPicking;
    use crate::PanelWidgetReader;
    use crate::Pt;
    use crate::Slider;
    use crate::TextStyle;
    use crate::TextWrap;
    use crate::WidgetOf;
    use crate::layout::El;
    use crate::layout::LayoutTreeChange;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::ResolvedScreenPanelPosition;
    use crate::screen_space::ScreenAttachmentResolveDiagnostics;
    use crate::screen_space::ScreenPanelRect;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetFocusAuthority;
    use crate::widgets::WidgetSpec;
    use crate::widgets::WidgetsPlugin;

    const DISTANT_WORLD_TARGET_Z: f32 = -45.0;
    const GEOMETRY_EPSILON: f32 = 1e-4;
    const GROWN_SCREEN_VIEWPORT: Vec2 = Vec2::new(520.0, 180.0);
    const MATERIALIZATION_UPDATES: usize = 5;
    const NARROW_WORLD_VIEWPORT: Vec2 = Vec2::new(96.0, 180.0);
    const OFFSET_VIEWPORT_ORIGIN: Vec2 = Vec2::new(180.0, 90.0);
    const OFFSET_VIEWPORT_SIZE: Vec2 = Vec2::new(240.0, 160.0);
    const ROTATED_TARGET_ANGLE: f32 = -std::f32::consts::FRAC_PI_4;
    const SMALL_SCREEN_VIEWPORT: Vec2 = Vec2::new(240.0, 180.0);
    const TEST_CAMERA_DISTANCE: f32 = 5.0;
    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);
    const WIDE_WORLD_VIEWPORT: Vec2 = Vec2::new(800.0, 180.0);

    struct ApplicationWorldTarget(Entity);

    #[derive(Default, Resource)]
    struct TooltipCleanupCount(usize);

    #[derive(Default, Resource)]
    struct TooltipVisibilityLog {
        shown:  Vec<Entity>,
        hidden: Vec<Entity>,
    }

    impl TooltipTarget for ApplicationWorldTarget {
        type Space = World;

        fn tooltip_target_entity(&self) -> Entity { self.0 }
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

    fn add_synthetic_pointer(app: &mut App, pointer: PointerId, location: Location) {
        app.world_mut()
            .spawn((pointer, PointerLocation::new(location)));
        let result = app.world_mut().run_system_cached(update_pointer_map);
        assert!(result.is_ok());
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

    fn seeded_perspective_camera_with_offset_viewport() -> Camera {
        let mut camera = seeded_perspective_camera_for(TEST_VIEWPORT);
        camera.viewport = Some(Viewport {
            physical_position: OFFSET_VIEWPORT_ORIGIN.as_uvec2(),
            physical_size: OFFSET_VIEWPORT_SIZE.as_uvec2(),
            ..Default::default()
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(OFFSET_VIEWPORT_SIZE.x, OFFSET_VIEWPORT_SIZE.y);
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

    fn set_presentation_viewport(app: &mut App, camera: Entity, window: Entity, viewport: Vec2) {
        if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
            window.resolution.set(viewport.x, viewport.y);
        }
        if let Some(mut camera) = app.world_mut().get_mut::<Camera>(camera) {
            *camera = seeded_perspective_camera_for(viewport);
        }
    }

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    fn screen_tooltip_target(bounds: PanelScreenBounds) -> ScreenTooltipTarget {
        let target = crate::screen_space::ScreenAnchorTarget::new(
            bounds,
            Entity::PLACEHOLDER,
            0,
            RenderLayers::layer(0),
            Unit::Pixels,
        );
        let rect = crate::screen_space::ScreenPanelRect::from_screen_target(&target);
        ScreenTooltipTarget {
            bounds,
            rect,
            window: Entity::PLACEHOLDER,
            camera_order: 0,
            render_layers: RenderLayers::layer(0),
            layout_unit: Unit::Pixels,
            layout_scale: Vec2::ONE,
        }
    }

    fn screen_tooltip_source(size: Vec2) -> Option<ScreenPanelRect> {
        let bounds = PanelScreenBounds::new(Vec2::ZERO, size).ok()?;
        Some(screen_tooltip_target(bounds).rect)
    }

    fn resolved_screen_tooltip_bounds(
        app: &App,
        controller: Entity,
        window: Entity,
    ) -> Option<PanelScreenBounds> {
        let panel = app.world().get::<DiegeticPanel>(controller)?;
        let transform = app.world().get::<Transform>(controller)?;
        let resolved = app.world().get::<ResolvedScreenPanelPosition>(controller)?;
        let window = app.world().get::<Window>(window)?;
        crate::screen_space::screen_panel_rect(
            panel,
            Some(resolved),
            Some(transform),
            Vec2::new(window.width(), window.height()),
        )?
        .projected_bounds()
    }

    fn projected_world_panel_bounds(
        app: &App,
        panel_entity: Entity,
        camera_entity: Entity,
    ) -> Option<PanelScreenBounds> {
        let panel = app.world().get::<DiegeticPanel>(panel_entity)?;
        let panel_global = app.world().get::<GlobalTransform>(panel_entity)?;
        let camera = app.world().get::<Camera>(camera_entity)?;
        let camera_global = app.world().get::<GlobalTransform>(camera_entity)?;
        let viewport = camera.logical_viewport_rect()?;
        let camera_view = TooltipCameraView {
            camera,
            global_transform: camera_global,
            viewport,
            inputs_changed: false,
        };
        let plane = crate::panel::PanelPlane::from_panel(panel, panel_global).ok()?;
        projected_points_bounds(
            [
                plane.point(Anchor::TopLeft),
                plane.point(Anchor::TopRight),
                plane.point(Anchor::BottomRight),
                plane.point(Anchor::BottomLeft),
            ],
            &camera_view,
        )
    }

    fn tooltip_tree(tooltip: Option<Tooltip>) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let button = El::new().button("action");
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

    fn tooltip_target_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(50.0, 20.0).button("action"), |_| {});
        builder.build()
    }

    fn slider_tooltip_tree(slider: Slider, tooltip: Option<Tooltip>) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let slider = El::new().widget("action", slider);
        match tooltip {
            Some(tooltip) => builder.with(slider.tooltip(tooltip), |_| {}),
            None => builder.with(slider, |_| {}),
        };
        builder.build()
    }

    fn editable_tooltip_tree(tooltip: Tooltip) -> crate::LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().editable_field("action", field).tooltip(tooltip),
            |_| {},
        );
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
            .get::<super::super::Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(controller, Entity::PLACEHOLDER);
        controller
    }

    #[test]
    fn editable_field_tooltip_reifies_against_its_widget_entity() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, editable_tooltip_tree(Tooltip::new(El::new())));
        app.update();
        let field = widget(&mut app, panel);
        let tooltip = controller(&app, field);

        assert_eq!(
            app.world()
                .get::<TooltipFor>(tooltip)
                .map(TooltipFor::target),
            Some(field),
        );
    }

    fn count_tooltip_cleanup(
        _despawn: On<Despawn, Tooltip>,
        mut cleanup_count: ResMut<TooltipCleanupCount>,
    ) {
        cleanup_count.0 += 1;
    }

    fn center_anchor_geometry() -> ResolvedAnchorGeometry {
        let frame = AnchorFrame::try_new(Position::from(Vec3::ZERO), Orientation::default());
        assert!(
            frame.is_ok(),
            "the center anchor uses finite identity geometry"
        );
        let geometry = ResolvedAnchorGeometry::try_new(
            [(AnchorSite::Center, frame.unwrap_or_default())],
            Vec::<Edge>::new(),
        );
        assert!(
            geometry.is_ok(),
            "a single center anchor has no invalid edges"
        );
        geometry.unwrap_or_default()
    }

    fn demand_mesh_target(app: &mut App, target: Entity) -> Entity {
        app.world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id()
    }

    fn settle_mesh_anchor_geometry(app: &mut App) {
        app.update();
        app.update();
    }

    fn mesh_anchor_center(app: &App, target: Entity) -> Option<Vec3> {
        app.world()
            .get::<ResolvedAnchorGeometry>(target)
            .and_then(|geometry| geometry.frame(AnchorSite::Center).ok())
            .map(|point| point.position().into_inner())
    }

    #[track_caller]
    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, GEOMETRY_EPSILON),
            "expected {expected:?}, got {actual:?}",
        );
    }

    #[track_caller]
    fn assert_quat_close(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, GEOMETRY_EPSILON)
                || actual.abs_diff_eq(-expected, GEOMETRY_EPSILON),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn assert_lightweight_controller(app: &App, widget: Entity, controller: Entity) {
        assert_eq!(
            app.world()
                .get::<super::super::Tooltips>(widget)
                .map(|tooltips| tooltips.iter().collect::<Vec<_>>()),
            Some(vec![controller])
        );
        assert!(app.world().get::<AnchoredHere>(widget).is_none());
        assert!(
            app.world()
                .get::<super::super::ScreenWidgetAnchoredHere>(widget)
                .is_none()
        );
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::ComputedDiegeticPanel>(controller)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::PanelTextRuns>(controller)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(
            app.world()
                .get::<super::super::ScreenWidgetAnchoredTo>(controller)
                .is_none()
        );
        assert!(app.world().get::<Aabb>(controller).is_none());
        assert!(
            app.world()
                .get::<ResolvedAnchorGeometry>(controller)
                .is_none()
        );
        assert_eq!(
            app.world()
                .get::<TooltipFor>(controller)
                .map(TooltipFor::target),
            Some(widget)
        );
    }

    fn assert_positive_z_mesh_geometry(geometry: &ResolvedAnchorGeometry) {
        assert_eq!(geometry.frames().count(), QUAD_ANCHOR_COUNT);
        for (anchor, expected) in [
            (Anchor::TopLeft, Vec3::new(-1.0, 2.0, 3.0)),
            (Anchor::TopRight, Vec3::new(1.0, 2.0, 3.0)),
            (Anchor::BottomRight, Vec3::new(1.0, -2.0, 3.0)),
            (Anchor::BottomLeft, Vec3::new(-1.0, -2.0, 3.0)),
            (Anchor::TopCenter, Vec3::new(0.0, 2.0, 3.0)),
            (Anchor::CenterRight, Vec3::new(1.0, 0.0, 3.0)),
            (Anchor::BottomCenter, Vec3::new(0.0, -2.0, 3.0)),
            (Anchor::CenterLeft, Vec3::new(-1.0, 0.0, 3.0)),
            (Anchor::Center, Vec3::new(0.0, 0.0, 3.0)),
        ] {
            let point = geometry.frame(AnchorSite::from(anchor));
            assert!(
                point.is_ok(),
                "mesh geometry contains every named quad anchor"
            );
            let Some(point) = point.ok() else {
                continue;
            };
            assert_vec3_close(point.position().into_inner(), expected);
            assert_quat_close(point.orientation().into_inner(), Quat::IDENTITY);
        }
        assert_eq!(
            geometry.edges(),
            &[
                Edge {
                    start: AnchorSite::from(Anchor::TopLeft),
                    end:   AnchorSite::from(Anchor::TopRight),
                },
                Edge {
                    start: AnchorSite::from(Anchor::TopRight),
                    end:   AnchorSite::from(Anchor::BottomRight),
                },
                Edge {
                    start: AnchorSite::from(Anchor::BottomRight),
                    end:   AnchorSite::from(Anchor::BottomLeft),
                },
                Edge {
                    start: AnchorSite::from(Anchor::BottomLeft),
                    end:   AnchorSite::from(Anchor::TopLeft),
                },
            ]
        );
    }

    fn unmaterialized_blueprint(
        app: &App,
        target: Entity,
        controller: Entity,
    ) -> Option<Arc<LayoutTree>> {
        let blueprint = app
            .world()
            .get::<Tooltip>(controller)
            .map(|tooltip| Arc::clone(tooltip.blueprint()));
        assert!(blueprint.is_some());
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(app.world().get::<MaterializedTooltip>(controller).is_none());
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(app.world().get::<AnchoredHere>(target).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        blueprint
    }

    fn assert_materialized_world_tooltip(
        app: &App,
        target: Entity,
        controller: Entity,
        blueprint: &Arc<LayoutTree>,
    ) -> Option<u64> {
        let materialized = app.world().get::<MaterializedTooltip>(controller);
        assert!(materialized.is_some());
        if let Some(materialized) = materialized {
            assert_eq!(materialized.target(), target);
            assert_eq!(materialized.space(), PanelSpace::World);
            assert_eq!(materialized.layout_unit(), Unit::Millimeters);
            assert!(Arc::ptr_eq(&materialized.blueprint, blueprint));
        }
        let panel = app.world().get::<DiegeticPanel>(controller);
        assert!(panel.is_some());
        if let Some(panel) = panel {
            assert_eq!(panel.layout_unit(), Unit::Millimeters);
            assert!(!std::ptr::eq(panel.tree(), blueprint.as_ref()));
        }
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<PanelPicking>(controller),
            Some(&PanelPicking::PASS_THROUGH)
        );
        assert_eq!(
            app.world()
                .get::<AnchoredTo>(controller)
                .map(AnchoredTo::target),
            Some(target)
        );
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(target)
                .map(AnchoredHere::len),
            Some(1)
        );
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        panel.map(|panel| u64::from(panel.tree_revision()))
    }

    #[test]
    fn defaults_and_clone_identity_match_the_public_contract() {
        let tooltip = Tooltip::new(El::new());
        let clone = tooltip.clone();
        let policy = clone.clone().show_after(Duration::from_secs(1));

        assert_eq!(tooltip, clone);
        assert!(Arc::ptr_eq(&tooltip.blueprint, &clone.blueprint));
        assert!(Arc::ptr_eq(&tooltip.blueprint, &policy.blueprint));
        assert_eq!(tooltip.show_after, DEFAULT_SHOW_DELAY);
        assert_eq!(tooltip.hide_after, Duration::ZERO);
        assert_eq!(tooltip.disabled_policy, TooltipDisabledPolicy::Suppress);
        assert_eq!(tooltip.source_anchor, Anchor::TopCenter);
        assert_eq!(tooltip.target_anchor, Anchor::BottomCenter);
        assert_eq!(
            tooltip.offset,
            PanelAnchorOffset::new(Px(0.0), Px(DEFAULT_TOOLTIP_GAP))
        );
        assert_eq!(
            tooltip.placement_policy,
            TooltipPlacementPolicy::KeepVisible
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
        assert!(
            app.world()
                .resource::<TooltipVisibilityLog>()
                .shown
                .is_empty()
        );

        set_interaction(&mut app, target, PickingInteraction::Hovered);
        for _ in 0..3 {
            app.update();
        }
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Visible)
        ));
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
        assert!(
            app.world()
                .resource::<TooltipVisibilityLog>()
                .hidden
                .is_empty()
        );

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
        assert!(
            app.world()
                .resource::<TooltipVisibilityLog>()
                .shown
                .is_empty()
        );

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
            .insert(super::super::WidgetDisabled::test_marker());
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
                space: PanelSpace::World,
                layout_unit: Unit::Millimeters,
                window: None,
                camera_order: None,
                render_layers: RenderLayers::layer(0),
                blueprint: Arc::new(LayoutBuilder::new(1.0, 1.0).build()),
                authored_attachment: PanelAttachment::new(Anchor::Center, Anchor::Center),
                placement_policy: TooltipPlacementPolicy::Fixed,
                previous_transform: None,
                previous_global: None,
                previous_visibility: None,
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

        let tooltip = Tooltip::new(El::new())
            .show_after(Duration::ZERO)
            .placement_policy(TooltipPlacementPolicy::Fixed);
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
        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Visible)
        ));
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
        let target = mesh_target(&mut app, Vec3A::splat(1.0));
        let controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(30.0, 10.0))
                .show_after(Duration::ZERO)
                .hide_after(Duration::ZERO)
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();

        app.world_mut()
            .entity_mut(target)
            .insert(PickingInteraction::Hovered);
        update_materialization(&mut app);

        assert!(matches!(
            app.world().get::<TooltipPhase>(controller),
            Some(TooltipPhase::Visible)
        ));
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
        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location.clone(),
            Press {
                button: PointerButton::Primary,
                hit:    HitData::new(lower, 0.0, None, None),
                count:  1,
            },
            widget,
        ));
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

        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            location,
            Press {
                button: PointerButton::Primary,
                hit:    HitData::new(higher, 0.0, None, None),
                count:  1,
            },
            widget,
        ));
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
    fn blueprint_mutation_uses_copy_on_write() {
        let tooltip = Tooltip::new(El::new());
        let mut changed = tooltip.clone();
        assert!(Arc::ptr_eq(&tooltip.blueprint, &changed.blueprint));

        changed.text("details");

        assert_ne!(tooltip, changed);
        assert!(!Arc::ptr_eq(&tooltip.blueprint, &changed.blueprint));
        assert_eq!(tooltip.tree().len(), 1);
        assert_eq!(changed.tree().len(), 2);
    }

    #[test]
    fn clone_captured_during_nested_authoring_resumes_at_the_root() {
        let mut tooltip = Tooltip::new(El::column());
        let mut captured = None;
        tooltip.with(El::row(), |tooltip| {
            tooltip.text("nested");
            captured = Some(tooltip.clone());
        });
        assert!(captured.is_some());
        let Some(mut captured) = captured else {
            return;
        };

        captured.text("captured root child");
        tooltip.text("original root child");

        let mut expected_captured = LayoutBuilder::with_root(El::column());
        expected_captured.with(El::row(), |builder| {
            builder.text("nested");
        });
        expected_captured.text("captured root child");
        let expected_captured = expected_captured.build();

        let mut expected_original = LayoutBuilder::with_root(El::column());
        expected_original.with(El::row(), |builder| {
            builder.text("nested");
        });
        expected_original.text("original root child");
        let expected_original = expected_original.build();

        assert_eq!(
            captured.tree().classify_change(&expected_captured),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            tooltip.tree().classify_change(&expected_original),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            captured.tree().element_id(2),
            Some(&PanelElementId::auto(0))
        );
        assert_eq!(
            captured.tree().element_id(3),
            Some(&PanelElementId::auto(1))
        );
        assert!(!Arc::ptr_eq(&tooltip.blueprint, &captured.blueprint));
    }

    #[test]
    fn nested_authoring_restores_the_parent_cursor() {
        let mut tooltip = Tooltip::new(El::column());
        tooltip.with(El::row(), |tooltip| {
            tooltip.text("nested");
        });
        tooltip.text("root child");

        assert_eq!(tooltip.tree().len(), 4);
        assert_eq!(tooltip.tree().field_display_text(1), Some("nested"));
        assert_eq!(tooltip.tree().element_text(2), Some("nested"));
        assert_eq!(tooltip.tree().element_text(3), Some("root child"));
    }

    #[test]
    fn tooltip_visual_authoring_matches_an_ordinary_layout_tree() {
        let root = El::column()
            .gap(Px(3.0))
            .background(Color::BLACK)
            .corner_radius(4.0);
        let row = El::row()
            .gap(Px(2.0))
            .background(Color::srgb(0.1, 0.2, 0.3));
        let overlay = El::overlay()
            .size(Px(18.0), Px(12.0))
            .background(Color::srgb(0.3, 0.2, 0.1));
        let image = El::new().size(Px(8.0), Px(6.0)).corner_radius(2.0);
        let text_style = crate::TextStyle::new(Px(11.0)).with_color(Color::srgb(0.8, 0.9, 1.0));
        let image_handle = Handle::<Image>::default();
        let image_tint = Color::srgb(0.7, 0.8, 0.9);

        let mut tooltip = Tooltip::new(root.clone());
        tooltip.with(row.clone(), |tooltip| {
            tooltip.text(Text::new("details", text_style.clone()));
            tooltip.with(overlay.clone(), |tooltip| {
                tooltip.image(image.clone(), image_handle.clone(), image_tint);
            });
        });

        let mut builder = LayoutBuilder::with_root(root);
        builder.with(row, |builder| {
            builder.text(Text::new("details", text_style));
            builder.with(overlay, |builder| {
                builder.image(image, image_handle, image_tint);
            });
        });
        let ordinary = builder.build();

        assert_eq!(
            tooltip.tree().classify_change(&ordinary),
            LayoutTreeChange::Identical
        );
        assert_eq!(
            ordinary.classify_change(tooltip.tree()),
            LayoutTreeChange::Identical
        );
    }

    fn replace_associated_tooltip(
        app: &mut App,
        panel: Entity,
        widget: Entity,
        previous_controller: Entity,
        baseline: &LayoutTree,
        replacement: Tooltip,
    ) -> Entity {
        let replacement_tree = tooltip_tree(Some(replacement.clone()));
        assert_eq!(
            baseline.classify_change(&replacement_tree),
            LayoutTreeChange::VisualOnly
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, replacement_tree)
                .is_ok()
        );
        app.update();
        let replacement_controller = controller(app, widget);
        assert_ne!(replacement_controller, previous_controller);
        assert!(app.world().get_entity(previous_controller).is_err());
        assert_eq!(
            app.world().get::<Tooltip>(replacement_controller),
            Some(&replacement)
        );
        replacement_controller
    }

    #[test]
    fn associated_controller_replaces_identity_for_changed_declarations() {
        let mut app = test_app();
        let tooltip = Tooltip::new(El::new());
        let tree = tooltip_tree(Some(tooltip.clone()));
        let tree_without_tooltip = tooltip_tree(None);
        assert_eq!(
            tree_without_tooltip.classify_change(&tree),
            LayoutTreeChange::VisualOnly
        );
        let panel = spawn_panel(&mut app, tree_without_tooltip);
        app.update();
        let widget = widget(&mut app, panel);
        let button_declaration = app.world().get::<WidgetSpec>(widget).cloned();
        assert!(button_declaration.is_some());
        assert!(app.world().get::<super::super::Tooltips>(widget).is_none());

        let attached = app.world_mut().commands().set_tree(panel, tree.clone());
        assert!(attached.is_ok());
        app.update();
        assert_eq!(self::widget(&mut app, panel), widget);
        assert_eq!(
            app.world().get::<super::super::WidgetSpec>(widget),
            button_declaration.as_ref()
        );
        let original = controller(&app, widget);
        assert_lightweight_controller(&app, widget, original);

        let indexed_controller = app
            .world()
            .get::<TooltipControllerIndex>(panel)
            .and_then(|index| index.entity(&PanelElementId::named("action")));
        assert_eq!(indexed_controller, Some(original));
        let computed_tick = app
            .world()
            .entity(panel)
            .get_ref::<crate::ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed());
        assert!(computed_tick.is_some());

        let identical = app.world_mut().commands().set_tree(panel, tree.clone());
        assert!(identical.is_ok());
        app.update();
        assert_eq!(controller(&app, widget), original);
        assert_eq!(
            app.world()
                .get::<TooltipControllerIndex>(panel)
                .and_then(|index| index.entity(&PanelElementId::named("action"))),
            indexed_controller
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<crate::ComputedDiegeticPanel>()
                .map(|computed| computed.last_changed()),
            computed_tick
        );

        let replacements = [
            tooltip.clone().show_after(Duration::from_secs(1)),
            tooltip.clone().hide_after(Duration::from_secs(1)),
            tooltip.clone().disabled_policy(TooltipDisabledPolicy::Show),
            tooltip.clone().source_anchor(Anchor::BottomLeft),
            tooltip.clone().target_anchor(Anchor::TopRight),
            tooltip
                .clone()
                .offset(PanelAnchorOffset::new(Px(3.0), Px(4.0))),
            tooltip.placement_policy(TooltipPlacementPolicy::Fixed),
        ];
        let mut previous_controller = original;
        for replacement in replacements {
            previous_controller = replace_associated_tooltip(
                &mut app,
                panel,
                widget,
                previous_controller,
                &tree,
                replacement,
            );
        }
        assert_eq!(
            app.world().get::<super::super::WidgetSpec>(widget),
            button_declaration.as_ref()
        );

        let mut blueprint = Tooltip::new(El::column());
        blueprint.text("replacement");
        let replaced = app
            .world_mut()
            .commands()
            .set_tree(panel, tooltip_tree(Some(blueprint.clone())));
        assert!(replaced.is_ok());
        app.update();
        let blueprint_controller = controller(&app, widget);
        assert_ne!(blueprint_controller, previous_controller);
        assert!(app.world().get_entity(previous_controller).is_err());
        assert_eq!(
            app.world().get::<Tooltip>(blueprint_controller),
            Some(&blueprint)
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
    fn attaching_a_tooltip_preserves_the_slider_declaration_and_identity() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=1.0).value(0.25);
        let tree_without_tooltip = slider_tooltip_tree(slider.clone(), None);
        let tree = slider_tooltip_tree(slider.clone(), Some(Tooltip::new(El::new())));
        let declaration = WidgetSpec::Slider(slider);

        let panel = spawn_panel(&mut app, tree_without_tooltip);
        app.update();
        let widget = widget(&mut app, panel);
        assert_eq!(
            app.world().get::<super::super::WidgetSpec>(widget),
            Some(&declaration)
        );
        assert!(app.world().get::<super::super::Tooltips>(widget).is_none());

        assert!(app.world_mut().commands().set_tree(panel, tree).is_ok());
        app.update();

        assert_eq!(self::widget(&mut app, panel), widget);
        assert_eq!(
            app.world().get::<super::super::WidgetSpec>(widget),
            Some(&declaration)
        );
        assert_eq!(
            app.world()
                .get::<TooltipFor>(controller(&app, widget))
                .map(TooltipFor::target),
            Some(widget)
        );
    }

    #[test]
    fn tree_replacement_linked_cleanup_precedes_index_retirement_once() {
        let mut app = test_app();
        app.set_error_handler(error::panic)
            .init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        app.update();
        let widget = widget(&mut app, panel);
        let controller = controller(&app, widget);

        let replacement = LayoutBuilder::new(100.0, 50.0).build();
        let result = app.world_mut().commands().set_tree(panel, replacement);
        assert!(result.is_ok());
        app.update();

        assert!(app.world().get_entity(widget).is_err());
        assert!(app.world().get_entity(controller).is_err());
        assert_eq!(
            app.world()
                .get::<TooltipControllerIndex>(panel)
                .and_then(|index| index.entity(&PanelElementId::named("action"))),
            None
        );
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 1);
    }

    #[test]
    fn retirement_despawns_a_live_malformed_indexed_controller() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, tooltip_tree(None));
        app.update();
        let malformed = app.world_mut().spawn_empty().id();
        let mut replacement = HashMap::new();
        replacement.insert(PanelElementId::named("action"), malformed);
        let index = app.world_mut().get_mut::<TooltipControllerIndex>(panel);
        assert!(index.is_some());
        if let Some(mut index) = index {
            index.replace(replacement);
        }

        let computed = app
            .world_mut()
            .get_mut::<crate::ComputedDiegeticPanel>(panel);
        assert!(computed.is_some());
        if let Some(mut computed) = computed {
            computed.set_changed();
        }
        app.update();

        assert!(app.world().get_entity(malformed).is_err());
    }

    #[test]
    fn ordinary_panel_reifies_widget_layout_text() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("action", crate::TextStyle::default()).layout(El::new().button("action")),
        );
        let panel = spawn_panel(&mut app, builder.build());

        app.update();

        let widget = widget(&mut app, panel);
        assert!(
            app.world()
                .get::<super::super::PanelWidget>(widget)
                .is_some()
        );
    }

    #[test]
    fn missing_standalone_target_removes_reserved_controller() {
        let mut app = test_app();
        let target = app.world_mut().spawn_empty().id();
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.world_mut().despawn(target);
        app.update();

        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn stale_panel_handle_does_not_fall_back_to_general_geometry() {
        let mut app = test_app();
        let target = spawn_panel(&mut app, tooltip_tree(None));
        let handle = PanelEntity::<World>::from_validated(target, PanelSpace::World);
        app.world_mut().entity_mut(target).remove::<DiegeticPanel>();
        app.world_mut()
            .entity_mut(target)
            .insert(center_anchor_geometry());
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn stale_widget_owner_does_not_fall_back_to_general_geometry() {
        let mut app = test_app();
        let owner = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let replacement_owner = spawn_panel(&mut app, tooltip_tree(None));
        app.update();
        let target = widget(&mut app, owner);
        let handle = WidgetEntity::<World>::from_validated(target, owner, PanelSpace::World);
        app.world_mut()
            .entity_mut(target)
            .insert((WidgetOf::new(replacement_owner), center_anchor_geometry()));
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn general_target_implementations_do_not_invent_panel_ownership() {
        let mut app = test_app();
        let target = spawn_panel(&mut app, tooltip_tree(None));
        app.world_mut()
            .entity_mut(target)
            .insert(center_anchor_geometry());
        let general_handle = TooltipTargetEntity::<World>::from_validated(target);
        let general_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(general_handle, Tooltip::new(El::new()));
        let application_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(ApplicationWorldTarget(target), Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<Tooltip>(general_controller).is_some());
        assert!(
            app.world()
                .get::<crate::panel::PanelOwned>(general_controller)
                .is_none()
        );
        assert!(app.world().get::<Tooltip>(application_controller).is_some());
        assert!(
            app.world()
                .get::<crate::panel::PanelOwned>(application_controller)
                .is_none()
        );
    }

    #[test]
    fn linked_target_despawn_removes_standalone_controller() {
        let mut app = test_app();
        let geometry = mesh_face_geometry(
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::ONE,
            },
            MeshFace::PositiveZ,
        );
        assert!(geometry.is_some());
        let target = geometry.map_or(Entity::PLACEHOLDER, |geometry| {
            app.world_mut().spawn(geometry).id()
        });
        assert_ne!(target, Entity::PLACEHOLDER);
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();
        assert!(app.world().get::<Tooltip>(controller).is_some());

        app.world_mut().despawn(target);

        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn mesh_face_provider_tracks_bounds_without_transform_rebuilds() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let source = app
            .world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::new(1.0, 2.0, 3.0),
        });
        app.update();
        let geometry = app.world().get::<ResolvedAnchorGeometry>(target);
        assert!(geometry.is_some());
        let Some(geometry) = geometry else {
            return;
        };
        assert_positive_z_mesh_geometry(geometry);
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
        let before_tick = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert!(before_tick.is_some());

        let translated = Transform::from_translation(Vec3::X);
        app.world_mut()
            .entity_mut(target)
            .insert((translated, GlobalTransform::from(translated)));
        app.update();
        let after_transform_tick = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert_eq!(after_transform_tick, before_tick);

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::new(0.0, 0.0, 1.75),
            half_extents: Vec3A::splat(0.25),
        });
        app.update();
        assert_eq!(
            app.world()
                .get::<ResolvedAnchorGeometry>(target)
                .and_then(|geometry| geometry.frame(AnchorSite::Center).ok())
                .map(|point| point.position().into_inner()),
            Some(Vec3::new(0.0, 0.0, 2.0))
        );
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(1.0, 0.0, 2.0))
        );

        app.world_mut().despawn(source);
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
    }

    #[test]
    fn transform_only_changes_do_not_rebuild_mesh_anchor_geometry() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        let before = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert!(before.is_some());

        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_translation(Vec3::X));
        app.update();

        let after = app
            .world()
            .entity(target)
            .get_ref::<ResolvedAnchorGeometry>()
            .map(|geometry| geometry.last_changed());
        assert_eq!(after, before);
    }

    #[test]
    fn mesh_component_removal_retires_geometry_until_readdition_recovers() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );

        app.world_mut().entity_mut(target).remove::<Mesh3d>();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());
        assert_eq!(
            app.world()
                .get::<MeshAnchorGeometryPending>(target)
                .map(|pending| pending.cause),
            Some(MeshAnchorPendingCause::WaitingForMesh)
        );

        app.world_mut().entity_mut(target).insert(Mesh3d::default());
        app.update();
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn mesh_component_removed_before_first_demand_does_not_publish_retained_bounds() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        app.update();
        assert!(app.world().get::<Aabb>(target).is_some());

        app.world_mut().entity_mut(target).remove::<Mesh3d>();
        app.update();
        assert!(app.world().get::<Aabb>(target).is_some());
        demand_mesh_target(&mut app, target);
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());
        assert_eq!(
            app.world()
                .get::<MeshAnchorGeometryPending>(target)
                .map(|pending| pending.cause),
            Some(MeshAnchorPendingCause::WaitingForMesh)
        );
    }

    #[test]
    fn bounds_removal_retires_geometry_until_current_bounds_return() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::ONE,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let source = app
            .world_mut()
            .spawn((
                center_anchor_geometry(),
                Transform::default(),
                GlobalTransform::default(),
                AnchoredTo::new(target, AnchorSite::Center, AnchorSite::Center),
            ))
            .id();
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );

        app.world_mut().entity_mut(target).remove::<Aabb>();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
        assert!(app.world().get::<MeshAnchorGeometry>(target).is_none());

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::splat(3.0),
        });
        app.update();
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
        assert_eq!(
            app.world()
                .get::<Transform>(source)
                .map(|transform| transform.translation),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );
    }

    #[test]
    fn selected_face_change_refreshes_local_geometry() {
        let mut app = test_app();
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center:       Vec3A::ZERO,
                    half_extents: Vec3A::new(1.0, 2.0, 3.0),
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        demand_mesh_target(&mut app, target);
        settle_mesh_anchor_geometry(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 3.0))
        );

        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveX);
        app.update();

        assert!(
            mesh_anchor_center(&app, target)
                .is_some_and(|center| center.abs_diff_eq(Vec3::X, 1e-5))
        );
    }

    #[test]
    fn panel_role_removal_cleans_associated_and_standalone_controllers() {
        let mut app = materialization_app();
        app.init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let panel_handle = crate::PanelEntity::<World>::from_validated(panel, PanelSpace::World);
        let standalone = app
            .world_mut()
            .commands()
            .spawn_tooltip(panel_handle, Tooltip::new(El::new()));
        app.update();
        let widget = widget(&mut app, panel);
        let associated = controller(&app, widget);
        assert!(app.world().get::<Tooltip>(standalone).is_some());
        prepare_tooltip(&mut app, associated);
        prepare_tooltip(&mut app, standalone);
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(associated).is_some());
        assert!(app.world().get::<MaterializedTooltip>(standalone).is_some());

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get_entity(associated).is_err());
        assert!(app.world().get_entity(standalone).is_err());
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 2);
    }

    #[test]
    fn panel_despawn_cleans_associated_and_standalone_controllers() {
        let mut app = materialization_app();
        app.init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let panel_handle = crate::PanelEntity::<World>::from_validated(panel, PanelSpace::World);
        let standalone = app
            .world_mut()
            .commands()
            .spawn_tooltip(panel_handle, Tooltip::new(El::new()));
        app.update();
        let widget = widget(&mut app, panel);
        let associated = controller(&app, widget);
        assert!(app.world().get::<Tooltip>(standalone).is_some());
        prepare_tooltip(&mut app, associated);
        prepare_tooltip(&mut app, standalone);
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(associated).is_some());
        assert!(app.world().get::<MaterializedTooltip>(standalone).is_some());

        app.world_mut().despawn(panel);
        app.update();

        assert!(app.world().get_entity(associated).is_err());
        assert!(app.world().get_entity(standalone).is_err());
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 2);
    }

    #[test]
    fn preparation_materializes_once_retains_blueprint_and_waits_for_a_compatible_camera() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller =
            standalone_world_tooltip(&mut app, target, fixed_size_tooltip(Vec2::new(30.0, 10.0)));
        app.update();

        let Some(blueprint) = unmaterialized_blueprint(&app, target, controller) else {
            return;
        };

        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let tree_revision = assert_materialized_world_tooltip(&app, target, controller, &blueprint);

        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(|panel| u64::from(panel.tree_revision())),
            tree_revision
        );
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(target)
                .map(AnchoredHere::len),
            Some(1)
        );

        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(1));
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(RenderLayers::layer(0));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn world_panel_and_widget_targets_inherit_layout_unit_and_apply_widget_offset() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target_panel = DiegeticPanel::world()
            .size(Pt(100.0), Pt(50.0))
            .with_tree(tooltip_target_tree())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let target_widget = widget(&mut app, target_panel);

        let panel_controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(30.0, 10.0)),
        );
        let widget_offset = 12.0;
        let widget_controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<World>::from_validated(target_widget, target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(30.0, 10.0))
                .offset(PanelAnchorOffset::new(0.0, widget_offset)),
        );
        app.update();
        for controller in [panel_controller, widget_controller] {
            prepare_tooltip(&mut app, controller);
            app.world_mut()
                .entity_mut(controller)
                .insert(TooltipPresentationCamera::new(camera));
        }
        update_materialization(&mut app);

        for (controller, target) in [
            (panel_controller, target_panel),
            (widget_controller, target_widget),
        ] {
            assert_eq!(
                app.world()
                    .get::<MaterializedTooltip>(controller)
                    .map(MaterializedTooltip::layout_unit),
                Some(Unit::Points)
            );
            assert_eq!(
                app.world()
                    .get::<AnchoredTo>(controller)
                    .map(AnchoredTo::target),
                Some(target)
            );
            let diagnostics = app
                .world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(
                app.world().get::<TooltipReadiness>(controller),
                Some(&TooltipReadiness::Ready),
                "controller {controller:?}: placement={:?}, diagnostics={diagnostics:?}",
                app.world().get::<TooltipPlacementState>(controller),
            );
            let local = app.world().get::<Transform>(controller);
            let global = app.world().get::<GlobalTransform>(controller);
            assert!(local.is_some());
            assert!(global.is_some());
            if let (Some(local), Some(global)) = (local, global) {
                assert!(
                    global
                        .translation()
                        .abs_diff_eq(local.translation, GEOMETRY_EPSILON)
                );
            }
        }

        let target_panel_data = app.world().get::<DiegeticPanel>(target_panel);
        assert!(target_panel_data.is_some());
        let Some(target_panel_data) = target_panel_data else {
            return;
        };
        let expected_y = -widget_offset
            * target_panel_data.layout_unit().to_points()
            * target_panel_data.points_to_world();
        assert!(
            app.world()
                .get::<ResolvedAnchorOffset>(widget_controller)
                .is_some_and(|offset| offset
                    .offset()
                    .into_inner()
                    .abs_diff_eq(Vec3::new(0.0, expected_y, 0.0), GEOMETRY_EPSILON,))
        );
    }

    #[test]
    fn world_keep_visible_tracks_oblique_target_moves_in_propagated_geometry() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target_panel = DiegeticPanel::world()
            .size(Pt(400.0), Pt(200.0))
            .world_width(2.0)
            .with_tree(LayoutBuilder::new(400.0, 200.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target_transform = Transform::from_xyz(2.7, 1.2, 0.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_3));
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn((panel, target_transform)).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(1_600.0, 500.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let initial_bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(initial_bounds.is_some());
        if let Some(initial_bounds) = initial_bounds {
            assert!(bounds_fit_viewport(
                initial_bounds.top_left(),
                initial_bounds.size(),
                zero_origin_viewport(TEST_VIEWPORT),
            ));
        }
        let initial_translation = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);

        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target_panel) {
            transform.translation = Vec3::new(-2.7, -1.2, 0.0);
            transform.rotation = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_3);
        }
        app.update();

        let moved_translation = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(initial_translation.is_some());
        assert!(moved_translation.is_some());
        assert_ne!(initial_translation, moved_translation);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let moved_bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(moved_bounds.is_some());
        if let Some(moved_bounds) = moved_bounds {
            assert!(bounds_fit_viewport(
                moved_bounds.top_left(),
                moved_bounds.size(),
                zero_origin_viewport(TEST_VIEWPORT),
            ));
        }
    }

    #[test]
    fn fixed_world_tooltip_moves_with_its_target_in_the_same_frame() {
        let mut app = materialization_app();
        app.init_resource::<TooltipPlacementRunCount>();
        let target_panel = DiegeticPanel::world()
            .size(Pt(200.0), Pt(100.0))
            .world_width(2.0)
            .with_tree(LayoutBuilder::new(200.0, 100.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn((panel, Transform::default())).id()
        });
        assert_ne!(target, Entity::PLACEHOLDER);
        app.update();

        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(200.0, 100.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        let settled_runs = app.world().resource::<TooltipPlacementRunCount>().world;
        app.update();
        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().world,
            settled_runs,
            "a settled world tooltip should not rerun placement on a quiet frame",
        );

        let initial_target = app
            .world()
            .get::<GlobalTransform>(target)
            .map(GlobalTransform::translation);
        let initial_tooltip = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(initial_target.is_some());
        assert!(initial_tooltip.is_some());

        let movement = Vec3::new(1.25, -0.75, 0.5);
        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target) {
            transform.translation += movement;
        }
        app.update();

        let moved_target = app
            .world()
            .get::<GlobalTransform>(target)
            .map(GlobalTransform::translation);
        let moved_tooltip = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(moved_target.is_some());
        assert!(moved_tooltip.is_some());
        if let (
            Some(initial_target),
            Some(initial_tooltip),
            Some(moved_target),
            Some(moved_tooltip),
        ) = (initial_target, initial_tooltip, moved_target, moved_tooltip)
        {
            assert!(
                moved_target.abs_diff_eq(initial_target + movement, GEOMETRY_EPSILON),
                "target did not publish its current-frame pose",
            );
            assert!(
                moved_tooltip.abs_diff_eq(initial_tooltip + movement, GEOMETRY_EPSILON),
                "tooltip should follow the same target movement in the same frame",
            );
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn world_camera_growth_restores_natural_width_before_rechecking_the_cap() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: NARROW_WORLD_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        set_presentation_viewport(&mut app, camera, window, NARROW_WORLD_VIEWPORT);
        let target = mesh_target(&mut app, Vec3A::ONE);
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(20), TextStyle::new(100.0)).wrap(TextWrap::Words));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        let constrained_max = app
            .world()
            .get::<TooltipWidthConstraint>(controller)
            .map(|constraint| constraint.0);
        assert!(constrained_width.is_some());
        assert!(constrained_max.is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );

        set_presentation_viewport(&mut app, camera, window, WIDE_WORLD_VIEWPORT);
        update_materialization(&mut app);

        let grown_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(grown_width.is_some());
        if let (Some(constrained_width), Some(grown_width)) = (constrained_width, grown_width) {
            assert!(
                grown_width > constrained_width,
                "expected width to grow beyond {constrained_width}, got {grown_width}",
            );
        }
        let grown_max = app
            .world()
            .get::<TooltipWidthConstraint>(controller)
            .map(|constraint| constraint.0);
        assert!(
            grown_max.is_none()
                || constrained_max
                    .zip(grown_max)
                    .is_some_and(|(constrained_max, grown_max)| grown_max > constrained_max)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                zero_origin_viewport(WIDE_WORLD_VIEWPORT),
            ));
        }
    }

    #[test]
    fn world_target_move_restores_natural_width_before_rechecking_the_cap() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: NARROW_WORLD_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        set_presentation_viewport(&mut app, camera, window, NARROW_WORLD_VIEWPORT);
        let target = mesh_target(&mut app, Vec3A::ONE);
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(20), TextStyle::new(100.0)).wrap(TextWrap::Words));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        let camera_tick = app
            .world()
            .entity(camera)
            .get_change_ticks::<Camera>()
            .map(|ticks| ticks.changed);
        assert!(constrained_width.is_some());
        assert!(
            app.world()
                .get::<TooltipWidthConstraint>(controller)
                .is_some()
        );

        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target) {
            transform.translation.z = DISTANT_WORLD_TARGET_Z;
        }
        update_materialization(&mut app);

        let grown_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(grown_width.is_some());
        if let (Some(constrained_width), Some(grown_width)) = (constrained_width, grown_width) {
            assert!(
                grown_width > constrained_width,
                "expected width to grow beyond {constrained_width}, got {grown_width}",
            );
        }
        assert_eq!(
            app.world()
                .entity(camera)
                .get_change_ticks::<Camera>()
                .map(|ticks| ticks.changed),
            camera_tick,
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
    }

    #[test]
    fn world_keep_visible_honors_an_offset_camera_viewport() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        app.world_mut()
            .entity_mut(camera)
            .insert(seeded_perspective_camera_with_offset_viewport());
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller =
            standalone_world_tooltip(&mut app, target, fixed_size_tooltip(Vec2::new(30.0, 10.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let viewport = Rect::from_corners(
            OFFSET_VIEWPORT_ORIGIN,
            OFFSET_VIEWPORT_ORIGIN + OFFSET_VIEWPORT_SIZE,
        );
        assert_eq!(
            app.world()
                .get::<Camera>(camera)
                .and_then(Camera::logical_viewport_rect),
            Some(viewport),
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                viewport,
            ));
        }
    }

    #[test]
    fn screen_panel_and_widget_targets_transfer_panel_presentation_context() {
        let mut app = materialization_app();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let window = app
            .world_mut()
            .spawn(Window {
                resolution: TEST_VIEWPORT.as_uvec2().into(),
                ..Default::default()
            })
            .id();
        let camera_order = 37;
        let render_layers = RenderLayers::layer(6);
        let target_panel = DiegeticPanel::screen()
            .size(Px(200.0), Px(100.0))
            .window_entity(window)
            .camera_order(camera_order)
            .render_layers(render_layers.clone())
            .with_tree(tooltip_target_tree())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let target_widget = widget(&mut app, target_panel);

        let panel_controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(40.0, 20.0)),
        );
        let widget_controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<Screen>::from_validated(target_widget, target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(40.0, 20.0)),
        );
        app.update();
        for controller in [panel_controller, widget_controller] {
            prepare_tooltip(&mut app, controller);
        }
        update_materialization(&mut app);

        for controller in [panel_controller, widget_controller] {
            let materialized = app.world().get::<MaterializedTooltip>(controller);
            assert!(materialized.is_some());
            if let Some(materialized) = materialized {
                assert_eq!(materialized.layout_unit(), Unit::Pixels);
                assert_eq!(materialized.window, Some(window));
                assert_eq!(materialized.camera_order, Some(camera_order));
                assert_eq!(materialized.render_layers, render_layers);
            }
            let diagnostics = app
                .world()
                .resource::<ScreenAttachmentResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>();
            assert!(
                app.world()
                    .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                    .is_some_and(|position| position.anchor_position.is_some()),
                "controller {controller:?}: target_transform={}, placement={:?}, attachment={:?}, diagnostics={diagnostics:?}",
                app.world().get::<Transform>(target_panel).is_some(),
                app.world().get::<TooltipPlacementState>(controller),
                app.world()
                    .get::<crate::panel::PanelAttachmentAuthored>(controller),
            );
            assert_eq!(
                app.world().get::<TooltipReadiness>(controller),
                Some(&TooltipReadiness::Ready)
            );
        }
    }

    #[test]
    fn unavailable_mesh_provider_and_current_diagnostic_keep_tooltip_pending() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        let handle = app
            .world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let controller = app.world_mut().commands().spawn_tooltip(
            handle,
            fixed_size_tooltip(Vec2::new(30.0, 10.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        assert!(app.world().get::<AnchoredHere>(target).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());

        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(controller).is_some());
        assert!(app.world().get::<AnchoredTo>(controller).is_some());
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::splat(2.0),
        });
        update_materialization(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 2.0))
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}, diagnostics={:?}",
            app.world().get::<TooltipPlacementState>(controller),
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>(),
        );

        app.world_mut().entity_mut(target).remove::<Aabb>();
        update_materialization(&mut app);
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn current_world_graph_failures_invalidate_and_recover_readiness() {
        let mut app = materialization_app();
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(20.0, 10.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut().entity_mut(target).insert(AnchoredTo::new(
            controller,
            AnchorSite::from(Anchor::Center),
            AnchorSite::from(Anchor::Center),
        ));
        app.update();
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut().entity_mut(target).remove::<AnchoredTo>();
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .remove::<GlobalTransform>();
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        let target_transform = app
            .world()
            .get::<Transform>(target)
            .copied()
            .unwrap_or_default();
        app.world_mut()
            .entity_mut(target)
            .insert(GlobalTransform::from(target_transform));
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn general_screen_target_transfers_secondary_window_context_and_becomes_ready() {
        let mut app = materialization_app();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let window = app
            .world_mut()
            .spawn(Window {
                resolution: TEST_VIEWPORT.as_uvec2().into(),
                ..Default::default()
            })
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let target_data = crate::screen_space::ScreenAnchorTarget::new(
            bounds,
            window,
            37,
            RenderLayers::layer(6),
            Unit::Pixels,
        );
        let handle = app
            .world_mut()
            .commands()
            .screen_anchor_target(target, target_data);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(120.0, 40.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let materialized = app.world().get::<MaterializedTooltip>(controller);
        assert!(materialized.is_some());
        if let Some(materialized) = materialized {
            assert_eq!(materialized.window, Some(window));
            assert_eq!(materialized.camera_order, Some(37));
            assert_eq!(materialized.render_layers, RenderLayers::layer(6));
            assert_eq!(materialized.layout_unit(), Unit::Pixels);
        }
        let coordinate_space = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::coordinate_space);
        assert!(matches!(
            coordinate_space,
            Some(CoordinateSpace::Screen {
                window: WindowRef::Entity(entity),
                camera_order: 37,
                ..
            }) if *entity == window
        ));
        assert!(
            app.world()
                .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                .is_some_and(|position| position.anchor_position.is_some())
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert_eq!(
            app.world().get::<PanelPicking>(controller),
            Some(&PanelPicking::PASS_THROUGH)
        );
        let local = app.world().get::<Transform>(controller);
        let global = app.world().get::<GlobalTransform>(controller);
        assert!(local.is_some());
        assert!(global.is_some());
        if let (Some(local), Some(global)) = (local, global) {
            assert!(
                global
                    .translation()
                    .abs_diff_eq(local.translation, GEOMETRY_EPSILON)
            );
        }
    }

    #[test]
    fn changed_screen_target_camera_order_and_render_layers_invalidate_readiness() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let camera_order = 37;
        let render_layers = RenderLayers::layer(6);
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                render_layers.clone(),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(120.0, 40.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order + 1,
                render_layers.clone(),
                Unit::Pixels,
            ));
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                render_layers,
                Unit::Pixels,
            ));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                RenderLayers::layer(7),
                Unit::Pixels,
            ));
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn settled_screen_tooltip_has_a_quiet_placement_frame() {
        let mut app = materialization_app();
        app.init_resource::<TooltipPlacementRunCount>();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app.world_mut().commands().spawn_tooltip(
            handle,
            fixed_size_tooltip(Vec2::new(60.0, 30.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        app.update();
        app.update();

        let placement_runs = app.world().resource::<TooltipPlacementRunCount>().screen;
        let placement_tick = app
            .world()
            .entity(controller)
            .get_change_ticks::<TooltipPlacementState>()
            .map(|ticks| ticks.changed);
        let placement = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .copied();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.update();

        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().screen,
            placement_runs,
        );
        assert_eq!(
            app.world()
                .entity(controller)
                .get_change_ticks::<TooltipPlacementState>()
                .map(|ticks| ticks.changed),
            placement_tick,
        );
        assert_eq!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .copied(),
            placement,
        );
    }

    #[test]
    fn a_general_screen_target_with_a_missing_window_stays_lightweight() {
        let mut app = materialization_app();
        let target = app.world_mut().spawn_empty().id();
        let missing_window = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::splat(20.0), Vec2::splat(10.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                missing_window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        assert!(app.world().get::<PrepareTooltip>(controller).is_some());
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(app.world().get::<MaterializedTooltip>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
    }

    #[test]
    fn fixed_tooltip_waits_for_the_first_completed_layout() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(100.0, 80.0), Vec2::new(40.0, 20.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app.world_mut().commands().spawn_tooltip(
            handle,
            fixed_size_tooltip(Vec2::new(60.0, 30.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        prepare_tooltip(&mut app, controller);

        app.update();

        assert!(app.world().get::<MaterializedTooltip>(controller).is_some());
        assert!(
            app.world()
                .get::<ComputedDiegeticPanel>(controller)
                .is_some_and(|computed| !computed.is_solved())
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.update();

        assert!(
            app.world()
                .get::<ComputedDiegeticPanel>(controller)
                .is_some_and(ComputedDiegeticPanel::is_solved)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn changed_tooltip_target_detaches_the_materialized_old_target() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let first_target = app.world_mut().spawn_empty().id();
        let second_target = app.world_mut().spawn_empty().id();
        let first_bounds = PanelScreenBounds::new(Vec2::new(70.0, 60.0), Vec2::new(40.0, 20.0));
        let second_bounds = PanelScreenBounds::new(Vec2::new(140.0, 100.0), Vec2::new(40.0, 20.0));
        assert!(first_bounds.is_ok());
        assert!(second_bounds.is_ok());
        let (Some(first_bounds), Some(second_bounds)) = (first_bounds.ok(), second_bounds.ok())
        else {
            return;
        };
        let first_handle = app.world_mut().commands().screen_anchor_target(
            first_target,
            crate::screen_space::ScreenAnchorTarget::new(
                first_bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        app.world_mut().commands().screen_anchor_target(
            second_target,
            crate::screen_space::ScreenAnchorTarget::new(
                second_bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app.world_mut().commands().spawn_tooltip(
            first_handle,
            fixed_size_tooltip(Vec2::new(60.0, 30.0))
                .placement_policy(TooltipPlacementPolicy::Fixed),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipFor::new(second_target));
        app.update();

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                .is_some_and(|position| position.anchor_position.is_none())
        );
        assert_eq!(
            app.world()
                .get::<MaterializedTooltip>(controller)
                .map(MaterializedTooltip::target),
            Some(first_target)
        );
    }

    #[test]
    fn direct_standalone_component_replacement_does_not_update_materialized_state() {
        let mut app = materialization_app();
        app.init_resource::<TooltipPlacementRunCount>();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let initial_bounds = PanelScreenBounds::new(Vec2::new(80.0, 60.0), Vec2::new(40.0, 20.0));
        let moved_bounds = PanelScreenBounds::new(Vec2::new(140.0, 100.0), Vec2::new(40.0, 20.0));
        assert!(initial_bounds.is_ok());
        assert!(moved_bounds.is_ok());
        let (Some(initial_bounds), Some(moved_bounds)) = (initial_bounds.ok(), moved_bounds.ok())
        else {
            return;
        };
        let camera_order = 5;
        let render_layers = RenderLayers::layer(3);
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                initial_bounds,
                window,
                camera_order,
                render_layers.clone(),
                Unit::Pixels,
            ),
        );
        let original = fixed_size_tooltip(Vec2::new(60.0, 30.0))
            .source_anchor(Anchor::BottomLeft)
            .target_anchor(Anchor::TopRight)
            .offset(PanelAnchorOffset::new(Px(4.0), Px(6.0)))
            .placement_policy(TooltipPlacementPolicy::Fixed);
        let original_attachment = original.attachment();
        let original_blueprint = Arc::clone(original.blueprint());
        let controller = app.world_mut().commands().spawn_tooltip(handle, original);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        app.update();
        app.update();

        let original_panel_size = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(|panel| Vec2::new(panel.width(), panel.height()));
        let settled_runs = app.world().resource::<TooltipPlacementRunCount>().screen;
        let replacement = fixed_size_tooltip(Vec2::new(140.0, 70.0))
            .source_anchor(Anchor::TopCenter)
            .target_anchor(Anchor::BottomCenter)
            .placement_policy(TooltipPlacementPolicy::KeepVisible);
        app.world_mut().entity_mut(controller).insert(replacement);
        app.update();
        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().screen,
            settled_runs,
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                moved_bounds,
                window,
                camera_order,
                render_layers,
                Unit::Pixels,
            ));
        app.update();

        let materialized = app.world().get::<MaterializedTooltip>(controller);
        let live_tooltip = app.world().get::<Tooltip>(controller);
        assert!(materialized.is_some());
        assert!(live_tooltip.is_some());
        if let (Some(materialized), Some(live_tooltip)) = (materialized, live_tooltip) {
            assert!(Arc::ptr_eq(&materialized.blueprint, &original_blueprint));
            assert!(!Arc::ptr_eq(
                &materialized.blueprint,
                live_tooltip.blueprint()
            ));
            assert_eq!(materialized.authored_attachment, original_attachment);
            assert_eq!(materialized.placement_policy, TooltipPlacementPolicy::Fixed);
        }
        assert_eq!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .map(|placement| placement.attachment),
            Some(original_attachment),
        );
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(|panel| Vec2::new(panel.width(), panel.height())),
            original_panel_size,
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
        );
    }

    #[test]
    fn screen_keep_visible_reflows_words_and_rejects_an_unbreakable_overflow() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds =
            PanelScreenBounds::new(TEST_VIEWPORT * 0.5 - Vec2::splat(20.0), Vec2::splat(40.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let font_size = 20.0;
        let mut reflowing = Tooltip::new(El::new());
        reflowing
            .text(Text::new("alpha ".repeat(40), TextStyle::new(font_size)).wrap(TextWrap::Words));
        let mut overflowing = Tooltip::new(El::new());
        overflowing
            .text(Text::new("x".repeat(100), TextStyle::new(font_size)).wrap(TextWrap::Words));
        let reflowing_controller = app.world_mut().commands().spawn_tooltip(handle, reflowing);
        let overflowing_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, overflowing);
        app.update();
        prepare_tooltip(&mut app, reflowing_controller);
        prepare_tooltip(&mut app, overflowing_controller);
        update_materialization(&mut app);

        let usable_width = usable_viewport_axis(TEST_VIEWPORT.x);
        assert!(usable_width.is_some());
        let Some(usable_width) = usable_width else {
            return;
        };
        let reflowing_panel = app.world().get::<DiegeticPanel>(reflowing_controller);
        let reflowing_computed = app
            .world()
            .get::<ComputedDiegeticPanel>(reflowing_controller);
        assert!(reflowing_panel.is_some());
        assert!(reflowing_computed.is_some());
        if let (Some(panel), Some(computed)) = (reflowing_panel, reflowing_computed) {
            assert!(panel.width() <= usable_width);
            assert!(computed.content_height() > font_size);
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(reflowing_controller),
            Some(&TooltipReadiness::Ready)
        );
        let overflow_panel_size = app
            .world()
            .get::<DiegeticPanel>(overflowing_controller)
            .map(|panel| (panel.width(), panel.height()));
        let overflow_layout = app
            .world()
            .get::<ComputedDiegeticPanel>(overflowing_controller)
            .map(|computed| {
                (
                    computed.content_width(),
                    computed.content_height(),
                    computed.content_bounds(),
                )
            });
        assert_eq!(
            app.world().get::<TooltipReadiness>(overflowing_controller),
            Some(&TooltipReadiness::Pending),
            "panel={overflow_panel_size:?}, computed={overflow_layout:?}, placement={:?}",
            app.world()
                .get::<TooltipPlacementState>(overflowing_controller),
        );
        assert_eq!(
            app.world().get::<Visibility>(overflowing_controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn screen_keep_visible_tracks_rotated_target_moves_in_resolved_geometry() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target_panel = DiegeticPanel::screen()
            .size(Px(80.0), Px(40.0))
            .anchor(Anchor::Center)
            .screen_position(185.0, 50.0)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(80.0, 40.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut()
                .spawn((
                    panel,
                    Transform::from_rotation(Quat::from_rotation_z(ROTATED_TARGET_ANGLE)),
                ))
                .id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(70.0, 30.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let initial_attachment = app.world().get::<PanelAttachmentAuthored>(controller);
        assert!(initial_attachment.is_some());
        if let Some(initial_attachment) = initial_attachment {
            assert_eq!(initial_attachment.source_anchor(), Anchor::TopCenter);
            assert_eq!(initial_attachment.target_anchor(), Anchor::BottomCenter);
        }
        let initial_bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(initial_bounds.is_some());
        if let Some(initial_bounds) = initial_bounds {
            assert!(bounds_fit_viewport(
                initial_bounds.top_left(),
                initial_bounds.size(),
                zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        let moved = app
            .world_mut()
            .get_mut::<DiegeticPanel>(target_panel)
            .is_some_and(|mut panel| panel.set_screen_position(Vec2::new(55.0, 130.0)));
        assert!(moved);
        app.update();
        app.update();

        let moved_attachment = app.world().get::<PanelAttachmentAuthored>(controller);
        assert!(moved_attachment.is_some());
        if let Some(moved_attachment) = moved_attachment {
            assert_eq!(moved_attachment.source_anchor(), Anchor::BottomCenter);
            assert_eq!(moved_attachment.target_anchor(), Anchor::TopCenter);
        }
        let moved_bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(moved_bounds.is_some());
        if let Some(moved_bounds) = moved_bounds {
            assert!(bounds_fit_viewport(
                moved_bounds.top_left(),
                moved_bounds.size(),
                zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
        let local_transform = app.world().get::<Transform>(controller);
        let global_transform = app.world().get::<GlobalTransform>(controller);
        assert!(local_transform.is_some());
        assert!(global_transform.is_some());
        if let (Some(local_transform), Some(global_transform)) = (local_transform, global_transform)
        {
            assert!(
                global_transform
                    .translation()
                    .abs_diff_eq(local_transform.translation, GEOMETRY_EPSILON,)
            );
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn attached_screen_target_move_updates_tooltip_placement_in_the_same_frame() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let root = DiegeticPanel::screen()
            .size(Px(40.0), Px(20.0))
            .anchor(Anchor::Center)
            .screen_position(120.0, 40.0)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(40.0, 20.0).build())
            .build();
        let target = DiegeticPanel::screen()
            .size(Px(40.0), Px(20.0))
            .anchor(Anchor::Center)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(40.0, 20.0).build())
            .build();
        assert!(root.is_ok());
        assert!(target.is_ok());
        let root = root.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        let target = target.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(root, Entity::PLACEHOLDER);
        assert_ne!(target, Entity::PLACEHOLDER);
        app.update();
        app.world_mut().commands().attach_to_panel(
            PanelEntity::<Screen>::from_validated(target, PanelSpace::Screen),
            PanelEntity::<Screen>::from_validated(root, PanelSpace::Screen),
            PanelAttachment::new(Anchor::Center, Anchor::Center),
        );
        app.update();

        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(60.0, 30.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        let moved = app
            .world_mut()
            .get_mut::<DiegeticPanel>(root)
            .is_some_and(|mut panel| panel.set_screen_position(Vec2::new(120.0, 140.0)));
        assert!(moved);
        app.update();

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let placement = app.world().get::<TooltipPlacementState>(controller);
        assert!(placement.is_some());
        if let Some(placement) = placement {
            assert_eq!(placement.attachment.source_anchor(), Anchor::BottomCenter);
            assert_eq!(placement.attachment.target_anchor(), Anchor::TopCenter);
        }
        let bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
    }

    #[test]
    fn screen_viewport_growth_relaxes_the_previous_keep_visible_width_cap() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(100.0, 80.0), Vec2::new(40.0, 20.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(3), TextStyle::new(20.0)).wrap(TextWrap::Words));
        let controller = app.world_mut().commands().spawn_tooltip(handle, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(constrained_width.is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
            window
                .resolution
                .set(GROWN_SCREEN_VIEWPORT.x, GROWN_SCREEN_VIEWPORT.y);
        }
        update_materialization(&mut app);

        let grown_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(grown_width.is_some());
        if let (Some(constrained_width), Some(grown_width)) = (constrained_width, grown_width) {
            assert!(
                grown_width > constrained_width,
                "expected width to grow beyond {constrained_width}, got {grown_width}",
            );
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        let bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                zero_origin_viewport(GROWN_SCREEN_VIEWPORT),
            ));
        }
    }

    #[test]
    fn hidden_materialized_panel_role_cleanup_detaches_once() {
        let mut app = materialization_app();
        let target = mesh_target(&mut app, Vec3A::ONE);
        let tooltip = fixed_size_tooltip(Vec2::new(20.0, 10.0))
            .placement_policy(TooltipPlacementPolicy::Fixed);
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
        let tooltip = fixed_size_tooltip(Vec2::new(20.0, 10.0))
            .placement_policy(TooltipPlacementPolicy::Fixed);
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

    #[test]
    fn keep_visible_side_order_and_tie_breaks_are_deterministic() {
        let target = PanelScreenBounds::new(Vec2::new(40.0, 40.0), Vec2::new(20.0, 20.0));
        assert!(target.is_ok());
        let Some(target) = target.ok() else {
            return;
        };
        let viewport = zero_origin_viewport(Vec2::splat(100.0));

        assert_eq!(
            tooltip_side_order(TooltipSide::Above, target, viewport),
            [
                TooltipSide::Above,
                TooltipSide::Below,
                TooltipSide::Right,
                TooltipSide::Left,
            ]
        );
        assert_eq!(
            tooltip_side_order(TooltipSide::Left, target, viewport),
            [
                TooltipSide::Left,
                TooltipSide::Right,
                TooltipSide::Below,
                TooltipSide::Above,
            ]
        );
    }

    #[test]
    fn keep_visible_uses_the_opposite_side_and_preserves_the_margin() {
        let target = PanelScreenBounds::new(Vec2::new(40.0, 2.0), Vec2::new(20.0, 20.0));
        assert!(target.is_ok());
        let Some(target) = target.ok() else {
            return;
        };
        let attachment = PanelAttachment::new(Anchor::BottomCenter, Anchor::TopCenter);
        let target_context = screen_tooltip_target(target);
        let Some(source) = screen_tooltip_source(Vec2::new(30.0, 20.0)) else {
            return;
        };
        let fitted = fit_tooltip_in_viewport(
            attachment,
            source,
            &target_context,
            zero_origin_viewport(Vec2::splat(100.0)),
        );
        assert!(fitted.is_some());
        let Some(fitted) = fitted else {
            return;
        };
        assert_eq!(fitted.source_anchor(), Anchor::TopCenter);
        assert_eq!(fitted.target_anchor(), Anchor::BottomCenter);
        let target_point = target_context
            .rect
            .oriented_anchor_point(fitted.target_anchor());
        assert!(target_point.is_some());
        let Some(target_point) = target_point else {
            return;
        };
        let local_offset = fitted.offset().to_layout_units(Unit::Pixels).truncate();
        let source_anchor_position =
            target_point + rotate_screen_vector(local_offset, target_context.rect.angle());
        let bounds = source.placed_bounds(fitted.source_anchor(), source_anchor_position);
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        assert!(bounds_fit_viewport(
            bounds.top_left(),
            bounds.size(),
            zero_origin_viewport(Vec2::splat(100.0))
        ));
    }

    #[test]
    fn keep_visible_limits_along_edge_shift_to_target_overlap() {
        let top_left = Vec2::new(-13.0, 20.0);
        let source_size = Vec2::new(40.0, 20.0);
        let target_point = Vec2::new(7.0, 20.0);
        let shift = limited_along_edge_shift(
            TooltipSide::Above,
            top_left,
            source_size,
            target_point,
            zero_origin_viewport(Vec2::splat(100.0)),
        );

        assert_eq!(shift, Vec2::new(20.0, 0.0));
        let shifted_left = top_left.x + shift.x;
        assert!(target_point.x >= shifted_left);
        assert!(target_point.x <= shifted_left + source_size.x);
    }

    #[test]
    fn keep_visible_rejects_oversize_and_off_viewport_results() {
        let centered = PanelScreenBounds::new(Vec2::new(45.0, 45.0), Vec2::splat(10.0));
        assert!(centered.is_ok());
        let Some(centered) = centered.ok() else {
            return;
        };
        let attachment = PanelAttachment::new(Anchor::BottomCenter, Anchor::TopCenter);
        let context = screen_tooltip_target(centered);
        let usable_width = usable_viewport_axis(100.0);
        assert!(usable_width.is_some());
        let Some(usable_width) = usable_width else {
            return;
        };
        let Some(oversize_source) = screen_tooltip_source(Vec2::new(usable_width + 1.0, 20.0))
        else {
            return;
        };
        assert!(
            fit_tooltip_in_viewport(
                attachment,
                oversize_source,
                &context,
                zero_origin_viewport(Vec2::splat(100.0)),
            )
            .is_none()
        );

        let outside = PanelScreenBounds::new(Vec2::new(-100.0, 45.0), Vec2::splat(10.0));
        assert!(outside.is_ok());
        let Some(outside) = outside.ok() else {
            return;
        };
        let Some(source) = screen_tooltip_source(Vec2::splat(10.0)) else {
            return;
        };
        assert!(
            fit_tooltip_in_viewport(
                attachment,
                source,
                &screen_tooltip_target(outside),
                zero_origin_viewport(Vec2::splat(100.0)),
            )
            .is_none()
        );
    }
}
