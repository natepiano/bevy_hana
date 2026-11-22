use hana_kana::Position;
use thiserror::Error;

/// Successful fit output: camera orbit radius and centered focus point.
#[derive(Debug, Clone, Copy)]
pub struct FitSolution {
    /// The optimal orbital radius.
    pub radius: f32,
    /// The centered focus point.
    pub focus:  Position,
}

/// Explicit fit calculation failures.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FitError {
    /// Camera viewport size/aspect ratio is unavailable.
    #[error("camera viewport size is unavailable")]
    NoViewport,
    /// All candidate fits projected points behind the camera.
    #[error("all candidate fits project points behind camera")]
    PointsBehindCamera,
    /// Projection variant is not supported (e.g. `Projection::Custom`).
    #[error("projection variant is not supported")]
    UnsupportedProjection,
}
