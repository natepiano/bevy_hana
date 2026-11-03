use core::mem::size_of;
use std::collections::HashSet;

use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::Query;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Vec2;
use bevy::prelude::With;

use super::VisualSlotId;
use crate::widgets::SliderDirection;
use crate::widgets::VisualElementCapabilities;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

/// SDF extent for one slider fill surface.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum FillExtentOverride {
    /// The SDF record keeps its authored geometry.
    #[default]
    Authored,
    /// The fill keeps the edge where `direction`'s values start and ends
    /// at `fraction` (0.0..=1.0) of its authored length on that axis.
    LeadingEdge {
        direction: SliderDirection,
        fraction:  f32,
    },
    /// The fill spans two fractions along the directed authored axis.
    Interval {
        direction: SliderDirection,
        start:     f32,
        end:       f32,
    },
}

impl FillExtentOverride {
    pub(crate) fn is_empty(self) -> bool {
        match self {
            Self::Authored => false,
            Self::LeadingEdge { fraction, .. } => !fraction.is_finite() || fraction <= 0.0,
            Self::Interval { start, end, .. } => {
                !start.is_finite()
                    || !end.is_finite()
                    || end.clamp(0.0, 1.0) <= start.clamp(0.0, 1.0)
            },
        }
    }
}

/// The surface keeps its authored extent instead of a slider interval.
pub(crate) struct AuthoredFillGeometry;

/// Two fractions along a slider surface's directed fill axis.
pub(crate) struct DirectedFillInterval {
    pub(crate) direction:      SliderDirection,
    pub(crate) start_fraction: f32,
    pub(crate) end_fraction:   f32,
}

/// State-only presentation override for one visual slot.
///
/// `tint` replaces only an image tint. `text_color` and `path_color` replace
/// text glyphs and panel-draw primitives respectively. `fill_color` and
/// `border_color` recolor only the slot's SDF fill or border role.
/// `border_widths`
/// replaces the slot's authored SDF border widths, which grow inward from the
/// authored outer bounds and so change no geometry layout solved. `offset`
/// translates
/// the slot's SDF, image, text, and panel-line records in the panel-local
/// render frame — panel world units with Y increasing upward — while
/// preserving authored draw depth. `material`
/// replaces the SDF, text, or panel-line source material and re-keys the
/// record when the replacement changes pipeline or resource compatibility.
/// `texture` replaces an image record's sampled texture and re-keys it to
/// the destination `ImageBatchKey`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VisualSlotOverride {
    /// Replacement tint multiplied against an image's decoded texture sample.
    pub tint:          Option<Color>,
    /// Replacement color for text glyphs only.
    pub text_color:    Option<Color>,
    /// Replacement color for panel draw primitives only.
    pub path_color:    Option<Color>,
    /// Replacement color for the SDF fill role only.
    pub fill_color:    Option<Color>,
    /// Replacement color for the SDF border role only.
    pub border_color:  Option<Color>,
    /// Replacement per-side SDF border widths [top, right, bottom, left] in
    /// panel-local world units, the same frame and order
    /// `ResolvedSdfSurface::border_widths` carries.
    pub border_widths: Option<[f32; 4]>,
    /// Panel-local render-frame XY translation added to retained record
    /// transforms: panel world units with Y increasing upward, distinct from
    /// the layout-point frame (Y increasing downward) the widget slot boxes
    /// use. Produce it from a layout-frame delta with
    /// [`layout_delta_to_render_offset`].
    pub offset:        Option<Vec2>,
    /// SDF extent measured from the slider's directed leading edge.
    pub fill_extent:   FillExtentOverride,
    /// Dim the resolved fill color when the disabled appearance does not
    /// author a replacement fill.
    pub dim_fill:      bool,
    /// Dim the resolved border color when the disabled appearance does not
    /// author a replacement border.
    pub dim_border:    bool,
    /// Replacement source material for SDF, text, and panel-line records.
    pub material:      Option<Handle<StandardMaterial>>,
    /// Replacement sampled texture for image records.
    pub texture:       Option<Handle<Image>>,
}

const _: () = assert!(size_of::<VisualSlotOverride>() == 200);

