use bevy::prelude::Reflect;

use super::unit::HasUnit;
use super::unit::Mm;
use super::unit::Unit;

/// Standard paper and card sizes.
///
/// Each variant stores its dimensions in millimeters internally and
/// converts to meters (or any [`Unit`]) on request. Implements
/// [`PanelSize`](crate::layout::PanelSize) so it can be passed directly to
/// [`DiegeticPanelBuilder::size`](crate::DiegeticPanelBuilder::size).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum PaperSize {
    // ── ISO A series ─────────────────────────────────────────────────
    /// A0 — 841 × 1189 mm.
    A0,
    /// A1 — 594 × 841 mm.
    A1,
    /// A2 — 420 × 594 mm.
    A2,
    /// A3 — 297 × 420 mm.
    A3,
    /// A4 — 210 × 297 mm.
    A4,
    /// A5 — 148 × 210 mm.
    A5,
    /// A6 — 105 × 148 mm.
    A6,
    /// A7 — 74 × 105 mm.
    A7,
    /// A8 — 52 × 74 mm.
    A8,

    // ── ISO B series ─────────────────────────────────────────────────
    /// B0 — 1000 × 1414 mm.
    B0,
    /// B1 — 707 × 1000 mm.
    B1,
    /// B2 — 500 × 707 mm.
    B2,
    /// B3 — 353 × 500 mm.
    B3,
    /// B4 — 250 × 353 mm.
    B4,
    /// B5 — 176 × 250 mm.
    B5,

    // ── North American ───────────────────────────────────────────────
    /// US Letter — 8.5 × 11 inches (215.9 × 279.4 mm).
    USLetter,
    /// US Legal — 8.5 × 14 inches (215.9 × 355.6 mm).
    USLegal,
    /// US Ledger / Tabloid — 11 × 17 inches (279.4 × 431.8 mm).
    USLedger,
    /// US Executive — 7.25 × 10.5 inches (184.2 × 266.7 mm).
    USExecutive,

    // ── Cards ────────────────────────────────────────────────────────
    /// US Business Card — 3.5 × 2 inches (88.9 × 50.8 mm).
    BusinessCard,
    /// Index Card 3 × 5 inches (76.2 × 127.0 mm).
    IndexCard3x5,
    /// Index Card 4 × 6 inches (101.6 × 152.4 mm).
    IndexCard4x6,
    /// Index Card 5 × 8 inches (127.0 × 203.2 mm).
    IndexCard5x8,

    // ── Photo ────────────────────────────────────────────────────────
    /// Photo 4 × 6 inches (101.6 × 152.4 mm).
    Photo4x6,
    /// Photo 5 × 7 inches (127.0 × 177.8 mm).
    Photo5x7,
    /// Photo 8 × 10 inches (203.2 × 254.0 mm).
    Photo8x10,

    // ── Poster ───────────────────────────────────────────────────────
    /// Poster 18 × 24 inches (457.2 × 609.6 mm).
    Poster18x24,
    /// Poster 24 × 36 inches (609.6 × 914.4 mm).
    Poster24x36,
}

impl PaperSize {
    /// Width in millimeters (shorter dimension).
    #[must_use]
    pub const fn width_mm(self) -> f32 {
        match self {
            Self::A0 => 841.0,
            Self::A1 => 594.0,
            Self::A2 => 420.0,
            Self::A3 => 297.0,
            Self::A4 => 210.0,
            Self::A5 => 148.0,
            Self::A6 => 105.0,
            Self::A7 => 74.0,
            Self::A8 => 52.0,
            Self::B0 => 1000.0,
            Self::B1 => 707.0,
            Self::B2 => 500.0,
            Self::B3 => 353.0,
            Self::B4 => 250.0,
            Self::B5 => 176.0,
            Self::USLetter | Self::USLegal => 215.9,
            Self::USLedger => 279.4,
            Self::USExecutive => 184.15,
            Self::BusinessCard => 88.9,
            Self::IndexCard3x5 => 76.2,
            Self::IndexCard4x6 | Self::Photo4x6 => 101.6,
            Self::IndexCard5x8 | Self::Photo5x7 => 127.0,
            Self::Photo8x10 => 203.2,
            Self::Poster18x24 => 457.2,
            Self::Poster24x36 => 609.6,
        }
    }

    /// Height in millimeters (longer dimension).
    #[must_use]
    pub const fn height_mm(self) -> f32 {
        match self {
            Self::A0 => 1189.0,
            Self::A1 => 841.0,
            Self::A2 => 594.0,
            Self::A3 => 420.0,
            Self::A4 => 297.0,
            Self::A5 => 210.0,
            Self::A6 => 148.0,
            Self::A7 => 105.0,
            Self::A8 => 74.0,
            Self::B0 => 1414.0,
            Self::B1 => 1000.0,
            Self::B2 => 707.0,
            Self::B3 => 500.0,
            Self::B4 => 353.0,
            Self::B5 => 250.0,
            Self::USLetter => 279.4,
            Self::USLegal => 355.6,
            Self::USLedger => 431.8,
            Self::USExecutive => 266.7,
            Self::BusinessCard => 50.8,
            Self::IndexCard3x5 => 127.0,
            Self::IndexCard4x6 | Self::Photo4x6 => 152.4,
            Self::IndexCard5x8 => 203.2,
            Self::Photo5x7 => 177.8,
            Self::Photo8x10 => 254.0,
            Self::Poster18x24 => 609.6,
            Self::Poster24x36 => 914.4,
        }
    }

