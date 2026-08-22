//! [`DiegeticPanelBuilder`] with compile-time state machine enforcing call
//! order on panel construction.

use std::marker::PhantomData;

use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::prelude::AlphaMode;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Query;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Vec2;
use bevy::window::WindowRef;
use sealed::CanBuild;

use super::constants::DEFAULT_SCREEN_SPACE_CAMERA_ORDER;
use super::constants::DEFAULT_SCREEN_SPACE_RENDER_LAYER;
use super::constants::MIN_PANEL_WORLD_HEIGHT;
use super::coordinate_space::CoordinateSpace;
use super::coordinate_space::PanelSpace;
use super::coordinate_space::ScreenPosition;
use super::coordinate_space::SurfaceShadow;
use super::diegetic_panel::DiegeticPanel;
use super::sizing::CompatibleUnits;
use super::sizing::PanelSizing;
use crate::PanelElementId;
use crate::SliderConfigError;
use crate::cascade::Cascade;
use crate::layout;
use crate::layout::Anchor;
use crate::layout::Dimension;
use crate::layout::InvalidSize;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutTree;
use crate::layout::PanelSize;
use crate::layout::PaperSize;
use crate::layout::Pt;
use crate::layout::Px;
use crate::layout::ShadowCasting;
use crate::layout::Sizing;
use crate::layout::Unit;
use crate::widgets;
use crate::widgets::IntoAppearance;
use crate::widgets::PanelPicking;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetPressedAppearance;

/// Error returned by [`DiegeticPanelBuilder::build`].
///
/// A sizing or layout-tree validation error returned before a panel is built.
#[derive(thiserror::Error, Debug)]
pub enum PanelBuildError {
    /// A fixed-size axis was zero or negative.
    #[error("{0}")]
    InvalidSize(#[from] InvalidSize),
    /// Two elements share the same author-assigned [`PanelElementId`]. Element
    /// ids and editable-field ids share one panel-local namespace, so a name
    /// reused across either kind is a build error.
    #[error("duplicate panel element id `{0}`")]
    DuplicateElementId(PanelElementId),
    /// A widget used a builder-minted auto id instead of a stable authored id.
    #[error("widget `{0}` requires a named panel element id")]
    WidgetRequiresNamedId(PanelElementId),
    /// A widget is inside a subtree rendered through precomposition.
    #[error("widget `{0}` is inside a precomposed subtree")]
    WidgetInsidePrecomposedSubtree(PanelElementId),
    /// A slider-thumb marker sits outside every slider subtree.
    #[error("slider thumb `{0}` must be inside a slider subtree")]
    SliderThumbOutsideSlider(PanelElementId),
    /// A slider subtree marks more than one thumb.
    #[error("slider `{0}` contains more than one thumb")]
    SliderHasMultipleThumbs(PanelElementId),
    /// A widget part named a state text color without emitting text itself.
    #[error("widget part `{0}` state text color requires text; add a text child")]
    StateTextColorRequiresText(PanelElementId),
    /// A widget part named a state path color without emitting a panel draw itself.
    #[error("widget part `{0}` state path color requires a draw; add a draw")]
    StatePathColorRequiresDraw(PanelElementId),
    /// A widget part named a state tint without emitting an image itself.
    #[error("widget part `{0}` state tint requires an image; add an image")]
    StateTintRequiresImage(PanelElementId),
    /// A slider's authored range, value, or step failed validation.
    #[error("slider `{0}`: {1}")]
    SliderConfig(PanelElementId, #[source] SliderConfigError),
}

// ── Typestate marker types ──────────────────────────────────────────────────

/// Marker for world-space panel and widget identities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct World;

/// Marker for screen-space panel and widget identities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Screen;

/// Opaque identity for a live panel in coordinate space `Space`.
///
/// Obtain handles through [`PanelEntityReader`]. After a queued coordinate-space
/// conversion applies, reacquire a handle for the destination space through the
/// reader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelEntity<Space> {
    entity: Entity,
    space:  PanelSpace,
    marker: PhantomData<fn() -> Space>,
}

impl<Space> PanelEntity<Space> {
    /// Returns the underlying Bevy entity for unrelated ECS work.
    #[must_use]
    pub const fn entity(&self) -> Entity { self.entity }

    pub(crate) const fn from_validated(entity: Entity, space: PanelSpace) -> Self {
        Self {
            entity,
            space,
            marker: PhantomData,
        }
    }

    pub(crate) const fn expected_space(&self) -> PanelSpace { self.space }
}

/// Opaque identity for a live widget owned by a panel in coordinate space
/// `Space`.
///
/// Obtain widget handles through
/// [`PanelWidgetReader::typed_entity`](crate::PanelWidgetReader::typed_entity).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WidgetEntity<Space> {
    entity: Entity,
    owner:  Entity,
    space:  PanelSpace,
    marker: PhantomData<fn() -> Space>,
}

impl<Space> WidgetEntity<Space> {
    /// Returns the underlying Bevy entity for unrelated ECS work.
    #[must_use]
    pub const fn entity(&self) -> Entity { self.entity }

    pub(crate) const fn from_validated(entity: Entity, owner: Entity, space: PanelSpace) -> Self {
        Self {
            entity,
            owner,
            space,
            marker: PhantomData,
        }
    }

    pub(crate) const fn owner(&self) -> Entity { self.owner }

    pub(super) const fn expected_space(&self) -> PanelSpace { self.space }
}

/// Read-only lookup that mints typed identities for live panels.
#[derive(SystemParam)]
pub struct PanelEntityReader<'w, 's> {
    panels: Query<'w, 's, &'static DiegeticPanel>,
}

