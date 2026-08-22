//! `SprinkleBuilder` impls — state-agnostic, `NoOrbitCam`, and `WithOrbitCam`.

use std::marker::PhantomData;

use bevy::app::App;
use bevy::app::Plugins;
use bevy::asset::AssetPlugin;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::AppExit;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::FromWorld;
use bevy::prelude::IntoSystem;
use bevy::prelude::KeyCode;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PresentMode;
use bevy::window::PrimaryWindow;
use bevy::winit::WinitSettings;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;

use super::CameraHomeBuilder;
use super::PrimitiveBuilder;
use super::StudioLightingBuilder;
use super::TitleBarBuilder;
use crate::Anchor;
use crate::bloom;
use crate::brp_extras;
use crate::camera_control_panel;
use crate::camera_control_panel::CameraControlPanelBackground;
use crate::camera_control_panel::CameraPresetSwitching;
use crate::camera_home::CameraHomeConfig;
use crate::camera_home::HomeTitleBarControl;
use crate::command_palette;
use crate::cube_spin;
use crate::cube_spin::CubeSpinConfig;
use crate::environment_map;
use crate::fold_controls;
use crate::hdr;
use crate::keymap;
use crate::keymap::CommandPaletteKeymap;
use crate::lighting::StudioLightingConfig;
use crate::orbit_cam;
use crate::orbit_cam::OrbitCamPose;
use crate::primitive::PrimitiveConfig;
use crate::restart;
use crate::restart_camera;
use crate::save_window_position;
use crate::screen_panels;
use crate::screen_panels::DescriptionPanel;
use crate::screen_panels::TitleBar;
use crate::shortcuts;
use crate::transparency;
use crate::unclamp;

/// Typestate marker: the builder has not yet spawned an `OrbitCam`.
///
/// Camera-attached capabilities are not defined for `SprinkleBuilder<NoOrbitCam>`,
/// so calling them is a compile error.
pub struct NoOrbitCam;

/// Typestate marker: the builder has spawned an `OrbitCam`.
///
/// Reached via [`SprinkleBuilder::with_orbit_cam_configured`]. Camera-attached
/// capabilities like [`SprinkleBuilder::with_stable_transparency`]
/// become callable in this state.
pub struct WithOrbitCam;

/// Typestate marker: the Fairy Dust baseline has not been installed yet.
///
/// Only this state exposes [`SprinkleBuilder::with_asset_root`]. Every other
/// builder operation installs the baseline with Bevy's default asset root and
/// returns [`BaselineInstalled`].
pub struct AssetRootPending;

/// Typestate marker: `DefaultPlugins` and the Fairy Dust baseline are installed.
pub struct BaselineInstalled;

enum BaselineStatus {
    Pending,
    Installed,
}

/// Builder returned by [`sprinkle_example`](crate::sprinkle_example). State-agnostic capability
/// methods are defined for any `S`; camera-attached methods are gated by
/// the typestate.
pub struct SprinkleBuilder<S, B = BaselineInstalled> {
    pub(super) app:  App,
    baseline_status: BaselineStatus,
    orbit:           PhantomData<S>,
    baseline:        PhantomData<B>,
}

// State-agnostic capabilities — available regardless of whether an `OrbitCam`
// has been configured.
impl<S, Baseline> SprinkleBuilder<S, Baseline> {
    fn into_installed(mut self) -> SprinkleBuilder<S> {
        if matches!(self.baseline_status, BaselineStatus::Pending) {
            crate::install_baseline(&mut self.app, AssetPlugin::default());
        }
        SprinkleBuilder {
            app:             self.app,
            baseline_status: BaselineStatus::Installed,
            orbit:           PhantomData,
            baseline:        PhantomData,
        }
    }

    /// Installs Hana fold playback with the standard `Space` fold,
    /// `Shift+Space` unfold, and `P` play controls.
    ///
    /// At a terminal, `P` selects the other endpoint. While idle in the
    /// interior, it follows the latest step direction; during a step, it
    /// continues that direction to the terminal; during play, it reverses
    /// immediately.
    #[must_use]
    pub fn with_fold_controls(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        fold_controls::install(&mut builder.app);
        builder
    }

