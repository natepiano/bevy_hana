use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use bevy::prelude::AlphaMode;
use bevy::prelude::Bundle;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::EntityCommands;
use bevy::prelude::Handle;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use hana_kana::ToF32;

use super::PanelTextLayout;
use super::PanelTextRuns;
use super::TextRunOf;
use super::glyph_cascade;
use super::glyph_cascade::InheritedTextCascades;
use super::glyph_cascade::PanelTextCascadeSources;
use super::layout::PanelTextDrawZIndex;
use super::layout::PanelTextDrawZIndexRank;
use crate::PanelElementId;
use crate::TextAlign;
use crate::cascade;
use crate::cascade::Cascade;
use crate::cascade::CascadeAttribute;
use crate::cascade::CascadeFrom;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::TextAlpha;
use crate::cascade::TextMaterial;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::icon::LucideFont;
use crate::icon::LucideFontReadiness;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::ShadowCasting;
use crate::layout::ShapedGlyph;
use crate::layout::ShapedRichParagraph;
use crate::layout::ShapedTextCache;
use crate::layout::Sidedness;
use crate::layout::TextStyle;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPerfStats;
use crate::panel::PanelLayout;
use crate::panel::PanelOwned;
use crate::render::clip;
use crate::render::draw_order::DrawCommandDepth;
use crate::render::draw_order::DrawOrder;
use crate::render::text_shaping;
use crate::render::text_shaping::TextShapingContext;
use crate::render::world_text::TextContent;

/// A reused panel-text child plus the components reification compares incoming
/// values against; a component is written back only when the incoming value
/// differs. The references borrow the `existing_runs` query for one reification
/// pass.
#[derive(Clone, Copy)]
struct ReusableChild<'a> {
    entity:                 Entity,
    text:                   &'a TextContent,
    style:                  &'a TextStyle,
    layout:                 &'a PanelTextLayout,
    z_index:                &'a PanelTextDrawZIndex,
    z_index_rank:           &'a PanelTextDrawZIndexRank,
    alpha:                  Option<&'a Cascade<TextAlpha>>,
    material:               Option<&'a Cascade<TextMaterial>>,
    lighting:               Option<&'a Cascade<Lighting>>,
    sidedness:              Option<&'a Cascade<Sidedness>>,
    shadow_casting:         Option<&'a Cascade<ShadowCasting>>,
    glyph_shadow_mode:      Option<&'a Cascade<GlyphShadowMode>>,
    hdr_text_coverage_bias: Option<&'a Cascade<HdrTextCoverageBias>>,
    glyph_source:           PanelGlyphSource<&'a RichRunGlyphs>,
}

/// Stable panel-local identity for one reified text-rendering child.
///
/// Text keeps its authored or builder-assigned run id plus wrapped-line
/// ordinal. Icons occupy a separate namespace and use their order among icons,
/// so inserting or removing non-icon layout elements cannot change their
/// identity across a tree rebuild.
#[derive(Component, Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum PanelTextRunIdentity {
    Text {
        id:         PanelElementId,
        line_index: usize,
    },
    RichText {
        id:         PanelElementId,
        line_index: usize,
        run_index:  usize,
    },
    Icon {
        ordinal: usize,
    },
}

/// One text render command resolved to its reification inputs: source element,
/// command depth, stable identity, run `id`, per-run `line_index`, the string,
/// its style, layout bounds, and the effective clip rect. Ordinary text borrows
/// its command; icons and rich paint runs own their resolved styles.
struct TextRunReification<'a> {
    element_idx:  usize,
    draw_depth:   DrawCommandDepth,
    identity:     PanelTextRunIdentity,
    id:           PanelElementId,
    line_index:   usize,
    text:         Cow<'a, str>,
    config:       Cow<'a, TextStyle>,
    bounds:       BoundingBox,
    clip:         BoundingBox,
    glyph_source: PanelGlyphSource<RichRunGlyphs>,
}

/// A pre-shaped paint run, with paragraph position retained for run locators.
#[derive(Component, Clone, Debug, PartialEq)]
pub(super) struct RichRunGlyphs {
    pub(super) glyphs: Arc<[ShapedGlyph]>,
    paragraph_x:       f32,
}

/// Plain children shape their text; rich children use the paragraph's glyphs.
#[derive(Clone, Copy)]
pub(super) enum PanelGlyphSource<T> {
    Plain,
    Rich(T),
}

struct RichShapingInputs<'a> {
    registry: &'a crate::FontRegistry,
    context:  &'a TextShapingContext,
    cache:    &'a ShapedTextCache,
}

/// Renderer resources are optional at the ECS boundary for plain headless panels.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct RichParagraphShaper<'w> {
    registry: Option<Res<'w, crate::FontRegistry>>,
    context:  Option<Res<'w, TextShapingContext>>,
    cache:    Option<Res<'w, ShapedTextCache>>,
}

enum ParagraphShapingAvailability<'a> {
    Ready(RichShapingInputs<'a>),
    FontsUnavailable,
}
impl RichParagraphShaper<'_> {
    fn availability(&self) -> ParagraphShapingAvailability<'_> {
        match (
            self.registry.as_deref(),
            self.context.as_deref(),
            self.cache.as_deref(),
        ) {
            (Some(registry), Some(context), Some(cache)) => {
                ParagraphShapingAvailability::Ready(RichShapingInputs {
                    registry,
                    context,
                    cache,
                })
            },
            _ => ParagraphShapingAvailability::FontsUnavailable,
        }
    }
}

/// Resolves the panel's text render commands into per-child reification inputs,
/// assigning each its run `id` (from the tree, auto fallback when absent) and a
/// per-run `line_index`. Text identity uses that pair; icon identity uses a
/// separate icon-only ordinal, so non-icon sibling edits cannot shift it.
fn collect_text_commands<'a>(
    panel: &DiegeticPanel,
    commands: &'a [RenderCommand],
    draw_order: &DrawOrder,
    clip_rects: &[Option<BoundingBox>],
    viewport: BoundingBox,
    lucide_font: LucideFontReadiness,
    rich_inputs: ParagraphShapingAvailability<'_>,
) -> Vec<TextRunReification<'a>> {
    let mut line_counter: HashMap<usize, usize> = HashMap::new();
    let mut next_icon_ordinal = 0;
    let mut has_rich = false;
    let mut pending: Vec<TextRunReification<'a>> = commands
        .iter()
        .enumerate()
        .filter_map(|(cmd_index, cmd)| match &cmd.kind {
            RenderCommandKind::Text { text, config } => {
                let active_clip = clip::effective_clip(cmd.bounds, clip_rects[cmd_index], viewport)
                    .unwrap_or_else(clip::empty_clip);
                let draw_depth = draw_order.depth_for(cmd_index)?;
                let id = panel
                    .tree()
                    .text_element_id(cmd.element_idx)
                    .cloned()
                    .unwrap_or_else(|| {
                        PanelElementId::auto(u32::try_from(cmd.element_idx).unwrap_or(0))
                    });
                let counter = line_counter.entry(cmd.element_idx).or_insert(0);
                let line_index = *counter;
                *counter += 1;
                Some(TextRunReification {
                    element_idx: cmd.element_idx,
                    draw_depth,
                    identity: PanelTextRunIdentity::Text {
                        id: id.clone(),
                        line_index,
                    },
                    id,
                    line_index,
                    text: Cow::Borrowed(text.as_str()),
                    config: Cow::Borrowed(config),
                    bounds: cmd.bounds,
                    clip: active_clip,
                    glyph_source: PanelGlyphSource::Plain,
                })
            },
            RenderCommandKind::Icon(icon) => {
                let ordinal = next_icon_ordinal;
                next_icon_ordinal += 1;
                let LucideFontReadiness::Registered(font_id) = lucide_font else {
                    return None;
                };
                let active_clip = clip::effective_clip(cmd.bounds, clip_rects[cmd_index], viewport)
                    .unwrap_or_else(clip::empty_clip);
                let draw_depth = draw_order.depth_for(cmd_index)?;
                let id = PanelElementId::auto(u32::try_from(cmd.element_idx).unwrap_or(0));
                // The solved square is already expressed in layout points. Use
                // it as the glyph em so registration cannot revive the icon's
                // pre-conversion authoring size and draw beyond that square.
                let (text, config) = icon.resolved_text_for_box(font_id, cmd.bounds.width);
                Some(TextRunReification {
                    element_idx: cmd.element_idx,
                    draw_depth,
                    identity: PanelTextRunIdentity::Icon { ordinal },
                    id,
                    line_index: 0,
                    text: Cow::Owned(text),
                    config: Cow::Owned(config),
                    bounds: cmd.bounds,
                    clip: active_clip,
                    glyph_source: PanelGlyphSource::Plain,
                })
            },
            RenderCommandKind::RichText { .. } => {
                has_rich = true;
                None
            },
            _ => None,
        })
        .collect();
    // The plain path keeps its single-pass collection and performs no rich work.
    if has_rich && let ParagraphShapingAvailability::Ready(rich_inputs) = rich_inputs {
        for (command_index, command) in commands.iter().enumerate() {
            append_rich_children(
                &mut pending,
                panel,
                command,
                command_index,
                draw_order,
                clip_rects,
                viewport,
                &rich_inputs,
            );
        }
        pending.sort_by_key(|child| child.draw_depth.draw_order_index());
    }
    pending
}