impl PanelEntityReader<'_, '_> {
    /// Returns a world-space handle when `entity` is currently a world panel.
    #[must_use]
    pub fn world(&self, entity: Entity) -> Option<PanelEntity<World>> {
        self.panel(entity, PanelSpace::World)
            .map(|()| PanelEntity::from_validated(entity, PanelSpace::World))
    }

    /// Returns a screen-space handle when `entity` is currently a screen panel.
    #[must_use]
    pub fn screen(&self, entity: Entity) -> Option<PanelEntity<Screen>> {
        self.panel(entity, PanelSpace::Screen)
            .map(|()| PanelEntity::from_validated(entity, PanelSpace::Screen))
    }

    fn panel(&self, entity: Entity, expected: PanelSpace) -> Option<()> {
        let panel = self.panels.get(entity).ok()?;
        (PanelSpace::from(panel.coordinate_space()) == expected).then_some(())
    }
}

/// Marker: builder needs `.size()` or `.paper()` before `.layout()` or `.build()`.
pub struct NeedsSize;

/// Marker: dimensions are set, `.layout()` or `.build()` are available.
pub struct HasSize;

/// Marker: layout tree is built, `.build()` is available.
pub struct Ready;

// ── Builder data (shared across all states) ─────────────────────────────────

#[derive(Default)]
pub(super) struct BuilderData {
    width:                      f32,
    height:                     f32,
    layout_unit:                Unit,
    font_unit:                  Cascade<Unit>,
    anchor:                     Option<Anchor>,
    world_width:                Option<f32>,
    world_height:               Option<f32>,
    shadow_casting:             Cascade<ShadowCasting>,
    widget_hovered_appearance:  Cascade<WidgetHoveredAppearance>,
    widget_pressed_appearance:  Cascade<WidgetPressedAppearance>,
    widget_focused_appearance:  Cascade<WidgetFocusedAppearance>,
    widget_disabled_appearance: Cascade<WidgetDisabledAppearance>,
    material:                   Cascade<Handle<StandardMaterial>>,
    text_material:              Cascade<Handle<StandardMaterial>>,
    shape_material:             Cascade<Handle<StandardMaterial>>,
    text_alpha_mode:            Cascade<AlphaMode>,
    hdr_text_coverage_bias:     Cascade<f32>,
    picking:                    PanelPicking,
    tree:                       Option<LayoutTree>,
    coordinate_space:           CoordinateSpace,
}

/// Builder for [`DiegeticPanel`].
///
/// Constructed via [`DiegeticPanel::world()`] or [`DiegeticPanel::screen()`].
/// The type parameters enforce the correct method call order at compile time:
///
/// - `Mode`: `World` or `Screen` — determines which methods are available
/// - `State`: `NeedsSize` → `HasSize` → `Ready` — enforces `.size()` before `.layout()`, and
///   `.layout()` before or instead of `.build()`
///
/// # State machine
///
/// ```text
/// NeedsSize ──.size()/.paper()──→ HasSize ──.layout()──→ Ready
///                                    │                     │
///                                    └──.build()───────────┘──→ Result<DiegeticPanel>
/// ```
///
/// # Screen vs World sizing — divergence table
///
/// `.size<W, H>(w, h)` accepts any value implementing
/// [`PanelSizing<Mode>`](super::sizing::PanelSizing). The trait is
/// implemented differently for each mode:
///
/// | Value | `PanelSizing<Screen>` | `PanelSizing<World>` | Rationale |
/// |---|:-:|:-:|---|
/// | [`Px`] / [`Mm`](crate::Mm) / [`Pt`] / [`In`](crate::layout::In) / bare `f32` | ✅ | ✅ | physical sizes — legal anywhere |
/// | [`Sizing`] (engine enum — escape hatch) | ✅ | ✅ | both modes share one engine |
/// | [`Fit`](super::sizing::Fit) / [`FitMax`](super::sizing::FitMax) / [`FitRange`](super::sizing::FitRange) | ✅ | ✅ | shrink-to-content works on both |
/// | [`Percent`](super::sizing::Percent) | ✅ | ❌ compile error | world has no parent |
/// | [`Grow`](super::sizing::Grow) / [`GrowMax`](super::sizing::GrowMax) / [`GrowRange`](super::sizing::GrowRange) | ✅ | ❌ compile error | world has no bounding container |
/// | Physical-unit cross-axis match | N/A (pixel-only) | enforced via [`CompatibleUnits`](super::sizing::CompatibleUnits) | unit mixing in 3D is always a bug |
/// | `.world_width(m)` / `.world_height(m)` | — | ✅ | world-space scaling |
///
/// # Zero / negative size is a runtime check
///
/// `Px(0.0)` is valid at compile time — the type system alone can't
/// reject zero or negative sizes without reaching for `NonZeroDimension`
/// wrappers that would break `Px(0.0)` as a legitimate `GrowRange` floor
/// or `FitMax` sentinel. So `.build()` returns
/// [`InvalidSize`](crate::layout::InvalidSize) when both axes are fixed
/// and either is zero or negative. Dynamic axes
/// (`Fit` / `Percent` / `Grow`) start at `0.0` and resolve later — they
/// do *not* trip this check.
///
/// # `Sizing` escape-hatch footgun on world panels
///
/// Every value type routes to the engine's [`Sizing`] enum via
/// [`PanelSizing::to_sizing`](super::sizing::PanelSizing::to_sizing).
/// The enum itself implements `PanelSizing<World>`, so a caller can
/// sneak a `Sizing::Grow { .. }` or `Sizing::Percent(_)` onto a world
/// panel through the escape hatch, even though the compile-time guard
/// rejects [`Grow`](super::sizing::Grow) / [`Percent`](super::sizing::Percent)
/// directly. Debug builds catch this with a `debug_assert!` in the
/// world `build()` path; release builds silently clamp to the resolved
/// dimensions.
pub struct DiegeticPanelBuilder<Mode, State> {
    data:   BuilderData,
    marker: PhantomData<(Mode, State)>,
}

