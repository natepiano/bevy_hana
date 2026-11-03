//! Public scroll-view behaviour, driven through picking at fixed 120 Hz.

use std::time::Duration;

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
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use hana_diegetic::BoundingBox;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ComputedScrollRecord;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::HoveredScrollTargets;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelScrolled;
use hana_diegetic::PointerScrollTarget;
use hana_diegetic::Pt;
use hana_diegetic::ScrollIndicatorState;
use hana_diegetic::ScrollRecordDrive;
use hana_diegetic::ScrollRestEdge;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_kana::ToF32;

const BODY: &str = "body";
const WIDTH: f32 = 100.0;
const HEIGHT: f32 = 100.0;
const ROW_HEIGHT: f32 = 20.0;
const DT: f32 = 1.0 / 120.0;
const POINTER: PointerId = PointerId::Mouse;

#[derive(Default, Resource)]
struct RecordedScrolls(Vec<PanelScrolled>);

fn record_scroll(event: On<PanelScrolled>, mut recorded: ResMut<RecordedScrolls>) {
    recorded.0.push(event.event().clone());
}

const fn location() -> Location {
    Location {
        target:   NormalizedRenderTarget::None {
            width:  1,
            height: 1,
        },
        position: Vec2::ZERO,
    }
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            DT,
        )))
        .insert_resource(DiegeticTextMeasurer::default())
        .init_resource::<Assets<Mesh>>()
        .init_resource::<RecordedScrolls>()
        .add_plugins((HeadlessDiegeticUiPlugin, InteractionPlugin))
        .insert_resource(PickingSettings {
            is_hover_enabled: false,
            ..default()
        })
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

fn tree(rows: usize, edge: ScrollRestEdge, author: bool) -> LayoutTree {
    named_tree(rows, edge, author, BODY)
}

fn named_tree(rows: usize, edge: ScrollRestEdge, author: bool, id: &str) -> LayoutTree {
    let mut builder =
        LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
    let body = El::column()
        .width(Sizing::GROW)
        .height(Sizing::GROW)
        .scroll_line(TextStyle::new(10.0));
    let body = if author {
        body.id(id).scroll_y(0.0)
    } else {
        match edge {
            ScrollRestEdge::Start => body.scroll_view(id),
            ScrollRestEdge::End => body.scroll_view_from_end(id),
        }
    };
    builder.with(body, |builder| {
        for _ in 0..rows {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(ROW_HEIGHT)),
                |_| {},
            );
        }
    });
    builder.build()
}

fn scene(rows: usize, edge: ScrollRestEdge) -> Result<(App, Entity), String> {
    let mut app = test_app();
    let panel = spawn(&mut app, tree(rows, edge, false))?;
    Ok((app, panel))
}

fn spawn(app: &mut App, tree: LayoutTree) -> Result<Entity, String> {
    let panel = DiegeticPanel::world()
        .size(Pt(WIDTH), Pt(HEIGHT))
        .with_tree(tree)
        .build()
        .map_err(|error| error.to_string())?;
    let entity = app.world_mut().spawn(panel).id();
    frames(app, 2);
    Ok(entity)
}

fn record(app: &App, panel: Entity) -> Result<ComputedScrollRecord, String> {
    app.world()
        .get::<ComputedDiegeticPanel>(panel)
        .and_then(|computed| {
            computed
                .scroll_records()
                .iter()
                .find(|record| record.id == PanelElementId::named(BODY))
        })
        .cloned()
        .ok_or_else(|| "missing body scroll record".to_string())
}

fn frames(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}

fn replace_rows(
    app: &mut App,
    panel: Entity,
    rows: usize,
    edge: ScrollRestEdge,
) -> Result<(), String> {
    let tree = tree(rows, edge, false);
    app.world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, tree.clone()))
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    frames(app, 2);
    Ok(())
}

fn hover(app: &mut App, panel: Entity) -> Result<(), String> {
    let bounds = record(app, panel)?.bounds;
    let point = Vec2::new(
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    );
    let data = app
        .world()
        .get::<DiegeticPanel>(panel)
        .ok_or("missing panel")?;
    let transform = app
        .world()
        .get::<GlobalTransform>(panel)
        .ok_or("missing transform")?;
    let (anchor_x, anchor_y) = data.anchor_offsets();
    let scale = data.points_to_world();
    let position = transform.transform_point(Vec3::new(
        point.x.mul_add(scale, -anchor_x),
        point.y.mul_add(-scale, anchor_y),
        0.0,
    ));
    app.world_mut().resource_mut::<HoverMap>().insert(
        POINTER,
        std::iter::once((
            panel,
            HitData::new(Entity::PLACEHOLDER, 0.0, Some(position), None),
        ))
        .collect(),
    );
    Ok(())
}

