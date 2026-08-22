//! Ergonomic builder for constructing layout trees.
//!
//! [`El`] is a lightweight builder that mirrors every layout property on
//! [`Element`](super::element::Element) but exposes them as a fluent chain. When added to the
//! tree, `El` converts itself into an `Element` via `into_element()` — it exists purely for
//! ergonomics so users never have to construct `Element` or `ElementContent` by hand.
//!
//! [`LayoutBuilder`] manages parent-child nesting with an internal stack. Calling
//! `.with(el, |b| { ... })` pushes a parent, runs the closure, and pops — so there are no
//! open/close pairs to get wrong.
//!
//! The closure-based nesting API is inspired by Clay's C API:
//!
//! ```ignore
//! let tree = LayoutBuilder::new(160.0, 160.0)
//!     .with(El::column().width(Sizing::GROW).height(Sizing::GROW).padding(Padding::all(8.0))
//!           .background(Color::srgb_u8(180, 96, 122)),
//!         |b| {
//!             b.text(("STATUS", TextStyle::new(7.0)));
//!             b.with(El::new().width(Sizing::GROW).height(Sizing::fixed(4.0))
//!                    .background(Color::srgb_u8(74, 196, 172)),
//!                 |_| {},
//!             );
//!         },
//!     )
//!     .build();
//! ```

use std::marker::PhantomData;
use std::ops::RangeInclusive;

use bevy::asset::Handle;
use bevy::color::Color;
use bevy::ecs::system::In;
use bevy::ecs::system::IntoSystem;
use bevy::image::Image;
use bevy::math::Vec2;
use bevy::pbr::StandardMaterial;

use super::AlignX;
use super::AlignY;
use super::Border;
use super::ChildDivider;
use super::CornerRadius;
use super::Dimension;
use super::DrawZIndex;
use super::Padding;
use super::PanelDraw;
use super::Px;
use super::ShadowCasting;
use super::Sizing;
use super::TextStyle;
use super::TextWrap;
use super::child_layout::ChildLayout;
use super::element::ChildOverflow;
use super::element::EditorElementOrigin;
use super::element::Element;
use super::element::ElementContent;
use super::element::LayoutTree;
use super::element::PrecomposeMode;
use super::element::ScrollAnchor;
use crate::DimensionMatch;
use crate::ImeEditableFieldSpec;
use crate::ImePanelField;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::render::AntiAlias;
use crate::render::HairlineFade;
use crate::widgets::Appearance;
use crate::widgets::Button;
use crate::widgets::ButtonClicked;
use crate::widgets::IntoAppearance;
use crate::widgets::Slider;
use crate::widgets::SliderDirection;
use crate::widgets::SliderResetBehavior;
use crate::widgets::StateAppearance;
use crate::widgets::Tooltip;
use crate::widgets::VisualSlotId;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetInteractivity;
use crate::widgets::WidgetPressedAppearance;
use crate::widgets::WidgetSpec;

/// Shorthand element declaration for the builder API.
///
/// This is a temporary configuration object that gets converted into an `Element`
/// when added to the tree.
#[must_use]
#[derive(Clone, Debug)]
pub struct El<L = Row, Role = LayoutOnly> {
    common:       CommonEl,
    child_layout: L,
    role:         PhantomData<fn() -> Role>,
}

/// Marker for an ordinary visual layout element.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutOnly;

/// An element declared through a widget's child builder.
#[derive(Clone, Copy, Debug)]
pub struct WidgetChild(());

/// Marker for an element that declares a panel widget of kind `W`.
#[derive(Clone, Copy, Debug)]
pub struct WidgetElement<W>(PhantomData<fn() -> W>);

/// An element inside a widget's children that authored a state look.
#[derive(Clone, Copy, Debug)]
pub struct WidgetPart(());

/// A widget part that authored a pressed state look.
#[derive(Clone, Copy, Debug)]
pub struct PressedPart(());

/// State colors for generated editable-field parts.
///
/// Each setter replaces the complete appearance bundle for its state. Start
/// with [`Self::new`], then set [`Self::focused`], [`Self::hovered`],
/// [`Self::disabled`], or [`Self::pressed`] in any order.
#[must_use]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorStateColors {
    hovered:  Option<Color>,
    focused:  Option<Color>,
    disabled: Option<Color>,
}

/// State colors that include an unsupported pressed state for an editable part.
///
/// [`EditorStateColors::pressed`] returns this distinct role so
/// [`El::editor_text`], [`El::editor_selection`], [`El::editor_caret`], and
/// [`El::editor_validation`] reject it at compile time.
#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub struct PressedEditorStateColors {
    colors:  EditorStateColors,
    pressed: Color,
}

impl EditorStateColors {
    /// Creates a colors declaration with no authored state colors.
    pub const fn new() -> Self {
        Self {
            hovered:  None,
            focused:  None,
            disabled: None,
        }
    }

    /// Sets the focused-state color.
    pub const fn focused(mut self, color: Color) -> Self {
        self.focused = Some(color);
        self
    }

    /// Sets a pressed-state color that editable parts reject.
    pub const fn pressed(self, color: Color) -> PressedEditorStateColors {
        PressedEditorStateColors {
            colors:  self,
            pressed: color,
        }
    }

    /// Sets the hovered-state color.
    pub const fn hovered(mut self, color: Color) -> Self {
        self.hovered = Some(color);
        self
    }

    /// Sets the disabled-state color.
    pub const fn disabled(mut self, color: Color) -> Self {
        self.disabled = Some(color);
        self
    }

    fn into_editor_part(self, role: EditorPartColorRole) -> EditorPart {
        let appearance = StateAppearance {
            hovered:  self.hovered.map_or(Cascade::Inherit, |color| {
                Cascade::Override(WidgetHoveredAppearance::new(role.appearance(color)))
            }),
            pressed:  Cascade::Inherit,
            focused:  self.focused.map_or(Cascade::Inherit, |color| {
                Cascade::Override(WidgetFocusedAppearance::new(role.appearance(color)))
            }),
            disabled: self.disabled.map_or(Cascade::Inherit, |color| {
                Cascade::Override(WidgetDisabledAppearance::new(role.appearance(color)))
            }),
        };
        let mut part = El::new();
        part.common.appearance = Some(Box::new(appearance));
        EditorPart::from_el(part.into_role())
    }
}

impl PressedEditorStateColors {
    /// Sets the hovered-state color.
    pub const fn hovered(mut self, color: Color) -> Self {
        self.colors.hovered = Some(color);
        self
    }

    /// Sets the focused-state color.
    pub const fn focused(mut self, color: Color) -> Self {
        self.colors.focused = Some(color);
        self
    }

    /// Sets the disabled-state color.
    pub const fn disabled(mut self, color: Color) -> Self {
        self.colors.disabled = Some(color);
        self
    }

    /// Replaces the pressed-state color.
    pub const fn pressed(mut self, color: Color) -> Self {
        self.pressed = color;
        self
    }
}

#[derive(Clone, Copy)]
enum EditorPartColorRole {
    Fill,
    Text,
}

impl EditorPartColorRole {
    const fn appearance(self, color: Color) -> Appearance {
        match self {
            Self::Fill => Appearance::new().background(color),
            Self::Text => Appearance::new().text_color(color),
        }
    }
}

/// Public marker trait for element roles accepted by ordinary panel layout.
pub trait ElementRole: private::RoleSealed {}

impl ElementRole for LayoutOnly {}

impl ElementRole for WidgetChild {}

impl<W> ElementRole for WidgetElement<W> {}

impl ElementRole for WidgetPart {}

impl ElementRole for PressedPart {}

/// A widget kind that owns a scoped widget-content builder.
///
/// Button, slider, and editable-field roots own the scope their descendants
/// use to author widget parts. This trait is sealed because each owner maps to
/// crate-private widget storage.
pub trait WidgetOwner: private::WidgetOwnerSealed {}

/// Marker for the editable-field widget owner.
#[derive(Clone, Copy, Debug)]
pub struct EditableField(());

impl WidgetOwner for Button {}

impl WidgetOwner for Slider {}

impl WidgetOwner for EditableField {}

/// A pre-built widget declaration an element can adopt through [`El::widget`].
///
/// Implemented for [`Button`] and [`Slider`]. The trait is sealed: the element
/// contract a widget converts into is private to this crate.
pub trait Widget: WidgetOwner + private::WidgetSealed {
    /// Converts this declaration into the opaque contract an element stores.
    #[doc(hidden)]
    fn into_declaration(self) -> WidgetDeclaration;

    /// The opaque root visual slot this widget's element records carry.
    #[doc(hidden)]
    fn root_visual_slot() -> WidgetRootSlot;
}

/// Opaque element-side form of a widget declaration.
///
/// Produced by [`Widget::into_declaration`] and consumed by [`El::widget`];
/// it carries no public structure.
#[derive(Clone, Debug)]
pub struct WidgetDeclaration(WidgetSpec);

/// Opaque identity of a widget's root visual slot.
///
/// Produced by [`Widget::root_visual_slot`] and consumed by [`El::widget`].
#[derive(Clone, Copy, Debug)]
pub struct WidgetRootSlot(VisualSlotId);

