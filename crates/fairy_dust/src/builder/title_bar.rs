//! `TitleBarBuilder` impls.

use std::marker::PhantomData;

use bevy::app::Plugins;
use bevy::ecs::change_detection::Tick;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::ScheduleSystem;
use bevy::ecs::system::SystemChangeTick;
use bevy::prelude::AppExit;
use bevy::prelude::Bundle;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::FromWorld;
use bevy::prelude::Last;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::With;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamPreset;

use super::CameraHomeBuilder;
use super::NoOrbitCam;
use super::PrimitiveBuilder;
use super::SprinkleBuilder;
use super::StudioLightingBuilder;
use super::WithOrbitCam;
use crate::OrbitCamPose;
use crate::cube_spin::CubeSpinConfig;
use crate::screen_panels::ControlActivation;
use crate::screen_panels::DescriptionPanel;
use crate::screen_panels::TitleBar;
use crate::screen_panels::TitleBarControlState;
use crate::screen_panels::TitleBarOverflowSystems;
use crate::screen_panels::TitleChipActivation;

/// Builder returned by [`SprinkleBuilder::with_title_bar`] for wiring chip
/// highlights to event lifecycles.
///
/// Chip-wiring methods are only reachable through this type, so calling
/// [`Self::wire_chip_to_events`] is a compile error when no title bar has
/// been installed.
///
/// Calling a non-wiring builder method finalizes the title bar configuration
/// and returns to the normal [`SprinkleBuilder`] chain.
pub struct TitleBarBuilder<S> {
    pub(super) parent: SprinkleBuilder<S>,
}

#[derive(Resource)]
struct WiredChipRead<R: Resource> {
    first_read_tick:  Tick,
    read_this_update: bool,
    warned:           bool,
    late_writes:      usize,
    resource:         PhantomData<fn() -> R>,
}

impl<R: Resource> Default for WiredChipRead<R> {
    fn default() -> Self {
        Self {
            first_read_tick:  Tick::default(),
            read_this_update: false,
            warned:           false,
            late_writes:      0,
            resource:         PhantomData,
        }
    }
}

/// Warns once when `R` changes after the first chip sync read it this update.
/// It runs in `Last`, so a `Last` system ordered after it, and a removal of
/// `R`, are outside its reach.
fn check_wired_chip_write<R: Resource>(
    state: Option<Res<R>>,
    mut read: ResMut<WiredChipRead<R>>,
    tick: SystemChangeTick,
) {
    if read.read_this_update {
        if let Some(state) = state
            && state
                .last_changed()
                .is_newer_than(read.first_read_tick, tick.this_run())
        {
            read.late_writes += 1;
            if !read.warned {
                bevy::log::warn!(
                    "{} changed after its title-bar chip sync; put its writer in TitleBarStateWriters",
                    std::any::type_name::<R>()
                );
                read.warned = true;
            }
        }
        read.read_this_update = false;
    }
}

impl<S> TitleBarBuilder<S> {
    /// Toggles `chip` active on `Begin` and inactive on `End` for any event
    /// of those types. Use this when only one chip responds to the event.
    /// For multi-chip discrimination (e.g. `AnimationBegin` shared by
    /// `LookAt` and `LookAtAndZoomToFit`), use
    /// [`Self::wire_chip_to_events_filtered`] instead.
    #[must_use]
    pub fn wire_chip_to_events<Begin, End>(mut self, chip: impl Into<String>) -> Self
    where
        Begin: bevy::ecs::event::Event,
        End: bevy::ecs::event::Event,
    {
        let chip = chip.into();
        let activate = chip.clone();
        self.parent.app.add_observer(
            move |_: On<Begin>, mut bars: Query<&mut TitleBarControlState>| {
                for mut bar in &mut bars {
                    bar.set_active(&activate, ControlActivation::Active);
                }
            },
        );
        let deactivate = chip;
        self.parent.app.add_observer(
            move |_: On<End>, mut bars: Query<&mut TitleBarControlState>| {
                for mut bar in &mut bars {
                    bar.set_active(&deactivate, ControlActivation::Inactive);
                }
            },
        );
        self
    }

