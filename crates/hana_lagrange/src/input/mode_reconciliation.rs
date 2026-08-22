use std::marker::PhantomData;

use bevy::prelude::App;
use bevy::prelude::Changed;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Or;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::prelude::warn;
use bevy_enhanced_input::prelude::Actions;

use super::BindingsError;
use super::CameraInputModeKind;
use super::CameraManual;
use super::InputMode;
use crate::FreeCamKind;
use crate::OrbitCamKind;
use crate::system_sets::CameraInputInternalSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeInputMode {
    Bindings,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallationStatus {
    Present,
    Missing,
}

impl InstallationStatus {
    const fn from_installation<K: CameraInputModeKind>(
        installation: Option<&CameraInputInstallation<K>>,
    ) -> Self {
        match installation {
            Some(_) => Self::Present,
            None => Self::Missing,
        }
    }

    const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedRuntimeInputMode<K: CameraInputModeKind> {
    bindings: Option<K::Bindings>,
}

impl<K: CameraInputModeKind> Default for PreparedRuntimeInputMode<K> {
    fn default() -> Self { Self { bindings: None } }
}

impl<K: CameraInputModeKind> PreparedRuntimeInputMode<K> {
    const fn bindings(bindings: K::Bindings) -> Self {
        Self {
            bindings: Some(bindings),
        }
    }

    const fn mode(&self) -> RuntimeInputMode {
        if self.bindings.is_some() {
            RuntimeInputMode::Bindings
        } else {
            RuntimeInputMode::Manual
        }
    }
}

#[derive(Clone, Copy, Debug, EntityEvent)]
pub(crate) struct CameraInputModeReplaced {
    #[event_target]
    pub(super) camera: Entity,
}

#[derive(Clone, Debug, PartialEq)]
struct ModeReconciliation<K: CameraInputModeKind> {
    camera:       Entity,
    mode:         InputMode<K>,
    last_valid:   Option<InputMode<K>>,
    installation: InstallationStatus,
}

trait RuntimeInputModeKind: CameraInputModeKind + Sized {
    const NAME: &'static str;

    fn prepare_runtime_mode(
        mode: &InputMode<Self>,
    ) -> Result<PreparedRuntimeInputMode<Self>, BindingsError>;

    fn emit_replaced(world: &mut World, camera: Entity);
}

impl RuntimeInputModeKind for OrbitCamKind {
    const NAME: &'static str = "OrbitCam";

    fn prepare_runtime_mode(
        mode: &InputMode<Self>,
    ) -> Result<PreparedRuntimeInputMode<Self>, BindingsError> {
        match mode {
            InputMode::Preset(preset) => {
                Ok(PreparedRuntimeInputMode::bindings(preset.to_bindings()?))
            },
            InputMode::Bindings(bindings) => {
                Ok(PreparedRuntimeInputMode::bindings(bindings.clone()))
            },
            InputMode::Manual => Ok(PreparedRuntimeInputMode::default()),
        }
    }

    fn emit_replaced(world: &mut World, camera: Entity) {
        world
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });
    }
}

impl RuntimeInputModeKind for FreeCamKind {
    const NAME: &'static str = "FreeCam";

    fn prepare_runtime_mode(
        mode: &InputMode<Self>,
    ) -> Result<PreparedRuntimeInputMode<Self>, BindingsError> {
        match mode {
            InputMode::Preset(preset) => {
                Ok(PreparedRuntimeInputMode::bindings(preset.to_bindings()?))
            },
            InputMode::Bindings(bindings) => {
                Ok(PreparedRuntimeInputMode::bindings(bindings.clone()))
            },
            InputMode::Manual => Ok(PreparedRuntimeInputMode::default()),
        }
    }

    fn emit_replaced(world: &mut World, camera: Entity) {
        world
            .entity_mut(camera)
            .trigger(|camera| CameraInputModeReplaced { camera });
    }
}

#[derive(Component, Clone, Debug, PartialEq)]
pub(crate) struct CameraResolvedBindings<K: CameraInputModeKind>(pub(crate) K::Bindings);

#[derive(Component, Clone, Debug, PartialEq)]
pub(crate) struct CameraInstalledBindings<K: CameraInputModeKind>(pub(crate) K::Bindings);

pub(crate) type OrbitCamResolvedBindings = CameraResolvedBindings<OrbitCamKind>;
pub(crate) type FreeCamResolvedBindings = CameraResolvedBindings<FreeCamKind>;

