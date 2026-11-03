//! Lazy panel-owned view position and activity-driven indicator fade.

use std::time::Duration;

use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::With;

use super::constants::INDICATOR_FADE_SECONDS;
use super::constants::INDICATOR_HOLD_SECONDS;
use super::constants::INDICATOR_INSET;
use super::constants::INDICATOR_MIN_LENGTH;
use super::constants::INDICATOR_MIN_OVERSCROLL_LENGTH;
use super::constants::INDICATOR_WIDTH;
use super::constants::SCROLL_REST_DISTANCE;
use super::scroll::ComputedScrollRecord;
use super::scroll::ScrollIndicatorState;
use super::scroll_motion::ScrollBounds;
use super::scroll_motion::ScrollInput;
use super::scroll_motion::ScrollMotion;
use crate::BoundingBox;
use crate::ComputedDiegeticPanel;
use crate::PanelElementId;
use crate::layout::ScrollDrive;
use crate::layout::ScrollRestEdge;
use crate::layout::ScrollSolveInput;
use crate::layout::ScrollViewSolve;

/// A pin is independent of content arriving below it; a following view is not.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ScrollViewPosition {
    PinnedFromStart(f32),
    FollowingEnd,
}

/// One view, allocated only when wheel input first reaches overflowing content.
#[derive(Clone, Debug, PartialEq)]
struct ScrollViewState {
    id:               PanelElementId,
    position:         ScrollViewPosition,
    content_origin_y: f32,
    motion:           ScrollMotion,
    indicator:        IndicatorFade,
}

#[derive(Component, Default)]
pub(crate) struct PanelScrollViews(Vec<ScrollViewState>);

/// Enables frame work only while motion or an indicator remains visible.
#[derive(Component)]
pub(crate) struct ScrollViewsInMotion;

impl ScrollViewState {
    const fn offset(&self, max: f32) -> f32 {
        match self.position {
            ScrollViewPosition::PinnedFromStart(offset) => offset.clamp(0.0, max),
            ScrollViewPosition::FollowingEnd => max,
        }
    }

    fn bounds(&self, record: &ComputedScrollRecord) -> ScrollBounds {
        let max = record.extent.max_offset().y;
        ScrollBounds {
            offset: self.offset(max),
            max,
            viewport: record.extent.viewport.y,
        }
    }

    fn move_by(&mut self, delta: f32, at: ScrollBounds) {
        let offset = (at.offset + delta).clamp(0.0, at.max);
        if delta != 0.0 {
            self.position = if (at.max - offset).abs() <= SCROLL_REST_DISTANCE {
                ScrollViewPosition::FollowingEnd
            } else {
                ScrollViewPosition::PinnedFromStart(offset)
            };
        }
    }
}

impl PanelScrollViews {
    pub(crate) fn input(&mut self, record: &ComputedScrollRecord, input: ScrollInput) {
        let index = self
            .0
            .iter()
            .position(|view| view.id == record.id)
            .unwrap_or_else(|| {
                self.0.push(ScrollViewState {
                    id:               record.id.clone(),
                    position:         if (record.extent.max_offset().y - record.offset.y).abs()
                        <= SCROLL_REST_DISTANCE
                    {
                        ScrollViewPosition::FollowingEnd
                    } else {
                        ScrollViewPosition::PinnedFromStart(record.offset.y)
                    },
                    content_origin_y: record.content_origin_y,
                    motion:           ScrollMotion::default(),
                    indicator:        IndicatorFade::default(),
                });
                self.0.len() - 1
            });
        let view = &mut self.0[index];
        let at = view.bounds(record);
        let (ScrollInput::Notches { distance } | ScrollInput::Pixels { distance, .. }) = input;
        // A bounds clamp can put a pin at the end before this input moves it.
        if distance > 0.0 && at.offset >= at.max {
            view.position = ScrollViewPosition::FollowingEnd;
        }
        let delta = view.motion.input(input, at);
        view.move_by(delta, at);
        view.indicator.show();
    }