impl Widget for Button {
    fn into_declaration(self) -> WidgetDeclaration { WidgetDeclaration(WidgetSpec::Button(self)) }

    fn root_visual_slot() -> WidgetRootSlot { WidgetRootSlot(VisualSlotId::BUTTON_ROOT) }
}

impl Widget for Slider {
    fn into_declaration(self) -> WidgetDeclaration { WidgetDeclaration(WidgetSpec::Slider(self)) }

    fn root_visual_slot() -> WidgetRootSlot { WidgetRootSlot(VisualSlotId::SLIDER_ROOT) }
}

/// A widget kind that can be held — a button press or a slider drag.
///
/// A widget root exposes [`El::pressed`] only when its widget kind implements
/// this trait. Parts also expose [`El::pressed`], but [`WidgetBuilder::with`]
/// rejects a pressed part when the enclosing widget kind is not pressable.
/// Widgets that are never held have no pressed state to author: an input text
/// box takes a caret and keystrokes, a read-only value readout is not grabbed
/// at all, and a scrolling log is driven by its content rather than by a pointer
/// hold. Those kinds still reach hover, focus, and disabled, and author only
/// those layers; an attempted pressed layer is a compile error rather than a
/// silent no-op.
pub trait Pressable: Widget {}

impl Pressable for Button {}

impl Pressable for Slider {}

/// Text sizing and wrapping policy for a layout text leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSizing {
    /// Measure the visible text naturally, optionally wrapping it.
    Natural {
        /// Wrapping policy for the visible text.
        wrap: TextWrap,
    },
    /// Measure this surrogate string while rendering the visible text as one line.
    MeasureAs {
        /// Surrogate string used for measurement.
        text: String,
    },
    /// Reserve the measured width of this surrogate string, then wrap visible text to it.
    WrapAtMeasure {
        /// Surrogate string whose measured width becomes the wrap width.
        text: String,
    },
}

impl Default for TextSizing {
    fn default() -> Self {
        Self::Natural {
            wrap: TextWrap::Words,
        }
    }
}

impl TextSizing {
    /// Creates natural text sizing with the requested wrapping mode.
    #[must_use]
    pub const fn wrap(wrap: TextWrap) -> Self { Self::Natural { wrap } }

    /// Creates sizing that measures a surrogate string instead of the visible text.
    #[must_use]
    pub fn measure_as(text: impl Into<String>) -> Self { Self::MeasureAs { text: text.into() } }

    /// Creates sizing that wraps visible text at the surrogate string's measured width.
    #[must_use]
    pub fn wrap_at_measure(text: impl Into<String>) -> Self {
        Self::WrapAtMeasure { text: text.into() }
    }

    pub(super) fn measure_text<'a>(&'a self, visible_text: &'a str) -> &'a str {
        match self {
            Self::Natural { .. } => visible_text,
            Self::MeasureAs { text } | Self::WrapAtMeasure { text } => text,
        }
    }

    pub(super) const fn visible_text_affects_layout(&self) -> bool {
        match self {
            Self::Natural { .. } | Self::WrapAtMeasure { .. } => true,
            Self::MeasureAs { .. } => false,
        }
    }
}

/// Text leaf declaration for [`LayoutBuilder::text`].
#[must_use]
#[derive(Clone, Debug)]
pub struct Text<Role = LayoutOnly> {
    layout:  CommonEl,
    content: String,
    style:   TextStyle,
    sizing:  TextSizing,
    role:    PhantomData<fn() -> Role>,
}

impl Text<LayoutOnly> {
    /// Creates a text declaration with visible text and style.
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            layout: CommonEl::default(),
            content: text.into(),
            style,
            sizing: TextSizing::default(),
            role: PhantomData,
        }
    }
}

impl<Role> Text<Role> {
    /// Marks this text leaf as an inline-editor generated part.
    pub(crate) const fn generated_editor_part(mut self) -> Self {
        self.layout.editor_origin = EditorElementOrigin::Generated;
        self
    }

    /// Assigns a panel-local id so this run can be addressed at runtime.
    pub fn id(mut self, id: impl Into<PanelElementId>) -> Self {
        self.layout.id = Some(id.into());
        self
    }

    /// Renders this text leaf into an LDR image, then draws that image in the
    /// parent panel.
    ///
    /// Use this when text should keep SDR edge behavior under an HDR scene
    /// camera, while surrounding panel backgrounds and borders stay on the
    /// normal analytic path.
    pub const fn precompose_ldr(mut self) -> Self {
        self.layout.precompose = PrecomposeMode::Ldr;
        self
    }

    /// Sets the element layout declaration for this text leaf.
    pub fn layout<L, NextRole>(mut self, layout: El<L, NextRole>) -> Text<NextRole>
    where
        L: ChildLayoutState,
        NextRole: ElementRole,
    {
        let current_id = self.layout.id.take();
        let El { common, .. } = layout;
        self.layout = common;
        if self.layout.id.is_none() {
            self.layout.id = current_id;
        }
        Text {
            layout:  self.layout,
            content: self.content,
            style:   self.style,
            sizing:  self.sizing,
            role:    PhantomData,
        }
    }

    /// Sets the complete sizing policy for this text leaf.
    pub fn sizing(mut self, sizing: TextSizing) -> Self {
        self.sizing = sizing;
        self
    }

    /// Measures the visible text naturally with the requested wrapping mode.
    pub fn wrap(mut self, wrap: TextWrap) -> Self {
        self.sizing = TextSizing::wrap(wrap);
        self
    }

    /// Measures this text leaf as though it contained the surrogate string.
    pub fn measure_as(mut self, text: impl Into<String>) -> Self {
        self.sizing = TextSizing::measure_as(text);
        self
    }

    /// Reserves the surrogate string's width and wraps visible text to that width.
    pub fn wrap_at_measure(mut self, text: impl Into<String>) -> Self {
        self.sizing = TextSizing::wrap_at_measure(text);
        self
    }

    fn into_element(self) -> Element {
        let Self {
            layout,
            content,
            style,
            sizing,
            role: _,
        } = self;
        text_leaf_element(
            layout,
            ElementContent::Text {
                text: content,
                config: style,
                sizing,
            },
        )
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self { Self::new(text, TextStyle::default()) }
}

impl From<&String> for Text {
    fn from(text: &String) -> Self { Self::new(text, TextStyle::default()) }
}

impl From<String> for Text {
    fn from(text: String) -> Self { Self::new(text, TextStyle::default()) }
}

impl<T> From<(T, TextStyle)> for Text
where
    T: Into<String>,
{
    fn from((text, style): (T, TextStyle)) -> Self { Self::new(text, style) }
}

/// Public row child-layout state for [`El`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Row {
    gap:     Dimension,
    divider: Option<ChildDivider>,
}

/// Public column child-layout state for [`El`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Column {
    gap:     Dimension,
    divider: Option<ChildDivider>,
}

/// Public overlay child-layout state for [`El`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay;

/// Public marker trait for child-layout states accepted by [`LayoutBuilder`].
pub trait ChildLayoutState: private::Sealed {}

impl ChildLayoutState for Row {}

impl ChildLayoutState for Column {}

impl ChildLayoutState for Overlay {}

#[derive(Clone, Debug, PartialEq)]
struct CommonEl {
    id:                Option<PanelElementId>,
    width:             Sizing,
    height:            Sizing,
    padding:           Padding,
    align_x:           AlignX,
    align_y:           AlignY,
    background:        Option<Color>,
    border:            Option<Border>,
    corner_radius:     CornerRadius,
    overflow:          ChildOverflow,
    scroll_offset:     Vec2,
    scroll_anchor_x:   ScrollAnchor,
    scroll_anchor_y:   ScrollAnchor,
    material:          Cascade<Handle<StandardMaterial>>,
    interactivity:     Cascade<WidgetInteractivity>,
    editable:          Option<ImePanelField>,
    widget:            Option<WidgetSpec>,
    /// Boxed so the per-state appearance a widget element authors does not
    /// widen every ordinary element declaration.
    appearance:        Option<Box<StateAppearance>>,
    editor_origin:     EditorElementOrigin,
    editor_text:       Option<Box<EditorPart>>,
    editor_selection:  Option<Box<EditorPart>>,
    editor_caret:      Option<Box<EditorPart>>,
    editor_validation: Option<Box<EditorPart>>,
    tooltip:           Option<Tooltip>,
    visual_slot:       Option<VisualSlotId>,
    draw:              Option<PanelDraw>,
    z_index:           DrawZIndex,
    anti_alias:        Cascade<AntiAlias>,
    hairline_fade:     Cascade<HairlineFade>,
    shadow_casting:    Cascade<ShadowCasting>,
    precompose:        PrecomposeMode,
}

/// An erased generated-editor part declaration retained by an editable field.
///
/// The declaration keeps the `CommonEl` and child-layout value from the
/// authored [`El`]. Its reconstruction helpers restore the original typed
/// `El` before the generated editor adds it to a [`LayoutBuilder`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditorPart {
    common:       CommonEl,
    child_layout: ChildLayout,
}

