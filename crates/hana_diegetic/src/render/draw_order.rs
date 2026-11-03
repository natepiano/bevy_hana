//! Stores the draw order for one panel command stream.
//!
//! `DrawOrder` maps each `(DrawZIndex, DrawSortTier, CommandIndex)` key from a
//! panel command stream to cached per-command render projections.
//!
//! Batch keys keep the authored [`DrawZIndex`] and the panel-local
//! [`DrawZIndexRank`] as compatibility splitters. Batch materials derive their
//! Bevy draw-item `depth_bias` from `DrawZIndexRank`, and uploaded record
//! `clip_depth_nudge` values are made relative to that batch's minimum
//! [`DrawOrderIndex`].
//!
//! [`OitPaintOrder`] advances a secondary OIT paint key by two units per
//! [`DrawStackLayer`]. A command's layer sits one above the highest layer among
//! earlier commands whose bounds it overlaps. The primary key is the unmodified
//! physical depth, so paint can only break a physical-depth tie. Opaque depth
//! is tested separately, before the local paint key is considered.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use hana_kana::ToF32;
use hana_kana::ToI32;
use hana_kana::ToU32;

use super::constants::ABUTTING_EDGE_ROUNDING;
use super::constants::LAYER_DEPTH_BIAS;
#[cfg(test)]
use super::constants::OIT_PACKED_PAINT_RANGE;
use super::constants::OIT_PAINT_STEP;
use super::panel_shapes;
use super::panel_shapes::PanelShapeLineLook;
use crate::layout::BoundingBox;
use crate::layout::DrawSortTier;
use crate::layout::DrawZIndex;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;

/// Zero-based index of a `RenderCommand` inside one panel's command stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CommandIndex(
    /// Slot in the `LayoutResult::commands` vector.
    usize,
);

/// Zero-based index of an `Element` inside one panel's layout tree.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ElementIndex(
    /// Slot in the `LayoutTree` element vector.
    usize,
);

/// Zero-based ordinal of a panel-shape source inside one panel's command stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ShapeOrdinal(
    /// Source ordinal assigned while traversing panel-shape commands.
    u32,
);

/// Zero-based ordinal of a primitive inside one resolved panel-shape source.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PrimitiveOrdinal(
    /// Primitive ordinal assigned while expanding one panel-shape source.
    u32,
);

/// Dense index in one panel's sorted draw-command stream.
///
/// `DrawOrder` assigns this once per draw-participating `RenderCommand`.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct DrawOrderIndex(i32);

/// Dense rank of a distinct `DrawZIndex` inside one panel's sorted z-index set.
///
/// `DrawOrder` assigns this once per authored z-index level. Many commands can
/// share the same rank when they share the same `DrawZIndex`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct DrawZIndexRank(i32);

/// Overlap layer of a draw command inside one panel's draw order.
///
/// Zero when no earlier command in the draw order shares area with the
/// command's bounds; otherwise one above the highest layer among those that do.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct DrawStackLayer(i32);

/// Screen `Transparent3d` sort value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScreenDepthBias(f32);

/// Share of one z-index band that a rank's surface runs spread across in
/// non-OIT `Transparent3d`. Panel shapes sort at its end, text at half a band,
/// and a tooltip shadow's runs at three quarters, so every run of either stays
/// under the next tier.
const SURFACE_RUNS_BAND_SHARE: f32 = 0.25;

/// Normalized panel-local paint order in OIT's secondary key.
/// Physical depth is compared first and is never offset by this value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OitPaintOrder(f32);

/// Shader clip-depth nudge value for vertex-pulled records.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ClipDepthNudge(f32);

/// Per-command material ordering values derived from one panel-local draw order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawCommandDepth {
    draw_order_index: DrawOrderIndex,
    z_index_rank:     DrawZIndexRank,
    z_index:          DrawZIndex,
    clip_depth_nudge: ClipDepthNudge,
    oit_paint_order:  OitPaintOrder,
}

/// Index-aligned draw order for one panel's command stream.
#[derive(Clone, Debug, Default)]
pub(crate) struct DrawOrder {
    depths:               Vec<Option<DrawCommandDepth>>,
    shape_depths:         Vec<Vec<DrawCommandDepth>>,
    deepest_z_index_rank: DrawZIndexRank,
}

/// Sort key for draw commands: `DrawZIndex`, then `DrawSortTier::sort_order`,
/// then `RenderCommand` stream index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DrawOrderKey {
    z_index:        DrawZIndex,
    draw_sort_tier: DrawSortTier,
    command_index:  CommandIndex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OrderedDrawCommand {
    draw_order_index: DrawOrderIndex,
    z_index_rank:     DrawZIndexRank,
    z_index:          DrawZIndex,
    stack_layer:      DrawStackLayer,
}

impl From<usize> for CommandIndex {
    fn from(value: usize) -> Self { Self(value) }
}

impl CommandIndex {
    /// Returns the index into the panel's `RenderCommand` vector.
    #[must_use]
    pub(crate) const fn get(self) -> usize { self.0 }
}

impl ElementIndex {
    /// Returns the index into the panel's `LayoutTree`.
    #[must_use]
    pub(crate) const fn get(self) -> usize { self.0 }
}

impl From<usize> for ElementIndex {
    fn from(value: usize) -> Self { Self(value) }
}

impl From<usize> for ShapeOrdinal {
    fn from(value: usize) -> Self { Self(value.to_u32()) }
}

