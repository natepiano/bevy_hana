use std::collections::HashMap;

use bevy::ecs::change_detection::Ref;
use bevy::ecs::system::EntityCommands;
use bevy::prelude::Add;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::error;
use hana_valence::AnchorFrame;
use hana_valence::AnchorSite;
use hana_valence::AnchoredHere;
use hana_valence::AnchoredTo as ValenceAnchoredTo;
use hana_valence::Displacement;
use hana_valence::Edge;
use hana_valence::GeometryError;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;

use super::PanelWidget;
use super::PanelWidgetIndex;
use super::PanelWidgets;
use super::ScreenWidgetAnchorProxy;
use super::ScreenWidgetAnchoredHere;
use super::SliderCancelCause;
use super::SliderCaptures;
use super::SliderState;
use super::StateAppearance;
use super::Tooltip;
use super::TooltipControllerIndex;
use super::TooltipFor;
use super::WidgetFocusable;
use super::WidgetKind;
use super::WidgetOf;
use super::WidgetSpec;
use super::WidgetVisualSlots;
use super::button;
use super::button::ButtonCallback;
use super::button::ButtonCallbackHandle;
use super::button::ButtonCancelCause;
use super::button::ButtonCaptures;
use super::slider;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::cascade::CascadeFrom;
use crate::layout::Anchor;
use crate::panel;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelComponentOwnership;
use crate::panel::PanelOwned;
use crate::panel::PanelSpace;

/// Reuses equal associated tooltip controllers and replaces changed declarations.
pub(super) fn reify_tooltip_controllers(
    mut panels: Query<
        (Entity, &ComputedDiegeticPanel, &mut TooltipControllerIndex),
        Changed<ComputedDiegeticPanel>,
    >,
    widget_reader: super::PanelWidgetReader,
    controllers: Query<(&Tooltip, &TooltipFor, Option<&PanelOwned>)>,
    live_entities: Query<()>,
    mut commands: Commands,
) {
    for (panel, computed, mut controller_index) in &mut panels {
        let mut next_index = HashMap::with_capacity(computed.tooltip_records().len());
        for record in computed.tooltip_records() {
            let Some(widget) = widget_reader.entity(panel, record.widget_id()) else {
                continue;
            };
            let reusable = controller_index
                .entity(record.widget_id())
                .filter(|entity| {
                    controllers
                        .get(*entity)
                        .is_ok_and(|(tooltip, _, _)| tooltip == record.tooltip())
                });
            let controller = reusable.unwrap_or_else(|| {
                commands
                    .spawn((
                        record.tooltip().clone(),
                        TooltipFor::new(widget),
                        PanelOwned::from(panel),
                    ))
                    .id()
            });
            if let Ok((tooltip, tooltip_for, ownership)) = controllers.get(controller) {
                debug_assert_eq!(tooltip, record.tooltip());
                let mut controller_commands = commands.entity(controller);
                if tooltip_for.target() != widget {
                    controller_commands.insert(TooltipFor::new(widget));
                }
                if ownership.is_none_or(|ownership| ownership.owner() != panel) {
                    controller_commands.insert(PanelOwned::from(panel));
                }
            }
            next_index.insert(record.widget_id().clone(), controller);
        }

        let retained = next_index.values().copied().collect::<Vec<_>>();
        for controller in controller_index.entities() {
            if !retained.contains(&controller) && live_entities.contains(controller) {
                commands.entity(controller).despawn();
            }
        }
        controller_index.replace(next_index);
    }
}

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(super) struct WidgetPreorder(usize);

#[derive(Clone, Copy, Component, Debug, PartialEq)]
pub(crate) struct WidgetAnchorRect {
    panel_offset: Vec3,
    size:         Vec2,
    anchor:       Anchor,
    space:        PanelSpace,
}

impl WidgetAnchorRect {
    fn new(panel: &DiegeticPanel, rect: crate::BoundingBox) -> Self {
        let scale = panel.points_to_world();
        let (x_offset, y_offset) = panel.anchor_offsets();
        let (center_x, center_y) = rect.center();
        let size = Vec2::new(rect.width * scale, rect.height * scale);
        Self {
            panel_offset: Vec3::new(
                center_x.mul_add(scale, -x_offset),
                (-center_y).mul_add(scale, y_offset),
                0.0,
            ),
            size,
            anchor: panel.anchor(),
            space: PanelSpace::from(panel.coordinate_space()),
        }
    }

    pub(crate) const fn panel_offset(self) -> Vec3 { self.panel_offset }

    pub(crate) const fn size(self) -> Vec2 { self.size }

    pub(crate) const fn space(self) -> PanelSpace { self.space }

    const fn transform(self) -> Transform { Transform::from_translation(self.panel_offset) }
}

pub(crate) fn on_screen_widget_demand_added(
    added: On<Add, ScreenWidgetAnchoredHere>,
    widgets: Query<&WidgetOf>,
    mut commands: Commands,
) {
    let widget = added.entity;
    let Ok(widget_of) = widgets.get(widget) else {
        return;
    };
    panel::write_owned_component(
        &mut commands,
        widget_of.panel(),
        widget,
        ScreenWidgetAnchorProxy,
    );
}

pub(crate) fn on_screen_widget_demand_removed(
    removed: On<Remove, ScreenWidgetAnchoredHere>,
    widgets: Query<
        (
            Option<&WidgetOf>,
            Option<&PanelComponentOwnership<ScreenWidgetAnchorProxy>>,
            Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
        ),
        With<PanelWidget>,
    >,
    mut commands: Commands,
) {
    let widget = removed.entity;
    let Ok((widget_of, proxy_ownership, geometry_ownership)) = widgets.get(widget) else {
        return;
    };
    let Some(panel) = widget_owner_or_recorded(widget_of, proxy_ownership, geometry_ownership)
    else {
        return;
    };
    panel::remove_owned_component::<ScreenWidgetAnchorProxy>(&mut commands, panel, widget);
    panel::remove_owned_component::<ResolvedAnchorGeometry>(&mut commands, panel, widget);
}

/// Reifies widget entities for every changed computed panel.
pub(super) fn reify_widgets(
    mut changed_panels: Query<
        (
            Entity,
            &DiegeticPanel,
            &GlobalTransform,
            &ComputedDiegeticPanel,
            Option<&PanelWidgets>,
            &mut PanelWidgetIndex,
        ),
        Changed<ComputedDiegeticPanel>,
    >,
    existing_widgets: Query<(
        &PanelWidget,
        &WidgetKind,
        &WidgetSpec,
        &Cascade<super::WidgetHoveredAppearance>,
        &Cascade<super::WidgetPressedAppearance>,
        &Cascade<super::WidgetFocusedAppearance>,
        &Cascade<super::WidgetDisabledAppearance>,
        &WidgetPreorder,
        &Transform,
        &WidgetAnchorRect,
        Option<&Cascade<super::WidgetInteractivity>>,
        Option<&CascadeFrom>,
        Option<&WidgetVisualSlots>,
        Option<&SliderState>,
    )>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    for (panel_entity, panel, panel_global, computed, panel_widgets, mut widget_index) in
        &mut changed_panels
    {
        let existing_entities: &[Entity] = panel_widgets.map_or(&[], |widgets| &**widgets);
        let existing_by_id: HashMap<&PanelElementId, Entity> = existing_entities
            .iter()
            .filter_map(|entity| {
                existing_widgets
                    .get(*entity)
                    .ok()
                    .map(|(widget, ..)| (widget.id(), *entity))
            })
            .collect();

        let mut visited = Vec::with_capacity(computed.widget_records().len());
        let mut next_widget_index = HashMap::with_capacity(computed.widget_records().len());
        for record in computed.widget_records() {
            let anchor_rect = WidgetAnchorRect::new(panel, record.rect());
            let visual_slots = WidgetVisualSlots::new(record.visual_slots().to_vec())
                .with_elements(record.visual_elements().to_vec())
                .with_generated_editor_elements(record.generated_editor_elements().to_vec())
                .with_part_appearances(record.part_appearances().to_vec());
            let entity = match existing_by_id.get(record.id()).copied() {
                None => spawn_widget(
                    &mut commands,
                    panel_entity,
                    panel_global,
                    record.id().clone(),
                    record.kind(),
                    record.authored().clone(),
                    record.appearance().clone(),
                    record.preorder(),
                    record.interactivity(),
                    anchor_rect,
                    visual_slots,
                ),
                Some(entity) => {
                    update_widget(
                        &mut commands,
                        entity,
                        record.kind(),
                        record.authored(),
                        record.appearance(),
                        record.preorder(),
                        record.interactivity(),
                        anchor_rect,
                        visual_slots,
                        panel_entity,
                        &existing_widgets,
                        &mut button_captures,
                        &mut slider_captures,
                    );
                    entity
                },
            };
            visited.push(entity);
            next_widget_index.insert(record.id().clone(), entity);
        }

        for &entity in existing_entities {
            if !visited.contains(&entity) {
                button::cancel_button_press(
                    entity,
                    ButtonCancelCause::WidgetRemoved,
                    &mut button_captures,
                    &mut commands,
                );
                slider::cancel_slider_drag(
                    entity,
                    SliderCancelCause::WidgetRemoved,
                    &mut slider_captures,
                    &mut commands,
                );
                commands.entity(entity).despawn();
            }
        }

        widget_index.replace(next_widget_index);
    }
}

