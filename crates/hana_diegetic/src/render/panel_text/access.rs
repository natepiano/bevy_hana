//! Public get/set access to a panel's text runs.
//!
//! Two ways to address a run divide the public surface:
//! - **By user marker** — [`DiegeticTextMut`] retexts standalone
//!   [`DiegeticText`](crate::DiegeticText) labels: name a marker `M`, call `set`/`for_each_mut`. It
//!   traverses the panel→run relationship internally, the ergonomic path for "retext my labels".
//! - **By [`PanelElementId`]** — [`PanelText`] is the read-write `SystemParam` for a named run's
//!   text or authored style on a multi-run panel; [`PanelTextReader`] is the read-only variant a
//!   reader system uses so it does not serialize on the `&mut DiegeticPanel` write claim.
//!
//! The id-addressed pair resolve a run through the panel's `id → Entity` index
//! ([`DiegeticPanel::text_child`]) and validate liveness via their
//! `PanelTextLayout` query, so a despawned run reads back as `None` rather than
//! a dangling `Entity`. The lone-run helpers ([`PanelTextReader::sole_text`],
//! [`PanelText::set_sole_text`], and [`DiegeticTextMut`]) resolve a label's run
//! through its [`PanelTextRuns`] set.

use bevy::ecs::system::SystemParam;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Mut;
use bevy::prelude::Query;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::warn;

use super::PanelTextLayout;
use super::PanelTextRuns;
use crate::PanelElementId;
use crate::layout::TextStyle;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPanelChangeClassification;

/// Read-only access to panel text runs by [`PanelElementId`].
///
/// Reads come from the `El.text` layout cache, which is authoritative for the
/// full run string — a wrapped run reifies as one child per visual line,
/// each holding a per-line slice, so no single child entity owns the whole
/// string. Use this in reader systems; it holds no mutable claim and so runs in
/// parallel with other text readers.
#[derive(SystemParam)]
pub struct PanelTextReader<'w, 's> {
    panels:  Query<'w, 's, (&'static DiegeticPanel, Option<&'static PanelTextRuns>)>,
    layouts: Query<'w, 's, &'static PanelTextLayout>,
}

impl PanelTextReader<'_, '_> {
    /// Resolves a named run to its `line_index == 0` entity, or `None` if no run
    /// carries the id or the mapped entity is no longer a live run.
    #[must_use]
    pub fn entity(&self, panel: Entity, id: &PanelElementId) -> Option<Entity> {
        let (data, _) = self.panels.get(panel).ok()?;
        self.resolve(data, id)
    }

    /// Reads a named run's full string from the `El.text` cache, never a line
    /// slice, so a wrapped run returns its whole text.
    #[must_use]
    pub fn text(&self, panel: Entity, id: &PanelElementId) -> Option<&str> {
        let (data, _) = self.panels.get(panel).ok()?;
        let child = self.resolve(data, id)?;
        let layout = self.layouts.get(child).ok()?;
        data.tree().element_text(layout.element_idx)
    }

    /// Resolves a run id to a live entity, the shared chokepoint for `entity` /
    /// `text`. Delegates to the free [`resolve_run_entity`] so [`PanelText`]
    /// (which holds a `&mut DiegeticPanel` query that cannot coexist with this
    /// reader's `&DiegeticPanel` one) shares the same resolution.
    fn resolve(&self, data: &DiegeticPanel, id: &PanelElementId) -> Option<Entity> {
        resolve_run_entity(data, id, &self.layouts)
    }

    /// Reads the lone run of a one-element panel (the runtime form of a
    /// [`DiegeticText`](crate::DiegeticText)), no id needed. Returns `None` if the
    /// panel has zero or more than one run.
    #[must_use]
    pub fn sole_text(&self, panel: Entity) -> Option<&str> {
        let child = self.sole_run_entity(panel)?;
        let (data, _) = self.panels.get(panel).ok()?;
        let layout = self.layouts.get(child).ok()?;
        data.tree().element_text(layout.element_idx)
    }

    /// Resolves a one-element panel's lone `line_index == 0` run entity from its
    /// [`PanelTextRuns`] set. The run's `Auto` id is not caller-addressable, so
    /// the lone run is found through the relationship rather than the id index.
    ///
    /// Filters `line_index == 0` rather than taking [`PanelTextRuns::sole`]: a
    /// wrapped run reifies as one entity per visual line, so its set holds
    /// several entities yet still has one logical run (its line-0 entity).
    /// Returns `None` when the panel has no run or more than one distinct run
    /// (the call is ambiguous then).
    fn sole_run_entity(&self, panel: Entity) -> Option<Entity> {
        let (_, runs) = self.panels.get(panel).ok()?;
        let runs = runs?;
        let mut found = None;
        for run in runs.iter() {
            let Ok(layout) = self.layouts.get(run) else {
                continue;
            };
            if layout.line_index == 0 {
                if found.is_some() {
                    return None;
                }
                found = Some(run);
            }
        }
        found
    }
}