impl ShapeOrdinal {
    /// Returns the panel-shape source ordinal as the GPU-friendly row value.
    #[must_use]
    #[expect(
        dead_code,
        reason = "Phase 9 panel-shape records will write this ordinal into GPU records"
    )]
    pub(crate) const fn as_u32(self) -> u32 { self.0 }
}

impl From<usize> for PrimitiveOrdinal {
    fn from(value: usize) -> Self { Self(value.to_u32()) }
}

impl PrimitiveOrdinal {
    /// Returns the primitive ordinal as the GPU-friendly row value.
    #[must_use]
    #[expect(
        dead_code,
        reason = "Phase 9 panel-shape records will write this ordinal into GPU records"
    )]
    pub(crate) const fn as_u32(self) -> u32 { self.0 }
}

impl ScreenDepthBias {
    #[must_use]
    pub(super) const fn get(self) -> f32 { self.0 }

    /// Lifts this bias past `earlier_runs` runs of one panel that share it, so
    /// each run sorts after the one before. The lift stays under
    /// `SURFACE_RUNS_BAND_SHARE` of a band, where the next draw tier starts.
    #[must_use]
    pub(super) fn after_earlier_runs(self, earlier_runs: u32) -> Self {
        let earlier_runs = earlier_runs.to_f32();
        let lift = SURFACE_RUNS_BAND_SHARE * earlier_runs / (earlier_runs + 1.0);
        Self(lift.mul_add(LAYER_DEPTH_BIAS, self.0))
    }
}

impl OitPaintOrder {
    #[must_use]
    pub(super) const fn get(self) -> f32 { self.0 }
}

impl ClipDepthNudge {
    #[must_use]
    pub(super) const fn get(self) -> f32 { self.0 }
}

impl From<usize> for DrawOrderIndex {
    fn from(value: usize) -> Self { Self(i32::try_from(value).unwrap_or(i32::MAX)) }
}

impl From<usize> for DrawZIndexRank {
    fn from(value: usize) -> Self { Self(i32::try_from(value).unwrap_or(i32::MAX)) }
}

impl DrawZIndexRank {
    pub(super) fn screen_depth_bias(self) -> ScreenDepthBias {
        ScreenDepthBias(self.0.to_f32() * LAYER_DEPTH_BIAS)
    }

    /// Sorts panel shapes after every same-rank surface run in non-OIT
    /// `Transparent3d`, as the draw order puts the panel-shape tier after the
    /// surface tier.
    pub(super) fn screen_panel_shape_depth_bias(self) -> ScreenDepthBias {
        ScreenDepthBias((self.0.to_f32() + SURFACE_RUNS_BAND_SHARE) * LAYER_DEPTH_BIAS)
    }

    /// Sorts panel text after same-rank fills in non-OIT `Transparent3d`.
    pub(super) fn screen_text_depth_bias(self) -> ScreenDepthBias {
        ScreenDepthBias((self.0.to_f32() + 0.5) * LAYER_DEPTH_BIAS)
    }

    /// Sorts a tooltip shadow over a host whose deepest rank is this one, in
    /// non-OIT `Transparent3d`: after that rank's text, so the shadow darkens
    /// it, and before the next rank's fills.
    pub(super) fn screen_tooltip_shadow_depth_bias(self) -> ScreenDepthBias {
        ScreenDepthBias((self.0.to_f32() + 0.75) * LAYER_DEPTH_BIAS)
    }

    /// Hardware depth-bias integer of this rank's draws, apart from any sort
    /// priority their batch adds.
    pub(super) fn hardware_depth_bias(self) -> i32 { self.screen_depth_bias().get().to_i32() }
}

impl DrawOrderIndex {
    pub(super) fn clip_depth_nudge(self) -> ClipDepthNudge { ClipDepthNudge(self.0.to_f32()) }

    fn to_usize(self) -> usize { usize::try_from(self.0).unwrap_or(usize::MAX) }
}

impl DrawStackLayer {
    const fn above(self) -> Self { Self(self.0.saturating_add(1)) }

    fn oit_paint_order(self) -> OitPaintOrder {
        OitPaintOrder(self.0.saturating_add(1).to_f32() * OIT_PAINT_STEP)
    }
}

impl DrawCommandDepth {
    fn new(command: OrderedDrawCommand) -> Self {
        Self {
            draw_order_index: command.draw_order_index,
            z_index_rank:     command.z_index_rank,
            z_index:          command.z_index,
            clip_depth_nudge: command.draw_order_index.clip_depth_nudge(),
            oit_paint_order:  command.stack_layer.oit_paint_order(),
        }
    }

    /// Returns the dense index in the panel-local draw order.
    #[cfg(test)]
    const fn draw_order_index_for_test(self) -> DrawOrderIndex { self.draw_order_index }

    /// Returns the dense index in the panel-local draw order.
    pub(super) const fn draw_order_index_value(self) -> DrawOrderIndex { self.draw_order_index }

    /// Returns the draw-order index as a nonnegative `usize`.
    pub(super) fn draw_order_index(self) -> usize { self.draw_order_index.to_usize() }

    /// Returns the command's authored z-index.
    pub(super) const fn z_index(self) -> DrawZIndex { self.z_index }

    /// Returns the dense rank of the command's authored z-index in its panel.
    pub(super) const fn z_index_rank(self) -> DrawZIndexRank { self.z_index_rank }

    /// Returns this command's clip-depth nudge, in layer-units, for the non-OIT
    /// shader path.
    pub(super) const fn clip_depth_nudge(self) -> ClipDepthNudge { self.clip_depth_nudge }