fn spawn_widget(
    commands: &mut Commands<'_, '_>,
    panel: Entity,
    panel_global: &GlobalTransform,
    id: PanelElementId,
    kind: WidgetKind,
    authored: WidgetSpec,
    appearance: StateAppearance,
    preorder: usize,
    interactivity: Cascade<super::WidgetInteractivity>,
    anchor_rect: WidgetAnchorRect,
    visual_slots: WidgetVisualSlots,
) -> Entity {
    let transform = anchor_rect.transform();
    let global_transform = panel_global.mul_transform(transform);
    let callback = widget_callback(&authored).cloned();
    let slider_state = match &authored {
        WidgetSpec::Slider(slider) => Some(slider.initial_state()),
        WidgetSpec::Button(_) | WidgetSpec::EditableField(_) => None,
    };
    let StateAppearance {
        hovered,
        pressed,
        focused,
        disabled,
    } = appearance;
    let mut spawned = Entity::PLACEHOLDER;
    commands.entity(panel).with_children(|children| {
        spawned = children
            .spawn((
                PanelWidget::new(id),
                WidgetFocusable,
                WidgetOf::new(panel),
                kind,
                authored,
                (hovered, pressed, focused, disabled),
                WidgetPreorder(preorder),
                transform,
                global_transform,
                anchor_rect,
                interactivity,
                CascadeFrom::new(panel),
                PanelOwned::from(panel),
                visual_slots,
            ))
            .id();
    });
    if let Some(state) = slider_state {
        commands.entity(spawned).insert(state);
    }
    if let Some(callback) = callback {
        install_callback_handle(commands, spawned, callback);
    }
    spawned
}

fn update_widget(
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    kind: WidgetKind,
    authored: &WidgetSpec,
    appearance: &StateAppearance,
    preorder: usize,
    interactivity: Cascade<super::WidgetInteractivity>,
    anchor_rect: WidgetAnchorRect,
    visual_slots: WidgetVisualSlots,
    panel: Entity,
    existing_widgets: &Query<(
        &PanelWidget,
        &WidgetKind,
        &WidgetSpec,
        &Cascade<super::WidgetHoveredAppearance>,
        &Cascade<super::WidgetPressedAppearance>,
        &Cascade<super::WidgetFocusedAppearance>,
        &Cascade<super::WidgetDisabledAppearance>,
        &WidgetPreorder,
        &Transform,
        &WidgetAnchorRect,
        Option<&Cascade<super::WidgetInteractivity>>,
        Option<&CascadeFrom>,
        Option<&WidgetVisualSlots>,
        Option<&SliderState>,
    )>,
    button_captures: &mut ButtonCaptures,
    slider_captures: &mut SliderCaptures,
) {
    let Ok((
        _,
        existing_kind,
        existing_authored,
        existing_hovered,
        existing_pressed,
        existing_focused,
        existing_disabled,
        existing_preorder,
        existing_transform,
        existing_anchor_rect,
        existing_interactivity,
        existing_cascade_from,
        existing_visual_slots,
        existing_slider_state,
    )) = existing_widgets.get(entity)
    else {
        return;
    };
    if *existing_kind != kind {
        match *existing_kind {
            WidgetKind::Button => {
                button::cancel_button_press(
                    entity,
                    ButtonCancelCause::WidgetKindChanged,
                    button_captures,
                    commands,
                );
            },
            WidgetKind::EditableField => {},
            WidgetKind::Slider => {
                slider::cancel_slider_drag(
                    entity,
                    SliderCancelCause::WidgetKindChanged,
                    slider_captures,
                    commands,
                );
            },
        }
        commands.entity(entity).insert(kind);
    }
    let mut widget = commands.entity(entity);
    if existing_authored != authored {
        widget.insert(authored.clone());
    }
    update_widget_appearance(
        &mut widget,
        existing_hovered,
        existing_pressed,
        existing_focused,
        existing_disabled,
        appearance,
    );
    if existing_preorder.0 != preorder {
        widget.insert(WidgetPreorder(preorder));
    }
    let transform = anchor_rect.transform();
    if *existing_transform != transform {
        widget.insert(transform);
    }
    if *existing_anchor_rect != anchor_rect {
        widget.insert(anchor_rect);
    }
    if existing_visual_slots != Some(&visual_slots) {
        widget.insert(visual_slots);
    }
    if existing_interactivity != Some(&interactivity) {
        widget.insert(interactivity);
    }
    if existing_cascade_from.is_none_or(|relationship| relationship.target() != panel) {
        widget.insert(CascadeFrom::new(panel));
    }
    match authored {
        WidgetSpec::Slider(slider) => match existing_slider_state {
            None => {
                widget.insert(slider.initial_state());
            },
            Some(state) if !state.matches_configuration(slider) => {
                widget.insert(SliderState::with_configuration(slider, state.value()));
            },
            Some(_) => {},
        },
        WidgetSpec::Button(_) | WidgetSpec::EditableField(_) => {
            if existing_slider_state.is_some() {
                widget.remove::<SliderState>();
            }
        },
    }
    let existing_callback = widget_callback(existing_authored);
    let next_callback = widget_callback(authored);
    if existing_callback != next_callback {
        match next_callback.cloned() {
            Some(callback) => install_callback_handle(commands, entity, callback),
            None => {
                commands.entity(entity).remove::<ButtonCallbackHandle>();
            },
        }
    }
}

fn update_widget_appearance(
    widget: &mut EntityCommands<'_>,
    existing_hovered: &Cascade<super::WidgetHoveredAppearance>,
    existing_pressed: &Cascade<super::WidgetPressedAppearance>,
    existing_focused: &Cascade<super::WidgetFocusedAppearance>,
    existing_disabled: &Cascade<super::WidgetDisabledAppearance>,
    appearance: &StateAppearance,
) {
    if existing_hovered != &appearance.hovered {
        widget.insert(appearance.hovered.clone());
    }
    if existing_pressed != &appearance.pressed {
        widget.insert(appearance.pressed.clone());
    }
    if existing_focused != &appearance.focused {
        widget.insert(appearance.focused.clone());
    }
    if existing_disabled != &appearance.disabled {
        widget.insert(appearance.disabled.clone());
    }
}

const fn widget_callback(authored: &WidgetSpec) -> Option<&ButtonCallback> {
    match authored {
        WidgetSpec::Button(button) => button.callback(),
        WidgetSpec::EditableField(_) | WidgetSpec::Slider(_) => None,
    }
}

/// Builds the tracked handle for `callback` on the world and stores it on the
/// widget, replacing (and thereby releasing) any prior tracked handle.
fn install_callback_handle(
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    callback: ButtonCallback,
) {
    commands.queue(move |world: &mut World| {
        let Ok(mut widget) = world.get_entity_mut(entity) else {
            return;
        };
        match callback.build_handle(&mut widget) {
            Ok(handle) => {
                widget.insert(ButtonCallbackHandle::new(handle));
            },
            Err(error) => error!("failed to register button click callback: {error}"),
        }
    });
}