fn wheel(
    app: &mut App,
    panel: Entity,
    unit: MouseScrollUnit,
    distance: f32,
    phase: TouchPhase,
) -> Result<(), String> {
    hover(app, panel)?;
    app.world_mut().write_message(PointerInput::new(
        POINTER,
        location(),
        PointerAction::Scroll {
            unit,
            x: 0.0,
            y: -distance,
            phase,
        },
    ));
    app.world_mut()
        .run_system_cached(pointer_events)
        .map_err(|error| error.to_string())?;
    app.world_mut().flush();
    Ok(())
}

fn pixels(app: &mut App, panel: Entity, distance: f32) -> Result<(), String> {
    wheel(
        app,
        panel,
        MouseScrollUnit::Pixel,
        distance,
        TouchPhase::Moved,
    )?;
    frames(app, 1);
    Ok(())
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.01,
        "actual={actual}, expected={expected}"
    );
}

fn shown(record: &ComputedScrollRecord) -> Result<(BoundingBox, f32), String> {
    match &record.indicator {
        ScrollIndicatorState::Shown { thumb, opacity } => Ok((*thumb, *opacity)),
        ScrollIndicatorState::Hidden => Err("expected visible indicator".to_string()),
    }
}

#[test]
fn a_scroll_view_opens_at_its_start_edge() -> Result<(), String> {
    let (app, panel) = scene(20, ScrollRestEdge::Start)?;
    let record = record(&app, panel)?;
    close(record.offset.y, 0.0);
    assert_eq!(record.drive, ScrollRecordDrive::Library);
    assert!(matches!(record.indicator, ScrollIndicatorState::Hidden));
    Ok(())
}

#[test]
fn a_scroll_view_from_end_opens_at_the_last_row_and_follows_new_rows() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::End)?;
    let initial = record(&app, panel)?;
    close(initial.offset.y, initial.extent.max_offset().y);
    replace_rows(&mut app, panel, 25, ScrollRestEdge::End)?;
    let grown = record(&app, panel)?;
    close(grown.offset.y, grown.extent.max_offset().y);
    assert!(grown.offset.y > initial.offset.y);
    // Following is also regained by scrolling a previously pinned log to its end.
    pixels(&mut app, panel, -40.0)?;
    pixels(&mut app, panel, 40.0)?;
    replace_rows(&mut app, panel, 30, ScrollRestEdge::End)?;
    let following = record(&app, panel)?;
    close(following.offset.y, following.extent.max_offset().y);
    Ok(())
}

#[test]
fn a_pinned_view_keeps_its_offset_when_rows_arrive_below() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::End)?;
    pixels(&mut app, panel, -40.0)?;
    let pinned = record(&app, panel)?.offset.y;
    replace_rows(&mut app, panel, 25, ScrollRestEdge::End)?;
    close(record(&app, panel)?.offset.y, pinned);
    Ok(())
}

#[test]
fn a_wheel_notch_travels_three_line_pitches_by_the_time_it_rests() -> Result<(), String> {
    let (mut app, panel) = scene(40, ScrollRestEdge::Start)?;
    let distance = 3.0 * record(&app, panel)?.line_pitch;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    frames(&mut app, 240);
    close(record(&app, panel)?.offset.y, distance);
    let resting = record(&app, panel)?.offset.y;
    frames(&mut app, 120);
    close(record(&app, panel)?.offset.y, resting);
    Ok(())
}

#[test]
fn pixel_input_moves_the_content_one_to_one() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 17.25)?;
    close(record(&app, panel)?.offset.y, 17.25);
    frames(&mut app, 120);
    close(record(&app, panel)?.offset.y, 17.25);
    pixels(&mut app, panel, -6.5)?;
    close(record(&app, panel)?.offset.y, 10.75);
    Ok(())
}

#[test]
fn the_offset_clamps_when_content_shrinks() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 180.0)?;
    replace_rows(&mut app, panel, 10, ScrollRestEdge::Start)?;
    let clamped = record(&app, panel)?;
    close(clamped.offset.y, clamped.extent.max_offset().y);
    close(clamped.overscroll, 0.0);
    replace_rows(&mut app, panel, 20, ScrollRestEdge::Start)?;
    close(record(&app, panel)?.offset.y, clamped.offset.y);
    Ok(())
}

