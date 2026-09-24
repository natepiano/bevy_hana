//! Scroll records computed at the panel boundary.

use bevy::math::Vec2;

use crate::PanelElementId;
use crate::layout::BoundingBox;
use crate::layout::LayoutResult;
use crate::layout::LayoutTree;
use crate::layout::VisibleScrollRegion;

/// Content size of a scrolling element against the viewport that shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollExtent {
    /// Size of the element's children, gaps included.
    pub content:  Vec2,
    /// Size of the element's content box, the visible window onto `content`.
    pub viewport: Vec2,
}

impl ScrollExtent {
    /// Largest offset per axis that still keeps the viewport inside the content.
    #[must_use]
    pub fn max_offset(self) -> Vec2 { (self.content - self.viewport).max(Vec2::ZERO) }

    /// Vertical offset from the start after moving `from_start` by `delta`,
    /// clamped to `[0, max_offset().y]`.
    #[must_use]
    pub fn scrolled(self, from_start: f32, delta: f32) -> f32 {
        (from_start + delta).clamp(0.0, self.max_offset().y)
    }
}

/// Computed scroll state of one named scrolling element on a laid-out panel.
///
/// Only elements built with `scroll_x`, `scroll_y`, or `scroll_y_from_end`
/// carry a record; a clip-only element does not. `extent`, `offset`, and
/// `line_pitch` are in the panel's layout unit, the unit its tree is authored
/// in. `bounds` and `visible_region` are in panel-local layout points, the space
/// [`ComputedDiegeticPanel::scroll_record_at_local_position`](crate::ComputedDiegeticPanel::scroll_record_at_local_position)
/// resolves against.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedScrollRecord {
    /// Panel-local semantic identity of the scrolling element.
    pub id:             PanelElementId,
    /// Element bounds in panel-local layout points.
    pub bounds:         BoundingBox,
    /// Part of `bounds` left visible by clipping ancestors, in panel-local
    /// layout points; pointer lookup tests against this.
    pub visible_region: VisibleScrollRegion,
    /// Content and viewport size in the panel's layout unit.
    pub extent:         ScrollExtent,
    /// Offset from the start the layout solve applied, after clamping, in the
    /// panel's layout unit.
    pub offset:         Vec2,
    /// Source element index in the panel's [`LayoutTree`].
    pub element_index:  usize,
    /// Distance one wheel line moves the content, in the panel's layout unit:
    /// one measured line of the element's scroll line style plus its child gap.
    pub line_pitch:     f32,
}

impl ComputedScrollRecord {
    /// Returns `true` when `panel_local` lies in the visible part of this
    /// record's element.
    #[must_use]
    pub fn contains(&self, panel_local: Vec2) -> bool { self.visible_region.contains(panel_local) }
}

/// Scrolling element found under a panel-local point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollRecordUnderPoint<'a> {
    /// The innermost named scrolling element containing the point.
    Inside(&'a ComputedScrollRecord),
    /// The point lies outside every named scrolling element on the panel,
    /// including when the panel scrolls nothing.
    OutsideScrollingElements,
}