pub(super) fn update_world_anchor_geometry(
    mut commands: Commands,
    changed_geometry_widgets: Query<
        (
            Entity,
            &WidgetOf,
            &WidgetAnchorRect,
            Option<&AnchoredHere>,
            Option<Ref<ResolvedAnchorGeometry>>,
            Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
        ),
        (
            With<PanelWidget>,
            Or<(Changed<WidgetAnchorRect>, Changed<AnchoredHere>)>,
        ),
    >,
    changed_relation_widgets: Query<
        (
            Entity,
            &WidgetOf,
            &WidgetAnchorRect,
            Option<&AnchoredHere>,
            Option<Ref<ValenceAnchoredTo>>,
            Option<&PanelComponentOwnership<ValenceAnchoredTo>>,
        ),
        (
            With<PanelWidget>,
            Or<(
                Changed<WidgetAnchorRect>,
                Changed<AnchoredHere>,
                Changed<GlobalTransform>,
            )>,
        ),
    >,
    panel_globals: Query<&GlobalTransform, With<DiegeticPanel>>,
    widgets: Query<(&WidgetOf, &WidgetAnchorRect, Option<&AnchoredHere>), With<PanelWidget>>,
    mut removed_demands: RemovedComponents<AnchoredHere>,
) {
    for (entity, widget_of, anchor_rect, world_demand, geometry, geometry_ownership) in
        &changed_geometry_widgets
    {
        let panel = widget_of.panel();
        if anchor_rect.space != PanelSpace::World {
            retire_world_bridge(&mut commands, panel, entity);
            continue;
        }
        if world_demand.is_none_or(AnchoredHere::is_empty) {
            retire_world_bridge(&mut commands, panel, entity);
            retire_anchor_geometry(&mut commands, panel, entity);
            continue;
        }

        let Ok(next_geometry) = widget_anchor_geometry(anchor_rect.size) else {
            continue;
        };
        let geometry_matches = geometry.as_ref().is_some_and(|geometry| {
            geometry_ownership
                .is_some_and(|ownership| ownership.owns(panel, geometry.last_changed()))
                && same_geometry(geometry, &next_geometry)
        });
        if !geometry_matches {
            panel::write_owned_component(&mut commands, panel, entity, next_geometry);
        }
    }

    for (entity, widget_of, anchor_rect, world_demand, relation, relation_ownership) in
        &changed_relation_widgets
    {
        let panel = widget_of.panel();
        if world_demand.is_none_or(AnchoredHere::is_empty) || anchor_rect.space != PanelSpace::World
        {
            continue;
        }
        let Ok(panel_global) = panel_globals.get(panel) else {
            continue;
        };
        let next_relation = widget_panel_relation(panel, *anchor_rect, panel_global);
        let relation_matches = relation.as_ref().is_some_and(|relation| {
            relation_ownership
                .is_some_and(|ownership| ownership.owns(panel, relation.last_changed()))
                && **relation == next_relation
        });
        if !relation_matches {
            panel::write_owned_component(&mut commands, panel, entity, next_relation);
        }
    }

    for entity in removed_demands.read() {
        let Ok((widget_of, anchor_rect, world_demand)) = widgets.get(entity) else {
            continue;
        };
        if world_demand.is_some_and(|demand| !demand.is_empty()) {
            continue;
        }
        let panel = widget_of.panel();
        retire_world_bridge(&mut commands, panel, entity);
        if anchor_rect.space == PanelSpace::World {
            retire_anchor_geometry(&mut commands, panel, entity);
        }
    }
}

pub(crate) fn update_screen_anchor_geometry(
    mut commands: Commands,
    changed_widgets: Query<
        (
            Entity,
            &WidgetOf,
            &WidgetAnchorRect,
            Option<&ScreenWidgetAnchoredHere>,
            Option<Ref<ResolvedAnchorGeometry>>,
            Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
        ),
        (
            With<PanelWidget>,
            Or<(Changed<WidgetAnchorRect>, Changed<ScreenWidgetAnchoredHere>)>,
        ),
    >,
    widgets: Query<
        (
            Option<&WidgetOf>,
            Option<&ScreenWidgetAnchoredHere>,
            Option<&PanelComponentOwnership<ScreenWidgetAnchorProxy>>,
            Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
        ),
        With<PanelWidget>,
    >,
    mut removed_proxies: RemovedComponents<ScreenWidgetAnchorProxy>,
) {
    for (entity, widget_of, anchor_rect, screen_demand, geometry, geometry_ownership) in
        &changed_widgets
    {
        let panel = widget_of.panel();
        if screen_demand.is_none_or(ScreenWidgetAnchoredHere::is_empty) {
            continue;
        }
        write_anchor_geometry(
            &mut commands,
            panel,
            entity,
            anchor_rect.size,
            geometry,
            geometry_ownership,
        );
    }

    for entity in removed_proxies.read() {
        let Ok((widget_of, screen_demand, proxy_ownership, geometry_ownership)) =
            widgets.get(entity)
        else {
            continue;
        };
        let Some(panel) = widget_owner_or_recorded(widget_of, proxy_ownership, geometry_ownership)
        else {
            continue;
        };
        panel::remove_owned_component::<ScreenWidgetAnchorProxy>(&mut commands, panel, entity);
        if screen_demand.is_none_or(ScreenWidgetAnchoredHere::is_empty) {
            retire_anchor_geometry(&mut commands, panel, entity);
        }
    }
}

fn widget_owner_or_recorded(
    widget_of: Option<&WidgetOf>,
    proxy_ownership: Option<&PanelComponentOwnership<ScreenWidgetAnchorProxy>>,
    geometry_ownership: Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
) -> Option<Entity> {
    widget_of
        .map(WidgetOf::panel)
        .or_else(|| proxy_ownership.map(PanelComponentOwnership::owner))
        .or_else(|| geometry_ownership.map(PanelComponentOwnership::owner))
}

fn write_anchor_geometry(
    commands: &mut Commands<'_, '_>,
    panel: Entity,
    widget: Entity,
    size: Vec2,
    geometry: Option<Ref<'_, ResolvedAnchorGeometry>>,
    geometry_ownership: Option<&PanelComponentOwnership<ResolvedAnchorGeometry>>,
) {
    let Ok(next_geometry) = widget_anchor_geometry(size) else {
        return;
    };
    let geometry_matches = geometry.as_ref().is_some_and(|geometry| {
        geometry_ownership.is_some_and(|ownership| ownership.owns(panel, geometry.last_changed()))
            && same_geometry(geometry, &next_geometry)
    });
    if !geometry_matches {
        panel::write_owned_component(commands, panel, widget, next_geometry);
    }
}

fn widget_panel_relation(
    panel: Entity,
    rect: WidgetAnchorRect,
    panel_global: &GlobalTransform,
) -> ValenceAnchoredTo {
    let panel_scale = panel_global.affine().transform_vector3(Vec3::X).length();
    ValenceAnchoredTo::new(panel, AnchorSite::Center, AnchorSite::from(rect.anchor))
        .with_offset(Displacement::from(rect.panel_offset * panel_scale))
}