#[test]
fn the_offset_clamps_when_the_panel_grows() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 250.0)?;
    app.world_mut()
        .get_mut::<DiegeticPanel>(panel)
        .ok_or("missing panel")?
        .set_size((Pt(WIDTH), Pt(250.0)))
        .map_err(|error| error.to_string())?;
    frames(&mut app, 2);
    let clamped = record(&app, panel)?;
    close(clamped.offset.y, clamped.extent.max_offset().y);
    close(clamped.overscroll, 0.0);
    app.world_mut()
        .get_mut::<DiegeticPanel>(panel)
        .ok_or("missing panel")?
        .set_size((Pt(WIDTH), Pt(HEIGHT)))
        .map_err(|error| error.to_string())?;
    frames(&mut app, 2);
    close(record(&app, panel)?.offset.y, clamped.offset.y);
    Ok(())
}

#[test]
fn a_view_whose_content_now_fits_returns_to_its_rest_edge() -> Result<(), String> {
    for edge in [ScrollRestEdge::Start, ScrollRestEdge::End] {
        let (mut app, panel) = scene(20, edge)?;
        let distance = match edge {
            ScrollRestEdge::Start => 60.0,
            ScrollRestEdge::End => -60.0,
        };
        pixels(&mut app, panel, distance)?;
        replace_rows(&mut app, panel, 2, edge)?;
        let fitting = record(&app, panel)?;
        close(fitting.offset.y, 0.0);
        close(fitting.overscroll, 0.0);
        assert!(matches!(fitting.indicator, ScrollIndicatorState::Hidden));
        replace_rows(&mut app, panel, 20, edge)?;
        let overflow = record(&app, panel)?;
        let expected = match edge {
            ScrollRestEdge::Start => 0.0,
            ScrollRestEdge::End => overflow.extent.max_offset().y,
        };
        close(overflow.offset.y, expected);
    }
    Ok(())
}

#[test]
fn content_that_fits_has_no_indicator_and_ignores_the_wheel() -> Result<(), String> {
    let (mut app, panel) = scene(2, ScrollRestEdge::Start)?;
    let before = record(&app, panel)?;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    pixels(&mut app, panel, 30.0)?;
    frames(&mut app, 240);
    assert_eq!(record(&app, panel)?, before);
    assert!(app.world().resource::<RecordedScrolls>().0.is_empty());
    assert!(matches!(before.indicator, ScrollIndicatorState::Hidden));
    Ok(())
}

#[test]
fn coasting_decays_to_rest_on_fixed_time_steps() -> Result<(), String> {
    let (mut app, panel) = scene(40, ScrollRestEdge::Start)?;
    let distance = 3.0 * record(&app, panel)?.line_pitch;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    let mut prior = 0;
    for step in [1, 10, 60] {
        frames(&mut app, step - prior);
        let expected = distance * (1.0 - (-12.0 * DT * step.to_f32()).exp());
        close(record(&app, panel)?.offset.y, expected);
        prior = step;
    }
    frames(&mut app, 180);
    close(record(&app, panel)?.offset.y, distance);
    Ok(())
}

#[test]
fn a_wheel_coast_stops_at_the_edge_without_overshoot() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    let max = record(&app, panel)?.extent.max_offset().y;
    pixels(&mut app, panel, max - 5.0)?;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    for _ in 0..240 {
        frames(&mut app, 1);
        let current = record(&app, panel)?;
        assert!(current.offset.y <= max);
        close(current.overscroll, 0.0);
    }
    close(record(&app, panel)?.offset.y, max);
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        -100.0,
        TouchPhase::Moved,
    )?;
    for _ in 0..240 {
        frames(&mut app, 1);
        let current = record(&app, panel)?;
        assert!(current.offset.y >= 0.0);
        close(current.overscroll, 0.0);
    }
    close(record(&app, panel)?.offset.y, 0.0);
    Ok(())
}