impl DiegeticPanelBuilder<World, NeedsSize> {
    pub(super) fn new_world() -> Self {
        Self {
            data:   BuilderData::default(),
            marker: PhantomData,
        }
    }
}

impl DiegeticPanelBuilder<Screen, NeedsSize> {
    pub(super) fn new_screen() -> Self {
        Self {
            data:   BuilderData {
                // Screen overlays remain legible when an application changes
                // its world-text alpha default for demonstration or rendering
                // purposes. Authors can opt back into the global cascade with
                // `inherit_text_alpha` after spawning the panel.
                text_alpha_mode: Cascade::Override(AlphaMode::Blend),
                coordinate_space: CoordinateSpace::Screen {
                    position:      ScreenPosition::default(),
                    // Placeholder — `.size()` overwrites both axes before build().
                    width:         Sizing::fixed(Px(0.0)),
                    height:        Sizing::fixed(Px(0.0)),
                    camera_order:  DEFAULT_SCREEN_SPACE_CAMERA_ORDER,
                    render_layers: RenderLayers::layer(DEFAULT_SCREEN_SPACE_RENDER_LAYER),
                    window:        WindowRef::Primary,
                },
                ..BuilderData::default()
            },
            marker: PhantomData,
        }
    }
}

// ── Shared methods (all modes, all states) ──────────────────────────────────

impl<M, S> DiegeticPanelBuilder<M, S> {
    /// Sets the anchor point (default [`Anchor::TopLeft`]).
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.data.anchor = Some(anchor);
        self
    }

    /// Sets the front- and back-face pointer behavior for the panel.
    ///
    /// Applied as a seed on spawn or replacement: it installs only when the
    /// entity has no live [`PanelPicking`], and an installed seed that the
    /// application never rewrites is removed together with the
    /// [`DiegeticPanel`] role.
    #[must_use]
    pub const fn picking(mut self, panel_picking: PanelPicking) -> Self {
        self.data.picking = panel_picking;
        self
    }

    /// Sets the default PBR material handle for backgrounds and borders.
    ///
    /// Create the handle once through `Assets<StandardMaterial>` and pass the
    /// handle here. `base_color` is overridden by the layout color when both
    /// are set. Individual elements can override via `El::material`.
    #[must_use]
    pub fn material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.data.material = Cascade::Override(material);
        self
    }

    /// Sets the default PBR material handle for text.
    ///
    /// Create the handle once through `Assets<StandardMaterial>` and pass the
    /// handle here. `base_color` is overridden by `TextStyle::color` when set.
    #[must_use]
    pub fn text_material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.data.text_material = Cascade::Override(material);
        self
    }

    /// Sets the default PBR material handle for panel-shape primitives.
    ///
    /// Create the handle once through `Assets<StandardMaterial>` and pass the
    /// handle here. Per-shape color values override the resolved material's
    /// `base_color` before the frame material-table row is projected.
    #[must_use]
    pub fn shape_material(mut self, material: Handle<StandardMaterial>) -> Self {
        self.data.shape_material = Cascade::Override(material);
        self
    }

    /// Sets a panel-wide [`AlphaMode`] default for every text chunk in this
    /// panel. Per-style overrides still win; this in turn overrides
    /// `TextAlpha`.
    ///
    /// The default is [`AlphaMode::Blend`], ordered by per-command
    /// `depth_bias`; [`AlphaMode::AlphaToCoverage`] with MSAA is an
    /// alternative for hard-edged coverage.
    #[must_use]
    pub const fn text_alpha_mode(mut self, mode: AlphaMode) -> Self {
        self.data.text_alpha_mode = Cascade::Override(mode);
        self
    }

    /// Sets a panel-wide HDR text coverage bias inherited by text runs in this
    /// panel.
    ///
    /// The default is `0.0`, which leaves the automatic screen-size adjustment
    /// unchanged. Positive values add opacity to fractional glyph edges beyond
    /// that adjustment, which can help dark text on light backgrounds under
    /// HDR. Per-label
    /// [`TextStyle::with_hdr_text_coverage_bias`](crate::TextStyle::with_hdr_text_coverage_bias)
    /// still wins.
    #[must_use]
    pub const fn hdr_text_coverage_bias(mut self, bias: f32) -> Self {
        self.data.hdr_text_coverage_bias = Cascade::Override(bias);
        self
    }

    /// Overrides the font unit (default inherits from
    /// [`PanelDefaults::panel_font_unit`](crate::PanelDefaults)).
    #[must_use]
    pub const fn font_unit(mut self, unit: Unit) -> Self {
        self.data.font_unit = Cascade::Override(unit);
        self
    }

    /// Compatibility adapter for old panel-surface shadow authoring.
    ///
    /// Prefer [`shadow_casting`](Self::shadow_casting), which participates in
    /// the shared [`ShadowCasting`] cascade.
    #[must_use]
    pub const fn surface_shadow(mut self, shadow: SurfaceShadow) -> Self {
        self.data.shadow_casting = Cascade::Override(match shadow {
            SurfaceShadow::Off => ShadowCasting::Off,
            SurfaceShadow::On => ShadowCasting::On,
        });
        self
    }

    /// Sets whether this panel and its descendants cast 3D shadows.
    #[must_use]
    pub const fn shadow_casting(mut self, shadow_casting: ShadowCasting) -> Self {
        self.data.shadow_casting = Cascade::Override(shadow_casting);
        self
    }

    /// Sets the appearance inherited by widgets while they are hovered.
    #[must_use]
    pub fn widget_hovered_appearance(mut self, appearance: impl IntoAppearance) -> Self {
        self.data.widget_hovered_appearance =
            Cascade::Override(WidgetHoveredAppearance::new(appearance));
        self
    }

    /// Sets the appearance inherited by widgets while they are pressed.
    #[must_use]
    pub fn widget_pressed_appearance(mut self, appearance: impl IntoAppearance) -> Self {
        self.data.widget_pressed_appearance =
            Cascade::Override(WidgetPressedAppearance::new(appearance));
        self
    }

    /// Sets the appearance inherited by widgets while their focus indicator is visible.
    #[must_use]
    pub fn widget_focused_appearance(mut self, appearance: impl IntoAppearance) -> Self {
        self.data.widget_focused_appearance =
            Cascade::Override(WidgetFocusedAppearance::new(appearance));
        self
    }

    /// Sets the appearance inherited by widgets while they are disabled.
    #[must_use]
    pub fn widget_disabled_appearance(mut self, appearance: impl IntoAppearance) -> Self {
        self.data.widget_disabled_appearance =
            Cascade::Override(WidgetDisabledAppearance::new(appearance));
        self
    }
}

