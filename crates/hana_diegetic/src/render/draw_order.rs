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
//! [`OitDepthOffset`] is a positive panel-global draw-order index span added to
//! `position.z` and packed into 24-bit depth. The first command starts one
//! [`OIT_DEPTH_STEP`] in front of its physical plane: Bevy's OIT resolve pass
//! compares that packed value with opaque depth, so a zero or negative offset
//! can reject a coplanar translucent fragment before composition. Adjacent
//! commands remain one step apart.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use bevy_kana::ToF32;
use bevy_kana::ToU32;

use super::constants::LAYER_DEPTH_BIAS;
use super::constants::OIT_DEPTH_STEP;
use crate::layout::DrawSortTier;
use crate::layout::DrawZIndex;
use crate::layout::RenderCommand;

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

/// Screen `Transparent3d` sort value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ScreenDepthBias(f32);

/// OIT per-fragment offset added to `position.z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OitDepthOffset(f32);

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
    oit_depth_offset: OitDepthOffset,
}

/// Index-aligned draw order for one panel's command stream.
#[derive(Clone, Debug, Default)]
pub(crate) struct DrawOrder {
    depths: Vec<Option<DrawCommandDepth>>,
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
}

impl OitDepthOffset {
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
}

impl DrawOrderIndex {
    pub(super) fn clip_depth_nudge(self) -> ClipDepthNudge { ClipDepthNudge(self.0.to_f32()) }

    fn oit_depth_offset(self) -> OitDepthOffset {
        OitDepthOffset(self.0.saturating_add(1).to_f32() * OIT_DEPTH_STEP)
    }

    fn to_usize(self) -> usize { usize::try_from(self.0).unwrap_or(usize::MAX) }
}

impl DrawCommandDepth {
    fn new(
        draw_order_index: DrawOrderIndex,
        z_index_rank: DrawZIndexRank,
        z_index: DrawZIndex,
    ) -> Self {
        Self {
            draw_order_index,
            z_index_rank,
            z_index,
            clip_depth_nudge: draw_order_index.clip_depth_nudge(),
            oit_depth_offset: draw_order_index.oit_depth_offset(),
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

    /// Returns the layer count consumed by non-OIT shader clip-depth nudging.
    pub(super) const fn clip_depth_nudge(self) -> ClipDepthNudge { self.clip_depth_nudge }

    /// Returns the OIT `position.z` offset for this command.
    pub(super) const fn oit_depth_offset(self) -> OitDepthOffset { self.oit_depth_offset }
}

impl DrawOrder {
    /// Builds index-aligned draw order from a full panel command stream.
    pub(crate) fn from_commands(commands: &[RenderCommand]) -> Self {
        let enumerated = enumerate_draw_commands(commands);
        let depths = enumerated
            .into_iter()
            .map(|command| {
                command.map(|command| {
                    DrawCommandDepth::new(
                        command.draw_order_index,
                        command.z_index_rank,
                        command.z_index,
                    )
                })
            })
            .collect();
        Self { depths }
    }

    /// Returns this command's draw-depth values, or `None` for scissor
    /// commands and out-of-range indices.
    pub(crate) fn depth_for(
        &self,
        command_index: impl Into<CommandIndex>,
    ) -> Option<DrawCommandDepth> {
        let command_index = command_index.into();
        self.depths.get(command_index.get()).copied().flatten()
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
        .into_iter()
        .map(|command| command.map(|command| command.draw_order_index))
        .collect()
}

fn enumerate_draw_commands(commands: &[RenderCommand]) -> Vec<Option<OrderedDrawCommand>> {
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
    for (draw_order_position, (key, index)) in keyed_commands.into_iter().enumerate() {
        enumerated[index] = Some(OrderedDrawCommand {
            draw_order_index: DrawOrderIndex(
                i32::try_from(draw_order_position).unwrap_or(i32::MAX),
            ),
            z_index_rank:     z_index_ranks
                .get(&key.z_index())
                .copied()
                .unwrap_or_default(),
            z_index:          key.z_index(),
        });
    }
    enumerated
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
                bounds: BoundingBox::default(),
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
                draw_command_depth.oit_depth_offset().get().to_bits(),
                (draw_order_index.0.saturating_add(1).to_f32() * OIT_DEPTH_STEP).to_bits(),
                "no-override command {index:?} keeps its positive OIT offset",
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
                low_depth.oit_depth_offset().get() < high_depth.oit_depth_offset().get(),
                "OIT offset must rise from {low} to {high}",
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
    fn oit_offsets_start_one_step_in_front_of_coplanar_opaque_depth() {
        let commands = representative_streams()
            .into_iter()
            .next()
            .expect("representative streams include drawing commands");
        let draw_order = DrawOrder::from_commands(&commands);
        let lowest_offset = drawing_indices(&commands)
            .into_iter()
            .map(|index| draw_depth_at(&draw_order, index).oit_depth_offset().get())
            .reduce(f32::min)
            .expect("representative stream includes drawing commands");

        assert_eq!(lowest_offset.to_bits(), OIT_DEPTH_STEP.to_bits());
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
    fn draw_order_uses_indices_and_positive_oit_offsets() {
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
                    draw_depth.oit_depth_offset().get().to_bits(),
                    (draw_order_index.0.saturating_add(1).to_f32() * OIT_DEPTH_STEP).to_bits(),
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
