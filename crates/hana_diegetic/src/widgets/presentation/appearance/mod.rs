//! Per-state appearance for widget elements.
//!
//! An element's resting appearance stays on its ordinary declarations —
//! [`El::background`](crate::El::background), [`El::border`](crate::El::border),
//! and [`El::material`](crate::El::material). A widget element adds a
//! [`StateAppearance`]: one [`Appearance`] per [`WidgetState`], naming only the
//! properties that state replaces on the widget's retained recipients.
//! [`ResolvedWidgetStateAppearances::resolve`] layers the active states in
//! [`WidgetState::LAYER_ORDER`] and returns the [`crate::widgets::VisualSlotOverride`] the
//! retained routes apply, so state presentation patches records layout already
//! emitted and never re-authors layout.

mod colors;
mod state_appearance;

use bevy::prelude::Color;
use bevy::prelude::Handle;
use bevy::prelude::Reflect;
use bevy::prelude::StandardMaterial;
pub use colors::BackgroundColor;
pub use colors::BorderColor;
pub use colors::IntoAppearance;
pub use colors::PathColor;
pub use colors::TextColor;
pub use colors::Tint;
pub(crate) use state_appearance::ResolvedWidgetStateAppearances;
pub(crate) use state_appearance::StateAppearance;
pub use state_appearance::WidgetDisabledAppearance;
pub use state_appearance::WidgetFocusedAppearance;
pub use state_appearance::WidgetHoveredAppearance;
pub use state_appearance::WidgetPressedAppearance;
pub(crate) use state_appearance::WidgetState;
pub(super) use state_appearance::WidgetStateCascades;

use crate::layout::Dimension;

/// One widget state's value for a single visual property.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub(crate) enum VisualChange<T> {
    /// The state keeps whatever value the prior layer resolved.
    #[default]
    Unchanged,
    /// The state replaces the property with this value.
    To(T),
}

impl<T> VisualChange<T> {
    /// Whether this layer authors a replacement.
    pub(crate) const fn is_authored(&self) -> bool { matches!(self, Self::To(_)) }
}

/// The visual properties a widget state replaces.
///
/// State methods on [`crate::El`] accept an `Appearance` bundle or a
/// property-specific color wrapper. Each builder below replaces values on a
/// retained record layout emits:
///
/// | Builder | Retained record |
/// | --- | --- |
/// | [`Appearance::background`] | Root SDF fill |
/// | [`Appearance::border_color`] | Root SDF border color |
/// | [`Appearance::border_width`] | Root SDF border widths |
/// | [`Appearance::text_color`] | Text glyphs |
/// | [`Appearance::path_color`] | Panel draw primitives |
/// | [`Appearance::tint`] | Image tint |
/// | [`Appearance::material`] | SDF, text, or panel-draw material |
///
/// A button and slider can author hovered, focused, pressed, and disabled
/// bundles. An editable field can author hovered, focused, and disabled
/// bundles, but has no pressed state. Each state inherits independently from
/// the lower-precedence cascade scopes before active states layer in this
/// order: focused, hovered, pressed, then disabled. A property named by a
/// later state replaces the same property from an earlier state; a
/// property the bundle does not name keeps the earlier state's result or the
/// ordinary declaration.
///
/// A state bundle only replaces values on an existing retained record, so
/// naming a property the element does not declare emits a transparent
/// stand-in to replace: a state background gets a [`Color::NONE`] fill, and a
/// state border color or width gets `Border::all(Px(0.0), Color::NONE)`.
/// A part's [`Appearance::text_color`], [`Appearance::path_color`], and
/// [`Appearance::tint`] instead require an emitted text, draw, or image
/// recipient respectively, because layout cannot synthesize any of those
/// record types.
/// Declare [`crate::El::border`] with the resting color when a state widens a
/// border that is normally invisible — a width replacement alone leaves the
/// defaulted border transparent.
///
/// # Examples
///
/// ```no_run
/// use bevy::color::Color;
/// use hana_diegetic::Appearance;
/// use hana_diegetic::BackgroundColor;
/// use hana_diegetic::Border;
/// use hana_diegetic::El;
/// use hana_diegetic::Px;
/// use hana_diegetic::Slider;
///
/// let button = El::new()
///     .background(Color::NONE)
///     .border(Border::all(Px(0.0), Color::WHITE))
///     .button("apply")
///     .hovered(BackgroundColor(Color::BLACK))
///     .focused(Appearance::new().border_width(Px(2.0)))
///     .pressed(Appearance::new().border_color(Color::WHITE))
///     .disabled(Appearance::new().material(Default::default()));
/// let slider = El::new()
///     .background(Color::NONE)
///     .border(Border::all(Px(0.0), Color::WHITE))
///     .widget("level", Slider::new(0.0..=1.0))
///     .hovered(BackgroundColor(Color::BLACK))
///     .focused(Appearance::new().border_width(Px(2.0)))
///     .pressed(Appearance::new().border_color(Color::WHITE))
///     .disabled(Appearance::new().material(Default::default()));
/// let _ = (button, slider);
/// ```
#[must_use]
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
pub struct Appearance {
    /// Replaces the element's authored background color.
    pub(crate) background:   VisualChange<Color>,
    /// Replaces the element's authored border color.
    pub(crate) border_color: VisualChange<Color>,
    /// Replaces the element's authored border width on all four sides.
    pub(crate) border_width: VisualChange<Dimension>,
    /// Replaces the element's authored text glyph color.
    pub(crate) text_color:   VisualChange<Color>,
    /// Replaces the element's authored panel-draw primitive color.
    pub(crate) path_color:   VisualChange<Color>,
    /// Replaces the element's authored image tint.
    pub(crate) tint:         VisualChange<Color>,
    /// Replaces the element's authored root material.
    pub(crate) material:     VisualChange<Handle<StandardMaterial>>,
}