/// Read-write access to panel text runs by [`PanelElementId`].
///
/// `set_text` writes the run's string into the panel's authoritative `El.text`
/// cache, while `set_style` writes its authored `El.config`. Either edit bumps
/// the tree revision, so the next layout and reification derive the run child
/// from the same source. Passing the whole string works for both single-line
/// and wrapped runs. Reads also come from the `El.text` cache. Both go through a single
/// `&mut DiegeticPanel` query, so this cannot embed a [`PanelTextReader`] (its
/// `&DiegeticPanel` query would conflict); the resolution helpers are shared as
/// free functions instead.
///
/// The `&mut DiegeticPanel` claim serializes against other panel writers: one
/// system should own the `PanelText` writes per frame; reader systems take
/// [`PanelTextReader`] instead.
#[derive(SystemParam)]
pub struct PanelText<'w, 's> {
    panels: Query<
        'w,
        's,
        (
            &'static mut DiegeticPanel,
            Option<&'static PanelTextRuns>,
            &'static mut DiegeticPanelChangeClassification,
        ),
    >,
    layouts: Query<'w, 's, &'static PanelTextLayout>,
}

impl PanelText<'_, '_> {
    /// Resolves a named run to its `line_index == 0` entity. See
    /// [`PanelTextReader::entity`].
    #[must_use]
    pub fn entity(&self, panel: Entity, id: &PanelElementId) -> Option<Entity> {
        let (data, _, _) = self.panels.get(panel).ok()?;
        resolve_run_entity(data, id, &self.layouts)
    }

    /// Reads a named run's full string from the `El.text` cache. See
    /// [`PanelTextReader::text`].
    #[must_use]
    pub fn text(&self, panel: Entity, id: &PanelElementId) -> Option<&str> {
        let (data, _, _) = self.panels.get(panel).ok()?;
        let child = resolve_run_entity(data, id, &self.layouts)?;
        let layout = self.layouts.get(child).ok()?;
        data.tree().element_text(layout.element_idx)
    }

    /// Reads the lone run of a one-element panel. See
    /// [`PanelTextReader::sole_text`].
    #[must_use]
    pub fn sole_text(&self, panel: Entity) -> Option<&str> {
        let child = self.sole_run_entity(panel)?;
        let (data, _, _) = self.panels.get(panel).ok()?;
        let layout = self.layouts.get(child).ok()?;
        data.tree().element_text(layout.element_idx)
    }

    /// The lone `line_index == 0` run of a one-element panel, or `None` when the
    /// panel has no run or more than one distinct run. See
    /// [`PanelTextReader::sole_run_entity`].
    fn sole_run_entity(&self, panel: Entity) -> Option<Entity> {
        let (_, runs, _) = self.panels.get(panel).ok()?;
        lone_run(runs?, &self.layouts)
    }

    /// Sets a named run's text, returning whether a run was found. Writes the
    /// whole string into the tree, so a wrapped run is retext by passing the
    /// whole replacement; an unchanged string leaves the panel un-dirtied (no
    /// relayout) via [`TextEdit::set_text`].
    pub fn set_text(
        &mut self,
        panel: Entity,
        id: &PanelElementId,
        text: impl Into<String>,
    ) -> bool {
        let Some(child) = self.entity(panel, id) else {
            return false;
        };
        let Ok(layout) = self.layouts.get(child) else {
            return false;
        };
        let element_idx = layout.element_idx;
        let Ok((data, _, classification)) = self.panels.get_mut(panel) else {
            return false;
        };
        TextEdit {
            panel: data,
            classification,
            element_idx,
        }
        .set_text(text);
        true
    }

    /// Sets a named run's authored style, returning whether a run was found.
    ///
    /// The panel tree's `El.config` is authoritative for both measurement and
    /// rendering; the reified run child is derived output. `set_style`
    /// writes the tree config and lets the next layout and reification apply the
    /// style, so a font change also refits the panel and a cascade-authoring
    /// change remains in force after later text edits. An unchanged style does
    /// not dirty the panel.
    pub fn set_style(&mut self, panel: Entity, id: &PanelElementId, style: TextStyle) -> bool {
        let Some(child) = self.entity(panel, id) else {
            return false;
        };
        let Ok(layout) = self.layouts.get(child) else {
            return false;
        };
        let element_idx = layout.element_idx;
        let Ok((mut data, _, _)) = self.panels.get_mut(panel) else {
            return false;
        };
        data.restyle_run(element_idx, style);
        true
    }

