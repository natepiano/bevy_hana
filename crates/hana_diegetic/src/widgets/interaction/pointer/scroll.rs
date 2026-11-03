use bevy::ecs::system::SystemParam;
use bevy::input::mouse::MouseScrollUnit;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::PointerScroll;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::World;

use super::size_gate::WidgetSizePicking;
use crate::ComputedDiegeticPanel;
use crate::ComputedScrollRecord;
use crate::DiegeticPanel;
use crate::PanelElementId;
use crate::ScrollExtent;
use crate::ScrollRecordDrive;
use crate::panel::ScrollRecordUnderPoint;
use crate::panel::scroll_motion::ScrollInput;
use crate::panel::scroll_view::PanelScrollViews;
use crate::panel::scroll_view::ScrollViewsInMotion;
use crate::render;
use crate::widgets::WidgetOf;
use crate::widgets::constants::WHEEL_LINES_PER_NOTCH;

/// Reports a wheel over a named scrolling element of a panel.
///
/// The event carries the requested movement, not a new offset: the owner of
/// the panel's tree holds the offset, applies `delta` with
/// [`ScrollExtent::scrolled`], and rebuilds the tree.
#[derive(Clone, Debug, EntityEvent)]
pub struct PanelScrolled {
    /// Panel whose element the wheel moved over.
    #[event_target]
    pub panel:      Entity,
    /// Id of the innermost named scrolling element under the pointer.
    pub element:    PanelElementId,
    /// Requested scroll movement in the panel's layout unit; wheel-up is
    /// negative, so a positive `y` moves toward the end of the content.
    pub delta:      Vec2,
    /// Vertical wheel movement in notches; positive moves toward the end of the content.
    pub notches:    f32,
    /// Content and viewport extent of the element at the latest layout solve.
    pub extent:     ScrollExtent,
    /// Pointer that scrolled.
    pub pointer_id: PointerId,
}

/// What a pointer's wheel would scroll if it turned now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerScrollTarget {
    /// A named scrolling element with somewhere to scroll lies under the
    /// pointer, on the panel directly or beneath one of its widgets.
    Scrollable {
        /// Panel that owns the element.
        panel:   Entity,
        /// Id of the innermost named scrolling element under the pointer.
        element: PanelElementId,
    },
    /// Nothing under the pointer scrolls: no panel, a panel point outside every
    /// named scrolling element, or an element whose content fits its viewport.
    NothingScrollable,
}

/// Panels whose layout a wheel hit is resolved against.
pub(super) type ScrollPanels<'w, 's> = Query<
    'w,
    's,
    (
        &'static DiegeticPanel,
        &'static ComputedDiegeticPanel,
        &'static GlobalTransform,
    ),
>;

/// Scrolling element a picking hit lands on.
pub(super) enum ScrollRecordAtHit<'a> {
    /// The hit projects inside a named scrolling element of `panel`.
    Inside {
        /// Panel that owns the element.
        panel:       Entity,
        /// Layout unit of the panel, in points.
        unit_points: f32,
        /// The innermost named scrolling element under the hit.
        record:      &'a ComputedScrollRecord,
    },
    /// The hit is on no panel or widget, carries no position, or lands outside
    /// every named scrolling element.
    OutsideScrollingElements,
}