impl Appearance {
    /// Creates a bundle that leaves every property unchanged.
    pub const fn new() -> Self {
        Self {
            background:   VisualChange::Unchanged,
            border_color: VisualChange::Unchanged,
            border_width: VisualChange::Unchanged,
            text_color:   VisualChange::Unchanged,
            path_color:   VisualChange::Unchanged,
            tint:         VisualChange::Unchanged,
            material:     VisualChange::Unchanged,
        }
    }

    /// Merges this bundle from a lower cascade level over `higher` property by property.
    ///
    /// Each [`VisualChange::To`] in this bundle replaces the matching property
    /// from `higher`; an [`VisualChange::Unchanged`] property keeps `higher`'s
    /// value.
    pub(super) fn merge_over(&self, higher: &Self) -> Self {
        Self {
            background:   match &self.background {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.background.clone(),
            },
            border_color: match &self.border_color {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.border_color.clone(),
            },
            border_width: match &self.border_width {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.border_width.clone(),
            },
            text_color:   match &self.text_color {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.text_color.clone(),
            },
            path_color:   match &self.path_color {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.path_color.clone(),
            },
            tint:         match &self.tint {
                VisualChange::To(value) => VisualChange::To(*value),
                VisualChange::Unchanged => higher.tint.clone(),
            },
            material:     match &self.material {
                VisualChange::To(value) => VisualChange::To(value.clone()),
                VisualChange::Unchanged => higher.material.clone(),
            },
        }
    }

    /// Replaces the root SDF fill color.
    ///
    /// Without [`crate::El::background`], layout emits a [`Color::NONE`] fill
    /// record for this value to replace.
    pub const fn background(mut self, color: Color) -> Self {
        self.background = VisualChange::To(color);
        self
    }

    /// Replaces the root SDF border color without changing its radii.
    ///
    /// Without [`crate::El::border`], layout emits a zero-width transparent
    /// border record for this value to replace.
    pub const fn border_color(mut self, color: Color) -> Self {
        self.border_color = VisualChange::To(color);
        self
    }

    /// Replaces all four root SDF border widths without changing solved layout.
    ///
    /// The retained border grows inward, leaving the element's outer bounds
    /// unchanged. Without [`crate::El::border`], layout emits a zero-width
    /// transparent border record — the widened border stays transparent, so
    /// declare the resting color there.
    pub fn border_width(mut self, width: impl Into<Dimension>) -> Self {
        self.border_width = VisualChange::To(width.into());
        self
    }

