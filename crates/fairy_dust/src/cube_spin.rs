//! Shared cube spin helpers for example presentation.

use std::marker::PhantomData;

use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SpawnRelated;
use bevy::prelude::SpawnWith;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Virtual;
use bevy::prelude::With;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::Start;
use bevy_enhanced_input::prelude::bindings;

use crate::constants::CUBE_SPIN_PAUSE_CONTROL_ID;
use crate::constants::CUBE_SPIN_PAUSE_CONTROL_LABEL;
use crate::constants::CUBE_SPIN_RESERVE_LABEL;
use crate::constants::DEFAULT_CUBE_SPIN_RADIANS_PER_SECOND;
use crate::ensure_plugin;
use crate::screen_panels;
use crate::screen_panels::ControlActivation;
use crate::screen_panels::TitleBarControlState;
use crate::screen_panels::TitleChip;
use crate::screen_panels::TitleChipActivation;
use crate::shortcuts;

/// Marker inserted by [`crate::PrimitiveBuilder::cube_spin`] for builder-owned spin.
#[derive(Component)]
pub struct FairyDustCubeSpinTarget;

/// Input context scoping the cube-spin toggle to marker `M`, so each cube-spin
/// instance owns an independent keybinding.
#[derive(Component)]
struct CubeSpinContext<M> {
    marker: PhantomData<M>,
}

/// Toggle action for the cube-spin keybinding, scoped to marker `M`. Only ever
/// used as the type parameter of `Action`/`Start`; never constructed.
#[derive(InputAction)]
#[action_output(bool)]
struct ToggleCubeSpin<M: 'static> {
    marker: PhantomData<M>,
}

/// Whether a cube spin helper is currently rotating its targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CubeSpinMode {
    /// Rotate targets.
    #[default]
    Spinning,
    /// Keep targets still.
    Paused,
}

impl CubeSpinMode {
    /// Returns the opposite spin mode.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Spinning => Self::Paused,
            Self::Paused => Self::Spinning,
        }
    }
}

impl TitleChipActivation for CubeSpinMode {
    fn activation(&self) -> ControlActivation {
        match self {
            Self::Spinning => ControlActivation::Inactive,
            Self::Paused => ControlActivation::Active,
        }
    }
}

/// Rotation motion applied by a cube spin helper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CubeSpinMotion {
    /// Rotate around local Y by radians per second.
    Yaw(f32),
    /// Rotate around an arbitrary local axis by radians per second.
    AxisAngle {
        /// Local-space axis.
        axis:               Vec3,
        /// Angular speed.
        radians_per_second: f32,
    },
    /// Rotate around local X/Y/Z by radians per second.
    Euler {
        /// Per-axis angular speed.
        radians_per_second: Vec3,
    },
}

impl Default for CubeSpinMotion {
    fn default() -> Self { Self::Yaw(DEFAULT_CUBE_SPIN_RADIANS_PER_SECOND) }
}

/// Time source used by cube spin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CubeSpinTimeSource {
    /// Bevy virtual time. Respects pause.
    #[default]
    Virtual,
    /// Wall-clock time. Ignores virtual-time pause.
    Real,
}

/// Configuration for [`crate::SprinkleBuilder::with_cube_spin`].
#[derive(Clone, Debug)]
pub struct CubeSpinConfig {
    /// Optional title chip added by the helper.
    pub chip:          Option<TitleChip>,
    /// Optional keyboard shortcut that toggles spin.
    pub key:           Option<KeyCode>,
    /// Spin mode that should highlight the title chip.
    pub active_mode:   CubeSpinMode,
    /// Rotation motion.
    pub motion:        CubeSpinMotion,
    /// Time source.
    pub time_source:   CubeSpinTimeSource,
    /// Initial spin mode.
    pub initial_state: CubeSpinMode,
}

impl CubeSpinConfig {
    /// Creates the canonical `P Pause` cube spin helper.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            chip:          Some(TitleChip::new(
                CUBE_SPIN_PAUSE_CONTROL_ID,
                CUBE_SPIN_PAUSE_CONTROL_LABEL,
            )),
            key:           Some(KeyCode::KeyP),
            active_mode:   CubeSpinMode::Paused,
            motion:        CubeSpinMotion::Yaw(DEFAULT_CUBE_SPIN_RADIANS_PER_SECOND),
            time_source:   CubeSpinTimeSource::Virtual,
            initial_state: CubeSpinMode::Spinning,
        }
    }

    /// Uses a different title chip.
    #[must_use]
    pub const fn with_chip(mut self, chip: TitleChip) -> Self {
        self.chip = Some(chip);
        self
    }

    /// Disables title-chip registration and highlighting.
    #[must_use]
    pub const fn without_chip(mut self) -> Self {
        self.chip = None;
        self
    }

    /// Uses a different toggle key.
    #[must_use]
    pub const fn with_key(mut self, key: KeyCode) -> Self {
        self.key = Some(key);
        self
    }

    /// Highlights the title chip when the helper is in `active_mode`.
    #[must_use]
    pub const fn with_active_mode(mut self, active_mode: CubeSpinMode) -> Self {
        self.active_mode = active_mode;
        self
    }

    /// Disables keyboard toggling.
    #[must_use]
    pub const fn without_key(mut self) -> Self {
        self.key = None;
        self
    }

    /// Uses a different spin motion.
    #[must_use]
    pub const fn with_motion(mut self, motion: CubeSpinMotion) -> Self {
        self.motion = motion;
        self
    }

    /// Uses a different time source.
    #[must_use]
    pub const fn with_time_source(mut self, time_source: CubeSpinTimeSource) -> Self {
        self.time_source = time_source;
        self
    }

    /// Uses a different initial spin mode.
    #[must_use]
    pub const fn with_initial_state(mut self, initial_state: CubeSpinMode) -> Self {
        self.initial_state = initial_state;
        self
    }
}