    /// Add a `hana_clerestory` `WindowManagerPlugin` so window position
    /// and size are persisted across runs.
    #[must_use]
    pub fn with_save_window_position(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        save_window_position::install(&mut builder.app);
        builder
    }

    /// Add a `bevy_brp_extras` `BrpExtrasPlugin` configured to display the
    /// BRP port in the window title when the port is non-default.
    #[must_use]
    pub fn with_brp_extras(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        brp_extras::install(&mut builder.app);
        builder
    }

    /// Enable HDR output on every camera (current and later-spawned). Required
    /// for over-bright (>1.0) colors to survive a multi-camera diegetic render
    /// chain — any camera left in LDR clamps them at that step.
    #[must_use]
    pub fn with_hdr(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        hdr::install(&mut builder.app);
        builder
    }

    /// Uncap the frame rate so a stress example reports its true per-frame
    /// cost. Sets the primary window to [`PresentMode::AutoNoVsync`] and swaps
    /// [`WinitSettings`] to [`continuous`](WinitSettings::continuous).
    ///
    /// The Bevy defaults hide real cost two ways: vsync (`Fifo`) pins frame
    /// time to display-refresh steps (120 / 60 / 40 fps on a 120 Hz panel),
    /// and the default [`WinitSettings::game`] throttles an unfocused window to
    /// 60 Hz reactive-low-power. With both removed, the on-screen overlay and a
    /// background BRP reader both see un-throttled frame time.
    #[must_use]
    pub fn with_perf_mode(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.insert_resource(WinitSettings::continuous());
        let mut windows = builder
            .app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        if let Ok(mut window) = windows.single_mut(builder.app.world_mut()) {
            window.present_mode = PresentMode::AutoNoVsync;
        }
        builder
    }

    /// Enable smart screen-space camera control panels for `OrbitCam` cameras.
    ///
    /// Cameras without an explicit [`CameraGuidance`](crate::CameraGuidance) component get
    /// [`CameraGuidance::auto()`](crate::CameraGuidance::auto), so the panel reflects the effective
    /// preset or binding configuration and highlights active interactions.
    #[must_use]
    pub fn with_camera_control_panel(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        camera_control_panel::install(&mut builder.app);
        builder
    }

    /// Enable the command palette: a searchable box listing every declared
    /// keymap command, opened with `Cmd+P` on macOS and `Ctrl+P` elsewhere.
    ///
    /// Only the box is added here. `hana_rubric`'s `KeymapPlugin` is installed
    /// by the baseline either way, because Fairy Dust's own chords — restart,
    /// the help overlay, the preset cycle — are declared commands the keymap
    /// dispatches. This names Fairy Dust's shipped document as the keymap
    /// choice, the same way
    /// [`with_command_palette_keymap`](Self::with_command_palette_keymap) names
    /// its own, so calling both with different documents is refused rather than
    /// silently running one of them. It names no application, so no keymap disk
    /// worker starts and nothing writes to the developer's configuration
    /// directory; the palette renders that unresolved configuration directory as
    /// an advisory row.
    ///
    /// An application that declares its own commands, or that wants the user's
    /// keymap read from disk, supplies its own
    /// [`CommandPaletteKeymap`](crate::CommandPaletteKeymap) with
    /// [`with_command_palette_keymap`](Self::with_command_palette_keymap).
    #[must_use]
    pub fn with_command_palette(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        keymap::configure(&mut builder.app, CommandPaletteKeymap::default());
        command_palette::install(&mut builder.app);
        builder
    }