impl VisualSlotOverride {
    /// Sets the visible slider fill length from its leading edge.
    pub(crate) fn slider_fill(direction: SliderDirection, fraction: f32) -> Self {
        Self {
            fill_extent: FillExtentOverride::LeadingEdge {
                direction,
                fraction,
            },
            ..Self::default()
        }
    }

    /// Sets a visible interval measured along the slider's directed axis.
    pub(crate) fn slider_interval(direction: SliderDirection, start: f32, end: f32) -> Self {
        Self {
            fill_extent: FillExtentOverride::Interval {
                direction,
                start,
                end,
            },
            ..Self::default()
        }
    }

    /// Returns the SDF extent instruction for the render route.
    pub(crate) const fn fill_interval(&self) -> Result<DirectedFillInterval, AuthoredFillGeometry> {
        match self.fill_extent {
            FillExtentOverride::Authored => Err(AuthoredFillGeometry),
            FillExtentOverride::LeadingEdge {
                direction,
                fraction,
            } => Ok(DirectedFillInterval {
                direction,
                start_fraction: 0.0,
                end_fraction: fraction,
            }),
            FillExtentOverride::Interval {
                direction,
                start,
                end,
            } => Ok(DirectedFillInterval {
                direction,
                start_fraction: start,
                end_fraction: end,
            }),
        }
    }

    pub(super) fn apply(&mut self, overlay: &Self) {
        self.tint = overlay.tint.or(self.tint);
        self.text_color = overlay.text_color.or(self.text_color);
        self.path_color = overlay.path_color.or(self.path_color);
        self.fill_color = overlay.fill_color.or(self.fill_color);
        self.border_color = overlay.border_color.or(self.border_color);
        self.border_widths = overlay.border_widths.or(self.border_widths);
        self.offset = overlay.offset.or(self.offset);
        self.dim_fill |= overlay.dim_fill;
        self.dim_border |= overlay.dim_border;
        if overlay.fill_extent != FillExtentOverride::Authored {
            self.fill_extent = overlay.fill_extent;
        }
        if overlay.material.is_some() {
            self.material.clone_from(&overlay.material);
        }
        if overlay.texture.is_some() {
            self.texture.clone_from(&overlay.texture);
        }
    }

    pub(super) fn apply_element(&mut self, overlay: &Self) {
        let offset = self.offset;
        let fill_extent = self.fill_extent;
        self.apply(overlay);
        if offset.is_some() {
            self.offset = offset;
        }
        if fill_extent != FillExtentOverride::Authored {
            self.fill_extent = fill_extent;
        }
    }

    pub(super) fn for_capabilities(mut self, capabilities: VisualElementCapabilities) -> Self {
        if !capabilities.contains(VisualElementCapabilities::SDF_FILL) {
            self.fill_color = None;
            self.fill_extent = FillExtentOverride::Authored;
            self.dim_fill = false;
        }
        if !capabilities.contains(VisualElementCapabilities::SDF_BORDER) {
            self.border_color = None;
            self.border_widths = None;
            self.dim_border = false;
        }
        if !capabilities.contains(VisualElementCapabilities::TEXT) {
            self.text_color = None;
        }
        if !capabilities.contains(VisualElementCapabilities::IMAGE) {
            self.tint = None;
        }
        if !capabilities.contains(VisualElementCapabilities::DRAW) {
            self.path_color = None;
        }
        if !capabilities.contains(VisualElementCapabilities::SDF_MATERIAL) {
            self.material = None;
        }
        self
    }
}

/// Fluent construction helpers for retained-renderer tests.
#[cfg(test)]
impl VisualSlotOverride {
    #[must_use]
    pub(crate) const fn with_fill_extent(mut self, fill_extent: FillExtentOverride) -> Self {
        self.fill_extent = fill_extent;
        self
    }

    #[must_use]
    pub(crate) const fn with_text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    #[must_use]
    pub(crate) const fn with_path_color(mut self, color: Color) -> Self {
        self.path_color = Some(color);
        self
    }

    #[must_use]
    pub(crate) const fn with_tint(mut self, color: Color) -> Self {
        self.tint = Some(color);
        self
    }

    #[must_use]
    pub(crate) const fn with_fill_color(mut self, color: Color) -> Self {
        self.fill_color = Some(color);
        self
    }