    /// Sets the lone run of a one-element panel (a
    /// [`DiegeticText`](crate::DiegeticText)), no id needed. Returns whether a
    /// single run was found.
    pub fn set_sole_text(&mut self, panel: Entity, text: impl Into<String>) -> bool {
        let Some(child) = self.sole_run_entity(panel) else {
            return false;
        };
        let Ok(layout) = self.layouts.get(child) else {
            return false;
        };
        let element_idx = layout.element_idx;
        let Ok((data, _, classification)) = self.panels.get_mut(panel) else {
            return false;
        };
        TextEdit {
            panel: data,
            classification,
            element_idx,
        }
        .set_text(text);
        true
    }

    /// Sets the authored style of the lone run of a one-element panel (a
    /// [`DiegeticText`](crate::DiegeticText)), no id needed. Returns whether a
    /// single run was found.
    ///
    /// A [`DiegeticText`](crate::DiegeticText)'s run is anonymous — it is given
    /// a crate-internal [`PanelElementId::Auto`] id that a caller cannot build —
    /// so this is the only way to restyle one such label. See
    /// [`Self::set_style`] for why the write goes to the tree rather than to the
    /// reified run child.
    pub fn set_sole_style(&mut self, panel: Entity, style: TextStyle) -> bool {
        let Some(child) = self.sole_run_entity(panel) else {
            return false;
        };
        let Ok(layout) = self.layouts.get(child) else {
            return false;
        };
        let element_idx = layout.element_idx;
        let Ok((mut data, _, _)) = self.panels.get_mut(panel) else {
            return false;
        };
        data.restyle_run(element_idx, style);
        true
    }
}

/// Resolves a run `id` to a live entity against `data`'s `id → Entity` index,
/// the shared chokepoint behind [`PanelTextReader::entity`] / `text` and
/// [`PanelText::set_text`]. A free function (not a method) because [`PanelText`]
/// resolves through a `&mut DiegeticPanel` query that cannot coexist with
/// [`PanelTextReader`]'s `&DiegeticPanel` one, so neither can embed the other.
///
/// `text_child` is an unchecked index read; the `layouts` query confirms the
/// entity is still a live run. A stale index entry (entity despawned out
/// of flow before reification rebuilt the index) falls to the miss path, but its
/// id is still valid in the tree — authoritative for valid ids at build time,
/// unlike the reification-timed index — so a `#[cfg(debug_assertions)]` `warn!`
/// fires only on a genuine typo, leaving the first-frame / post-`set_tree`
/// window quiet.
fn resolve_run_entity(
    data: &DiegeticPanel,
    id: &PanelElementId,
    layouts: &Query<&PanelTextLayout>,
) -> Option<Entity> {
    if let Some(child) = data.text_child(id)
        && layouts.contains(child)
    {
        return Some(child);
    }
    #[cfg(debug_assertions)]
    if !data.tree().contains_text_id(id) {
        warn!("no text run with id {id}");
    }
    None
}

/// The `line_index == 0` entity of a label's lone run, or `None` when the set
/// has no run or more than one distinct run.
///
/// Shared by [`DiegeticTextMut`] and [`PanelText`]; the same rule
/// [`PanelTextReader::sole_run_entity`] applies, so a wrapped label (one entity
/// per visual line) resolves to its line-0 entity rather than `None`.
fn lone_run(runs: &PanelTextRuns, layouts: &Query<&PanelTextLayout>) -> Option<Entity> {
    let mut found = None;
    for run in runs.iter() {
        let Ok(layout) = layouts.get(run) else {
            continue;
        };
        if layout.line_index == 0 {
            if found.is_some() {
                return None;
            }
            found = Some(run);
        }
    }
    found
}

/// A tree-routed edit handle for one panel text run.
///
/// Handed to the [`DiegeticTextMut::for_each_mut`] closure and used internally by
/// [`PanelText`]. Keeps the `text()` / `set_text()` ergonomics callers had on
/// `&mut TextContent`, but forwards them to the panel's authoritative `El.text`
/// cache instead of the derived run child.
///
/// `set_text` read-compares against the current tree string first, so an
/// unchanged write never takes the `&mut DiegeticPanel` path and never dirties
/// the panel — a no-op edit drives no relayout and no measure. It holds a
/// [`Mut`] (not a `&mut`) for the same reason: reads go through `Deref` and do
/// not flag the panel changed.
///
/// A real write also records a `VisualOnly` change on the panel's
/// `DiegeticPanelChangeClassification` sibling, so `compute_panel_layouts`
/// re-measures only the edited leaf and takes the geometry-stable skip (reuse
/// cached geometry, regenerate commands) when the box did not move — leaving the
/// full engine solve for a genuine reflow.
pub struct TextEdit<'a> {
    panel:          Mut<'a, DiegeticPanel>,
    classification: Mut<'a, DiegeticPanelChangeClassification>,
    element_idx:    usize,
}