    /// Enable the command palette against `keymap` instead of Fairy Dust's
    /// shipped defaults. Use this when the application declares its own
    /// commands, so the embedded document binds only ids the command registry
    /// knows — a document naming an unregistered id is rejected whole, which
    /// leaves the palette unopenable.
    ///
    /// The supplied document replaces Fairy Dust's shipped one, so it must bind
    /// the `fairy_dust::` commands it wants to keep as well as its own ids.
    ///
    /// Call this or [`with_command_palette`](Self::with_command_palette) once. A
    /// second call carrying a different keymap panics rather than silently
    /// keeping the first, because an application running bindings it never asked
    /// for is worse than a crash at startup.
    #[must_use]
    pub fn with_command_palette_keymap(self, keymap: CommandPaletteKeymap) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        keymap::configure(&mut builder.app, keymap);
        command_palette::install(&mut builder.app);
        builder
    }

    /// Pins the camera to its spawned preset: suppresses the Shift+C cycle and
    /// its entry in the keyboard-shortcut overlay. Pair with
    /// [`with_camera_control_panel`](Self::with_camera_control_panel).
    #[must_use]
    pub fn lock_camera_preset(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.insert_resource(CameraPresetSwitching::Disabled);
        builder
    }

    /// Overrides the inner background color of the camera control panel.
    /// Pair with [`with_camera_control_panel`](Self::with_camera_control_panel).
    /// Use [`DEFAULT_PANEL_BACKGROUND`](crate::DEFAULT_PANEL_BACKGROUND) and
    /// [`Color::with_alpha`](bevy::color::Alpha::with_alpha) to tweak only the opacity:
    /// `.with_camera_control_panel_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.85))`.
    #[must_use]
    pub fn with_camera_control_panel_background_color(self, color: Color) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder
            .app
            .insert_resource(CameraControlPanelBackground(color));
        builder
    }

    /// Wire a one-shot keyboard shortcut: pressing `key` runs `system` once.
    /// The example writes only a plain Bevy system — no input macros, no input
    /// crate import:
    ///
    /// ```ignore
    /// fairy_dust::sprinkle_example()
    ///     .with_shortcut(KeyCode::KeyL, look_at)
    ///     .run();
    ///
    /// fn look_at(camera: Query<Entity, With<OrbitCam>>, /* ... */) { /* ... */ }
    /// ```
    ///
    /// The shortcut fires only while no modifier is held, so a bare key stays
    /// quiet during `Ctrl`/`Shift`/`Alt`/`Cmd` and Fairy Dust's modifier chords
    /// (`Ctrl+Shift+L` and friends) reach only their own action.
    ///
    /// Reusing a key Fairy Dust already binds bare (`H` home with
    /// [`with_camera_home`](Self::with_camera_home), `P` with
    /// [`with_cube_spin`](Self::with_cube_spin) or
    /// [`with_fold_controls`](Self::with_fold_controls)) fails at startup —
    /// use the matching capability instead of a manual shortcut.
    #[must_use]
    pub fn with_shortcut<Sys, M>(self, key: KeyCode, system: Sys) -> SprinkleBuilder<S>
    where
        Sys: IntoSystem<(), (), M> + 'static,
    {
        let mut builder = self.into_installed();
        shortcuts::install(&mut builder.app);
        let system_id = builder.app.world_mut().register_system(system);
        shortcuts::register_press(&mut builder.app, key, system_id);
        builder
    }

    /// Wire a continuous keyboard shortcut: while `key` is held, `system` runs
    /// every frame. Use this for held motion such as a light brighten/dim or a
    /// log scroll; the `system` reads `Res<Time>` itself and scales by
    /// `delta_secs`.
    ///
    /// Modifier guarding and the reserved-key check match
    /// [`with_shortcut`](Self::with_shortcut); only the firing cadence differs
    /// (every held frame instead of once per press).
    #[must_use]
    pub fn with_held_shortcut<Sys, M>(self, key: KeyCode, system: Sys) -> SprinkleBuilder<S>
    where
        Sys: IntoSystem<(), (), M> + 'static,
    {
        let mut builder = self.into_installed();
        shortcuts::install(&mut builder.app);
        let system_id = builder.app.world_mut().register_system(system);
        shortcuts::register_held(&mut builder.app, key, system_id);
        builder
    }

    /// Adds a marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin<M: Component>(self) -> SprinkleBuilder<S> {
        self.with_cube_spin_config::<M>(CubeSpinConfig::default())
    }

    /// Adds a marker-scoped cube spin helper with a customized configuration.
    #[must_use]
    pub fn with_cube_spin_config<M: Component>(self, config: CubeSpinConfig) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        cube_spin::install::<M>(&mut builder.app, config);
        builder
    }

    /// Add a reusable key/fill/rim lighting setup for simple example scenes.
    ///
    /// Returns a [`StudioLightingBuilder`] so the key light position and aim
    /// target can be tweaked before the rig spawns. Methods on the returned
    /// builder are only reachable through this call, so lighting tweaks are
    /// a compile error without `with_studio_lighting`.
    #[must_use]
    pub fn with_studio_lighting(self) -> StudioLightingBuilder<S> {
        StudioLightingBuilder {
            parent: self.into_installed(),
            config: StudioLightingConfig::default(),
        }
    }

    /// Spawn a static side panel that describes the example.
    #[must_use]
    pub fn with_description_panel(self, panel: DescriptionPanel) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        screen_panels::install_description(&mut builder.app, panel);
        builder
    }

    /// Spawn a compact top-left title bar for example controls and switch to
    /// a [`TitleBarBuilder`] so chip highlights can be wired to event
    /// lifecycles.
    #[must_use]
    pub fn with_title_bar(self, title_bar: TitleBar) -> TitleBarBuilder<S> {
        let mut builder = self.into_installed();
        screen_panels::install_title_bar(&mut builder.app, title_bar);
        TitleBarBuilder { parent: builder }
    }

    /// Mirror of [`App::add_plugins`].
    #[must_use]
    pub fn add_plugins<M>(self, plugins: impl Plugins<M>) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.add_plugins(plugins);
        builder
    }

    /// Mirror of [`App::add_systems`].
    #[must_use]
    pub fn add_systems<M>(
        self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.add_systems(schedule, systems);
        builder
    }

    /// Mirror of [`App::add_observer`].
    #[must_use]
    pub fn add_observer<E, BundleType, M, I>(self, observer: I) -> SprinkleBuilder<S>
    where
        E: bevy::ecs::event::Event,
        BundleType: Bundle,
        I: bevy::ecs::system::IntoObserverSystem<E, BundleType, M>,
    {
        let mut builder = self.into_installed();
        builder.app.add_observer(observer);
        builder
    }

    /// Mirror of [`App::init_resource`].
    #[must_use]
    pub fn init_resource<R: Resource + FromWorld>(self) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.init_resource::<R>();
        builder
    }

    /// Mirror of [`App::insert_resource`].
    #[must_use]
    pub fn insert_resource<R: Resource>(self, resource: R) -> SprinkleBuilder<S> {
        let mut builder = self.into_installed();
        builder.app.insert_resource(resource);
        builder
    }

    /// Run the configured app. Mirror of [`App::run`], with the exception
    /// that a `Ctrl+Shift+R` press handled via `with_restart_key`
    /// will re-exec the current binary before this method returns.
    pub fn run(self) -> AppExit {
        let mut builder = self.into_installed();
        keymap::install(&mut builder.app);
        let exit = builder.app.run();
        restart::perform_restart_if_requested();
        exit
    }
}