    #[must_use]
    pub(crate) const fn with_border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    #[must_use]
    pub(crate) const fn with_border_widths(mut self, widths: [f32; 4]) -> Self {
        self.border_widths = Some(widths);
        self
    }

    #[must_use]
    pub(crate) const fn with_offset(mut self, offset: Vec2) -> Self {
        self.offset = Some(offset);
        self
    }

    #[must_use]
    pub(crate) fn with_material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.material = Some(material);
        self
    }

    #[must_use]
    pub(crate) fn with_texture(mut self, texture: Handle<Image>) -> Self {
        self.texture = Some(texture);
        self
    }
}

/// Changed-only override authoring owned by one widget entity.
#[derive(Clone, Component, Debug, Default, PartialEq)]
pub(crate) struct WidgetVisualOverrides {
    slots:    Vec<(VisualSlotId, VisualSlotOverride)>,
    elements: Vec<(usize, VisualSlotOverride)>,
}

impl WidgetVisualOverrides {
    /// Returns the stored override for `slot`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn get(&self, slot: VisualSlotId) -> Option<&VisualSlotOverride> {
        self.slots
            .iter()
            .find(|(id, _)| *id == slot)
            .map(|(_, value)| value)
    }

    /// Sets or replaces the override for `slot`; an equal stored value is
    /// left untouched.
    ///
    /// The skip cannot suppress Bevy change detection on its own: reaching
    /// `set` through `Mut<WidgetVisualOverrides>` already marked the
    /// component changed, so [`dispatch_visual_overrides`] re-resolves the
    /// widget's index entries. The repeated-identical no-op guarantee lives
    /// at the retained renderer level — every route rebuilds the same record
    /// values and the batch stores compare before dirtying, so no GPU buffer
    /// re-uploads. Production writers go through [`write_widget_overrides`],
    /// which compares immutably before taking a mutable component reference,
    /// keeping unchanged frames out of `Changed<WidgetVisualOverrides>`
    /// entirely.
    pub(crate) fn set(&mut self, slot: VisualSlotId, value: VisualSlotOverride) {
        match self.slots.iter_mut().find(|(id, _)| *id == slot) {
            Some((_, current)) => {
                if *current != value {
                    *current = value;
                }
            },
            None => self.slots.push((slot, value)),
        }
    }

    /// Removes the override for `slot`, restoring authored presentation.
    #[cfg(test)]
    pub(crate) fn clear(&mut self, slot: VisualSlotId) { self.slots.retain(|(id, _)| *id != slot); }

    /// Sets or replaces an element override while preserving element-index order.
    pub(crate) fn set_element(&mut self, element_index: usize, value: VisualSlotOverride) {
        let insertion_index = self
            .elements
            .partition_point(|(existing_index, _)| *existing_index < element_index);
        match self.elements.get_mut(insertion_index) {
            Some((existing_index, current)) if *existing_index == element_index => {
                if *current != value {
                    *current = value;
                }
            },
            Some(_) | None => self
                .elements
                .insert(insertion_index, (element_index, value)),
        }
    }

    pub(super) fn slot_overrides(
        &self,
    ) -> impl Iterator<Item = (VisualSlotId, &VisualSlotOverride)> {
        self.slots.iter().map(|(slot, value)| (*slot, value))
    }

    pub(crate) fn element_overrides(&self) -> &[(usize, VisualSlotOverride)] { &self.elements }
}

/// Collects the widgets of `kind` whose presentation inputs moved this frame.
///
/// `changed` carries the entities a per-widget `Changed` filter matched and
/// `removed` the entities drained from that widget's [`RemovedComponents`]
/// streams; a removal names an entity with no live component to read, so its
/// kind comes from `kinds` instead. Both are consumed here, so a quiet frame
/// never walks the live widgets.
pub(in crate::widgets) fn dirty_widgets(
    changed: impl Iterator<Item = (Entity, WidgetKind)>,
    removed: impl Iterator<Item = Entity>,
    kinds: &Query<&WidgetKind, With<WidgetOf>>,
    kind: WidgetKind,
) -> HashSet<Entity> {
    let mut dirty: HashSet<Entity> = changed
        .filter_map(|(entity, changed_kind)| (changed_kind == kind).then_some(entity))
        .collect();
    dirty.extend(removed.filter(|&entity| kinds.get(entity) == Ok(&kind)));
    dirty
}

