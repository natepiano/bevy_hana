//! Camera fit: framing a target's bounds in the viewport. Holds the fit-family
//! triggers (`ZoomToFit`, `AnimateToFit`, `LookAt`, `LookAtAndZoomToFit`,
//! `SetFitTarget`) with the observers that drive them, the projection geometry
//! the solve relies on, and the optional debug overlay.

mod camera_pose;
mod constants;
mod geometry;
mod target;
mod triggers;

#[cfg(feature = "fit_overlay")]
mod overlay;

use bevy::prelude::App;
use bevy::prelude::Plugin;
pub use geometry::FitAnchor;
#[cfg(feature = "fit_overlay")]
pub use overlay::FitOverlay;
#[cfg(feature = "fit_overlay")]
use overlay::FitOverlayPlugin;
#[cfg(feature = "fit_overlay")]
pub use overlay::FitTargetOverlayConfig;
pub use target::CurrentFitTarget;
pub use target::SetFitTarget;
pub use triggers::AnimateToFit;
pub use triggers::LookAt;
pub use triggers::LookAtAndZoomToFit;
pub use triggers::ZoomBegin;
pub use triggers::ZoomContext;
pub use triggers::ZoomEnd;
pub use triggers::ZoomReason;
pub use triggers::ZoomToFit;

/// `FitPlugin` registers the camera-fit domain's shared target lifecycle, unified fit/look
/// request observers, and the optional debug overlay when the `fit_overlay`
/// feature is enabled. The request observers are shared across camera kinds and
/// are also registered idempotently by the [`CameraKind`](crate::CameraKind)
/// compatibility hooks. The core fit solve is pure logic with nothing to
/// register.
pub(crate) struct FitPlugin;

impl Plugin for FitPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(target::on_set_fit_target);
        if !app.is_plugin_added::<UnifiedFitRequestObserversPlugin>() {
            app.add_plugins(UnifiedFitRequestObserversPlugin);
        }

        #[cfg(feature = "fit_overlay")]
        app.add_plugins(FitOverlayPlugin);
    }
}

/// `UnifiedFitRequestObserversPlugin` owns one observer for each public fit and look request.
pub(crate) struct UnifiedFitRequestObserversPlugin;

impl Plugin for UnifiedFitRequestObserversPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(triggers::on_animate_to_fit)
            .add_observer(triggers::on_zoom_to_fit)
            .add_observer(triggers::on_look_at)
            .add_observer(triggers::on_look_at_and_zoom_to_fit);
    }
}