enum EditorPartDeclaration {
    Row(El<Row>),
    Column(El<Column>),
    Overlay(El<Overlay>),
}

impl EditorPart {
    fn from_el<L>(el: El<L, WidgetPart>) -> Self
    where
        L: ChildLayoutState,
    {
        let El {
            common,
            child_layout,
            role: _,
        } = el;
        let child_layout =
            private::Sealed::into_child_layout(child_layout, common.align_x, common.align_y);
        Self {
            common,
            child_layout,
        }
    }

    pub(crate) const fn with_width(mut self, width: Sizing) -> Self {
        self.common.width = width;
        self
    }

    pub(crate) const fn with_height(mut self, height: Sizing) -> Self {
        self.common.height = height;
        self
    }

    pub(crate) const fn with_background_if_unset(mut self, background: Color) -> Self {
        if self.common.background.is_none() {
            self.common.background = Some(background);
        }
        self
    }

    pub(crate) fn scaled(&self, default_scale: f32) -> Self {
        let mut scaled = self.clone();
        scaled.common.width = scaled.common.width.resolved(default_scale);
        scaled.common.height = scaled.common.height.resolved(default_scale);
        scaled.common.padding = scaled.common.padding.resolved(default_scale);
        scaled.child_layout = scaled.child_layout.to_points(default_scale);
        if let Some(border) = &mut scaled.common.border {
            *border = border.resolved(default_scale);
        }
        scaled.common.corner_radius = scaled.common.corner_radius.resolved(default_scale);
        if let Some(panel_draw) = &mut scaled.common.draw {
            *panel_draw = panel_draw.scaled(default_scale);
        }
        scaled.common.scroll_offset *= default_scale;
        scale_editor_part(&mut scaled.common.editor_text, default_scale);
        scale_editor_part(&mut scaled.common.editor_selection, default_scale);
        scale_editor_part(&mut scaled.common.editor_caret, default_scale);
        scale_editor_part(&mut scaled.common.editor_validation, default_scale);
        scaled
    }

    pub(crate) fn into_text(mut self, text: &str, style: &TextStyle) -> Text {
        self.common.id = None;
        self.common.editor_origin = EditorElementOrigin::Generated;
        match self.into_declaration() {
            EditorPartDeclaration::Row(el) => Text::new(text, style.clone()).layout(el),
            EditorPartDeclaration::Column(el) => Text::new(text, style.clone()).layout(el),
            EditorPartDeclaration::Overlay(el) => Text::new(text, style.clone()).layout(el),
        }
    }

    pub(crate) fn with_children(
        mut self,
        builder: &mut LayoutBuilder,
        children: impl FnOnce(&mut LayoutBuilder),
    ) {
        self.common.editor_origin = EditorElementOrigin::Generated;
        match self.into_declaration() {
            EditorPartDeclaration::Row(el) => {
                builder.with(el, children);
            },
            EditorPartDeclaration::Column(el) => {
                builder.with(el, children);
            },
            EditorPartDeclaration::Overlay(el) => {
                builder.with(el, children);
            },
        }
    }

    fn into_declaration(self) -> EditorPartDeclaration {
        let Self {
            common,
            child_layout,
        } = self;
        match child_layout {
            ChildLayout::Row { gap, divider, .. } => EditorPartDeclaration::Row(El {
                common,
                child_layout: Row { gap, divider },
                role: PhantomData,
            }),
            ChildLayout::Column { gap, divider, .. } => EditorPartDeclaration::Column(El {
                common,
                child_layout: Column { gap, divider },
                role: PhantomData,
            }),
            ChildLayout::Overlay { .. } => EditorPartDeclaration::Overlay(El {
                common,
                child_layout: Overlay,
                role: PhantomData,
            }),
        }
    }
}

fn scale_editor_part(part: &mut Option<Box<EditorPart>>, default_scale: f32) {
    if let Some(part) = part {
        let scaled = part.scaled(default_scale);
        **part = scaled;
    }
}

impl CommonEl {
    /// Emits the surface records a state [`Appearance`] replaces values on.
    ///
    /// A [`VisualSlotOverride`](crate::widgets::VisualSlotOverride) replaces
    /// values on records layout already emitted; it never authors a missing
    /// one. A state background therefore needs an [`Element::background`] fill
    /// record and a state border color or width needs an [`Element::border`]
    /// record, so a declaration naming one without the matching ordinary
    /// declaration gets a transparent stand-in to replace. A state material
    /// carries its own color — the SDF fill reads
    /// `StandardMaterial::base_color`. The `content` parameter identifies
    /// text and draw recipients that can re-key that material, so this method
    /// adds a fill record only when no border or content recipient exists.
    fn default_state_surfaces(&mut self, content: &ElementContent) {
        let Some(appearance) = self.appearance.as_deref() else {
            return;
        };
        let cascades = appearance.cascades();
        let state_background = cascades.any(|layer| layer.background.is_authored());
        let state_border = cascades
            .any(|layer| layer.border_color.is_authored() || layer.border_width.is_authored());
        let state_material = cascades.any(|layer| layer.material.is_authored());
        let has_material_recipient = matches!(content, ElementContent::Text { .. })
            || self
                .draw
                .as_ref()
                .is_some_and(|draw| !draw.shapes_ref().is_empty());

        if self.background.is_none()
            && (state_background
                || (state_material && self.border.is_none() && !has_material_recipient))
        {
            self.background = Some(Color::NONE);
        }
        if state_border && self.border.is_none() {
            self.border = Some(Border::all(Px(0.0), Color::NONE));
        }
    }
}

impl Default for CommonEl {
    fn default() -> Self {
        Self {
            id:                None,
            width:             Sizing::FIT,
            height:            Sizing::FIT,
            padding:           Padding::default(),
            align_x:           AlignX::default(),
            align_y:           AlignY::default(),
            background:        None,
            border:            None,
            corner_radius:     CornerRadius::ZERO,
            overflow:          ChildOverflow::Visible,
            scroll_offset:     Vec2::ZERO,
            scroll_anchor_x:   ScrollAnchor::Start,
            scroll_anchor_y:   ScrollAnchor::Start,
            material:          Cascade::Inherit,
            interactivity:     Cascade::Inherit,
            editable:          None,
            widget:            None,
            appearance:        None,
            editor_origin:     EditorElementOrigin::Authored,
            editor_text:       None,
            editor_selection:  None,
            editor_caret:      None,
            editor_validation: None,
            tooltip:           None,
            visual_slot:       None,
            draw:              None,
            z_index:           DrawZIndex::default(),
            anti_alias:        Cascade::Inherit,
            hairline_fade:     Cascade::Inherit,
            shadow_casting:    Cascade::Inherit,
            precompose:        PrecomposeMode::Direct,
        }
    }
}

fn text_leaf_element(mut common: CommonEl, content: ElementContent) -> Element {
    common.default_state_surfaces(&content);
    Element {
        id: common.id,
        width: common.width,
        height: common.height,
        padding: common.padding,
        child_layout: ChildLayout::default(),
        background: common.background,
        border: common.border,
        corner_radius: common.corner_radius,
        overflow: common.overflow,
        scroll_offset: common.scroll_offset,
        scroll_anchor_x: common.scroll_anchor_x,
        scroll_anchor_y: common.scroll_anchor_y,
        material: common.material,
        interactivity: common.interactivity,
        editable: common.editable,
        widget: common.widget,
        appearance: common.appearance,
        editor_origin: common.editor_origin,
        editor_text: common.editor_text,
        editor_selection: common.editor_selection,
        editor_caret: common.editor_caret,
        editor_validation: common.editor_validation,
        tooltip: common.tooltip,
        visual_slot: common.visual_slot,
        draw: common.draw,
        z_index: common.z_index,
        anti_alias: common.anti_alias,
        hairline_fade: common.hairline_fade,
        shadow_casting: common.shadow_casting,
        precompose: common.precompose,
        content,
    }
}

impl<L> Default for El<L, LayoutOnly>
where
    L: Default,
{
    fn default() -> Self {
        Self {
            common:       CommonEl::default(),
            child_layout: L::default(),
            role:         PhantomData,
        }
    }
}

impl<L, Role> El<L, Role> {
    /// Marks this element as an inline-editor generated part.
    pub(crate) const fn generated_editor_part(mut self) -> Self {
        self.common.editor_origin = EditorElementOrigin::Generated;
        self
    }
}

impl El<Row, LayoutOnly> {
    /// Creates a new row element declaration with default settings.
    pub fn new() -> Self { Self::row() }

    /// Creates a left-to-right row element declaration.
    pub fn row() -> Self { Self::default() }
}

impl<Role: ElementRole> El<Row, Role> {
    /// Sets the gap between adjacent row children.
    pub fn gap(mut self, gap: impl Into<Dimension>) -> Self {
        self.child_layout.gap = gap.into();
        self
    }

    /// Sets a separator between adjacent row children.
    pub const fn child_divider(mut self, divider: ChildDivider) -> Self {
        self.child_layout.divider = Some(divider);
        self
    }
}

impl El<Column, LayoutOnly> {
    /// Creates a top-to-bottom column element declaration.
    pub fn column() -> Self { Self::default() }
}