// ── NeedsSize → HasSize transitions ────────────────────────────────────────

impl DiegeticPanelBuilder<World, NeedsSize> {
    /// Sets the panel dimensions and layout unit.
    ///
    /// Each axis may use any value implementing
    /// [`PanelSizing<World>`](super::sizing::PanelSizing): [`Px`], [`Mm`](crate::Mm),
    /// [`Pt`], [`In`](crate::layout::In), bare `f32`,
    /// [`Fit`](super::sizing::Fit), [`FitMax`](super::sizing::FitMax),
    /// [`FitRange`](super::sizing::FitRange), or the engine [`Sizing`]
    /// enum (escape hatch). Screen-only variants
    /// ([`Percent`](super::sizing::Percent), [`Grow`](super::sizing::Grow),
    /// [`GrowMax`](super::sizing::GrowMax),
    /// [`GrowRange`](super::sizing::GrowRange)) are compile errors on a
    /// world panel — there is no parent container to resolve against.
    ///
    /// Cross-axis physical-unit consistency is enforced at compile time
    /// via [`CompatibleUnits`](super::sizing::CompatibleUnits): two
    /// concrete physical units must match (e.g. `(Mm, Mm)` is fine;
    /// `(Mm, Px)` is a compile error). Unit-less values like [`Fit`](crate::Fit) or
    /// bare `f32` adopt the other axis's unit, or fall back to
    /// [`Unit::Meters`] when both axes are unit-less.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// DiegeticPanel::world().size(0.5, 0.3)              // 0.5 × 0.3 meters
    /// DiegeticPanel::world().size(Mm(210.0), Mm(297.0))  // A4 in mm
    /// DiegeticPanel::world().size(Fit, FitMax(Mm(300.0)))
    /// ```
    #[must_use]
    pub fn size<W, H>(mut self, w: W, h: H) -> DiegeticPanelBuilder<World, HasSize>
    where
        W: PanelSizing<World>,
        H: PanelSizing<World>,
        (W::Unit, H::Unit): CompatibleUnits,
    {
        let w_sizing = w.to_sizing();
        let h_sizing = h.to_sizing();
        let unit = sizing_unit(w_sizing)
            .or_else(|| sizing_unit(h_sizing))
            .unwrap_or(Unit::Meters);
        self.data.width = initial_panel_size(w_sizing);
        self.data.height = initial_panel_size(h_sizing);
        self.data.layout_unit = unit;
        self.data.coordinate_space = CoordinateSpace::World {
            width:  w_sizing,
            height: h_sizing,
        };
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }

    /// Sets the panel dimensions from a predefined paper size.
    ///
    /// Uses the paper's native unit (mm for ISO, inches for North American).
    #[must_use]
    pub fn paper(mut self, paper: PaperSize) -> DiegeticPanelBuilder<World, HasSize> {
        let (w, h, unit) = paper.dimensions();
        self.data.width = w;
        self.data.height = h;
        self.data.layout_unit = unit;
        self.data.coordinate_space = CoordinateSpace::World {
            width:  Sizing::fixed(Dimension {
                value: w,
                unit:  Some(unit),
            }),
            height: Sizing::fixed(Dimension {
                value: h,
                unit:  Some(unit),
            }),
        };
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }
}