impl TextEdit<'_> {
    /// The run's current string from the `El.text` cache, or `""` if the element
    /// index no longer resolves.
    #[must_use]
    pub fn text(&self) -> &str {
        self.panel
            .tree()
            .element_text(self.element_idx)
            .unwrap_or_default()
    }

    /// Writes the whole run string into the `El.text` cache, bumping the tree
    /// revision so layout re-wraps and reification re-derives the child, and
    /// recording the edit as `VisualOnly` for the geometry-stable skip. An
    /// unchanged string is skipped before the `&mut` access, so it neither
    /// dirties the panel, records a change, nor triggers a relayout.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        // Read through `Deref` (no change flag) and return on a no-op, so an
        // unchanged string never takes the `&mut` path.
        if self.panel.tree().element_text(self.element_idx) == Some(text.as_str()) {
            return;
        }
        if self.panel.sync_run_text_cache(self.element_idx, &text) {
            self.classification.note_text_edit();
        }
    }
}

/// Ergonomic mutation of standalone [`DiegeticText`](crate::DiegeticText) labels
/// addressed by a user marker `M` — the public path for "retext my labels".
///
/// A standalone label is a one-element panel: the marker `M` sits on the panel
/// entity, while the run text is stored in the panel's authoritative `El.text`
/// tree cache (the run child's `TextContent` is derived output reification
/// rewrites). This `SystemParam` traverses the panel→run relationship
/// internally, so a caller
/// names only its own marker and never touches [`PanelTextRuns`] / the tree /
/// `sole()`:
///
/// ```ignore
/// fn rename(mut labels: DiegeticTextMut<CubeFaceLabel>) {
///     labels.set("hello");
/// }
/// ```
///
/// [`set`](Self::set) writes one string to every `M`-marked label (a single
/// label or a uniform update); [`for_each_mut`](Self::for_each_mut) yields each
/// label's marker and a [`TextEdit`] handle for per-label strings. Both resolve
/// a label's lone run by its `line_index == 0` entity, so a wrapped label is
/// editable too.
///
/// Monomorphizes per distinct marker type used in a system (a handful),
/// independent of label count; an unused marker costs nothing. For an
/// id-addressed run on a multi-run panel, use [`PanelText`] instead.
#[derive(SystemParam)]
pub struct DiegeticTextMut<'w, 's, M: Component> {
    runs:    Query<'w, 's, (Entity, &'static M, &'static PanelTextRuns)>,
    layouts: Query<'w, 's, &'static PanelTextLayout>,
    panels: Query<
        'w,
        's,
        (
            &'static mut DiegeticPanel,
            &'static mut DiegeticPanelChangeClassification,
        ),
    >,
}

impl<M: Component> DiegeticTextMut<'_, '_, M> {
    /// Writes `text` to every `M`-marked label's lone run, returning how many
    /// labels resolved. An unchanged string leaves a label un-dirtied.
    pub fn set(&mut self, text: impl Into<String>) -> usize {
        let text = text.into();
        let mut written = 0;
        for (panel_entity, _, runs) in &self.runs {
            let Some(run) = lone_run(runs, &self.layouts) else {
                continue;
            };
            let Ok(layout) = self.layouts.get(run) else {
                continue;
            };
            let element_idx = layout.element_idx;
            let Ok((panel, classification)) = self.panels.get_mut(panel_entity) else {
                continue;
            };
            TextEdit {
                panel,
                classification,
                element_idx,
            }
            .set_text(text.clone());
            written += 1;
        }
        written
    }