fn collect_existing_text_children<'a>(
    existing_run_entities: &[Entity],
    existing_runs: &'a Query<(
        &TextContent,
        &TextStyle,
        &PanelTextLayout,
        &PanelTextRunIdentity,
        &PanelTextDrawZIndex,
        &PanelTextDrawZIndexRank,
        Option<&Cascade<TextAlpha>>,
        Option<&Cascade<TextMaterial>>,
        Option<&Cascade<Lighting>>,
        Option<&Cascade<Sidedness>>,
        Option<&Cascade<ShadowCasting>>,
        Option<&Cascade<GlyphShadowMode>>,
        Option<&Cascade<HdrTextCoverageBias>>,
        Option<&RichRunGlyphs>,
    )>,
) -> HashMap<PanelTextRunIdentity, ReusableChild<'a>> {
    let mut existing_by_identity = HashMap::new();
    for &entity in existing_run_entities {
        let Ok((
            text,
            style,
            layout,
            identity,
            z_index,
            z_index_rank,
            alpha,
            material,
            lighting,
            sidedness,
            shadow_casting,
            glyph_shadow_mode,
            hdr_text_coverage_bias,
            rich_glyphs,
        )) = existing_runs.get(entity)
        else {
            continue;
        };
        existing_by_identity.insert(
            identity.clone(),
            ReusableChild {
                entity,
                text,
                style,
                layout,
                z_index,
                z_index_rank,
                alpha,
                material,
                lighting,
                sidedness,
                shadow_casting,
                glyph_shadow_mode,
                hdr_text_coverage_bias,
                glyph_source: rich_glyphs.map_or(PanelGlyphSource::Plain, PanelGlyphSource::Rich),
            },
        );
    }
    existing_by_identity
}

/// Centre only the icon drawing. Layout, hit testing and clipping keep the
/// original solved square; text keeps its normal baseline placement.
fn centre_icon_ink(
    runs: &mut [TextRunReification<'_>],
    font: &LucideFont,
    registry: &crate::FontRegistry,
) {
    for run in runs {
        if !matches!(run.identity, PanelTextRunIdentity::Icon { .. }) {
            continue;
        }
        let Some(glyph) = run.text.chars().next() else {
            continue;
        };
        let centre = font.ink_centre(glyph, &run.config, registry);
        run.bounds.x += f32::mul_add(run.bounds.width, 0.5, -centre.x);
        run.bounds.y += f32::mul_add(run.bounds.height, 0.5, -centre.y);
    }
}

/// Reifies [`TextContent`] entities for each changed [`ComputedDiegeticPanel`].
///
/// Writes [`DiegeticPerfStats::reify_ms`] with this pass's wall time.
/// `route_image_batch_records` owns image command routing and does not add to
/// this text-child timing.
pub(super) fn reify_text_entities(
    mut changed_panels: Query<
        (
            Entity,
            &mut DiegeticPanel,
            &ComputedDiegeticPanel,
            Option<&PanelTextRuns>,
        ),
        Changed<ComputedDiegeticPanel>,
    >,
    existing_runs: Query<(
        &TextContent,
        &TextStyle,
        &PanelTextLayout,
        &PanelTextRunIdentity,
        &PanelTextDrawZIndex,
        &PanelTextDrawZIndexRank,
        Option<&Cascade<TextAlpha>>,
        Option<&Cascade<TextMaterial>>,
        Option<&Cascade<Lighting>>,
        Option<&Cascade<Sidedness>>,
        Option<&Cascade<ShadowCasting>>,
        Option<&Cascade<GlyphShadowMode>>,
        Option<&Cascade<HdrTextCoverageBias>>,
        Option<&RichRunGlyphs>,
    )>,
    mut commands: Commands,
    mut perf: ResMut<DiegeticPerfStats>,
    lucide_font: Option<Res<LucideFont>>,
    rich_shaper: RichParagraphShaper,
    cascade_sources: PanelTextCascadeSources,
) {
    let reify_start = Instant::now();
    let lucide_readiness = lucide_font
        .as_deref()
        .map_or(LucideFontReadiness::Loading, LucideFont::readiness);
    for (panel_entity, mut panel, computed, panel_runs) in &mut changed_panels {
        let PanelLayout::Solved(result) = computed.layout() else {
            continue;
        };

        let points_to_world = panel.points_to_world();
        let (anchor_x, anchor_y) = panel.anchor_offsets();

        let clip_rects = clip::compute_clip_rects(&result.commands);
        let viewport = clip::panel_viewport(&panel);
        // Text commands keep their source id and line ordinal. Icon commands
        // use an icon-only ordinal, so unrelated structural edits cannot move
        // them through the text id namespace.
        let mut text_commands = collect_text_commands(
            &panel,
            &result.commands,
            computed.draw_order(),
            &clip_rects,
            viewport,
            lucide_readiness,
            rich_shaper.availability(),
        );
        if let (Some(font), Some(registry)) =
            (lucide_font.as_deref(), rich_shaper.registry.as_deref())
        {
            centre_icon_ink(&mut text_commands, font, registry);
        }

        // Source the panel's existing runs from `PanelTextRuns` (the typed
        // text-run index) rather than scanning every `TextContent` child and
        // filtering by parent. `None` means no run has spawned yet (first pass).
        let existing_run_entities: &[Entity] = panel_runs.map_or(&[][..], |runs| &**runs);
        let mut existing_by_identity =
            collect_existing_text_children(existing_run_entities, &existing_runs);

        let inherited = cascade_sources.for_panel(panel_entity);
        let mut text_index: HashMap<PanelElementId, Entity> = HashMap::new();
        for TextRunReification {
            element_idx,
            draw_depth,
            identity,
            id,
            line_index,
            text,
            config,
            bounds,
            clip,
            glyph_source,
        } in &text_commands
        {
            // A label's own cascade overrides (`TextStyle::with_alpha_mode` /
            // `with_material` / `with_lighting`) are captured before
            // `for_shaping()` clears them, then inserted as `Cascade<A>` on the
            // label. `None` means the label inherits the panel value. Text ink
            // always draws on the front face only.
            let label_cascades = TextLabelCascadeOverrides::from_style(config);
            let style = config.for_shaping(Anchor::TopLeft);
            let z_index = PanelTextDrawZIndex(draw_depth.z_index());
            let z_index_rank = PanelTextDrawZIndexRank(draw_depth.z_index_rank());
            let panel_text_child = PanelTextLayout {
                id: id.clone(),
                line_index: *line_index,
                element_idx: *element_idx,
                draw_ordinal: draw_depth.draw_order_index(),
                clip_depth_nudge: draw_depth.clip_depth_nudge().get(),
                oit_paint_order: draw_depth.oit_paint_order().get(),
                bounds: *bounds,
                scale_x: points_to_world,
                scale_y: points_to_world,
                anchor_x,
                anchor_y,
                clip_rect: Some(*clip),
            };

            let entity = if let Some(reusable) = existing_by_identity.remove(identity) {
                update_reused_panel_text_child(UpdateReusedChild {
                    commands: &mut commands,
                    reusable,
                    text,
                    style,
                    layout: panel_text_child,
                    z_index,
                    z_index_rank,
                    label_cascades,
                    glyph_source,
                });
                reusable.entity
            } else {
                spawn_panel_text_child(SpawnPanelTextChild {
                    inherited: &inherited,
                    commands: &mut commands,
                    panel_entity,
                    identity: identity.clone(),
                    text,
                    style,
                    layout: panel_text_child,
                    z_index,
                    z_index_rank,
                    label_cascades,
                    glyph_source,
                })
            };

            // Address a run by its first line: `text_child(id)` resolves this
            // entity. Auto-id runs land here too but are unreachable — no caller
            // can build their `PanelElementId::Auto`.
            if *line_index == 0 {
                text_index.entry(id.clone()).or_insert(entity);
            }
        }

        for unused in existing_by_identity.into_values() {
            commands.entity(unused.entity).despawn();
        }

        // Rebuilt from scratch each pass. Write it without tripping
        // `Changed<DiegeticPanel>`, or this `&mut DiegeticPanel` write would
        // re-dirty the panel and loop layout → reification every frame.
        panel.bypass_change_detection().text_index = text_index;
    }
    perf.reify_ms = reify_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
}

/// Inputs to [`spawn_panel_text_child`]. Grouped into a struct because reification
/// threads text, style, layout, and captured cascade overrides through to a
/// freshly spawned child.
struct SpawnPanelTextChild<'a, 'w, 's> {
    inherited:      &'a InheritedTextCascades,
    commands:       &'a mut Commands<'w, 's>,
    panel_entity:   Entity,
    identity:       PanelTextRunIdentity,
    text:           &'a str,
    style:          TextStyle,
    layout:         PanelTextLayout,
    z_index:        PanelTextDrawZIndex,
    z_index_rank:   PanelTextDrawZIndexRank,
    label_cascades: TextLabelCascadeOverrides,
    glyph_source:   &'a PanelGlyphSource<RichRunGlyphs>,
}