impl DiegeticPanelBuilder<Screen, NeedsSize> {
    /// Sets the panel dimensions on each axis.
    ///
    /// Each axis may use any value implementing
    /// [`PanelSizing<Screen>`](super::sizing::PanelSizing): [`Px`], [`Mm`](crate::Mm),
    /// [`Pt`], [`In`](crate::layout::In), bare `f32`,
    /// [`Fit`](super::sizing::Fit), [`FitMax`](super::sizing::FitMax),
    /// [`FitRange`](super::sizing::FitRange),
    /// [`Percent`](super::sizing::Percent), [`Grow`](super::sizing::Grow),
    /// [`GrowMax`](super::sizing::GrowMax),
    /// [`GrowRange`](super::sizing::GrowRange), or the engine [`Sizing`]
    /// enum (escape hatch). Axes are independent — no cross-axis unit
    /// check applies.
    ///
    /// Screen panels always operate in logical pixels for layout; the
    /// panel's internal `layout_unit` is always [`Unit::Pixels`]
    /// regardless of what the argument types carry.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// DiegeticPanel::screen().size(Px(600.0), Px(44.0))            // fixed
    /// DiegeticPanel::screen().size(Percent(0.25), Px(44.0))        // % width
    /// DiegeticPanel::screen().size(FitMax(Px(400.0)), Fit)         // auto-shrink
    /// DiegeticPanel::screen().size(Grow, Grow)                     // fill
    /// ```
    #[must_use]
    pub fn size<W, H>(mut self, w: W, h: H) -> DiegeticPanelBuilder<Screen, HasSize>
    where
        W: PanelSizing<Screen>,
        H: PanelSizing<Screen>,
    {
        let w_sizing = w.to_sizing();
        let h_sizing = h.to_sizing();
        // Screen panels always use `Unit::Pixels` for their layout dimension.
        self.data.layout_unit = Unit::Pixels;
        self.data.width = initial_panel_size(w_sizing);
        self.data.height = initial_panel_size(h_sizing);
        if let CoordinateSpace::Screen { width, height, .. } = &mut self.data.coordinate_space {
            *width = w_sizing;
            *height = h_sizing;
        }
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }

    /// Sets the panel dimensions from a predefined paper size.
    ///
    /// Uses [`Sizing::Fixed`] for both axes, in pixels (1 pt → 1 px under
    /// the orthographic overlay camera).
    #[must_use]
    pub fn paper(mut self, paper: PaperSize) -> DiegeticPanelBuilder<Screen, HasSize> {
        let w = paper.width_as::<Pt>();
        let h = paper.height_as::<Pt>();
        self.data.width = w;
        self.data.height = h;
        self.data.layout_unit = Unit::Pixels;
        if let CoordinateSpace::Screen { width, height, .. } = &mut self.data.coordinate_space {
            *width = Sizing::fixed(Px(w));
            *height = Sizing::fixed(Px(h));
        }
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }
}

/// Initial panel width/height to stash in [`BuilderData`]. For `Fixed` we know
/// the exact value; for any dynamic [`Sizing`] we use `0.0` as a placeholder
/// that the screen-space system resolves each frame against the window and
/// the layout result.
const fn initial_panel_size(s: Sizing) -> f32 {
    match s {
        Sizing::Fixed(d) => d.value,
        _ => 0.0,
    }
}

/// Returns the first explicit [`Unit`] observed inside a [`Sizing`] value,
/// or `None` if the sizing carries only unit-less dimensions.
///
/// `Sizing::Fixed(Dimension { unit: Some(u), .. })` yields `Some(u)`;
/// `Sizing::Fit { min, max }` prefers `min.unit` over `max.unit`;
/// `Sizing::Percent` has no backing dimension and yields `None`.
const fn sizing_unit(s: Sizing) -> Option<Unit> {
    match s {
        Sizing::Fixed(d) => d.unit,
        Sizing::Fit { min, max } | Sizing::Grow { min, max } => match min.unit {
            Some(u) => Some(u),
            None => max.unit,
        },
        Sizing::Percent(_) => None,
    }
}

// ── World-only methods (any state) ──────────────────────────────────────────

impl<S> DiegeticPanelBuilder<World, S> {
    /// Scales the panel uniformly so its world width matches this value in meters.
    /// Height follows the aspect ratio.
    #[must_use]
    pub const fn world_width(mut self, meters: f32) -> Self {
        self.data.world_width = Some(meters);
        self
    }

    /// Scales the panel uniformly so its world height matches this value in meters.
    /// Width follows the aspect ratio.
    #[must_use]
    pub const fn world_height(mut self, meters: f32) -> Self {
        self.data.world_height = Some(meters);
        self
    }
}

// ── Screen-only methods on HasSize ──────────────────────────────────────────

impl DiegeticPanelBuilder<Screen, HasSize> {
    /// Places the panel at an explicit pixel position (top-left origin, y-down).
    #[must_use]
    pub const fn screen_position(mut self, x: f32, y: f32) -> Self {
        if let CoordinateSpace::Screen { position, .. } = &mut self.data.coordinate_space {
            *position = ScreenPosition::At(Vec2::new(x, y));
        }
        self
    }

    /// Sets the overlay camera render order. Default: `100` (high enough that
    /// user-spawned 3D viewport cameras don't collide).
    #[must_use]
    pub const fn camera_order(mut self, order: isize) -> Self {
        if let CoordinateSpace::Screen { camera_order, .. } = &mut self.data.coordinate_space {
            *camera_order = order;
        }
        self
    }

    /// Chooses which screen-space panels share an overlay view. Panels with
    /// the same layers, camera order, and target window share one view.
    /// `hana_diegetic` assigns that view a private render layer so views for
    /// other windows cannot draw these panels. Default: layer 31.
    #[must_use]
    pub fn render_layers(mut self, layers: RenderLayers) -> Self {
        if let CoordinateSpace::Screen { render_layers, .. } = &mut self.data.coordinate_space {
            *render_layers = layers;
        }
        self
    }

    /// Pins the panel to a specific window. Default: [`WindowRef::Primary`].
    #[must_use]
    pub const fn window(mut self, window: WindowRef) -> Self {
        if let CoordinateSpace::Screen { window: w, .. } = &mut self.data.coordinate_space {
            *w = window;
        }
        self
    }

    /// Sugar for [`Self::window`] with [`WindowRef::Entity`].
    #[must_use]
    pub const fn window_entity(self, entity: Entity) -> Self {
        self.window(WindowRef::Entity(entity))
    }
}

// ── HasSize → Ready transition (layout) ─────────────────────────────────────