fn widget_anchor_geometry(size: Vec2) -> Result<ResolvedAnchorGeometry, GeometryError> {
    let (center_x, center_y) = Anchor::Center.offset(size.x, size.y);
    let points = [
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
    .into_iter()
    .map(|anchor| {
        let (x, y) = anchor.offset(size.x, size.y);
        AnchorFrame::try_new(
            Position::from(Vec3::new(x - center_x, center_y - y, 0.0)),
            Orientation::default(),
        )
        .map(|frame| (AnchorSite::from(anchor), frame))
    })
    .collect::<Result<Vec<_>, _>>()?;
    let edge_candidates = [
        (Anchor::TopLeft, Anchor::TopRight),
        (Anchor::TopRight, Anchor::BottomRight),
        (Anchor::BottomRight, Anchor::BottomLeft),
        (Anchor::BottomLeft, Anchor::TopLeft),
    ]
    .map(|(start, end)| Edge {
        start: AnchorSite::from(start),
        end:   AnchorSite::from(end),
    })
    .to_vec();

    // A reified widget may be point-sized while its layout is still settling.
    // It can still provide named attachment sites, but a collapsed rectangle
    // has no ordered axes to publish as edges. Filter those candidates before
    // `ResolvedAnchorGeometry::try_new` instead of weakening its validation.
    let sites_only = ResolvedAnchorGeometry::try_new(points.clone(), Vec::<Edge>::new())?;
    let edges = edge_candidates
        .into_iter()
        .filter(|edge| edge.axis(&sites_only).is_ok())
        .collect::<Vec<_>>();
    ResolvedAnchorGeometry::try_new(points, edges)
}

fn same_geometry(left: &ResolvedAnchorGeometry, right: &ResolvedAnchorGeometry) -> bool {
    left.frames().count() == right.frames().count()
        && left.frames().all(|(site, left_frame)| {
            right
                .frame(*site)
                .is_ok_and(|right_frame| right_frame == left_frame)
        })
        && left.edges() == right.edges()
}

fn retire_world_bridge(commands: &mut Commands<'_, '_>, panel: Entity, widget: Entity) {
    panel::remove_owned_component::<ValenceAnchoredTo>(commands, panel, widget);
}

fn retire_anchor_geometry(commands: &mut Commands<'_, '_>, panel: Entity, widget: Entity) {
    panel::remove_owned_component::<ResolvedAnchorGeometry>(commands, panel, widget);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;

    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::ecs::system::SystemIdMarker;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::Bundle;
    use bevy::prelude::ChildOf;
    use bevy::prelude::Click;
    use bevy::prelude::Color;
    use bevy::prelude::Component;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::In;
    use bevy::prelude::Mesh3d;
    use bevy::prelude::MeshMaterial3d;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::Press;
    use bevy::prelude::Quat;
    use bevy::prelude::Release;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Window;
    use bevy::transform::TransformPlugin;
    use bevy::window::PrimaryWindow;
    use hana_valence::AnchorPose;
    use hana_valence::AnchorSite;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo as ValenceAnchoredTo;
    use hana_valence::Displacement;
    use hana_valence::ResolvedAnchorGeometry;
    use hana_valence::ResolvedAnchorOffset;

    use super::StateAppearance;
    use super::WidgetPreorder;
    use crate::Anchor;
    use crate::Appearance;
    use crate::Button;
    use crate::ButtonClicked;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::Fit;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidget;
    use crate::PanelWidgetReader;
    use crate::PanelWidgets;
    use crate::Slider;
    use crate::TextStyle;
    use crate::WidgetDisabledAppearance;
    use crate::WidgetFocusedAppearance;
    use crate::WidgetHoveredAppearance;
    use crate::WidgetInteractivity;
    use crate::WidgetOf;
    use crate::WidgetPressedAppearance;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeFrom;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::PanelComponentOwnership;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::PanelWidgetIndex;
    use crate::widgets::ScreenWidgetAnchorProxy;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetKind;
    use crate::widgets::WidgetSpec;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::button::ButtonCallbackHandle;
    use crate::widgets::button::ButtonPress;

    #[derive(Bundle)]
    struct TestAttachment {
        authored: PanelAttachmentAuthored,
        offset:   crate::PanelAnchorOffset,
    }

    impl TestAttachment {
        const fn new(target: Entity, source: Anchor, target_anchor: Anchor) -> Self {
            Self {
                authored: PanelAttachmentAuthored::new(target, source, target_anchor),
                offset:   crate::PanelAnchorOffset::ZERO,
            }
        }
    }

    const ANCHOR_EPSILON: f32 = 1e-4;
    const ANCHORED_OWNER_ROTATION: f32 = 0.3;
    const ANCHORED_OWNER_TRANSLATION: Vec3 = Vec3::new(1.0, 2.0, 0.0);
    const ANCHORED_POSE_TRANSLATION: Vec3 = Vec3::new(0.4, -0.2, 0.1);
    const DEPENDENT_PANEL_HEIGHT: f32 = 10.0;
    const DEPENDENT_PANEL_WIDTH: f32 = 20.0;
    const DEPENDENT_PANEL_WORLD_WIDTH: f32 = 0.2;
    const OFFSET_PANEL_HEIGHT: f32 = 50.0;
    const OFFSET_PANEL_WIDTH: f32 = 100.0;
    const OFFSET_PANEL_WORLD_WIDTH: f32 = 2.0;
    const OFFSET_WIDGET_HEIGHT: f32 = 10.0;
    const OFFSET_WIDGET_SPACER: f32 = 30.0;
    const OFFSET_WIDGET_WIDTH: f32 = 20.0;
    const RESIZED_WIDGET_WIDTH: f32 = 40.0;
    const SCREEN_FIT_SPACER_WIDTH: f32 = 30.0;
    const SCREEN_FIT_WIDGET_HEIGHT: f32 = 10.0;
    const SCREEN_FIT_WIDGET_WIDTH: f32 = 20.0;
    const UPDATED_OWNER_SCALE: f32 = 1.75;

    #[derive(Component)]
    struct ApplicationData;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CallbackSource {
        Authored,
        Replacement,
    }

    #[derive(Default, Resource)]
    struct CallbackRuns(Vec<(CallbackSource, Option<PointerId>)>);

    fn record_authored_callback(click: In<ButtonClicked>, mut runs: ResMut<CallbackRuns>) {
        runs.0.push((CallbackSource::Authored, click.pointer_id));
    }

    fn record_replacement_callback(click: In<ButtonClicked>, mut runs: ResMut<CallbackRuns>) {
        runs.0.push((CallbackSource::Replacement, click.pointer_id));
    }

    fn callback_tree(button: Button) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().widget("action", button), |_| {});
        builder.build()
    }

    fn widget_tree(ids: &[&str]) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for id in ids {
            builder.with(El::new().button(*id), |_| {});
        }
        builder.build()
    }

    fn appearance_tree(color: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(Color::WHITE)
                .button("action")
                .hovered(Appearance::new().background(color)),
            |_| {},
        );
        builder.build()
    }

    fn hovered_empty_appearance() -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(Appearance::new())),
            ..StateAppearance::default()
        }
    }

    fn part_appearance_tree(include_leading_sibling: bool) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("action"), |builder| {
            if include_leading_sibling {
                builder.with(El::new(), |_| {});
            }
            builder.with(El::new(), |_| {});
        });
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn editable_display_tree() -> LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().editable_field("gain", field), |builder| {
            builder.text(("display", TextStyle::new(10.0)));
            builder.with(El::new(), |_| {});
        });
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn inline_editor_content_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(El::new());
        builder.with(El::new(), |_| {});
        builder.with(El::new(), |_| {});
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn part_appearance_keys(app: &App, widget: Entity) -> Option<Vec<usize>> {
        app.world().get::<WidgetVisualSlots>(widget).map(|slots| {
            slots
                .part_appearances()
                .iter()
                .map(|(element_index, _)| *element_index)
                .collect()
        })
    }

    fn offset_widget_tree(width: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(OFFSET_PANEL_WIDTH, OFFSET_PANEL_HEIGHT);
        builder.with(
            El::row().size(OFFSET_PANEL_WIDTH, OFFSET_PANEL_HEIGHT),
            |builder| {
                builder.with(
                    El::new().size(OFFSET_WIDGET_SPACER, OFFSET_WIDGET_HEIGHT),
                    |_| {},
                );
                builder.with(
                    El::new().size(width, OFFSET_WIDGET_HEIGHT).button("offset"),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn slider_tree(id: &str, initial_value: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().widget(id, slider(initial_value)), |_| {});
        builder.build()
    }

    fn ranked_slider_tree(initial_value: f32, z_index: i8) -> LayoutTree {
        let slider = slider(initial_value);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .widget("level", slider)
                .widget_interactivity(WidgetInteractivity::Disabled)
                .z_index(z_index),
            |_| {},
        );
        builder.with(El::new().size(20.0, 10.0).button("peer"), |_| {});
        builder.build()
    }

    fn slider(initial_value: f32) -> Slider { Slider::new(0.0..=10.0).value(initial_value) }

    fn slider_spec(initial_value: f32) -> WidgetSpec { WidgetSpec::Slider(slider(initial_value)) }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn world_anchor_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(TransformPlugin)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn screen_anchor_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(TransformPlugin)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app
    }

    fn spawn_world_panel(
        app: &mut App,
        tree: LayoutTree,
        anchor: Anchor,
        transform: Transform,
    ) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(OFFSET_PANEL_WIDTH), Mm(OFFSET_PANEL_HEIGHT))
            .world_width(OFFSET_PANEL_WORLD_WIDTH)
            .anchor(anchor)
            .with_tree(tree)
            .build()
            .expect("world panel should build");
        app.world_mut().spawn((panel, transform)).id()
    }

    fn spawn_screen_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::screen()
            .size(Mm(OFFSET_PANEL_WIDTH), Mm(OFFSET_PANEL_HEIGHT))
            .anchor(Anchor::Center)
            .screen_position(100.0, 100.0)
            .with_tree(tree)
            .build()
            .expect("screen panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn spawn_widget_dependent(
        app: &mut App,
        target: Entity,
        source_anchor: Anchor,
        target_anchor: Anchor,
    ) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(DEPENDENT_PANEL_WIDTH), Mm(DEPENDENT_PANEL_HEIGHT))
            .world_width(DEPENDENT_PANEL_WORLD_WIDTH)
            .anchor(Anchor::Center)
            .layout(|_| {})
            .build()
            .expect("dependent panel should build");
        app.world_mut()
            .spawn((
                panel,
                Transform::default(),
                TestAttachment::new(target, source_anchor, target_anchor),
                ApplicationData,
            ))
            .id()
    }

    fn anchor_world_position(app: &App, entity: Entity, anchor: Anchor) -> Vec3 {
        let geometry = app
            .world()
            .get::<ResolvedAnchorGeometry>(entity)
            .expect("entity should have anchor geometry");
        let point = geometry
            .frame(AnchorSite::from(anchor))
            .expect("geometry should contain the requested anchor");
        app.world()
            .get::<GlobalTransform>(entity)
            .expect("entity should have a global transform")
            .transform_point(point.position().into_inner())
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Option<Entity> {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return None;
        };
        Some(app.world_mut().spawn(panel).id())
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: PanelElementId) -> Option<Entity> {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
    }

    fn callback_system_entity(app: &App, widget: Entity) -> Option<Entity> {
        app.world()
            .get::<ButtonCallbackHandle>(widget)
            .map(ButtonCallbackHandle::system_entity)
    }

    fn system_marker_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&SystemIdMarker>()
            .iter(app.world())
            .count()
    }

    fn pointer_location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  1,
                height: 1,
            },
            position: Vec2::ZERO,
        }
    }

    fn pointer_hit() -> HitData { HitData::new(Entity::PLACEHOLDER, 0.0, None, None) }

    fn press_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            pointer_location(),
            Press {
                button: PointerButton::Primary,
                hit:    pointer_hit(),
                count:  1,
            },
            widget,
        ));
        app.world_mut().flush();
    }

    fn click_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            pointer_location(),
            Click {
                button:   PointerButton::Primary,
                hit:      pointer_hit(),
                duration: std::time::Duration::ZERO,
                count:    1,
            },
            widget,
        ));
        app.world_mut().flush();
    }

    fn release_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(Pointer::new(
            pointer_id,
            pointer_location(),
            Release {
                button: PointerButton::Primary,
                hit:    pointer_hit(),
            },
            widget,
        ));
        app.world_mut().flush();
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[track_caller]
    fn assert_close_3d(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, ANCHOR_EPSILON),
            "expected {expected:?}, got {actual:?}",
        );
    }

    #[test]
    fn reify_creates_child_relationship_and_lookup() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };

        assert!(
            resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none(),
            "a widget must not resolve before its first computed reification",
        );
        app.update();

        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        assert_eq!(
            app.world().get::<PanelWidget>(widget).map(PanelWidget::id),
            Some(&PanelElementId::named("action"))
        );
        assert_eq!(
            app.world().get::<WidgetOf>(widget).map(WidgetOf::panel),
            Some(panel)
        );
        assert_eq!(
            app.world().get::<ChildOf>(widget).map(ChildOf::parent),
            Some(panel)
        );
        assert!(
            app.world()
                .get::<PanelWidgets>(panel)
                .is_some_and(|widgets| widgets.contains(&widget))
        );
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("missing")).is_none());
    }

    #[test]
    fn reify_places_widget_transform_at_its_solved_panel_offset() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::row().size(100.0, 50.0), |builder| {
            builder.with(El::new().size(30.0, 10.0), |_| {});
            builder.with(El::new().size(20.0, 10.0).button("offset"), |_| {});
        });
        let mut app = test_app();
        let panel = spawn_panel(&mut app, builder.build()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should remain live");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("widget should carry a transform");
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        let scale = panel_component.points_to_world();
        let rect = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| {
                computed
                    .widget_records()
                    .iter()
                    .find(|record| record.id() == &PanelElementId::named("offset"))
            })
            .map(crate::widgets::ComputedWidgetRecord::rect);
        let rect = rect.expect("widget should have computed bounds");
        let (center_x, center_y) = rect.center();
        assert_close(transform.translation.x, center_x.mul_add(scale, -anchor_x));
        assert_close(
            transform.translation.y,
            (-center_y).mul_add(scale, anchor_y),
        );
        assert!(app.world().get::<Mesh3d>(widget).is_none());
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(widget)
                .is_none()
        );
    }

    #[test]
    fn centered_fit_panel_reifies_from_final_dimensions() {
        let mut tree = LayoutBuilder::with_root(El::column());
        tree.with(El::new().size(20.0, 10.0).button("centered"), |_| {});
        let panel = DiegeticPanel::world()
            .size(Fit, Fit)
            .anchor(Anchor::Center)
            .with_tree(tree.build())
            .build()
            .expect("fit panel should build");
        let mut app = test_app();
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, PanelElementId::named("centered"))
            .expect("centered widget should be reified");
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should remain live");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("widget should carry a transform");
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        assert!(anchor_x > 0.0);
        assert!(anchor_y > 0.0);
        assert_close(transform.translation.x, 0.0);
        assert_close(transform.translation.y, 0.0);
    }

    #[test]
    fn centered_screen_fit_reifies_off_origin_widget_from_final_dimensions() {
        let mut tree = LayoutBuilder::with_root(El::row());
        tree.with(
            El::new().size(SCREEN_FIT_SPACER_WIDTH, SCREEN_FIT_WIDGET_HEIGHT),
            |_| {},
        );
        tree.with(
            El::new()
                .size(SCREEN_FIT_WIDGET_WIDTH, SCREEN_FIT_WIDGET_HEIGHT)
                .button("screen-fit"),
            |_| {},
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .anchor(Anchor::Center)
            .with_tree(tree.build())
            .build()
            .expect("screen Fit panel should build");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let (scale, anchor_x, anchor_y, rect) = {
            let panel_component = app
                .world()
                .get::<DiegeticPanel>(panel)
                .expect("screen Fit panel should remain live");
            assert_close(
                panel_component.width(),
                SCREEN_FIT_SPACER_WIDTH + SCREEN_FIT_WIDGET_WIDTH,
            );
            assert_close(panel_component.height(), SCREEN_FIT_WIDGET_HEIGHT);
            let computed = app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("screen Fit panel should have computed output");
            computed
                .layout()
                .solved()
                .expect("screen Fit panel should have a computed layout result");
            let record = computed
                .widget_records()
                .iter()
                .find(|record| record.id() == &PanelElementId::named("screen-fit"))
                .expect("screen Fit widget should have a computed record");
            assert!(
                record.rect().x > 0.0,
                "widget should be off the panel origin"
            );
            let (anchor_x, anchor_y) = panel_component.anchor_offsets();
            assert_close(anchor_x, panel_component.width() * 0.5);
            assert_close(anchor_y, panel_component.height() * 0.5);
            (
                panel_component.points_to_world(),
                anchor_x,
                anchor_y,
                record.rect(),
            )
        };
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("screen-fit"))
            .expect("screen Fit widget should be reified");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("screen Fit widget should carry a transform");
        let (center_x, center_y) = rect.center();
        assert_close(transform.translation.x, center_x.mul_add(scale, -anchor_x));
        assert_close(
            transform.translation.y,
            (-center_y).mul_add(scale, anchor_y),
        );
    }

    #[test]
    fn world_anchor_state_tracks_demand_and_rect_changes() {
        let mut app = world_anchor_app();
        let panel = spawn_world_panel(
            &mut app,
            offset_widget_tree(OFFSET_WIDGET_WIDTH),
            Anchor::Center,
            Transform::default(),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");

        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_none());
        assert!(app.world().get::<ValenceAnchoredTo>(widget).is_none());

        let first = spawn_widget_dependent(&mut app, widget, Anchor::Center, Anchor::TopRight);
        let second = spawn_widget_dependent(&mut app, widget, Anchor::Center, Anchor::BottomRight);
        app.update();

        let first_right = app
            .world()
            .get::<ResolvedAnchorGeometry>(widget)
            .and_then(|geometry| geometry.frame(AnchorSite::from(Anchor::TopRight)).ok())
            .map(|point| point.position().into_inner().x)
            .expect("demand should publish widget geometry");
        assert!(app.world().get::<ValenceAnchoredTo>(widget).is_some());
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(widget)
                .map(AnchoredHere::len),
            Some(2),
        );

        app.world_mut().entity_mut(first).remove::<TestAttachment>();
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_some());
        assert!(app.world().get::<ValenceAnchoredTo>(widget).is_some());
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(widget)
                .map(AnchoredHere::len),
            Some(1),
        );

        app.world_mut()
            .commands()
            .set_tree(panel, offset_widget_tree(RESIZED_WIDGET_WIDTH))
            .expect("resized tree should be accepted");
        app.update();
        assert_eq!(
            resolve_widget(&mut app, panel, PanelElementId::named("offset")),
            Some(widget),
        );
        let resized_right = app
            .world()
            .get::<ResolvedAnchorGeometry>(widget)
            .and_then(|geometry| geometry.frame(AnchorSite::from(Anchor::TopRight)).ok())
            .map(|point| point.position().into_inner().x)
            .expect("rect change should refill widget geometry");
        assert!(resized_right > first_right);

        app.world_mut()
            .entity_mut(second)
            .remove::<TestAttachment>();
        app.update();
        assert!(app.world().get::<AnchoredHere>(widget).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_none());
        assert!(app.world().get::<ValenceAnchoredTo>(widget).is_none());
    }

    #[test]
    fn final_screen_demand_preserves_application_replacement_proxy_and_geometry() {
        let mut app = screen_anchor_app();
        let panel = spawn_screen_panel(&mut app, offset_widget_tree(OFFSET_WIDGET_WIDTH));
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let screen_source = app
            .world_mut()
            .spawn(ScreenWidgetAnchoredTo::new(widget))
            .id();
        app.update();

        app.world_mut().entity_mut(widget).insert((
            ScreenWidgetAnchorProxy,
            ResolvedAnchorGeometry::try_new([], Vec::new())
                .expect("an empty proxy geometry has no invalid sites or edges"),
        ));
        app.world_mut()
            .entity_mut(screen_source)
            .remove::<ScreenWidgetAnchoredTo>();
        app.update();

        assert!(
            app.world()
                .get::<ResolvedAnchorGeometry>(widget)
                .is_some_and(
                    |geometry| geometry.frames().next().is_none() && geometry.edges().is_empty()
                )
        );
        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_some());
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ScreenWidgetAnchorProxy>>(widget)
                .is_none()
        );
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ResolvedAnchorGeometry>>(widget)
                .is_none()
        );
    }

    #[test]
    fn active_widget_demand_tracks_later_owner_scale_without_refilling_geometry() {
        let mut app = world_anchor_app();
        let panel = spawn_world_panel(
            &mut app,
            offset_widget_tree(OFFSET_WIDGET_WIDTH),
            Anchor::Center,
            Transform::from_translation(ANCHORED_OWNER_TRANSLATION)
                .with_rotation(Quat::from_rotation_z(ANCHORED_OWNER_ROTATION)),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let authored_center = app
            .world()
            .get::<Transform>(widget)
            .expect("widget should keep its authored panel-local center")
            .translation;
        assert_ne!(
            authored_center,
            Vec3::ZERO,
            "widget should be off the owner panel origin",
        );
        let dependent = spawn_widget_dependent(&mut app, widget, Anchor::Center, Anchor::TopRight);
        app.update();

        let owner_global = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("owner panel should have a global transform");
        assert_close_3d(
            app.world()
                .get::<GlobalTransform>(widget)
                .expect("demanded widget should have a global transform")
                .translation(),
            owner_global.transform_point(authored_center),
        );
        let geometry_tick = app
            .world()
            .entity(widget)
            .get_ref::<ResolvedAnchorGeometry>()
            .expect("demand should publish widget geometry")
            .last_changed();

        app.world_mut()
            .get_mut::<Transform>(panel)
            .expect("owner panel should have a transform")
            .scale = Vec3::splat(UPDATED_OWNER_SCALE);

        // `TransformSystems::Propagate` exposes the edited owner scale after
        // the resolver pass that still reads the prior `GlobalTransform`.
        app.update();
        let propagated_scale = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("owner panel should have a propagated global transform")
            .to_scale_rotation_translation()
            .0;
        assert_close_3d(propagated_scale, Vec3::splat(UPDATED_OWNER_SCALE));

        // This is the first resolver pass that reads the propagated owner scale.
        app.update();

        let owner_global = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("owner panel should retain its global transform");
        assert_close_3d(
            app.world()
                .get::<GlobalTransform>(widget)
                .expect("demanded widget should retain its global transform")
                .translation(),
            owner_global.transform_point(authored_center),
        );
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<ResolvedAnchorGeometry>()
                .expect("demanded widget should retain its geometry")
                .last_changed(),
            geometry_tick,
            "owner scale changes should not rewrite widget geometry",
        );
        assert_close_3d(
            anchor_world_position(&app, dependent, Anchor::Center),
            anchor_world_position(&app, widget, Anchor::TopRight),
        );
        assert!(
            app.world()
                .get::<AnchoredHere>(widget)
                .is_some_and(|demand| demand.contains(&dependent)),
            "dependent should keep active widget demand",
        );
    }

    #[test]
    fn final_demand_retirement_preserves_application_relation_and_removes_owned_geometry() {
        let mut app = world_anchor_app();
        let panel = spawn_world_panel(
            &mut app,
            offset_widget_tree(OFFSET_WIDGET_WIDTH),
            Anchor::Center,
            Transform::default(),
        );
        let application_target = spawn_world_panel(
            &mut app,
            LayoutBuilder::new(OFFSET_PANEL_WIDTH, OFFSET_PANEL_HEIGHT).build(),
            Anchor::Center,
            Transform::default(),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let dependent = spawn_widget_dependent(&mut app, widget, Anchor::Center, Anchor::TopRight);
        app.update();

        let application_relation =
            ValenceAnchoredTo::new(application_target, AnchorSite::Center, AnchorSite::Center);
        app.world_mut()
            .entity_mut(widget)
            .insert(application_relation);
        app.world_mut()
            .entity_mut(dependent)
            .remove::<TestAttachment>();
        app.update();

        assert_eq!(
            app.world().get::<ValenceAnchoredTo>(widget),
            Some(&application_relation),
        );
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ValenceAnchoredTo>>(widget)
                .is_none(),
        );
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_none());
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ResolvedAnchorGeometry>>(widget)
                .is_none(),
        );
    }

    #[test]
    fn anchored_panel_widget_chain_resolves_same_frame_motion_in_order() {
        let mut app = world_anchor_app();
        let base = spawn_world_panel(
            &mut app,
            LayoutBuilder::new(OFFSET_PANEL_WIDTH, OFFSET_PANEL_HEIGHT).build(),
            Anchor::Center,
            Transform::from_translation(ANCHORED_OWNER_TRANSLATION)
                .with_rotation(Quat::from_rotation_z(ANCHORED_OWNER_ROTATION)),
        );
        let panel = spawn_world_panel(
            &mut app,
            offset_widget_tree(OFFSET_WIDGET_WIDTH),
            Anchor::Center,
            Transform::default(),
        );
        app.world_mut().entity_mut(panel).insert((
            TestAttachment::new(base, Anchor::Center, Anchor::Center),
            AnchorPose::default(),
        ));
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let dependent =
            spawn_widget_dependent(&mut app, widget, Anchor::TopLeft, Anchor::BottomRight);
        app.world_mut()
            .get_mut::<AnchorPose>(panel)
            .expect("anchored owner should have a pose")
            .translation = Displacement::from(ANCHORED_POSE_TRANSLATION);

        app.update();

        assert_close_3d(
            anchor_world_position(&app, dependent, Anchor::TopLeft),
            anchor_world_position(&app, widget, Anchor::BottomRight),
        );
    }

    #[test]
    fn panel_role_teardown_detaches_widget_target_dependents() {
        let mut app = world_anchor_app();
        let panel = spawn_world_panel(
            &mut app,
            offset_widget_tree(OFFSET_WIDGET_WIDTH),
            Anchor::Center,
            Transform::default(),
        );
        app.world_mut().entity_mut(panel).insert(ApplicationData);
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let dependent =
            spawn_widget_dependent(&mut app, widget, Anchor::Center, Anchor::BottomCenter);
        app.update();
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_some());
        assert!(app.world().get::<ValenceAnchoredTo>(widget).is_some());
        assert!(app.world().get::<ValenceAnchoredTo>(dependent).is_some());

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get::<ApplicationData>(panel).is_some());
        assert!(app.world().get_entity(widget).is_err());
        assert!(app.world().get_entity(dependent).is_ok());
        assert!(app.world().get::<ApplicationData>(dependent).is_some());
        assert!(
            app.world()
                .get::<PanelAttachmentAuthored>(dependent)
                .is_none()
        );
        assert!(app.world().get::<ValenceAnchoredTo>(dependent).is_none());
        assert!(app.world().get::<ResolvedAnchorOffset>(dependent).is_none());
        assert!(
            app.world()
                .get::<AnchoredHere>(panel)
                .is_none_or(|demand| !demand.contains(&widget)),
        );
    }

    #[test]
    fn removing_panel_role_despawns_owned_children_and_preserves_application_state() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(&["action"])).expect("panel should build");
        app.world_mut().entity_mut(panel).insert(ApplicationData);
        let application_child = app
            .world_mut()
            .spawn((ApplicationData, ChildOf(panel)))
            .id();
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get::<ApplicationData>(panel).is_some());
        assert!(app.world().get_entity(application_child).is_ok());
        assert!(app.world().get::<PanelWidgetIndex>(panel).is_none());
        assert!(app.world().get::<ComputedDiegeticPanel>(panel).is_none());
        assert!(app.world().get::<PanelWidgets>(panel).is_none());
        assert!(app.world().get_entity(widget).is_err());
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none());
    }

    #[test]
    fn identical_ids_resolve_independently_per_panel() {
        let mut app = test_app();
        let Some(first_panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        let Some(second_panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        app.update();

        let first = resolve_widget(&mut app, first_panel, PanelElementId::named("action"));
        let second = resolve_widget(&mut app, second_panel, PanelElementId::named("action"));
        assert!(first.is_some());
        assert!(second.is_some());
        assert_ne!(first, second);
    }

    #[test]
    fn identical_tree_replacement_preserves_widget_lookup() {
        let mut app = test_app();
        let tree = widget_tree(&["action"]);
        let Some(panel) = spawn_panel(&mut app, tree.clone()) else {
            return;
        };
        app.update();

        let before = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        assert!(before.is_some());
        let revision = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map(DiegeticPanel::tree_revision)
            .map(u64::from);

        let result = app.world_mut().commands().set_tree(panel, tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        assert_eq!(after, before);
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .map(DiegeticPanel::tree_revision)
                .map(u64::from),
            revision.map(|revision| revision + 1)
        );
    }

    #[test]
    fn reorder_reuses_entities_and_refreshes_preorder() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["first", "second"])) else {
            return;
        };
        app.update();
        let first_before = resolve_widget(&mut app, panel, PanelElementId::named("first"));
        let second_before = resolve_widget(&mut app, panel, PanelElementId::named("second"));

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["second", "first"]));
        assert!(result.is_ok());
        app.update();

        let first_after = resolve_widget(&mut app, panel, PanelElementId::named("first"));
        let second_after = resolve_widget(&mut app, panel, PanelElementId::named("second"));
        assert_eq!(first_before, first_after);
        assert_eq!(second_before, second_after);
        let Some(first) = first_after else {
            return;
        };
        let Some(second) = second_after else {
            return;
        };
        assert!(
            app.world()
                .get::<WidgetPreorder>(second)
                .zip(app.world().get::<WidgetPreorder>(first))
                .is_some_and(|(second_order, first_order)| second_order.0 < first_order.0)
        );
    }

    #[test]
    fn reified_widget_carries_four_appearance_cascades_without_state_appearance() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, appearance_tree(Color::srgb(1.0, 0.0, 0.0))) else {
            return;
        };
        app.update();
        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        let entity = app.world().entity(widget);

        assert!(matches!(
            entity.get::<Cascade<WidgetHoveredAppearance>>(),
            Some(Cascade::Override(_))
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetPressedAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetFocusedAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetDisabledAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(!entity.contains_type_id(TypeId::of::<StateAppearance>()));
    }

    #[test]
    fn reauthoring_one_state_reinserts_only_its_cascade_channel() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, appearance_tree(Color::srgb(1.0, 0.0, 0.0))) else {
            return;
        };
        app.update();
        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        let before = (
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetHoveredAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetPressedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetFocusedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetDisabledAppearance>>()
                .map(|cascade| cascade.last_changed()),
        );
        let (
            Some(before_hovered),
            Some(before_pressed),
            Some(before_focused),
            Some(before_disabled),
        ) = before
        else {
            return;
        };

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, appearance_tree(Color::srgb(0.0, 0.0, 1.0)))
                .is_ok()
        );
        app.update();

        let after = (
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetHoveredAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetPressedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetFocusedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetDisabledAppearance>>()
                .map(|cascade| cascade.last_changed()),
        );
        let (Some(after_hovered), Some(after_pressed), Some(after_focused), Some(after_disabled)) =
            after
        else {
            return;
        };

        assert_ne!(after_hovered, before_hovered);
        assert_eq!(after_pressed, before_pressed);
        assert_eq!(after_focused, before_focused);
        assert_eq!(after_disabled, before_disabled);
    }

    #[test]
    fn reifying_structure_changes_replaces_part_appearance_indices() {
        let mut app = test_app();
        let tree = part_appearance_tree(false);
        let panel = spawn_panel(&mut app, tree).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let before = part_appearance_keys(&app, widget)
            .expect("widget should carry reified part appearances");
        assert_eq!(before.len(), 1);

        let tree = part_appearance_tree(true);
        assert!(app.world_mut().commands().set_tree(panel, tree).is_ok());
        app.update();

        let after = part_appearance_keys(&app, widget)
            .expect("widget should replace its reified part appearances");
        assert_eq!(after.len(), 1);
        assert_ne!(after, before);
        assert!(before.iter().all(|index| !after.contains(index)));
    }

    #[test]
    fn inline_editor_round_trip_replaces_part_appearance_indices() {
        let mut app = test_app();
        let mut authoritative_tree = editable_display_tree();
        let panel = spawn_panel(&mut app, authoritative_tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("gain"))
            .expect("field should be reified");
        let display_before = part_appearance_keys(&app, widget)
            .expect("field should carry reified part appearances");
        assert_eq!(display_before.len(), 1);

        let editor_content = inline_editor_content_tree();
        let mut editor_tree = authoritative_tree.clone();
        assert_eq!(
            editor_tree.set_field_editing_content(&PanelElementId::named("gain"), &editor_content),
            crate::layout::FieldDisplayTextUpdate::Updated,
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, editor_tree)
                .is_ok()
        );
        app.update();

        let editor = part_appearance_keys(&app, widget)
            .expect("field should replace its reified part appearances");
        assert_eq!(editor.len(), 1);
        assert_ne!(editor, display_before);
        assert!(display_before.iter().all(|index| !editor.contains(index)));

        assert_eq!(
            authoritative_tree.set_field_display_text(&PanelElementId::named("gain"), "edited"),
            crate::layout::FieldDisplayTextUpdate::Updated,
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, authoritative_tree)
                .is_ok()
        );
        app.update();

        let display_after = part_appearance_keys(&app, widget)
            .expect("field should restore its reified part appearances");
        assert_eq!(display_after, display_before);
        assert!(editor.iter().all(|index| !display_after.contains(index)));
    }

    #[test]
    fn removal_sweeps_widget_and_stale_lookup_returns_none() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["keep", "remove"])) else {
            return;
        };
        app.update();
        let removed = resolve_widget(&mut app, panel, PanelElementId::named("remove"));
        assert!(removed.is_some());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["keep"]));
        assert!(result.is_ok());
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("remove")).is_none());
        assert!(removed.is_some_and(|entity| app.world().get_entity(entity).is_err()));
    }

    #[test]
    fn kind_replacement_retains_entity_and_replaces_authored_snapshot() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(&["control"]));
        assert!(panel.is_some());
        let Some(panel) = panel else {
            return;
        };
        app.update();
        let before = resolve_widget(&mut app, panel, PanelElementId::named("control"));
        assert!(before.is_some());
        let Some(before) = before else {
            return;
        };
        let interactivity_tick = app
            .world()
            .entity(before)
            .get_ref::<Cascade<WidgetInteractivity>>()
            .map(|authored| authored.last_changed());
        let relationship_tick = app
            .world()
            .entity(before)
            .get_ref::<CascadeFrom>()
            .map(|relationship| relationship.last_changed());
        let tree = slider_tree("control", 4.0);
        let expected_authored = slider_spec(4.0);

        let result = app.world_mut().commands().set_tree(panel, tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("control"));
        assert_eq!(after, Some(before));
        let Some(widget) = after else {
            return;
        };
        assert_eq!(
            app.world().get::<WidgetKind>(widget),
            Some(&WidgetKind::Slider)
        );
        assert_eq!(
            app.world().get::<WidgetSpec>(widget),
            Some(&expected_authored)
        );
        assert!(matches!(expected_authored, WidgetSpec::Slider(_)));
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetInteractivity>>()
                .map(|authored| authored.last_changed()),
            interactivity_tick
        );
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<CascadeFrom>()
                .map(|relationship| relationship.last_changed()),
            relationship_tick
        );
    }

    #[test]
    fn visual_only_slider_edit_refreshes_snapshot_without_replacing_entity() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, slider_tree("level", 2.0)) else {
            return;
        };
        app.update();
        let before = resolve_widget(&mut app, panel, PanelElementId::named("level"));
        let next_tree = slider_tree("level", 8.0);

        let result = app.world_mut().commands().set_tree(panel, next_tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("level"));
        assert_eq!(before, after);
        let Some(widget) = after else {
            return;
        };
        let expected = slider_spec(8.0);
        assert_eq!(app.world().get::<WidgetSpec>(widget), Some(&expected));
    }

    #[test]
    fn visual_only_refresh_preserves_geometry_and_updates_rank_without_layout() {
        let mut app = test_app();
        let first_tree = ranked_slider_tree(2.0, -1);
        let panel = spawn_panel(&mut app, first_tree).expect("panel should build");
        app.update();
        let before_entity = resolve_widget(&mut app, panel, PanelElementId::named("level"))
            .expect("slider should be reified");
        let (rect, clipped_rect, rank, interactivity, layout_solves) = {
            let computed = app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should have computed output");
            let record = computed
                .widget_records()
                .iter()
                .find(|record| record.id() == &PanelElementId::named("level"))
                .expect("slider should have a computed record");
            (
                record.rect(),
                record.clipped_rect(),
                record.interaction_rank(),
                record.interactivity(),
                computed.layout_solves(),
            )
        };
        let next_tree = ranked_slider_tree(8.0, 1);

        app.world_mut()
            .commands()
            .set_tree(panel, next_tree)
            .expect("visual-only replacement should be accepted");
        app.update();

        let after_entity = resolve_widget(&mut app, panel, PanelElementId::named("level"))
            .expect("slider should remain reified");
        assert_eq!(after_entity, before_entity);
        let expected = slider_spec(8.0);
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("panel should retain computed output");
        let record = computed
            .widget_records()
            .iter()
            .find(|record| record.id() == &PanelElementId::named("level"))
            .expect("slider should retain its computed record");
        assert_eq!(record.rect(), rect);
        assert_eq!(record.clipped_rect(), clipped_rect);
        assert_ne!(record.interaction_rank(), rank);
        assert_eq!(record.authored(), &expected);
        assert_eq!(record.interactivity(), interactivity);
        assert_eq!(computed.layout_solves(), layout_solves);
    }

    #[test]
    fn stale_entity_and_panel_despawn_are_safe() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        let Some(widget) = widget else {
            return;
        };
        app.world_mut().entity_mut(widget).despawn();
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["replacement"]));
        assert!(result.is_ok());
        app.update();
        let replacement = resolve_widget(&mut app, panel, PanelElementId::named("replacement"));
        assert!(replacement.is_some());
        app.world_mut().entity_mut(panel).despawn();
        assert!(replacement.is_some_and(|entity| app.world().get_entity(entity).is_err()));
    }

    #[test]
    fn identical_tree_replacement_reuses_the_tracked_callback_without_reregistering() {
        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let tree = callback_tree(Button::new().on_click(record_authored_callback));
        let panel = spawn_panel(&mut app, tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let system_entity = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");

        // The first replacement also registers `set_tree`'s own cached command
        // system, so the registered-system count is measured after it.
        app.world_mut()
            .commands()
            .set_tree(panel, tree.clone())
            .expect("identical tree should be accepted");
        app.update();
        assert_eq!(callback_system_entity(&app, widget), Some(system_entity));
        let registered_systems = system_marker_count(&mut app);

        app.world_mut()
            .commands()
            .set_tree(panel, tree)
            .expect("repeated identical tree should be accepted");
        app.update();

        assert_eq!(callback_system_entity(&app, widget), Some(system_entity));
        assert_eq!(system_marker_count(&mut app), registered_systems);

        let pointer_id = PointerId::Mouse;
        press_widget(&mut app, widget, pointer_id);
        click_widget(&mut app, widget, pointer_id);
        release_widget(&mut app, widget, pointer_id);
        assert_eq!(
            app.world().resource::<CallbackRuns>().0,
            [(CallbackSource::Authored, Some(pointer_id))]
        );
    }

    #[test]
    fn callback_replacement_during_live_press_swaps_the_handle_and_keeps_the_press() {
        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let panel = spawn_panel(
            &mut app,
            callback_tree(Button::new().on_click(record_authored_callback)),
        )
        .expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let authored_system = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");

        let pointer_id = PointerId::Mouse;
        press_widget(&mut app, widget, pointer_id);
        assert!(app.world().get::<ButtonPress>(widget).is_some());

        app.world_mut()
            .commands()
            .set_tree(
                panel,
                callback_tree(Button::new().on_click(record_replacement_callback)),
            )
            .expect("replacement tree should be accepted");
        app.update();
        app.update();

        let replacement_system = callback_system_entity(&app, widget)
            .expect("replacement should install exactly one tracked handle");
        assert_ne!(replacement_system, authored_system);
        assert!(
            app.world().get_entity(authored_system).is_err(),
            "replacement must release only the prior tracked handle",
        );
        assert!(app.world().get_entity(replacement_system).is_ok());
        assert!(
            app.world().get::<ButtonPress>(widget).is_some(),
            "callback replacement must not disturb the live press",
        );

        click_widget(&mut app, widget, pointer_id);
        release_widget(&mut app, widget, pointer_id);
        assert_eq!(
            app.world().resource::<CallbackRuns>().0,
            [(CallbackSource::Replacement, Some(pointer_id))]
        );
    }

    #[test]
    fn widget_removal_releases_the_final_tracked_callback() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            callback_tree(Button::new().on_click(record_authored_callback)),
        )
        .expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let system_entity = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");
        assert!(app.world().get_entity(system_entity).is_ok());

        app.world_mut()
            .commands()
            .set_tree(panel, LayoutBuilder::new(100.0, 50.0).build())
            .expect("empty tree should be accepted");
        app.update();
        app.update();

        assert!(app.world().get_entity(widget).is_err());
        assert!(
            app.world().get_entity(system_entity).is_err(),
            "final tracked-handle drop must release the registered system",
        );
    }

    #[test]
    fn slot_attachment_and_overrides_preserve_authored_spec_and_callback() {
        const TEST_SLOT: VisualSlotId = VisualSlotId::new(3);

        fn slotted_callback_tree(button: Button) -> LayoutTree {
            let mut builder = LayoutBuilder::new(100.0, 50.0);
            builder.with(
                El::new()
                    .background(Color::WHITE)
                    .widget("action", button)
                    .visual_slot(TEST_SLOT),
                |_| {},
            );
            builder.build()
        }

        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let button = Button::new().on_click(record_authored_callback);
        let panel =
            spawn_panel(&mut app, callback_tree(button.clone())).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let callback_system = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");
        let authored_spec = app
            .world()
            .get::<WidgetSpec>(widget)
            .cloned()
            .expect("widget should carry its authored spec");
        assert!(
            app.world()
                .get::<WidgetVisualSlots>(widget)
                .is_some_and(|slots| slots.element_index(TEST_SLOT).is_none()),
            "an unslotted tree reifies empty slot references",
        );

        app.world_mut()
            .commands()
            .set_tree(panel, slotted_callback_tree(button))
            .expect("slotted tree should be accepted");
        app.update();

        let element_index = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(TEST_SLOT))
            .expect("slot attachment should reify onto the widget");
        assert_eq!(
            callback_system_entity(&app, widget),
            Some(callback_system),
            "slot attachment must preserve the tracked callback handle",
        );
        assert_eq!(
            app.world().get::<WidgetSpec>(widget),
            Some(&authored_spec),
            "slot attachment must preserve the authored spec",
        );

        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            TEST_SLOT,
            VisualSlotOverride::default().with_fill_color(Color::BLACK),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();

        assert!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, element_index)
                .is_some(),
            "the dispatched override should resolve through the reified slot",
        );
        assert_eq!(
            callback_system_entity(&app, widget),
            Some(callback_system),
            "slot updates must retain the unchanged button's callback system",
        );
        assert_eq!(app.world().get::<WidgetSpec>(widget), Some(&authored_spec));
    }
}