#[derive(Component, Clone, Debug)]
struct LastValidInputMode<K: CameraInputModeKind>(InputMode<K>);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
struct CameraInputInstallation<K: CameraInputModeKind> {
    entities: Vec<Entity>,
    marker:   PhantomData<fn() -> K>,
}

impl<K: CameraInputModeKind> CameraInputInstallation<K> {
    fn new(entities: Vec<Entity>) -> Self {
        Self {
            entities,
            marker: PhantomData,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CameraInputInstallationOf<K: CameraInputModeKind> {
    camera: Entity,
    marker: PhantomData<fn() -> K>,
}

impl<K: CameraInputModeKind> CameraInputInstallationOf<K> {
    pub(super) const fn new(camera: Entity) -> Self {
        Self {
            camera,
            marker: PhantomData,
        }
    }

    pub(crate) const fn camera(self) -> Entity { self.camera }
}

pub(crate) type OrbitCamInputInstallationOf = CameraInputInstallationOf<OrbitCamKind>;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
struct CameraInputInstallationPlaceholder<K: CameraInputModeKind> {
    marker: PhantomData<fn() -> K>,
}

impl<K: CameraInputModeKind> Default for CameraInputInstallationPlaceholder<K> {
    fn default() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

pub(crate) struct CameraInputModesPlugin;

impl Plugin for CameraInputModesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                reconcile_input_modes::<OrbitCamKind>,
                reconcile_input_modes::<FreeCamKind>,
            )
                .in_set(CameraInputInternalSet::InputModes),
        );
    }
}

fn reconcile_input_modes<K: RuntimeInputModeKind>(world: &mut World)
where
    InputMode<K>: Clone + PartialEq,
{
    let mut query = world.query_filtered::<(
        Entity,
        &InputMode<K>,
        Option<&LastValidInputMode<K>>,
        Option<&CameraInputInstallation<K>>,
    ), (
        With<K::Camera>,
        Or<(Changed<InputMode<K>>, Without<CameraInputInstallation<K>>)>,
    )>();
    let reconciliations = query
        .iter(world)
        .map(
            |(camera, mode, last_valid, installation)| ModeReconciliation {
                camera,
                mode: mode.clone(),
                last_valid: last_valid.map(|mode| mode.0.clone()),
                installation: InstallationStatus::from_installation(installation),
            },
        )
        .collect::<Vec<_>>();

    for reconciliation in reconciliations {
        apply_mode_reconciliation::<K>(world, reconciliation);
    }
}

fn apply_mode_reconciliation<K: RuntimeInputModeKind>(
    world: &mut World,
    reconciliation: ModeReconciliation<K>,
) where
    InputMode<K>: Clone + PartialEq,
{
    if reconciliation.installation.is_present()
        && reconciliation
            .last_valid
            .as_ref()
            .is_some_and(|last_valid| *last_valid == reconciliation.mode)
    {
        return;
    }
    let prepared_mode = match K::prepare_runtime_mode(&reconciliation.mode) {
        Ok(prepared_mode) => prepared_mode,
        Err(error) => {
            restore_last_valid_input_mode::<K>(world, &reconciliation, error);
            return;
        },
    };

    clear_camera_input::<K>(world, reconciliation.camera);
    let runtime_mode = prepared_mode.mode();
    apply_prepared_runtime_input_mode::<K>(world, reconciliation.camera, prepared_mode);
    replace_input_installation::<K>(world, reconciliation.camera, runtime_mode);
    world
        .entity_mut(reconciliation.camera)
        .insert(LastValidInputMode::<K>(reconciliation.mode));
    K::emit_replaced(world, reconciliation.camera);
}

fn apply_prepared_runtime_input_mode<K: RuntimeInputModeKind>(
    world: &mut World,
    camera: Entity,
    prepared_mode: PreparedRuntimeInputMode<K>,
) {
    match prepared_mode.bindings {
        Some(bindings) => {
            world
                .entity_mut(camera)
                .insert(CameraResolvedBindings::<K>(bindings))
                .remove::<CameraManual<K>>();
        },
        None => {
            world
                .entity_mut(camera)
                .remove::<CameraResolvedBindings<K>>()
                .insert(CameraManual::<K>::default());
        },
    }
}

