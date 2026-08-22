//! Typestate-driven panel sizing API (Design B‴).
//!
//! [`PanelSizing<M>`] is a sealed trait parameterised by a mode marker
//! ([`super::builder::Screen`] or [`super::builder::World`]) with an
//! associated [`Unit`](sealed::Unit). Values implement `PanelSizing<M>` for
//! the mode(s) in which they are legal:
//!
//! - Shared (both modes): [`Px`] / [`Mm`] / [`Pt`] / [`In`], bare `f32`, [`Fit`], [`FitMax`],
//!   [`FitRange`], and the engine's [`Sizing`] enum (escape hatch).
//! - Screen only: [`Percent`], [`Grow`], [`GrowMax`], [`GrowRange`].
//!
//! World-panel cross-axis physical-unit consistency is enforced by the
//! [`CompatibleUnits`] tuple trait on `(W::Unit, H::Unit)`. Concrete
//! physical-unit pairs must match exactly; [`AnyUnit`] is compatible with
//! any concrete unit (adopts the other axis's unit, or falls back to the
//! mode's default).
//!
//! # Compile-time rejections
//!
//! Screen-only sizing on a world panel is a compile error:
//!
//! ```compile_fail
//! use hana_diegetic::{DiegeticPanel, Percent, Mm};
//! let _ = DiegeticPanel::world().size(Percent(0.5), Mm(100.0));
//! ```
//!
//! ```compile_fail
//! use hana_diegetic::{DiegeticPanel, Grow, Mm};
//! let _ = DiegeticPanel::world().size(Grow, Mm(100.0));
//! ```
//!
//! ```compile_fail
//! use hana_diegetic::{DiegeticPanel, GrowMax, Mm};
//! let _ = DiegeticPanel::world().size(GrowMax(Mm(1.0).into()), Mm(100.0));
//! ```
//!
//! Mismatched physical units on a world panel are a compile error:
//!
//! ```compile_fail
//! use hana_diegetic::{DiegeticPanel, Mm, Px};
//! let _ = DiegeticPanel::world().size(Mm(210.0), Px(297.0));
//! ```
//!
//! `.build()` before `.size()` is a compile error:
//!
//! ```compile_fail
//! use hana_diegetic::DiegeticPanel;
//! let _ = DiegeticPanel::world().build();
//! ```

use sealed::Mode;
use sealed::PhysicalUnit;
use sealed::Unit;

use crate::layout::Dimension;
use crate::layout::In;
use crate::layout::Mm;
use crate::layout::Pt;
use crate::layout::Px;
use crate::layout::Sizing;

// The `sealed` module is kept private, so external crates can't name —
// let alone implement — `Mode`, `Unit`, or `PhysicalUnit`. Using these
// traits as bounds on public items (`PanelSizing<M: sealed::Mode>`,
// `type Unit: sealed::Unit`, `<U: sealed::PhysicalUnit>` in
// `CompatibleUnits` impls) seals the public API through reachability
// alone — no separate `Sealed` supertrait is necessary.
mod sealed {
    pub trait Mode {}
    pub trait Unit {}
    pub trait PhysicalUnit: Unit {}
}

impl Mode for super::builder::Screen {}
impl Mode for super::builder::World {}

// ── Unit markers ─────────────────────────────────────────────────────────────

/// `Unit` marker: millimeters.
#[derive(Clone, Copy, Debug)]
pub struct Millimeters;
/// `Unit` marker: inches.
#[derive(Clone, Copy, Debug)]
pub struct Inches;
/// `Unit` marker: typographic points.
#[derive(Clone, Copy, Debug)]
pub struct Points;
/// `Unit` marker: logical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Pixels;
/// `Unit` marker: unit-less values (e.g. [`Fit`], bare `f32`, [`Sizing`]).
///
/// Adopts the other axis's unit on world panels, or the mode default
/// when both axes are unit-less.
#[derive(Clone, Copy, Debug)]
pub struct AnyUnit;

impl Unit for Millimeters {}
impl PhysicalUnit for Millimeters {}
impl Unit for Inches {}
impl PhysicalUnit for Inches {}
impl Unit for Points {}
impl PhysicalUnit for Points {}
impl Unit for Pixels {}
impl PhysicalUnit for Pixels {}
impl Unit for AnyUnit {}
// AnyUnit is deliberately NOT `PhysicalUnit` — keeps `CompatibleUnits`
// impls disjoint.

// ── Value types ──────────────────────────────────────────────────────────────

/// Shrink-wrap to content (no min/max).
#[derive(Clone, Copy, Debug)]
pub struct Fit;

