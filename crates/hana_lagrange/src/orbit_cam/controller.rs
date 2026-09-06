use std::f32::consts::FRAC_PI_2;
use std::f32::consts::PI;
use std::f32::consts::TAU;

use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Virtual;

use super::OrbitCam;
use super::OrbitCamHomePose;
use super::OrbitCamInput;
use super::OrbitCamUpdateRequest;
use super::UpsideDownPolicy;
use super::constants::SCROLL_ZOOM_FACTOR;
use super::drag_state::CameraOrientation;
use super::drag_state::DragActivity;
use super::drag_state::OrbitDragState;
use super::orbit_transform;
use crate::CameraBasis;
use crate::CameraHomePending;
use crate::Initialization;
use crate::input::CameraInputSurfaceMetrics;
use crate::input::ResolvedCameraInputRoute;
use crate::operation::Limit;
use crate::operation::OrbitAngles;
use crate::operation::Radius;
use crate::time_source::TimeSource;

/// Aggregated camera input for a single frame.
struct CameraInput {
    orbit:        Vec2,
    pan:          Vec2,
    scroll_line:  f32,
    scroll_pixel: f32,
}

/// Whether an `apply_*_input` step changed the camera's target state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MotionStatus {
    Changed,
    Unchanged,
}

impl MotionStatus {
    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
            _ => Self::Changed,
        }
    }

    const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
}

/// Holds the orbit target's pitch within `[-PI/2, PI/2]` when the policy forbids
/// orbiting upside down; leaves it untouched when the policy allows it.
fn constrain_angles(angles: OrbitAngles, upside_down_policy: UpsideDownPolicy) -> OrbitAngles {
    match upside_down_policy {
        UpsideDownPolicy::Prevent => OrbitAngles {
            yaw:   angles.yaw,
            pitch: angles.pitch.clamp(-FRAC_PI_2, FRAC_PI_2),
        },
        UpsideDownPolicy::Allow => angles,
    }
}

/// Establishes the start pose on the first controller pass, applying all limits,
/// then marks the camera `Active`.
///
/// `FromPose` uses the pose already seeded into the operations; `FromTransform`
/// derives it from the entity's `Transform`.
pub(crate) fn initialize_orbit_cam(
    orbit_cam: &mut OrbitCam,
    basis: CameraBasis,
    transform: &mut Transform,
    projection: &mut Projection,
) {
    let (angles, radius, focus) = match orbit_cam.initialization {
        Initialization::FromPose => (
            orbit_cam.orbit.current(),
            orbit_cam.zoom.current(),
            orbit_cam.pan.current(),
        ),
        // FromTransform derives the pose from the entity's `Transform`. (`Active`
        // never reaches here — the caller guards on `!= Active`.)
        Initialization::FromTransform | Initialization::Active => {
            let focus = orbit_cam.pan.current();
            let (yaw, pitch, radius) = orbit_transform::calculate_from_translation_and_focus(
                transform.translation,
                focus.0,
                basis.axes(),
            );
            (OrbitAngles { yaw, pitch }, Radius(radius), focus)
        },
    };

    let angles = constrain_angles(
        orbit_cam.orbit.limit().constrain(angles),
        orbit_cam.upside_down_policy,
    );
    let radius = orbit_cam.zoom.limit().constrain(radius);
    let focus = orbit_cam.pan.limit().constrain(focus);

    orbit_cam.orbit.snap_to(angles);
    orbit_cam.zoom.snap_to(radius);
    orbit_cam.pan.snap_to(focus);

    orbit_transform::update_orbit_transform(
        angles.yaw,
        angles.pitch,
        radius.0,
        focus.0,
        transform,
        projection,
        basis.axes(),
    );

    orbit_cam.initialization = Initialization::Active;
}

/// Converts finalized semantic input into controller movement values.
fn collect_camera_input(orbit_cam: &OrbitCam, input: &OrbitCamInput) -> CameraInput {
    let mut camera_input = CameraInput {
        orbit:        Vec2::ZERO,
        pan:          Vec2::ZERO,
        scroll_line:  0.0,
        scroll_pixel: 0.0,
    };

    if input.has_orbit() {
        camera_input.orbit = input.orbit().pixels() * orbit_cam.orbit.sensitivity();
    }
    if input.has_pan() {
        camera_input.pan = input.pan().pixels() * orbit_cam.pan.sensitivity();
    }
    if input.has_zoom() {
        camera_input.scroll_line = input.zoom_coarse().amount() * orbit_cam.zoom.sensitivity();
        camera_input.scroll_pixel = input.zoom_smooth().amount() * orbit_cam.zoom.sensitivity();
    }

    camera_input
}