    /// Replaces the glyph color of text emitted by this element.
    ///
    /// A part that names this property must emit text itself; panel building
    /// reports an error for a structural part with no text recipient.
    pub const fn text_color(mut self, color: Color) -> Self {
        self.text_color = VisualChange::To(color);
        self
    }

    /// Replaces the color of panel draw primitives emitted by this element.
    ///
    /// A part that names this property must emit a draw itself; panel building
    /// reports an error for a structural part with no draw recipient.
    pub const fn path_color(mut self, color: Color) -> Self {
        self.path_color = VisualChange::To(color);
        self
    }

    /// Replaces the tint multiplied against an image's decoded texture sample.
    ///
    /// A part that names this property must emit an image itself; panel
    /// building reports an error for a structural part with no image recipient.
    pub const fn tint(mut self, color: Color) -> Self {
        self.tint = VisualChange::To(color);
        self
    }

    /// Replaces the material for SDF, text, and panel-draw records.
    ///
    /// The material carries its own color — the fill reads
    /// `StandardMaterial::base_color` — so without [`crate::El::background`]
    /// or [`crate::El::border`], layout emits a [`Color::NONE`] fill record
    /// for this material to re-key. Text and panel-draw recipients need no
    /// additional fill record. Repeated state transitions should use materials
    /// with matching pipeline and resource compatibility so the renderer can
    /// update material-table rows in place. An incompatible material remains
    /// supported and moves only its affected retained records to a compatible
    /// batch.
    pub fn material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.material = VisualChange::To(material);
        self
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Assets;
    use bevy::prelude::Color;
    use bevy::prelude::StandardMaterial;

    use super::Appearance;
    use crate::Px;

    const FOCUS_PATH: Color = Color::srgb(0.3, 0.5, 0.7);
    const HOVER_TINT: Color = Color::srgb(0.6, 0.4, 0.8);
    const HIGHER_BACKGROUND: Color = Color::srgb(0.1, 0.2, 0.3);
    const LOWER_BACKGROUND: Color = Color::srgb(0.3, 0.2, 0.1);
    const HIGHER_BORDER: Color = Color::srgb(0.2, 0.3, 0.4);
    const LOWER_BORDER: Color = Color::srgb(0.4, 0.3, 0.2);
    const HIGHER_PATH: Color = Color::srgb(0.3, 0.4, 0.5);
    const LOWER_PATH: Color = Color::srgb(0.5, 0.4, 0.3);
    const HIGHER_TEXT: Color = Color::srgb(0.4, 0.5, 0.6);
    const LOWER_TEXT: Color = Color::srgb(0.6, 0.5, 0.4);

    #[test]
    fn merge_over_resolves_all_properties_for_each_authorship_combination() {
        let mut materials = Assets::<StandardMaterial>::default();
        let higher_material = materials.add(StandardMaterial::from(Color::WHITE));
        let lower_material = materials.add(StandardMaterial::from(Color::BLACK));
        let higher = Appearance::new()
            .background(HIGHER_BACKGROUND)
            .border_color(HIGHER_BORDER)
            .border_width(Px(1.0))
            .text_color(HIGHER_TEXT)
            .path_color(HIGHER_PATH)
            .tint(HOVER_TINT)
            .material(higher_material);
        let lower = Appearance::new()
            .background(LOWER_BACKGROUND)
            .border_color(LOWER_BORDER)
            .border_width(Px(2.0))
            .text_color(LOWER_TEXT)
            .path_color(LOWER_PATH)
            .tint(FOCUS_PATH)
            .material(lower_material);
        let empty = Appearance::new();

        assert_eq!(empty.merge_over(&empty), empty);
        assert_eq!(empty.merge_over(&higher), higher);
        assert_eq!(lower.merge_over(&empty), lower);
        assert_eq!(lower.merge_over(&higher), lower);
    }

    #[test]
    fn merge_over_preserves_properties_from_each_authored_level() {
        let global = Appearance::new().background(HIGHER_BACKGROUND);
        let panel = Appearance::new().text_color(HIGHER_TEXT);
        let widget = Appearance::new().background(LOWER_BACKGROUND);

        let resolved = widget.merge_over(&panel).merge_over(&global);

        assert_eq!(
            resolved,
            Appearance::new()
                .background(LOWER_BACKGROUND)
                .text_color(HIGHER_TEXT),
        );
    }
}