/// Shrink-wrap to content, capped at a maximum.
#[derive(Clone, Copy, Debug)]
pub struct FitMax(pub Dimension);

/// Shrink-wrap to content, clamped to `[min, max]`.
#[derive(Clone, Copy, Debug)]
pub struct FitRange {
    /// Minimum size.
    pub min: Dimension,
    /// Maximum size.
    pub max: Dimension,
}

/// Expand to fill available space (screen only).
#[derive(Clone, Copy, Debug)]
pub struct Grow;

/// Expand with a maximum cap (screen only).
#[derive(Clone, Copy, Debug)]
pub struct GrowMax(pub Dimension);

/// Expand, clamped to `[min, max]` (screen only).
#[derive(Clone, Copy, Debug)]
pub struct GrowRange {
    /// Minimum size (floor).
    pub min: Dimension,
    /// Maximum size (cap).
    pub max: Dimension,
}

/// Fraction of the parent viewport (screen only, 0.0–1.0).
#[derive(Clone, Copy, Debug)]
pub struct Percent(pub f32);

// ── PanelSizing trait ────────────────────────────────────────────────────────

/// Value usable as a width or height on a panel of mode `M`.
///
/// The associated `Unit` captures the physical unit this value carries
/// so that world-panel cross-axis consistency can be enforced at compile
/// time via [`CompatibleUnits`]. External crates cannot implement this
/// trait because they cannot name `sealed::Unit` — the private module
/// boundary provides the seal.
pub trait PanelSizing<M: sealed::Mode>: Copy {
    /// Physical-unit marker for this value (or [`AnyUnit`] if unit-less).
    type Unit: sealed::Unit;

    /// Convert to an engine [`Sizing`], resolving bare `f32` against the
    /// panel's layout unit when needed.
    fn to_sizing(self) -> Sizing;
}

// ── Impls: shared (both Screen and World) ────────────────────────────────────

macro_rules! impl_both_modes {
    ($ty:ty, $unit:ty, |$self_:ident| $to_sizing:expr) => {
        impl PanelSizing<super::builder::Screen> for $ty {
            type Unit = $unit;

            fn to_sizing($self_) -> Sizing { $to_sizing }
        }
        impl PanelSizing<super::builder::World> for $ty {
            type Unit = $unit;

            fn to_sizing($self_) -> Sizing { $to_sizing }
        }
    };
}

impl_both_modes!(Px, Pixels, |self| Sizing::fixed(self));
impl_both_modes!(Mm, Millimeters, |self| Sizing::fixed(self));
impl_both_modes!(Pt, Points, |self| Sizing::fixed(self));
impl_both_modes!(In, Inches, |self| Sizing::fixed(self));
impl_both_modes!(f32, AnyUnit, |self| Sizing::fixed(self));
impl_both_modes!(Fit, AnyUnit, |self| Sizing::FIT);
impl_both_modes!(FitMax, AnyUnit, |self| Sizing::Fit {
    min: Dimension {
        value: 0.0,
        unit:  None,
    },
    max: self.0,
});
impl_both_modes!(FitRange, AnyUnit, |self| Sizing::Fit {
    min: self.min,
    max: self.max,
});
impl_both_modes!(Sizing, AnyUnit, |self| self);

// ── Impls: screen-only ───────────────────────────────────────────────────────

impl PanelSizing<super::builder::Screen> for Percent {
    type Unit = AnyUnit;

    fn to_sizing(self) -> Sizing { Sizing::percent(self.0) }
}

impl PanelSizing<super::builder::Screen> for Grow {
    type Unit = AnyUnit;

    fn to_sizing(self) -> Sizing { Sizing::GROW }
}

impl PanelSizing<super::builder::Screen> for GrowMax {
    type Unit = AnyUnit;

    fn to_sizing(self) -> Sizing {
        Sizing::Grow {
            min: Dimension {
                value: 0.0,
                unit:  None,
            },
            max: self.0,
        }
    }
}

impl PanelSizing<super::builder::Screen> for GrowRange {
    type Unit = AnyUnit;

    fn to_sizing(self) -> Sizing {
        Sizing::Grow {
            min: self.min,
            max: self.max,
        }
    }
}

// ── CompatibleUnits ──────────────────────────────────────────────────────────