fn restore_last_valid_input_mode<K: RuntimeInputModeKind>(
    world: &mut World,
    reconciliation: &ModeReconciliation<K>,
    error: BindingsError,
) where
    InputMode<K>: Clone,
{
    warn!(
        "failed to build {} input mode for {:?}: {error}",
        K::NAME,
        reconciliation.camera
    );
    if let Some(last_valid) = &reconciliation.last_valid {
        world
            .entity_mut(reconciliation.camera)
            .insert(last_valid.clone());
    }
}

fn clear_camera_input<K: RuntimeInputModeKind>(world: &mut World, camera: Entity) {
    if let Some(mut input) = world.get_mut::<super::InputIntent<K>>(camera) {
        input.clear();
    }
}

fn replace_input_installation<K: RuntimeInputModeKind>(
    world: &mut World,
    camera: Entity,
    runtime_input_mode: RuntimeInputMode,
) {
    // Drop the previous installation through the `Actions` relationship rather
    // than iterating `installed_input_entities`. That flat list holds both the
    // action entities and their `BindingOf` children; despawning an action
    // already despawns its bindings through `linked_spawn`, so iterating the list
    // despawned each binding a second time after its action was gone, which
    // `World::despawn` reports as despawning an absent entity. `despawn_related`
    // walks each action subtree exactly once and skips entities already despawned.
    world
        .entity_mut(camera)
        .despawn_related::<Actions<K::Context>>();

    let entities = match runtime_input_mode {
        RuntimeInputMode::Bindings => {
            vec![
                world
                    .spawn((
                        CameraInputInstallationOf::<K>::new(camera),
                        CameraInputInstallationPlaceholder::<K>::default(),
                    ))
                    .id(),
            ]
        },
        RuntimeInputMode::Manual => Vec::new(),
    };

    world
        .entity_mut(camera)
        .insert(CameraInputInstallation::<K>::new(entities));
}

pub(crate) fn installed_input_entities_for<K: CameraInputModeKind>(
    world: &World,
    camera: Entity,
) -> Vec<Entity> {
    let Some(installation) = world.get::<CameraInputInstallation<K>>(camera) else {
        return Vec::new();
    };

    installation
        .entities
        .iter()
        .copied()
        .filter(|entity| {
            world
                .get::<CameraInputInstallationOf<K>>(*entity)
                .is_some_and(|owner| owner.camera() == camera)
        })
        .collect()
}

pub(crate) fn input_installation_has_placeholder_for<K: CameraInputModeKind>(
    world: &World,
    camera: Entity,
) -> bool {
    installed_input_entities_for::<K>(world, camera)
        .iter()
        .any(|entity| {
            world
                .get::<CameraInputInstallationPlaceholder<K>>(*entity)
                .is_some()
        })
}