impl DiegeticPanelBuilder<World, HasSize> {
    /// Builds the layout tree from a closure. The closure receives a
    /// [`LayoutBuilder`] pre-configured with the panel's dimensions.
    #[must_use]
    pub fn layout(
        mut self,
        f: impl FnOnce(&mut LayoutBuilder),
    ) -> DiegeticPanelBuilder<World, Ready> {
        let mut builder = LayoutBuilder::new(self.data.width, self.data.height);
        f(&mut builder);
        self.data.tree = Some(builder.build());
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }

    /// Attaches a pre-built layout tree.
    #[must_use]
    pub fn with_tree(mut self, tree: LayoutTree) -> DiegeticPanelBuilder<World, Ready> {
        self.data.tree = Some(tree);
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }
}

impl DiegeticPanelBuilder<Screen, HasSize> {
    /// Builds the layout tree from a closure. The closure receives a
    /// [`LayoutBuilder`] pre-configured with the panel's dimensions.
    #[must_use]
    pub fn layout(
        mut self,
        f: impl FnOnce(&mut LayoutBuilder),
    ) -> DiegeticPanelBuilder<Screen, Ready> {
        let mut builder = LayoutBuilder::new(self.data.width, self.data.height);
        f(&mut builder);
        self.data.tree = Some(builder.build());
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }

    /// Attaches a pre-built layout tree.
    #[must_use]
    pub fn with_tree(mut self, tree: LayoutTree) -> DiegeticPanelBuilder<Screen, Ready> {
        self.data.tree = Some(tree);
        DiegeticPanelBuilder {
            data:   self.data,
            marker: PhantomData,
        }
    }
}

// ── Sealed CanBuild trait ───────────────────────────────────────────────────

mod sealed {
    pub trait CanBuild {}
}

impl CanBuild for HasSize {}
impl CanBuild for Ready {}

// ── Build (on HasSize or Ready, either mode) ────────────────────────────────

impl<S: sealed::CanBuild> DiegeticPanelBuilder<World, S> {
    /// Consumes the builder and returns a [`DiegeticPanel`] component.
    ///
    /// Dynamic axes (`Fit { .. }`) start at `0.0`; the post-layout system
    /// resolves them to content bounds each frame, so zero-size inputs
    /// are only rejected when *both* axes are `Fixed(_)`.
    ///
    /// `Percent` / `Grow` are compile-rejected for world panels via
    /// [`PanelSizing<World>`](super::sizing::PanelSizing); a caller that
    /// routes them in via [`Sizing`] (the escape hatch) gets them clamped
    /// to the resolved dimensions — a `debug_assert!` flags the footgun
    /// in debug builds.
    ///
    /// # Errors
    ///
    /// Returns [`PanelBuildError::InvalidSize`] if both axes are fixed-size and
    /// width or height is zero or negative. Returns a widget validation variant
    /// when ids or interactive nesting violate the layout-tree contract.
    pub fn build(mut self) -> Result<DiegeticPanel, PanelBuildError> {
        let (w_sizing, h_sizing) = match self.data.coordinate_space {
            CoordinateSpace::World { width, height } => (width, height),
            CoordinateSpace::Screen { .. } => {
                return Err(InvalidSize {
                    width:  self.data.width,
                    height: self.data.height,
                }
                .into());
            },
        };

        debug_assert!(
            !matches!(w_sizing, Sizing::Grow { .. } | Sizing::Percent(_)),
            "world panel width is Grow/Percent — the escape-hatch `Sizing` bypassed the \
             compile-time guard. These variants have no parent container in world space; \
             use `Fit`, `FitMax`, `FitRange`, or a fixed physical unit instead."
        );
        debug_assert!(
            !matches!(h_sizing, Sizing::Grow { .. } | Sizing::Percent(_)),
            "world panel height is Grow/Percent — the escape-hatch `Sizing` bypassed the \
             compile-time guard. These variants have no parent container in world space; \
             use `Fit`, `FitMax`, `FitRange`, or a fixed physical unit instead."
        );

        let w_dynamic = !matches!(w_sizing, Sizing::Fixed(_));
        let h_dynamic = !matches!(h_sizing, Sizing::Fixed(_));

        if !w_dynamic && !h_dynamic && (self.data.width <= 0.0 || self.data.height <= 0.0) {
            return Err(InvalidSize {
                width:  self.data.width,
                height: self.data.height,
            }
            .into());
        }

        if let Some(ref mut tree) = self.data.tree {
            match w_sizing {
                Sizing::Fit { min, max } => layout::set_root_fit_width(tree, min, max),
                Sizing::Grow { min, max } => {
                    layout::set_root_grow_width(tree, min, max);
                },
                Sizing::Percent(_) | Sizing::Fixed(_) => {},
            }
            match h_sizing {
                Sizing::Fit { min, max } => layout::set_root_fit_height(tree, min, max),
                Sizing::Grow { min, max } => {
                    layout::set_root_grow_height(tree, min, max);
                },
                Sizing::Percent(_) | Sizing::Fixed(_) => {},
            }
        }

        if let Some(tree) = self.data.tree.as_ref() {
            widgets::validate_tree(tree)?;
        }

        Ok(build_panel(self.data))
    }
}