    /// Calls `f` with each `M`-marked label's marker and a [`TextEdit`] handle,
    /// for setting a different string per label. Returns how many labels were
    /// visited.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&M, &mut TextEdit)) -> usize {
        let mut visited = 0;
        for (panel_entity, marker, runs) in &self.runs {
            let Some(run) = lone_run(runs, &self.layouts) else {
                continue;
            };
            let Ok(layout) = self.layouts.get(run) else {
                continue;
            };
            let element_idx = layout.element_idx;
            let Ok((panel, classification)) = self.panels.get_mut(panel_entity) else {
                continue;
            };
            let mut edit = TextEdit {
                panel,
                classification,
                element_idx,
            };
            f(marker, &mut edit);
            visited += 1;
        }
        visited
    }

    /// Calls `f` with each `M`-marked label's authored [`TextStyle`], for
    /// restyling a label (font, color, size) at runtime. Returns how many labels
    /// were visited.
    ///
    /// Unlike text, the authoritative style is the panel tree's `El.config`, not
    /// the run child — the run's style is a `for_shaping`-derived projection the
    /// layout engine never measures. So this edits the tree config in place and
    /// relayouts: the new style reaches both measurement (the panel re-fits to
    /// the new font) and rendering (reification re-derives the run from the config).
    /// Mutating the run style alone would render the new font while measuring the
    /// old one, the exact mismatch this routes around.
    pub fn for_each_style_mut(&mut self, mut f: impl FnMut(&mut TextStyle)) -> usize {
        let mut visited = 0;
        for (panel_entity, _, runs) in &self.runs {
            let Some(run) = lone_run(runs, &self.layouts) else {
                continue;
            };
            let Ok(layout) = self.layouts.get(run) else {
                continue;
            };
            let element_idx = layout.element_idx;
            // A restyle changes `El.config`, which can affect measurement, so it
            // is left to the full engine solve — the classification slot stays at
            // its default (no `VisualOnly` skip recorded here).
            let Ok((mut panel, _)) = self.panels.get_mut(panel_entity) else {
                continue;
            };
            let Some(mut style) = panel.tree().element_style(element_idx).cloned() else {
                continue;
            };
            f(&mut style);
            panel.restyle_run(element_idx, style);
            visited += 1;
        }
        visited
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Changed;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Last;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::With;
    use hana_kana::ToF32;

    use super::DiegeticTextMut;
    use super::PanelText;
    use super::PanelTextReader;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::constants::MONOSPACE_WIDTH_RATIO;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::Text;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::layout::TextWrap;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanel;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render::panel_text::PanelTextLayout;
    use crate::render::panel_text::PanelTextRuns;
    use crate::render::panel_text::reify;
    use crate::text::DiegeticTextMeasurer;

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

    /// A measurer whose height encodes the `font_id`, so a restyle that changes
    /// the font produces an observably different measured size. A
    /// `for_each_style_mut` that wrote the run's derived style instead of the
    /// authoritative tree config would leave the layout engine measuring the old
    /// font; a font-id-sensitive height turns that into a test failure.
    fn font_id_height_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|text: &str, measure: &TextMeasure| {
                let char_width = measure.size * MONOSPACE_WIDTH_RATIO;
                let width = text.chars().count().to_f32() * char_width;
                TextDimensions {
                    width,
                    height: measure.size * (1.0 + f32::from(measure.font_id)),
                    line_height: measure.size,
                }
            }),
        }
    }

    /// Headless layout plus the reification pass (`PostUpdate`), so the public
    /// `PanelText` / `DiegeticTextMut` get/set is exercised against a live index:
    /// a write goes straight to the authoritative tree (`El.text`), the layout
    /// pipeline relayouts, and reification re-derives the run child. Parameterized
    /// by measurer so a test can choose one whose output depends on the font.
    fn app_with_measurer(measurer: DiegeticTextMeasurer) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(measurer);
        app.add_plugins(HeadlessLayoutPlugin);
        app.add_systems(PostUpdate, reify::reify_text_entities);
        app
    }

    /// Headless layout with the monospace approximation measurer — the default
    /// for tests whose assertions do not depend on which font measures.
    fn access_app() -> App { app_with_measurer(monospace_measurer()) }

    fn auto_tree(text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, TextStyle::new(10.0)));
        builder.build()
    }

    fn named_tree(id: &PanelElementId, text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new(text, TextStyle::new(10.0)).id(id.clone()));
        builder.build()
    }

    /// Two distinct top-level text elements: two runs, each its own line-0, so
    /// the lone-run path is ambiguous.
    fn two_text_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(("Alpha", TextStyle::new(10.0)));
        builder.text(("Beta", TextStyle::new(10.0)));
        builder.build()
    }

    /// A tree with no text element at all, so the panel never gains a run.
    fn empty_tree() -> LayoutTree { LayoutBuilder::new(100.0, 50.0).build() }

    /// One text element that wraps at explicit newlines into `n` visual lines,
    /// so its run reifies as `n` entities sharing one id (`line_index`
    /// 0..n).
    fn wrapped_tree(text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new(text, TextStyle::new(10.0)).wrap(TextWrap::Newlines));
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

    /// Spawns and runs two frames so the run reifies and the layout settles
    /// before a test edits it.
    fn settled_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = spawn_panel(app, tree);
        app.update();
        app.update();
        panel
    }

    fn content_width(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel should exist")
            .content_bounds()
            .expect("content bounds should exist")
            .width
    }

    fn content_height(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel should exist")
            .content_bounds()
            .expect("content bounds should exist")
            .height
    }

    fn first_run_entity(app: &mut App) -> Entity {
        let mut state = app
            .world_mut()
            .query_filtered::<Entity, With<PanelTextLayout>>();
        let runs: Vec<Entity> = state.iter(app.world()).collect();
        assert_eq!(runs.len(), 1, "expected exactly one run child");
        runs[0]
    }

    #[test]
    fn reader_resolves_a_named_run_and_reads_its_text() {
        let mut app = access_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));

        let lookup = id.clone();
        let resolved = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| reader.entity(panel, &lookup))
            .expect("system runs");
        assert!(
            resolved.is_some(),
            "a named run should resolve to an entity"
        );

        let lookup = id;
        let text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.text(panel, &lookup).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(text.as_deref(), Some("Hi"));
    }

    #[test]
    fn unknown_id_resolves_to_none() {
        let mut app = access_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));

        let resolved = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.entity(panel, &PanelElementId::named("missing"))
            })
            .expect("system runs");
        assert!(resolved.is_none(), "an unknown id must not resolve");
    }

    #[test]
    fn auto_id_run_is_not_addressable_but_sole_text_reads_it() {
        let mut app = access_app();
        let panel = settled_panel(&mut app, auto_tree("Hi"));

        // No public `PanelElementId` names an auto run, so a named lookup misses.
        let by_name = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.entity(panel, &PanelElementId::named("Hi"))
            })
            .expect("system runs");
        assert!(by_name.is_none(), "an auto-id run is not name-addressable");

        // The one-element `sole_text` path reaches it without an id.
        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(sole.as_deref(), Some("Hi"));
    }

    #[test]
    fn set_text_through_panel_text_relayouts() {
        let mut app = access_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));
        let before = content_width(&app, panel);

        let target = id.clone();
        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_text(panel, &target, "Hello World")
            })
            .expect("system runs");
        assert!(wrote, "a named run should accept the write");
        app.update();

        let read = id;
        let after_text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.text(panel, &read).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(after_text.as_deref(), Some("Hello World"));

        let after = content_width(&app, panel);
        assert!(
            after > before,
            "the wider string should relayout: width {before} -> {after}",
        );
    }

    #[test]
    fn set_text_unknown_id_is_a_noop() {
        let mut app = access_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));

        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_text(panel, &PanelElementId::named("missing"), "ignored")
            })
            .expect("system runs");
        assert!(!wrote, "an unknown id must not write");

        let read = id;
        let still = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.text(panel, &read).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(still.as_deref(), Some("Hi"), "the run text is unchanged");
    }

    #[test]
    fn set_style_through_panel_text_refits_the_panel() {
        let mut app = app_with_measurer(font_id_height_measurer());
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));
        let before = content_height(&app, panel);

        let target = id;
        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_style(panel, &target, TextStyle::new(10.0).with_font(1))
            })
            .expect("system runs");
        assert!(wrote, "a named run should accept the restyle");
        app.update();

        let after = content_height(&app, panel);
        assert!(
            after > before,
            "a named restyle must update the authoritative tree and refit the panel: \
             height {before} -> {after}",
        );
    }

    #[test]
    fn orphaned_run_resolves_to_none_through_the_system_param() {
        let mut app = access_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));

        // Despawn the run out of flow. The panel's `text_index` still maps the id
        // to the now-dead entity until the next reification rebuilds it, so the
        // liveness check inside the SystemParam is what must yield `None`.
        let run = first_run_entity(&mut app);
        app.world_mut().entity_mut(run).despawn();

        let lookup = id;
        let resolved = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| reader.entity(panel, &lookup))
            .expect("system runs");
        assert!(
            resolved.is_none(),
            "a despawned run must resolve to None, not a dangling entity",
        );
    }

    #[test]
    fn set_sole_text_retexts_a_one_element_panel() {
        let mut app = access_app();
        let panel = settled_panel(&mut app, auto_tree("Hi"));

        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| text.set_sole_text(panel, "Bye"))
            .expect("system runs");
        assert!(wrote, "the lone run should accept the write");
        app.update();

        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(sole.as_deref(), Some("Bye"));
    }

    /// Marks a panel as a single retextable label for [`DiegeticTextMut`].
    #[derive(Component)]
    struct Label;

    /// Marks a panel and carries which face it is, so a per-label update can set
    /// a different string per marked panel.
    #[derive(Component)]
    struct Face(u8);

    #[test]
    fn panel_text_runs_populates_and_sole_returns_the_lone_run() {
        let mut app = access_app();
        let panel = settled_panel(&mut app, auto_tree("Hi"));

        let runs = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel should carry PanelTextRuns");
        assert_eq!(runs.len(), 1, "a single-line label has exactly one run");
        assert!(
            runs.sole().is_some(),
            "a one-run set resolves through sole()"
        );
    }

    #[test]
    fn diegetic_text_mut_set_retexts_a_marked_label() {
        let mut app = access_app();
        let panel = spawn_panel(&mut app, auto_tree("Hi"));
        app.world_mut().entity_mut(panel).insert(Label);
        app.update();
        app.update();
        let before = content_width(&app, panel);

        let written = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Label>| labels.set("Hello World"))
            .expect("system runs");
        assert_eq!(written, 1, "one marked label is written");
        app.update();

        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(sole.as_deref(), Some("Hello World"));
        let after = content_width(&app, panel);
        assert!(
            after > before,
            "the wider string should relayout: width {before} -> {after}",
        );
    }

    #[test]
    fn diegetic_text_mut_for_each_mut_sets_per_label_strings() {
        let mut app = access_app();
        let front = spawn_panel(&mut app, auto_tree("A"));
        let back = spawn_panel(&mut app, auto_tree("B"));
        app.world_mut().entity_mut(front).insert(Face(0));
        app.world_mut().entity_mut(back).insert(Face(1));
        app.update();
        app.update();

        let visited = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Face>| {
                labels.for_each_mut(|face, content| content.set_text(format!("face {}", face.0)))
            })
            .expect("system runs");
        assert_eq!(visited, 2, "both marked labels are visited");
        app.update();

        let front_text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(front).map(str::to_owned)
            })
            .expect("system runs");
        let back_text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(back).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(front_text.as_deref(), Some("face 0"));
        assert_eq!(back_text.as_deref(), Some("face 1"));
    }

    #[test]
    fn restyle_through_diegetic_text_mut_refits_the_panel() {
        // The measurer's height grows with `font_id`, so re-fitting the panel
        // after a restyle is observable as a height change. If
        // `for_each_style_mut` mutated only the run's derived style, the tree
        // config the engine measures would keep font 0 and the height would
        // hold — the new font rendering while the panel never resized to it.
        let mut app = app_with_measurer(font_id_height_measurer());
        let panel = spawn_panel(&mut app, auto_tree("Hi"));
        app.world_mut().entity_mut(panel).insert(Label);
        app.update();
        app.update();
        let before = content_height(&app, panel);

        let visited = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Label>| {
                labels.for_each_style_mut(|style| style.set_font_id(1))
            })
            .expect("system runs");
        assert_eq!(visited, 1, "one marked label is restyled");
        app.update();

        let after = content_height(&app, panel);
        assert!(
            after > before,
            "a font restyle must re-fit the panel through the authoritative tree \
             config the layout engine measures: height {before} -> {after}",
        );
    }

    #[test]
    fn sole_resolution_is_none_for_a_multi_run_panel() {
        let mut app = access_app();
        let panel = spawn_panel(&mut app, two_text_tree());
        app.world_mut().entity_mut(panel).insert(Label);
        app.update();
        app.update();

        // Raw count contract (`relationship.rs`): two distinct runs, so the
        // count-based `sole()` declines.
        let runs = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel should carry PanelTextRuns");
        assert_eq!(runs.len(), 2, "two distinct elements spawn two runs");
        assert!(runs.sole().is_none(), "a two-run set has no lone run");

        // Access-layer contract (`access.rs` `sole_run_entity`): two line-0 runs
        // are ambiguous, so the lone-run read resolves to None.
        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert!(
            sole.is_none(),
            "sole_text is None when two distinct runs make the call ambiguous"
        );

        // The marker mutator hits the same `lone_run` guard and writes nothing.
        let written = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Label>| labels.set("ignored"))
            .expect("system runs");
        assert_eq!(written, 0, "an ambiguous multi-run label is not written");
    }

    #[test]
    fn sole_resolution_is_none_for_a_zero_run_panel() {
        let mut app = access_app();
        let panel = spawn_panel(&mut app, empty_tree());
        app.world_mut().entity_mut(panel).insert(Label);
        app.update();
        app.update();

        // No `TextRunOf` source ever points at the panel, so it gains no
        // `PanelTextRuns` target at all.
        assert!(
            app.world().get::<PanelTextRuns>(panel).is_none(),
            "a panel with no run gains no relationship target"
        );

        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert!(
            sole.is_none(),
            "sole_text is None when the panel has no run"
        );

        let written = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Label>| labels.set("ignored"))
            .expect("system runs");
        assert_eq!(written, 0, "a run-less label is not written");
    }

    #[test]
    fn a_wrapped_label_resolves_through_sole_text_and_diegetic_text_mut() {
        let mut app = access_app();
        let panel = spawn_panel(&mut app, wrapped_tree("Line1\nLine2\nLine3"));
        app.world_mut().entity_mut(panel).insert(Label);
        app.update();
        app.update();

        // A wrapped run is one entity per visual line, so the count-based
        // `sole()` sees three entities and declines.
        let runs = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel should carry PanelTextRuns");
        assert_eq!(
            runs.len(),
            3,
            "three wrapped lines spawn three run entities"
        );
        assert!(
            runs.sole().is_none(),
            "the count-based sole() sees three entities for one logical run"
        );

        // The access layer filters `line_index == 0`, so the lone logical run
        // still resolves and reads back its whole string from the cache.
        let before = content_width(&app, panel);
        let sole = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(sole.as_deref(), Some("Line1\nLine2\nLine3"));

        // `DiegeticTextMut::set` retexts that line-0 entity; the reactor re-wraps
        // from the new string, collapsing the run back to a single narrow line.
        let written = app
            .world_mut()
            .run_system_once(|mut labels: DiegeticTextMut<Label>| labels.set("X"))
            .expect("system runs");
        assert_eq!(written, 1, "the wrapped label's lone run is written once");
        app.update();

        let after_text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.sole_text(panel).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(after_text.as_deref(), Some("X"));
        let after = content_width(&app, panel);
        assert!(
            after < before,
            "the narrower replacement should relayout: width {before} -> {after}",
        );
    }

    /// A named run that wraps at explicit newlines, so it reifies as one
    /// entity per line while staying id-addressable.
    fn named_wrapped_tree(id: &PanelElementId, text: &str) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new(text, TextStyle::new(10.0))
                .id(id.clone())
                .wrap(TextWrap::Newlines),
        );
        builder.build()
    }

    #[test]
    fn set_text_on_a_named_wrapped_run_replaces_the_whole_string() {
        let mut app = access_app();
        let id = PanelElementId::named("body");
        let panel = settled_panel(&mut app, named_wrapped_tree(&id, "one\ntwo"));

        // Two wrapped lines reify as two run entities sharing the id.
        let runs = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel should carry PanelTextRuns");
        assert_eq!(runs.len(), 2, "two wrapped lines, two run entities");

        // Write a three-line replacement through the id-addressed `set_text`. It
        // resolves the `line_index == 0` child; the reactor re-wraps from the new
        // whole string.
        let target = id.clone();
        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_text(panel, &target, "alpha\nbeta\ngamma")
            })
            .expect("system runs");
        assert!(wrote, "the named wrapped run accepts the write");
        app.update();

        // `text` reads the full `El.text` cache, never a line slice, so the whole
        // new string round-trips with no line dropped.
        let read = id;
        let after_text = app
            .world_mut()
            .run_system_once(move |reader: PanelTextReader| {
                reader.text(panel, &read).map(str::to_owned)
            })
            .expect("system runs");
        assert_eq!(after_text.as_deref(), Some("alpha\nbeta\ngamma"));

        // The replacement re-wrapped into three line runs.
        let runs_after = app
            .world()
            .get::<PanelTextRuns>(panel)
            .expect("a reified panel should carry PanelTextRuns");
        assert_eq!(
            runs_after.len(),
            3,
            "the three-line replacement re-wraps into three run entities",
        );
    }

    /// Counts the frames in which any panel's [`ComputedDiegeticPanel`] changed,
    /// so a test can assert a `set_text` edit drives exactly one relayout pass.
    #[derive(Resource, Default)]
    struct ComputedChanges(usize);

    fn count_computed_changes(
        mut probe: ResMut<ComputedChanges>,
        changed: Query<(), Changed<ComputedDiegeticPanel>>,
    ) {
        if !changed.is_empty() {
            probe.0 += 1;
        }
    }

    /// [`access_app`] plus a `Last`-schedule probe that counts relayout passes.
    fn single_pass_app() -> App {
        let mut app = access_app();
        app.init_resource::<ComputedChanges>();
        app.add_systems(Last, count_computed_changes);
        app
    }

    #[test]
    fn a_set_text_edit_fires_exactly_one_relayout_pass() {
        let mut app = single_pass_app();
        let id = PanelElementId::named("title");
        let panel = settled_panel(&mut app, named_tree(&id, "Hi"));
        // One more frame so the panel is fully quiescent before the edit.
        app.update();

        // Discard the spawn/settle passes; count only what the edit triggers.
        app.world_mut().resource_mut::<ComputedChanges>().0 = 0;

        let target = id;
        let wrote = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_text(panel, &target, "Hello World")
            })
            .expect("system runs");
        assert!(wrote, "the edit is accepted");

        // Run several frames. The edit must drive exactly one relayout: the write
        // lands once in the authoritative tree, reification re-derives the run child
        // from it, and — with no child→tree sync-back — nothing oscillates after.
        app.update();
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<ComputedChanges>().0,
            1,
            "a single set_text edit fires exactly one ComputedDiegeticPanel change",
        );
    }
}
