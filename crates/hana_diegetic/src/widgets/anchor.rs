use std::ops::Deref;

use bevy::ecs::change_detection::Ref;
use bevy::prelude::Add;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
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
use super::WidgetOf;
use crate::layout::Anchor;
use crate::panel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelComponentOwnership;
use crate::panel::PanelSpace;

/// Private lowering from a screen panel attachment to its widget target.
///
/// The matching [`ScreenWidgetAnchoredHere`] component is the authoritative
/// screen-space geometry demand. This relation has no `linked_spawn`; panel
/// role teardown removes the authored attachment while the widget is still
/// queryable.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
#[relationship(relationship_target = ScreenWidgetAnchoredHere)]
pub(crate) struct ScreenWidgetAnchoredTo(
    #[relationship]
    #[entities]
    Entity,
);

impl ScreenWidgetAnchoredTo {
    pub(crate) const fn new(target: Entity) -> Self { Self(target) }

    pub(crate) const fn target(self) -> Entity { self.0 }
}

impl FromWorld for ScreenWidgetAnchoredTo {
    fn from_world(_: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Reverse membership for screen panels attached to one widget.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = ScreenWidgetAnchoredTo)]
pub(crate) struct ScreenWidgetAnchoredHere(Vec<Entity>);

impl ScreenWidgetAnchoredHere {
    pub(crate) fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }

    pub(crate) const fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl Deref for ScreenWidgetAnchoredHere {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Marks a widget that contributes a synthetic screen resolver candidate.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(crate) struct ScreenWidgetAnchorProxy;

/// Where a widget's tooltip slides first, in its panel's layout space.
#[derive(Clone, Copy, Component, Debug, PartialEq)]
pub(crate) enum TooltipBoundary {
    /// Bounds of the nearest ancestor marked with
    /// [`El::tooltip_boundary`](crate::El::tooltip_boundary).
    Element(crate::BoundingBox),
    /// No marked ancestor: only the placement area bounds the tooltip.
    PlacementAreaOnly,
}

#[derive(Clone, Copy, Component, Debug, PartialEq)]
pub(crate) struct WidgetAnchorRect {
    panel_offset: Vec3,
    size:         Vec2,
    anchor:       Anchor,
    space:        PanelSpace,
}

impl WidgetAnchorRect {
    pub(in crate::widgets) fn new(panel: &DiegeticPanel, rect: crate::BoundingBox) -> Self {
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

    pub(in crate::widgets) const fn transform(self) -> Transform {
        Transform::from_translation(self.panel_offset)
    }
}

pub(crate) fn on_screen_widget_demand_added(
    added: On<Add<ScreenWidgetAnchoredHere>>,
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
    removed: On<Remove<ScreenWidgetAnchoredHere>>,
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

pub(in crate::widgets) fn update_world_anchor_geometry(
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
        (With<PanelWidget>, Without<ScreenWidgetAnchorProxy>),
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
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Bundle;
    use bevy::prelude::Component;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Quat;
    use bevy::prelude::Transform;
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

    use super::*;
    use crate::Anchor;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::PanelComponentOwnership;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ScreenWidgetAnchorProxy;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::WidgetsPlugin;

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
    const UPDATED_OWNER_SCALE: f32 = 1.75;

    #[derive(Component)]
    struct ApplicationData;

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

    fn resolve_widget(app: &mut App, panel: Entity, id: PanelElementId) -> Option<Entity> {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
    }

    #[track_caller]
    fn assert_close_3d(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, ANCHOR_EPSILON),
            "expected {expected:?}, got {actual:?}",
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
    fn replacing_screen_anchor_source_keeps_widget_proxy_and_geometry() {
        let mut app = screen_anchor_app();
        let panel = spawn_screen_panel(&mut app, offset_widget_tree(OFFSET_WIDGET_WIDTH));
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let first_source = app
            .world_mut()
            .spawn(ScreenWidgetAnchoredTo::new(widget))
            .id();
        app.update();
        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_some());

        app.world_mut().despawn(first_source);
        app.world_mut().spawn(ScreenWidgetAnchoredTo::new(widget));
        app.update();

        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_some());
        assert!(app.world().get::<WidgetAnchorRect>(widget).is_some());
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_some());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(widget)
                .is_some_and(|demand| demand.len() == 1)
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
    fn screen_widget_relationship_tracks_retarget_remove_and_despawn() {
        let mut world = World::new();
        let first_target = world.spawn_empty().id();
        let second_target = world.spawn_empty().id();
        let first_source = world.spawn(ScreenWidgetAnchoredTo::new(first_target)).id();
        let second_source = world.spawn(ScreenWidgetAnchoredTo::new(first_target)).id();

        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![first_source, second_source],
        );

        world
            .entity_mut(first_source)
            .insert(ScreenWidgetAnchoredTo::new(second_target));
        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![second_source],
        );
        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(second_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![first_source],
        );

        world
            .entity_mut(second_source)
            .remove::<ScreenWidgetAnchoredTo>();
        assert!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .is_none()
        );

        world.despawn(first_source);
        assert!(
            world
                .get::<ScreenWidgetAnchoredHere>(second_target)
                .is_none()
        );
    }
}