    /// The natural unit for this paper size.
    ///
    /// ISO and metric sizes return [`Unit::Millimeters`].
    /// North American, card, photo, and poster sizes return [`Unit::Inches`].
    #[must_use]
    pub const fn native_unit(self) -> Unit {
        match self {
            // ISO series — metric
            Self::A0
            | Self::A1
            | Self::A2
            | Self::A3
            | Self::A4
            | Self::A5
            | Self::A6
            | Self::A7
            | Self::A8
            | Self::B0
            | Self::B1
            | Self::B2
            | Self::B3
            | Self::B4
            | Self::B5 => Unit::Millimeters,

            // North American, cards, photos, posters — imperial
            Self::USLetter
            | Self::USLegal
            | Self::USLedger
            | Self::USExecutive
            | Self::BusinessCard
            | Self::IndexCard3x5
            | Self::IndexCard4x6
            | Self::IndexCard5x8
            | Self::Photo4x6
            | Self::Photo5x7
            | Self::Photo8x10
            | Self::Poster18x24
            | Self::Poster24x36 => Unit::Inches,
        }
    }

    /// Width in the given unit.
    ///
    /// Conversions involve floating-point arithmetic and may accumulate
    /// rounding error. For maximum precision, use [`width_mm`](Self::width_mm)
    /// directly for millimeter values.
    #[must_use]
    pub fn width_as<U: HasUnit>(&self) -> f32 {
        self.width_mm() * Unit::Millimeters.meters_per_unit() / U::UNIT.meters_per_unit()
    }

    /// Height in the given unit.
    ///
    /// Conversions involve floating-point arithmetic and may accumulate
    /// rounding error. For maximum precision, use [`height_mm`](Self::height_mm)
    /// directly for millimeter values.
    #[must_use]
    pub fn height_as<U: HasUnit>(&self) -> f32 {
        self.height_mm() * Unit::Millimeters.meters_per_unit() / U::UNIT.meters_per_unit()
    }

    /// Width in meters.
    #[deprecated(
        since = "0.1.0",
        note = "use `width_as::<Mm>()` or the native unit accessor"
    )]
    #[must_use]
    pub const fn width(self) -> f32 { self.width_mm() * 0.001 }

    /// Height in meters.
    #[deprecated(
        since = "0.1.0",
        note = "use `height_as::<Mm>()` or the native unit accessor"
    )]
    #[must_use]
    pub const fn height(self) -> f32 { self.height_mm() * 0.001 }

    /// Width in the given unit.
    #[deprecated(since = "0.1.0", note = "use `width_as::<U>()` instead")]
    #[must_use]
    pub fn width_in(self, unit: Unit) -> f32 {
        self.width_mm() * Unit::Millimeters.meters_per_unit() / unit.meters_per_unit()
    }

    /// Height in the given unit.
    #[deprecated(since = "0.1.0", note = "use `height_as::<U>()` instead")]
    #[must_use]
    pub fn height_in(self, unit: Unit) -> f32 {
        self.height_mm() * Unit::Millimeters.meters_per_unit() / unit.meters_per_unit()
    }

    /// Returns `(width, height)` in portrait orientation (taller than wide).
    ///
    /// If the paper is already portrait, returns as-is. If landscape
    /// (wider than tall), swaps width and height. Returns [`Mm`]
    /// newtypes so the result works directly with
    /// [`DiegeticPanelBuilder::size`](crate::DiegeticPanelBuilder::size).
    #[must_use]
    pub fn portrait(self) -> (Mm, Mm) {
        let (w, h) = (self.width_mm(), self.height_mm());
        if w > h {
            (Mm(h), Mm(w))
        } else {
            (Mm(w), Mm(h))
        }
    }

    /// Returns `(width, height)` in landscape orientation (wider than tall).
    ///
    /// If the paper is already landscape, returns as-is. If portrait
    /// (taller than wide), swaps width and height. Returns [`Mm`]
    /// newtypes so the result works directly with
    /// [`DiegeticPanelBuilder::size`](crate::DiegeticPanelBuilder::size).
    #[must_use]
    pub fn landscape(self) -> (Mm, Mm) {
        let (w, h) = (self.width_mm(), self.height_mm());
        if h > w {
            (Mm(h), Mm(w))
        } else {
            (Mm(w), Mm(h))
        }
    }
}