/// Replaces one widget's complete presentation override, taking a mutable
/// component reference only when the stored value differs from `desired`.
pub(in crate::widgets) fn write_widget_overrides(
    widget: Entity,
    desired: WidgetVisualOverrides,
    overrides: &mut Query<&mut WidgetVisualOverrides>,
    commands: &mut Commands<'_, '_>,
) {
    let Ok(current) = overrides.get(widget) else {
        if desired != WidgetVisualOverrides::default() {
            commands.entity(widget).insert(desired);
        }
        return;
    };
    if *current == desired {
        return;
    }
    if desired == WidgetVisualOverrides::default() {
        commands.entity(widget).remove::<WidgetVisualOverrides>();
        return;
    }
    let Ok(mut current) = overrides.get_mut(widget) else {
        return;
    };
    *current = desired;
}

/// Converts a widget slot's layout-frame translation delta into the
/// panel-local render frame the retained routes add to record transforms.
///
/// `layout_delta` is a delta in layout points: X increases rightward and Y
/// increases downward. The returned offset is in the panel-local render frame
/// — panel world units with X unchanged and Y increasing upward — so the
/// layout Y axis is inverted and both axes scale by `points_to_world`, the
/// owning panel's
/// [`DiegeticPanel::points_to_world`](crate::DiegeticPanel::points_to_world)
/// factor. It is the single boundary that reconciles those two frames for a
/// [`VisualSlotOverride::offset`]; returns `None` when `layout_delta` is
/// non-finite or `points_to_world` is non-finite or non-positive, so a slot
/// whose owning panel scale is unavailable writes no manufactured offset.
pub(in crate::widgets) fn layout_delta_to_render_offset(
    layout_delta: Vec2,
    points_to_world: f32,
) -> Option<Vec2> {
    if !layout_delta.is_finite() || !points_to_world.is_finite() || points_to_world <= 0.0 {
        return None;
    }
    Some(Vec2::new(
        layout_delta.x * points_to_world,
        -layout_delta.y * points_to_world,
    ))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::Vec2;

    use super::FillExtentOverride;
    use super::VisualSlotOverride;
    use crate::widgets::SliderDirection;
    use crate::widgets::VisualElementCapabilities;

    #[test]
    fn slider_fill_extent_survives_element_appearance_only_for_sdf_fills() {
        let extent = FillExtentOverride::LeadingEdge {
            direction: SliderDirection::BottomToTop,
            fraction:  0.4,
        };
        let mut override_value = VisualSlotOverride::default().with_fill_extent(extent);
        override_value.apply_element(&VisualSlotOverride::default().with_offset(Vec2::ONE));
        assert_eq!(override_value.fill_extent, extent);
        assert_eq!(
            override_value
                .clone()
                .for_capabilities(VisualElementCapabilities::SDF_FILL)
                .fill_extent,
            extent,
        );
        assert_eq!(
            override_value
                .for_capabilities(VisualElementCapabilities::TEXT)
                .fill_extent,
            FillExtentOverride::Authored,
        );
    }

    #[test]
    fn render_offset_scales_and_inverts_the_layout_y_axis() {
        // A non-unit scale multiplies both axes and the layout Y axis (down)
        // maps to the render Y axis (up), so the sign of Y flips.
        let offset = super::layout_delta_to_render_offset(Vec2::new(4.0, 6.0), 0.25)
            .expect("finite delta and positive scale convert");
        assert!((offset.x - 1.0).abs() < 1e-6, "X scales without inverting");
        assert!((offset.y + 1.5).abs() < 1e-6, "Y scales and inverts");
    }

    #[test]
    fn render_offset_rejects_invalid_input_or_scale() {
        assert_eq!(
            super::layout_delta_to_render_offset(Vec2::new(f32::NAN, 1.0), 1.0),
            None,
            "a non-finite delta manufactures no offset",
        );
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                super::layout_delta_to_render_offset(Vec2::new(1.0, 1.0), scale),
                None,
                "a non-positive or non-finite scale manufactures no offset",
            );
        }
    }
}