impl<S> SprinkleBuilder<S, AssetRootPending> {
    pub(crate) const fn new(app: App) -> Self {
        Self {
            app,
            baseline_status: BaselineStatus::Pending,
            orbit: PhantomData,
            baseline: PhantomData,
        }
    }

    /// Install the Fairy Dust baseline with a package-owned asset directory.
    ///
    /// This method is only available immediately after [`crate::sprinkle_example`]
    /// and consumes the pre-installation builder state:
    ///
    /// ```
    /// let builder = fairy_dust::sprinkle_example()
    ///     .with_asset_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
    /// # drop(builder);
    /// ```
    ///
    /// Any ordinary builder operation installs Bevy with its default asset
    /// root, so configuring an asset root later is rejected:
    ///
    /// ```compile_fail
    /// let builder = fairy_dust::sprinkle_example()
    ///     .with_brp_extras()
    ///     .with_asset_root("assets");
    /// # drop(builder);
    /// ```
    #[must_use]
    pub fn with_asset_root(mut self, asset_root: impl Into<String>) -> SprinkleBuilder<S> {
        crate::install_baseline(
            &mut self.app,
            AssetPlugin {
                file_path: asset_root.into(),
                ..AssetPlugin::default()
            },
        );
        self.baseline_status = BaselineStatus::Installed;
        self.into_installed()
    }