impl<Role: ElementRole> El<Column, Role> {
    /// Sets the gap between adjacent column children.
    pub fn gap(mut self, gap: impl Into<Dimension>) -> Self {
        self.child_layout.gap = gap.into();
        self
    }

    /// Sets a separator between adjacent column children.
    pub const fn child_divider(mut self, divider: ChildDivider) -> Self {
        self.child_layout.divider = Some(divider);
        self
    }
}

impl El<Overlay, LayoutOnly> {
    /// Creates an overlay element declaration.
    pub fn overlay() -> Self { Self::default() }
}

impl<L, Role> El<L, Role> {
    /// Sets the width sizing rule.
    ///
    /// Can be overridden by a subsequent `.size()` call (last wins).
    ///
    /// Common patterns: [`Sizing::GROW`], [`Sizing::FIT`], [`Sizing::fixed`], [`Sizing::percent`].
    pub const fn width(mut self, sizing: Sizing) -> Self {
        self.common.width = sizing;
        self
    }

    /// Assigns a panel-local id to this element.
    ///
    /// Named element ids share one namespace across the panel tree, including
    /// text elements and editable fields. Use ids for persistent element
    /// identity such as text lookup, hit targets, and precompose cache keys.
    pub fn id(mut self, id: impl Into<PanelElementId>) -> Self {
        self.common.id = Some(id.into());
        self
    }

    /// Sets the height sizing rule.
    ///
    /// Can be overridden by a subsequent `.size()` call (last wins).
    ///
    /// Common patterns: [`Sizing::GROW`], [`Sizing::FIT`], [`Sizing::fixed`], [`Sizing::percent`].
    pub const fn height(mut self, sizing: Sizing) -> Self {
        self.common.height = sizing;
        self
    }

    /// Sets both width and height to [`Sizing::fixed`] from two matching dimensions.
    ///
    /// Bare floats inherit the panel's layout unit. Typed wrappers like
    /// [`Mm`](crate::Mm) or [`Pt`](crate::Pt) set the unit explicitly.
    /// Both arguments must have the same type; use `.width(...)` and
    /// `.height(...)` separately when you intentionally want different
    /// unit types on each axis.
    ///
    /// Can be overridden by subsequent `.width()` or `.height()` calls
    /// (last wins).
    pub fn size<DM: DimensionMatch>(self, w: DM, h: DM) -> Self {
        let wd = w.into();
        let hd = h.into();
        self.width(Sizing::fixed(wd)).height(Sizing::fixed(hd))
    }

    /// Sets padding on all sides.
    pub const fn padding(mut self, padding: Padding) -> Self {
        self.common.padding = padding;
        self
    }

    /// Sets both horizontal and vertical child alignment.
    pub const fn alignment(mut self, x: AlignX, y: AlignY) -> Self {
        self.common.align_x = x;
        self.common.align_y = y;
        self
    }

    /// Sets horizontal child alignment.
    pub const fn align_x(mut self, align: AlignX) -> Self {
        self.common.align_x = align;
        self
    }

    /// Sets vertical child alignment.
    pub const fn align_y(mut self, align: AlignY) -> Self {
        self.common.align_y = align;
        self
    }

    /// Sets a background color.
    pub const fn background(mut self, color: Color) -> Self {
        self.common.background = Some(color);
        self
    }

    /// Sets a border.
    pub const fn border(mut self, border: Border) -> Self {
        self.common.border = Some(border);
        self
    }

    /// Sets the corner radius for rounded backgrounds and borders.
    ///
    /// Accepts `CornerRadius::all(8.0)`, `CornerRadius::new(tl, tr, br, bl)`,
    /// or a bare `f32` for uniform radius on all corners.
    pub fn corner_radius(mut self, radius: impl Into<CornerRadius>) -> Self {
        self.common.corner_radius = radius.into();
        self
    }

    /// Sets overflow to `Clipped`; default is `Visible`.
    pub const fn clip(mut self) -> Self {
        self.common.overflow = ChildOverflow::Clipped;
        self
    }

    /// Scrolls children vertically by `offset` logical px from the top and clips
    /// overflow.
    ///
    /// The offset is clamped during positioning to `[0, content - viewport]`;
    /// pass `f32::MAX` to pin to the bottom.
    pub const fn scroll_y(mut self, offset: f32) -> Self {
        self.common.scroll_offset.y = offset;
        self.common.scroll_anchor_y = ScrollAnchor::Start;
        self.common.overflow = ChildOverflow::Clipped;
        self
    }

    /// Scrolls children vertically by `scrollback` logical px measured from the
    /// bottom and clips overflow.
    ///
    /// `0` pins to the bottom, so a log following a growing tail needs no
    /// knowledge of its content height; increasing `scrollback` walks upward.
    /// Clamped during positioning to `[0, content - viewport]`.
    pub const fn scroll_y_from_end(mut self, scrollback: f32) -> Self {
        self.common.scroll_offset.y = scrollback;
        self.common.scroll_anchor_y = ScrollAnchor::End;
        self.common.overflow = ChildOverflow::Clipped;
        self
    }

    /// Scrolls children horizontally by `offset` logical px and clips overflow.
    ///
    /// The offset is clamped during positioning to `[0, content - viewport]`;
    /// pass `f32::MAX` to pin to the right edge.
    pub const fn scroll_x(mut self, offset: f32) -> Self {
        self.common.scroll_offset.x = offset;
        self.common.scroll_anchor_x = ScrollAnchor::Start;
        self.common.overflow = ChildOverflow::Clipped;
        self
    }

    /// Sets a PBR material handle override for this element.
    ///
    /// Controls surface properties (roughness, metallic, reflectance, etc.)
    /// for backgrounds, borders, and element-owned panel-shape primitives on
    /// this element. For panel-shape primitives, this is the element-local
    /// source above the panel `.shape_material(...)` handle and the global
    /// `ShapeMaterial` cascade default. For backgrounds and borders, it is
    /// above the panel `.material(...)` handle and the global `SdfMaterial`
    /// cascade default. If the element also has a `.background()` color, that
    /// color overrides the material's `base_color`. Create the material asset
    /// once through `Assets<StandardMaterial>`; do not create assets per frame.
    pub fn material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.common.material = Cascade::Override(material);
        self
    }

    /// Marks this ordinary element as the thumb of its nearest enclosing
    /// [`El::slider`].
    ///
    /// The element stays ordinary layout — it creates no ECS child and exposes
    /// no anatomy component. Value presentation reads its solved border box to
    /// translate it along the slider's active axis without relayout. A thumb
    /// outside every slider subtree, or a second thumb in one slider, is a
    /// panel build error. Zero marked thumbs leaves the slider valid with no
    /// automatic value visualization.
    pub const fn slider_thumb(self) -> Self { self.visual_slot(VisualSlotId::SLIDER_THUMB) }

    /// Authors widget interactivity for this element and its widget descendants.
    ///
    /// A descendant can replace this value with its own override.
    pub const fn widget_interactivity(mut self, value: WidgetInteractivity) -> Self {
        self.common.interactivity = Cascade::Override(value);
        self
    }

    /// Attaches a stable private visual-slot id to this element's retained
    /// render records.
    pub(crate) const fn visual_slot(mut self, slot: VisualSlotId) -> Self {
        self.common.visual_slot = Some(slot);
        self
    }
}

impl<L> El<L, LayoutOnly> {
    /// Marks this element as an editable IME widget.
    ///
    /// The `field_id` is panel-local semantic identity used for hit testing,
    /// focus traversal, anchoring, and commit routing. The widget participates
    /// in ordinary focus traversal automatically. Semantic activation opens
    /// its editor; while editing, the active IME session reserves Tab and
    /// Shift+Tab instead of moving widget focus.
    pub fn editable_field(
        mut self,
        field_id: impl Into<PanelElementId>,
        field_spec: ImeEditableFieldSpec,
    ) -> El<L, WidgetElement<EditableField>> {
        self.common.editable = Some(ImePanelField::new(field_id, field_spec));
        self.common.visual_slot = Some(VisualSlotId::EDITABLE_ROOT);
        self.into_widget_element()
    }

    /// Marks this element as a button with panel-local semantic identity `id`.
    ///
    /// Click behavior is authored on the returned element with
    /// [`El::on_click`]; the button's resting look stays on this element's
    /// ordinary [`El::background`], [`El::border`], and [`El::material`]
    /// declarations, and its per-state look on the state builders.
    pub fn button(self, id: impl Into<PanelElementId>) -> El<L, WidgetElement<Button>> {
        self.widget(id, Button::new())
    }

    /// Marks this element as a slider over `range` with panel-local semantic
    /// identity `id`.
    ///
    /// The element also receives the private root visual slot whose solved
    /// content box slider pointer projection reads. Range, value, and step are
    /// stored as authored and validated when the owning panel builds, so the
    /// declaration chain stays infallible.
    pub fn slider(
        self,
        id: impl Into<PanelElementId>,
        range: RangeInclusive<f32>,
    ) -> El<L, WidgetElement<Slider>> {
        self.widget(id, Slider::new(range))
    }

