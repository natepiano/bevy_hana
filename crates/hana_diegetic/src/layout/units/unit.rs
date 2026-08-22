use bevy::prelude::Reflect;

use crate::layout::constants::METERS_PER_INCH;
use crate::layout::constants::MIN_CUSTOM_METERS_PER_UNIT;
use crate::layout::constants::PIXELS_PER_INCH;
use crate::layout::constants::POINTS_PER_INCH;

/// Physical unit for interpreting numeric dimensions.
///
/// Used by `FontUnit` and panel construction defaults to define
/// what "1.0" means for layout dimensions and font sizes. Standalone
/// per-entity font units are authored through `override_font_unit`.
///
/// `Custom(f32)` is an escape hatch for any unit not covered by the named
/// variants — the value is meters per unit.
///
/// # Examples
///
/// ```ignore
/// Unit::Meters          // 1 unit = 1 meter (Bevy default)
/// Unit::Millimeters     // 1 unit = 1mm
/// Unit::Points          // 1 unit = 1 typographic point (1/72 inch)
/// Unit::Pixels          // 1 unit = 1 logical pixel on screen
/// Unit::Inches          // 1 unit = 1 inch
/// Unit::Custom(0.01)    // 1 unit = 1 centimeter
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum Unit {
    /// 1 unit = 1 meter. Bevy's default world-space convention.
    #[default]
    Meters,
    /// 1 unit = 1 millimeter (0.001 m).
    Millimeters,
    /// 1 unit = 1 typographic point (1/72 inch ≈ 0.000353 m).
    Points,
    /// 1 unit = 1 logical pixel on screen.
    ///
    /// In the layout engine, one pixel converts to 0.75 typographic points
    /// because the unit system uses 96 logical pixels and 72 points per inch.
    /// The camera system provides the actual screen-pixel behavior:
    /// screen-space panels use an orthographic camera where one world unit is
    /// one logical pixel, while world-space panels can use per-frame
    /// `Transform` scaling.
    Pixels,
    /// 1 unit = 1 inch (0.0254 m).
    Inches,
    /// 1 unit = the given number of meters.
    Custom(f32),
}

impl Unit {
    /// Returns the conversion factor from this unit to meters.
    ///
    /// [`Unit::Custom`] values below [`Unit::Points`] (0.000353 m) are clamped.
    #[must_use]
    pub const fn meters_per_unit(self) -> f32 {
        match self {
            Self::Meters => 1.0,
            Self::Millimeters => 0.001,
            Self::Points => METERS_PER_INCH / POINTS_PER_INCH,
            // 96 DPI convention: 1 point = 96/72 pixels ≈ 1.333 pixels.
            Self::Pixels => METERS_PER_INCH / PIXELS_PER_INCH,
            Self::Inches => METERS_PER_INCH,
            Self::Custom(meters_per_unit) => {
                if meters_per_unit < MIN_CUSTOM_METERS_PER_UNIT {
                    MIN_CUSTOM_METERS_PER_UNIT
                } else {
                    meters_per_unit
                }
            },
        }
    }

    /// Returns the multiplier to convert a value in this unit to typographic points.
    #[must_use]
    pub const fn to_points(self) -> f32 { self.meters_per_unit() / Self::Points.meters_per_unit() }
}

/// A dimension with an optional unit.
///
/// Carries both the numeric value and the unit it's expressed in.
/// Created implicitly via `From` impls on [`Pt`], [`Mm`], [`In`], and bare `f32`.
///
/// - `Pt(24.0).into()` → value 24.0, unit `Points`
/// - `Mm(6.0).into()` → value 6.0, unit `Millimeters`
/// - `In(0.24).into()` → value 0.24, unit `Inches`
/// - `(24.0_f32).into()` → value 24.0, unit None (contextual default)
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Dimension {
    /// The numeric size value.
    pub value: f32,
    /// The unit the value is expressed in. `None` = contextual default.
    pub unit:  Option<Unit>,
}

impl Dimension {
    /// Resolves this dimension to points.
    ///
    /// If the dimension carries an explicit unit, converts using that
    /// unit's `to_points()`. Otherwise multiplies by `default_scale`
    /// (typically `layout_to_points` or `font_to_points`).
    #[must_use]
    pub fn to_points(self, default_scale: f32) -> f32 {
        let scaled = match self.unit {
            Some(unit) => self.value * unit.to_points(),
            None => self.value * default_scale,
        };
        // The `Sizing::Fit`/`Grow` unbounded sentinel is `f32::MAX`. Scaling it
        // by a large factor (world meters->points is ~2835x) overflows to
        // `inf`, which breaks downstream `Fit` width resolution (an unbounded
        // axis collapses to zero). Saturate so "unbounded" stays representable.
        scaled.min(f32::MAX)
    }