/// Spawns a fully seeded panel-text child in one bundle. Authored overrides
/// resolve against the panel cache; inheriting attributes retain their cascade
/// state for later propagation.
fn spawn_panel_text_child(request: SpawnPanelTextChild<'_, '_, '_>) -> Entity {
    let SpawnPanelTextChild {
        inherited,
        commands,
        panel_entity,
        identity,
        text,
        style,
        layout,
        z_index,
        z_index_rank,
        label_cascades,
        glyph_source,
    } = request;
    let mut spawned = Entity::PLACEHOLDER;
    commands.entity(panel_entity).with_children(|children| {
        // `TextRunOf` is inserted only here, on a newly spawned run; the reuse
        // branch must never re-insert it (a reused run already carries it, and
        // re-inserting would fire the relationship hook and mutate
        // `PanelTextRuns` on a no-op). `with_children` already adds `ChildOf`.
        let bundle = (
            TextContent::new(text.to_owned()),
            style,
            layout,
            identity,
            z_index,
            z_index_rank,
            TextRunOf(panel_entity),
            PanelOwned::from(panel_entity),
            CascadeFrom::new(panel_entity),
            label_cascades.spawn_bundle(inherited),
        );
        let child = match glyph_source {
            PanelGlyphSource::Plain => children.spawn(bundle),
            PanelGlyphSource::Rich(glyphs) => children.spawn((bundle, glyphs.clone())),
        };
        spawned = child.id();
    });
    spawned
}

/// Inputs to [`update_reused_panel_text_child`]. Grouped into a struct because
/// reification threads text, style, layout, and captured cascade overrides through
/// to a reused child.
struct UpdateReusedChild<'a, 'w, 's> {
    commands:       &'a mut Commands<'w, 's>,
    reusable:       ReusableChild<'a>,
    text:           &'a str,
    style:          TextStyle,
    layout:         PanelTextLayout,
    z_index:        PanelTextDrawZIndex,
    z_index_rank:   PanelTextDrawZIndexRank,
    label_cascades: TextLabelCascadeOverrides,
    glyph_source:   &'a PanelGlyphSource<RichRunGlyphs>,
}

/// Cascade overrides authored by one `TextStyle` before shaping clears
/// render-only fields from the glyph-layout style.
struct TextLabelCascadeOverrides {
    alpha:                  Cascade<AlphaMode>,
    material:               Cascade<Handle<StandardMaterial>>,
    lighting:               Cascade<Lighting>,
    sidedness:              Cascade<Sidedness>,
    shadow_casting:         Cascade<ShadowCasting>,
    glyph_shadow_mode:      Cascade<GlyphShadowMode>,
    hdr_text_coverage_bias: Cascade<f32>,
}

impl TextLabelCascadeOverrides {
    fn spawn_bundle(self, inherited: &InheritedTextCascades) -> impl Bundle {
        (
            glyph_cascade::seeded_text_cascade(self.alpha.map(TextAlpha), &inherited.alpha),
            glyph_cascade::seeded_text_cascade(
                self.material.map(TextMaterial),
                &inherited.material,
            ),
            glyph_cascade::seeded_text_cascade(self.lighting, &inherited.lighting),
            glyph_cascade::seeded_text_cascade(self.sidedness, &inherited.sidedness),
            glyph_cascade::seeded_text_cascade(self.shadow_casting, &inherited.shadow_casting),
            glyph_cascade::seeded_text_cascade(
                self.glyph_shadow_mode,
                &inherited.glyph_shadow_mode,
            ),
            glyph_cascade::seeded_text_cascade(Cascade::Inherit, &inherited.anti_alias),
            glyph_cascade::seeded_text_cascade(
                self.hdr_text_coverage_bias.map(HdrTextCoverageBias),
                &inherited.hdr_text_coverage_bias,
            ),
        )
    }

    fn from_style(style: &TextStyle) -> Self {
        Self {
            alpha:                  style.alpha_mode(),
            material:               style.material().cloned(),
            lighting:               style.lighting(),
            sidedness:              TextStyle::sidedness(),
            shadow_casting:         style.shadow_casting(),
            glyph_shadow_mode:      style.shadow_mode(),
            hdr_text_coverage_bias: style.hdr_text_coverage_bias(),
        }
    }
}