    /// Marks this element as the pre-built widget `widget` with panel-local
    /// semantic identity `id`.
    ///
    /// This is the declaration path for a [`Button`] or [`Slider`] constructed
    /// away from the layout chain; [`El::button`] and [`El::slider`] construct
    /// one inline. The id and declaration are assigned together so a widget
    /// cannot be authored without its identity.
    pub fn widget<W: Widget>(
        mut self,
        id: impl Into<PanelElementId>,
        widget: W,
    ) -> El<L, WidgetElement<W>> {
        self.common.id = Some(id.into());
        self.common.visual_slot = Some(W::root_visual_slot().0);
        self.common.widget = Some(widget.into_declaration().0);
        self.into_widget_element()
    }

    fn into_widget_element<W>(self) -> El<L, WidgetElement<W>> {
        El {
            common:       self.common,
            child_layout: self.child_layout,
            role:         PhantomData,
        }
    }
}

impl<L> El<L, WidgetChild> {
    /// Sets the appearance while the enclosing widget is hovered.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn hovered(mut self, appearance: impl IntoAppearance) -> El<L, WidgetPart> {
        self.appearance_mut().hovered = Cascade::Override(WidgetHoveredAppearance::new(appearance));
        self.into_role()
    }

    /// Sets the appearance while the enclosing widget's focus indicator is visible.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn focused(mut self, appearance: impl IntoAppearance) -> El<L, WidgetPart> {
        self.appearance_mut().focused = Cascade::Override(WidgetFocusedAppearance::new(appearance));
        self.into_role()
    }

    /// Sets the appearance while the enclosing widget is disabled.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn disabled(mut self, appearance: impl IntoAppearance) -> El<L, WidgetPart> {
        self.appearance_mut().disabled =
            Cascade::Override(WidgetDisabledAppearance::new(appearance));
        self.into_role()
    }

    /// Sets the appearance while the enclosing widget is held by a press or drag.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn pressed(mut self, appearance: impl IntoAppearance) -> El<L, PressedPart> {
        self.appearance_mut().pressed = Cascade::Override(WidgetPressedAppearance::new(appearance));
        self.into_role()
    }
}

impl<L, W> El<L, WidgetElement<W>> {
    /// Attaches a tooltip declaration to this widget element.
    ///
    /// A later call replaces the earlier declaration.
    pub fn tooltip(mut self, tooltip: Tooltip) -> Self {
        self.common.tooltip = Some(tooltip);
        self
    }

    /// Sets the appearance while a pointer hovers this widget.
    ///
    /// See [`Appearance`] for each property's retained record and ordinary
    /// declaration requirement.
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn hovered(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().hovered = Cascade::Override(WidgetHoveredAppearance::new(appearance));
        self
    }

    /// Sets the appearance while this widget's keyboard focus indicator is visible.
    ///
    /// See [`Appearance`] for each property's retained record and ordinary
    /// declaration requirement.
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn focused(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().focused = Cascade::Override(WidgetFocusedAppearance::new(appearance));
        self
    }

    /// Sets the appearance while this widget is disabled.
    ///
    /// See [`Appearance`] for each property's retained record and ordinary
    /// declaration requirement.
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn disabled(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().disabled =
            Cascade::Override(WidgetDisabledAppearance::new(appearance));
        self
    }

    const fn widget_mut(&mut self) -> Option<&mut WidgetSpec> { self.common.widget.as_mut() }
}

impl<L> El<L, WidgetElement<EditableField>> {
    /// Authors the state appearance for generated committed and preedit text runs.
    ///
    /// A later call replaces an earlier text colors declaration. The generated
    /// text content and [`TextStyle`] remain editor-controlled; the colors
    /// apply to the generated text glyphs.
    pub fn editor_text(mut self, colors: EditorStateColors) -> Self {
        self.common.editor_text =
            Some(Box::new(colors.into_editor_part(EditorPartColorRole::Text)));
        self
    }

    /// Authors the state appearance for the generated selection highlight box.
    ///
    /// A later call replaces an earlier selection colors declaration. The
    /// generated selection keeps its editor-controlled dimensions, and the
    /// colors apply to its fill.
    pub fn editor_selection(mut self, colors: EditorStateColors) -> Self {
        self.common.editor_selection =
            Some(Box::new(colors.into_editor_part(EditorPartColorRole::Fill)));
        self
    }

    /// Authors the state appearance for the generated caret box.
    ///
    /// A later call replaces an earlier caret colors declaration. The generated
    /// caret keeps its editor-controlled dimensions, and the colors apply to
    /// its fill.
    pub fn editor_caret(mut self, colors: EditorStateColors) -> Self {
        self.common.editor_caret =
            Some(Box::new(colors.into_editor_part(EditorPartColorRole::Fill)));
        self
    }

    /// Authors the state appearance for the generated validation message run.
    ///
    /// A later call replaces an earlier validation colors declaration. The
    /// generated validation message and [`TextStyle`] remain editor-controlled;
    /// the colors apply to the validation glyphs.
    pub fn editor_validation(mut self, colors: EditorStateColors) -> Self {
        self.common.editor_validation =
            Some(Box::new(colors.into_editor_part(EditorPartColorRole::Text)));
        self
    }
}

impl<L, W: Pressable> El<L, WidgetElement<W>> {
    /// Sets the appearance while this widget is held by a button press or slider drag.
    ///
    /// See [`Appearance`] for each property's retained record and ordinary
    /// declaration requirement.
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn pressed(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().pressed = Cascade::Override(WidgetPressedAppearance::new(appearance));
        self
    }
}

impl<L> El<L, WidgetPart> {
    /// Sets the appearance while the enclosing widget is hovered.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn hovered(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().hovered = Cascade::Override(WidgetHoveredAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget's focus indicator is visible.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn focused(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().focused = Cascade::Override(WidgetFocusedAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget is disabled.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn disabled(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().disabled =
            Cascade::Override(WidgetDisabledAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget is held by a press or drag.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn pressed(mut self, appearance: impl IntoAppearance) -> El<L, PressedPart> {
        self.appearance_mut().pressed = Cascade::Override(WidgetPressedAppearance::new(appearance));
        self.into_role()
    }
}

impl<L> El<L, PressedPart> {
    /// Sets the appearance while the enclosing widget is hovered.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn hovered(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().hovered = Cascade::Override(WidgetHoveredAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget's focus indicator is visible.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn focused(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().focused = Cascade::Override(WidgetFocusedAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget is disabled.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn disabled(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().disabled =
            Cascade::Override(WidgetDisabledAppearance::new(appearance));
        self
    }

    /// Sets the appearance while the enclosing widget is held by a press or drag.
    ///
    /// A later call replaces any bundle an earlier call authored for this state.
    pub fn pressed(mut self, appearance: impl IntoAppearance) -> Self {
        self.appearance_mut().pressed = Cascade::Override(WidgetPressedAppearance::new(appearance));
        self
    }
}

impl<L> El<L, WidgetElement<Button>> {
    /// Runs `system` with each completed [`ButtonClicked`](crate::ButtonClicked)
    /// for this button.
    ///
    /// See [`Button::on_click`] for the callback contract.
    pub fn on_click<M>(mut self, system: impl IntoSystem<In<ButtonClicked>, (), M>) -> Self {
        if let Some(WidgetSpec::Button(button)) = self.widget_mut() {
            button.set_callback(system);
        }
        self
    }
}

impl<L> El<L, WidgetElement<Slider>> {
    /// Sets the slider's authored default value.
    ///
    /// See [`Slider::value`].
    pub fn value(self, value: f32) -> Self { self.configure_slider(|slider| slider.value(value)) }

    /// Sets the slider's step interval.
    ///
    /// See [`Slider::step`].
    pub fn step(self, step: f32) -> Self { self.configure_slider(|slider| slider.step(step)) }

    /// Sets the direction in which slider values increase.
    ///
    /// See [`Slider::direction`].
    pub fn direction(self, direction: SliderDirection) -> Self {
        self.configure_slider(|slider| slider.direction(direction))
    }

    /// Sets the optional thumb gesture that proposes the authored default.
    ///
    /// See [`Slider::reset_behavior`].
    pub fn reset_behavior(self, reset_behavior: SliderResetBehavior) -> Self {
        self.configure_slider(|slider| slider.reset_behavior(reset_behavior))
    }

    fn configure_slider(mut self, configure: impl FnOnce(Slider) -> Slider) -> Self {
        if let Some(WidgetSpec::Slider(slider)) = self.widget_mut() {
            *slider = configure(slider.clone());
        }
        self
    }
}

impl<L, Role> El<L, Role> {
    fn appearance_mut(&mut self) -> &mut StateAppearance {
        self.common.appearance.get_or_insert_default()
    }

    fn into_role<NextRole>(self) -> El<L, NextRole> {
        El {
            common:       self.common,
            child_layout: self.child_layout,
            role:         PhantomData,
        }
    }

    /// Sets paint-only draw primitives owned by this element.
    ///
    /// `PanelDraw` does not affect layout measurement. It is stored for later
    /// render-command resolution after the element has computed bounds.
    pub fn draw(mut self, panel_draw: PanelDraw) -> Self {
        self.common.draw = Some(panel_draw);
        self
    }