    pub(super) fn solve_input(
        &self,
        scale: f32,
        tree: &crate::LayoutTree,
        input: &mut ScrollSolveInput,
    ) {
        for view in &self.0 {
            input.views.insert(
                view.id.clone(),
                ScrollViewSolve {
                    offset_points:     match view.position {
                        // The next solve may have more content than the previous one.
                        ScrollViewPosition::FollowingEnd => f32::MAX,
                        ScrollViewPosition::PinnedFromStart(offset) => {
                            let removed = (tree.scroll_content_origin_y(&view.id) / scale
                                - view.content_origin_y)
                                .max(0.0);
                            (offset - removed).max(0.0) * scale
                        },
                    },
                    overscroll_points: view.motion.overscroll * scale,
                    indicator_opacity: view.indicator.opacity(),
                },
            );
        }
    }

    /// Retain the applied clamp, without turning a clamped pin into a tail follower.
    pub(super) fn reconcile(&mut self, computed: &ComputedDiegeticPanel, tree: &crate::LayoutTree) {
        self.0.retain_mut(|view| {
            let Some(record) = computed
                .scroll_records()
                .iter()
                .find(|record| record.id == view.id)
            else {
                return false;
            };
            let ScrollDrive::Library(edge) = tree.element_scroll_drive(record.element_index) else {
                return false;
            };
            if let ScrollViewPosition::PinnedFromStart(offset) = &mut view.position {
                *offset =
                    (*offset - (record.content_origin_y - view.content_origin_y).max(0.0)).max(0.0);
            }
            view.content_origin_y = record.content_origin_y;
            if record.extent.max_offset().y <= 0.0 {
                view.position = match edge {
                    ScrollRestEdge::Start => ScrollViewPosition::PinnedFromStart(0.0),
                    ScrollRestEdge::End => ScrollViewPosition::FollowingEnd,
                };
                view.motion = ScrollMotion::default();
                view.indicator.hide();
            } else if let ScrollViewPosition::PinnedFromStart(offset) = &mut view.position
                && *offset > record.extent.max_offset().y
            {
                // Only a bounds clamp cancels motion; unit round trips can
                // lower the applied offset by one ULP during an ordinary coast.
                *offset = record.offset.y;
                view.motion = ScrollMotion::default();
            }
            true
        });
    }
}

/// Runs only while a marker exists; idle panels are neither touched nor solved.
pub(super) fn advance_scroll_views(
    time: Res<Time>,
    mut panels: Query<
        (Entity, &mut PanelScrollViews, &ComputedDiegeticPanel),
        With<ScrollViewsInMotion>,
    >,
    mut commands: Commands,
) {
    for (entity, mut views, computed) in &mut panels {
        let mut changed = false;
        let mut active = false;
        for view in &mut views.bypass_change_detection().0 {
            let previous = (
                view.position,
                view.motion.overscroll,
                view.indicator.opacity(),
            );
            if let Some(record) = computed
                .scroll_records()
                .iter()
                .find(|record| record.id == view.id)
            {
                let at = view.bounds(record);
                let delta = view.motion.step(time.delta_secs(), at);
                view.move_by(delta, at);
                view.indicator.step(time.delta(), !view.motion.is_at_rest());
            } else {
                view.motion = ScrollMotion::default();
                view.indicator.hide();
            }
            changed |= (
                view.position,
                view.motion.overscroll,
                view.indicator.opacity(),
            ) != previous;
            active |= !view.motion.is_at_rest() || view.indicator.opacity() > 0.0;
        }
        if changed {
            views.set_changed();
        }
        if !active {
            commands.entity(entity).remove::<ScrollViewsInMotion>();
        }
    }
}