    /// Mirrors the activation state of `chip` onto the value of `R`. The
    /// extractor maps the current resource value to
    /// [`ControlActivation::Active`] or [`ControlActivation::Inactive`]; the
    /// chip is updated whenever `R` changes (including the first frame after
    /// the resource is inserted). Use this for sticky toggles like
    /// "debug outlines on/off" — pair an enum resource with a closure that
    /// matches on its variants. Put other systems that write `R` in
    /// [`crate::TitleBarStateWriters`] for a redraw in the same update.
    /// Fairy Dust shortcut handlers already run before the chip sync; callers
    /// never join them to that set by hand.
    #[must_use]
    pub fn wire_chip_to_state<R, F>(mut self, chip: impl Into<String>, extractor: F) -> Self
    where
        R: Resource,
        F: Fn(&R) -> ControlActivation + Send + Sync + 'static,
    {
        let chip = chip.into();
        if !self
            .parent
            .app
            .world()
            .contains_resource::<WiredChipRead<R>>()
        {
            self.parent.app.init_resource::<WiredChipRead<R>>();
            self.parent
                .app
                .add_systems(Last, check_wired_chip_write::<R>);
        }
        self.parent.app.add_systems(
            Update,
            (move |state: Option<Res<R>>,
                   mut bars: Query<&mut TitleBarControlState>,
                   mut read: ResMut<WiredChipRead<R>>,
                   tick: SystemChangeTick| {
                if !read.read_this_update {
                    read.first_read_tick = tick.this_run();
                    read.read_this_update = true;
                }
                let Some(state) = state else { return };
                if !state.is_changed() {
                    return;
                }
                let activation = extractor(&state);
                for mut bar in &mut bars {
                    bar.set_active(&chip, activation);
                }
            })
            .in_set(TitleBarOverflowSystems::WriteState),
        );
        self
    }

    /// Mirrors the activation state of `chip` onto a resource that implements
    /// [`TitleChipActivation`]. Use this only for one-resource / one-chip state.
    #[must_use]
    pub fn wire_chip_to_activation<R>(self, chip: impl Into<String>) -> Self
    where
        R: Resource + TitleChipActivation,
    {
        self.wire_chip_to_state::<R, _>(chip, TitleChipActivation::activation)
    }

    /// Like [`Self::wire_chip_to_events`], but each filter reports whether a
    /// given event applies to this chip. Return `false` to skip it.
    #[must_use]
    pub fn wire_chip_to_events_filtered<Begin, End, FStart, FEnd>(
        mut self,
        chip: impl Into<String>,
        start_filter: FStart,
        end_filter: FEnd,
    ) -> Self
    where
        Begin: bevy::ecs::event::Event,
        End: bevy::ecs::event::Event,
        FStart: Fn(&Begin) -> bool + Send + Sync + 'static,
        FEnd: Fn(&End) -> bool + Send + Sync + 'static,
    {
        let chip = chip.into();
        let activate = chip.clone();
        self.parent.app.add_observer(
            move |trigger: On<Begin>, mut bars: Query<&mut TitleBarControlState>| {
                if !start_filter(&trigger) {
                    return;
                }
                for mut bar in &mut bars {
                    bar.set_active(&activate, ControlActivation::Active);
                }
            },
        );
        let deactivate = chip;
        self.parent.app.add_observer(
            move |trigger: On<End>, mut bars: Query<&mut TitleBarControlState>| {
                if !end_filter(&trigger) {
                    return;
                }
                for mut bar in &mut bars {
                    bar.set_active(&deactivate, ControlActivation::Inactive);
                }
            },
        );
        self
    }

    /// Toggles `chip` active while a fit animation frames an entity carrying
    /// marker `M`, and inactive when it ends.
    ///
    /// `AnimateToFit`, `LookAt`, and `ZoomToFit` all carry the framed `target`
    /// on their lifecycle events; matching on `M` is what distinguishes a
    /// caller's fit from the built-in Home fit, so a chip wired this way lights
    /// only for fits aimed at `M`-marked entities — never the Home pose, which
    /// frames its own internal cube.
    #[must_use]
    pub fn wire_chip_to_fit_target<M: Component>(mut self, chip: impl Into<String>) -> Self {
        let chip = chip.into();
        let activate = chip.clone();
        self.parent.app.add_observer(
            move |trigger: On<AnimationBegin>,
                  targets: Query<(), With<M>>,
                  mut bars: Query<&mut TitleBarControlState>| {
                let Some(target) = trigger.target else { return };
                if targets.get(target).is_err() {
                    return;
                }
                for mut bar in &mut bars {
                    bar.set_active(&activate, ControlActivation::Active);
                }
            },
        );
        let deactivate = chip;
        self.parent.app.add_observer(
            move |trigger: On<AnimationEnd>,
                  targets: Query<(), With<M>>,
                  mut bars: Query<&mut TitleBarControlState>| {
                let Some(target) = trigger.target else { return };
                if targets.get(target).is_err() {
                    return;
                }
                for mut bar in &mut bars {
                    bar.set_active(&deactivate, ControlActivation::Inactive);
                }
            },
        );
        self
    }