    /// Install the Fairy Dust baseline with Bevy's default asset root.
    ///
    /// This explicit transition makes [`SprinkleBuilder::app_mut`] available
    /// before selecting another capability.
    #[must_use]
    pub fn with_default_asset_root(self) -> SprinkleBuilder<S> { self.into_installed() }

    /// Installs the baseline, then starts configuring a reusable ground plane.
    #[must_use]
    pub fn with_ground_plane(self) -> PrimitiveBuilder<S> {
        self.into_installed().with_ground_plane()
    }

    /// Installs the baseline, then starts configuring a reusable cube.
    #[must_use]
    pub fn with_cube(self) -> PrimitiveBuilder<S> { self.into_installed().with_cube() }

    /// Installs the baseline, then begins configuring a camera home pose.
    #[must_use]
    pub fn with_camera_home(self) -> CameraHomeBuilder<S> {
        self.into_installed().with_camera_home()
    }
}

impl<S> SprinkleBuilder<S> {
    /// Starts configuring a reusable ground plane for the example scene.
    #[must_use]
    pub const fn with_ground_plane(self) -> PrimitiveBuilder<S> {
        PrimitiveBuilder {
            parent:  self,
            config:  PrimitiveConfig::ground_plane(),
            inserts: Vec::new(),
        }
    }

    /// Starts configuring a reusable cube for the example scene.
    #[must_use]
    pub const fn with_cube(self) -> PrimitiveBuilder<S> {
        PrimitiveBuilder {
            parent:  self,
            config:  PrimitiveConfig::cube(),
            inserts: Vec::new(),
        }
    }

    /// Begin configuring a generalized camera "home" pose.
    ///
    /// Snaps the camera to the union of every [`crate::CameraHomeTarget`] on
    /// startup, refits it on window resize while still home, and fills empty
    /// Lagrange presets with Fairy Dust's home input (`H` for keyboard-family
    /// presets, Select for gamepads). If a title bar is installed, the `H Home`
    /// chip is prepended automatically unless disabled with
    /// [`CameraHomeBuilder::without_title_bar_control`].
    #[must_use]
    pub const fn with_camera_home(self) -> CameraHomeBuilder<S> {
        CameraHomeBuilder {
            parent: self,
            config: CameraHomeConfig {
                yaw:               0.0,
                pitch:             0.0,
                margin:            crate::constants::HOME_DEFAULT_MARGIN,
                anchor:            Anchor::Center,
                offset_px:         Vec2::ZERO,
                title_bar_control: HomeTitleBarControl::Shown,
            },
        }
    }

    /// Escape hatch: borrow the underlying [`App`] for capabilities not yet
    /// surfaced as `with_*` methods.
    pub const fn app_mut(&mut self) -> &mut App { &mut self.app }
}