impl<S: sealed::CanBuild> DiegeticPanelBuilder<Screen, S> {
    /// Consumes the builder and returns a [`DiegeticPanel`] component.
    ///
    /// For screen-space panels, sets `world_height = height` if neither
    /// `world_width` nor `world_height` is set, so that `points_to_world`
    /// equals 1.0 (1 layout unit = 1 world unit = 1 screen pixel under
    /// the orthographic overlay camera).
    ///
    /// When the panel's width or height is [`Sizing::Percent`], [`Sizing::Fit`],
    /// or [`Sizing::Grow`] the layout tree's root element is automatically
    /// updated so the engine reflows against the resolved panel dimensions
    /// without needing a tree rebuild.
    ///
    /// # Errors
    ///
    /// Returns [`PanelBuildError::InvalidSize`] if width or height is zero or
    /// negative and no dynamic sizing will fill it later. Returns a widget
    /// validation variant when ids or interactive nesting violate the
    /// layout-tree contract.
    pub fn build(mut self) -> Result<DiegeticPanel, PanelBuildError> {
        let CoordinateSpace::Screen {
            width: w_sizing,
            height: h_sizing,
            ..
        } = self.data.coordinate_space
        else {
            return Err(InvalidSize {
                width:  self.data.width,
                height: self.data.height,
            }
            .into());
        };
        let has_dynamic_width = !matches!(w_sizing, Sizing::Fixed(_));
        let has_dynamic_height = !matches!(h_sizing, Sizing::Fixed(_));

        if !has_dynamic_width
            && !has_dynamic_height
            && (self.data.width <= 0.0 || self.data.height <= 0.0)
        {
            return Err(InvalidSize {
                width:  self.data.width,
                height: self.data.height,
            }
            .into());
        }

        // Freeze `world_height` for fixed screen panels so 1 layout pixel
        // maps to 1 world unit under the ortho camera. Dynamic-axis
        // panels skip this: their `self.data.height` is still `0.0` at
        // build time, and `anchor_offsets()` short-circuits to
        // `panel.width`/`panel.height` directly for screen panels so
        // the frozen field isn't needed for positioning.
        if !has_dynamic_width
            && !has_dynamic_height
            && self.data.world_height.is_none()
            && self.data.world_width.is_none()
        {
            self.data.world_height = Some(self.data.height.max(MIN_PANEL_WORLD_HEIGHT));
        }

        if let Some(ref mut tree) = self.data.tree {
            // The layout engine's two passes (bottom-up `propagate_fit_sizes`
            // and top-down `size_along_axis`) already resolve `Fit` roots to
            // their natural content size, so we route each Sizing variant to
            // the matching root kind here. `Percent` routes to an unbounded
            // `Grow` at the root because `resolve_screen_axis` has already
            // pre-multiplied the panel width by the percent fraction — the
            // viewport the engine receives is the post-percent panel size.
            match w_sizing {
                Sizing::Fit { min, max } => layout::set_root_fit_width(tree, min, max),
                Sizing::Grow { min, max } => {
                    layout::set_root_grow_width(tree, min, max);
                },
                Sizing::Percent(_) => {
                    layout::set_root_grow_width(
                        tree,
                        Dimension {
                            value: 0.0,
                            unit:  None,
                        },
                        Dimension {
                            value: f32::MAX,
                            unit:  None,
                        },
                    );
                },
                Sizing::Fixed(_) => {},
            }
            match h_sizing {
                Sizing::Fit { min, max } => layout::set_root_fit_height(tree, min, max),
                Sizing::Grow { min, max } => {
                    layout::set_root_grow_height(tree, min, max);
                },
                Sizing::Percent(_) => {
                    layout::set_root_grow_height(
                        tree,
                        Dimension {
                            value: 0.0,
                            unit:  None,
                        },
                        Dimension {
                            value: f32::MAX,
                            unit:  None,
                        },
                    );
                },
                Sizing::Fixed(_) => {},
            }
        }
        let _ = (has_dynamic_width, has_dynamic_height);

        if let Some(tree) = self.data.tree.as_ref() {
            widgets::validate_tree(tree)?;
        }

        Ok(build_panel(self.data))
    }
}