    fn finish(self) -> SprinkleBuilder<S> { self.parent }

    /// Finalizes the title bar and starts configuring a ground plane.
    #[must_use]
    pub fn with_ground_plane(self) -> PrimitiveBuilder<S> { self.finish().with_ground_plane() }

    /// Finalizes the title bar and starts configuring a cube.
    #[must_use]
    pub fn with_cube(self) -> PrimitiveBuilder<S> { self.finish().with_cube() }

    /// Finalizes the title bar and adds window position persistence.
    #[must_use]
    pub fn with_save_window_position(self) -> SprinkleBuilder<S> {
        self.finish().with_save_window_position()
    }

    /// Finalizes the title bar and adds BRP extras.
    #[must_use]
    pub fn with_brp_extras(self) -> SprinkleBuilder<S> { self.finish().with_brp_extras() }

    /// Finalizes the title bar and adds the camera control panel.
    #[must_use]
    pub fn with_camera_control_panel(self) -> SprinkleBuilder<S> {
        self.finish().with_camera_control_panel()
    }

    /// Finalizes the title bar and adds a marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin<M: Component>(self) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin::<M>()
    }

    /// Finalizes the title bar and adds a customized marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin_config<M: Component>(self, config: CubeSpinConfig) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin_config::<M>(config)
    }

    /// Finalizes the title bar and adds studio lighting.
    #[must_use]
    pub fn with_studio_lighting(self) -> StudioLightingBuilder<S> {
        self.finish().with_studio_lighting()
    }

    /// Finalizes the title bar and adds an example description panel.
    #[must_use]
    pub fn with_description_panel(self, panel: DescriptionPanel) -> SprinkleBuilder<S> {
        self.finish().with_description_panel(panel)
    }

    /// Finalizes the title bar and installs another title bar.
    #[must_use]
    pub fn with_title_bar(self, title_bar: TitleBar) -> Self {
        self.finish().with_title_bar(title_bar)
    }

    /// Finalizes the title bar and starts configuring a camera home pose.
    #[must_use]
    pub fn with_camera_home(self) -> CameraHomeBuilder<S> { self.finish().with_camera_home() }

    /// Finalizes the title bar and mirrors [`App::add_plugins`](bevy::app::App::add_plugins).
    #[must_use]
    pub fn add_plugins<M>(self, plugins: impl Plugins<M>) -> SprinkleBuilder<S> {
        self.finish().add_plugins(plugins)
    }

    /// Finalizes the title bar and mirrors [`App::add_systems`](bevy::app::App::add_systems).
    #[must_use]
    pub fn add_systems<M>(
        self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> SprinkleBuilder<S> {
        self.finish().add_systems(schedule, systems)
    }

    /// Finalizes the title bar and mirrors [`App::add_observer`](bevy::app::App::add_observer).
    #[must_use]
    pub fn add_observer<M, I>(self, observer: I) -> SprinkleBuilder<S>
    where
        I: bevy::ecs::observer::IntoObserver<M>,
    {
        self.finish().add_observer(observer)
    }

    /// Finalizes the title bar and mirrors [`App::init_resource`](bevy::app::App::init_resource).
    #[must_use]
    pub fn init_resource<R: Resource + FromWorld>(self) -> SprinkleBuilder<S> {
        self.finish().init_resource::<R>()
    }

    /// Finalizes the title bar and mirrors
    /// [`App::insert_resource`](bevy::app::App::insert_resource).
    #[must_use]
    pub fn insert_resource<R: Resource>(self, resource: R) -> SprinkleBuilder<S> {
        self.finish().insert_resource(resource)
    }

    /// Finalizes the title bar and runs the configured app.
    pub fn run(self) -> AppExit { self.finish().run() }
}