/// Resolves a picking hit to the scrolling element under it.
///
/// A panel hit reads that panel; a widget hit reads its [`WidgetOf`] panel,
/// because widgets block the panel beneath them from the hover map. Both
/// project the hit position onto the panel's layout.
fn scroll_record_at_hit<'a>(
    hit_entity: Entity,
    position: Option<Vec3>,
    widgets: &Query<&WidgetOf>,
    panels: &'a ScrollPanels,
    camera: Entity,
    size_picking: &WidgetSizePicking,
) -> ScrollRecordAtHit<'a> {
    let panel_entity = widgets.get(hit_entity).map_or(hit_entity, WidgetOf::panel);
    let Ok((panel, computed, transform)) = panels.get(panel_entity) else {
        return ScrollRecordAtHit::OutsideScrollingElements;
    };
    let Some(panel_local) =
        position.and_then(|position| render::project_flat_panel_hit(position, panel, transform))
    else {
        return ScrollRecordAtHit::OutsideScrollingElements;
    };
    match computed.scroll_record_at_local_position(panel_local) {
        ScrollRecordUnderPoint::Inside(record)
            if size_picking.accepts_scroll(panel_entity, record, panel, transform, camera) =>
        {
            ScrollRecordAtHit::Inside {
                panel: panel_entity,
                unit_points: panel.layout_unit().to_points(),
                record,
            }
        },
        ScrollRecordUnderPoint::Inside(_) | ScrollRecordUnderPoint::OutsideScrollingElements => {
            ScrollRecordAtHit::OutsideScrollingElements
        },
    }
}

/// Reads what each pointer's wheel would scroll from picking's hover map.
///
/// Apps without picking have no [`HoverMap`]; every pointer then scrolls
/// nothing.
#[derive(SystemParam)]
pub struct HoveredScrollTargets<'w, 's> {
    hover_map:    Option<Res<'w, HoverMap>>,
    widgets:      Query<'w, 's, &'static WidgetOf>,
    panels:       ScrollPanels<'w, 's>,
    size_picking: WidgetSizePicking<'w, 's>,
}

impl HoveredScrollTargets<'_, '_> {
    /// What the nearest hit of `pointer` would scroll.
    ///
    /// The nearest hit decides alone: a panel in front of a scrollable body
    /// hides it. Only an element with content beyond its viewport is
    /// [`PointerScrollTarget::Scrollable`]; a hovered widget row resolves to the
    /// body it sits in. The body must also pass the projected size gate for
    /// the picking camera; otherwise the wheel stays available to the owner.
    #[must_use]
    pub fn under(&mut self, pointer: PointerId) -> PointerScrollTarget {
        let Some((&entity, hit)) = self
            .hover_map
            .as_ref()
            .and_then(|hover_map| hover_map.get(&pointer))
            .and_then(|hovered| {
                hovered
                    .iter()
                    .min_by(|(_, near), (_, far)| near.depth.total_cmp(&far.depth))
            })
        else {
            return PointerScrollTarget::NothingScrollable;
        };
        match scroll_record_at_hit(
            entity,
            hit.position,
            &self.widgets,
            &self.panels,
            hit.camera,
            &self.size_picking,
        ) {
            ScrollRecordAtHit::Inside { panel, record, .. }
                if record.extent.max_offset().y > 0.0 =>
            {
                PointerScrollTarget::Scrollable {
                    panel,
                    element: record.id.clone(),
                }
            },
            ScrollRecordAtHit::Inside { .. } | ScrollRecordAtHit::OutsideScrollingElements => {
                PointerScrollTarget::NothingScrollable
            },
        }
    }
}

