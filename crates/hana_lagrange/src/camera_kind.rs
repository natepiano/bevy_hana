//! Compile-time identity for lagrange camera families.

use bevy::prelude::App;
use bevy::prelude::Component;

mod sealed {
    pub trait Sealed {}
}

use sealed::Sealed;

/// Type-family key for one complete lagrange camera kind.
///
/// Implementors are zero-sized marker types. The trait lists the behavior every
/// camera kind supplies: controller registration, `AnimateToFit` support,
/// `ZoomToFit` support, and `LookAt` / `LookAtAndZoomToFit` support. A camera
/// kind that omits one of them does not compile.
/// Generic input code extends the same kind key with input-specific associated
/// types.
/// The trait is sealed, so [`OrbitCamKind`] and [`FreeCamKind`] are the only
/// implementors.
///
/// [`FreeCamKind`]: crate::FreeCamKind
/// [`OrbitCamKind`]: crate::OrbitCamKind
pub trait CameraKind: Copy + Send + Sync + Sealed + 'static {
    /// The camera component for this camera family.
    type Camera: Component;

    /// Registers every required system for this camera kind.
    ///
    /// Camera plugins should call this default method instead of sequencing the
    /// individual registration methods themselves, so a method added to this
    /// trait later reaches every camera plugin at once.
    /// Fit and look requests are served by observers that do not vary by camera
    /// kind; the three registration methods below add the shared
    /// `UnifiedFitRequestObserversPlugin` only when the app does not already
    /// have it, so registering both camera kinds installs it once.
    fn add_camera_kind_systems(app: &mut App) {
        Self::add_controller_systems(app);
        Self::add_animate_to_fit_systems(app);
        Self::add_zoom_to_fit_systems(app);
        Self::add_look_at_systems(app);
        Self::add_camera_kind_support_systems(app);
    }

    /// Registers the camera's controller and required per-kind runtime systems.
    fn add_controller_systems(app: &mut App);

    /// Ensures the shared `AnimateToFit` request observer is registered.
    fn add_animate_to_fit_systems(app: &mut App);

    /// Ensures the shared `ZoomToFit` request observer is registered.
    fn add_zoom_to_fit_systems(app: &mut App);

    /// Ensures the shared `LookAt` / `LookAtAndZoomToFit` request observers are
    /// registered.
    fn add_look_at_systems(app: &mut App);

    /// Registers optional shared support systems used by this camera kind.
    ///
    /// This hook covers systems shared by several required behaviors on the
    /// same kind. It defaults to doing nothing; the behavior-specific methods
    /// above have no default and must be written out.
    fn add_camera_kind_support_systems(_: &mut App) {}
}

impl Sealed for crate::OrbitCamKind {}
impl Sealed for crate::FreeCamKind {}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::prelude::Assets;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::Mesh;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Projection;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;

    use super::*;
    use crate::AnimateToFit;
    use crate::AnimationRejected;
    use crate::AnimationRejectionReason;
    use crate::AnimationSource;
    use crate::CameraRequestPreparationError;
    use crate::FreeCam;
    use crate::FreeCamKind;
    use crate::LookAt;
    use crate::LookAtAndZoomToFit;
    use crate::OrbitCam;
    use crate::OrbitCamKind;
    use crate::ZoomToFit;
    use crate::animation::AnimationPlugin;
    use crate::fit::FitPlugin;
    use crate::input::InputPlugin;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct RecordedHigherCameraRequestRejection {
        camera: Entity,
        source: AnimationSource,
        target: Entity,
        error:  CameraRequestPreparationError,
    }

    #[derive(Resource, Default)]
    struct HigherCameraRequestRejections(Vec<RecordedHigherCameraRequestRejection>);

    fn record_higher_camera_request_rejection(
        rejected: On<AnimationRejected>,
        mut rejections: ResMut<HigherCameraRequestRejections>,
    ) {
        let AnimationRejectionReason::RequestPreparationFailed(error) = &rejected.reason else {
            panic!("expected a typed higher camera request preparation rejection");
        };
        let Some(target) = rejected.target else {
            panic!("a higher camera request rejection must preserve its target");
        };
        rejections.0.push(RecordedHigherCameraRequestRejection {
            camera: rejected.camera,
            source: rejected.source,
            target,
            error: *error,
        });
    }

    fn registration_test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AnimationPlugin, InputPlugin))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<HigherCameraRequestRejections>()
            .add_observer(record_higher_camera_request_rejection);
        app
    }

    fn spawn_orbit_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id()
    }

    fn spawn_free_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::default(),
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id()
    }

    fn trigger_fit_and_look_requests(app: &mut App, camera: Entity, target: Entity) {
        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| AnimateToFit::new(camera, target));
        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target));
        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAt::new(camera, target));
        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAtAndZoomToFit::new(camera, target));
        app.update();
    }

    fn assert_each_unified_request_was_rejected_once(app: &App, camera: Entity, target: Entity) {
        let rejection = |source| RecordedHigherCameraRequestRejection {
            camera,
            source,
            target,
            error: CameraRequestPreparationError::MissingTargetTransform,
        };
        assert_eq!(
            app.world().resource::<HigherCameraRequestRejections>().0,
            [
                rejection(AnimationSource::AnimateToFit),
                rejection(AnimationSource::ZoomToFit),
                rejection(AnimationSource::LookAt),
                rejection(AnimationSource::LookAtAndZoomToFit),
            ]
        );
    }

    #[test]
    fn direct_orbit_kind_registration_installs_unified_fit_and_look_observers() {
        let mut app = registration_test_app();
        OrbitCamKind::add_camera_kind_systems(&mut app);
        app.finish();
        let camera = spawn_orbit_camera(&mut app);
        let target = app.world_mut().spawn_empty().id();

        trigger_fit_and_look_requests(&mut app, camera, target);

        assert_each_unified_request_was_rejected_once(&app, camera, target);
    }

    #[test]
    fn direct_free_kind_registration_installs_unified_fit_and_look_observers() {
        let mut app = registration_test_app();
        FreeCamKind::add_camera_kind_systems(&mut app);
        app.finish();
        let camera = spawn_free_camera(&mut app);
        let target = app.world_mut().spawn_empty().id();

        trigger_fit_and_look_requests(&mut app, camera, target);

        assert_each_unified_request_was_rejected_once(&app, camera, target);
    }

    #[test]
    fn fit_plugin_and_both_camera_kinds_handle_each_request_once() {
        let mut app = registration_test_app();
        app.add_plugins(FitPlugin);
        OrbitCamKind::add_camera_kind_systems(&mut app);
        FreeCamKind::add_camera_kind_systems(&mut app);
        app.finish();
        let camera = spawn_orbit_camera(&mut app);
        let target = app.world_mut().spawn_empty().id();

        trigger_fit_and_look_requests(&mut app, camera, target);

        assert_each_unified_request_was_rejected_once(&app, camera, target);
    }
}