impl TitleBarBuilder<NoOrbitCam> {
    /// Finalizes the title bar, adds `LagrangePlugin`, and spawns an `OrbitCam`.
    pub fn with_orbit_cam_configured<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_configured(configure)
    }

    /// Finalizes the title bar, adds `LagrangePlugin`, spawns an `OrbitCam`,
    /// and inserts extra camera-side components.
    pub fn with_orbit_cam<F, B>(self, configure: F, bundle: B) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam(configure, bundle)
    }

    /// Finalizes the title bar, spawns an `OrbitCam`, and installs one
    /// built-in input preset.
    pub fn with_orbit_cam_preset<F>(
        self,
        configure: F,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_preset(configure, preset)
    }

    /// Finalizes the title bar, spawns an `OrbitCam` with an explicit startup
    /// pose, and installs one built-in input preset.
    pub fn with_orbit_cam_preset_pose(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_orbit_cam_preset_pose(pose, preset)
    }

    /// Finalizes the title bar, spawns an `OrbitCam`, installs one built-in
    /// input preset, and inserts extra camera-side components.
    pub fn with_orbit_cam_preset_bundle<F, B>(
        self,
        configure: F,
        preset: impl Into<OrbitCamPreset>,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_preset_bundle(configure, preset, bundle)
    }

    /// Finalizes the title bar, spawns an `OrbitCam` with an explicit startup
    /// pose, installs one built-in input preset, and inserts extra camera-side
    /// components.
    pub fn with_orbit_cam_preset_pose_bundle<B>(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_preset_pose_bundle(pose, preset, bundle)
    }

    /// Finalizes the title bar, spawns an `OrbitCam`, and installs app-owned
    /// input bindings.
    pub fn with_orbit_cam_bindings<F>(
        self,
        configure: F,
        bindings: OrbitCamBindings,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_bindings(configure, bindings)
    }

    /// Finalizes the title bar, spawns an `OrbitCam`, installs app-owned input
    /// bindings, and inserts extra camera-side components.
    pub fn with_orbit_cam_bindings_bundle<F, B>(
        self,
        configure: F,
        bindings: OrbitCamBindings,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_bindings_bundle(configure, bindings, bundle)
    }

    /// Finalizes the title bar and spawns a manually driven `OrbitCam`.
    pub fn with_orbit_cam_manual<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_manual(configure)
    }

    /// Finalizes the title bar, spawns a manually driven `OrbitCam`, and
    /// inserts extra camera-side components.
    pub fn with_orbit_cam_manual_bundle<F, B>(
        self,
        configure: F,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_manual_bundle(configure, bundle)
    }
}