/// Writes each component of a reused panel-text child only when it differs, so
/// an unchanged run stays un-`Changed`.
///
/// `shape_panel_text_children` classifies `Changed<TextStyle>` with
/// `TextStyle::gating_eq`: geometry fields rebuild glyphs, while render-only
/// fields refresh `PreparedPanelText` and material-table rows. Cascade
/// overrides (alpha, material, lighting, sidedness, HDR text coverage bias) are
/// gated on their own because writing one unconditionally would re-fire
/// `Changed<Resolved<A>>` on every run and defeat the per-run short-circuit
/// downstream.
fn update_reused_panel_text_child(request: UpdateReusedChild<'_, '_, '_>) {
    let UpdateReusedChild {
        commands,
        reusable,
        text,
        style,
        layout,
        z_index,
        z_index_rank,
        label_cascades,
        glyph_source,
    } = request;
    let mut child = commands.entity(reusable.entity);
    if reusable.text.text() != text {
        child.insert(TextContent::new(text.to_owned()));
    }
    if reusable.style != &style {
        child.insert(style);
    }
    if !reusable.layout.gating_eq(&layout) {
        child.insert(layout);
    }
    if *reusable.z_index != z_index {
        child.insert(z_index);
    }
    if *reusable.z_index_rank != z_index_rank {
        child.insert(z_index_rank);
    }
    match (glyph_source, reusable.glyph_source) {
        (PanelGlyphSource::Rich(glyphs), PanelGlyphSource::Rich(previous))
            if glyphs == previous => {},
        (PanelGlyphSource::Rich(glyphs), _) => {
            child.insert(glyphs.clone());
        },
        (PanelGlyphSource::Plain, PanelGlyphSource::Rich(_)) => {
            child.remove::<RichRunGlyphs>();
        },
        (PanelGlyphSource::Plain, PanelGlyphSource::Plain) => {},
    }
    sync_label_cascade_overrides(&mut child, label_cascades, reusable);
}

fn sync_label_cascade_overrides(
    child: &mut EntityCommands<'_>,
    label_cascades: TextLabelCascadeOverrides,
    reusable: ReusableChild<'_>,
) {
    sync_cascade_override(child, label_cascades.alpha.map(TextAlpha), reusable.alpha);
    sync_cascade_override(
        child,
        label_cascades.material.map(TextMaterial),
        reusable.material,
    );
    sync_cascade_override(child, label_cascades.lighting, reusable.lighting);
    sync_cascade_override(child, label_cascades.sidedness, reusable.sidedness);
    sync_cascade_override(
        child,
        label_cascades.shadow_casting,
        reusable.shadow_casting,
    );
    sync_cascade_override(
        child,
        label_cascades.glyph_shadow_mode,
        reusable.glyph_shadow_mode,
    );
    sync_cascade_override(
        child,
        label_cascades
            .hdr_text_coverage_bias
            .map(HdrTextCoverageBias),
        reusable.hdr_text_coverage_bias,
    );
}

fn sync_cascade_override<A>(
    child: &mut EntityCommands<'_>,
    incoming: Cascade<A>,
    current: Option<&Cascade<A>>,
) where
    A: CascadeAttribute,
{
    match incoming {
        Cascade::Override(value) => {
            if current.and_then(Cascade::as_override) != Some(&value) {
                cascade::apply_cascade_override(child, value);
            }
        },
        Cascade::Inherit => {
            if !current.is_some_and(Cascade::is_inherit) {
                cascade::remove_cascade_override::<A>(child);
            }
        },
    }
}

/// Expands paragraph geometry into paint runs without shaping substrings.
fn append_rich_children<'a>(
    pending: &mut Vec<TextRunReification<'a>>,
    panel: &DiegeticPanel,
    command: &'a RenderCommand,
    command_index: usize,
    order: &DrawOrder,
    clips: &[Option<BoundingBox>],
    viewport: BoundingBox,
    inputs: &RichShapingInputs<'_>,
) {
    let RenderCommandKind::RichText {
        paragraph,
        line_index,
    } = &command.kind
    else {
        return;
    };
    let Some(depth) = order.depth_for(command_index) else {
        return;
    };
    let id = panel
        .tree()
        .text_element_id(command.element_idx)
        .cloned()
        .unwrap_or_else(|| PanelElementId::auto(u32::try_from(command.element_idx).unwrap_or(0)));
    let shaped = text_shaping::shape_rich_paragraph_cached(
        paragraph,
        1.0,
        inputs.registry,
        inputs.context,
        inputs.cache,
    );
    let shaped_line = *line_index;
    if shaped.line_metrics.get(shaped_line).is_none() {
        return;
    }
    let Some(&line_width) = shaped.line_widths.get(shaped_line) else {
        return;
    };
    let line_offset = match paragraph.base.text_align() {
        TextAlign::Left => 0.0,
        TextAlign::Center => (command.bounds.width - line_width) * 0.5,
        TextAlign::Right => command.bounds.width - line_width,
    };
    let runs = paragraph_paint_runs(paragraph, &shaped, shaped_line);
    for (run_index, run) in runs.into_iter().enumerate() {
        let left = run
            .glyphs
            .iter()
            .map(|glyph| glyph.x)
            .fold(f32::INFINITY, f32::min);
        let right = run
            .glyphs
            .iter()
            .map(|glyph| glyph.x + glyph.advance)
            .fold(f32::NEG_INFINITY, f32::max);
        if !left.is_finite() {
            continue;
        }
        let resolved = &paragraph.runs[run.paint_index];
        let config = paragraph
            .base
            .clone()
            .with_color(resolved.color)
            .with_word_spacing(resolved.measure.word_spacing);
        let bounds = BoundingBox {
            x:      command.bounds.x + line_offset + left,
            y:      command.bounds.y,
            width:  (right - left).max(0.0),
            height: command.bounds.height,
        };
        let glyphs = run
            .glyphs
            .into_iter()
            .map(|mut glyph| {
                glyph.x -= left;
                // A selection box can start above the line origin when leading
                // is negative. Keep the same typographic origin as plain text.
                glyph.baseline = shaped_line
                    .to_f32()
                    .mul_add(-shaped.dimensions.line_height, glyph.baseline);
                glyph
            })
            .collect::<Vec<_>>();
        pending.push(TextRunReification {
            element_idx: command.element_idx,
            draw_depth: depth,
            identity: PanelTextRunIdentity::RichText {
                id: id.clone(),
                line_index: *line_index,
                run_index,
            },
            id: id.clone(),
            line_index: *line_index,
            text: Cow::Borrowed(&paragraph.text[run.range]),
            config: Cow::Owned(config),
            bounds,
            clip: clip::effective_clip(bounds, clips[command_index], viewport)
                .unwrap_or_else(clip::empty_clip),
            glyph_source: PanelGlyphSource::Rich(RichRunGlyphs {
                glyphs:      glyphs.into(),
                paragraph_x: left,
            }),
        });
    }
}

struct RichPaintRun {
    range:       Range<usize>,
    paint_index: usize,
    glyphs:      Vec<ShapedGlyph>,
}