// State transition: `NoOrbitCam` → `WithOrbitCam`.
impl<Baseline> SprinkleBuilder<NoOrbitCam, Baseline> {
    /// Add `hana_lagrange::LagrangePlugin` and spawn an `OrbitCam` entity.
    /// The caller's `configure` closure can set `focus`, `radius`, `yaw`,
    /// `pitch`, sensitivity, limits, or other camera behavior fields. Input
    /// uses `OrbitCamPreset::simple_mouse()` unless another input mode is inserted.
    pub fn with_orbit_cam_configured<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        let mut builder = self.into_installed();
        orbit_cam::install_with(&mut builder.app, configure);
        SprinkleBuilder {
            app:             builder.app,
            baseline_status: BaselineStatus::Installed,
            orbit:           PhantomData,
            baseline:        PhantomData,
        }
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity, and
    /// insert extra camera-side components such as `OrbitCamInputMode` or
    /// [`CameraGuidance`](crate::CameraGuidance).
    pub fn with_orbit_cam<F, BundleType>(
        self,
        configure: F,
        bundle: BundleType,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        BundleType: Bundle + Send + Sync + 'static,
    {
        let mut builder = self.into_installed();
        orbit_cam::install_with_bundle(&mut builder.app, configure, bundle);
        SprinkleBuilder {
            app:             builder.app,
            baseline_status: BaselineStatus::Installed,
            orbit:           PhantomData,
            baseline:        PhantomData,
        }
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity, and
    /// install one built-in input preset.
    pub fn with_orbit_cam_preset<F>(
        self,
        configure: F,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.with_orbit_cam(configure, OrbitCamInputMode::with_preset(preset))
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity with an
    /// explicit startup pose, and install one built-in input preset.
    pub fn with_orbit_cam_preset_pose(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam> {
        let mut builder = self.into_installed();
        orbit_cam::install_pose_with_bundle(
            &mut builder.app,
            pose,
            OrbitCamInputMode::with_preset(preset),
        );
        SprinkleBuilder {
            app:             builder.app,
            baseline_status: BaselineStatus::Installed,
            orbit:           PhantomData,
            baseline:        PhantomData,
        }
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity,
    /// install one built-in input preset, and insert extra camera-side
    /// components.
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
        self.with_orbit_cam(configure, (OrbitCamInputMode::with_preset(preset), bundle))
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity with an
    /// explicit startup pose, install one built-in input preset, and insert extra
    /// camera-side components.
    pub fn with_orbit_cam_preset_pose_bundle<B>(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        B: Bundle + Send + Sync + 'static,
    {
        let mut builder = self.into_installed();
        orbit_cam::install_pose_with_bundle(
            &mut builder.app,
            pose,
            (OrbitCamInputMode::with_preset(preset), bundle),
        );
        SprinkleBuilder {
            app:             builder.app,
            baseline_status: BaselineStatus::Installed,
            orbit:           PhantomData,
            baseline:        PhantomData,
        }
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity, and
    /// install app-owned input bindings.
    pub fn with_orbit_cam_bindings<F>(
        self,
        configure: F,
        bindings: OrbitCamBindings,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.with_orbit_cam(configure, OrbitCamInputMode::Bindings(bindings))
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn an `OrbitCam` entity,
    /// install app-owned input bindings, and insert extra camera-side
    /// components.
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
        self.with_orbit_cam(configure, (OrbitCamInputMode::Bindings(bindings), bundle))
    }

    /// Add `hana_lagrange::LagrangePlugin` and spawn a manually driven
    /// `OrbitCam` entity.
    pub fn with_orbit_cam_manual<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.with_orbit_cam(configure, OrbitCamInputMode::Manual)
    }

    /// Add `hana_lagrange::LagrangePlugin`, spawn a manually driven `OrbitCam`,
    /// and insert extra camera-side components.
    pub fn with_orbit_cam_manual_bundle<F, B>(
        self,
        configure: F,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.with_orbit_cam(configure, (OrbitCamInputMode::Manual, bundle))
    }
}

// Camera-attached capabilities — only valid after an `OrbitCam` has been
// configured.
impl SprinkleBuilder<WithOrbitCam> {
    /// Capture the current `OrbitCam` pose on hot restart and make the restore
    /// animation available through [`crate::RestoreWindowAnimation`].
    #[must_use]
    pub fn with_restore_camera_on_restart(mut self) -> Self {
        restart_camera::install(&mut self.app);
        self
    }

    /// Insert `hana_diegetic::StableTransparency` on the spawned `OrbitCam`,
    /// which adds `OrderIndependentTransparencySettings`, sets the camera's
    /// depth texture to `TEXTURE_BINDING`, and forces `Msaa::Off` on the
    /// camera and on every screen-space overlay camera in the app.
    ///
    /// Use this when coplanar `WorldText` shows a view-angle color shift,
    /// when you need animated alpha fades on text, or when you need correct
    /// depth compositing of text with other translucent primitives. Pair
    /// with `AlphaMode::Blend` on text. Inert without `DiegeticUiPlugin`,
    /// which is added deduplicated.
    #[must_use]
    pub fn with_stable_transparency(mut self) -> Self {
        transparency::install(&mut self.app);
        self
    }

    /// Add a thresholded bloom to the orbit camera so over-bright (>1.0) colors
    /// glow while normal-range content stays crisp. `Bloom` requires HDR; pair
    /// with [`with_hdr`](Self::with_hdr) so every camera in a multi-camera
    /// render chain keeps the over-bright values.
    #[must_use]
    pub fn with_bloom(mut self) -> Self {
        bloom::install(&mut self.app);
        self
    }

    /// Insert an [`EnvironmentMapLight`](bevy::prelude::EnvironmentMapLight) on the orbit camera,
    /// lighting PBR surfaces (panel backgrounds and text glyphs both run `apply_pbr_lighting`)
    /// with diffuse ambient fill and specular reflection from a bundled
    /// `pisa` cathedral HDRI. The cubemaps are embedded, so no runtime
    /// `assets/` directory is required.
    ///
    /// A sharp specular glint only appears on a metallic, low-roughness
    /// surface; on the default rough-dielectric panel material the visible
    /// effect is the diffuse ambient term.
    #[must_use]
    pub fn with_environment_map(mut self) -> Self {
        environment_map::install(&mut self.app);
        self
    }

    /// Clear the example-default pitch and zoom limits on the spawned
    /// `OrbitCam`: the `orbit` angle limit resets to unbounded and the `zoom`
    /// limit drops its ceiling, keeping only a tiny positive floor.
    /// Use to inspect geometry from steep angles or at extreme zoom. Overrides
    /// limits set in the camera `configure` closure.
    #[must_use]
    pub fn unclamped(mut self) -> Self {
        unclamp::install(&mut self.app);
        self
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::asset::AssetServer;
    use hana_lagrange::OrbitCamBlenderLikePreset;

    use super::CommandPaletteKeymap;
    use super::NoOrbitCam;
    use super::SprinkleBuilder;
    use super::WithOrbitCam;
    use crate::builder::CameraHomeBuilder;
    use crate::builder::PrimitiveBuilder;
    use crate::builder::StudioLightingBuilder;
    use crate::builder::TitleBarBuilder;
    use crate::keymap;
    use crate::keymap::KeymapChoice;

    const CUSTOM_ASSET_ROOT: &str = "custom-assets";
    const OTHER_KEYMAP: &str = r#"{ "bindings": [] }"#;

    #[derive(bevy::prelude::Resource)]
    struct BaselineTransitionProbe;

    #[test]
    fn ordinary_first_operation_installs_default_baseline() {
        let builder = crate::sprinkle_example().insert_resource(BaselineTransitionProbe);

        assert_baseline(&builder, &AssetPlugin::default().file_path);
        assert!(
            builder
                .app
                .world()
                .contains_resource::<BaselineTransitionProbe>()
        );
    }

    #[test]
    fn explicit_default_asset_root_installs_default_baseline() {
        let builder = crate::sprinkle_example().with_default_asset_root();

        assert_baseline(&builder, &AssetPlugin::default().file_path);
    }

    #[test]
    fn custom_asset_root_installs_custom_baseline() {
        let builder = crate::sprinkle_example().with_asset_root(CUSTOM_ASSET_ROOT);

        assert_baseline(&builder, CUSTOM_ASSET_ROOT);
    }

    #[test]
    fn operation_after_baseline_transition_does_not_duplicate_asset_plugin() {
        let builder = crate::sprinkle_example()
            .with_default_asset_root()
            .insert_resource(BaselineTransitionProbe);

        assert_baseline(&builder, &AssetPlugin::default().file_path);
    }

    #[test]
    fn builder_wrappers_accept_typed_preset_payloads() {
        let _: fn(
            SprinkleBuilder<NoOrbitCam>,
            OrbitCamBlenderLikePreset,
        ) -> SprinkleBuilder<WithOrbitCam> = sprinkle_builder_with_preset;
        let _: fn(
            PrimitiveBuilder<NoOrbitCam>,
            OrbitCamBlenderLikePreset,
        ) -> SprinkleBuilder<WithOrbitCam> = primitive_builder_with_preset;
        let _: fn(
            StudioLightingBuilder<NoOrbitCam>,
            OrbitCamBlenderLikePreset,
        ) -> SprinkleBuilder<WithOrbitCam> = studio_lighting_builder_with_preset;
        let _: fn(
            CameraHomeBuilder<NoOrbitCam>,
            OrbitCamBlenderLikePreset,
        ) -> SprinkleBuilder<WithOrbitCam> = camera_home_builder_with_preset;
        let _: fn(
            TitleBarBuilder<NoOrbitCam>,
            OrbitCamBlenderLikePreset,
        ) -> SprinkleBuilder<WithOrbitCam> = title_bar_builder_with_preset;
    }

    fn assert_baseline(builder: &SprinkleBuilder<NoOrbitCam>, expected_asset_root: &str) {
        let asset_plugins = builder.app.get_added_plugins::<AssetPlugin>();
        assert_eq!(asset_plugins.len(), 1);
        assert_eq!(asset_plugins[0].file_path, expected_asset_root);
        assert!(builder.app.world().contains_resource::<AssetServer>());
    }

    fn sprinkle_builder_with_preset(
        builder: SprinkleBuilder<NoOrbitCam>,
        preset: OrbitCamBlenderLikePreset,
    ) -> SprinkleBuilder<WithOrbitCam> {
        builder.with_orbit_cam_preset(|_| {}, preset)
    }

    fn primitive_builder_with_preset(
        builder: PrimitiveBuilder<NoOrbitCam>,
        preset: OrbitCamBlenderLikePreset,
    ) -> SprinkleBuilder<WithOrbitCam> {
        builder.with_orbit_cam_preset(|_| {}, preset)
    }

    fn studio_lighting_builder_with_preset(
        builder: StudioLightingBuilder<NoOrbitCam>,
        preset: OrbitCamBlenderLikePreset,
    ) -> SprinkleBuilder<WithOrbitCam> {
        builder.with_orbit_cam_preset(|_| {}, preset)
    }

    fn camera_home_builder_with_preset(
        builder: CameraHomeBuilder<NoOrbitCam>,
        preset: OrbitCamBlenderLikePreset,
    ) -> SprinkleBuilder<WithOrbitCam> {
        builder.with_orbit_cam_preset(|_| {}, preset)
    }

    fn title_bar_builder_with_preset(
        builder: TitleBarBuilder<NoOrbitCam>,
        preset: OrbitCamBlenderLikePreset,
    ) -> SprinkleBuilder<WithOrbitCam> {
        builder.with_orbit_cam_preset(|_| {}, preset)
    }

    /// Opening the palette names Fairy Dust's shipped document, so naming the
    /// same one again is the application repeating itself rather than asking for
    /// two different sets of bindings.
    #[test]
    fn the_palette_and_the_shipped_keymap_name_the_same_document() {
        let builder = crate::sprinkle_example()
            .with_command_palette()
            .with_command_palette_keymap(CommandPaletteKeymap::default());

        assert_eq!(
            keymap::chosen(&builder.app),
            KeymapChoice::Named(CommandPaletteKeymap::default())
        );
    }

    #[test]
    #[should_panic(expected = "already configured with a different document")]
    fn a_keymap_named_after_the_palette_is_refused() {
        let _builder = crate::sprinkle_example()
            .with_command_palette()
            .with_command_palette_keymap(CommandPaletteKeymap::new(OTHER_KEYMAP));
    }

    #[test]
    #[should_panic(expected = "already configured with a different document")]
    fn a_palette_opened_after_a_keymap_is_refused() {
        let _builder = crate::sprinkle_example()
            .with_command_palette_keymap(CommandPaletteKeymap::new(OTHER_KEYMAP))
            .with_command_palette();
    }
}