/// Maps each solved scroll whose element carries an id to a record.
///
/// The engine solves in layout points; `layout_to_points` converts extent and
/// offset back to the panel's layout unit.
pub(super) fn collect_panel_scroll_records(
    tree: &LayoutTree,
    result: &LayoutResult,
    layout_to_points: f32,
) -> Vec<ComputedScrollRecord> {
    result
        .scrolls
        .iter()
        .filter_map(|solved| {
            let id = tree.element_id(solved.element_index)?;
            let computed = result.computed.get(solved.element_index)?;
            Some(ComputedScrollRecord {
                id:             id.clone(),
                bounds:         computed.bounds,
                visible_region: solved.visible_region,
                extent:         ScrollExtent {
                    content:  solved.content_points / layout_to_points,
                    viewport: solved.viewport_points / layout_to_points,
                },
                offset:         solved.offset_points / layout_to_points,
                element_index:  solved.element_index,
                line_pitch:     solved.line_pitch_points / layout_to_points,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::Vec2;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use hana_kana::ToF32;

    use super::ComputedScrollRecord;
    use super::ScrollRecordUnderPoint;
    use super::collect_panel_scroll_records;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::LayoutTreeChange;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::Sizing;
    use crate::TextDimensions;
    use crate::TextMeasure;
    use crate::TextStyle;
    use crate::layout::LayoutEngine;
    use crate::text::DiegeticTextMeasurer;

    fn approx_eq(a: f32, b: f32) -> bool { (a - b).abs() < 0.01 }

    fn measure(text: &str, measure: &TextMeasure) -> TextDimensions {
        TextDimensions {
            width:       text.len().to_f32() * measure.size,
            height:      measure.size,
            line_height: measure.size,
        }
    }

    /// A 100-tall named body over 300 of children, beside an unnamed clipping
    /// sibling of the same shape.
    fn body_records(offset: f32) -> Vec<ComputedScrollRecord> {
        let mut builder = LayoutBuilder::new(200.0, 100.0);
        builder.with(
            El::new().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                for id in [Some("body"), None] {
                    let mut body = El::column()
                        .width(Sizing::fixed(100.0))
                        .height(Sizing::fixed(100.0))
                        .scroll_y(offset);
                    if let Some(id) = id {
                        body = body.id(id);
                    }
                    builder.with(body, |builder| {
                        for _ in 0..3 {
                            builder.with(
                                El::new().width(Sizing::GROW).height(Sizing::fixed(100.0)),
                                |_| {},
                            );
                        }
                    });
                }
            },
        );
        let tree = builder.build();
        let result = LayoutEngine::new(Arc::new(measure)).compute(&tree, 200.0, 100.0, 1.0);
        collect_panel_scroll_records(&tree, &result, 1.0)
    }

    #[test]
    fn scroll_records_carry_the_engine_clamp_for_named_clipped_elements() {
        let records = body_records(50.0);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, PanelElementId::from("body"));
        assert!(approx_eq(record.extent.max_offset().y, 200.0));
        assert!(approx_eq(record.offset.y, 50.0));

        let clamped = body_records(f32::MAX);
        assert_eq!(clamped.len(), 1);
        assert!(approx_eq(clamped[0].offset.y, 200.0));
    }

    #[test]
    fn scroll_record_lookup_picks_the_innermost_record_under_a_point() {
        // A named 100×100 outer scroller whose first child is a named 40×40
        // inner scroller at the outer's origin.
        let mut builder = LayoutBuilder::new(200.0, 100.0);
        builder.with(
            El::column()
                .id("outer")
                .width(Sizing::fixed(100.0))
                .height(Sizing::fixed(100.0))
                .scroll_y(0.0),
            |builder| {
                builder.with(
                    El::column()
                        .id("inner")
                        .width(Sizing::fixed(40.0))
                        .height(Sizing::fixed(40.0))
                        .scroll_y(0.0),
                    |builder| {
                        builder.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(80.0)),
                            |_| {},
                        );
                    },
                );
                builder.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(200.0)),
                    |_| {},
                );
            },
        );
        let tree = builder.build();
        let result = LayoutEngine::new(Arc::new(measure)).compute(&tree, 200.0, 100.0, 1.0);
        let records = collect_panel_scroll_records(&tree, &result, 1.0);

        let mut panel = ComputedDiegeticPanel::default();
        assert_eq!(
            panel.scroll_record_at_local_position(Vec2::new(20.0, 20.0)),
            ScrollRecordUnderPoint::OutsideScrollingElements
        );

        panel.set_result_with_fields(
            result,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            records,
        );
        let id_under = |point: Vec2| match panel.scroll_record_at_local_position(point) {
            ScrollRecordUnderPoint::Inside(record) => Some(record.id.clone()),
            ScrollRecordUnderPoint::OutsideScrollingElements => None,
        };
        assert_eq!(
            id_under(Vec2::new(20.0, 20.0)),
            Some(PanelElementId::from("inner"))
        );
        assert_eq!(
            id_under(Vec2::new(80.0, 80.0)),
            Some(PanelElementId::from("outer"))
        );
        assert_eq!(id_under(Vec2::new(150.0, 20.0)), None);
    }

    /// A 100-tall body over three 100-tall rows with a 5 gap, optionally
    /// declaring a 10-size scroll line.
    fn gapped_body_record(declared: bool) -> Option<ComputedScrollRecord> {
        let mut builder = LayoutBuilder::new(200.0, 100.0);
        let mut body = El::column()
            .id("body")
            .width(Sizing::fixed(100.0))
            .height(Sizing::fixed(100.0))
            .gap(5.0)
            .scroll_y(0.0);
        if declared {
            body = body.scroll_line(TextStyle::new(10.0));
        }
        builder.with(body, |builder| {
            for _ in 0..3 {
                builder.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(100.0)),
                    |_| {},
                );
            }
        });
        records_of(&builder.build()).into_iter().next()
    }

    #[test]
    fn the_wheel_line_pitch_is_one_measured_scroll_line_plus_the_child_gap() {
        // The test measurer's line height equals the font size.
        let declared = gapped_body_record(true);
        assert!(declared.is_some_and(|record| approx_eq(record.line_pitch, 15.0)));
        let undeclared = gapped_body_record(false);
        assert!(
            undeclared.is_some_and(|record| approx_eq(
                record.line_pitch,
                TextStyle::default().size() + 5.0
            ))
        );
    }

    fn records_of(tree: &LayoutTree) -> Vec<ComputedScrollRecord> {
        let result = LayoutEngine::new(Arc::new(measure)).compute(tree, 200.0, 100.0, 1.0);
        collect_panel_scroll_records(tree, &result, 1.0)
    }

    #[test]
    fn a_named_clip_only_element_yields_no_scroll_record() {
        let mut builder = LayoutBuilder::new(200.0, 100.0);
        builder.with(
            El::column()
                .id("clipped")
                .width(Sizing::fixed(100.0))
                .height(Sizing::fixed(100.0))
                .clip(),
            |builder| {
                builder.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(300.0)),
                    |_| {},
                );
            },
        );
        assert!(records_of(&builder.build()).is_empty());
    }

    #[test]
    fn scroll_record_lookup_ignores_the_clipped_out_part_of_an_inner_scroller() {
        // A 30-tall header above a named 60-tall outer scroller at y 30..90,
        // scrolled 20 down, whose first child is a named 40×40 inner scroller.
        // The inner scroller's bounds are y 10..50; only 30..50 is visible.
        let mut builder =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        builder.with(
            El::new().width(Sizing::GROW).height(Sizing::fixed(30.0)),
            |_| {},
        );
        builder.with(
            El::column()
                .id("outer")
                .width(Sizing::fixed(100.0))
                .height(Sizing::fixed(60.0))
                .scroll_y(20.0),
            |builder| {
                builder.with(
                    El::column()
                        .id("inner")
                        .width(Sizing::fixed(40.0))
                        .height(Sizing::fixed(40.0))
                        .scroll_y(0.0),
                    |builder| {
                        builder.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(80.0)),
                            |_| {},
                        );
                    },
                );
                builder.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(200.0)),
                    |_| {},
                );
            },
        );
        let tree = builder.build();
        let result = LayoutEngine::new(Arc::new(measure)).compute(&tree, 200.0, 100.0, 1.0);
        let records = collect_panel_scroll_records(&tree, &result, 1.0);
        let inner = records
            .iter()
            .find(|record| record.id == PanelElementId::from("inner"));
        assert!(inner.is_some_and(|inner| inner.bounds.contains(Vec2::new(20.0, 20.0))));

        let mut panel = ComputedDiegeticPanel::default();
        panel.set_result_with_fields(
            result,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            records,
        );
        assert_eq!(
            panel.scroll_record_at_local_position(Vec2::new(20.0, 20.0)),
            ScrollRecordUnderPoint::OutsideScrollingElements
        );
        assert!(matches!(
            panel.scroll_record_at_local_position(Vec2::new(20.0, 40.0)),
            ScrollRecordUnderPoint::Inside(record) if record.id == PanelElementId::from("inner")
        ));
    }

    fn headless_layout_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        app
    }

    /// A millimetre panel whose body, named `body_id`, shows 40 mm of 100 mm of
    /// rows, scrolled 10 mm down.
    fn millimetre_body_tree(body_id: &str) -> LayoutTree {
        let mut builder =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        builder.with(
            El::column()
                .id(body_id)
                .width(Sizing::GROW)
                .height(Sizing::fixed(40.0))
                .scroll_y(10.0),
            |body| {
                for _ in 0..5 {
                    body.with(
                        El::new().width(Sizing::GROW).height(Sizing::fixed(20.0)),
                        |_| {},
                    );
                }
            },
        );
        builder.build()
    }

    fn spawn_millimetre_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        panel
    }

    fn panel_scroll_records(app: &App, panel: Entity) -> Vec<ComputedScrollRecord> {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .map_or_else(Vec::new, |computed| computed.scroll_records().to_vec())
    }

    #[test]
    fn scroll_records_of_a_millimetre_panel_are_in_millimetres() {
        let mut app = headless_layout_app();
        let panel = spawn_millimetre_panel(&mut app, millimetre_body_tree("body"));
        let points_per_mm = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map_or(f32::NAN, |panel| panel.layout_unit().to_points());
        assert!(!approx_eq(points_per_mm, 1.0));

        let records = panel_scroll_records(&app, panel);
        assert_eq!(records.len(), 1);
        let Some(record) = records.first() else {
            return;
        };
        assert!(approx_eq(record.extent.content.y, 100.0));
        assert!(approx_eq(record.extent.viewport.y, 40.0));
        assert!(approx_eq(record.extent.max_offset().y, 60.0));
        assert!(approx_eq(record.offset.y, 10.0));
        assert!(approx_eq(record.bounds.height, 40.0 * points_per_mm));
    }

    #[test]
    fn a_visual_only_rename_refreshes_the_scroll_record_id() {
        assert_eq!(
            millimetre_body_tree("body").classify_change(&millimetre_body_tree("renamed")),
            LayoutTreeChange::VisualOnly
        );
        let mut app = headless_layout_app();
        let panel = spawn_millimetre_panel(&mut app, millimetre_body_tree("body"));
        let layout_solves = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .map(ComputedDiegeticPanel::layout_solves);

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, millimetre_body_tree("renamed"));
        assert!(result.is_ok());
        app.update();

        // The rename took the geometry-reuse path, not a fresh solve.
        assert_eq!(
            app.world()
                .get::<ComputedDiegeticPanel>(panel)
                .map(ComputedDiegeticPanel::layout_solves),
            layout_solves
        );
        let ids: Vec<PanelElementId> = panel_scroll_records(&app, panel)
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, vec![PanelElementId::from("renamed")]);
    }
}