#[test]
fn pixel_overscroll_follows_the_rubber_band_and_releases_on_ended() -> Result<(), String> {
    for (edge, direction) in [(ScrollRestEdge::Start, -1.0), (ScrollRestEdge::End, 1.0)] {
        let (mut app, panel) = scene(20, edge)?;
        let initial = record(&app, panel)?;
        pixels(&mut app, panel, direction * 40.0)?;
        let stretched = record(&app, panel)?;
        let d = initial.extent.viewport.y;
        let rubber = d * (1.0 - 1.0 / (40.0 * 0.55 / d + 1.0));
        close(stretched.overscroll, direction * rubber);
        let (thumb, _) = shown(&stretched)?;
        let base_length = ((d - 4.0) * d / initial.extent.content.y).max(18.0);
        close(thumb.height, (base_length - rubber).max(8.0));
        wheel(
            &mut app,
            panel,
            MouseScrollUnit::Pixel,
            0.0,
            TouchPhase::Ended,
        )?;
        frames(&mut app, 1);
        assert!(record(&app, panel)?.overscroll.abs() < rubber);
        frames(&mut app, 240);
        close(record(&app, panel)?.overscroll, 0.0);
        close(record(&app, panel)?.offset.y, initial.offset.y);
    }
    Ok(())
}

#[test]
fn pixel_overscroll_releases_after_the_idle_time_without_an_ended_phase() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, -40.0)?;
    let stretch = record(&app, panel)?.overscroll;
    frames(&mut app, 9);
    close(record(&app, panel)?.overscroll, stretch);
    frames(&mut app, 10);
    assert!(record(&app, panel)?.overscroll.abs() < stretch.abs());
    frames(&mut app, 240);
    close(record(&app, panel)?.overscroll, 0.0);
    Ok(())
}

#[test]
fn the_indicator_length_is_the_visible_fraction_of_the_track() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 30.0)?;
    let current = record(&app, panel)?;
    let (thumb, opacity) = shown(&current)?;
    close(
        thumb.height,
        ((current.bounds.height - 4.0) * current.extent.viewport.y / current.extent.content.y)
            .max(18.0),
    );
    close(thumb.width, 3.0);
    close(thumb.x, current.bounds.x + current.bounds.width - 5.0);
    close(opacity, 1.0);
    replace_rows(&mut app, panel, 100, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 1.0)?;
    close(shown(&record(&app, panel)?)?.0.height, 18.0);
    Ok(())
}

#[test]
fn the_indicator_top_tracks_the_offset() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    for distance in [30.0, 120.0, 150.0] {
        pixels(&mut app, panel, distance)?;
        let record = record(&app, panel)?;
        let (thumb, _) = shown(&record)?;
        close(
            thumb.y,
            (record.offset.y / record.extent.max_offset().y).mul_add(
                record.bounds.height - 4.0 - thumb.height,
                record.bounds.y + 2.0,
            ),
        );
    }
    Ok(())
}

#[test]
fn the_indicator_holds_then_fades_to_hidden() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 30.0)?;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Pixel,
        0.0,
        TouchPhase::Ended,
    )?;
    frames(&mut app, 1);
    frames(&mut app, 84);
    close(shown(&record(&app, panel)?)?.1, 1.0);
    frames(&mut app, 27);
    let middle = shown(&record(&app, panel)?)?.1;
    assert!((middle - 0.5).abs() <= 0.07, "half-fade opacity={middle}");
    frames(&mut app, 30);
    assert!(matches!(
        record(&app, panel)?.indicator,
        ScrollIndicatorState::Hidden
    ));
    close(record(&app, panel)?.offset.y, 30.0);
    Ok(())
}

#[test]
fn a_scroll_view_with_overflow_is_a_scrollable_wheel_target() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    hover(&mut app, panel)?;
    let target = app
        .world_mut()
        .run_system_once(|mut targets: HoveredScrollTargets| targets.under(POINTER))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        target,
        PointerScrollTarget::Scrollable {
            panel,
            element: PanelElementId::named(BODY)
        }
    );
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    app.world_mut().resource_mut::<HoverMap>().clear();
    let target = app
        .world_mut()
        .run_system_once(|mut targets: HoveredScrollTargets| targets.under(POINTER))
        .map_err(|error| error.to_string())?;
    assert_eq!(target, PointerScrollTarget::NothingScrollable);
    Ok(())
}

#[test]
fn a_scroll_view_whose_content_fits_is_not_a_wheel_target() -> Result<(), String> {
    let (mut app, panel) = scene(2, ScrollRestEdge::Start)?;
    hover(&mut app, panel)?;
    let target = app
        .world_mut()
        .run_system_once(|mut targets: HoveredScrollTargets| targets.under(POINTER))
        .map_err(|error| error.to_string())?;
    assert_eq!(target, PointerScrollTarget::NothingScrollable);
    Ok(())
}

