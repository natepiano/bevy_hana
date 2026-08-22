//! Built-in reusable sheet providers.
//!
//! [`QuadSheet`] and [`TriangleSheet`] are ordinary
//! [`ArrangementProvider`](crate::ArrangementProvider) implementations: they
//! enumerate deterministic logical cells, author one connection forest, and
//! retain row and column fold alternatives built from the sources of the
//! connections they just authored. Neither reads or writes a Bevy
//! [`World`](bevy_ecs::world::World).
//!
//! Hex sheets and box nets stay downstream. The two shipped sheets exist to
//! cover the rectangular cases every consumer needs; anything else is an
//! ordinary implementation of the same public trait.

mod quad;
mod sheet;
mod triangle;

pub use quad::QuadCell;
pub use quad::QuadFoldGroupSelection;
pub use quad::QuadSheet;
pub use triangle::TriangleCell;
pub use triangle::TriangleCellOrientation;
pub use triangle::TriangleFoldGroupSelection;
pub use triangle::TriangleSheet;