    /// Returns the local paint order used only at equal physical depth.
    pub(super) const fn oit_paint_order(self) -> OitPaintOrder { self.oit_paint_order }

    /// Places a moving control surface above every paint part of its track.
    /// A viewport-clipped border can retain a later command than its child.
    pub(super) fn in_front_of(self, track: Self) -> Self {
        let draw_order_index = DrawOrderIndex(
            self.draw_order_index
                .0
                .max(track.draw_order_index.0.saturating_add(1)),
        );
        Self {
            draw_order_index,
            clip_depth_nudge: draw_order_index.clip_depth_nudge(),
            oit_paint_order: OitPaintOrder(
                self.oit_paint_order
                    .0
                    .max(track.oit_paint_order.0 + OIT_PAINT_STEP),
            ),
            ..self
        }
    }
}

impl DrawOrder {
    /// Builds index-aligned draw order from a full panel command stream.
    pub(crate) fn from_commands(commands: &[RenderCommand]) -> Self {
        let (enumerated, shape_layers) = enumerate_draw_commands(commands);
        let depths: Vec<Option<DrawCommandDepth>> = enumerated
            .into_iter()
            .map(|command| command.map(DrawCommandDepth::new))
            .collect();
        let shape_depths: Vec<Vec<DrawCommandDepth>> = shape_layers
            .into_iter()
            .map(|layers| layers.into_iter().map(DrawCommandDepth::new).collect())
            .collect();
        let deepest_z_index_rank = depths
            .iter()
            .flatten()
            .chain(shape_depths.iter().flatten())
            .map(|depth: &DrawCommandDepth| depth.z_index_rank)
            .max()
            .unwrap_or_default();
        Self {
            depths,
            shape_depths,
            deepest_z_index_rank,
        }
    }

    /// Returns the deepest z-index rank any of this panel's commands draws at.
    pub(crate) const fn deepest_z_index_rank(&self) -> DrawZIndexRank { self.deepest_z_index_rank }

    /// Returns this command's draw-depth values, or `None` for scissor
    /// commands and out-of-range indices.
    pub(crate) fn depth_for(
        &self,
        command_index: impl Into<CommandIndex>,
    ) -> Option<DrawCommandDepth> {
        let command_index = command_index.into();
        self.depths.get(command_index.get()).copied().flatten()
    }

    /// Returns the layer assigned to one line of a panel-shape command.
    pub(crate) fn depth_for_shape_line(
        &self,
        command_index: impl Into<CommandIndex>,
        line_index: usize,
    ) -> Option<DrawCommandDepth> {
        self.shape_depths
            .get(command_index.into().get())?
            .get(line_index)
            .copied()
    }
}

impl Ord for DrawOrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.z_index()
            .cmp(&other.z_index())
            .then(
                self.draw_sort_tier
                    .sort_order()
                    .cmp(&other.draw_sort_tier.sort_order()),
            )
            .then(self.command_index.cmp(&other.command_index))
    }
}

impl DrawOrderKey {
    const fn z_index(self) -> DrawZIndex { self.z_index }
}

impl PartialOrd for DrawOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

/// Enumerates draw-participating commands into panel-local draw-order indices.
///
/// The returned vector is index-aligned with `commands`; scissor commands map
/// to `None`. Each `DrawOrderIndex` stores the dense index.
#[cfg(test)]
fn index_draw_commands_for_test(commands: &[RenderCommand]) -> Vec<Option<DrawOrderIndex>> {
    enumerate_draw_commands(commands)
        .0
        .into_iter()
        .map(|command| command.map(|command| command.draw_order_index))
        .collect()
}

fn enumerate_draw_commands(
    commands: &[RenderCommand],
) -> (
    Vec<Option<OrderedDrawCommand>>,
    Vec<Vec<OrderedDrawCommand>>,
) {
    let mut keyed_commands = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            command.kind.draw_sort_tier().map(|step| {
                (
                    DrawOrderKey {
                        z_index:        command.z_index,
                        draw_sort_tier: step,
                        command_index:  CommandIndex::from(index),
                    },
                    index,
                )
            })
        })
        .collect::<Vec<_>>();

    keyed_commands.sort_by_key(|(key, _)| *key);

    let mut z_index_ranks = BTreeMap::new();
    for (key, _) in &keyed_commands {
        if !z_index_ranks.contains_key(&key.z_index()) {
            z_index_ranks.insert(key.z_index(), DrawZIndexRank::from(z_index_ranks.len()));
        }
    }

    let mut enumerated = vec![None; commands.len()];
    let mut shape_layers = vec![Vec::new(); commands.len()];
    let mut stacked = Vec::with_capacity(keyed_commands.len());
    for (draw_order_position, (key, index)) in keyed_commands.into_iter().enumerate() {
        let command = &commands[index];
        let ordered = OrderedDrawCommand {
            draw_order_index: DrawOrderIndex(
                i32::try_from(draw_order_position).unwrap_or(i32::MAX),
            ),
            z_index_rank:     z_index_ranks
                .get(&key.z_index())
                .copied()
                .unwrap_or_default(),
            z_index:          key.z_index(),
            stack_layer:      DrawStackLayer::default(),
        };
        if let RenderCommandKind::PanelShapes { shapes } = &command.kind
            && !shapes.is_empty()
        {
            for shape in shapes {
                let bounds = shape.visual_bounds();
                let look = panel_shapes::line_look(command.element_idx, shape);
                let stack_layer = stack_layer_over(&stacked, bounds, Some((index, &look)));
                stacked.push((bounds, stack_layer, Some((index, look))));
                shape_layers[index].push(OrderedDrawCommand {
                    stack_layer,
                    ..ordered
                });
            }
            enumerated[index] = shape_layers[index].first().copied();
        } else {
            let bounds = command.bounds;
            let stack_layer = stack_layer_over(&stacked, bounds, None);
            stacked.push((bounds, stack_layer, None));
            enumerated[index] = Some(OrderedDrawCommand {
                stack_layer,
                ..ordered
            });
        }
    }
    (enumerated, shape_layers)
}