fn merged_surface_metrics(
    routed: Option<CameraInputSurfaceMetrics>,
    explicit: Option<CameraInputSurfaceMetrics>,
) -> CameraInputSurfaceMetrics {
    let mut metrics = routed.unwrap_or_default();
    if let Some(explicit) = explicit {
        if explicit.camera_view_size.is_some() {
            metrics.camera_view_size = explicit.camera_view_size;
        }
        if explicit.input_surface_size.is_some() {
            metrics.input_surface_size = explicit.input_surface_size;
        }
    }
    metrics
}

/// Applies orbit input to the orbit target. Returns `MotionStatus::Changed`
/// if the camera moved.
fn apply_orbit_input(
    orbit: Vec2,
    orbit_cam: &mut OrbitCam,
    drag_state: OrbitDragState,
    window_size: Option<Vec2>,
) -> MotionStatus {
    if orbit.length_squared() > 0.0 {
        // Use window size for rotation otherwise the sensitivity is far too high for small
        // viewports
        if let Some(window_size) = window_size {
            let delta_x = {
                let delta = orbit.x / window_size.x * TAU;
                match drag_state.orientation {
                    CameraOrientation::UpsideDown => -delta,
                    CameraOrientation::Normal => delta,
                }
            };
            let delta_y = orbit.y / window_size.y * PI;
            let mut target = orbit_cam.orbit.target();
            target.yaw -= delta_x;
            target.pitch += delta_y;
            orbit_cam.orbit.set_target(target);
            return MotionStatus::Changed;
        }
    }
    MotionStatus::Unchanged
}

/// Applies pan input to the focus target. Returns `MotionStatus::Changed` if
/// the camera moved.
fn apply_pan_input(
    mut pan: Vec2,
    orbit_cam: &mut OrbitCam,
    basis: CameraBasis,
    viewport_size: Option<Vec2>,
    transform: &Transform,
    projection: &Projection,
) -> MotionStatus {
    if pan.length_squared() > 0.0 {
        // Make panning distance independent of resolution and FOV
        if let Some(viewport_size) = viewport_size {
            let mut multiplier = 1.0;
            match *projection {
                Projection::Perspective(ref perspective_projection) => {
                    pan *= Vec2::new(
                        perspective_projection.fov * perspective_projection.aspect_ratio,
                        perspective_projection.fov,
                    ) / viewport_size;
                    // Make panning proportional to distance away from focus point
                    multiplier = orbit_cam.zoom.current().0;
                },
                Projection::Orthographic(ref orthographic_projection) => {
                    pan *= Vec2::new(
                        orthographic_projection.area.width(),
                        orthographic_projection.area.height(),
                    ) / viewport_size;
                },
                Projection::Custom(_) => todo!(),
            }
            // Translate by local axes
            let right = transform.rotation * basis.right * -pan.x;
            let up = transform.rotation * basis.up * pan.y;
            let translation = (right + up) * multiplier;
            orbit_cam
                .pan
                .set_target(orbit_cam.pan.target() + translation);
            return MotionStatus::Changed;
        }
    }
    MotionStatus::Unchanged
}

/// Applies scroll/zoom input to the radius target. Returns `MotionStatus::Changed`
/// if the camera moved.
//
// Multiplicative (exponential) zoom: one out-tick is the exact inverse of one
// in-tick at any radius. Additive `radius *= (1 ± k)` would feel symmetric
// for a single tick but compounds asymmetrically — zoom-out lags zoom-in
// once you're close.
fn apply_scroll_input(
    scroll_line: f32,
    scroll_pixel: f32,
    orbit_cam: &mut OrbitCam,
) -> MotionStatus {
    if (scroll_line + scroll_pixel).abs() > 0.0 {
        let line_factor = (-scroll_line * SCROLL_ZOOM_FACTOR).exp();
        let pixel_factor = (-scroll_pixel * SCROLL_ZOOM_FACTOR).exp();

        orbit_cam
            .zoom
            .set_target(orbit_cam.zoom.target() * (line_factor * pixel_factor));

        // Pixel-based scrolling is applied directly to the current value (already smooth)
        let snapped = orbit_cam
            .zoom
            .limit()
            .constrain(orbit_cam.zoom.current() * pixel_factor);
        orbit_cam.zoom.set_current(snapped);

        return MotionStatus::Changed;
    }
    MotionStatus::Unchanged
}