#[test]
fn an_author_driven_scroll_still_triggers_panel_scrolled() -> Result<(), String> {
    let mut app = test_app();
    let panel = spawn(&mut app, tree(20, ScrollRestEdge::Start, true))?;
    let before = record(&app, panel)?;
    assert_eq!(before.drive, ScrollRecordDrive::Author);
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    let events = &app.world().resource::<RecordedScrolls>().0;
    assert_eq!(events.len(), 1);
    let event = events.first().ok_or("missing author event")?;
    assert_eq!(event.panel, panel);
    assert_eq!(event.element, PanelElementId::named(BODY));
    close(event.delta.y, 3.0 * before.line_pitch);
    frames(&mut app, 120);
    close(record(&app, panel)?.offset.y, 0.0);
    Ok(())
}

#[test]
fn a_library_scroll_view_triggers_no_panel_scrolled() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    wheel(
        &mut app,
        panel,
        MouseScrollUnit::Line,
        1.0,
        TouchPhase::Moved,
    )?;
    pixels(&mut app, panel, 10.0)?;
    frames(&mut app, 120);
    assert!(app.world().resource::<RecordedScrolls>().0.is_empty());
    assert!(record(&app, panel)?.offset.y > 0.0);
    Ok(())
}

fn assert_clamped_end_follows(unit: MouseScrollUnit) -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 180.0)?;
    replace_rows(&mut app, panel, 10, ScrollRestEdge::Start)?;
    let clamped = record(&app, panel)?;
    close(clamped.offset.y, clamped.extent.max_offset().y);
    wheel(&mut app, panel, unit, 1.0, TouchPhase::Moved)?;
    frames(&mut app, 240);
    replace_rows(&mut app, panel, 20, ScrollRestEdge::Start)?;
    let appended = record(&app, panel)?;
    close(appended.offset.y, appended.extent.max_offset().y);
    Ok(())
}

#[test]
fn scroll_repair_downward_notch_at_a_clamped_end_restores_following() -> Result<(), String> {
    assert_clamped_end_follows(MouseScrollUnit::Line)
}

#[test]
fn scroll_repair_downward_pixel_at_a_clamped_end_restores_following() -> Result<(), String> {
    assert_clamped_end_follows(MouseScrollUnit::Pixel)
}

fn assert_notch_travel(app: &mut App, panel: Entity) -> Result<(), String> {
    frames(app, 2);
    let initial = record(app, panel)?.offset.y;
    let distance = 3.0 * record(app, panel)?.line_pitch;
    wheel(app, panel, MouseScrollUnit::Line, 1.0, TouchPhase::Moved)?;
    frames(app, 240);
    close(record(app, panel)?.offset.y, initial + distance);
    Ok(())
}

#[test]
fn scroll_repair_a_notch_travels_its_whole_distance_in_millimeters() -> Result<(), String> {
    let (mut app, panel) = scene(40, ScrollRestEdge::Start)?;
    app.world_mut()
        .get_mut::<DiegeticPanel>(panel)
        .ok_or("missing panel")?
        .set_size((hana_diegetic::Mm(WIDTH), hana_diegetic::Mm(HEIGHT)))
        .map_err(|error| error.to_string())?;
    assert_notch_travel(&mut app, panel)
}

#[test]
fn scroll_repair_a_notch_travels_its_whole_distance_in_pixels() -> Result<(), String> {
    let (mut app, panel) = scene(40, ScrollRestEdge::Start)?;
    app.world_mut()
        .get_mut::<DiegeticPanel>(panel)
        .ok_or("missing panel")?
        .set_size((hana_diegetic::Px(WIDTH), hana_diegetic::Px(HEIGHT)))
        .map_err(|error| error.to_string())?;
    frames(&mut app, 2);
    pixels(&mut app, panel, 17.25)?;
    assert_notch_travel(&mut app, panel)
}

#[test]
fn scroll_repair_a_renamed_view_starts_at_its_own_rest_edge() -> Result<(), String> {
    let (mut app, panel) = scene(20, ScrollRestEdge::Start)?;
    pixels(&mut app, panel, 40.0)?;
    assert!(record(&app, panel)?.offset.y > 0.0);
    let tree = named_tree(20, ScrollRestEdge::Start, false, "new");
    app.world_mut()
        .run_system_once(move |mut commands: Commands| commands.set_tree(panel, tree.clone()))
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    frames(&mut app, 2);
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or("missing panel")?;
    let renamed = computed
        .scroll_records()
        .iter()
        .find(|record| record.id == PanelElementId::named("new"))
        .ok_or("missing renamed view")?;
    close(renamed.offset.y, 0.0);
    assert_eq!(renamed.indicator, ScrollIndicatorState::Hidden);
    Ok(())
}
