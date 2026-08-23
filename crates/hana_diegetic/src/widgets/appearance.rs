//! Per-state appearance for widget elements.
//!
//! An element's resting appearance stays on its ordinary declarations —
//! [`El::background`](crate::El::background), [`El::border`](crate::El::border),
//! and [`El::material`](crate::El::material). A widget element adds a
//! [`StateAppearance`]: one [`Appearance`] per [`WidgetState`], naming only the
//! properties that state replaces on the widget's retained recipients.
//! [`ResolvedWidgetStateAppearances::resolve`] layers the active states in
//! [`WidgetState::LAYER_ORDER`] and returns the [`VisualSlotOverride`] the
//! retained routes apply, so state presentation patches records layout already
//! emitted and never re-authors layout.

use core::mem::size_of;
use std::sync::Arc;
use std::sync::LazyLock;

use bevy::prelude::Color;
use bevy::prelude::Handle;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use hana_kana::CascadeRootResource;

use super::VisualSlotOverride;
use crate::DiegeticPanel;
use crate::cascade::CASCADE_ATTRIBUTE_BYTES;
use crate::cascade::Cascade;
use crate::cascade::CascadeRoot;
use crate::layout::Dimension;

/// One widget state's decision for a single visual property.
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

/// A color that replaces a state appearance's background property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for BackgroundColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's border property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for BorderColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's text property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for TextColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's panel-draw property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathColor(
    /// The replacement color.
    pub Color,
);

impl From<Color> for PathColor {
    fn from(color: Color) -> Self { Self(color) }
}

/// A color that replaces a state appearance's image tint property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint(
    /// The replacement color.
    pub Color,
);

impl From<Color> for Tint {
    fn from(color: Color) -> Self { Self(color) }
}

/// Converts a state appearance bundle or property-specific color into an [`Appearance`].
#[diagnostic::on_unimplemented(
    message = "a bare `Color` does not say which property it sets",
    label = "wrap it: `BackgroundColor({Self})`, `TextColor({Self})`, `BorderColor({Self})`, `PathColor({Self})`, or `Tint({Self})`"
)]
pub trait IntoAppearance {
    /// Returns the corresponding state appearance bundle.
    fn into_appearance(self) -> Appearance;
}

impl IntoAppearance for Appearance {
    fn into_appearance(self) -> Appearance { self }
}

impl IntoAppearance for BackgroundColor {
    fn into_appearance(self) -> Appearance { Appearance::new().background(self.0) }
}

impl IntoAppearance for BorderColor {
    fn into_appearance(self) -> Appearance { Appearance::new().border_color(self.0) }
}

impl IntoAppearance for TextColor {
    fn into_appearance(self) -> Appearance { Appearance::new().text_color(self.0) }
}

impl IntoAppearance for PathColor {
    fn into_appearance(self) -> Appearance { Appearance::new().path_color(self.0) }
}

impl IntoAppearance for Tint {
    fn into_appearance(self) -> Appearance { Appearance::new().tint(self.0) }
}

static EMPTY_APPEARANCE: LazyLock<Arc<Appearance>> = LazyLock::new(|| Arc::new(Appearance::new()));

/// Hovered-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::hovered`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the hovered appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetHoveredAppearance(Arc<Appearance>);