/// The track uses the view's child clip box, inside its border, including padding.
/// Overflow and the length ratio use the separate padding-excluded viewport.
/// All dimensions here are solved layout points, independent of panel units.
pub(crate) fn solve_indicator(
    clip_box: BoundingBox,
    viewport_height: f32,
    content_height: f32,
    offset: f32,
    overscroll: f32,
    opacity: f32,
) -> ScrollIndicatorState {
    let max = (content_height - viewport_height).max(0.0);
    let track = 2.0_f32.mul_add(-INDICATOR_INSET, clip_box.height).max(0.0);
    if opacity <= 0.0
        || max <= 0.0
        || track <= 0.0
        || clip_box.width < 2.0_f32.mul_add(INDICATOR_INSET, INDICATOR_WIDTH)
    {
        return ScrollIndicatorState::Hidden;
    }
    let natural_length = (track * viewport_height / content_height)
        .max(INDICATOR_MIN_LENGTH)
        .min(track);
    let length = if overscroll == 0.0 {
        natural_length
    } else {
        (natural_length - overscroll.abs())
            .max(INDICATOR_MIN_OVERSCROLL_LENGTH)
            .min(track)
    };
    let track_top = clip_box.y + INDICATOR_INSET;
    let top = if overscroll < 0.0 {
        track_top
    } else if overscroll > 0.0 {
        track_top + track - length
    } else {
        (offset / max)
            .clamp(0.0, 1.0)
            .mul_add(track - length, track_top)
    };
    ScrollIndicatorState::Shown {
        thumb:   BoundingBox {
            x:      clip_box.x + clip_box.width - INDICATOR_INSET - INDICATOR_WIDTH,
            y:      top,
            width:  INDICATOR_WIDTH,
            height: length,
        },
        opacity: opacity.clamp(0.0, 1.0),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum IndicatorPhase {
    Hidden,
    Moving,
    Holding(Timer),
    Fading(Timer),
}

#[derive(Clone, Debug, PartialEq)]
struct IndicatorFade {
    phase: IndicatorPhase,
}

impl Default for IndicatorFade {
    fn default() -> Self {
        Self {
            phase: IndicatorPhase::Hidden,
        }
    }
}

impl IndicatorFade {
    const fn show(&mut self) { self.phase = IndicatorPhase::Moving; }

    const fn hide(&mut self) { self.phase = IndicatorPhase::Hidden; }

    fn opacity(&self) -> f32 {
        match &self.phase {
            IndicatorPhase::Hidden => 0.0,
            IndicatorPhase::Moving | IndicatorPhase::Holding(_) => 1.0,
            IndicatorPhase::Fading(timer) => 1.0 - timer.fraction(),
        }
    }

    fn step(&mut self, dt: Duration, moving: bool) {
        if moving {
            self.show();
            return;
        }
        if matches!(self.phase, IndicatorPhase::Moving) {
            self.phase = IndicatorPhase::Holding(Timer::from_seconds(
                INDICATOR_HOLD_SECONDS,
                TimerMode::Once,
            ));
        }
        match &mut self.phase {
            IndicatorPhase::Holding(timer) => {
                let remaining = timer.remaining();
                timer.tick(dt);
                if timer.is_finished() {
                    let mut fade = Timer::from_seconds(INDICATOR_FADE_SECONDS, TimerMode::Once);
                    fade.tick(dt.saturating_sub(remaining));
                    self.phase = if fade.is_finished() {
                        IndicatorPhase::Hidden
                    } else {
                        IndicatorPhase::Fading(fade)
                    };
                }
            },
            IndicatorPhase::Fading(timer) => {
                timer.tick(dt);
                if timer.is_finished() {
                    self.hide();
                }
            },
            IndicatorPhase::Hidden | IndicatorPhase::Moving => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::schedule::common_conditions::any_with_component;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::touch::TouchPhase;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Pt;
    use crate::Sizing;

    fn retained_rows_tree(first: usize, origin: f32, edge: ScrollRestEdge) -> crate::LayoutTree {
        let root = El::column().width(Sizing::GROW).height(Sizing::GROW);
        let root = match edge {
            ScrollRestEdge::Start => root.scroll_view("retained"),
            ScrollRestEdge::End => root.scroll_view_from_end("retained"),
        };
        let mut builder = LayoutBuilder::with_root(root.scroll_content_origin_y(origin));
        for row in first..20 {
            builder.with(
                El::new()
                    .id(format!("row-{row}"))
                    .height(Sizing::fixed(20.0)),
                |_| {},
            );
        }
        builder.build()
    }

    fn retained_rows_panel(edge: ScrollRestEdge) -> Result<(App, Entity), String> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        // Millimetres exercise origin conversion independently of layout points.
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(crate::Mm(100.0), crate::Mm(100.0))
                    .with_tree(retained_rows_tree(0, 0.0, edge))
                    .build()
                    .map_err(|error| error.to_string())?,
            )
            .id();
        app.update();
        Ok((app, panel))
    }

    fn retained_scroll(app: &App, panel: Entity) -> Result<&ComputedScrollRecord, String> {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| computed.scroll_records().first())
            .ok_or_else(|| "missing retained scroll view".to_owned())
    }

    fn retained_row_y(app: &App, panel: Entity, row: usize) -> Result<f32, String> {
        let authored = app
            .world()
            .get::<DiegeticPanel>(panel)
            .ok_or("missing panel")?;
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or("missing layout")?;
        let record = retained_scroll(app, panel)?;
        let id = PanelElementId::named(format!("row-{row}"));
        let index = authored
            .tree()
            .children_of(record.element_index)
            .iter()
            .find(|&&index| authored.tree().element_id(index) == Some(&id))
            .ok_or("missing retained row")?;
        Ok(computed
            .layout()
            .solved()
            .ok_or("missing solved layout")?
            .computed[*index]
            .bounds
            .y)
    }

    fn replace_retained_rows(
        app: &mut App,
        panel: Entity,
        first: usize,
        origin: f32,
        edge: ScrollRestEdge,
    ) -> Result<(), String> {
        app.world_mut()
            .commands()
            .set_tree(panel, retained_rows_tree(first, origin, edge))
            .map_err(|error| error.to_string())?;
        app.update();
        Ok(())
    }

    #[test]
    fn a_pinned_scroll_view_preserves_retained_rows_when_its_content_origin_grows()
    -> Result<(), String> {
        let (mut app, panel) = retained_rows_panel(ScrollRestEdge::Start)?;
        let mut views = PanelScrollViews::default();
        views.input(
            retained_scroll(&app, panel)?,
            ScrollInput::Pixels {
                distance: 100.0,
                phase:    TouchPhase::Moved,
            },
        );
        app.world_mut().entity_mut(panel).insert(views);
        app.update();
        assert!((retained_scroll(&app, panel)?.offset.y - 100.0).abs() < 0.001);
        let row_y = retained_row_y(&app, panel, 5)?;
        replace_retained_rows(&mut app, panel, 4, 80.0, ScrollRestEdge::Start)?;
        let record = retained_scroll(&app, panel)?;
        assert!((record.offset.y - 20.0).abs() < 0.001);
        // Both views start on row 5: five original rows, or one retained row.
        assert!((record.content_origin_y + record.offset.y - 100.0).abs() < 0.001);
        assert!((retained_row_y(&app, panel, 5)? - row_y).abs() < 0.001);
        replace_retained_rows(&mut app, panel, 8, 160.0, ScrollRestEdge::Start)?;
        assert_eq!(retained_scroll(&app, panel)?.offset.y, 0.0);
        let solves = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or("missing solved panel")?
            .layout_solves();
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("missing solved panel")?
                .layout_solves(),
            solves
        );
        Ok(())
    }

    #[test]
    fn a_pinned_scroll_view_keeps_its_offset_when_its_content_origin_drops() -> Result<(), String> {
        let (mut app, panel) = retained_rows_panel(ScrollRestEdge::Start)?;
        let mut views = PanelScrollViews::default();
        views.input(
            retained_scroll(&app, panel)?,
            ScrollInput::Pixels {
                distance: 100.0,
                phase:    TouchPhase::Moved,
            },
        );
        app.world_mut().entity_mut(panel).insert(views);
        app.update();
        replace_retained_rows(&mut app, panel, 4, 80.0, ScrollRestEdge::Start)?;
        let offset = retained_scroll(&app, panel)?.offset;
        assert!((offset.y - 20.0).abs() < 0.001);
        replace_retained_rows(&mut app, panel, 4, 0.0, ScrollRestEdge::Start)?;
        assert_eq!(retained_scroll(&app, panel)?.offset, offset);
        // Another solve verifies the retained pin, as well as the first projection.
        replace_retained_rows(&mut app, panel, 5, 0.0, ScrollRestEdge::Start)?;
        assert_eq!(retained_scroll(&app, panel)?.offset, offset);
        Ok(())
    }

    #[test]
    fn an_end_following_scroll_view_keeps_following_when_its_content_origin_grows()
    -> Result<(), String> {
        let (mut app, panel) = retained_rows_panel(ScrollRestEdge::End)?;
        let mut views = PanelScrollViews::default();
        for distance in [-20.0, 20.0] {
            views.input(
                retained_scroll(&app, panel)?,
                ScrollInput::Pixels {
                    distance,
                    phase: TouchPhase::Moved,
                },
            );
        }
        app.world_mut().entity_mut(panel).insert(views);
        app.update();
        replace_retained_rows(&mut app, panel, 2, 40.0, ScrollRestEdge::End)?;
        let record = retained_scroll(&app, panel)?;
        assert!((record.offset.y - record.extent.max_offset().y).abs() < 0.001);
        assert_eq!(
            app.world()
                .get::<PanelScrollViews>(panel)
                .ok_or("missing used scroll state")?
                .0[0]
                .position,
            ScrollViewPosition::FollowingEnd
        );
        replace_retained_rows(&mut app, panel, 4, 80.0, ScrollRestEdge::End)?;
        let record = retained_scroll(&app, panel)?;
        assert!((record.offset.y - record.extent.max_offset().y).abs() < 0.001);
        Ok(())
    }

    #[test]
    fn an_idle_scroll_view_runs_no_solve() -> Result<(), String> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let mut builder = LayoutBuilder::with_root(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .scroll_view("idle"),
        );
        for _ in 0..20 {
            builder.with(
                El::new().width(Sizing::GROW).height(Sizing::fixed(20.0)),
                |_| {},
            );
        }
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Pt(100.0), Pt(100.0))
                    .with_tree(builder.build())
                    .build()
                    .map_err(|error| error.to_string())?,
            )
            .id();
        app.update();
        let solves = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or("missing solved panel")?
            .layout_solves();
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("missing solved panel")?
                .layout_solves(),
            solves
        );
        assert!(app.world().get::<PanelScrollViews>(panel).is_none());
        assert!(app.world().get::<ScrollViewsInMotion>(panel).is_none());

        // A used view must also stop requesting solves once its fade ends.
        let mut views = PanelScrollViews::default();
        views.input(
            &app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("missing scroll record")?
                .scroll_records()[0],
            ScrollInput::Pixels {
                distance: 20.0,
                phase:    TouchPhase::Ended,
            },
        );
        app.world_mut()
            .entity_mut(panel)
            .insert((views, ScrollViewsInMotion));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 120.0,
        )));
        for _ in 0..240 {
            app.update();
        }
        assert!(app.world().get::<ScrollViewsInMotion>(panel).is_none());
        let solves = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or("missing solved panel")?
            .layout_solves();
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("missing solved panel")?
                .layout_solves(),
            solves
        );
        Ok(())
    }

    #[test]
    fn scroll_repair_removing_the_panel_role_discards_motion_and_position() -> Result<(), String> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let mut builder = LayoutBuilder::with_root(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .scroll_view("body"),
        );
        for _ in 0..20 {
            builder.with(El::new().height(Sizing::fixed(20.0)), |_| {});
        }
        let authored = DiegeticPanel::world()
            .size(Pt(100.0), Pt(100.0))
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn(authored.clone()).id();
        app.update();
        let mut views = PanelScrollViews::default();
        views.input(
            &app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("missing computed panel")?
                .scroll_records()[0],
            ScrollInput::Notches { distance: 45.0 },
        );
        app.world_mut()
            .entity_mut(panel)
            .insert((views, ScrollViewsInMotion));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 120.0,
        )));
        app.update();
        assert!(
            !app.world()
                .get::<PanelScrollViews>(panel)
                .ok_or("missing scroll state")?
                .0[0]
                .motion
                .is_at_rest()
        );
        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.world_mut().flush();
        assert!(app.world().get::<PanelScrollViews>(panel).is_none());
        assert!(app.world().get::<ScrollViewsInMotion>(panel).is_none());
        assert!(
            !app.world_mut()
                .run_system_once(any_with_component::<ScrollViewsInMotion>)
                .map_err(|error| error.to_string())?
        );
        app.update();
        app.world_mut().entity_mut(panel).insert(authored);
        app.update();
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .ok_or("missing reinserted panel")?;
        assert_eq!(computed.scroll_records()[0].offset.y, 0.0);
        assert_eq!(
            computed.scroll_records()[0].indicator,
            ScrollIndicatorState::Hidden
        );
        Ok(())
    }

    #[test]
    fn scroll_indicator_holds_then_fades_on_elapsed_time() {
        let mut fade = IndicatorFade::default();
        assert_eq!(fade.opacity(), 0.0);
        fade.show();
        fade.step(Duration::from_secs_f32(0.79), false);
        assert_eq!(fade.opacity(), 1.0);
        fade.step(Duration::from_secs_f32(0.135), false);
        assert!((fade.opacity() - 0.5).abs() < 0.001);
        fade.step(Duration::from_secs_f32(0.125), false);
        assert_eq!(fade.opacity(), 0.0);
    }

    #[test]
    fn scroll_indicator_motion_restarts_the_hold_and_a_long_frame_finishes_it() {
        let mut fade = IndicatorFade::default();
        fade.show();
        fade.step(Duration::from_secs(1), false);
        assert!(fade.opacity() < 1.0);
        fade.step(Duration::from_secs(1), true);
        assert_eq!(fade.opacity(), 1.0);
        fade.step(Duration::from_secs(2), false);
        assert_eq!(fade.opacity(), 0.0);
    }

    #[test]
    fn scroll_indicator_thumb_tracks_fraction_and_shortens_at_pulled_edges() -> Result<(), String> {
        let viewport = BoundingBox {
            x:      10.0,
            y:      20.0,
            width:  100.0,
            height: 100.0,
        };
        let thumb_at = |offset, overscroll| {
            let ScrollIndicatorState::Shown { thumb, .. } =
                solve_indicator(viewport, viewport.height, 400.0, offset, overscroll, 1.0)
            else {
                return Err("overflow must show a thumb".to_string());
            };
            Ok(thumb)
        };
        let start = thumb_at(0.0, 0.0)?;
        assert_eq!(
            start,
            BoundingBox {
                x:      105.0,
                y:      22.0,
                width:  3.0,
                height: 24.0,
            }
        );
        let middle = thumb_at(150.0, 0.0)?;
        assert_eq!(middle.y, 58.0);
        assert_eq!(thumb_at(0.0, -50.0)?.height, 8.0);
        assert_eq!(thumb_at(0.0, -50.0)?.y, 22.0);
        let end = thumb_at(300.0, 50.0)?;
        assert_eq!(end.y + end.height, 118.0);
        assert_eq!(
            solve_indicator(viewport, viewport.height, 100.0, 0.0, 0.0, 1.0),
            ScrollIndicatorState::Hidden
        );
        assert_eq!(
            solve_indicator(viewport, viewport.height, 400.0, 0.0, 0.0, 0.0),
            ScrollIndicatorState::Hidden
        );
        Ok(())
    }
}