/// Pairs of physical-unit markers that may legally coexist on the two
/// axes of a `World` panel.
///
/// Implementations cover: same concrete unit on both axes, and any mix
/// of a concrete unit with [`AnyUnit`]. Concrete × different-concrete
/// pairs have no impl, so a world panel with mismatched physical units
/// is a compile error.
///
/// External crates can technically add trivial impls like
/// `impl CompatibleUnits for (MyType, MyOtherType) {}`, but those impls
/// are unreachable through the builder API: `.size()` requires each
/// axis's type to implement [`PanelSizing<World>`], and `PanelSizing`'s
/// `type Unit: sealed::Unit` bound can only be satisfied by the unit
/// markers in this module. External `CompatibleUnits` impls are
/// harmless dead code.
pub trait CompatibleUnits {}

impl CompatibleUnits for (Millimeters, Millimeters) {}
impl CompatibleUnits for (Inches, Inches) {}
impl CompatibleUnits for (Points, Points) {}
impl CompatibleUnits for (Pixels, Pixels) {}
impl<U: sealed::PhysicalUnit> CompatibleUnits for (U, AnyUnit) {}
impl<U: sealed::PhysicalUnit> CompatibleUnits for (AnyUnit, U) {}
impl CompatibleUnits for (AnyUnit, AnyUnit) {}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact expected layout values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::*;
    use crate::panel::builder::Screen;
    use crate::panel::builder::World;

    fn is_screen<T: PanelSizing<Screen>>() {}
    fn is_world<T: PanelSizing<World>>() {}
    fn compatible<A: sealed::Unit, B: sealed::Unit>()
    where
        (A, B): CompatibleUnits,
    {
    }

    #[test]
    fn shared_values_impl_both_modes() {
        is_screen::<Px>();
        is_world::<Px>();
        is_screen::<Mm>();
        is_world::<Mm>();
        is_screen::<Pt>();
        is_world::<Pt>();
        is_screen::<In>();
        is_world::<In>();
        is_screen::<f32>();
        is_world::<f32>();
        is_screen::<Fit>();
        is_world::<Fit>();
        is_screen::<FitMax>();
        is_world::<FitMax>();
        is_screen::<FitRange>();
        is_world::<FitRange>();
        is_screen::<Sizing>();
        is_world::<Sizing>();
    }

    #[test]
    fn screen_only_values_impl_screen() {
        is_screen::<Percent>();
        is_screen::<Grow>();
        is_screen::<GrowMax>();
        is_screen::<GrowRange>();
    }

    #[test]
    fn compatible_unit_pairs() {
        compatible::<Millimeters, Millimeters>();
        compatible::<Inches, Inches>();
        compatible::<Points, Points>();
        compatible::<Pixels, Pixels>();
        compatible::<Millimeters, AnyUnit>();
        compatible::<AnyUnit, Millimeters>();
        compatible::<AnyUnit, AnyUnit>();
    }

    #[test]
    fn fit_to_sizing_is_unbounded_fit() {
        let sizing: Sizing = <Fit as PanelSizing<Screen>>::to_sizing(Fit);
        assert!(matches!(sizing, Sizing::Fit { .. }));
        assert!(sizing.is_fit());
    }

    #[test]
    fn fitmax_to_sizing_caps_max() {
        let fm = FitMax(Px(400.0).into());
        let sizing: Sizing = <FitMax as PanelSizing<Screen>>::to_sizing(fm);
        match sizing {
            Sizing::Fit { min, max } => {
                assert_eq!(min.value, 0.0);
                assert_eq!(max.value, 400.0);
            },
            _ => panic!("expected Sizing::Fit, got {sizing:?}"),
        }
    }

    #[test]
    fn percent_to_sizing() {
        let sizing: Sizing = <Percent as PanelSizing<Screen>>::to_sizing(Percent(0.25));
        assert!(matches!(sizing, Sizing::Percent(f) if (f - 0.25).abs() < 1e-6));
    }

    #[test]
    fn grow_to_sizing_is_unbounded_grow() {
        let sizing: Sizing = <Grow as PanelSizing<Screen>>::to_sizing(Grow);
        assert!(sizing.is_grow());
    }

    #[test]
    fn bare_f32_has_any_unit() {
        fn assert_any_unit<M: sealed::Mode, T: PanelSizing<M, Unit = AnyUnit>>() {}
        assert_any_unit::<Screen, f32>();
        assert_any_unit::<World, f32>();
        assert_any_unit::<World, Fit>();
    }

    #[test]
    fn physical_units_tagged_correctly() {
        fn assert_unit<M: sealed::Mode, T: PanelSizing<M, Unit = U>, U: sealed::Unit>() {}
        assert_unit::<Screen, Mm, Millimeters>();
        assert_unit::<World, Mm, Millimeters>();
        assert_unit::<Screen, Px, Pixels>();
        assert_unit::<World, In, Inches>();
        assert_unit::<World, Pt, Points>();
    }
}