/// Returns the layer for a command drawn after `stacked`: one above every
/// earlier command it shares area with, or layer zero when it covers none.
fn stack_layer_over(
    stacked: &[(
        BoundingBox,
        DrawStackLayer,
        Option<(usize, PanelShapeLineLook)>,
    )],
    bounds: BoundingBox,
    same_look: Option<(usize, &PanelShapeLineLook)>,
) -> DrawStackLayer {
    stacked
        .iter()
        .filter(|(below, _, _)| shares_area(*below, bounds))
        .map(|(_, layer, previous_look)| {
            if previous_look.as_ref().zip(same_look).is_some_and(
                |((previous_command, previous), (command, current))| {
                    *previous_command == command && previous == current
                },
            ) {
                *layer
            } else {
                layer.above()
            }
        })
        .max()
        .unwrap_or_default()
}

/// Whether two command bounds share area wider than the float rounding that
/// can leave between the touching edges of two abutting boxes.
fn shares_area(below: BoundingBox, above: BoundingBox) -> bool {
    let magnitude = [
        below.x,
        below.y,
        below.x + below.width,
        below.y + below.height,
        above.x,
        above.y,
        above.x + above.width,
        above.y + above.height,
    ]
    .into_iter()
    .map(f32::abs)
    .fold(0.0, f32::max);
    let rounding = magnitude * ABUTTING_EDGE_ROUNDING;
    below
        .intersect(&above)
        .is_some_and(|shared| shared.width > rounding && shared.height > rounding)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::image::Image;
    use bevy::prelude::Color;
    use bevy::prelude::Handle;

    use super::*;
    use crate::layout::Border;
    use crate::layout::BoundingBox;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommandKind;
    use crate::layout::TextStyle;

    const LOWERED_LEVEL: DrawZIndex = DrawZIndex(-1);
    /// `DrawZIndex` pairs `(low, high)` spanning negative, default, positive, and
    /// saturated ranges.
    const ORDERED_Z_INDEX_PAIRS: [(i8, i8); 6] = [
        (i8::MIN, -1),
        (-1, 0),
        (0, 1),
        (1, 63),
        (63, 65),
        (65, i8::MAX),
    ];
    const RAISED_LEVEL: DrawZIndex = DrawZIndex(1);
    /// Bounds every representative command shares, so each one stacks on all
    /// commands drawn before it.
    const STACKED_BOUNDS: BoundingBox = BoundingBox {
        x:      0.0,
        y:      0.0,
        width:  10.0,
        height: 10.0,
    };

    fn text_at(x: f32, y: f32, width: f32, height: f32) -> RenderCommand {
        RenderCommand {
            bounds:      BoundingBox {
                x,
                y,
                width,
                height,
            },
            kind:        text(),
            element_idx: 0,
            z_index:     DrawZIndex::default(),
        }
    }

    fn rectangle_at(x: f32, y: f32, width: f32, height: f32) -> RenderCommand {
        RenderCommand {
            kind: rectangle(),
            ..text_at(x, y, width, height)
        }
    }

    fn representative_streams() -> [Vec<RenderCommand>; 2] {
        [
            commands_from_kinds([
                (rectangle(), DrawZIndex::default()),
                (text(), DrawZIndex::default()),
                (image(), DrawZIndex::default()),
                (panel_shapes(), DrawZIndex::default()),
                (text(), DrawZIndex::default()),
                (RenderCommandKind::ScissorStart, DrawZIndex::default()),
                (RenderCommandKind::ScissorEnd, DrawZIndex::default()),
            ]),
            commands_from_kinds([
                (text(), DrawZIndex::default()),
                (panel_shapes(), DrawZIndex::default()),
                (rectangle(), DrawZIndex::default()),
                (RenderCommandKind::ScissorStart, DrawZIndex::default()),
                (border(), DrawZIndex::default()),
                (text(), DrawZIndex::default()),
                (RenderCommandKind::ScissorEnd, DrawZIndex::default()),
                (image(), DrawZIndex::default()),
            ]),
        ]
    }

    fn commands_from_kinds<const N: usize>(
        entries: [(RenderCommandKind, DrawZIndex); N],
    ) -> Vec<RenderCommand> {
        entries
            .into_iter()
            .enumerate()
            .map(|(element_idx, (kind, z_index))| RenderCommand {
                bounds: STACKED_BOUNDS,
                kind,
                element_idx,
                z_index,
            })
            .collect()
    }

    fn rectangle() -> RenderCommandKind {
        RenderCommandKind::Rectangle {
            color:  Color::WHITE,
            source: RectangleSource::Background,
        }
    }

    fn image() -> RenderCommandKind {
        RenderCommandKind::Image {
            handle: Handle::<Image>::default(),
            tint:   Color::WHITE,
        }
    }

    fn border() -> RenderCommandKind {
        RenderCommandKind::Border {
            border: Border::default(),
        }
    }

    fn panel_shapes() -> RenderCommandKind { RenderCommandKind::PanelShapes { shapes: Vec::new() } }

    fn text() -> RenderCommandKind {
        RenderCommandKind::Text {
            text:   String::new(),
            config: TextStyle::default(),
        }
    }

    fn drawing_indices(commands: &[RenderCommand]) -> Vec<CommandIndex> {
        commands
            .iter()
            .enumerate()
            .filter(|(_, command)| command.kind.draw_sort_tier().is_some())
            .map(|(index, _)| CommandIndex::from(index))
            .collect()
    }

    fn draw_order_index_at(
        indices_by_command_index: &[Option<DrawOrderIndex>],
        command_index: CommandIndex,
    ) -> DrawOrderIndex {
        indices_by_command_index[command_index.get()].expect("drawing commands receive draw order")
    }

    fn draw_depth_at(draw_order: &DrawOrder, command_index: CommandIndex) -> DrawCommandDepth {
        draw_order
            .depth_for(command_index.get())
            .expect("drawing command receives draw depth")
    }

    fn screen_depth_bias(draw_depth: DrawCommandDepth) -> ScreenDepthBias {
        draw_depth.z_index_rank().screen_depth_bias()
    }

    fn draw_order_indices_for_tier(
        commands: &[RenderCommand],
        draw_order_indices: &[Option<DrawOrderIndex>],
        draw_sort_tier: DrawSortTier,
    ) -> Vec<i32> {
        commands
            .iter()
            .enumerate()
            .filter(|(_, command)| command.kind.draw_sort_tier() == Some(draw_sort_tier))
            .map(|(index, _)| draw_order_index_at(draw_order_indices, CommandIndex::from(index)).0)
            .collect()
    }

    fn draw_order_indices_for_z_index(
        commands: &[RenderCommand],
        draw_order_indices: &[Option<DrawOrderIndex>],
        z_index: DrawZIndex,
    ) -> Vec<i32> {
        commands
            .iter()
            .enumerate()
            .filter(|(_, command)| command.z_index == z_index)
            .map(|(index, _)| draw_order_index_at(draw_order_indices, CommandIndex::from(index)).0)
            .collect()
    }

    fn assert_scissors_excluded(commands: &[RenderCommand]) {
        let draw_order_indices = index_draw_commands_for_test(commands);
        for (index, command) in commands.iter().enumerate() {
            if command.kind.draw_sort_tier().is_none() {
                assert_eq!(
                    draw_order_indices[index], None,
                    "scissor command {index} maps to None"
                );
            }
        }
        assert_eq!(
            draw_order_indices.iter().flatten().count(),
            drawing_indices(commands).len(),
            "only drawing commands receive draw order indices",
        );
    }

    fn assert_screen_depth_bias_uses_z_index_rank(commands: &[RenderCommand]) {
        let draw_order = DrawOrder::from_commands(commands);

        for index in drawing_indices(commands) {
            let draw_command_depth = draw_depth_at(&draw_order, index);
            assert_eq!(
                screen_depth_bias(draw_command_depth).get().to_bits(),
                draw_command_depth
                    .z_index_rank()
                    .screen_depth_bias()
                    .get()
                    .to_bits(),
                "command {index:?} uses z-index rank for screen depth",
            );
        }
    }

    fn assert_no_override_draw_order_uses_draw_order_indices(commands: &[RenderCommand]) {
        assert!(
            commands
                .iter()
                .all(|command| command.z_index == DrawZIndex::default()),
            "no-override streams use the default z-index level",
        );
        let draw_order_indices = index_draw_commands_for_test(commands);
        let draw_order = DrawOrder::from_commands(commands);

        for index in drawing_indices(commands) {
            let command = &commands[index.get()];
            let draw_order_index = draw_order_index_at(&draw_order_indices, index);
            let draw_command_depth = draw_depth_at(&draw_order, index);
            assert_eq!(draw_command_depth.z_index(), 0.into());
            assert_eq!(
                draw_command_depth.draw_order_index_for_test(),
                draw_order_index
            );
            assert_eq!(draw_command_depth.z_index_rank(), DrawZIndexRank::default());
            assert_eq!(
                screen_depth_bias(draw_command_depth).get().to_bits(),
                DrawZIndexRank::default()
                    .screen_depth_bias()
                    .get()
                    .to_bits(),
                "no-override command {index:?} uses the default z-index rank for screen depth",
            );
            assert_eq!(
                draw_command_depth.clip_depth_nudge().get().to_bits(),
                draw_order_index.clip_depth_nudge().get().to_bits(),
                "no-override command {index:?} uses the draw order index for clip depth",
            );
            assert_eq!(
                draw_command_depth.oit_paint_order().get().to_bits(),
                (draw_order_index.0.saturating_add(1).to_f32() * OIT_PAINT_STEP).to_bits(),
                "no-override command {index:?} keeps its positive OIT paint order",
            );

            assert!(command.kind.draw_sort_tier().is_some());
        }
    }

    #[test]
    fn sorted_and_oit_orderings_agree_for_every_z_index_pair() {
        for (low, high) in ORDERED_Z_INDEX_PAIRS {
            let commands = commands_from_kinds([
                (rectangle(), DrawZIndex(low)),
                (rectangle(), DrawZIndex(high)),
            ]);
            let draw_order = DrawOrder::from_commands(&commands);
            let low_depth = draw_depth_at(&draw_order, CommandIndex::from(0));
            let high_depth = draw_depth_at(&draw_order, CommandIndex::from(1));
            assert!(
                screen_depth_bias(low_depth).get() < screen_depth_bias(high_depth).get(),
                "sorted bias must rise from {low} to {high}",
            );
            assert!(
                low_depth.oit_paint_order().get() < high_depth.oit_paint_order().get(),
                "OIT paint order must rise from {low} to {high}",
            );
        }
    }

    #[test]
    fn draw_order_indices_order_tiers_for_default_z_index() {
        for commands in representative_streams() {
            assert!(
                commands
                    .iter()
                    .all(|command| command.z_index == DrawZIndex::default()),
                "representative streams use the default z-index level",
            );
            let draw_order_indices = index_draw_commands_for_test(&commands);
            let fill_max =
                draw_order_indices_for_tier(&commands, &draw_order_indices, DrawSortTier::Surface)
                    .into_iter()
                    .max()
                    .expect("representative streams include fill commands");
            let panel_shape_min = draw_order_indices_for_tier(
                &commands,
                &draw_order_indices,
                DrawSortTier::PanelShape,
            )
            .into_iter()
            .min()
            .expect("representative streams include panel-shape commands");
            let text_min =
                draw_order_indices_for_tier(&commands, &draw_order_indices, DrawSortTier::Text)
                    .into_iter()
                    .min()
                    .expect("representative streams include text commands");
            assert!(fill_max < panel_shape_min);
            assert!(panel_shape_min < text_min);
        }
    }

    #[test]
    fn draw_order_indices_exclude_scissors() {
        for commands in representative_streams() {
            assert_scissors_excluded(&commands);
        }
    }

    #[test]
    fn shape_line_depth_returns_none_for_missing_line() {
        let commands = commands_from_kinds([(panel_shapes(), DrawZIndex::default())]);
        let draw_order = DrawOrder::from_commands(&commands);
        assert_eq!(draw_order.depth_for_shape_line(0, 0), None);
        assert_eq!(draw_order.depth_for_shape_line(1, 0), None);
    }

    #[test]
    fn oit_paint_orders_start_at_one_local_step() {
        let commands = representative_streams()
            .into_iter()
            .next()
            .expect("representative streams include drawing commands");
        let draw_order = DrawOrder::from_commands(&commands);
        let lowest_offset = drawing_indices(&commands)
            .into_iter()
            .map(|index| draw_depth_at(&draw_order, index).oit_paint_order().get())
            .reduce(f32::min)
            .expect("representative stream includes drawing commands");

        assert_eq!(lowest_offset.to_bits(), OIT_PAINT_STEP.to_bits());
    }

    #[test]
    fn oit_draw_callers_test_opaque_occlusion_before_reconstructing_the_plane() {
        let shaders = [
            (
                "sdf_panel.wesl",
                include_str!("../shaders/sdf_panel.wesl"),
                1,
            ),
            (
                "image_panel.wesl",
                include_str!("../shaders/image_panel.wesl"),
                1,
            ),
            (
                "analytic_path.wesl",
                include_str!("analytic_paths/analytic_path.wesl"),
                1,
            ),
        ];
        for (name, shader, expected_callers) in shaders {
            let lines = shader.lines().map(str::trim).collect::<Vec<_>>();
            let tolerance = lines
                .iter()
                .find_map(|line| line.strip_prefix("const OIT_OCCLUSION_TOLERANCE: f32 = "))
                .and_then(|value| value.strip_suffix(';'))
                .and_then(|value| value.parse::<f32>().ok())
                .expect("each oit_draw shader declares its occlusion tolerance");
            assert_eq!(tolerance.to_bits(), OIT_PAINT_STEP.to_bits(), "{name}");

            let callers = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.starts_with("oit_draw_layer("))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert_eq!(callers.len(), expected_callers, "{name}");
            for caller in callers {
                let before = &lines[..caller];
                let position = before
                    .iter()
                    .rposition(|line| line.ends_with("= in.position;"))
                    .expect("each oit_draw position starts from the fragment position");
                let occlusion = before
                    .iter()
                    .rposition(|line| {
                        line.contains("OIT_OCCLUSION_TOLERANCE < prepass_utils::prepass_depth(")
                    })
                    .expect("each oit_draw caller tests the depth prepass itself");
                assert!(
                    lines[caller].contains("oit_paint_order"),
                    "each caller passes its separate paint offset",
                );
                assert!(
                    position < occlusion && occlusion < caller,
                    "{name}: the oit_draw_layer call on line {} must test opaque occlusion first",
                    caller + 1,
                );
                assert_eq!(
                    occlusion.checked_sub(1).and_then(|index| before.get(index)),
                    Some(&"@if(DEPTH_PREPASS) {"),
                    "{name}",
                );
            }
        }
    }

    #[test]
    fn adjacent_panel_oit_layers_keep_two_packed_quanta_at_focus_distance() {
        let sdf = include_str!("../shaders/sdf_panel.wesl");
        let text = include_str!("analytic_paths/analytic_path.wesl");
        assert!(sdf.contains("oit_draw_layer(oit_pos, out.color, record.oit_paint_order)"));
        assert!(text.contains("return run_records[render_index(in)].oit_paint_order;"));
        assert!(text.contains("return uniforms.oit_paint_order;"));
        assert!(!sdf.contains("bounded_oit_paint_order"));
        assert!(!text.contains("bounded_oit_paint_order"));
        assert_eq!(OIT_PAINT_STEP * OIT_PACKED_PAINT_RANGE, 2.0);

        for zoom in [1.0_f32, 4.45, 7.754, 12.0, 24.0] {
            let near = zoom * 0.001;
            let depth = near / zoom;
            let physical = depth.to_bits();
            let mut previous_packed = 0;
            for layer in 1_u16..=16 {
                let offset = f32::from(layer) * OIT_PAINT_STEP;
                let packed = (offset * OIT_PACKED_PAINT_RANGE).round().to_u32();
                assert_eq!((physical, packed).0, depth.to_bits());
                assert!(
                    packed - previous_packed >= 2,
                    "layer {layer} at zoom {zoom}: {} packed quanta",
                    packed - previous_packed,
                );
                previous_packed = packed;
            }
        }
    }

    #[test]
    fn first_four_panel_oit_layers_stay_behind_info_card_glass() {
        let sdf = include_str!("../shaders/sdf_panel.wesl");
        let text = include_str!("analytic_paths/analytic_path.wesl");
        assert!(sdf.contains("oit_draw_layer(oit_pos, out.color, record.oit_paint_order)"));
        assert!(text.contains("return run_records[render_index(in)].oit_paint_order;"));
        assert!(text.contains("return uniforms.oit_paint_order;"));
        assert!(!sdf.contains("bounded_oit_paint_order"));
        assert!(!text.contains("bounded_oit_paint_order"));

        let distance = 4.3_f32;
        let near = 7.754 * 0.001;
        let depth = near / distance;
        for layer in 1_u16..=4 {
            let offset = f32::from(layer) * OIT_PAINT_STEP;
            let quanta = (offset * OIT_PACKED_PAINT_RANGE).round().to_u32();
            let key = (depth.to_bits(), quanta);
            let physical_depth = f32::from_bits(key.0);
            assert_eq!(physical_depth.to_bits(), depth.to_bits());
            assert!(key < ((near / (distance - 0.0025)).to_bits(), 0));
            let world_push = distance - near / physical_depth;
            assert!(
                world_push < 0.0025,
                "layer {layer} pushes {world_push} m into the glass",
            );
        }
    }

    #[test]
    fn oit_paint_orders_climb_only_where_commands_overlap() {
        let commands = vec![
            rectangle_at(0.0, 0.0, 100.0, 200.0),
            rectangle_at(10.0, 10.0, 40.0, 20.0),
            rectangle_at(12.0, 12.0, 10.0, 10.0),
            rectangle_at(60.0, 10.0, 30.0, 20.0),
        ];
        let draw_order = DrawOrder::from_commands(&commands);
        let offsets = (0..commands.len())
            .map(|index| draw_depth_at(&draw_order, CommandIndex::from(index)).oit_paint_order())
            .map(OitPaintOrder::get)
            .collect::<Vec<_>>();

        assert_eq!(offsets[0].to_bits(), OIT_PAINT_STEP.to_bits(), "plate");
        assert_eq!(
            offsets[1].to_bits(),
            (2.0 * OIT_PAINT_STEP).to_bits(),
            "track"
        );
        assert_eq!(
            offsets[2].to_bits(),
            (3.0 * OIT_PAINT_STEP).to_bits(),
            "knob"
        );
        assert_eq!(
            offsets[3].to_bits(),
            offsets[1].to_bits(),
            "a sibling beside the track shares its layer",
        );
    }

    #[test]
    fn abutting_rows_share_one_oit_layer_over_their_plate() {
        const ROWS: usize = 160;
        const ROW_HEIGHT: f32 = 12.3;

        let mut commands = vec![rectangle_at(0.0, 0.0, 100.0, 2000.0)];
        let mut top = 0.0_f32;
        for _ in 0..ROWS {
            commands.push(text_at(0.0, top, 100.0, ROW_HEIGHT));
            top += ROW_HEIGHT;
        }
        let draw_order = DrawOrder::from_commands(&commands);
        let largest_offset = (0..commands.len())
            .map(|index| {
                draw_depth_at(&draw_order, CommandIndex::from(index))
                    .oit_paint_order()
                    .get()
            })
            .reduce(f32::max)
            .expect("drawn rows");

        assert_eq!(largest_offset.to_bits(), (2.0 * OIT_PAINT_STEP).to_bits());
    }

    #[test]
    fn screen_depth_bias_uses_z_index_rank_for_representative_streams() {
        for commands in representative_streams() {
            assert_screen_depth_bias_uses_z_index_rank(&commands);
        }
    }

    #[test]
    fn no_override_draw_order_uses_draw_order_indices() {
        for commands in representative_streams() {
            assert_no_override_draw_order_uses_draw_order_indices(&commands);
        }
    }

    #[test]
    fn draw_order_uses_indices_and_positive_oit_paint_orders() {
        for commands in representative_streams() {
            let draw_order_indices = index_draw_commands_for_test(&commands);
            let draw_order = DrawOrder::from_commands(&commands);

            for index in drawing_indices(&commands) {
                let draw_order_index = draw_order_index_at(&draw_order_indices, index);
                let draw_depth = draw_order
                    .depth_for(index.get())
                    .expect("drawing command receives draw depth");
                assert_eq!(draw_depth.draw_order_index_for_test(), draw_order_index);
                assert_eq!(
                    screen_depth_bias(draw_depth).get().to_bits(),
                    draw_depth
                        .z_index_rank()
                        .screen_depth_bias()
                        .get()
                        .to_bits(),
                );
                assert_eq!(
                    draw_depth.clip_depth_nudge().get().to_bits(),
                    draw_order_index.clip_depth_nudge().get().to_bits(),
                );
                assert_eq!(
                    draw_depth.oit_paint_order().get().to_bits(),
                    (draw_order_index.0.saturating_add(1).to_f32() * OIT_PAINT_STEP).to_bits(),
                );
            }
        }
    }

    #[test]
    fn screen_depth_bias_uses_draw_z_index_rank() {
        let commands = commands_from_kinds([
            (text(), LOWERED_LEVEL),
            (panel_shapes(), LOWERED_LEVEL),
            (rectangle(), DrawZIndex::default()),
            (image(), DrawZIndex::default()),
            (text(), RAISED_LEVEL),
        ]);
        let draw_order = DrawOrder::from_commands(&commands);

        let lowered_text = draw_depth_at(&draw_order, CommandIndex::from(0));
        let lowered_shape = draw_depth_at(&draw_order, CommandIndex::from(1));
        let default_rectangle = draw_depth_at(&draw_order, CommandIndex::from(2));
        let default_image = draw_depth_at(&draw_order, CommandIndex::from(3));
        let raised_text = draw_depth_at(&draw_order, CommandIndex::from(4));

        assert_eq!(lowered_text.z_index_rank(), DrawZIndexRank(0));
        assert_eq!(lowered_shape.z_index_rank(), DrawZIndexRank(0));
        assert_eq!(default_rectangle.z_index_rank(), DrawZIndexRank(1));
        assert_eq!(default_image.z_index_rank(), DrawZIndexRank(1));
        assert_eq!(raised_text.z_index_rank(), DrawZIndexRank(2));

        assert_eq!(
            screen_depth_bias(lowered_text).get().to_bits(),
            screen_depth_bias(lowered_shape).get().to_bits(),
            "commands in the same authored z-index share screen depth",
        );
        assert_ne!(
            lowered_text.clip_depth_nudge().get().to_bits(),
            lowered_shape.clip_depth_nudge().get().to_bits(),
            "commands in the same authored z-index still keep separate per-command clip depth",
        );
        assert_eq!(
            screen_depth_bias(default_rectangle).get().to_bits(),
            screen_depth_bias(default_image).get().to_bits(),
            "commands in the default authored z-index share screen depth",
        );
        assert_eq!(
            screen_depth_bias(lowered_text).get().to_bits(),
            DrawZIndexRank(0).screen_depth_bias().get().to_bits(),
        );
        assert_eq!(
            screen_depth_bias(default_rectangle).get().to_bits(),
            DrawZIndexRank(1).screen_depth_bias().get().to_bits(),
        );
        assert_eq!(
            screen_depth_bias(raised_text).get().to_bits(),
            DrawZIndexRank(2).screen_depth_bias().get().to_bits(),
        );
    }

    #[test]
    fn panel_text_sorts_after_fills_at_the_same_z_without_oit() {
        let commands = commands_from_kinds([
            (rectangle(), DrawZIndex::default()),
            (text(), DrawZIndex::default()),
        ]);
        let draw_order = DrawOrder::from_commands(&commands);
        let fill = draw_depth_at(&draw_order, CommandIndex::from(0));
        let text = draw_depth_at(&draw_order, CommandIndex::from(1));

        assert_eq!(fill.z_index_rank(), text.z_index_rank());
        assert!(
            fill.z_index_rank().screen_depth_bias().get()
                < text.z_index_rank().screen_text_depth_bias().get(),
            "Transparent3d sorts increasing distance, so text needs a higher bias"
        );
    }

    #[test]
    fn panel_text_sorts_between_its_fills_and_the_next_rank() {
        for rank in [0, 1, 12] {
            let current = DrawZIndexRank(rank);
            let next = DrawZIndexRank(rank + 1);
            let text_bias = current.screen_text_depth_bias().get();
            assert!(current.screen_depth_bias().get() < text_bias);
            assert!(text_bias < next.screen_depth_bias().get());
        }
    }

    #[test]
    fn z_index_overrides_move_commands_between_sort_tiers() {
        let raised_fill_commands = commands_from_kinds([
            (text(), DrawZIndex::default()),
            (rectangle(), RAISED_LEVEL),
            (text(), DrawZIndex::default()),
        ]);
        let raised_fill_indices = index_draw_commands_for_test(&raised_fill_commands);
        let raised_fill_index = draw_order_indices_for_z_index(
            &raised_fill_commands,
            &raised_fill_indices,
            RAISED_LEVEL,
        )
        .into_iter()
        .next()
        .expect("raised fill command receives a draw order index");
        for text_index in draw_order_indices_for_tier(
            &raised_fill_commands,
            &raised_fill_indices,
            DrawSortTier::Text,
        ) {
            assert!(
                raised_fill_index > text_index,
                "raised fill draw order index must sit above default text indices",
            );
        }

        let lowered_text_commands = commands_from_kinds([
            (text(), LOWERED_LEVEL),
            (rectangle(), DrawZIndex::default()),
            (image(), DrawZIndex::default()),
        ]);
        let lowered_text_indices = index_draw_commands_for_test(&lowered_text_commands);
        let lowered_text_index = draw_order_indices_for_z_index(
            &lowered_text_commands,
            &lowered_text_indices,
            LOWERED_LEVEL,
        )
        .into_iter()
        .next()
        .expect("lowered text command receives a draw order index");
        for fill_index in draw_order_indices_for_tier(
            &lowered_text_commands,
            &lowered_text_indices,
            DrawSortTier::Surface,
        ) {
            assert!(
                lowered_text_index < fill_index,
                "lowered text draw order index must sit below default fill indices",
            );
        }
    }
}