    /// Converts this dimension to world meters.
    ///
    /// Dimensions with an explicit unit convert via `unit.meters_per_unit()`.
    /// Dimensions without a unit (bare `f32`) use `default_meters_per_unit`
    /// (typically the panel's layout unit conversion factor).
    #[must_use]
    pub fn to_meters(self, default_meters_per_unit: f32) -> f32 {
        match self.unit {
            Some(unit) => self.value * unit.meters_per_unit(),
            None => self.value * default_meters_per_unit,
        }
    }
}

impl From<f32> for Dimension {
    fn from(value: f32) -> Self { Self { value, unit: None } }
}

// ── Dimensional newtypes ─────────────────────────────────────────────────────

/// A value in typographic points (1/72 inch ≈ 0.353 mm).
#[derive(Clone, Copy, Debug)]
pub struct Pt(pub f32);

/// A value in millimeters.
#[derive(Clone, Copy, Debug)]
pub struct Mm(pub f32);

/// A value in inches.
#[derive(Clone, Copy, Debug)]
pub struct In(pub f32);

/// A value in logical pixels.
///
/// On screen-space panels, pixels map 1:1 to
/// on-screen logical pixels. On world-space panels, the system resolves
/// pixel dimensions per-frame using the active camera's projection.
#[derive(Clone, Copy, Debug)]
pub struct Px(pub f32);

impl From<Pt> for f32 {
    fn from(v: Pt) -> Self { v.0 * Unit::Points.meters_per_unit() }
}

impl From<Mm> for f32 {
    fn from(v: Mm) -> Self { v.0 * Unit::Millimeters.meters_per_unit() }
}

impl From<In> for f32 {
    fn from(v: In) -> Self { v.0 * Unit::Inches.meters_per_unit() }
}

impl From<Pt> for Dimension {
    fn from(v: Pt) -> Self {
        Self {
            value: v.0,
            unit:  Some(Unit::Points),
        }
    }
}

impl From<Mm> for Dimension {
    fn from(v: Mm) -> Self {
        Self {
            value: v.0,
            unit:  Some(Unit::Millimeters),
        }
    }
}

impl From<In> for Dimension {
    fn from(v: In) -> Self {
        Self {
            value: v.0,
            unit:  Some(Unit::Inches),
        }
    }
}

impl From<Px> for Dimension {
    fn from(v: Px) -> Self {
        Self {
            value: v.0,
            unit:  Some(Unit::Pixels),
        }
    }
}

// ── DimensionMatch trait ────────────────────────────────────────────────────

/// Marker trait for values accepted by `.size(w, h)` methods.
///
/// The `.size(...)` APIs on
/// [`DiegeticPanelBuilder`](crate::DiegeticPanelBuilder)
/// and [`El`](crate::El) require both arguments to have the same concrete type:
///
/// - `size(100.0, 50.0)` works
/// - `size(Mm(210.0), Mm(297.0))` works
/// - `size(Mm(210.0), In(11.0))` is a compile error
///
/// Use `.width(...)` and `.height(...)` separately when you intentionally want
/// different unit types on each axis.
pub trait DimensionMatch: Into<Dimension> {}

impl DimensionMatch for f32 {}
impl DimensionMatch for Dimension {}
impl DimensionMatch for Pt {}
impl DimensionMatch for Mm {}
impl DimensionMatch for In {}
impl DimensionMatch for Px {}

/// Trait for types that carry a compile-time unit association.
///
/// Implemented by the dimensional newtypes ([`Mm`], [`In`], [`Pt`], [`Px`]).
/// Used by the generic [`PanelSize`](crate::layout::PanelSize) implementation
/// for same-unit tuples: `impl<U: HasUnit> PanelSize for (U, U)`.
pub trait HasUnit {
    /// The [`Unit`] this type represents.
    const UNIT: Unit;

    /// Returns the raw numeric value.
    fn value(self) -> f32;
}

impl HasUnit for Mm {
    const UNIT: Unit = Unit::Millimeters;

    fn value(self) -> f32 { self.0 }
}

impl HasUnit for In {
    const UNIT: Unit = Unit::Inches;

    fn value(self) -> f32 { self.0 }
}

impl HasUnit for Pt {
    const UNIT: Unit = Unit::Points;

    fn value(self) -> f32 { self.0 }
}

impl HasUnit for Px {
    const UNIT: Unit = Unit::Pixels;

    fn value(self) -> f32 { self.0 }
}