    /// Sets the authored `z_index` for this element's render commands.
    pub fn z_index(mut self, z_index: impl Into<DrawZIndex>) -> Self {
        self.common.z_index = z_index.into();
        self
    }

    /// Overrides the anti-alias mode for this element's analytic line marks.
    ///
    /// Without an override the element inherits the panel entity's
    /// cascade-resolved [`AntiAlias`] (panel override else the global
    /// resource). Per-record data — an override never splits a batch.
    pub const fn anti_alias(mut self, mode: AntiAlias) -> Self {
        self.common.anti_alias = Cascade::Override(mode);
        self
    }

    /// Overrides the hairline fade policy for this element's analytic line
    /// marks.
    ///
    /// Without an override the element inherits the panel entity's
    /// cascade-resolved [`HairlineFade`] (panel override else
    /// [`HairlineWidth::fade`](crate::HairlineWidth)). Per-record data — an
    /// override never splits a batch.
    pub const fn hairline_fade(mut self, fade: HairlineFade) -> Self {
        self.common.hairline_fade = Cascade::Override(fade);
        self
    }

    /// Overrides shadow casting for this element and its render commands.
    pub const fn shadow_casting(mut self, shadow_casting: ShadowCasting) -> Self {
        self.common.shadow_casting = Cascade::Override(shadow_casting);
        self
    }

    /// Renders this element's subtree into an LDR image, then draws that image
    /// in the parent panel.
    ///
    /// This is useful when a panel subtree should keep the SDR text edge
    /// behavior even while the main scene camera renders HDR. The flattened
    /// result behaves as one alpha-blended image in the parent panel; it does
    /// not preserve per-descendant depth or interaction.
    pub const fn precompose_ldr(mut self) -> Self {
        self.common.precompose = PrecomposeMode::Ldr;
        self
    }

    /// Converts this declaration into an [`Element`] with the given content.
    fn into_element(self, content: ElementContent) -> Element
    where
        L: ChildLayoutState,
        Role: ElementRole,
    {
        let Self {
            mut common,
            child_layout,
            role: _,
        } = self;
        common.default_state_surfaces(&content);
        let child_layout = if matches!(
            content,
            ElementContent::Text { .. } | ElementContent::Image { .. }
        ) {
            ChildLayout::default()
        } else {
            private::Sealed::into_child_layout(child_layout, common.align_x, common.align_y)
        };
        Element {
            id: common.id,
            width: common.width,
            height: common.height,
            padding: common.padding,
            child_layout,
            background: common.background,
            border: common.border,
            corner_radius: common.corner_radius,
            overflow: common.overflow,
            scroll_offset: common.scroll_offset,
            scroll_anchor_x: common.scroll_anchor_x,
            scroll_anchor_y: common.scroll_anchor_y,
            material: common.material,
            interactivity: common.interactivity,
            editable: common.editable,
            widget: common.widget,
            appearance: common.appearance,
            editor_origin: common.editor_origin,
            editor_text: common.editor_text,
            editor_selection: common.editor_selection,
            editor_caret: common.editor_caret,
            editor_validation: common.editor_validation,
            tooltip: common.tooltip,
            visual_slot: common.visual_slot,
            draw: common.draw,
            z_index: common.z_index,
            anti_alias: common.anti_alias,
            hairline_fade: common.hairline_fade,
            shadow_casting: common.shadow_casting,
            precompose: common.precompose,
            content,
        }
    }
}

mod private {
    use super::AlignX;
    use super::AlignY;
    use super::Button;
    use super::Column;
    use super::EditableField;
    use super::LayoutBuilder;
    use super::LayoutOnly;
    use super::Overlay;
    use super::PressedPart;
    use super::Row;
    use super::Slider;
    use super::WidgetBuilder;
    use super::WidgetChild;
    use super::WidgetElement;
    use super::WidgetPart;
    use crate::layout::child_layout::ChildLayout;

    pub trait Sealed {
        fn into_child_layout(self, align_x: AlignX, align_y: AlignY) -> ChildLayout;
    }

    pub trait RoleSealed {}

    pub trait BuilderSealed {}

    pub struct ChildScope<'a>(&'a mut LayoutBuilder);

    impl<'a> ChildScope<'a> {
        pub(super) const fn new(layout_builder: &'a mut LayoutBuilder) -> Self {
            Self(layout_builder)
        }

        pub(super) const fn into_layout_builder(self) -> &'a mut LayoutBuilder { self.0 }
    }

    pub trait WidgetSealed {}

    pub trait WidgetOwnerSealed {}

    impl RoleSealed for LayoutOnly {}

    impl RoleSealed for WidgetChild {}

    impl<W> RoleSealed for WidgetElement<W> {}

    impl RoleSealed for WidgetPart {}

    impl RoleSealed for PressedPart {}

    impl BuilderSealed for LayoutBuilder {}

    impl<W> BuilderSealed for WidgetBuilder<'_, W> {}

    impl WidgetSealed for Button {}

    impl WidgetSealed for Slider {}

    impl WidgetOwnerSealed for Button {}

    impl WidgetOwnerSealed for Slider {}

    impl WidgetOwnerSealed for EditableField {}

    impl Sealed for Row {
        fn into_child_layout(self, align_x: AlignX, align_y: AlignY) -> ChildLayout {
            ChildLayout::Row {
                gap: self.gap,
                align_x,
                align_y,
                divider: self.divider,
            }
        }
    }

    impl Sealed for Column {
        fn into_child_layout(self, align_x: AlignX, align_y: AlignY) -> ChildLayout {
            ChildLayout::Column {
                gap: self.gap,
                align_x,
                align_y,
                divider: self.divider,
            }
        }
    }

    impl Sealed for Overlay {
        fn into_child_layout(self, align_x: AlignX, align_y: AlignY) -> ChildLayout {
            ChildLayout::Overlay { align_x, align_y }
        }
    }
}

/// Builds a [`LayoutTree`] using a closure-based nesting API.
pub struct LayoutBuilder {
    tree:         LayoutTree,
    /// Stack of parent indices for nesting.
    parent_stack: Vec<usize>,
    /// Per-build counter that mints [`PanelElementId::Auto`] ids for unnamed text
    /// runs in build order. It starts at `0` for every builder, so auto ids are
    /// stable only within one build (`set_tree` rebuilds restart it) and never
    /// persisted or compared across panels — the positional identity an unnamed
    /// run keeps from the old `(element_idx, command_index)` reuse key.
    next_auto_id: u32,
}

/// Builder scope that accepts only the children of widget owner `W`.
///
/// `LayoutBuilder::with_widget_root` returns this builder with `'static` as an
/// owned-storage marker; callers do not need to hold a `'static` borrow. Child
/// closures receive a shorter reborrowed scope.
pub struct WidgetBuilder<'a, W> {
    storage: WidgetBuilderStorage<'a>,
    owner:   PhantomData<fn() -> W>,
}

enum WidgetBuilderStorage<'a> {
    Owned(LayoutBuilder),
    Borrowed(&'a mut LayoutBuilder),
}

impl<'a, W> WidgetBuilder<'a, W> {
    fn borrowed(layout_builder: &'a mut LayoutBuilder) -> Self {
        Self {
            storage: WidgetBuilderStorage::Borrowed(layout_builder),
            owner:   PhantomData,
        }
    }

    const fn layout_builder_mut(&mut self) -> &mut LayoutBuilder {
        match &mut self.storage {
            WidgetBuilderStorage::Owned(layout_builder) => layout_builder,
            WidgetBuilderStorage::Borrowed(layout_builder) => layout_builder,
        }
    }
}

impl<W> WidgetBuilder<'static, W> {
    fn owned(layout_builder: LayoutBuilder) -> Self {
        Self {
            storage: WidgetBuilderStorage::Owned(layout_builder),
            owner:   PhantomData,
        }
    }

    /// Finishes building the widget-rooted layout tree.
    #[must_use]
    pub fn build(self) -> LayoutTree {
        match self.storage {
            WidgetBuilderStorage::Owned(layout_builder) => layout_builder.build(),
            WidgetBuilderStorage::Borrowed(layout_builder) => layout_builder.take_tree(),
        }
    }
}

/// Describes which element roles a builder scope accepts.
///
/// A missing implementation rejects a state-appearance part outside a widget
/// closure or a widget nested in another widget.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot accept `{Role}`",
    label = "this element is not allowed in the current builder scope",
    note = "put state appearance parts (`hovered` / `focused` / `disabled` / `pressed`) inside a widget closure; a widget closure accepts parts and layout content, but not another widget"
)]
pub trait AcceptsElement<Role: ElementRole>: private::BuilderSealed {
    /// Builder reborrow passed to the child closure.
    #[doc(hidden)]
    type ChildBuilder<'a>: LayoutContentBuilder
    where
        Self: 'a;

    /// Passes the crate-minted child scope associated with `Role` to `children`.
    #[doc(hidden)]
    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a;
}