fn paragraph_paint_runs(
    paragraph: &crate::RichParagraph,
    shaped: &ShapedRichParagraph,
    line_index: usize,
) -> Vec<RichPaintRun> {
    let mut runs: Vec<RichPaintRun> = Vec::new();
    for cluster in shaped
        .clusters
        .iter()
        .filter(|cluster| cluster.line_index == line_index)
    {
        let Some(paint_index) = paragraph
            .runs
            .iter()
            .position(|run| run.range.contains(&cluster.range.start))
        else {
            continue;
        };
        if let Some(previous) = runs.last_mut()
            && paragraph.runs[previous.paint_index].color == paragraph.runs[paint_index].color
        {
            previous.range.start = previous.range.start.min(cluster.range.start);
            previous.range.end = previous.range.end.max(cluster.range.end);
            previous.glyphs.extend_from_slice(&cluster.glyphs);
        } else {
            runs.push(RichPaintRun {
                range: cluster.range.clone(),
                paint_index,
                glyphs: cluster.glyphs.clone(),
            });
        }
    }
    runs
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use bevy::asset::AssetApp;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::observer::Observer;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::gizmos::GizmoAsset;
    use bevy::prelude::Add;
    use bevy::prelude::AlphaMode;
    use bevy::prelude::App;
    use bevy::prelude::ApplyDeferred;
    use bevy::prelude::Changed;
    use bevy::prelude::Color;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::Image;
    use bevy::prelude::Insert;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::Mesh;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::Ref;
    use bevy::prelude::RelationshipTarget;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::With;
    use bevy::render::storage::ShaderBuffer;
    use bevy::shader::Shader;
    use hana_kana::ToF32;

    use super::GlyphShadowMode;
    use super::HdrTextCoverageBias;
    use super::LucideFontReadiness;
    use super::PanelTextRunIdentity;
    use super::RichRunGlyphs;
    use super::ShadowCasting;
    use super::Sidedness;
    use super::TextMaterial;
    use super::collect_text_commands;
    use super::reify_text_entities;
    use crate::DiegeticUiPlugin;
    use crate::Icon;
    use crate::LucideIcon;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelText;
    use crate::cascade;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeFrom;
    use crate::cascade::Resolved;
    use crate::cascade::TextAlpha;
    use crate::constants::MONOSPACE_WIDTH_RATIO;
    use crate::layout::BoundingBox;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::Lighting;
    use crate::layout::RenderCommandKind;
    use crate::layout::RichText;
    use crate::layout::Text;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanel;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render::AntiAlias;
    use crate::render::TooltipDepthPriority;
    use crate::render::clip;
    use crate::render::constants::LAYER_DEPTH_BIAS;
    use crate::render::panel_text::PanelTextCascadeBridgePlugin;
    use crate::render::panel_text::PanelTextLayout;
    use crate::render::panel_text::PanelTextRuns;
    use crate::render::world_text::TextContent;
    use crate::text::DiegeticTextMeasurer;
    use crate::text::FontId;

    /// Records which reused children had each gated component rewritten in the
    /// most recent reification pass, so a test can assert an unchanged run stays
    /// un-`Changed`.
    #[derive(Resource, Default)]
    struct ChangedProbe {
        text:   Vec<Entity>,
        style:  Vec<Entity>,
        layout: Vec<Entity>,
        alpha:  Vec<Entity>,
    }

    /// Captures, each frame, the labels whose gated components changed since the
    /// probe last ran. Runs after reification + its command flush.
    fn probe_changed(
        mut probe: ResMut<ChangedProbe>,
        labels: Query<(
            Entity,
            Ref<TextContent>,
            Ref<TextStyle>,
            Ref<PanelTextLayout>,
            Option<Ref<Cascade<TextAlpha>>>,
        )>,
    ) {
        probe.text.clear();
        probe.style.clear();
        probe.layout.clear();
        probe.alpha.clear();
        for (entity, text, style, layout, alpha) in &labels {
            if text.is_changed() {
                probe.text.push(entity);
            }
            if style.is_changed() {
                probe.style.push(entity);
            }
            if layout.is_changed() {
                probe.layout.push(entity);
            }
            if alpha.is_some_and(|node_override| node_override.is_changed()) {
                probe.alpha.push(entity);
            }
        }
    }

    fn monospace_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|text: &str, measure: &TextMeasure| {
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let width = text
                    .lines()
                    .map(|line| line.chars().count().to_f32() * char_width)
                    .fold(0.0_f32, f32::max);
                let line_count = text.lines().count().max(1).to_f32();
                TextDimensions {
                    width,
                    height: measure.size * line_count,
                    line_height: measure.size,
                }
            }),
        }
    }

    /// App with headless layout plus gated reification and a change probe
    /// chained after the command flush.
    fn reify_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(monospace_measurer());
        app.add_plugins(HeadlessLayoutPlugin);
        app.init_resource::<ChangedProbe>();
        app.add_systems(
            PostUpdate,
            (reify_text_entities, ApplyDeferred, probe_changed).chain(),
        );
        app
    }

    fn two_text_tree(first: Color, second: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", TextStyle::new(10.0).with_color(first)));
        builder.text(("Beta", TextStyle::new(10.0).with_color(second)));
        builder.build()
    }

    fn single_alpha_text_tree(color: Color, alpha: AlphaMode) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((
            "Glow",
            TextStyle::new(10.0)
                .with_color(color)
                .with_alpha_mode(alpha),
        ));
        builder.build()
    }

    fn explicit_cascade_text_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((
            "Glow",
            TextStyle::new(10.0)
                .with_alpha_mode(AlphaMode::Add)
                .with_lighting(Lighting::Unlit),
        ));
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        app.world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(tree)
                    .build()
                    .expect("panel should build"),
            )
            .id()
    }

    fn labels_by_text(app: &mut App) -> HashMap<String, Entity> {
        let mut state = app
            .world_mut()
            .query_filtered::<(Entity, &TextContent), With<TextContent>>();
        state
            .iter(app.world())
            .map(|(entity, text)| (text.text().to_owned(), entity))
            .collect()
    }

    #[derive(Default, Resource)]
    struct LabelPublicationProbe {
        observed:         usize,
        missing_cascades: Vec<Entity>,
    }

    fn probe_label_publication(
        added: On<Add<TextContent>>,
        cascades: Query<(
            &Cascade<TextAlpha>,
            &Cascade<TextMaterial>,
            &Cascade<Lighting>,
            &Cascade<Sidedness>,
            &Cascade<ShadowCasting>,
            &Cascade<GlyphShadowMode>,
            &Cascade<HdrTextCoverageBias>,
        )>,
        mut probe: ResMut<LabelPublicationProbe>,
    ) {
        probe.observed += 1;
        if cascades.get(added.entity).is_err() {
            probe.missing_cascades.push(added.entity);
        }
    }

    #[test]
    fn labels_publish_all_cascades_with_their_first_text_content() {
        let mut app = reify_app();
        app.init_resource::<LabelPublicationProbe>()
            .add_observer(probe_label_publication);
        spawn_panel(&mut app, two_text_tree(Color::WHITE, Color::BLACK));
        spawn_panel(&mut app, explicit_cascade_text_tree());
        app.update();
        let probe = app.world().resource::<LabelPublicationProbe>();
        assert_eq!(probe.observed, 3);
        assert!(
            probe.missing_cascades.is_empty(),
            "every label must publish its inherited or authored cascades once at spawn"
        );
    }

    #[derive(Default, Resource)]
    struct CascadeInsertionProbe(HashMap<Entity, usize>);

    #[derive(Default, Resource)]
    struct LabelObserverRuns(HashMap<String, usize>);

    // Mirror the production descriptors, rather than a hand-written count of
    // attributes: each probe receives exactly the events its observer receives.
    fn count_label_observer_dispatches(app: &mut App) {
        let world = app.world_mut();
        let add = bevy::ecs::lifecycle::ADD;
        let insert = bevy::ecs::lifecycle::INSERT;
        let descriptors: Vec<_> = world
            .query::<&Observer>()
            .iter(world)
            .filter(|observer| {
                let name = observer.system_name();
                [
                    "hana_kana::cascade::",
                    "hana_diegetic::panel::lifecycle::",
                    "hana_diegetic::render::panel_text::",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            })
            .map(|observer| {
                let descriptor = observer.descriptor();
                (
                    observer.system_name().to_string(),
                    descriptor.components().to_vec(),
                    descriptor.event_keys().contains(&add),
                    descriptor.event_keys().contains(&insert),
                )
            })
            .collect();
        assert!(
            descriptors
                .iter()
                .any(|(name, ..)| name.contains("teardown_panel_role")),
            "the production lifecycle observers and their names must be available"
        );
        app.init_resource::<LabelObserverRuns>();
        for (name, components, observes_add, observes_insert) in descriptors {
            if observes_add {
                let name = name.clone();
                app.world_mut().spawn(
                    Observer::new(
                        move |added: On<Add<()>>,
                              labels: Query<(), With<PanelTextLayout>>,
                              mut runs: ResMut<LabelObserverRuns>| {
                            if labels.contains(added.entity) {
                                *runs.0.entry(name.clone()).or_default() += 1;
                            }
                        },
                    )
                    .with_components(components.clone()),
                );
            }
            if observes_insert {
                app.world_mut().spawn(
                    Observer::new(
                        move |inserted: On<Insert<()>>,
                              labels: Query<(), With<PanelTextLayout>>,
                              mut runs: ResMut<LabelObserverRuns>| {
                            if labels.contains(inserted.entity) {
                                *runs.0.entry(name.clone()).or_default() += 1;
                            }
                        },
                    )
                    .with_components(components),
                );
            }
        }
    }

    #[test]
    fn complete_labels_dispatch_no_cascade_or_ownership_observers() {
        let mut app = production_reify_app();
        count_label_observer_dispatches(&mut app);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Plain", TextStyle::new(10.0)));
        builder.rich_text(RichText::new(TextStyle::new(10.0)), |rich| {
            rich.span("Rich");
        });
        spawn_panel(&mut app, builder.build());
        app.update();
        assert_eq!(labels_by_text(&mut app).len(), 2);
        let runs = &app.world().resource::<LabelObserverRuns>().0;
        assert_eq!(
            runs.values().sum::<usize>(),
            0,
            "observer dispatches: {runs:#?}"
        );
    }

    fn production_reify_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Shader>()
            .init_asset::<Mesh>()
            .init_asset::<GizmoAsset>()
            .init_asset::<Image>()
            .init_asset::<ShaderBuffer>()
            .add_plugins(DiegeticUiPlugin);
        app
    }

    #[test]
    fn labels_spawn_with_their_complete_cascade_archetype() {
        let mut app = production_reify_app();
        app.init_resource::<CascadeInsertionProbe>().add_observer(
            |inserted: On<Insert<CascadeFrom>>,
             labels: Query<(), With<PanelTextLayout>>,
             mut probe: ResMut<CascadeInsertionProbe>| {
                if labels.contains(inserted.entity) {
                    *probe.0.entry(inserted.entity).or_default() += 1;
                }
            },
        );
        let incomplete = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(AtomicUsize::new(0));
        let misses = Arc::clone(&incomplete);
        let seen = Arc::clone(&observed);
        app.add_observer(
            move |added: On<Add<TextContent>>,
                  complete: Query<
                (),
                (
                    With<CascadeFrom>,
                    With<Cascade<AntiAlias>>,
                    With<Resolved<TextAlpha>>,
                    With<Resolved<TextMaterial>>,
                    With<Resolved<Lighting>>,
                    With<Resolved<Sidedness>>,
                    With<Resolved<ShadowCasting>>,
                    With<Resolved<GlyphShadowMode>>,
                    With<Resolved<AntiAlias>>,
                    With<Resolved<HdrTextCoverageBias>>,
                ),
            >| {
                seen.fetch_add(1, Ordering::Relaxed);
                if !complete.contains(added.entity) {
                    misses.fetch_add(1, Ordering::Relaxed);
                }
            },
        );
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Plain", TextStyle::new(10.0)));
        builder.rich_text(RichText::new(TextStyle::new(10.0)), |rich| {
            rich.span("Rich");
        });
        spawn_panel(&mut app, builder.build());
        app.update();
        assert_eq!(observed.load(Ordering::Relaxed), 2);
        assert_eq!(
            incomplete.load(Ordering::Relaxed),
            0,
            "no label should need a later cascade archetype transition"
        );
        let labels = labels_by_text(&mut app);
        let probe = app.world().resource::<CascadeInsertionProbe>();
        assert_eq!(probe.0.len(), 2);
        for text in ["Plain", "Rich"] {
            let label = labels[text];
            assert_eq!(
                app.world().get::<RichRunGlyphs>(label).is_some(),
                text == "Rich"
            );
        }
        assert_eq!(
            [probe.0[&labels["Plain"]], probe.0[&labels["Rich"]]],
            [1, 1],
            "plain and rich labels must each publish CascadeFrom once, without deferred bridge writes"
        );
    }

    #[test]
    fn collecting_mixed_runs_borrows_text_and_resolves_only_ready_icons() {
        let mut app = reify_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new("Label", TextStyle::new(10.0)).id(PanelElementId::named("label")));
        builder.icon(Icon::new(LucideIcon::Camera).size(10.0));
        let panel_entity = spawn_panel(&mut app, builder.build());
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(panel_entity)
            .expect("panel exists");
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel_entity)
            .expect("panel is computed");
        let result = computed.layout().solved().expect("layout is solved");
        let clips = clip::compute_clip_rects(&result.commands);
        let collect = |readiness| {
            collect_text_commands(
                panel,
                &result.commands,
                computed.draw_order(),
                &clips,
                clip::panel_viewport(panel),
                readiness,
                super::ParagraphShapingAvailability::FontsUnavailable,
            )
        };
        let waiting = collect(LucideFontReadiness::Loading);
        assert_eq!(waiting.len(), 1, "text does not wait for the icon font");
        let ready = collect(LucideFontReadiness::Registered(FontId(11)));
        assert_eq!(ready.len(), 2, "registration adds exactly one icon run");
        let text_run = &ready[0];
        let (text, config) = result
            .commands
            .iter()
            .find_map(|command| match &command.kind {
                RenderCommandKind::Text { text, config }
                    if command.element_idx == text_run.element_idx =>
                {
                    Some((text, config))
                },
                _ => None,
            })
            .expect("the label has a source text command");
        assert!(
            matches!(text_run.text, Cow::Borrowed(_)),
            "ordinary text borrows its string"
        );
        assert!(
            matches!(text_run.config, Cow::Borrowed(_)),
            "ordinary text borrows its style"
        );
        assert!(std::ptr::eq(text_run.text.as_ref(), text.as_str()));
        assert!(std::ptr::eq(text_run.config.as_ref(), config));
        assert_eq!(text_run.identity, waiting[0].identity);
        assert_eq!(
            text_run.identity,
            PanelTextRunIdentity::Text {
                id:         PanelElementId::named("label"),
                line_index: 0,
            }
        );
        let icon_run = &ready[1];
        assert_eq!(icon_run.identity, PanelTextRunIdentity::Icon { ordinal: 0 });
        assert!(matches!(icon_run.text, Cow::Owned(_)));
        assert!(matches!(icon_run.config, Cow::Owned(_)));
        assert_eq!(
            icon_run.text.chars().count(),
            1,
            "an icon resolves to one glyph"
        );
        assert_eq!(icon_run.config.font_id(), 11);
        assert_eq!(icon_run.config.size(), icon_run.bounds.width);
    }

    #[test]
    fn world_tooltip_text_layout_keeps_panel_local_depth() {
        let mut app = reify_app();
        let one_text = |label: &'static str| {
            let mut builder = LayoutBuilder::new(100.0, 50.0);
            builder.text((label, TextStyle::new(10.0)));
            builder.build()
        };
        spawn_panel(&mut app, one_text("Host"));
        let tooltip = spawn_panel(&mut app, one_text("Tooltip"));
        app.world_mut()
            .entity_mut(tooltip)
            .insert(TooltipDepthPriority);
        app.update();

        let labels = labels_by_text(&mut app);
        let host = app.world().get::<PanelTextLayout>(labels["Host"]);
        let tooltip = app.world().get::<PanelTextLayout>(labels["Tooltip"]);
        assert!(host.is_some() && tooltip.is_some());
        let (Some(host), Some(tooltip)) = (host, tooltip) else {
            return;
        };
        assert_eq!(
            tooltip.oit_paint_order.to_bits(),
            host.oit_paint_order.to_bits()
        );
        assert_eq!(
            tooltip.clip_depth_nudge.to_bits(),
            host.clip_depth_nudge.to_bits()
        );
    }

    #[test]
    fn unchanged_run_is_not_rewritten_across_a_visual_only_rebuild() {
        let mut app = reify_app();
        let panel = spawn_panel(&mut app, two_text_tree(Color::WHITE, Color::WHITE));
        app.update();

        let labels = labels_by_text(&mut app);
        let recolored = labels["Alpha"];
        let untouched = labels["Beta"];

        // Visual-only: only "Alpha" changes color. "Beta" stays byte-identical,
        // so its text/style/layout must not be rewritten.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, two_text_tree(Color::BLACK, Color::WHITE))
                .is_ok()
        );
        app.update();

        let probe = app.world().resource::<ChangedProbe>();
        // The recolored run's style is rewritten; its text and layout are not.
        assert!(probe.style.contains(&recolored));
        assert!(!probe.text.contains(&recolored));
        assert!(!probe.layout.contains(&recolored));
        // The byte-identical run is left entirely alone.
        assert!(!probe.text.contains(&untouched));
        assert!(!probe.style.contains(&untouched));
        assert!(!probe.layout.contains(&untouched));
    }

    #[test]
    fn alpha_unchanged_reused_child_keeps_its_override() {
        let mut app = reify_app();
        let panel = spawn_panel(
            &mut app,
            single_alpha_text_tree(Color::WHITE, AlphaMode::Add),
        );
        app.update();

        let label = labels_by_text(&mut app)["Glow"];
        assert!(
            app.world().get::<Cascade<TextAlpha>>(label).is_some(),
            "explicit-alpha label should carry Cascade<TextAlpha> after the first pass"
        );

        // Recolor only; alpha stays AlphaMode::Add.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, single_alpha_text_tree(Color::BLACK, AlphaMode::Add))
                .is_ok()
        );
        app.update();

        let probe = app.world().resource::<ChangedProbe>();
        // The color change rewrites the style, proving reification ran on this run.
        assert!(probe.style.contains(&label));
        // The unchanged alpha override is not re-touched.
        assert!(!probe.alpha.contains(&label));
    }

    #[test]
    fn first_spawn_preserves_explicit_label_cascades() {
        let mut app = reify_app();
        app.add_plugins(cascade::cascade_plugin::<TextAlpha>())
            .add_plugins(cascade::cascade_plugin::<Lighting>())
            .add_plugins(PanelTextCascadeBridgePlugin);
        spawn_panel(&mut app, explicit_cascade_text_tree());

        app.update();

        let label = labels_by_text(&mut app)["Glow"];
        assert_eq!(
            app.world().get::<Cascade<TextAlpha>>(label),
            Some(&Cascade::Override(TextAlpha(AlphaMode::Add)))
        );
        assert_eq!(
            app.world().get::<Cascade<Lighting>>(label),
            Some(&Cascade::Override(Lighting::Unlit))
        );
        assert_eq!(
            app.world()
                .get::<Resolved<TextAlpha>>(label)
                .map(|value| value.0),
            Some(TextAlpha(AlphaMode::Add))
        );
        assert_eq!(
            app.world()
                .get::<Resolved<Lighting>>(label)
                .map(|value| value.0),
            Some(Lighting::Unlit)
        );
    }

    #[test]
    fn reification_keys_by_run_id_and_line_index() {
        // One wrapped run (shared id) across three lines: the `(id, line_index)`
        // key distinguishes the three children, while keying by id alone would
        // collapse them — the property reification relies on to reuse each line.
        let id = PanelElementId::named("run");
        let existing: Vec<(Entity, PanelTextLayout)> = (0..3)
            .map(|line| {
                let panel_text_child = PanelTextLayout {
                    id:               id.clone(),
                    line_index:       line,
                    element_idx:      7,
                    draw_ordinal:     line,
                    clip_depth_nudge: line.to_f32() * LAYER_DEPTH_BIAS,
                    oit_paint_order:  0.0,
                    bounds:           BoundingBox {
                        x:      0.0,
                        y:      line.to_f32() * 10.0,
                        width:  100.0,
                        height: 10.0,
                    },
                    scale_x:          1.0,
                    scale_y:          1.0,
                    anchor_x:         0.0,
                    anchor_y:         0.0,
                    clip_rect:        None,
                };
                (
                    Entity::from_raw_u32(line.try_into().expect("small")).expect("valid"),
                    panel_text_child,
                )
            })
            .collect();

        let mut by_key: HashMap<(PanelElementId, usize), Entity> = HashMap::new();
        for (entity, layout) in &existing {
            by_key.insert((layout.id.clone(), layout.line_index), *entity);
        }
        assert_eq!(by_key.len(), 3);

        let mut by_id_only: HashMap<PanelElementId, Entity> = HashMap::new();
        for (entity, layout) in &existing {
            by_id_only.insert(layout.id.clone(), *entity);
        }
        assert_eq!(by_id_only.len(), 1);
    }

    fn one_text_tree(text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, TextStyle::new(10.0)));
        builder.build()
    }

    /// App with headless layout and gated reification (`PostUpdate`), so a test
    /// can edit the authoritative tree and watch the layout pipeline relayout and
    /// reification re-derive the run child end to end.
    fn text_source_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(monospace_measurer());
        app.add_plugins(HeadlessLayoutPlugin);
        app.add_systems(PostUpdate, reify_text_entities);
        app
    }

    fn first_text_child(app: &mut App) -> Entity {
        let mut state = app
            .world_mut()
            .query_filtered::<Entity, With<TextContent>>();
        let children: Vec<Entity> = state.iter(app.world()).collect();
        assert_eq!(children.len(), 1, "expected exactly one text child");
        children[0]
    }

    fn content_width(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel should exist")
            .content_bounds()
            .expect("content bounds should exist")
            .width
    }

    #[test]
    fn editing_run_text_in_the_tree_relayouts_and_re_derives_the_child() {
        let mut app = text_source_app();
        let panel = spawn_panel(&mut app, one_text_tree("Hi"));
        app.update();
        app.update();

        let child = first_text_child(&mut app);
        let element_idx = app
            .world()
            .get::<PanelTextLayout>(child)
            .expect("child should carry its layout")
            .element_idx;
        let before = content_width(&app, panel);

        // Tree-authoritative edit — exactly what `PanelText` / `DiegeticTextMut`
        // do internally: write `El.text` and bump the tree revision.
        app.world_mut()
            .get_mut::<DiegeticPanel>(panel)
            .expect("panel should exist")
            .sync_run_text_cache(element_idx, "Hello World");
        app.update();

        // The tree carries the new string ...
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .expect("panel should exist")
                .tree()
                .element_text(element_idx),
            Some("Hello World"),
        );
        // ... reification re-derived the run child from it ...
        let child = first_text_child(&mut app);
        assert_eq!(
            app.world()
                .get::<TextContent>(child)
                .expect("child should carry TextContent")
                .text(),
            "Hello World",
        );
        // ... and the wider string grew the content bounds.
        let after = content_width(&app, panel);
        assert!(
            after > before,
            "editing the run should relayout: width {before} -> {after}",
        );
    }

    // ── Panel↔run relationship lifecycle (`TextRunOf`/`PanelTextRuns`) ──

    /// Records whether any panel's [`PanelTextRuns`] changed since the probe last
    /// ran, so a test can assert reuse-only reification leaves the set untouched.
    #[derive(Resource, Default)]
    struct RunsChangedProbe {
        changed: bool,
    }

    fn probe_runs_changed(
        mut probe: ResMut<RunsChangedProbe>,
        changed: Query<(), Changed<PanelTextRuns>>,
    ) {
        probe.changed = !changed.is_empty();
    }

    /// Like [`reify_app`], but probes [`PanelTextRuns`] change detection
    /// instead of the per-component change probe.
    fn relationship_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(monospace_measurer());
        app.add_plugins(HeadlessLayoutPlugin);
        app.init_resource::<RunsChangedProbe>();
        app.add_systems(
            PostUpdate,
            (reify_text_entities, ApplyDeferred, probe_runs_changed).chain(),
        );
        app
    }

    fn empty_tree() -> LayoutTree { LayoutBuilder::new(100.0, 50.0).build() }

    fn two_named_tree(first: &str, second: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new(first, TextStyle::new(10.0)).id(PanelElementId::named("a")));
        builder.text(Text::new(second, TextStyle::new(10.0)).id(PanelElementId::named("b")));
        builder.build()
    }

    #[test]
    fn two_no_op_reification_passes_leave_panel_text_runs_unchanged() {
        let mut app = relationship_app();
        let panel = spawn_panel(&mut app, two_text_tree(Color::WHITE, Color::WHITE));

        // Frame 1 spawns both runs, which mutates the relationship set.
        app.update();
        assert!(
            app.world().resource::<RunsChangedProbe>().changed,
            "the spawn pass populates PanelTextRuns"
        );
        // Frame 2 settles: nothing dirties the panel, so the set is left alone.
        app.update();

        // Two further visual-only rebuilds: recolor only, same auto ids, so
        // Reification reuses every run entity. `TextRunOf` is never re-inserted, so
        // the relationship set must not register a change on either pass.
        for color in [Color::BLACK, Color::srgb(0.5, 0.5, 0.5)] {
            assert!(
                app.world_mut()
                    .commands()
                    .set_tree(panel, two_text_tree(color, Color::WHITE))
                    .is_ok()
            );
            app.update();
            assert!(
                !app.world().resource::<RunsChangedProbe>().changed,
                "reuse-only reification must not mutate PanelTextRuns",
            );
        }
    }

    #[test]
    fn set_tree_empties_the_run_set_then_reification_repopulates_it() {
        let mut app = relationship_app();
        let panel = spawn_panel(&mut app, two_named_tree("Alpha", "Beta"));
        app.update();
        assert_eq!(
            app.world()
                .get::<PanelTextRuns>(panel)
                .map(RelationshipTarget::len),
            Some(2),
            "two named runs spawn under the panel",
        );

        // Swap to a text-less tree: every run is unvisited this pass, so reification
        // despawns them and the relationship set empties.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, empty_tree())
                .is_ok()
        );
        app.update();
        let emptied = app
            .world()
            .get::<PanelTextRuns>(panel)
            .map_or(0, RelationshipTarget::len);
        assert_eq!(emptied, 0, "set_tree to an empty tree drops every run");

        // Swap back to a multi-run tree: reification repopulates the set and the
        // named index resolves each run by a single O(1) `text_child` lookup.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, two_named_tree("Gamma", "Delta"))
                .is_ok()
        );
        app.update();
        let runs = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("the set repopulates from the new tree");
        assert_eq!(runs.len(), 2, "both new runs re-enter the set");

        let data = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel exists");
        assert!(
            data.text_child(&PanelElementId::named("a")).is_some(),
            "named run 'a' resolves O(1) after repopulate",
        );
        assert!(
            data.text_child(&PanelElementId::named("b")).is_some(),
            "named run 'b' resolves O(1) after repopulate",
        );
    }

    #[test]
    fn identical_tree_replacement_preserves_named_text_lookup() {
        let mut app = relationship_app();
        let tree = two_named_tree("Alpha", "Beta");
        let panel = spawn_panel(&mut app, tree.clone());
        app.update();

        let id = PanelElementId::named("a");
        let before = app
            .world()
            .get::<DiegeticPanel>(panel)
            .and_then(|panel| panel.text_child(&id));
        assert!(before.is_some());

        let result = app.world_mut().commands().set_tree(panel, tree);
        assert!(result.is_ok());
        app.update();

        let after = app
            .world()
            .get::<DiegeticPanel>(panel)
            .and_then(|panel| panel.text_child(&id));
        assert_eq!(after, before);
    }

    #[test]
    fn panel_despawn_drops_all_runs_without_double_despawn() {
        let mut app = relationship_app();
        let panel = spawn_panel(&mut app, two_named_tree("Alpha", "Beta"));
        app.update();

        let runs: Vec<Entity> = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel carries its runs")
            .iter()
            .collect();
        assert_eq!(runs.len(), 2, "both runs are tracked before despawn");

        // `ChildOf`'s `linked_spawn` is the sole recursive despawn path;
        // `PanelTextRuns` deliberately omits `linked_spawn`, so despawning the
        // panel must drop every run exactly once with no second recursive pass.
        app.world_mut().entity_mut(panel).despawn();
        app.update();

        for run in runs {
            assert!(
                !app.world().entities().contains(run),
                "every run despawns with its panel",
            );
        }
        assert!(
            !app.world().entities().contains(panel),
            "the panel itself is gone",
        );
    }

    /// A run of auto text elements followed by one named run. Auto ids come from
    /// a per-build counter over unnamed `text()` declarations in build order, so
    /// an auto run's id is its position among the autos; the named run's id is
    /// fixed.
    fn autos_then_named_tree(autos: &[&str], named: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for text in autos {
            builder.text((*text, TextStyle::new(10.0)));
        }
        builder.text(Text::new(named, TextStyle::new(10.0)).id(PanelElementId::named("keep")));
        builder.build()
    }

    #[test]
    fn a_structural_edit_keeps_named_runs_but_repositions_auto_runs() {
        let mut app = reify_app();
        // One auto run ("first") and one named run ("keep").
        let panel = spawn_panel(&mut app, autos_then_named_tree(&["first"], "keep"));
        app.update();

        let before = labels_by_text(&mut app);
        let named_before = before["keep"];
        let first_before = before["first"];

        // Insert an auto sibling ahead of "first". The named run's key is
        // `(Named("keep"), 0)` — id-stable, so it keeps its entity. The
        // auto ids are positional: "inserted" takes `Auto(0)` and reuses the old
        // entity, so "first" shifts to `Auto(1)` and lands on a fresh entity —
        // content identity does not follow an auto run across the edit.
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, autos_then_named_tree(&["inserted", "first"], "keep"))
                .is_ok()
        );
        app.update();

        let after = labels_by_text(&mut app);
        assert_eq!(
            after["keep"], named_before,
            "the named run keeps its entity across the structural edit",
        );
        assert_ne!(
            after["first"], first_before,
            "the auto run is positional: \"first\" shifts to a new entity when a sibling is inserted ahead",
        );
    }

    /// Monospace measurer that counts every call, so a test can assert a no-op
    /// edit re-measures nothing.
    fn counting_measurer(counter: Arc<AtomicUsize>) -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(move |text: &str, measure: &TextMeasure| {
                counter.fetch_add(1, Ordering::Relaxed);
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let width = text
                    .lines()
                    .map(|line| line.chars().count().to_f32() * char_width)
                    .fold(0.0_f32, f32::max);
                let line_count = text.lines().count().max(1).to_f32();
                TextDimensions {
                    width,
                    height: measure.size * line_count,
                    line_height: measure.size,
                }
            }),
        }
    }

    /// [`text_source_app`] with a call-counting measurer in place of the plain
    /// monospace one.
    fn counting_text_source_app(counter: Arc<AtomicUsize>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(counting_measurer(counter));
        app.add_plugins(HeadlessLayoutPlugin);
        app.add_systems(PostUpdate, reify_text_entities);
        app
    }

    #[test]
    fn an_unchanged_set_text_fires_no_measure() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut app = counting_text_source_app(Arc::clone(&counter));
        let panel = spawn_panel(&mut app, one_text_tree("Hi"));
        app.update();
        app.update();
        let baseline = counter.load(Ordering::Relaxed);
        assert!(baseline > 0, "the initial layout measures the run");

        // Rewrite the byte-identical string through the public edit path.
        // `TextEdit::set_text` read-compares before taking the `&mut DiegeticPanel`
        // borrow, so an unchanged string never dirties the panel, never relayouts,
        // and so `MeasureTextFn` fires zero more times.
        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| text.set_sole_text(panel, "Hi"))
            .expect("system runs");
        assert!(wrote, "the lone run resolves");
        app.update();

        assert_eq!(
            counter.load(Ordering::Relaxed),
            baseline,
            "a no-op set_text must not re-invoke MeasureTextFn",
        );
    }
}