impl WidgetHoveredAppearance {
    /// Wraps one appearance bundle as the hovered-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this hovered-state bundle.
    pub(super) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetHoveredAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetHoveredAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetHoveredAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetHoveredAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Pressed-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::pressed`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the pressed appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetPressedAppearance(Arc<Appearance>);

impl WidgetPressedAppearance {
    /// Wraps one appearance bundle as the pressed-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this pressed-state bundle.
    pub(super) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetPressedAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetPressedAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetPressedAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetPressedAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Focused-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::focused`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the focused appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetFocusedAppearance(Arc<Appearance>);

impl WidgetFocusedAppearance {
    /// Wraps one appearance bundle as the focused-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this focused-state bundle.
    pub(super) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetFocusedAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetFocusedAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetFocusedAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetFocusedAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Disabled-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::disabled`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the disabled appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetDisabledAppearance(Arc<Appearance>);

impl WidgetDisabledAppearance {
    /// Wraps one appearance bundle as the disabled-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this disabled-state bundle.
    pub(super) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetDisabledAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetDisabledAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetDisabledAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetDisabledAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Authored per-state [`Appearance`] bundles held on a layout element and in
/// [`ComputedWidgetRecord`](super::ComputedWidgetRecord).
///
/// The four channels reach the widget entity as separate
/// `Cascade<WidgetHoveredAppearance>`, `Cascade<WidgetPressedAppearance>`,
/// `Cascade<WidgetFocusedAppearance>`, and `Cascade<WidgetDisabledAppearance>`
/// components.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StateAppearance {
    pub(crate) hovered:  Cascade<WidgetHoveredAppearance>,
    pub(crate) pressed:  Cascade<WidgetPressedAppearance>,
    pub(crate) focused:  Cascade<WidgetFocusedAppearance>,
    pub(crate) disabled: Cascade<WidgetDisabledAppearance>,
}

impl StateAppearance {
    /// Borrows this appearance's four independently cascaded state layers.
    pub(crate) const fn cascades(&self) -> WidgetStateCascades<'_> {
        WidgetStateCascades::new(&self.hovered, &self.pressed, &self.focused, &self.disabled)
    }
}

/// Borrowed state-appearance cascades read from a widget entity or layout element.
pub(crate) struct WidgetStateCascades<'a> {
    hovered:  &'a Cascade<WidgetHoveredAppearance>,
    pressed:  &'a Cascade<WidgetPressedAppearance>,
    focused:  &'a Cascade<WidgetFocusedAppearance>,
    disabled: &'a Cascade<WidgetDisabledAppearance>,
}

impl<'a> WidgetStateCascades<'a> {
    /// Borrows the four independently cascaded widget-state appearance layers.
    const fn new(
        hovered: &'a Cascade<WidgetHoveredAppearance>,
        pressed: &'a Cascade<WidgetPressedAppearance>,
        focused: &'a Cascade<WidgetFocusedAppearance>,
        disabled: &'a Cascade<WidgetDisabledAppearance>,
    ) -> Self {
        Self {
            hovered,
            pressed,
            focused,
            disabled,
        }
    }

    /// Whether any state channel explicitly uses [`Cascade::Override`].
    pub(crate) const fn any_overridden(&self) -> bool {
        self.hovered.as_override().is_some()
            || self.pressed.as_override().is_some()
            || self.focused.as_override().is_some()
            || self.disabled.as_override().is_some()
    }

    /// Borrows this state channel's authored override, if it has one.
    pub(super) fn layer(&self, state: WidgetState) -> Option<&Appearance> {
        match state {
            WidgetState::Focused => self
                .focused
                .as_override()
                .map(WidgetFocusedAppearance::appearance),
            WidgetState::Hovered => self
                .hovered
                .as_override()
                .map(WidgetHoveredAppearance::appearance),
            WidgetState::Pressed => self
                .pressed
                .as_override()
                .map(WidgetPressedAppearance::appearance),
            WidgetState::Disabled => self
                .disabled
                .as_override()
                .map(WidgetDisabledAppearance::appearance),
        }
    }

    /// Whether any state layer authors the property `authored` selects.
    pub(crate) fn any(&self, authored: impl Fn(&Appearance) -> bool) -> bool {
        WidgetState::LAYER_ORDER
            .into_iter()
            .any(|state| self.layer(state).is_some_and(&authored))
    }

    /// Composes authored active-state layers for focused unit tests.
    #[cfg(test)]
    pub(crate) fn resolve(
        &self,
        active: &[Option<WidgetState>],
        panel: Option<&DiegeticPanel>,
    ) -> VisualSlotOverride {
        resolve_active_layers(active, panel, |state| self.layer(state))
    }
}

/// Borrowed resolved appearance bundles for the four widget states.
pub(crate) struct ResolvedWidgetStateAppearances<'a> {
    hovered:  &'a Appearance,
    pressed:  &'a Appearance,
    focused:  &'a Appearance,
    disabled: &'a Appearance,
}

impl<'a> ResolvedWidgetStateAppearances<'a> {
    /// Borrows each state bundle after global, panel, and widget cascade resolution.
    pub(crate) const fn new(
        hovered: &'a Appearance,
        pressed: &'a Appearance,
        focused: &'a Appearance,
        disabled: &'a Appearance,
    ) -> Self {
        Self {
            hovered,
            pressed,
            focused,
            disabled,
        }
    }

    /// Borrows one resolved state bundle.
    pub(crate) const fn layer(&self, state: WidgetState) -> &'a Appearance {
        match state {
            WidgetState::Focused => self.focused,
            WidgetState::Hovered => self.hovered,
            WidgetState::Pressed => self.pressed,
            WidgetState::Disabled => self.disabled,
        }
    }

    /// Composes the resolved active-state layers into one record override.
    pub(crate) fn resolve(
        &self,
        active: &[Option<WidgetState>],
        panel: Option<&DiegeticPanel>,
    ) -> VisualSlotOverride {
        resolve_active_layers(active, panel, |state| Some(self.layer(state)))
    }
}

fn resolve_active_layers<'a>(
    active: &[Option<WidgetState>],
    panel: Option<&DiegeticPanel>,
    layer: impl Fn(WidgetState) -> Option<&'a Appearance>,
) -> VisualSlotOverride {
    let mut background = None;
    let mut border_color = None;
    let mut border_width = None;
    let mut text_color = None;
    let mut path_color = None;
    let mut tint = None;
    let mut material = None;
    for state in WidgetState::LAYER_ORDER {
        if active.contains(&Some(state))
            && let Some(layer) = layer(state)
        {
            if let VisualChange::To(value) = &layer.background {
                background = Some(value);
            }
            if let VisualChange::To(value) = &layer.border_color {
                border_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.border_width {
                border_width = Some(value);
            }
            if let VisualChange::To(value) = &layer.text_color {
                text_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.path_color {
                path_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.tint {
                tint = Some(value);
            }
            if let VisualChange::To(value) = &layer.material {
                material = Some(value);
            }
        }
    }
    let border_widths = border_width
        .zip(panel)
        .and_then(|(width, panel)| render_border_widths(*width, panel));
    VisualSlotOverride {
        fill_color: background.copied(),
        border_color: border_color.copied(),
        border_widths,
        text_color: text_color.copied(),
        path_color: path_color.copied(),
        tint: tint.copied(),
        material: material.cloned(),
        ..VisualSlotOverride::default()
    }
}

/// One interaction state a widget element authors an [`Appearance`] for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WidgetState {
    /// The widget's keyboard focus indicator is visible.
    Focused,
    /// A pointer is over the widget.
    Hovered,
    /// The widget is held: a button press or a slider drag.
    Pressed,
    /// The widget refuses interaction.
    Disabled,
}

impl WidgetState {
    /// Layering order for [`ResolvedWidgetStateAppearances::resolve`]: a later state replaces
    /// the properties it authors over an earlier one.
    pub(crate) const LAYER_ORDER: [Self; 4] =
        [Self::Focused, Self::Hovered, Self::Pressed, Self::Disabled];
}

/// Converts an authored state border width into the panel-local world widths
/// retained SDF records carry.
///
/// Mirrors the authored border path: the dimension resolves to layout points
/// against the panel's layout unit, then scales by the panel's points-to-world
/// factor. Returns `None` for a negative or non-finite width, or a non-finite
/// or non-positive panel scale, so an unresolvable width leaves the authored
/// widths in place instead of writing a manufactured one.
fn render_border_widths(width: Dimension, panel: &DiegeticPanel) -> Option<[f32; 4]> {
    let points = width.to_points(panel.layout_unit().to_points());
    let points_to_world = panel.points_to_world();
    if !points.is_finite() || points < 0.0 || !points_to_world.is_finite() || points_to_world <= 0.0
    {
        return None;
    }
    Some([points * points_to_world; 4])
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Assets;
    use bevy::prelude::Color;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::StandardMaterial;

    use super::Appearance;
    use super::BackgroundColor;
    use super::BorderColor;
    use super::PathColor;
    use super::StateAppearance;
    use super::TextColor;
    use super::Tint;
    use super::VisualChange;
    use super::WidgetDisabledAppearance;
    use super::WidgetFocusedAppearance;
    use super::WidgetHoveredAppearance;
    use super::WidgetPressedAppearance;
    use super::WidgetState;
    use crate::DiegeticPanel;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::cascade::Cascade;
    use crate::cascade::Resolved;
    use crate::widgets::WidgetsPlugin;

    const HOVER_FILL: Color = Color::srgb(0.1, 0.2, 0.3);
    const FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const PRESS_FILL: Color = Color::srgb(0.4, 0.5, 0.6);
    const HOVER_TEXT: Color = Color::srgb(0.7, 0.5, 0.3);
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
    const DISABLED_BACKGROUND: Color = Color::srgb(0.2, 0.2, 0.2);
    const DISABLED_BORDER: Color = Color::srgb(0.8, 0.1, 0.1);

    macro_rules! assert_resolved_appearance {
        ($app:expr, $widget:expr, $attribute:ty, $expected:expr $(,)?) => {{
            let expected = $expected;
            assert_eq!(
                $app.world()
                    .get::<Resolved<$attribute>>($widget)
                    .map(|resolved| resolved.0.appearance()),
                Some(&expected),
            );
        }};
    }

    fn cascade_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn two_widget_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("first"), |_| {});
        builder.with(El::new().button("second"), |_| {});
        builder.build()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        let Some(widget) = widget else {
            return Entity::PLACEHOLDER;
        };
        widget
    }

    fn panel() -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .build()
            .expect("a sized world panel builds")
    }

    fn state_appearance() -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().background(HOVER_FILL),
            )),
            pressed: Cascade::Override(WidgetPressedAppearance::new(
                Appearance::new().background(PRESS_FILL),
            )),
            focused: Cascade::Override(WidgetFocusedAppearance::new(
                Appearance::new()
                    .border_color(FOCUS_BORDER)
                    .border_width(Px(2.0)),
            )),
            ..StateAppearance::default()
        }
    }

    #[test]
    fn background_color_sets_only_the_background_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("background")
                .hovered(BackgroundColor(HOVER_FILL)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "background");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::To(HOVER_FILL),
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn border_color_sets_only_the_border_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("border")
                .hovered(BorderColor(FOCUS_BORDER)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "border");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::To(FOCUS_BORDER),
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn text_color_sets_only_the_text_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button("text").hovered(TextColor(HOVER_TEXT)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "text");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::To(HOVER_TEXT),
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn path_color_sets_only_the_panel_draw_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button("path").hovered(PathColor(FOCUS_PATH)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "path");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::To(FOCUS_PATH),
                tint:         VisualChange::Unchanged,
                material:     VisualChange::Unchanged,
            },
        );
    }

    #[test]
    fn tint_sets_only_the_image_appearance() {
        let mut app = cascade_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("tint").hovered(Tint(HOVER_TINT)), |_| {});
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(builder.build())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "tint");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance {
                background:   VisualChange::Unchanged,
                border_color: VisualChange::Unchanged,
                border_width: VisualChange::Unchanged,
                text_color:   VisualChange::Unchanged,
                path_color:   VisualChange::Unchanged,
                tint:         VisualChange::To(HOVER_TINT),
                material:     VisualChange::Unchanged,
            },
        );
    }

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

    #[test]
    fn global_state_appearance_defaults_reach_every_widget_without_state_authoring() {
        let mut app = cascade_test_app();
        let hovered = Appearance::new().background(HOVER_FILL);
        let pressed = Appearance::new().background(PRESS_FILL);
        let focused = Appearance::new().border_color(FOCUS_BORDER);
        let disabled = Appearance::new().background(DISABLED_BACKGROUND);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(hovered.clone());
        *app.world_mut().resource_mut::<WidgetPressedAppearance>() =
            WidgetPressedAppearance::new(pressed.clone());
        *app.world_mut().resource_mut::<WidgetFocusedAppearance>() =
            WidgetFocusedAppearance::new(focused.clone());
        *app.world_mut().resource_mut::<WidgetDisabledAppearance>() =
            WidgetDisabledAppearance::new(disabled.clone());
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        for id in ["first", "second"] {
            let widget = resolve_widget(&mut app, panel, id);
            assert_resolved_appearance!(&app, widget, WidgetHoveredAppearance, hovered.clone());
            assert_resolved_appearance!(&app, widget, WidgetPressedAppearance, pressed.clone());
            assert_resolved_appearance!(&app, widget, WidgetFocusedAppearance, focused.clone());
            assert_resolved_appearance!(&app, widget, WidgetDisabledAppearance, disabled.clone());
        }
    }

    #[test]
    fn panel_state_appearances_merge_with_globals_in_the_reification_frame() {
        let mut app = cascade_test_app();
        let global = Appearance::new()
            .background(HIGHER_BACKGROUND)
            .text_color(HIGHER_TEXT);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetPressedAppearance>() =
            WidgetPressedAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetFocusedAppearance>() =
            WidgetFocusedAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetDisabledAppearance>() =
            WidgetDisabledAppearance::new(global);
        let hovered = Appearance::new()
            .background(HOVER_FILL)
            .border_color(HIGHER_BORDER);
        let pressed = Appearance::new()
            .background(PRESS_FILL)
            .border_color(LOWER_BORDER);
        let focused = Appearance::new()
            .background(LOWER_BACKGROUND)
            .border_color(FOCUS_BORDER);
        let disabled = Appearance::new()
            .background(DISABLED_BACKGROUND)
            .border_color(DISABLED_BORDER);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(hovered)
            .widget_pressed_appearance(pressed)
            .widget_focused_appearance(focused)
            .widget_disabled_appearance(disabled)
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "first");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance::new()
                .background(HOVER_FILL)
                .border_color(HIGHER_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetPressedAppearance,
            Appearance::new()
                .background(PRESS_FILL)
                .border_color(LOWER_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetFocusedAppearance,
            Appearance::new()
                .background(LOWER_BACKGROUND)
                .border_color(FOCUS_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetDisabledAppearance,
            Appearance::new()
                .background(DISABLED_BACKGROUND)
                .border_color(DISABLED_BORDER)
                .text_color(HIGHER_TEXT),
        );
    }

    #[test]
    fn panel_hovered_appearance_preserves_global_properties_through_the_cascade() {
        let mut app = cascade_test_app();
        let global = Appearance::new()
            .background(HIGHER_BACKGROUND)
            .text_color(HIGHER_TEXT);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(global);
        let hovered = Appearance::new().border_color(HIGHER_BORDER);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(hovered)
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "first");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance::new()
                .background(HIGHER_BACKGROUND)
                .border_color(HIGHER_BORDER)
                .text_color(HIGHER_TEXT),
        );
    }

    #[test]
    fn unchanged_state_appearance_propagation_does_not_dirty_resolved_values() {
        let mut app = cascade_test_app();
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(HOVER_FILL));
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "first");
        let before = (
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetHoveredAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetPressedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetFocusedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetDisabledAppearance>>()
                .map(|resolved| resolved.last_changed()),
        );

        app.update();

        let after = (
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetHoveredAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetPressedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetFocusedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetDisabledAppearance>>()
                .map(|resolved| resolved.last_changed()),
        );
        assert_eq!(after, before);
    }

    #[test]
    fn properties_layer_independently() {
        // Focus authors only the border and hover only the fill, so both
        // survive when the two states are live together.
        let state_appearance = state_appearance();
        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Focused), Some(WidgetState::Hovered)],
            None,
        );
        assert_eq!(resolved.fill_color, Some(HOVER_FILL));
        assert_eq!(resolved.border_color, Some(FOCUS_BORDER));
    }

    #[test]
    fn later_layers_replace_earlier_ones_regardless_of_active_order() {
        // Pressed sits after hovered in the layer order, so it wins the fill
        // even when the caller lists hovered last.
        let state_appearance = state_appearance();
        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Pressed), Some(WidgetState::Hovered)],
            None,
        );
        assert_eq!(resolved.fill_color, Some(PRESS_FILL));
    }

    #[test]
    fn text_and_path_colors_layer_independently() {
        let state_appearance = StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().text_color(HOVER_TEXT),
            )),
            focused: Cascade::Override(WidgetFocusedAppearance::new(
                Appearance::new().path_color(FOCUS_PATH),
            )),
            ..StateAppearance::default()
        };

        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Focused), Some(WidgetState::Hovered)],
            None,
        );

        assert_eq!(resolved.text_color, Some(HOVER_TEXT));
        assert_eq!(resolved.path_color, Some(FOCUS_PATH));
        assert_eq!(resolved.fill_color, None);
        assert_eq!(resolved.border_color, None);
    }

    #[test]
    fn inactive_states_author_nothing() {
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[None, None, None, None], None);
        assert_eq!(resolved.fill_color, None);
        assert_eq!(resolved.border_color, None);
        assert_eq!(resolved.border_widths, None);
    }

    #[test]
    fn border_width_resolves_through_the_owning_panel_scale() {
        let panel = panel();
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[Some(WidgetState::Focused)], Some(&panel));
        let widths = resolved
            .border_widths
            .expect("a focused width and a live panel resolve to render widths");
        // Two logical pixels resolve to points through the fixed 96 DPI
        // convention, then scale to panel world units.
        let expected = 2.0 * 0.75 * panel.points_to_world();
        for width in widths {
            assert!(
                (width - expected).abs() < 1e-6,
                "each side carries the same resolved width",
            );
        }
    }

    #[test]
    fn border_width_without_a_panel_leaves_the_authored_width() {
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[Some(WidgetState::Focused)], None);
        assert_eq!(resolved.border_widths, None);
        assert_eq!(
            resolved.border_color,
            Some(FOCUS_BORDER),
            "a missing panel must not suppress the other properties",
        );
    }
}