fn build_panel(data: BuilderData) -> DiegeticPanel {
    let mut panel = DiegeticPanel::with_initial_tree(data.tree.unwrap_or_default());
    panel.width = data.width;
    panel.height = data.height;
    panel.layout_unit = data.layout_unit;
    panel.font_unit = data.font_unit;
    panel.anchor = data.anchor.unwrap_or(Anchor::TopLeft);
    panel.world_width = data.world_width;
    panel.world_height = data.world_height;
    panel.shadow_casting = data.shadow_casting;
    panel.widget_hovered_appearance = data.widget_hovered_appearance;
    panel.widget_pressed_appearance = data.widget_pressed_appearance;
    panel.widget_focused_appearance = data.widget_focused_appearance;
    panel.widget_disabled_appearance = data.widget_disabled_appearance;
    panel.material = data.material;
    panel.text_material = data.text_material;
    panel.shape_material = data.shape_material;
    panel.text_alpha_mode = data.text_alpha_mode;
    panel.hdr_text_coverage_bias = data.hdr_text_coverage_bias;
    panel.picking = data.picking;
    panel.coordinate_space = data.coordinate_space;
    panel
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use bevy::prelude::AlphaMode;
    use bevy::prelude::Color;
    use bevy::prelude::Handle;
    use bevy_kana::Cascade;

    use super::PanelBuildError;
    use crate::Appearance;
    use crate::Border;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::Fit;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::InvalidSize;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::SliderConfigError;
    use crate::Text;
    use crate::TextStyle;

    fn style() -> TextStyle { TextStyle::new(Mm(6.0)) }

    fn field_spec() -> ImeEditableFieldSpec {
        ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text))
    }

    #[test]
    fn panel_build_error_messages_are_stable() {
        let cases = [
            (
                PanelBuildError::from(InvalidSize {
                    width:  0.0,
                    height: 12.0,
                }),
                "panel dimensions must be positive, got 0×12",
            ),
            (
                PanelBuildError::DuplicateElementId(PanelElementId::named("title")),
                "duplicate panel element id `title`",
            ),
            (
                PanelBuildError::WidgetRequiresNamedId(PanelElementId::auto(3)),
                "widget `#auto-3` requires a named panel element id",
            ),
            (
                PanelBuildError::WidgetInsidePrecomposedSubtree(PanelElementId::named("button")),
                "widget `button` is inside a precomposed subtree",
            ),
            (
                PanelBuildError::SliderThumbOutsideSlider(PanelElementId::named("thumb")),
                "slider thumb `thumb` must be inside a slider subtree",
            ),
            (
                PanelBuildError::SliderHasMultipleThumbs(PanelElementId::named("volume")),
                "slider `volume` contains more than one thumb",
            ),
            (
                PanelBuildError::StateTintRequiresImage(PanelElementId::named("image")),
                "widget part `image` state tint requires an image; add an image",
            ),
            (
                PanelBuildError::SliderConfig(
                    PanelElementId::named("volume"),
                    SliderConfigError::UnorderedRange,
                ),
                "slider `volume`: slider range start must be less than its end",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn invalid_size_conversion_preserves_source() {
        let error = PanelBuildError::from(InvalidSize {
            width:  -2.0,
            height: 4.0,
        });

        assert!(matches!(
            error.source(),
            Some(source)
                if source.is::<InvalidSize>()
                    && source.to_string()
                        == "panel dimensions must be positive, got -2×4"
        ));
    }

    #[test]
    fn screen_panel_authors_stable_text_alpha() {
        let result = DiegeticPanel::screen().size(Fit, Fit).build();

        assert!(matches!(
            result,
            Ok(panel)
                if panel.text_alpha_mode == Cascade::Override(AlphaMode::Blend)
        ));
    }

    #[test]
    fn duplicate_named_element_ids_error_at_build() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::new().id("title"), |_| {});
                builder.text(Text::new("B", style()).id("title"));
            })
            .build();

        assert!(matches!(
            result,
            Err(PanelBuildError::DuplicateElementId(ref id)) if *id == PanelElementId::named("title")
        ));
    }

    #[test]
    fn duplicate_widget_ids_use_panel_element_validation() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::new().button("action"), |_| {});
                builder.with(El::new().button("action"), |_| {});
            })
            .build();

        assert!(matches!(
            result,
            Err(PanelBuildError::DuplicateElementId(ref id))
                if *id == PanelElementId::named("action")
        ));
    }

    #[test]
    fn widget_auto_id_errors_at_build() {
        let auto_id = PanelElementId::auto(4);
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::new().button(auto_id.clone()), |_| {});
            })
            .build();

        assert!(matches!(
            result,
            Err(PanelBuildError::WidgetRequiresNamedId(id)) if id == auto_id
        ));
    }

    #[test]
    fn widget_inside_precomposed_subtree_errors_at_build() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::column().precompose_ldr(), |builder| {
                    builder.with(El::new().button("action"), |_| {});
                });
            })
            .build();

        assert!(matches!(
            result,
            Err(PanelBuildError::WidgetInsidePrecomposedSubtree(id))
                if id == PanelElementId::named("action")
        ));
    }

    #[test]
    fn button_state_background_without_background_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .button("action")
                        .hovered(Appearance::new().background(Color::srgb(0.2, 0.4, 0.8))),
                    |_| {},
                );
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn button_state_border_color_without_border_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .background(Color::srgb(0.1, 0.1, 0.1))
                        .button("action")
                        .focused(Appearance::new().border_color(Color::srgb(0.9, 0.8, 0.2))),
                    |_| {},
                );
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn editable_focus_border_color_without_border_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .editable_field("name", field_spec())
                        .focused(Appearance::new().border_color(Color::srgb(0.9, 0.8, 0.2))),
                    |_| {},
                );
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn button_state_material_without_surface_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .button("action")
                        .pressed(Appearance::new().material(Handle::default())),
                    |_| {},
                );
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn button_state_builders_with_authored_targets_build_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(
                    El::new()
                        .background(Color::srgb(0.1, 0.1, 0.1))
                        .border(Border::all(1.0, Color::srgb(0.4, 0.4, 0.4)))
                        .button("action")
                        .hovered(Appearance::new().background(Color::srgb(0.2, 0.4, 0.8)))
                        .focused(Appearance::new().border_color(Color::srgb(0.9, 0.8, 0.2)))
                        .pressed(Appearance::new().material(Handle::default())),
                    |_| {},
                );
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn widget_part_state_background_without_surface_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::new().button("action"), |builder| {
                    builder.with(
                        builder
                            .child(El::new())
                            .disabled(Appearance::new().background(Color::srgb(0.2, 0.4, 0.8))),
                        |_| {},
                    );
                });
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn element_id_with_distinct_child_text_id_builds_ok() {
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::column().id("cell"), |builder| {
                    builder.text(Text::new("A", style()).id("label"));
                });
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn many_unnamed_runs_never_collide() {
        // Auto ids are minted per build; eight unnamed runs must not read as a
        // duplicate.
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                for _ in 0..8 {
                    builder.text(("x", style()));
                }
            })
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn text_id_colliding_with_editable_field_errors() {
        // Element ids and editable-field ids share one namespace: a text element
        // named `name` collides with an editable field of the same id.
        let result = DiegeticPanel::world()
            .size(Mm(50.0), Mm(30.0))
            .layout(|builder| {
                builder.with(El::new().editable_field("name", field_spec()), |builder| {
                    builder.text(("v", style()));
                });
                builder.text(Text::new("dup", style()).id("name"));
            })
            .build();

        assert!(matches!(
            result,
            Err(PanelBuildError::DuplicateElementId(_))
        ));
    }
}