/// Common layout-content operations available in panel and widget scopes.
///
/// This trait requires [`AcceptsElement`] for [`LayoutOnly`], so helpers can
/// author ordinary layout content without losing the enclosing widget owner.
pub trait LayoutContentBuilder: private::BuilderSealed + AcceptsElement<LayoutOnly> {
    /// Adds a child container under the current parent, then fills it in.
    fn with<L, Role>(
        &mut self,
        el: El<L, Role>,
        children: impl FnOnce(&mut <Self as AcceptsElement<Role>>::ChildBuilder<'_>),
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>;

    /// Adds a text leaf as a child of the current parent.
    fn text(&mut self, text: impl Into<Text<LayoutOnly>>) -> &mut Self;

    /// Adds an image leaf as a child of the current parent.
    fn image<L>(&mut self, el: El<L, LayoutOnly>, handle: Handle<Image>, tint: Color) -> &mut Self
    where
        L: ChildLayoutState;
}

impl LayoutBuilder {
    /// Creates a new builder with an implicit fixed-size root.
    ///
    /// This is the "layout inside a viewport/canvas" constructor. The builder
    /// inserts a root element whose width and height are [`Sizing::Fixed`],
    /// using the provided `width` and `height`.
    ///
    /// That means the returned tree always has an outer box of exactly this
    /// size, even if the visible content inside it shrink-wraps smaller. This
    /// is useful when you want a stable layout viewport for:
    ///
    /// - mapping layout units to world-space dimensions,
    /// - wrapping text against a known maximum width,
    /// - `Grow` / `Percent` sizing against a known parent size,
    /// - keeping panel dimensions stable while content changes.
    ///
    /// Use [`Self::with_root`] instead when you do not want this extra fixed
    /// wrapper and want the root element itself to be content-driven (`Fit`),
    /// growable, or otherwise fully caller-defined.
    #[must_use]
    pub fn new(width: impl Into<Dimension>, height: impl Into<Dimension>) -> Self {
        let mut tree = LayoutTree::new();
        let root = tree.add(Element {
            width: Sizing::Fixed(width.into()),
            height: Sizing::Fixed(height.into()),
            ..Element::default()
        });
        tree.set_root(root);

        Self {
            tree,
            parent_stack: vec![root],
            next_auto_id: 0,
        }
    }

    /// Like [`Self::new`] but pre-allocates capacity for the element vec.
    ///
    /// Each row of content typically creates 3–5 elements. Pre-allocating
    /// avoids repeated vec reallocations during tree construction.
    #[must_use]
    pub fn with_capacity(
        width: impl Into<Dimension>,
        height: impl Into<Dimension>,
        capacity: usize,
    ) -> Self {
        let mut tree = LayoutTree::with_capacity(capacity);
        let root = tree.add(Element {
            width: Sizing::Fixed(width.into()),
            height: Sizing::Fixed(height.into()),
            ..Element::default()
        });
        tree.set_root(root);

        Self {
            tree,
            parent_stack: vec![root],
            next_auto_id: 0,
        }
    }

    /// Creates a new builder with a caller-supplied root element.
    ///
    /// This is the "my visible panel *is* the root" constructor. Unlike
    /// [`Self::new`], it does not insert an implicit fixed-size wrapper first.
    /// The `El` you provide becomes the actual root of the layout tree.
    ///
    /// Use this when you want the root itself to control sizing, for example:
    ///
    /// - a `Fit` root that grows with its content,
    /// - a root with its own border/background/padding,
    /// - a root constrained by `fit_range` rather than fixed dimensions,
    /// - a tree where the computed root bounds should reflect the visible panel rather than an
    ///   invisible outer viewport.
    ///
    /// Note that this only changes the layout tree structure. It does not
    /// remove the need for higher-level code to decide how layout units map to
    /// world space.
    #[must_use]
    pub fn with_root<L>(el: El<L, LayoutOnly>) -> Self
    where
        L: ChildLayoutState,
    {
        Self::from_root(el)
    }

    /// Creates a widget-scoped builder with a caller-supplied widget root.
    ///
    /// The returned builder owns its layout storage; the `'static` lifetime is
    /// that ownership marker, not a borrow requirement for the caller.
    #[must_use]
    pub fn with_widget_root<L, W>(el: El<L, WidgetElement<W>>) -> WidgetBuilder<'static, W>
    where
        L: ChildLayoutState,
        W: WidgetOwner,
    {
        WidgetBuilder::owned(Self::from_root(el))
    }

    fn from_root<L, Role>(el: El<L, Role>) -> Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
    {
        let mut tree = LayoutTree::new();
        let root = tree.add(el.into_element(ElementContent::Empty));
        tree.set_root(root);

        Self {
            tree,
            parent_stack: vec![root],
            next_auto_id: 0,
        }
    }

    /// Adds a child container under the current parent, then fills it in.
    ///
    /// The passed `El` is converted into an `Element` and inserted as a
    /// child of whatever the current parent is:
    ///
    /// - after [`Self::new`], the initial current parent is the implicit fixed-size root inserted
    ///   by the builder,
    /// - after [`Self::with_root`], the initial current parent is the custom root you supplied.
    ///
    /// The closure runs with this newly inserted child pushed as the current
    /// parent, so nested calls to `.with(...)` or `.text(...)` add descendants
    /// inside it. When the closure returns, the parent stack is restored.
    ///
    /// In other words, `.with(...)` always creates another node in the tree.
    /// It does not modify the existing root element; choose that root up front
    /// with [`Self::new`] or [`Self::with_root`].
    pub fn with<L, Role>(
        &mut self,
        el: El<L, Role>,
        children: impl FnOnce(&mut <Self as AcceptsElement<Role>>::ChildBuilder<'_>),
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.with_element(el, |layout_builder| {
            <Self as AcceptsElement<Role>>::with_child_builder(
                private::ChildScope::new(layout_builder),
                children,
            );
        });
        self
    }

    /// Adds a text leaf as a child of the current parent.
    ///
    /// Like [`Self::with`], this inserts a new node under the current parent:
    ///
    /// - after [`Self::new`], that initially means the implicit fixed-size root,
    /// - after [`Self::with_root`], that initially means your custom root,
    /// - inside a `.with(...)` closure, it means the container introduced by that `.with(...)`
    ///   call.
    ///
    /// Text nodes are leaves, not containers, so they cannot receive children
    /// of their own. Use [`Self::with`] when you want to create another nested
    /// container instead of a text leaf.
    ///
    /// The run is given a builder-minted [`PanelElementId::Auto`] id unless the
    /// declaration supplies [`Text::id`].
    pub fn text<Role>(&mut self, text: impl Into<Text<Role>>) -> &mut Self
    where
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.add_text(text);
        self
    }

    /// Mints the next build-order [`PanelElementId::Auto`] id for an unnamed run.
    const fn take_auto_id(&mut self) -> PanelElementId {
        let id = PanelElementId::auto(self.next_auto_id);
        self.next_auto_id += 1;
        id
    }

    /// Adds an image leaf as a child of the current parent.
    ///
    /// Image elements are leaves — they cannot have children. The element's
    /// [`Sizing`] rules control the rendered dimensions. Use
    /// [`Sizing::GROW`] to fill the parent or [`Sizing::fixed`] for an
    /// explicit size.
    ///
    /// The `tint` color is multiplied against the texture sample
    /// ([`Color::WHITE`] = no tint).
    pub fn image<L, Role>(
        &mut self,
        el: El<L, Role>,
        handle: Handle<Image>,
        tint: Color,
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.add_image(el, handle, tint);
        self
    }

    fn with_element<L, Role>(&mut self, el: El<L, Role>, children: impl FnOnce(&mut Self))
    where
        L: ChildLayoutState,
        Role: ElementRole,
    {
        let parent = self.current_parent();
        let index = self
            .tree
            .add_child(parent, el.into_element(ElementContent::Empty));
        self.parent_stack.push(index);
        children(self);
        self.parent_stack.pop();
    }

    fn add_text<Role>(&mut self, text: impl Into<Text<Role>>)
    where
        Role: ElementRole,
    {
        let parent = self.current_parent();
        let mut text = text.into();
        let id = text
            .layout
            .id
            .clone()
            .unwrap_or_else(|| self.take_auto_id());
        text.layout.id = Some(id);
        self.tree.add_child(parent, text.into_element());
    }

    fn add_image<L, Role>(&mut self, el: El<L, Role>, handle: Handle<Image>, tint: Color)
    where
        L: ChildLayoutState,
        Role: ElementRole,
    {
        let parent = self.current_parent();
        self.tree.add_child(
            parent,
            el.into_element(ElementContent::Image { handle, tint }),
        );
    }

    /// Finishes building and returns the layout tree.
    #[must_use]
    pub fn build(self) -> LayoutTree { self.tree }

    fn take_tree(&mut self) -> LayoutTree { std::mem::replace(&mut self.tree, LayoutTree::new()) }

    /// Returns the current parent index.
    fn current_parent(&self) -> usize { self.parent_stack.last().copied().unwrap_or(0) }
}

impl AcceptsElement<LayoutOnly> for LayoutBuilder {
    type ChildBuilder<'a> = Self;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        children(child_scope.into_layout_builder());
    }
}

impl<W: WidgetOwner> AcceptsElement<WidgetElement<W>> for LayoutBuilder {
    type ChildBuilder<'a> = WidgetBuilder<'a, W>;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        let mut widget_builder = WidgetBuilder::borrowed(child_scope.into_layout_builder());
        children(&mut widget_builder);
    }
}