/// Converts a wheel over a panel's named scrolling element into
/// [`PanelScrolled`].
///
/// Each wheel is read once, at the entity the pointer hit; the copies that
/// bubble to its ancestors are the same wheel. A hit on a panel scrolls that
/// panel. A hit on a widget scrolls the widget's panel, unless the panel is
/// also hovered by the same pointer: the panel then receives its own delivery,
/// and reading both would scroll twice.
///
/// A wheel line moves the element by [`WHEEL_LINES_PER_NOTCH`] of its measured
/// line pitch; a pixel moves it one point.
pub(in crate::widgets) fn scroll_from_pointer(
    mut scroll: On<PointerScroll>,
    widgets: Query<&WidgetOf>,
    panels: ScrollPanels,
    hover_map: Option<Res<HoverMap>>,
    mut commands: Commands,
    size_picking: WidgetSizePicking,
) {
    let hit_entity = scroll.original_event_target();
    if scroll.event_target() != hit_entity {
        return;
    }
    if let Ok(widget_of) = widgets.get(hit_entity) {
        let panel_hovered = hover_map.is_some_and(|hover_map| {
            hover_map
                .get(&scroll.pointer.id)
                .is_some_and(|hovered| hovered.contains_key(&widget_of.panel()))
        });
        if panel_hovered {
            return;
        }
    }
    let ScrollRecordAtHit::Inside {
        panel,
        unit_points,
        record,
    } = scroll_record_at_hit(
        hit_entity,
        scroll.hit.position,
        &widgets,
        &panels,
        scroll.hit.camera,
        &size_picking,
    )
    else {
        return;
    };
    let wheel = Vec2::new(-scroll.x, -scroll.y);
    let delta = match scroll.unit {
        MouseScrollUnit::Line => wheel * WHEEL_LINES_PER_NOTCH * record.line_pitch,
        MouseScrollUnit::Pixel => wheel / unit_points,
    };
    if record.drive == ScrollRecordDrive::Library {
        if record.extent.max_offset().y > 0.0 {
            let record = record.clone();
            let input = match scroll.unit {
                MouseScrollUnit::Line => ScrollInput::Notches { distance: delta.y },
                MouseScrollUnit::Pixel => ScrollInput::Pixels {
                    distance: delta.y,
                    phase:    scroll.phase,
                },
            };
            commands.queue(move |world: &mut World| {
                let Ok(mut entity) = world.get_entity_mut(panel) else {
                    return;
                };
                if let Some(mut views) = entity.get_mut::<PanelScrollViews>() {
                    views.input(&record, input);
                } else {
                    let mut views = PanelScrollViews::default();
                    views.input(&record, input);
                    entity.insert(views);
                }
                entity.insert(ScrollViewsInMotion);
            });
            scroll.propagate(false);
        }
        return;
    }
    let event = PanelScrolled {
        panel,
        element: record.id.clone(),
        delta,
        notches: match scroll.unit {
            MouseScrollUnit::Line => wheel.y,
            MouseScrollUnit::Pixel => delta.y / (record.line_pitch * WHEEL_LINES_PER_NOTCH),
        },
        extent: record.extent,
        pointer_id: scroll.pointer.id,
    };
    scroll.propagate(false);
    commands.trigger(event);
}

