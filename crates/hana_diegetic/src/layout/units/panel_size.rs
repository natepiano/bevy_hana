use super::paper_size::PaperSize;
use super::unit::HasUnit;
use super::unit::Unit;

/// Trait for types that can provide panel dimensions with their unit.
///
/// Returns `(width, height, unit)` where width and height are in the
/// returned unit's coordinate space.
///
/// Implemented by:
/// - [`PaperSize`] — standard paper/card sizes in their natural unit
/// - Same-unit tuples like `(Mm(210.0), Mm(297.0))` — compiler rejects mixed units
/// - Bare `(f32, f32)` — defaults to [`Unit::Meters`]
pub trait PanelSize {
    /// Returns `(width, height, unit)`.
    fn dimensions(self) -> (f32, f32, Unit);
}

impl PanelSize for PaperSize {
    fn dimensions(self) -> (f32, f32, Unit) {
        let unit = self.native_unit();
        match unit {
            Unit::Millimeters => (self.width_mm(), self.height_mm(), unit),
            Unit::Inches => {
                let width = self.width_mm() * Unit::Millimeters.meters_per_unit()
                    / Unit::Inches.meters_per_unit();
                let height = self.height_mm() * Unit::Millimeters.meters_per_unit()
                    / Unit::Inches.meters_per_unit();
                (width, height, unit)
            },
            _ => (self.width_mm(), self.height_mm(), Unit::Millimeters),
        }
    }
}

/// Same-unit tuples: `(Mm(210.0), Mm(297.0))`, `(In(8.5), In(11.0))`, etc.
/// Mixed-unit tuples like `(Mm(210.0), In(11.0))` are a compile error.
impl<U: HasUnit> PanelSize for (U, U) {
    fn dimensions(self) -> (f32, f32, Unit) { (self.0.value(), self.1.value(), U::UNIT) }
}

/// Bare `(f32, f32)` tuples default to [`Unit::Meters`] (Bevy's world space).
impl PanelSize for (f32, f32) {
    fn dimensions(self) -> (f32, f32, Unit) { (self.0, self.1, Unit::Meters) }
}