pub(crate) fn replace_installed_input_entities_for<K: CameraInputModeKind>(
    world: &mut World,
    camera: Entity,
    entities: Vec<Entity>,
) {
    world
        .entity_mut(camera)
        .insert(CameraInputInstallation::<K>::new(entities));
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;

    use super::*;
    use crate::FreeCam;
    use crate::FreeCamInput;
    use crate::FreeCamInputMode;
    use crate::OrbitCam;
    use crate::OrbitCamInput;
    use crate::OrbitCamInputMode;
    use crate::TranslateDelta;
    use crate::input::FreeCamKeyboardMousePreset;
    use crate::input::FreeCamManualInputWriter;
    use crate::input::FreeCamPreset;
    use crate::input::INVALID_SOURCE_INPUT_GAIN;
    use crate::input::InteractionSources;
    use crate::input::ManualInputSource;
    use crate::input::OrbitCamBlenderLikePreset;
    use crate::input::OrbitCamInputGain;
    use crate::input::OrbitCamManualInputWriter;
    use crate::input::OrbitCamPreset;
    use crate::system_sets::LagrangeSystemSetsPlugin;

    #[derive(Resource)]
    struct ManualWriterTestCamera {
        manual: Entity,
        preset: Entity,
    }

    #[derive(Resource, Default)]
    struct ManualWriterTestResult {
        manual_written:  bool,
        preset_rejected: bool,
    }

    #[derive(Resource)]
    struct FreeManualWriterTestCamera {
        manual: Entity,
        preset: Entity,
    }

    #[derive(Resource, Default)]
    struct FreeManualWriterTestResult {
        manual_written:  bool,
        preset_rejected: bool,
    }

    #[derive(Resource, Default)]
    struct ModeReplacementEvents(usize);

    type TestResult = Result<(), &'static str>;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            LagrangeSystemSetsPlugin,
            CameraInputModesPlugin,
        ));
        app
    }

    fn invalid_blender_like_preset() -> OrbitCamPreset {
        OrbitCamBlenderLikePreset::default()
            .mouse_input_gain(OrbitCamInputGain::uniform(INVALID_SOURCE_INPUT_GAIN))
            .into()
    }

    fn invalid_free_cam_preset() -> FreeCamPreset {
        FreeCamKeyboardMousePreset::default()
            .slow_scale(f32::NAN)
            .into()
    }

    fn mark_current_input(app: &mut App, camera: Entity) -> TestResult {
        app.world_mut()
            .get_mut::<OrbitCamInput>(camera)
            .ok_or("camera should have OrbitCamInput")?
            .add_orbit_with_sources(Vec2::new(1.0, 0.0), InteractionSources::MOUSE);
        Ok(())
    }

    #[test]
    fn preinput_lowers_required_default_preset() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((OrbitCam::default(), OrbitCamInput::default()))
            .id();

        app.update();

        assert_eq!(
            app.world().get::<OrbitCamInputMode>(camera),
            Some(&OrbitCamInputMode::with_preset(
                OrbitCamPreset::simple_mouse()
            ))
        );
        assert!(
            app.world()
                .get::<OrbitCamResolvedBindings>(camera)
                .is_some()
        );
        assert_eq!(
            installed_input_entities_for::<OrbitCamKind>(app.world(), camera).len(),
            1
        );
    }

    #[test]
    fn preinput_lowers_manual_mode() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                OrbitCamInputMode::Manual,
            ))
            .id();

        app.update();

        assert!(
            app.world()
                .get::<CameraManual<OrbitCamKind>>(camera)
                .is_some()
        );
        assert!(
            app.world()
                .get::<OrbitCamResolvedBindings>(camera)
                .is_none()
        );
        assert!(installed_input_entities_for::<OrbitCamKind>(app.world(), camera).is_empty());
    }

    #[test]
    fn preinput_lowers_free_cam_default_preset_mode() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((FreeCam::default(), FreeCamInput::default()))
            .id();

        app.update();

        assert_eq!(
            app.world().get::<FreeCamInputMode>(camera),
            Some(&FreeCamInputMode::with_preset(FreeCamPreset::default()))
        );
        assert!(app.world().get::<FreeCamResolvedBindings>(camera).is_some());
        assert!(
            app.world()
                .get::<CameraManual<FreeCamKind>>(camera)
                .is_none()
        );
    }

    #[test]
    fn free_cam_preset_lowers_to_resolved_bindings() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInput::default(),
                FreeCamInputMode::Manual,
            ))
            .id();

        app.update();
        app.world_mut()
            .entity_mut(camera)
            .insert(FreeCamInputMode::with_preset(FreeCamPreset::default()));
        app.update();

        assert_eq!(
            app.world().get::<FreeCamInputMode>(camera),
            Some(&FreeCamInputMode::with_preset(FreeCamPreset::default()))
        );
        assert!(app.world().get::<FreeCamResolvedBindings>(camera).is_some());
        assert!(
            app.world()
                .get::<CameraManual<FreeCamKind>>(camera)
                .is_none()
        );
    }

    #[test]
    fn invalid_direct_preset_replacement_keeps_previous_runtime_state() -> TestResult {
        let mut app = test_app();
        let valid_mode = OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like());
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                valid_mode.clone(),
            ))
            .id();

        app.update();
        mark_current_input(&mut app, camera)?;
        let previous_bindings = app
            .world()
            .get::<OrbitCamResolvedBindings>(camera)
            .cloned()
            .ok_or("camera should have resolved bindings")?;
        let previous_installation =
            installed_input_entities_for::<OrbitCamKind>(app.world(), camera);
        app.init_resource::<ModeReplacementEvents>();
        app.world_mut().entity_mut(camera).observe(
            |_replaced: On<CameraInputModeReplaced>, mut events: ResMut<ModeReplacementEvents>| {
                events.0 += 1;
            },
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(OrbitCamInputMode::with_preset(invalid_blender_like_preset()));
        app.update();

        assert_eq!(
            app.world().get::<OrbitCamInputMode>(camera),
            Some(&valid_mode)
        );
        assert_eq!(
            app.world().get::<OrbitCamResolvedBindings>(camera),
            Some(&previous_bindings)
        );
        assert_eq!(
            installed_input_entities_for::<OrbitCamKind>(app.world(), camera),
            previous_installation
        );
        assert!(
            app.world()
                .get::<OrbitCamInput>(camera)
                .is_some_and(OrbitCamInput::has_orbit)
        );
        assert_eq!(app.world().resource::<ModeReplacementEvents>().0, 0);

        Ok(())
    }

    #[test]
    fn invalid_free_cam_preset_replacement_keeps_previous_runtime_state() -> TestResult {
        let mut app = test_app();
        let valid_mode = FreeCamInputMode::with_preset(FreeCamPreset::default());
        let camera = app
            .world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInput::default(),
                valid_mode.clone(),
            ))
            .id();

        app.update();
        let previous_bindings = app
            .world()
            .get::<FreeCamResolvedBindings>(camera)
            .cloned()
            .ok_or("camera should have resolved bindings")?;
        let previous_installation =
            installed_input_entities_for::<FreeCamKind>(app.world(), camera);
        app.init_resource::<ModeReplacementEvents>();
        app.world_mut().entity_mut(camera).observe(
            |_replaced: On<CameraInputModeReplaced>, mut events: ResMut<ModeReplacementEvents>| {
                events.0 += 1;
            },
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(FreeCamInputMode::with_preset(invalid_free_cam_preset()));
        app.update();

        assert_eq!(
            app.world().get::<FreeCamInputMode>(camera),
            Some(&valid_mode)
        );
        assert_eq!(
            app.world().get::<FreeCamResolvedBindings>(camera),
            Some(&previous_bindings)
        );
        assert_eq!(
            installed_input_entities_for::<FreeCamKind>(app.world(), camera),
            previous_installation
        );
        assert_eq!(app.world().resource::<ModeReplacementEvents>().0, 0);

        Ok(())
    }

    fn write_manual_test_input(
        mut writer: OrbitCamManualInputWriter,
        cameras: Res<ManualWriterTestCamera>,
        mut result: ResMut<ManualWriterTestResult>,
    ) {
        result.manual_written = writer
            .get_mut(cameras.manual, ManualInputSource::manual())
            .is_ok();
        result.preset_rejected = writer
            .get_mut(cameras.preset, ManualInputSource::manual())
            .is_err();
    }

    #[test]
    fn manual_writer_only_yields_manual_cameras() {
        let mut app = test_app();
        let manual = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                OrbitCamInputMode::Manual,
            ))
            .id();
        let preset = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id();
        app.insert_resource(ManualWriterTestCamera { manual, preset })
            .init_resource::<ManualWriterTestResult>()
            .add_systems(Update, write_manual_test_input);

        app.update();

        let result = app.world().resource::<ManualWriterTestResult>();
        assert!(result.manual_written);
        assert!(result.preset_rejected);
    }

    fn write_free_manual_test_input(
        mut writer: FreeCamManualInputWriter,
        cameras: Res<FreeManualWriterTestCamera>,
        mut result: ResMut<FreeManualWriterTestResult>,
    ) {
        result.manual_written = writer
            .get_mut(cameras.manual, ManualInputSource::manual())
            .map(|mut input| {
                input.translate(Vec3::Z);
            })
            .is_ok();
        result.preset_rejected = writer
            .get_mut(cameras.preset, ManualInputSource::manual())
            .is_err();
    }

    #[test]
    fn free_manual_writer_only_yields_manual_cameras() {
        let mut app = test_app();
        let manual = app
            .world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInput::default(),
                FreeCamInputMode::Manual,
            ))
            .id();
        let preset = app
            .world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInput::default(),
                FreeCamInputMode::with_preset(FreeCamPreset::default()),
            ))
            .id();
        app.insert_resource(FreeManualWriterTestCamera { manual, preset })
            .init_resource::<FreeManualWriterTestResult>()
            .add_systems(Update, write_free_manual_test_input);

        app.update();

        let result = app.world().resource::<FreeManualWriterTestResult>();
        assert!(result.manual_written);
        assert!(result.preset_rejected);
        assert_eq!(
            app.world()
                .get::<FreeCamInput>(manual)
                .ok_or("manual camera should have FreeCamInput")
                .map(FreeCamInput::translate),
            Ok(TranslateDelta::from(Vec3::Z))
        );
    }

    #[test]
    fn mode_replacement_triggers_internal_hook() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInput::default(),
                OrbitCamInputMode::Manual,
            ))
            .id();
        app.init_resource::<ModeReplacementEvents>();
        app.world_mut().entity_mut(camera).observe(
            |_replaced: On<CameraInputModeReplaced>, mut events: ResMut<ModeReplacementEvents>| {
                events.0 += 1;
            },
        );

        app.update();

        assert_eq!(app.world().resource::<ModeReplacementEvents>().0, 1);
    }

    fn type_is_registered<T: 'static>(app: &App) -> bool {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        registry.get(TypeId::of::<T>()).is_some()
    }

    #[test]
    fn reflect_input_mode_types_are_registered() {
        use crate::*;

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin));

        assert!(type_is_registered::<ActionBindingDescriptor>(&app));
        assert!(type_is_registered::<BindingEngagement>(&app));
        assert!(type_is_registered::<BindingGates>(&app));
        assert!(type_is_registered::<BindingRoutePolicy>(&app));
        assert!(type_is_registered::<CameraInputGamepadSelectionPolicy>(
            &app
        ));
        assert!(type_is_registered::<InteractionSources>(&app));
        assert!(type_is_registered::<ControlSpeed>(&app));
        assert!(type_is_registered::<InputAxisTransform>(&app));
        assert!(type_is_registered::<InputBindingModifiers>(&app));
        assert!(type_is_registered::<InputBindingScale>(&app));
        assert!(type_is_registered::<InputDeadZone>(&app));
        assert!(type_is_registered::<InputDeltaScale>(&app));
        assert!(type_is_registered::<InputGain>(&app));
        assert!(type_is_registered::<FreeCamInputGain>(&app));
        assert!(type_is_registered::<BindingGate>(&app));
        assert!(type_is_registered::<
            OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>,
        >(&app));
        assert!(type_is_registered::<
            OrbitCamBindingWithInputGain<OrbitCamMouseWheelZoom>,
        >(&app));
        assert!(type_is_registered::<
            OrbitCamBindingWithInputGain<OrbitCamPinchZoom>,
        >(&app));
        assert!(type_is_registered::<
            OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
        >(&app));
        assert!(type_is_registered::<OrbitCamBindings>(&app));
        assert!(type_is_registered::<OrbitCamBlenderLikeKeyboardPreset>(
            &app
        ));
        assert!(type_is_registered::<OrbitCamBlenderLikePreset>(&app));
        assert!(type_is_registered::<OrbitCamButtonDragZoom>(&app));
        assert!(type_is_registered::<OrbitCamButtonDragZoomAxis>(&app));
        assert!(type_is_registered::<OrbitCamGamepadPreset>(&app));
        assert!(type_is_registered::<GateInput>(&app));
        assert!(type_is_registered::<GatePolarity>(&app));
        assert!(type_is_registered::<HeldBinding>(&app));
        assert!(type_is_registered::<InputBinding>(&app));
        assert!(type_is_registered::<OrbitCamInputMode>(&app));
        assert!(type_is_registered::<OrbitCamKeyboardPreset>(&app));
        assert!(type_is_registered::<OrbitCamMouseDrag>(&app));
        assert!(type_is_registered::<OrbitCamMouseWheelZoom>(&app));
        assert!(type_is_registered::<OrbitCamOrbitBinding>(&app));
        assert!(type_is_registered::<OrbitCamPanBinding>(&app));
        assert!(type_is_registered::<OrbitCamPinchZoom>(&app));
        assert!(type_is_registered::<OrbitCamPreset>(&app));
        assert!(type_is_registered::<OrbitCamPresetKind>(&app));
        assert!(type_is_registered::<CameraInputScalePolicy>(&app));
        assert!(type_is_registered::<OrbitCamInputGain>(&app));
        assert!(type_is_registered::<OrbitCamSimpleMouseKeyboardPreset>(
            &app
        ));
        assert!(type_is_registered::<OrbitCamSimpleMousePreset>(&app));
        assert!(type_is_registered::<CameraSlowMode>(&app));
        assert!(type_is_registered::<OrbitCamTouchBinding>(&app));
        assert!(type_is_registered::<OrbitCamTouchBindingConfig>(&app));
        assert!(type_is_registered::<OrbitCamTrackpadScroll>(&app));
        assert!(type_is_registered::<OrbitCamZoomBinding>(&app));
        assert!(type_is_registered::<ZoomInversion>(&app));
    }
}