#[cfg(test)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::mouse::MouseScrollUnit;
    use bevy::input::touch::TouchPhase;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::PickingSettings;
    use bevy::picking::PickingSystems;
    use bevy::picking::backend::HitData;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::events::pointer_events;
    use bevy::picking::hover::HoverMap;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;

    use super::HoveredScrollTargets;
    use super::PanelScrolled;
    use super::PointerScrollTarget;
    use super::WHEEL_LINES_PER_NOTCH;
    use crate::ComputedDiegeticPanel;
    use crate::ComputedScrollRecord;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Sizing;
    use crate::layout::BoundingBox;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;

    const BODY_ID: &str = "body";
    const ROW_BUTTON_ID: &str = "row-action";
    const HEADER_HEIGHT: f32 = 10.0;
    const BODY_HEIGHT: f32 = 40.0;
    const ROW_HEIGHT: f32 = 20.0;
    const ROW_COUNT: usize = 5;
    const POINTER: PointerId = PointerId::Mouse;

    #[derive(Default, Resource)]
    struct RecordedScrolls(Vec<PanelScrolled>);

    fn record_scroll(event: On<PanelScrolled>, mut scrolls: ResMut<RecordedScrolls>) {
        scrolls.0.push(event.event().clone());
    }

    /// Layout and widgets headless, with picking's hover state and pointer
    /// event dispatch installed but no backend: each test seeds the
    /// [`HoverMap`] a backend would have produced.
    fn scroll_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedScrolls>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, InteractionPlugin))
            .configure_sets(
                PreUpdate,
                PickingSystems::Hover.run_if(PickingSettings::hover_should_run),
            )
            .init_resource::<PointerMap>()
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .add_observer(record_scroll);
        app.world_mut()
            .spawn((POINTER, PointerLocation::new(location())));
        app
    }

    /// A fixed header above a named body that scrolls five rows through a
    /// viewport two rows tall; the first row is a button.
    fn header_and_scrolling_body_tree() -> LayoutTree {
        let mut builder =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::fixed(HEADER_HEIGHT)),
            |_| {},
        );
        builder.with(
            El::column()
                .id(BODY_ID)
                .width(Sizing::GROW)
                .height(Sizing::fixed(BODY_HEIGHT))
                .scroll_y(0.0),
            |body| {
                body.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::fixed(ROW_HEIGHT))
                        .button(ROW_BUTTON_ID),
                    |_| {},
                );
                for _ in 1..ROW_COUNT {
                    body.with(
                        El::new()
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(ROW_HEIGHT)),
                        |_| {},
                    );
                }
            },
        );
        builder.build()
    }

    fn spawn_solved_panel(app: &mut App) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(header_and_scrolling_body_tree())
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        app.world_mut().flush();
        panel
    }

    fn row_widget(app: &mut App, panel: Entity) -> Option<Entity> {
        let id = PanelElementId::named(ROW_BUTTON_ID);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
    }

    /// Inverts `project_flat_panel_hit`: the world position whose projection
    /// is `panel_local`.
    fn world_position(app: &App, panel: Entity, panel_local: Vec2) -> Option<Vec3> {
        let panel_data = app.world().get::<DiegeticPanel>(panel)?;
        let transform = app.world().get::<GlobalTransform>(panel)?;
        let points_to_world = panel_data.points_to_world();
        let (anchor_x, anchor_y) = panel_data.anchor_offsets();
        let local = Vec3::new(
            panel_local.x.mul_add(points_to_world, -anchor_x),
            (-panel_local.y).mul_add(points_to_world, anchor_y),
            0.0,
        );
        Some(transform.transform_point(local))
    }

    fn body_record(app: &App, panel: Entity) -> Option<ComputedScrollRecord> {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)?
            .scroll_records()
            .iter()
            .find(|record| record.id == PanelElementId::named(BODY_ID))
            .cloned()
    }

    /// Panel-local bounds of the body's scroll record, in layout points.
    fn body_bounds(app: &App, panel: Entity) -> Option<BoundingBox> {
        body_record(app, panel).map(|record| record.bounds)
    }

    fn layout_unit_points(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<DiegeticPanel>(panel)
            .map_or(f32::NAN, |panel| panel.layout_unit().to_points())
    }

    fn location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  1,
                height: 1,
            },
            position: Vec2::ZERO,
        }
    }

    /// Seeds the hover map with `hovered`, each hit at `position`, and runs one
    /// wheel through picking's pointer event dispatch.
    fn wheel(app: &mut App, hovered: &[Entity], unit: MouseScrollUnit, y: f32, position: Vec3) {
        let hits = hovered
            .iter()
            .map(|&entity| {
                (
                    entity,
                    HitData::new(Entity::PLACEHOLDER, 0.0, Some(position), None),
                )
            })
            .collect();
        app.world_mut()
            .resource_mut::<HoverMap>()
            .insert(POINTER, hits);
        app.world_mut().write_message(PointerInput::new(
            POINTER,
            location(),
            PointerAction::Scroll {
                unit,
                x: 0.0,
                y,
                phase: TouchPhase::Moved,
            },
        ));
        let result = app.world_mut().run_system_cached(pointer_events);
        assert!(result.is_ok());
        app.world_mut().flush();
    }

    fn take_scrolls(app: &mut App) -> Vec<PanelScrolled> {
        std::mem::take(&mut app.world_mut().resource_mut::<RecordedScrolls>().0)
    }

    #[test]
    fn wheel_over_a_named_body_triggers_panel_scrolled_with_converted_delta() {
        let mut app = scroll_test_app();
        let panel = spawn_solved_panel(&mut app);
        let body = body_bounds(&app, panel);
        assert!(body.is_some());
        let Some(body) = body else {
            return;
        };
        let body_center = Vec2::new(
            body.width.mul_add(0.5, body.x),
            body.height.mul_add(0.5, body.y),
        );
        let position = world_position(&app, panel, body_center);
        assert!(position.is_some());
        let Some(position) = position else {
            return;
        };
        let unit_points = layout_unit_points(&app, panel);
        let line_pitch = body_record(&app, panel).map_or(f32::NAN, |record| record.line_pitch);
        assert!(line_pitch > 0.0);

        wheel(&mut app, &[panel], MouseScrollUnit::Line, 1.0, position);
        let scrolls = take_scrolls(&mut app);
        assert_eq!(scrolls.len(), 1);
        let Some(line) = scrolls.first() else {
            return;
        };
        assert_eq!(line.panel, panel);
        assert_eq!(line.element, PanelElementId::named(BODY_ID));
        assert_eq!(line.pointer_id, POINTER);
        assert_eq!(
            line.delta,
            Vec2::new(0.0, -WHEEL_LINES_PER_NOTCH * line_pitch)
        );
        assert!(line.extent.max_offset().y > 0.0);

        wheel(&mut app, &[panel], MouseScrollUnit::Pixel, 12.0, position);
        let scrolls = take_scrolls(&mut app);
        assert_eq!(scrolls.len(), 1);
        let Some(pixel) = scrolls.first() else {
            return;
        };
        assert_eq!(pixel.delta, Vec2::new(0.0, -12.0 / unit_points));
    }

    #[test]
    fn wheel_over_the_header_triggers_nothing() {
        let mut app = scroll_test_app();
        let panel = spawn_solved_panel(&mut app);
        let body = body_bounds(&app, panel);
        assert!(body.is_some());
        let Some(body) = body else {
            return;
        };
        let header = world_position(
            &app,
            panel,
            Vec2::new(body.width.mul_add(0.5, body.x), body.y * 0.5),
        );
        assert!(header.is_some());
        let Some(header) = header else {
            return;
        };

        wheel(&mut app, &[panel], MouseScrollUnit::Line, 1.0, header);
        assert!(take_scrolls(&mut app).is_empty());
    }

    #[test]
    fn wheel_over_a_widget_row_in_a_named_body_scrolls_the_body_once() {
        let mut app = scroll_test_app();
        let panel = spawn_solved_panel(&mut app);
        let body = body_bounds(&app, panel);
        let widget = row_widget(&mut app, panel);
        assert!(body.is_some());
        assert!(widget.is_some());
        let (Some(body), Some(widget)) = (body, widget) else {
            return;
        };
        // A quarter of the way down the two-row body lies inside its first
        // row, the button.
        let over_row = world_position(
            &app,
            panel,
            Vec2::new(
                body.width.mul_add(0.5, body.x),
                body.height.mul_add(0.25, body.y),
            ),
        );
        assert!(over_row.is_some());
        let Some(over_row) = over_row else {
            return;
        };

        // Widgets block lower hits, so a backend reports only the widget.
        wheel(&mut app, &[widget], MouseScrollUnit::Line, 1.0, over_row);
        let scrolls = take_scrolls(&mut app);
        assert_eq!(scrolls.len(), 1);
        let Some(scrolled) = scrolls.first() else {
            return;
        };
        assert_eq!(scrolled.panel, panel);
        assert_eq!(scrolled.element, PanelElementId::named(BODY_ID));

        // With the panel hovered too, the panel's own delivery is the one read.
        wheel(
            &mut app,
            &[widget, panel],
            MouseScrollUnit::Line,
            1.0,
            over_row,
        );
        assert_eq!(take_scrolls(&mut app).len(), 1);
    }

    fn hovered_target(app: &mut App) -> Option<PointerScrollTarget> {
        app.world_mut()
            .run_system_once(|mut targets: HoveredScrollTargets| targets.under(POINTER))
            .ok()
    }

    fn hover(app: &mut App, hovered: Entity, position: Vec3) {
        let hit = HitData::new(Entity::PLACEHOLDER, 0.0, Some(position), None);
        app.world_mut()
            .resource_mut::<HoverMap>()
            .insert(POINTER, std::iter::once((hovered, hit)).collect());
    }

    #[test]
    fn a_hovered_widget_row_resolves_to_its_scrollable_body() {
        let mut app = scroll_test_app();
        let panel = spawn_solved_panel(&mut app);
        let body = body_bounds(&app, panel);
        let widget = row_widget(&mut app, panel);
        assert!(body.is_some());
        assert!(widget.is_some());
        let (Some(body), Some(widget)) = (body, widget) else {
            return;
        };
        let over_row = world_position(
            &app,
            panel,
            Vec2::new(
                body.width.mul_add(0.5, body.x),
                body.height.mul_add(0.25, body.y),
            ),
        );
        let over_header = world_position(
            &app,
            panel,
            Vec2::new(body.width.mul_add(0.5, body.x), body.y * 0.5),
        );
        assert!(over_row.is_some());
        assert!(over_header.is_some());
        let (Some(over_row), Some(over_header)) = (over_row, over_header) else {
            return;
        };

        assert_eq!(
            hovered_target(&mut app),
            Some(PointerScrollTarget::NothingScrollable)
        );

        // Widgets block lower hits, so the hover map holds only the widget.
        hover(&mut app, widget, over_row);
        assert_eq!(
            hovered_target(&mut app),
            Some(PointerScrollTarget::Scrollable {
                panel,
                element: PanelElementId::named(BODY_ID),
            })
        );

        hover(&mut app, panel, over_header);
        assert_eq!(
            hovered_target(&mut app),
            Some(PointerScrollTarget::NothingScrollable)
        );
    }

    /// A panel of the same size as the scrolling one with no scrolling element.
    fn spawn_plain_panel(app: &mut App) -> Entity {
        let mut builder =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        builder.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        app.world_mut().flush();
        panel
    }

    fn hover_at_depths(app: &mut App, hovered: &[(Entity, f32)], position: Vec3) {
        let hits = hovered
            .iter()
            .map(|&(entity, depth)| {
                (
                    entity,
                    HitData::new(Entity::PLACEHOLDER, depth, Some(position), None),
                )
            })
            .collect();
        app.world_mut()
            .resource_mut::<HoverMap>()
            .insert(POINTER, hits);
    }

    #[test]
    fn a_panel_in_front_of_a_scrollable_body_hides_it_from_the_wheel() {
        let mut app = scroll_test_app();
        let scrolling = spawn_solved_panel(&mut app);
        let plain = spawn_plain_panel(&mut app);
        let body = body_bounds(&app, scrolling);
        assert!(body.is_some());
        let Some(body) = body else {
            return;
        };
        let body_center = world_position(
            &app,
            scrolling,
            Vec2::new(
                body.width.mul_add(0.5, body.x),
                body.height.mul_add(0.5, body.y),
            ),
        );
        assert!(body_center.is_some());
        let Some(body_center) = body_center else {
            return;
        };

        hover_at_depths(&mut app, &[(plain, 1.0), (scrolling, 2.0)], body_center);
        assert_eq!(
            hovered_target(&mut app),
            Some(PointerScrollTarget::NothingScrollable)
        );

        hover_at_depths(&mut app, &[(plain, 3.0), (scrolling, 2.0)], body_center);
        assert_eq!(
            hovered_target(&mut app),
            Some(PointerScrollTarget::Scrollable {
                panel:   scrolling,
                element: PanelElementId::named(BODY_ID),
            })
        );
    }
}