impl Default for CubeSpinConfig {
    fn default() -> Self { Self::new() }
}

/// Marker-scoped cube spin state.
#[derive(Resource)]
pub struct CubeSpinControl<M> {
    mode:        CubeSpinMode,
    key:         Option<KeyCode>,
    chip_id:     Option<String>,
    active_mode: CubeSpinMode,
    motion:      CubeSpinMotion,
    time_source: CubeSpinTimeSource,
    marker:      PhantomData<M>,
}

impl<M> CubeSpinControl<M> {
    fn new(config: &CubeSpinConfig) -> Self {
        Self {
            mode:        config.initial_state,
            key:         config.key,
            chip_id:     config.chip.map(|chip| chip.id().to_string()),
            active_mode: config.active_mode,
            motion:      config.motion,
            time_source: config.time_source,
            marker:      PhantomData,
        }
    }

    /// Returns the current spin mode.
    #[must_use]
    pub const fn mode(&self) -> CubeSpinMode { self.mode }

    /// Flips between spinning and paused. Examples that drive the toggle from a
    /// non-keyboard input (a gamepad button, say) call this; the title chip
    /// follows automatically through `sync_cube_spin_chip`.
    pub const fn toggle(&mut self) { self.mode = self.mode.toggled(); }
}

impl<M: 'static> TitleChipActivation for CubeSpinControl<M> {
    fn activation(&self) -> ControlActivation {
        if self.mode == self.active_mode {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

pub(crate) fn install<M: Component>(app: &mut App, config: CubeSpinConfig) {
    if let Some(key) = config.key {
        shortcuts::reserve_key::<CubeSpinContext<M>>(app, key, CUBE_SPIN_RESERVE_LABEL);
    }
    if let Some(chip) = config.chip {
        screen_panels::register_title_control(app, chip);
    }
    ensure_plugin(app, EnhancedInputPlugin);
    app.insert_resource(CubeSpinControl::<M>::new(&config));
    app.add_input_context::<CubeSpinContext<M>>();
    app.add_systems(Startup, spawn_cube_spin_action::<M>);
    app.add_systems(Update, (sync_cube_spin_chip::<M>, spin_cube_targets::<M>));
    app.add_observer(toggle_cube_spin_on_start::<M>);
}

fn spawn_cube_spin_action<M: Component>(mut commands: Commands, control: Res<CubeSpinControl<M>>) {
    let Some(key) = control.key else {
        return;
    };
    commands.spawn((
        CubeSpinContext::<M> {
            marker: PhantomData,
        },
        Actions::<CubeSpinContext<M>>::spawn(SpawnWith(
            move |spawner: &mut ActionSpawner<CubeSpinContext<M>>| {
                spawner.spawn((Action::<ToggleCubeSpin<M>>::new(), bindings![key]));
            },
        )),
    ));
}

fn toggle_cube_spin_on_start<M: Component>(
    _start: On<Start<ToggleCubeSpin<M>>>,
    mut control: ResMut<CubeSpinControl<M>>,
) {
    control.mode = control.mode.toggled();
}

fn sync_cube_spin_chip<M: Component>(
    control: Res<CubeSpinControl<M>>,
    mut bars: Query<&mut TitleBarControlState>,
) {
    if !control.is_changed() {
        return;
    }
    let Some(chip_id) = control.chip_id.as_deref() else {
        return;
    };
    for mut bar in &mut bars {
        bar.set_active(chip_id, control.activation());
    }
}

fn spin_cube_targets<M: Component>(
    time_real: Res<Time<Real>>,
    time_virtual: Res<Time<Virtual>>,
    control: Res<CubeSpinControl<M>>,
    mut targets: Query<&mut Transform, With<M>>,
) {
    if control.mode != CubeSpinMode::Spinning {
        return;
    }
    let delta_secs = match control.time_source {
        CubeSpinTimeSource::Virtual => time_virtual.delta_secs(),
        CubeSpinTimeSource::Real => time_real.delta_secs(),
    };
    for mut transform in &mut targets {
        match control.motion {
            CubeSpinMotion::Yaw(speed) => transform.rotate_y(speed * delta_secs),
            CubeSpinMotion::AxisAngle {
                axis,
                radians_per_second,
            } => transform.rotate(Quat::from_axis_angle(
                axis.normalize_or_zero(),
                radians_per_second * delta_secs,
            )),
            CubeSpinMotion::Euler { radians_per_second } => {
                transform.rotate_x(radians_per_second.x * delta_secs);
                transform.rotate_y(radians_per_second.y * delta_secs);
                transform.rotate_z(radians_per_second.z * delta_secs);
            },
        }
    }
}