/// Eases the operations one frame and writes the resulting camera transform.
fn smooth_and_update_transform(
    orbit_cam: &mut OrbitCam,
    basis: CameraBasis,
    transform: &mut Transform,
    projection: &mut Projection,
    delta: f32,
) {
    orbit_cam.orbit.update(delta);
    orbit_cam.zoom.update(delta);
    orbit_cam.pan.update(delta);

    let angles = orbit_cam.orbit.current();
    orbit_transform::update_orbit_transform(
        angles.yaw,
        angles.pitch,
        orbit_cam.zoom.current().0,
        orbit_cam.pan.current().0,
        transform,
        projection,
        basis.axes(),
    );
}

/// Initializes each `OrbitCam` on its first pass, applies one frame of
/// `OrbitCamInput` to the orbit, pan, and zoom targets, and writes the camera
/// `Transform` when something changed.
pub(super) fn orbit_cam(
    route: Res<ResolvedCameraInputRoute>,
    mut orbit_cameras: Query<(
        Entity,
        &mut OrbitCam,
        &mut OrbitDragState,
        &OrbitCamInput,
        Option<&CameraInputSurfaceMetrics>,
        Ref<CameraBasis>,
        &mut Transform,
        &mut Projection,
        &TimeSource,
        Has<OrbitCamHomePose>,
    )>,
    time_real: Res<Time<Real>>,
    time_virt: Res<Time<Virtual>>,
    mut commands: Commands,
) {
    for (
        entity,
        mut orbit_cam,
        mut drag_state,
        input,
        explicit_metrics,
        basis,
        mut transform,
        mut projection,
        time_source,
        has_home,
    ) in &mut orbit_cameras
    {
        let basis_changed = basis.is_changed();
        let basis = *basis;

        if orbit_cam.initialization != Initialization::Active {
            initialize_orbit_cam(&mut orbit_cam, basis, &mut transform, &mut projection);
            if !has_home {
                commands
                    .entity(entity)
                    .insert(OrbitCamHomePose::from_current(&orbit_cam));
                if !input.has_input() {
                    commands.entity(entity).insert(CameraHomePending);
                }
            }
        }

        let input = collect_camera_input(&orbit_cam, input);
        let metrics = merged_surface_metrics(route.metrics_for(entity), explicit_metrics.copied());

        // Only check for upside down when orbiting started or ended this frame,
        // so we don't reverse the yaw direction while the user is still dragging
        let orbit_drag = DragActivity::from(input.orbit != Vec2::ZERO);
        if orbit_drag != drag_state.orbit_drag {
            let world_up = basis.up;
            drag_state.orientation = if transform.up().dot(world_up) < 0.0 {
                CameraOrientation::UpsideDown
            } else {
                CameraOrientation::Normal
            };
            drag_state.orbit_drag = orbit_drag;
        }

        let motion = apply_orbit_input(
            input.orbit,
            &mut orbit_cam,
            *drag_state,
            metrics.input_surface_size,
        )
        .merge(apply_pan_input(
            input.pan,
            &mut orbit_cam,
            basis,
            metrics.camera_view_size,
            &transform,
            &projection,
        ))
        .merge(apply_scroll_input(
            input.scroll_line,
            input.scroll_pixel,
            &mut orbit_cam,
        ));

        // Enforce the upside-down policy on the orbit target; the operations' own
        // limits (angle, radius, region) are applied inside `Operation::update`.
        let constrained = constrain_angles(orbit_cam.orbit.target(), orbit_cam.upside_down_policy);
        orbit_cam.orbit.set_target(constrained);

        let delta = match time_source {
            TimeSource::Real => time_real.delta_secs(),
            TimeSource::Virtual => time_virt.delta_secs(),
        };

        // Only pass `&mut transform` when something actually changed.
        // Passing it unconditionally triggers Bevy's `DerefMut` change detection,
        // marking `Transform` (and therefore `GlobalTransform`) as changed every
        // frame — even when the camera is idle.
        let update_request = orbit_cam.consume_update_request();
        let needs_update = motion.is_changed()
            || basis_changed
            || update_request == OrbitCamUpdateRequest::ForceUpdate
            || orbit_cam.orbit.target() != orbit_cam.orbit.current()
            || orbit_cam.zoom.target() != orbit_cam.zoom.current()
            || orbit_cam.pan.target() != orbit_cam.pan.current();

        if needs_update {
            smooth_and_update_transform(
                &mut orbit_cam,
                basis,
                &mut transform,
                &mut projection,
                delta,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::input::mouse::AccumulatedMouseScroll;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Vec3;
    use bevy::prelude::With;
    use bevy::window::WindowRef;

    use super::*;
    use crate::CameraInputPhase;
    use crate::LagrangePlugin;
    use crate::ManualInputSource;
    use crate::OrbitCamInputMode;
    use crate::OrbitCamManualInputWriter;
    use crate::input::InteractionSources;

    const START_FOCUS: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const START_YAW: f32 = 0.5;
    const START_PITCH: f32 = -0.25;
    const START_RADIUS: f32 = 6.0;
    const TEST_SURFACE_SIZE: Vec2 = Vec2::new(100.0, 100.0);

    type TestResult = Result<(), &'static str>;

    fn start_pose() -> OrbitCamHomePose {
        OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   START_YAW,
                pitch: START_PITCH,
            },
            pan:   crate::Focus(START_FOCUS),
            zoom:  Radius(START_RADIUS),
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>();
        app.finish();
        app
    }

    fn spawn_orbit_cam(app: &mut App, home: Option<OrbitCamHomePose>) -> Entity {
        let start = start_pose();
        let mut entity = app.world_mut().spawn((
            OrbitCam::from_pose(start.pan, start.orbit, start.zoom),
            OrbitCamInput::default(),
            OrbitCamInputMode::Manual,
            Camera::default(),
            RenderTarget::Window(WindowRef::Primary),
            Transform::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            CameraInputSurfaceMetrics::camera_view_and_input_surface(
                TEST_SURFACE_SIZE,
                TEST_SURFACE_SIZE,
            ),
            CameraBasis::Y_UP,
            TimeSource::Virtual,
        ));
        if let Some(home) = home {
            entity.insert(home);
        }
        entity.id()
    }

    fn home_for(app: &App, camera: Entity) -> Result<OrbitCamHomePose, &'static str> {
        app.world()
            .get::<OrbitCamHomePose>(camera)
            .copied()
            .ok_or("camera missing OrbitCamHomePose")
    }

    #[test]
    fn spawn_without_home_captures_provisional_home() -> TestResult {
        let mut app = test_app();
        let camera = spawn_orbit_cam(&mut app, None);

        app.update();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_some());
        let orbit_cam = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("camera missing OrbitCam")?;
        assert_eq!(orbit_cam.initialization, Initialization::Active);
        Ok(())
    }

    #[test]
    fn input_active_at_spawn_locks_home_immediately() -> TestResult {
        let mut app = test_app();
        app.add_systems(
            PreUpdate,
            mark_orbit_active_for_orbit_cams.in_set(CameraInputPhase::WriteManual),
        );
        let camera = spawn_orbit_cam(&mut app, None);

        app.update();

        assert_eq!(home_for(&app, camera)?, start_pose());
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    fn mark_orbit_active_for_orbit_cams(
        mut writer: OrbitCamManualInputWriter,
        cameras: Query<Entity, With<OrbitCam>>,
    ) {
        for camera in &cameras {
            if let Ok(mut input) = writer.get_mut(camera, ManualInputSource::observed_keyboard()) {
                input.mark_orbit_active();
            }
        }
    }

    #[test]
    fn collect_camera_input_scales_finalized_intent() {
        let mut orbit_cam = OrbitCam::default();
        orbit_cam.orbit.set_sensitivity(2.0);
        orbit_cam.pan.set_sensitivity(3.0);
        orbit_cam.zoom.set_sensitivity(4.0);
        let mut input = OrbitCamInput::default();
        input
            .add_orbit_with_sources(Vec2::new(1.0, 2.0), InteractionSources::MOUSE)
            .add_pan_with_sources(Vec2::new(3.0, 4.0), InteractionSources::MOUSE)
            .add_zoom_coarse_with_sources(5.0, InteractionSources::WHEEL)
            .add_zoom_smooth_with_sources(6.0, InteractionSources::SMOOTH_SCROLL);

        let input = collect_camera_input(&orbit_cam, &input);

        assert_eq!(input.orbit, Vec2::new(2.0, 4.0));
        assert_eq!(input.pan, Vec2::new(9.0, 12.0));
        assert!((input.scroll_line - 20.0).abs() <= f32::EPSILON);
        assert!((input.scroll_pixel - 24.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn explicit_surface_metrics_override_routed_metrics() {
        let routed = CameraInputSurfaceMetrics::camera_view_and_input_surface(
            Vec2::new(100.0, 200.0),
            Vec2::new(300.0, 400.0),
        );
        let explicit = CameraInputSurfaceMetrics::camera_view(Vec2::new(500.0, 600.0));

        let metrics = merged_surface_metrics(Some(routed), Some(explicit));

        assert_eq!(metrics.camera_view_size, Some(Vec2::new(500.0, 600.0)));
        assert_eq!(metrics.input_surface_size, Some(Vec2::new(300.0, 400.0)));
    }
}