impl TitleBarBuilder<WithOrbitCam> {
    /// Finalizes the title bar and makes the restart camera animation available
    /// through [`crate::RestoreWindowAnimation`].
    #[must_use]
    pub fn with_restore_camera_on_restart(self) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_restore_camera_on_restart()
    }

    /// Finalizes the title bar and adds stable transparency to the spawned
    /// `OrbitCam`.
    #[must_use]
    pub fn with_stable_transparency(self) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_stable_transparency()
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::InputSystems;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Commands;
    use bevy::prelude::KeyCode;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use hana_diegetic::DiegeticPanel;

    use super::*;
    use crate::TitleBarStateWriters;

    #[derive(Resource, Default)]
    struct ModeActive(bool);

    #[derive(Resource, Default)]
    struct ChangeModeOnUpdate(bool);

    fn change_mode_on_update(change: Res<ChangeModeOnUpdate>, mut commands: Commands) {
        if change.0 {
            commands.insert_resource(ModeActive(true));
        }
    }

    fn set_mode(change: Res<ChangeModeOnUpdate>, mut mode: ResMut<ModeActive>) {
        if change.0 {
            mode.0 = true;
        }
    }

    fn toggle_mode(mut mode: ResMut<ModeActive>) { mode.0 = !mode.0; }

    fn wired_test_app() -> App {
        crate::sprinkle_example()
            .with_title_bar(TitleBar::new().with_title("State").control("M Mode"))
            .wire_chip_to_state::<ModeActive, _>("M Mode", |mode| {
                if mode.0 {
                    ControlActivation::Active
                } else {
                    ControlActivation::Inactive
                }
            })
            .init_resource::<ModeActive>()
            .init_resource::<ChangeModeOnUpdate>()
            .build()
    }

    fn late_writes(app: &App) -> usize {
        app.world()
            .resource::<WiredChipRead<ModeActive>>()
            .late_writes
    }

    #[test]
    fn update_writer_after_chip_sync_is_detected() {
        let mut app = wired_test_app();
        app.add_systems(Update, set_mode.after(TitleBarOverflowSystems::Read));
        app.update();
        app.world_mut().resource_mut::<ChangeModeOnUpdate>().0 = true;
        app.update();
        assert_eq!(late_writes(&app), 1);
    }

    #[test]
    fn joined_update_writer_does_not_trigger_late_write_check() {
        let mut app = wired_test_app();
        app.add_systems(Update, set_mode.in_set(TitleBarStateWriters));
        app.update();
        app.world_mut().resource_mut::<ChangeModeOnUpdate>().0 = true;
        app.update();
        assert_eq!(late_writes(&app), 0);
    }

    #[test]
    fn pre_update_writer_does_not_trigger_late_write_check() {
        let mut app = wired_test_app();
        app.add_systems(PreUpdate, set_mode);
        app.update();
        app.world_mut().resource_mut::<ChangeModeOnUpdate>().0 = true;
        app.update();
        assert_eq!(late_writes(&app), 0);
    }

    #[test]
    fn resource_change_between_updates_does_not_trigger_late_write_check() {
        let mut app = wired_test_app();
        app.update();
        app.world_mut().resource_mut::<ModeActive>().0 = true;
        app.update();
        assert_eq!(late_writes(&app), 0);
    }

    #[test]
    fn shortcut_updates_wired_chip_without_a_late_write() -> Result<(), String> {
        let mut app = crate::sprinkle_example()
            .with_title_bar(TitleBar::new().with_title("State").control("M Mode"))
            .wire_chip_to_state::<ModeActive, _>("M Mode", |mode| {
                if mode.0 {
                    ControlActivation::Active
                } else {
                    ControlActivation::Inactive
                }
            })
            .init_resource::<ModeActive>()
            .with_shortcut(KeyCode::KeyJ, toggle_mode)
            .build();
        app.update();

        app.add_systems(
            PreUpdate,
            (|mut keys: ResMut<ButtonInput<KeyCode>>| keys.press(KeyCode::KeyJ))
                .after(InputSystems),
        );
        app.update();

        assert!(app.world().resource::<ModeActive>().0);
        assert_eq!(late_writes(&app), 0);
        let mut states = app.world_mut().query::<&TitleBarControlState>();
        assert_eq!(
            states
                .single(app.world())
                .map_err(|error| error.to_string())?
                .activation("M Mode"),
            ControlActivation::Active
        );
        Ok(())
    }

    #[test]
    fn wired_resource_redraws_its_chip_in_the_same_update() -> Result<(), String> {
        let mut app = crate::sprinkle_example()
            .with_title_bar(TitleBar::new().with_title("State").control("M Mode"))
            .wire_chip_to_state::<ModeActive, _>("M Mode", |mode| {
                if mode.0 {
                    ControlActivation::Active
                } else {
                    ControlActivation::Inactive
                }
            })
            .init_resource::<ModeActive>()
            .init_resource::<ChangeModeOnUpdate>()
            .add_systems(Update, change_mode_on_update.in_set(TitleBarStateWriters))
            .build();
        app.update();
        let mut panels = app.world_mut().query::<(&TitleBar, &DiegeticPanel)>();
        let before = format!(
            "{:?}",
            panels
                .single(app.world())
                .map_err(|error| error.to_string())?
                .1
                .tree()
        );

        app.world_mut().resource_mut::<ChangeModeOnUpdate>().0 = true;
        app.update();
        let after = format!(
            "{:?}",
            panels
                .single(app.world())
                .map_err(|error| error.to_string())?
                .1
                .tree()
        );
        assert_ne!(
            before, after,
            "chip tree did not redraw with resource change"
        );
        let mut states = app.world_mut().query::<&TitleBarControlState>();
        assert_eq!(
            states
                .single(app.world())
                .map_err(|error| error.to_string())?
                .activation("M Mode"),
            ControlActivation::Active
        );
        Ok(())
    }
}