impl<W: WidgetOwner> AcceptsElement<LayoutOnly> for WidgetBuilder<'_, W> {
    type ChildBuilder<'a>
        = WidgetBuilder<'a, W>
    where
        Self: 'a;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        let mut widget_builder = WidgetBuilder::borrowed(child_scope.into_layout_builder());
        children(&mut widget_builder);
    }
}

impl<W: WidgetOwner> AcceptsElement<WidgetChild> for WidgetBuilder<'_, W> {
    type ChildBuilder<'a>
        = WidgetBuilder<'a, W>
    where
        Self: 'a;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        let mut widget_builder = WidgetBuilder::borrowed(child_scope.into_layout_builder());
        children(&mut widget_builder);
    }
}

impl<W: WidgetOwner> AcceptsElement<WidgetPart> for WidgetBuilder<'_, W> {
    type ChildBuilder<'a>
        = WidgetBuilder<'a, W>
    where
        Self: 'a;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        let mut widget_builder = WidgetBuilder::borrowed(child_scope.into_layout_builder());
        children(&mut widget_builder);
    }
}

impl<W: Pressable> AcceptsElement<PressedPart> for WidgetBuilder<'_, W> {
    type ChildBuilder<'a>
        = WidgetBuilder<'a, W>
    where
        Self: 'a;

    fn with_child_builder<'a>(
        child_scope: private::ChildScope<'a>,
        children: impl FnOnce(&mut Self::ChildBuilder<'a>),
    ) where
        Self: 'a,
    {
        let mut widget_builder = WidgetBuilder::borrowed(child_scope.into_layout_builder());
        children(&mut widget_builder);
    }
}

impl LayoutContentBuilder for LayoutBuilder {
    fn with<L, Role>(
        &mut self,
        el: El<L, Role>,
        children: impl FnOnce(&mut <Self as AcceptsElement<Role>>::ChildBuilder<'_>),
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.with_element(el, |layout_builder| {
            <Self as AcceptsElement<Role>>::with_child_builder(
                private::ChildScope::new(layout_builder),
                children,
            );
        });
        self
    }

    fn text(&mut self, text: impl Into<Text<LayoutOnly>>) -> &mut Self {
        self.add_text(text);
        self
    }

    fn image<L>(&mut self, el: El<L, LayoutOnly>, handle: Handle<Image>, tint: Color) -> &mut Self
    where
        L: ChildLayoutState,
    {
        self.add_image(el, handle, tint);
        self
    }
}

impl<W: WidgetOwner> WidgetBuilder<'_, W> {
    /// Converts an ordinary declaration into a child of this widget.
    ///
    /// The returned element can author [`El::hovered`], [`El::focused`],
    /// [`El::disabled`], and, for pressable widgets, [`El::pressed`].
    pub fn child<L>(&self, el: El<L, LayoutOnly>) -> El<L, WidgetChild> { el.into_role() }

    /// Adds a child container under the current widget-scope parent.
    pub fn with<L, Role>(
        &mut self,
        el: El<L, Role>,
        children: impl FnOnce(&mut WidgetBuilder<'_, W>),
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        let layout_builder = self.layout_builder_mut();
        let parent = layout_builder.current_parent();
        let index = layout_builder
            .tree
            .add_child(parent, el.into_element(ElementContent::Empty));
        layout_builder.parent_stack.push(index);
        {
            let mut child_builder = WidgetBuilder::borrowed(layout_builder);
            children(&mut child_builder);
        }
        layout_builder.parent_stack.pop();
        self
    }

    /// Adds a text leaf under the current widget-scope parent.
    pub fn text<Role>(&mut self, text: impl Into<Text<Role>>) -> &mut Self
    where
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.layout_builder_mut().add_text(text);
        self
    }

    /// Adds an image leaf under the current widget-scope parent.
    pub fn image<L, Role>(
        &mut self,
        el: El<L, Role>,
        handle: Handle<Image>,
        tint: Color,
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.layout_builder_mut().add_image(el, handle, tint);
        self
    }
}

impl<W: WidgetOwner> LayoutContentBuilder for WidgetBuilder<'_, W> {
    fn with<L, Role>(
        &mut self,
        el: El<L, Role>,
        children: impl FnOnce(&mut <Self as AcceptsElement<Role>>::ChildBuilder<'_>),
    ) -> &mut Self
    where
        L: ChildLayoutState,
        Role: ElementRole,
        Self: AcceptsElement<Role>,
    {
        self.layout_builder_mut()
            .with_element(el, |layout_builder| {
                <Self as AcceptsElement<Role>>::with_child_builder(
                    private::ChildScope::new(layout_builder),
                    children,
                );
            });
        self
    }

    fn text(&mut self, text: impl Into<Text<LayoutOnly>>) -> &mut Self {
        self.layout_builder_mut().add_text(text);
        self
    }

    fn image<L>(&mut self, el: El<L, LayoutOnly>, handle: Handle<Image>, tint: Color) -> &mut Self
    where
        L: ChildLayoutState,
    {
        self.layout_builder_mut().add_image(el, handle, tint);
        self
    }
}

impl LayoutTree {
    pub(crate) fn tooltip_add_container<L>(&mut self, parent: usize, el: El<L, LayoutOnly>) -> usize
    where
        L: ChildLayoutState,
    {
        self.add_child(parent, el.into_element(ElementContent::Empty))
    }

    pub(crate) fn tooltip_add_text(
        &mut self,
        parent: usize,
        text: impl Into<Text<LayoutOnly>>,
        next_auto_id: &mut u32,
    ) {
        let mut text = text.into();
        let id = text
            .layout
            .id
            .clone()
            .unwrap_or_else(|| PanelElementId::auto(*next_auto_id));
        if text.layout.id.is_none() {
            *next_auto_id += 1;
        }
        text.layout.id = Some(id);
        self.add_child(parent, text.into_element());
    }

    pub(crate) fn tooltip_add_image<L>(
        &mut self,
        parent: usize,
        el: El<L, LayoutOnly>,
        handle: Handle<Image>,
        tint: Color,
    ) where
        L: ChildLayoutState,
    {
        self.add_child(
            parent,
            el.into_element(ElementContent::Image { handle, tint }),
        );
    }
}

#[cfg(test)]
mod tests {
    use bevy::color::Color;
    use bevy::pbr::StandardMaterial;
    use bevy::prelude::Handle;

    use super::El;
    use super::ElementContent;
    use crate::Appearance;
    use crate::Border;
    use crate::Px;
    use crate::cascade::Cascade;

    #[test]
    fn explicit_empty_hovered_appearance_is_a_cascade_override() {
        let element = El::new().button("action").hovered(Appearance::new());
        let appearance = element.common.appearance.unwrap_or_default();

        assert!(matches!(appearance.hovered, Cascade::Override(_)));
        assert!(matches!(appearance.pressed, Cascade::Inherit));
        assert!(matches!(appearance.focused, Cascade::Inherit));
        assert!(matches!(appearance.disabled, Cascade::Inherit));
    }

    #[test]
    fn state_background_without_a_background_defaults_a_transparent_fill() {
        let element = El::new()
            .button("action")
            .hovered(Appearance::new().background(Color::WHITE))
            .into_element(ElementContent::Empty);

        assert_eq!(element.background, Some(Color::NONE));
        assert_eq!(element.border, None);
    }

    #[test]
    fn state_border_width_without_a_border_defaults_a_transparent_border() {
        let element = El::new()
            .button("action")
            .hovered(Appearance::new().border_width(Px(2.0)))
            .into_element(ElementContent::Empty);

        assert_eq!(element.border, Some(Border::all(Px(0.0), Color::NONE)));
        assert_eq!(element.background, None);
    }

    #[test]
    fn state_material_without_a_surface_defaults_a_transparent_fill() {
        let element = El::new()
            .button("action")
            .hovered(Appearance::new().material(Handle::<StandardMaterial>::default()))
            .into_element(ElementContent::Empty);

        assert_eq!(element.background, Some(Color::NONE));
    }

    #[test]
    fn state_material_re_keys_an_authored_border_without_defaulting_a_fill() {
        let element = El::new()
            .button("action")
            .border(Border::all(Px(1.0), Color::BLACK))
            .hovered(Appearance::new().material(Handle::<StandardMaterial>::default()))
            .into_element(ElementContent::Empty);

        assert_eq!(element.background, None);
    }

    #[test]
    fn a_declared_surface_survives_the_state_defaulting() {
        let element = El::new()
            .button("action")
            .background(Color::BLACK)
            .border(Border::all(Px(1.0), Color::WHITE))
            .hovered(
                Appearance::new()
                    .background(Color::WHITE)
                    .border_width(Px(2.0)),
            )
            .into_element(ElementContent::Empty);

        assert_eq!(element.background, Some(Color::BLACK));
        assert_eq!(element.border, Some(Border::all(Px(1.0), Color::WHITE)));
    }
}
