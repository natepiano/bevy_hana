use bevy::prelude::GamepadAxis;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::BindingGates;
use super::CameraInputModeKind;
use super::CameraSemanticAction;
use super::CameraSlowMode;
use super::FreeCamControlDirection;
use super::FreeCamInputMode;
use super::FreeCamInteractionKind;
use super::GateInput;
use super::GatePolarity;
use super::HeldActionBindingEntry;
use super::HeldCameraAction;
use super::ImpulseActionBindingEntry;
use super::InputAxisTransform;
use super::InputBindingDescriptor;
use super::InputBindingEntry;
use super::InputMode;
use super::InteractionSources;
use super::OrbitCamInputMode;
use super::OrbitCamInteractionKind;
use super::bindings;
use super::constants::APP_AUTHORED_INPUT_ROW_LABEL;
use super::constants::CUSTOM_BINDINGS_MODE_VALUE;
use super::constants::CUSTOM_INPUT_ROW_LABEL;
use super::constants::FREE_CAM_CAMERA_LABEL;
use super::constants::GAMEPAD_BINDING_SOURCE_LABEL;
use super::constants::GAMEPAD_BINDINGS_ROW_LABEL;
use super::constants::INPUT_BINDING_SOURCE_LABEL;
use super::constants::INPUT_MODE_LABEL;
use super::constants::INVERT_Y_BINDING_LABEL;
use super::constants::INVERT_Y_STATUS_LABEL;
use super::constants::KEYBOARD_BINDING_SOURCE_LABEL;
use super::constants::MANUAL_INPUT_SOURCE_LABEL;
use super::constants::MANUAL_MODE_VALUE;
use super::constants::MOUSE_BINDING_SOURCE_LABEL;
use super::constants::MOUSE_DESCRIPTOR_LABEL;
use super::constants::ONE_FINGER_TOUCH_ROW_LABEL;
use super::constants::ORBIT_CAM_CAMERA_LABEL;
use super::constants::PINCH_SOURCE_LABEL;
use super::constants::PINCH_ZOOM_IN_LABEL;
use super::constants::PINCH_ZOOM_OUT_LABEL;
use super::constants::PRESET_MODE_LABEL;
use super::constants::ROLL_DISABLED_ROW_LABEL;
use super::constants::ROLL_LEFT_ACTION_LABEL;
use super::constants::ROLL_RIGHT_ACTION_LABEL;
use super::constants::SMOOTH_SCROLL_ZOOM_IN_LABEL;
use super::constants::SMOOTH_SCROLL_ZOOM_OUT_LABEL;
use super::constants::TOUCH_SOURCE_LABEL;
use super::constants::TRACKPAD_SOURCE_LABEL;
use super::constants::TRANSLATE_BOOST_ACTION_LABEL;
use super::constants::TRANSLATE_DOWN_ACTION_LABEL;
use super::constants::TRANSLATE_UP_ACTION_LABEL;
use super::constants::TWO_FINGER_TOUCH_ROW_LABEL;
use super::constants::WHEEL_SOURCE_LABEL;
use super::constants::WHEEL_ZOOM_IN_LABEL;
use super::constants::WHEEL_ZOOM_OUT_LABEL;
use crate::FreeCam;
use crate::ScalarLimit;
use crate::free_cam::FreeCamBindings;
use crate::free_cam::FreeCamLookPitch;
use crate::orbit_cam::CameraInputGamepadSelectionPolicy;
use crate::orbit_cam::OrbitCamBindingWithInputGain;
use crate::orbit_cam::OrbitCamBindings;
use crate::orbit_cam::OrbitCamButtonDragZoom;
use crate::orbit_cam::OrbitCamLineScroll;
use crate::orbit_cam::OrbitCamTouchBinding;
use crate::orbit_cam::OrbitCamTouchBindingConfig;
use crate::orbit_cam::OrbitCamTrackpadScroll;
use crate::orbit_cam::ZoomInversion;

/// Derived, read-only display summary of the controls configured for a
/// lagrange camera input mode.
///
/// `CameraControlSummary` is produced from runtime input-mode settings and,
/// when available, camera state, and is rendered by UI/help surfaces such as
/// Fairy Dust panels. It does not contain the runtime input bindings consumed
/// by the controller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraControlSummary {
    /// Camera type label.
    pub camera_label: String,
    /// Label for the selected input mode category.
    pub mode_label:   String,
    /// Value for the selected input mode category.
    pub mode_value:   String,
    /// Whether an enabled control exposes a displayable slow-mode toggle.
    pub slow_mode:    SlowModeHint,
    /// Display bindings that describe the selected input mode and related
    /// control settings.
    pub bindings:     Vec<CameraControlBinding>,
}

/// One displayable control binding in a [`CameraControlSummary`].
///
/// `CameraControlBinding` is a derived UI/help description. It is not the
/// runtime input binding installed into `bevy_enhanced_input`; it is the
/// already-resolved label, action section, source metadata, speed lane, and
/// binding kind used by renderers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraControlBinding {
    /// Semantic camera action associated with this binding.
    pub action:              CameraControlAction,
    /// Human-readable binding label.
    pub label:               String,
    /// Sources associated with this binding.
    pub interaction_sources: InteractionSources,
    /// Whether this binding belongs to normal or slow controls.
    pub speed:               ControlSpeed,
    /// Whether this binding directly controls the action or changes a related
    /// setting.
    pub kind:                CameraControlBindingKind,
    /// Whether the right column uses the action's name or a replacement.
    /// Replacements mark decomposed rows whose action name alone would be ambiguous — a
    /// gamepad translate's `Up`/`Down` triggers, its `Boost` gate, or a split
    /// roll's `Roll →`/`Roll ←` directions.
    pub action_label:        ActionColumnLabel,
    /// Live-highlight filter for a decomposed `FreeCam` row, so a panel
    /// lights only the direction currently engaged.
    pub direction:           FreeCamDirectionFilter,
}

/// A displayable slow-mode toggle, or the absence of one for effective controls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SlowModeHint {
    /// The key hint that toggles slow variants.
    ToggledBy(String),
    /// No enabled control has a displayable slow toggle.
    #[default]
    NoDisplayableSlowToggle,
}

/// The value displayed in a control row's action column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ActionColumnLabel {
    /// Display the action's own name.
    #[default]
    ActionName,
    /// Display a more specific label for a decomposed row.
    Replacement(String),
}

/// Which `FreeCam` engagement may highlight a control row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FreeCamDirectionFilter {
    /// Any engagement of this row's interaction sources.
    #[default]
    AnySourceEngagement,
    /// Only engagement in this direction.
    Only(FreeCamControlDirection),
}

/// Kind of displayable control binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraControlBindingKind {
    /// The binding directly controls the action section, such as `rmb drag` for
    /// `Look` or `Q/E` for `Roll`.
    Direct,
    /// The binding changes a setting associated with the action section, such
    /// as `alt-i` toggling `Invert Y` under `Look`.
    Setting {
        /// Human-readable setting label shown as the binding value.
        value:      String,
        /// Whether this setting should be highlighted.
        activation: CameraControlActivation,
    },
}

/// Highlight state for a camera control setting binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum CameraControlActivation {
    /// Highlight the setting binding.
    Active,
    /// Render the setting binding without highlight.
    #[default]
    Inactive,
}

/// Semantic action represented by a camera control binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum CameraControlAction {
    /// Orbit around the camera focus.
    Orbit,
    /// Pan the camera focus.
    Pan,
    /// Zoom in or out without a specific direction.
    Zoom,
    /// Zoom in toward the target.
    ZoomIn,
    /// Zoom out away from the target.
    ZoomOut,
    /// Rotate the free-flight camera view.
    Look,
    /// Translate the free-flight camera position.
    Translate,
    /// Roll the free-flight camera around its forward axis.
    Roll,
    /// Reset the camera to its home pose.
    Home,
    /// Fallback for a camera action with no variant of its own.
    Other,
}

/// Point-in-time display summary of the controls configured for an `OrbitCam`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrbitCamControlSummary {
    /// Camera type label.
    pub camera_label: String,
    /// Label for the selected input mode category.
    pub mode_label:   String,
    /// Value for the selected input mode category.
    pub mode_value:   String,
    /// Display rows that describe the selected input mode.
    pub rows:         Vec<OrbitCamControlRow>,
}

/// One display row in an [`OrbitCamControlSummary`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrbitCamControlRow {
    /// Interaction kind controlled by this row.
    pub kind:                       OrbitCamInteractionKind,
    /// Human-readable binding label.
    pub label:                      String,
    /// Sources associated with this binding.
    pub camera_interaction_sources: InteractionSources,
    /// Whether this binding is the normal or the slow (precise) variant.
    pub speed:                      ControlSpeed,
    /// The zoom direction this row drives when it is direction-specific.
    pub zoom_direction:             ZoomRowFilter,
}

/// Which zoom engagement may highlight an orbit control row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomRowFilter {
    /// Orbit, pan, or a bidirectional zoom kept as one row.
    #[default]
    Unrestricted,
    /// Only zoom in this direction.
    Only(ZoomDirection),
}

/// The direction a zoom affordance drives the camera.
///
/// Every zoom row from a built-in preset carries one of these so the control
/// panel can highlight only the engaged direction — pressing the zoom-in
/// trigger lights only the zoom-in row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ZoomDirection {
    /// Moves the camera toward its target.
    In,
    /// Moves the camera away from its target.
    Out,
}

/// Distinguishes a normal binding from its slow, precise counterpart — for the
/// gamepad preset, the slow variant is the one gated behind `rb`/`lb`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum ControlSpeed {
    /// Default speed binding.
    #[default]
    Normal,
    /// Slow, precise binding engaged while a modifier gate is held.
    Slow,
}

impl OrbitCamControlRow {
    /// Overrides the binding speed variant.
    #[must_use]
    pub const fn with_speed(mut self, speed: ControlSpeed) -> Self {
        self.speed = speed;
        self
    }

    /// Tags this row with the zoom direction it drives.
    #[must_use]
    pub const fn with_zoom_direction(mut self, zoom_direction: ZoomDirection) -> Self {
        self.zoom_direction = ZoomRowFilter::Only(zoom_direction);
        self
    }
}

impl CameraControlBinding {
    /// Overrides the binding speed variant.
    #[must_use]
    pub const fn with_speed(mut self, speed: ControlSpeed) -> Self {
        self.speed = speed;
        self
    }

    /// Overrides the right-column label shown for this binding.
    #[must_use]
    pub fn with_action_label(mut self, action_label: impl Into<String>) -> Self {
        self.action_label = ActionColumnLabel::Replacement(action_label.into());
        self
    }

    /// Tags this binding with the decomposed `FreeCam` direction it drives, so a
    /// panel lights only this row when that direction is engaged.
    #[must_use]
    pub const fn with_direction(mut self, direction: FreeCamControlDirection) -> Self {
        self.direction = FreeCamDirectionFilter::Only(direction);
        self
    }
}

impl CameraControlAction {
    /// Builds the shared action for an `OrbitCam` row.
    #[must_use]
    pub const fn from_orbit(kind: OrbitCamInteractionKind, direction: ZoomRowFilter) -> Self {
        match (kind, direction) {
            (OrbitCamInteractionKind::Orbit, _) => Self::Orbit,
            (OrbitCamInteractionKind::Pan, _) => Self::Pan,
            (OrbitCamInteractionKind::Zoom, ZoomRowFilter::Only(ZoomDirection::In)) => Self::ZoomIn,
            (OrbitCamInteractionKind::Zoom, ZoomRowFilter::Only(ZoomDirection::Out)) => {
                Self::ZoomOut
            },
            (OrbitCamInteractionKind::Zoom, ZoomRowFilter::Unrestricted) => Self::Zoom,
        }
    }

    /// Builds the shared action for a `FreeCam` interaction kind.
    #[must_use]
    pub const fn from_free(kind: FreeCamInteractionKind) -> Self {
        match kind {
            FreeCamInteractionKind::Translate => Self::Translate,
            FreeCamInteractionKind::Look => Self::Look,
            FreeCamInteractionKind::Roll => Self::Roll,
        }
    }
}

impl From<OrbitCamControlRow> for CameraControlBinding {
    fn from(row: OrbitCamControlRow) -> Self {
        Self {
            action:              CameraControlAction::from_orbit(row.kind, row.zoom_direction),
            label:               row.label,
            interaction_sources: row.camera_interaction_sources,
            speed:               row.speed,
            kind:                CameraControlBindingKind::Direct,
            action_label:        ActionColumnLabel::ActionName,
            direction:           FreeCamDirectionFilter::AnySourceEngagement,
        }
    }
}

impl From<OrbitCamControlSummary> for CameraControlSummary {
    fn from(summary: OrbitCamControlSummary) -> Self {
        Self {
            camera_label: summary.camera_label,
            mode_label:   summary.mode_label,
            mode_value:   summary.mode_value,
            slow_mode:    SlowModeHint::NoDisplayableSlowToggle,
            bindings:     summary.rows.into_iter().map(Into::into).collect(),
        }
    }
}

/// Describes the effective controls for a camera input mode.
#[must_use]
pub fn describe_controls<K: CameraInputModeKind>(mode: &InputMode<K>) -> CameraControlSummary {
    K::describe_controls(mode)
}

/// Describes the effective controls for a camera and input mode.
#[must_use]
pub fn describe_controls_for<K: CameraInputModeKind>(
    camera: &K::Camera,
    mode: &InputMode<K>,
) -> CameraControlSummary {
    K::describe_controls_for(camera, mode)
}

/// Describes the effective `OrbitCam` controls for an input mode.
#[must_use]
pub fn describe_orbit_cam_controls(mode: &OrbitCamInputMode) -> OrbitCamControlSummary {
    match mode {
        OrbitCamInputMode::Preset(preset) => match preset.to_bindings() {
            Ok(bindings) => describe_bindings(PRESET_MODE_LABEL, preset.kind().name(), &bindings),
            Err(_) => OrbitCamControlSummary {
                camera_label: ORBIT_CAM_CAMERA_LABEL.to_string(),
                mode_label:   PRESET_MODE_LABEL.to_string(),
                mode_value:   preset.kind().name().to_string(),
                rows:         Vec::new(),
            },
        },
        OrbitCamInputMode::Bindings(bindings) => {
            describe_bindings(INPUT_MODE_LABEL, CUSTOM_BINDINGS_MODE_VALUE, bindings)
        },
        OrbitCamInputMode::Manual => describe_manual_controls(),
    }
}

/// Describes the effective `OrbitCam` controls as a camera-neutral summary.
#[must_use]
pub(super) fn describe_orbit_camera_controls(mode: &OrbitCamInputMode) -> CameraControlSummary {
    let summary = describe_orbit_cam_controls(mode);
    let has_rows = !summary.rows.is_empty();
    let slow_mode = orbit_slow_mode(mode, has_rows)
        .map(|slow| SlowModeHint::ToggledBy(slow_mode_binding_hint(slow)))
        .unwrap_or_default();
    let mut camera_summary: CameraControlSummary = summary.into();
    camera_summary.slow_mode = slow_mode;
    camera_summary.bindings.extend(orbit_home_bindings(mode));
    camera_summary
}

/// Describes the effective `FreeCam` controls for an input mode.
#[must_use]
pub(super) fn describe_free_cam_controls(mode: &FreeCamInputMode) -> CameraControlSummary {
    describe_free_controls(mode, FreeCamRollControl::Enabled)
}

/// Describes the effective `FreeCam` controls for a camera and input mode.
#[must_use]
pub(super) fn describe_free_cam_controls_for(
    camera: &FreeCam,
    mode: &FreeCamInputMode,
) -> CameraControlSummary {
    describe_free_controls(mode, FreeCamRollControl::from_camera(camera))
}

fn describe_free_controls(
    mode: &FreeCamInputMode,
    roll_control: FreeCamRollControl,
) -> CameraControlSummary {
    match mode {
        FreeCamInputMode::Preset(preset) => match preset.to_bindings() {
            Ok(bindings) => {
                describe_free_bindings(PRESET_MODE_LABEL, preset.name(), &bindings, roll_control)
            },
            Err(_) => CameraControlSummary {
                camera_label: FREE_CAM_CAMERA_LABEL.to_string(),
                mode_label: PRESET_MODE_LABEL.to_string(),
                mode_value: preset.name().to_string(),
                ..Default::default()
            },
        },
        FreeCamInputMode::Bindings(bindings) => describe_free_bindings(
            INPUT_MODE_LABEL,
            CUSTOM_BINDINGS_MODE_VALUE,
            bindings,
            roll_control,
        ),
        FreeCamInputMode::Manual => CameraControlSummary {
            camera_label: FREE_CAM_CAMERA_LABEL.to_string(),
            mode_label: INPUT_MODE_LABEL.to_string(),
            mode_value: MANUAL_MODE_VALUE.to_string(),
            bindings: vec![
                camera_control_binding(
                    CameraControlAction::Look,
                    APP_AUTHORED_INPUT_ROW_LABEL,
                    InteractionSources::MANUAL,
                ),
                camera_control_binding(
                    CameraControlAction::Translate,
                    APP_AUTHORED_INPUT_ROW_LABEL,
                    InteractionSources::MANUAL,
                ),
                free_roll_control_row(
                    roll_control,
                    APP_AUTHORED_INPUT_ROW_LABEL,
                    InteractionSources::MANUAL,
                ),
            ],
            ..Default::default()
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FreeCamRollControl {
    Enabled,
    Disabled,
}

impl FreeCamRollControl {
    fn from_camera(camera: &FreeCam) -> Self {
        match camera.roll.limit() {
            ScalarLimit::Clamp { min, max }
                if min.abs() <= f32::EPSILON && max.abs() <= f32::EPSILON =>
            {
                Self::Disabled
            },
            _ => Self::Enabled,
        }
    }
}

fn orbit_slow_mode(mode: &OrbitCamInputMode, has_effective_rows: bool) -> Option<CameraSlowMode> {
    if !has_effective_rows {
        return None;
    }
    match mode {
        OrbitCamInputMode::Preset(preset) => preset.to_bindings().ok()?.slow_mode().copied(),
        OrbitCamInputMode::Bindings(bindings) => bindings.slow_mode().copied(),
        OrbitCamInputMode::Manual => None,
    }
}

fn describe_free_bindings(
    mode_label: &str,
    mode_value: &str,
    bindings: &FreeCamBindings,
    roll_control: FreeCamRollControl,
) -> CameraControlSummary {
    let mut control_bindings = Vec::new();
    control_bindings.extend(bindings.look().enabled_entries().map(|entry| {
        camera_control_binding(
            CameraControlAction::Look,
            held_binding_stem(entry),
            entry.sources(),
        )
    }));
    if bindings.enabled_trackpad_look().next().is_some() {
        control_bindings.push(camera_control_binding(
            CameraControlAction::Look,
            trackpad_stem(ModKeys::empty()),
            InteractionSources::SMOOTH_SCROLL,
        ));
    }
    control_bindings.push(look_pitch_setting_binding(bindings.look_pitch()));
    control_bindings.extend(describe_free_translate_rows(
        bindings.translate().enabled_entries(),
    ));
    match roll_control {
        FreeCamRollControl::Enabled => {
            control_bindings.extend(describe_free_roll_rows(bindings.roll().enabled_entries()));
        },
        FreeCamRollControl::Disabled => {
            control_bindings.push(camera_control_binding(
                CameraControlAction::Roll,
                ROLL_DISABLED_ROW_LABEL,
                InteractionSources::NONE,
            ));
        },
    }
    control_bindings.extend(home_binding_rows(bindings.home().bindings()));
    let slow_mode = effective_free_slow_mode(bindings)
        .copied()
        .map(|slow| SlowModeHint::ToggledBy(slow_mode_binding_hint(slow)))
        .unwrap_or_default();
    CameraControlSummary {
        camera_label: FREE_CAM_CAMERA_LABEL.to_string(),
        mode_label: mode_label.to_string(),
        mode_value: mode_value.to_string(),
        slow_mode,
        bindings: control_bindings,
    }
}

/// Builds the `FreeCam` translate rows for a set of held translate bindings.
///
/// A gamepad move binding fuses stick axes with up/down triggers into one
/// `gamepad_vec3` entry, so each such entry is split into a stick row plus, for
/// the ungated entry, one row per vertical trigger. Keyboard translate keys stay
/// a single row per entry.
fn describe_free_translate_rows<'a, A: HeldCameraAction + 'a>(
    entries: impl Iterator<Item = &'a HeldActionBindingEntry<A>>,
) -> Vec<CameraControlBinding> {
    let entries = entries.collect::<Vec<_>>();
    if !entries.is_empty() && entries.iter().all(|&entry| is_gamepad_move_entry(entry)) {
        return decompose_gamepad_translate(&entries);
    }
    entries
        .into_iter()
        .map(|entry| {
            camera_control_binding(
                CameraControlAction::Translate,
                held_binding_stem(entry),
                entry.sources(),
            )
        })
        .collect()
}

/// Splits gamepad `gamepad_vec3` translate entries into stick and vertical rows.
/// The stick rows come first in entry order (the ungated move, then the
/// boost-gated move); the two vertical trigger rows follow, derived once from
/// the ungated entry.
fn decompose_gamepad_translate<A: HeldCameraAction>(
    entries: &[&HeldActionBindingEntry<A>],
) -> Vec<CameraControlBinding> {
    let mut rows = entries
        .iter()
        .filter_map(|&entry| gamepad_translate_stick_row(entry))
        .collect::<Vec<_>>();
    let base = entries
        .iter()
        .copied()
        .find(|&entry| !has_required_gate(entry))
        .or_else(|| entries.first().copied());
    if let Some(base) = base {
        rows.extend(gamepad_translate_vertical_rows(base));
    }
    rows
}

/// Builds the stick row for one gamepad move entry, keeping its gate prefix and
/// tagging the boost-gated entry with the `Boost` label and direction.
fn gamepad_translate_stick_row<A: HeldCameraAction>(
    entry: &HeldActionBindingEntry<A>,
) -> Option<CameraControlBinding> {
    let axes = entry
        .enabled_motion_entries()
        .filter(|motion| matches!(motion.binding(), Binding::GamepadAxis(_)))
        .collect::<Vec<_>>();
    let stem = gamepad_axis_stem(&axes)?;
    let row = camera_control_binding(
        CameraControlAction::Translate,
        with_required_gates(entry.gates(), stem),
        entry.sources(),
    );
    Some(if has_required_gate(entry) {
        row.with_action_label(TRANSLATE_BOOST_ACTION_LABEL)
            .with_direction(FreeCamControlDirection::Boost)
    } else {
        row.with_direction(FreeCamControlDirection::Stick)
    })
}

/// Builds the `Up`/`Down` rows for the vertical trigger buttons of a gamepad
/// move entry, deriving each direction from the button's net input sign.
fn gamepad_translate_vertical_rows<A: HeldCameraAction>(
    entry: &HeldActionBindingEntry<A>,
) -> Vec<CameraControlBinding> {
    entry
        .enabled_motion_entries()
        .filter_map(|motion| {
            gamepad_button_entry_label(motion).map(|label| {
                let (action_label, direction) = if entry_net_sign(motion) < 0.0 {
                    (TRANSLATE_DOWN_ACTION_LABEL, FreeCamControlDirection::Down)
                } else {
                    (TRANSLATE_UP_ACTION_LABEL, FreeCamControlDirection::Up)
                };
                camera_control_binding(CameraControlAction::Translate, label, entry.sources())
                    .with_action_label(action_label)
                    .with_direction(direction)
            })
        })
        .collect()
}

/// Builds the `FreeCam` roll rows for a set of held roll bindings. A
/// bidirectional gamepad-button roll splits into one row per direction
/// (`Roll →` / `Roll ←`); every other roll binding stays a single row.
fn describe_free_roll_rows<'a, A: HeldCameraAction + 'a>(
    entries: impl Iterator<Item = &'a HeldActionBindingEntry<A>>,
) -> Vec<CameraControlBinding> {
    entries.flat_map(describe_free_roll_entry).collect()
}

fn describe_free_roll_entry<A: HeldCameraAction>(
    entry: &HeldActionBindingEntry<A>,
) -> Vec<CameraControlBinding> {
    let motion = entry.enabled_motion_entries().collect::<Vec<_>>();
    let split = if motion.len() == 2 {
        motion
            .iter()
            .map(|&motion_entry| {
                gamepad_button_entry_label(motion_entry).map(|label| {
                    let (action_label, direction) = if entry_net_sign(motion_entry) < 0.0 {
                        (ROLL_LEFT_ACTION_LABEL, FreeCamControlDirection::RollLeft)
                    } else {
                        (ROLL_RIGHT_ACTION_LABEL, FreeCamControlDirection::RollRight)
                    };
                    camera_control_binding(
                        CameraControlAction::Roll,
                        with_required_gates(entry.gates(), label),
                        entry.sources(),
                    )
                    .with_action_label(action_label)
                    .with_direction(direction)
                })
            })
            .collect::<Option<Vec<_>>>()
    } else {
        None
    };

    split.unwrap_or_else(|| {
        vec![camera_control_binding(
            CameraControlAction::Roll,
            held_binding_stem(entry),
            entry.sources(),
        )]
    })
}

/// Returns the button label for a gamepad-button motion entry, or `None` for any
/// other binding kind — this is what limits the roll split and vertical-trigger
/// decomposition to gamepad buttons.
fn gamepad_button_entry_label(entry: &InputBindingEntry) -> Option<String> {
    match entry.binding() {
        Binding::GamepadButton(button) => Some(gamepad_button_label(button)),
        Binding::Keyboard { .. }
        | Binding::MouseButton { .. }
        | Binding::MouseMotion { .. }
        | Binding::MouseWheel { .. }
        | Binding::GamepadAxis(_)
        | Binding::AnyKey
        | Binding::Custom(_)
        | Binding::None => None,
    }
}

fn is_gamepad_move_entry<A: HeldCameraAction>(entry: &HeldActionBindingEntry<A>) -> bool {
    let has_axis = entry
        .enabled_motion_entries()
        .any(|motion| matches!(motion.binding(), Binding::GamepadAxis(_)));
    let has_button = entry
        .enabled_motion_entries()
        .any(|motion| matches!(motion.binding(), Binding::GamepadButton(_)));
    has_axis && has_button
}

fn has_required_gate<A: HeldCameraAction>(entry: &HeldActionBindingEntry<A>) -> bool {
    entry
        .gates()
        .entries()
        .iter()
        .any(|gate| gate.polarity == GatePolarity::Required)
}

fn free_roll_control_row(
    roll_control: FreeCamRollControl,
    enabled_label: impl Into<String>,
    sources: InteractionSources,
) -> CameraControlBinding {
    match roll_control {
        FreeCamRollControl::Enabled => {
            camera_control_binding(CameraControlAction::Roll, enabled_label, sources)
        },
        FreeCamRollControl::Disabled => camera_control_binding(
            CameraControlAction::Roll,
            ROLL_DISABLED_ROW_LABEL,
            InteractionSources::NONE,
        ),
    }
}

fn orbit_home_bindings(mode: &OrbitCamInputMode) -> Vec<CameraControlBinding> {
    match mode {
        OrbitCamInputMode::Preset(preset) => {
            preset.to_bindings().ok().map_or_else(Vec::new, |bindings| {
                home_binding_rows(bindings.home().bindings())
            })
        },
        OrbitCamInputMode::Bindings(bindings) => home_binding_rows(bindings.home().bindings()),
        OrbitCamInputMode::Manual => Vec::new(),
    }
}

fn home_binding_rows(bindings: impl IntoIterator<Item = Binding>) -> Vec<CameraControlBinding> {
    bindings.into_iter().filter_map(home_binding_row).collect()
}

fn home_binding_row(binding: Binding) -> Option<CameraControlBinding> {
    home_binding_label(binding).map(|label| {
        camera_control_binding(
            CameraControlAction::Home,
            label,
            bindings::sources_for_binding(binding),
        )
    })
}

fn home_binding_label(binding: Binding) -> Option<String> {
    match binding {
        Binding::Keyboard { key, mod_keys } => Some(with_mod_keys(mod_keys, key_label(key))),
        Binding::GamepadButton(button) => Some(gamepad_button_label(button)),
        Binding::MouseButton { .. }
        | Binding::MouseMotion { .. }
        | Binding::MouseWheel { .. }
        | Binding::GamepadAxis(_)
        | Binding::AnyKey
        | Binding::Custom(_)
        | Binding::None => None,
    }
}

fn look_pitch_setting_binding(look_pitch: FreeCamLookPitch) -> CameraControlBinding {
    let activation = match look_pitch {
        FreeCamLookPitch::Normal => CameraControlActivation::Inactive,
        FreeCamLookPitch::Inverted => CameraControlActivation::Active,
    };
    CameraControlBinding {
        action:              CameraControlAction::Look,
        label:               INVERT_Y_BINDING_LABEL.to_string(),
        interaction_sources: InteractionSources::KEYBOARD,
        speed:               ControlSpeed::Normal,
        kind:                CameraControlBindingKind::Setting {
            value: INVERT_Y_STATUS_LABEL.to_string(),
            activation,
        },
        action_label:        ActionColumnLabel::ActionName,
        direction:           FreeCamDirectionFilter::AnySourceEngagement,
    }
}

fn slow_mode_binding_hint(slow_mode: CameraSlowMode) -> String {
    let key = key_label(slow_mode.toggle_key);
    if slow_mode.mod_keys.is_empty() {
        key
    } else {
        format!("{} {key}", compact_mod_keys(slow_mode.mod_keys))
    }
}

pub(super) fn effective_slow_mode(bindings: &OrbitCamBindings) -> Option<&CameraSlowMode> {
    let slow_mode = bindings.slow_mode()?;
    (!effective_control_rows(bindings).is_empty()).then_some(slow_mode)
}

pub(super) fn effective_free_slow_mode(bindings: &FreeCamBindings) -> Option<&CameraSlowMode> {
    let slow_mode = bindings.slow_mode()?;
    has_effective_free_controls(bindings).then_some(slow_mode)
}

fn has_effective_free_controls(bindings: &FreeCamBindings) -> bool {
    bindings.translate().enabled_entries().next().is_some()
        || bindings.look().enabled_entries().next().is_some()
        || bindings.enabled_trackpad_look().next().is_some()
        || bindings.roll().enabled_entries().next().is_some()
}

fn describe_manual_controls() -> OrbitCamControlSummary {
    OrbitCamControlSummary {
        camera_label: ORBIT_CAM_CAMERA_LABEL.to_string(),
        mode_label:   INPUT_MODE_LABEL.to_string(),
        mode_value:   MANUAL_MODE_VALUE.to_string(),
        rows:         vec![
            control_row(
                OrbitCamInteractionKind::Orbit,
                APP_AUTHORED_INPUT_ROW_LABEL,
                InteractionSources::MANUAL,
            ),
            control_row(
                OrbitCamInteractionKind::Pan,
                APP_AUTHORED_INPUT_ROW_LABEL,
                InteractionSources::MANUAL,
            ),
            control_row(
                OrbitCamInteractionKind::Zoom,
                APP_AUTHORED_INPUT_ROW_LABEL,
                InteractionSources::MANUAL,
            ),
        ],
    }
}

fn describe_bindings(
    mode_label: &str,
    mode_value: &str,
    bindings: &OrbitCamBindings,
) -> OrbitCamControlSummary {
    OrbitCamControlSummary {
        camera_label: ORBIT_CAM_CAMERA_LABEL.to_string(),
        mode_label:   mode_label.to_string(),
        mode_value:   mode_value.to_string(),
        rows:         effective_control_rows(bindings),
    }
}

fn effective_control_rows(bindings: &OrbitCamBindings) -> Vec<OrbitCamControlRow> {
    let mut rows = Vec::new();

    rows.extend(
        bindings
            .orbit()
            .enabled_entries()
            .map(|entry| describe_held_entry(entry, OrbitCamInteractionKind::Orbit)),
    );
    rows.extend(
        enabled_input_gain_entries(bindings.trackpad_orbit())
            .map(|trackpad| describe_trackpad(trackpad, OrbitCamInteractionKind::Orbit)),
    );
    rows.extend(
        enabled_input_gain_entries(bindings.line_orbit()).map(|binding| {
            control_row(
                OrbitCamInteractionKind::Orbit,
                with_mod_keys(binding.mod_keys, WHEEL_SOURCE_LABEL.to_string()),
                InteractionSources::WHEEL,
            )
        }),
    );

    rows.extend(
        bindings
            .pan()
            .enabled_entries()
            .map(|entry| describe_held_entry(entry, OrbitCamInteractionKind::Pan)),
    );
    rows.extend(
        enabled_input_gain_entries(bindings.trackpad_pan())
            .map(|trackpad| describe_trackpad(trackpad, OrbitCamInteractionKind::Pan)),
    );
    rows.extend(
        enabled_input_gain_entries(bindings.line_pan()).map(|binding| {
            control_row(
                OrbitCamInteractionKind::Pan,
                with_mod_keys(binding.mod_keys, WHEEL_SOURCE_LABEL.to_string()),
                InteractionSources::WHEEL,
            )
        }),
    );

    // Every zoom source shows one row per direction so zoom in and zoom out read
    // separately. `inversion_sign` flips the synthesized in/out tags for sources
    // routed through the inverting zoom path (wheel, pinch, smooth-scroll); the
    // native trigger/key bindings derive their direction from their own scale.
    let inversion_sign = zoom_inversion_sign(bindings.zoom_inversion());

    for entry in bindings.zoom_smooth().enabled_entries() {
        rows.extend(describe_zoom_held_entry(entry));
    }
    rows.extend(
        bindings
            .zoom_coarse()
            .enabled_entries()
            .map(|entry| describe_action_entry(entry, OrbitCamInteractionKind::Zoom)),
    );

    if enabled_input_gain_option(bindings.mouse_wheel_zoom()).is_some() {
        push_zoom_pair(
            &mut rows,
            WHEEL_ZOOM_IN_LABEL,
            WHEEL_ZOOM_OUT_LABEL,
            InteractionSources::WHEEL,
            inversion_sign,
        );
    }

    for trackpad in enabled_input_gain_entries(bindings.trackpad_zoom()) {
        push_trackpad_zoom_pair(&mut rows, trackpad, inversion_sign);
    }

    for binding in enabled_input_gain_entries(bindings.line_zoom()) {
        push_line_zoom_pair(&mut rows, binding, inversion_sign);
    }

    if let Some(button_drag) = enabled_input_gain_option(bindings.button_drag_zoom()) {
        rows.push(describe_button_drag(button_drag));
    }

    if enabled_input_gain_option(bindings.pinch_zoom_binding()).is_some() {
        push_zoom_pair(
            &mut rows,
            PINCH_ZOOM_IN_LABEL,
            PINCH_ZOOM_OUT_LABEL,
            InteractionSources::PINCH,
            inversion_sign,
        );
    }

    if let Some(touch) = bindings
        .touch_config()
        .filter(|touch| touch.has_enabled_action())
    {
        rows.extend(describe_touch(touch));
    }

    if bindings.gamepad() == CameraInputGamepadSelectionPolicy::Active
        && !rows.iter().any(|row| {
            row.camera_interaction_sources
                .contains(InteractionSources::GAMEPAD)
        })
    {
        rows.push(control_row(
            OrbitCamInteractionKind::Orbit,
            GAMEPAD_BINDINGS_ROW_LABEL,
            InteractionSources::GAMEPAD,
        ));
    }

    rows
}

fn enabled_input_gain_entries<T: Copy>(
    entries: &[OrbitCamBindingWithInputGain<T>],
) -> impl Iterator<Item = OrbitCamBindingWithInputGain<T>> + '_ {
    entries
        .iter()
        .copied()
        .filter(|entry| entry.input_gain().is_enabled())
}

fn enabled_input_gain_option<T: Copy>(
    entry: Option<OrbitCamBindingWithInputGain<T>>,
) -> Option<OrbitCamBindingWithInputGain<T>> {
    entry.filter(|entry| entry.input_gain().is_enabled())
}

fn describe_held_entry<A: HeldCameraAction>(
    entry: &HeldActionBindingEntry<A>,
    kind: OrbitCamInteractionKind,
) -> OrbitCamControlRow {
    let label = held_binding_stem(entry);
    control_row(kind, label, entry.sources()).with_speed(entry.speed())
}

fn describe_action_entry<A: CameraSemanticAction>(
    entry: &ImpulseActionBindingEntry<A>,
    kind: OrbitCamInteractionKind,
) -> OrbitCamControlRow {
    let label = descriptor_stem(entry.binding_descriptor(), entry.sources());
    control_row(kind, label, entry.sources())
}

fn describe_trackpad(
    trackpad: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
    kind: OrbitCamInteractionKind,
) -> OrbitCamControlRow {
    control_row(
        kind,
        trackpad_stem(trackpad.binding().mod_keys),
        InteractionSources::SMOOTH_SCROLL,
    )
}

/// Describes one smooth-zoom held binding. A single signed binding (a gamepad
/// trigger or one zoom key) stays one row tagged with its direction; a
/// bidirectional binding (the keyboard `+`/`-` pair) splits into one row per
/// signed entry so each direction displays on its own.
fn describe_zoom_held_entry<A: HeldCameraAction>(
    entry: &HeldActionBindingEntry<A>,
) -> Vec<OrbitCamControlRow> {
    let motion_entries = entry.enabled_motion_entries().collect::<Vec<_>>();

    if motion_entries.len() == 1 {
        let direction = zoom_direction_from_sign(entry_net_sign(motion_entries[0]));
        return vec![
            control_row(
                OrbitCamInteractionKind::Zoom,
                held_binding_stem(entry),
                entry.sources(),
            )
            .with_speed(entry.speed())
            .with_zoom_direction(direction),
        ];
    }

    let split = motion_entries
        .iter()
        .map(|&motion_entry| {
            zoom_entry_label(motion_entry).map(|stem| {
                let direction = zoom_direction_from_sign(entry_net_sign(motion_entry));
                control_row(
                    OrbitCamInteractionKind::Zoom,
                    with_required_gates(entry.gates(), stem),
                    entry.sources(),
                )
                .with_speed(entry.speed())
                .with_zoom_direction(direction)
            })
        })
        .collect::<Option<Vec<_>>>();

    split.unwrap_or_else(|| vec![describe_held_entry(entry, OrbitCamInteractionKind::Zoom)])
}

/// Pushes the zoom-in and zoom-out rows for a bidirectional zoom source whose
/// two directions are synthesized rather than read from separate bindings
/// (mouse wheel, pinch, smooth-scroll). `inversion_sign` is `-1.0` when the
/// camera inverts zoom, which flips both tags so they keep matching the live
/// zoom sign.
fn push_zoom_pair(
    rows: &mut Vec<OrbitCamControlRow>,
    zoom_in_label: &str,
    zoom_out_label: &str,
    sources: InteractionSources,
    inversion_sign: f32,
) {
    rows.push(
        control_row(OrbitCamInteractionKind::Zoom, zoom_in_label, sources)
            .with_zoom_direction(zoom_direction_from_sign(inversion_sign)),
    );
    rows.push(
        control_row(OrbitCamInteractionKind::Zoom, zoom_out_label, sources)
            .with_zoom_direction(zoom_direction_from_sign(-inversion_sign)),
    );
}

/// Pushes the smooth-scroll zoom-in and zoom-out rows, prefixing the gesture
/// label with any required modifier keys (matching `trackpad_stem`).
fn push_trackpad_zoom_pair(
    rows: &mut Vec<OrbitCamControlRow>,
    trackpad: OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll>,
    inversion_sign: f32,
) {
    let label = |value: &str| {
        let mod_keys = trackpad.binding().mod_keys;
        if mod_keys.is_empty() {
            value.to_string()
        } else {
            with_mod_keys(mod_keys, value.to_lowercase())
        }
    };
    let zoom_in = label(SMOOTH_SCROLL_ZOOM_IN_LABEL);
    let zoom_out = label(SMOOTH_SCROLL_ZOOM_OUT_LABEL);
    push_zoom_pair(
        rows,
        &zoom_in,
        &zoom_out,
        InteractionSources::SMOOTH_SCROLL,
        inversion_sign,
    );
}

fn push_line_zoom_pair(
    rows: &mut Vec<OrbitCamControlRow>,
    binding: OrbitCamBindingWithInputGain<OrbitCamLineScroll>,
    inversion_sign: f32,
) {
    push_zoom_pair(
        rows,
        &with_mod_keys(binding.mod_keys, WHEEL_ZOOM_IN_LABEL.to_string()),
        &with_mod_keys(binding.mod_keys, WHEEL_ZOOM_OUT_LABEL.to_string()),
        InteractionSources::WHEEL,
        inversion_sign,
    );
}

/// Returns the net sign a binding entry contributes to its action, combining the
/// scale modifier's sign with any negating axis transform. `+1.0` zooms in,
/// `-1.0` zooms out.
fn entry_net_sign(entry: &InputBindingEntry) -> f32 {
    let scale_sign = entry.modifiers().scale().map_or(1.0, f32::signum);
    let transform_sign = match entry.modifiers().axis_transform() {
        InputAxisTransform::Negate
        | InputAxisTransform::SwizzleNegate
        | InputAxisTransform::SwizzleZNegate => -1.0,
        InputAxisTransform::None | InputAxisTransform::Swizzle | InputAxisTransform::SwizzleZ => {
            1.0
        },
    };
    scale_sign * transform_sign
}

fn zoom_direction_from_sign(sign: f32) -> ZoomDirection {
    if sign < 0.0 {
        ZoomDirection::Out
    } else {
        ZoomDirection::In
    }
}

const fn zoom_inversion_sign(inversion: ZoomInversion) -> f32 {
    match inversion {
        ZoomInversion::Normal => 1.0,
        ZoomInversion::Inverted => -1.0,
    }
}

/// Returns a per-entry label for a single binding entry of a bidirectional zoom
/// binding, or `None` for an entry kind that has no standalone label.
fn zoom_entry_label(entry: &InputBindingEntry) -> Option<String> {
    match entry.binding() {
        Binding::Keyboard { key, mod_keys } => Some(with_mod_keys(mod_keys, key_label(key))),
        Binding::GamepadButton(button) => Some(gamepad_button_label(button)),
        Binding::MouseButton { .. }
        | Binding::MouseMotion { .. }
        | Binding::MouseWheel { .. }
        | Binding::GamepadAxis(_)
        | Binding::AnyKey
        | Binding::Custom(_)
        | Binding::None => None,
    }
}

fn describe_button_drag(
    button_drag: OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom>,
) -> OrbitCamControlRow {
    control_row(
        OrbitCamInteractionKind::Zoom,
        format!("{} drag", mouse_button_label(button_drag.binding().button)),
        InteractionSources::MOUSE,
    )
}

fn describe_touch(touch: OrbitCamTouchBindingConfig) -> Vec<OrbitCamControlRow> {
    let mut rows = Vec::new();
    match touch.binding() {
        OrbitCamTouchBinding::OneFingerOrbit => {
            push_enabled_touch_row(
                &mut rows,
                touch.orbit_enabled(),
                OrbitCamInteractionKind::Orbit,
                ONE_FINGER_TOUCH_ROW_LABEL,
            );
            push_enabled_touch_row(
                &mut rows,
                touch.pan_enabled(),
                OrbitCamInteractionKind::Pan,
                TWO_FINGER_TOUCH_ROW_LABEL,
            );
            push_enabled_touch_row(
                &mut rows,
                touch.zoom_enabled(),
                OrbitCamInteractionKind::Zoom,
                TWO_FINGER_TOUCH_ROW_LABEL,
            );
        },
        OrbitCamTouchBinding::TwoFingerOrbit => {
            push_enabled_touch_row(
                &mut rows,
                touch.pan_enabled(),
                OrbitCamInteractionKind::Pan,
                ONE_FINGER_TOUCH_ROW_LABEL,
            );
            push_enabled_touch_row(
                &mut rows,
                touch.orbit_enabled(),
                OrbitCamInteractionKind::Orbit,
                TWO_FINGER_TOUCH_ROW_LABEL,
            );
            push_enabled_touch_row(
                &mut rows,
                touch.zoom_enabled(),
                OrbitCamInteractionKind::Zoom,
                TWO_FINGER_TOUCH_ROW_LABEL,
            );
        },
    }
    rows
}

fn push_enabled_touch_row(
    rows: &mut Vec<OrbitCamControlRow>,
    enabled: bool,
    kind: OrbitCamInteractionKind,
    label: &'static str,
) {
    if enabled {
        rows.push(touch_row(kind, label));
    }
}

fn touch_row(kind: OrbitCamInteractionKind, label: &'static str) -> OrbitCamControlRow {
    control_row(kind, label, InteractionSources::TOUCH)
}

fn held_binding_stem<A: HeldCameraAction>(entry: &HeldActionBindingEntry<A>) -> String {
    let stem = if let Some(label) =
        mouse_drag_stem(entry.motion_descriptor(), entry.engagement_descriptor())
    {
        label
    } else {
        let motion = descriptor_stem(entry.motion_descriptor(), entry.sources());
        if entry.motion_descriptor() == entry.engagement_descriptor() {
            motion
        } else {
            let engagement = descriptor_stem(entry.engagement_descriptor(), entry.sources());
            format!("{motion} {engagement}")
        }
    };

    with_required_gates(entry.gates(), stem)
}

fn with_required_gates(gates: &BindingGates, label: String) -> String {
    let required = gates
        .entries()
        .iter()
        .filter(|gate| gate.polarity == GatePolarity::Required)
        .map(|gate| match gate.input {
            GateInput::GamepadButton(button) => gamepad_button_label(button),
            GateInput::Key(key) => key_label(key),
        })
        .collect::<Vec<_>>();
    if required.is_empty() {
        label
    } else {
        format!("{} {label}", required.join(" "))
    }
}

fn mouse_drag_stem(
    motion: &InputBindingDescriptor,
    engagement: &InputBindingDescriptor,
) -> Option<String> {
    let motion_mod_keys = mouse_motion_mod_keys(motion)?;
    let (button, button_mod_keys) = engagement.enabled_mouse_button_engagement()?;
    if motion_mod_keys != button_mod_keys {
        return None;
    }

    Some(format!(
        "{} drag",
        mouse_button_stem(button, button_mod_keys)
    ))
}

fn mouse_motion_mod_keys(descriptor: &InputBindingDescriptor) -> Option<ModKeys> {
    descriptor
        .enabled_entries()
        .find_map(|entry| match entry.binding() {
            Binding::MouseMotion { mod_keys } => Some(mod_keys),
            Binding::Keyboard { .. }
            | Binding::MouseButton { .. }
            | Binding::MouseWheel { .. }
            | Binding::GamepadButton(_)
            | Binding::GamepadAxis(_)
            | Binding::AnyKey
            | Binding::Custom(_)
            | Binding::None => None,
        })
}

fn descriptor_stem(descriptor: &InputBindingDescriptor, sources: InteractionSources) -> String {
    let entries = descriptor.enabled_entries().collect::<Vec<_>>();
    if let Some(label) = keyboard_stem(&entries) {
        return label;
    }
    if let Some(label) = gamepad_axis_stem(&entries) {
        return label;
    }
    if let Some(label) = gamepad_button_stem(&entries) {
        return label;
    }
    if let Some((button, mod_keys)) = descriptor.enabled_mouse_button_engagement() {
        return mouse_button_stem(button, mod_keys);
    }
    if entries.iter().any(|entry| {
        matches!(
            entry.binding(),
            Binding::MouseWheel { .. } | Binding::MouseMotion { .. }
        )
    }) {
        return MOUSE_DESCRIPTOR_LABEL.to_string();
    }
    if entries
        .iter()
        .any(|entry| matches!(entry.binding(), Binding::Custom(_)))
    {
        return CUSTOM_INPUT_ROW_LABEL.to_string();
    }

    source_stem(sources).to_string()
}

fn keyboard_stem(entries: &[&InputBindingEntry]) -> Option<String> {
    let keys = entries
        .iter()
        .map(|entry| match entry.binding() {
            Binding::Keyboard { key, mod_keys } => Some((key, mod_keys)),
            Binding::MouseButton { .. }
            | Binding::MouseMotion { .. }
            | Binding::MouseWheel { .. }
            | Binding::GamepadButton(_)
            | Binding::GamepadAxis(_)
            | Binding::AnyKey
            | Binding::Custom(_)
            | Binding::None => None,
        })
        .collect::<Option<Vec<_>>>()?;

    match keys.as_slice() {
        [] => None,
        [(key, mod_keys)] => Some(with_mod_keys(*mod_keys, key_label(*key))),
        [(positive, _), (negative, _)] => Some(format!(
            "{} / {}",
            key_label(*positive),
            key_label(*negative)
        )),
        keys if is_arrow_keys(keys) => Some("Arrow keys".to_string()),
        keys if is_wasd(keys) => Some("WASD".to_string()),
        _ => Some("Keyboard keys".to_string()),
    }
}

fn gamepad_axis_stem(entries: &[&InputBindingEntry]) -> Option<String> {
    let gamepad_axes = entries
        .iter()
        .map(|entry| match entry.binding() {
            Binding::GamepadAxis(axis) => Some(axis),
            Binding::Keyboard { .. }
            | Binding::MouseButton { .. }
            | Binding::MouseMotion { .. }
            | Binding::MouseWheel { .. }
            | Binding::GamepadButton(_)
            | Binding::AnyKey
            | Binding::Custom(_)
            | Binding::None => None,
        })
        .collect::<Option<Vec<_>>>()?;

    match gamepad_axes.as_slice() {
        [] => None,
        [GamepadAxis::LeftStickX, GamepadAxis::LeftStickY]
        | [GamepadAxis::LeftStickY, GamepadAxis::LeftStickX] => Some("LS".to_string()),
        [GamepadAxis::RightStickX, GamepadAxis::RightStickY]
        | [GamepadAxis::RightStickY, GamepadAxis::RightStickX] => Some("RS".to_string()),
        [single_axis] => Some(gamepad_axis_label(*single_axis)),
        _ => Some("Gamepad axes".to_string()),
    }
}

fn gamepad_button_stem(entries: &[&InputBindingEntry]) -> Option<String> {
    let buttons = entries
        .iter()
        .map(|entry| match entry.binding() {
            Binding::GamepadButton(button) => Some(button),
            Binding::Keyboard { .. }
            | Binding::MouseButton { .. }
            | Binding::MouseMotion { .. }
            | Binding::MouseWheel { .. }
            | Binding::GamepadAxis(_)
            | Binding::AnyKey
            | Binding::Custom(_)
            | Binding::None => None,
        })
        .collect::<Option<Vec<_>>>()?;

    match buttons.as_slice() {
        [] => None,
        [GamepadButton::RightTrigger2, GamepadButton::LeftTrigger2]
        | [GamepadButton::LeftTrigger2, GamepadButton::RightTrigger2] => {
            Some("RT / LT".to_string())
        },
        [positive, negative] => Some(format!(
            "{} / {}",
            gamepad_button_label(*positive),
            gamepad_button_label(*negative)
        )),
        [button] => Some(gamepad_button_label(*button)),
        _ => Some("Gamepad buttons".to_string()),
    }
}

fn is_arrow_keys(keys: &[(KeyCode, ModKeys)]) -> bool {
    contains_key(keys, KeyCode::ArrowUp)
        && contains_key(keys, KeyCode::ArrowRight)
        && contains_key(keys, KeyCode::ArrowDown)
        && contains_key(keys, KeyCode::ArrowLeft)
}

fn is_wasd(keys: &[(KeyCode, ModKeys)]) -> bool {
    contains_key(keys, KeyCode::KeyW)
        && contains_key(keys, KeyCode::KeyD)
        && contains_key(keys, KeyCode::KeyS)
        && contains_key(keys, KeyCode::KeyA)
}

fn contains_key(keys: &[(KeyCode, ModKeys)], key: KeyCode) -> bool {
    keys.iter().any(|(candidate, _)| *candidate == key)
}

fn trackpad_stem(mod_keys: ModKeys) -> String {
    if mod_keys.is_empty() {
        TRACKPAD_SOURCE_LABEL.to_string()
    } else {
        format!(
            "{} {}",
            compact_mod_keys(mod_keys),
            TRACKPAD_SOURCE_LABEL.to_lowercase()
        )
    }
}

fn with_mod_keys(mod_keys: ModKeys, label: String) -> String {
    if mod_keys.is_empty() {
        label
    } else {
        format!("{} {label}", compact_mod_keys(mod_keys))
    }
}

fn compact_mod_keys(mod_keys: ModKeys) -> String { mod_keys.to_string().replace(" + ", " ") }

fn key_label(key: KeyCode) -> String {
    match key {
        KeyCode::ArrowUp => "↑".to_string(),
        KeyCode::ArrowRight => "→".to_string(),
        KeyCode::ArrowDown => "↓".to_string(),
        KeyCode::ArrowLeft => "←".to_string(),
        KeyCode::ControlLeft => "Ctrl".to_string(),
        KeyCode::Equal => "+".to_string(),
        KeyCode::Minus => "-".to_string(),
        KeyCode::Space => "Space".to_string(),
        KeyCode::KeyA => "A".to_string(),
        KeyCode::KeyD => "D".to_string(),
        KeyCode::KeyE => "E".to_string(),
        KeyCode::KeyH => "H".to_string(),
        KeyCode::KeyM => "M".to_string(),
        KeyCode::KeyQ => "Q".to_string(),
        KeyCode::KeyS => "S".to_string(),
        KeyCode::KeyW => "W".to_string(),
        _ => debug_name(key, "Key"),
    }
}

fn mouse_button_label(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "Left".to_string(),
        MouseButton::Right => "Right".to_string(),
        MouseButton::Middle => "Middle".to_string(),
        _ => format!("{button:?}"),
    }
}

fn mouse_button_stem(button: MouseButton, mod_keys: ModKeys) -> String {
    let label = mouse_button_label(button);
    if mod_keys.is_empty() {
        label
    } else {
        with_mod_keys(mod_keys, label.to_lowercase())
    }
}

fn gamepad_axis_label(axis: GamepadAxis) -> String {
    match axis {
        GamepadAxis::LeftStickX => "LS X".to_string(),
        GamepadAxis::LeftStickY => "LS Y".to_string(),
        GamepadAxis::RightStickX => "RS X".to_string(),
        GamepadAxis::RightStickY => "RS Y".to_string(),
        _ => debug_name(axis, ""),
    }
}

fn gamepad_button_label(button: GamepadButton) -> String {
    match button {
        GamepadButton::LeftTrigger => "LB".to_string(),
        GamepadButton::RightTrigger => "RB".to_string(),
        GamepadButton::LeftTrigger2 => "LT".to_string(),
        GamepadButton::RightTrigger2 => "RT".to_string(),
        GamepadButton::LeftThumb => "L3".to_string(),
        GamepadButton::RightThumb => "R3".to_string(),
        _ => debug_name(button, ""),
    }
}

fn debug_name(value: impl std::fmt::Debug, prefix: &str) -> String {
    let name = format!("{value:?}");
    name.strip_prefix(prefix).unwrap_or(&name).to_string()
}

const fn source_stem(sources: InteractionSources) -> &'static str {
    if sources.contains(InteractionSources::KEYBOARD) {
        KEYBOARD_BINDING_SOURCE_LABEL
    } else if sources.contains(InteractionSources::GAMEPAD) {
        GAMEPAD_BINDING_SOURCE_LABEL
    } else if sources.contains(InteractionSources::MOUSE) {
        MOUSE_BINDING_SOURCE_LABEL
    } else if sources.contains(InteractionSources::WHEEL) {
        WHEEL_SOURCE_LABEL
    } else if sources.contains(InteractionSources::SMOOTH_SCROLL) {
        TRACKPAD_SOURCE_LABEL
    } else if sources.contains(InteractionSources::PINCH) {
        PINCH_SOURCE_LABEL
    } else if sources.contains(InteractionSources::TOUCH) {
        TOUCH_SOURCE_LABEL
    } else if sources.contains(InteractionSources::MANUAL) {
        MANUAL_INPUT_SOURCE_LABEL
    } else {
        INPUT_BINDING_SOURCE_LABEL
    }
}

fn control_row(
    kind: OrbitCamInteractionKind,
    label: impl Into<String>,
    sources: InteractionSources,
) -> OrbitCamControlRow {
    OrbitCamControlRow {
        kind,
        label: label.into(),
        camera_interaction_sources: sources,
        speed: ControlSpeed::Normal,
        zoom_direction: ZoomRowFilter::Unrestricted,
    }
}

fn camera_control_binding(
    action: CameraControlAction,
    label: impl Into<String>,
    sources: InteractionSources,
) -> CameraControlBinding {
    CameraControlBinding {
        action,
        label: label.into(),
        interaction_sources: sources,
        speed: ControlSpeed::Normal,
        kind: CameraControlBindingKind::Direct,
        action_label: ActionColumnLabel::ActionName,
        direction: FreeCamDirectionFilter::AnySourceEngagement,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::free_cam::FreeCamBindings;
    use crate::free_cam::FreeCamKeyboardMousePreset;
    use crate::free_cam::FreeCamMouseLook;
    use crate::free_cam::FreeCamPreset;
    use crate::free_cam::FreeCamRollBinding;
    use crate::free_cam::FreeCamTrackpadLook;
    use crate::free_cam::FreeCamTranslateKeys;
    use crate::input::BindingsError;
    use crate::input::CUSTOM_SLOW_SCALE;
    use crate::input::CameraInputScalePolicy;
    use crate::input::CameraSlowMode;
    use crate::input::InputBinding;
    use crate::input::InputGain;
    use crate::orbit_cam::OrbitCamBlenderLikePreset;
    use crate::orbit_cam::OrbitCamButtonDragZoom;
    use crate::orbit_cam::OrbitCamGamepadPreset;
    use crate::orbit_cam::OrbitCamInputGain;
    use crate::orbit_cam::OrbitCamMouseDrag;
    use crate::orbit_cam::OrbitCamMouseWheelZoom;
    use crate::orbit_cam::OrbitCamPinchZoom;
    use crate::orbit_cam::OrbitCamPreset;
    use crate::orbit_cam::OrbitCamTouchBinding;
    use crate::orbit_cam::OrbitCamTrackpadScroll;

    fn assert_either_key_spacing(label: &str) {
        for (index, character) in label.char_indices() {
            if character == '/' {
                assert!(label[..index].ends_with(' '), "{label}");
                assert!(label[index + 1..].starts_with(' '), "{label}");
            }
        }
        let arrows = ['↑', '↓', '←', '→'];
        let characters: Vec<_> = label.chars().collect();
        assert!(
            !characters
                .windows(2)
                .any(|pair| arrows.contains(&pair[0]) && arrows.contains(&pair[1])),
            "{label}"
        );
    }

    #[test]
    fn every_preset_drawn_key_label_separates_either_keys() {
        for preset in [
            OrbitCamPreset::simple_mouse(),
            OrbitCamPreset::blender_like(),
            OrbitCamPreset::keyboard(),
            OrbitCamPreset::simple_mouse_keyboard(),
            OrbitCamPreset::blender_like_keyboard(),
            OrbitCamPreset::gamepad(),
        ] {
            let summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(preset));
            for row in summary.rows {
                assert_either_key_spacing(&row.label);
            }
        }
        for preset in [
            FreeCamPreset::keyboard_mouse(),
            FreeCamPreset::gamepad(),
            FreeCamPreset::gamepad_southpaw(),
        ] {
            let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(preset));
            for binding in summary.bindings {
                assert_either_key_spacing(&binding.label);
            }
        }
    }

    #[test]
    fn every_trackpad_row_names_the_two_finger_drag_source() {
        for preset in [
            OrbitCamPreset::simple_mouse(),
            OrbitCamPreset::blender_like(),
            OrbitCamPreset::keyboard(),
            OrbitCamPreset::simple_mouse_keyboard(),
            OrbitCamPreset::blender_like_keyboard(),
            OrbitCamPreset::gamepad(),
        ] {
            let summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(preset));
            for row in summary.rows {
                if row
                    .camera_interaction_sources
                    .contains(InteractionSources::SMOOTH_SCROLL)
                {
                    assert!(
                        row.label.to_lowercase().contains("two-finger drag"),
                        "{}",
                        row.label
                    );
                    assert!(
                        !row.label.to_lowercase().contains("scroll"),
                        "{}",
                        row.label
                    );
                }
            }
        }
    }

    #[test]
    fn every_preset_uses_lowercase_mouse_buttons_after_modifiers() {
        let assert_button_case = |label: &str| {
            for (index, word) in label.split_whitespace().enumerate() {
                if index > 0 {
                    assert!(!["Left", "Right", "Middle"].contains(&word), "{label}");
                }
            }
        };
        for preset in [
            OrbitCamPreset::simple_mouse(),
            OrbitCamPreset::blender_like(),
            OrbitCamPreset::keyboard(),
            OrbitCamPreset::simple_mouse_keyboard(),
            OrbitCamPreset::blender_like_keyboard(),
            OrbitCamPreset::gamepad(),
        ] {
            let summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(preset));
            for row in summary.rows {
                assert_button_case(&row.label);
            }
        }
        for preset in [
            FreeCamPreset::keyboard_mouse(),
            FreeCamPreset::gamepad(),
            FreeCamPreset::gamepad_southpaw(),
        ] {
            let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(preset));
            for binding in summary.bindings {
                assert_button_case(&binding.label);
            }
        }
        let blender = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::blender_like(),
        ));
        let labels = summary_labels(&blender);
        assert!(labels.contains(&"Middle drag"));
        assert!(labels.contains(&"Shift middle drag"));
        assert_eq!(
            mouse_button_stem(MouseButton::Left, ModKeys::CONTROL),
            "Ctrl left"
        );
    }

    #[test]
    fn camera_summary_uses_plain_panel_labels() {
        let orbit = describe_orbit_camera_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::blender_like(),
        ));
        assert_eq!(orbit.camera_label, "Orbit camera");
        assert_eq!(orbit.mode_label, "Controls");
        assert_eq!(orbit.mode_value, "Blender");
        assert_eq!(
            orbit.slow_mode,
            SlowModeHint::ToggledBy("Alt S".to_string())
        );
        assert!(orbit.bindings.iter().any(|row| row.label == "Middle drag"));
        assert!(
            orbit
                .bindings
                .iter()
                .any(|row| row.label == "Two-finger drag")
        );

        let free = describe_free_cam_controls(&FreeCamInputMode::with_preset(
            FreeCamPreset::keyboard_mouse(),
        ));
        assert_eq!(free.camera_label, "Free camera");
        assert_eq!(free.mode_label, "Controls");
        assert_eq!(free.mode_value, "Mouse and keyboard");
        assert_eq!(free.slow_mode, SlowModeHint::ToggledBy("Alt S".to_string()));
        assert!(free.bindings.iter().any(|row| row.label == "Alt I"));

        let gamepad =
            describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()));
        assert!(gamepad.rows.iter().any(|row| row.label == "RB RS"));
    }

    #[test]
    fn camera_panel_summary_words_start_with_capitals() {
        let orbit_modes = [
            OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
            OrbitCamInputMode::Manual,
        ];
        for mode in orbit_modes {
            let summary = describe_orbit_cam_controls(&mode);
            assert!(summary.mode_value.starts_with(char::is_uppercase));
            for row in summary.rows {
                assert!(row.label.starts_with(char::is_uppercase), "{}", row.label);
            }
        }
        let free = describe_free_cam_controls(&FreeCamInputMode::with_preset(
            FreeCamPreset::keyboard_mouse(),
        ));
        assert!(free.mode_value.starts_with(char::is_uppercase));
        for binding in free.bindings {
            assert!(
                binding.label.starts_with(char::is_uppercase),
                "{}",
                binding.label
            );
        }
        assert_eq!(key_label(KeyCode::Space), "Space");
        assert_eq!(source_stem(InteractionSources::MOUSE), "Mouse binding");
        assert_eq!(source_stem(InteractionSources::PINCH), "Pinch");
    }

    #[test]
    fn forwarded_trackpad_summary_reports_wheel_modifiers() {
        let preset = crate::OrbitCamBlenderLikePreset::default()
            .line_scroll_input_gain(Some(crate::OrbitCamInputGain::uniform(20.0)));
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Preset(preset.into()));
        let rows: Vec<_> = summary
            .rows
            .iter()
            .filter(|row| row.camera_interaction_sources == InteractionSources::WHEEL)
            .collect();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().any(
            |row| row.kind == OrbitCamInteractionKind::Orbit && row.label == WHEEL_SOURCE_LABEL
        ));
        assert!(
            rows.iter()
                .any(|row| row.kind == OrbitCamInteractionKind::Pan
                    && row.label == with_mod_keys(ModKeys::SHIFT, WHEEL_SOURCE_LABEL.to_string()))
        );
        for row in rows
            .iter()
            .filter(|row| row.kind == OrbitCamInteractionKind::Zoom)
        {
            assert!(row.label.starts_with(&compact_mod_keys(ModKeys::CONTROL)));
        }
    }

    #[test]
    fn summary_labels_follow_input_mode_variant() -> Result<(), BindingsError> {
        let preset_summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::blender_like(),
        ));
        assert_eq!(preset_summary.mode_label, "Controls");
        assert_eq!(preset_summary.mode_value, "Blender");

        let tuned_bindings = OrbitCamBlenderLikePreset::default()
            .slow_scale(0.25)
            .build()?;
        let bindings_summary =
            describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(tuned_bindings));
        assert_eq!(bindings_summary.mode_label, "Input");
        assert_eq!(bindings_summary.mode_value, "Custom controls");

        let manual_summary = describe_orbit_cam_controls(&OrbitCamInputMode::Manual);
        assert_eq!(manual_summary.mode_label, "Input");
        assert_eq!(manual_summary.mode_value, "Manual input");

        Ok(())
    }

    #[test]
    fn blender_like_summary_reflects_preset_bindings() {
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::blender_like(),
        ));
        let labels = summary_labels(&summary);

        assert_eq!(summary.camera_label, "Orbit camera");
        assert_eq!(summary.mode_label, "Controls");
        assert_eq!(summary.mode_value, "Blender");
        assert!(
            labels
                .iter()
                .all(|label| !label.contains('+') || *label == "+"),
            "{labels:?}"
        );
        assert!(labels.contains(&"Middle drag"));
        assert!(labels.contains(&"Two-finger drag"));
        assert!(labels.contains(&"Shift middle drag"));
        assert!(labels.contains(&"Shift two-finger drag"));
        // Zoom sources split into one row per direction.
        assert!(labels.contains(&"Wheel ↑"));
        assert!(labels.contains(&"Wheel ↓"));
        assert!(labels.contains(&"Ctrl two-finger drag ↑"));
        assert!(labels.contains(&"Ctrl two-finger drag ↓"));
        assert!(labels.contains(&"Pinch out"));
        assert!(labels.contains(&"Pinch in"));
        assert_eq!(row_direction(&summary, "Wheel ↑"), Some(ZoomDirection::In));
        assert_eq!(row_direction(&summary, "Wheel ↓"), Some(ZoomDirection::Out));
        assert_eq!(
            row_direction(&summary, "Ctrl two-finger drag ↑"),
            Some(ZoomDirection::In)
        );
        assert_eq!(
            row_direction(&summary, "Ctrl two-finger drag ↓"),
            Some(ZoomDirection::Out)
        );
        assert_eq!(
            row_direction(&summary, "Pinch out"),
            Some(ZoomDirection::In)
        );
        assert_eq!(
            row_direction(&summary, "Pinch in"),
            Some(ZoomDirection::Out)
        );
        // Orbit and pan rows stay non-directional.
        assert_eq!(row_direction(&summary, "Middle drag"), None);
        assert_eq!(row_direction(&summary, "Two-finger drag"), None);
    }

    #[test]
    fn custom_bindings_override_preset_summary() -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Right))
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));
        let labels = summary_labels(&summary);

        assert_eq!(summary.mode_label, "Input");
        assert_eq!(summary.mode_value, "Custom controls");
        assert!(labels.contains(&"Right drag"));
        assert!(!labels.contains(&"Middle drag"));

        Ok(())
    }

    #[test]
    fn disabled_native_bindings_are_omitted_from_summary() -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .orbit(
                OrbitCamMouseDrag::new(MouseButton::Right).with_input_gain(InputGain::DISABLED.0),
            )
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));

        assert_eq!(summary.rows, []);

        Ok(())
    }

    #[test]
    fn disabled_adapter_bindings_are_omitted_from_summary() -> Result<(), BindingsError> {
        let disabled = InputGain::DISABLED.0;
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamTrackpadScroll::default().with_input_gain(disabled))
            .pan(OrbitCamTrackpadScroll::default().with_input_gain(disabled))
            .zoom(OrbitCamTrackpadScroll::default().with_input_gain(disabled))
            .zoom(OrbitCamMouseWheelZoom.with_input_gain(disabled))
            .zoom(OrbitCamPinchZoom.with_input_gain(disabled))
            .zoom(OrbitCamButtonDragZoom::new(MouseButton::Middle).with_input_gain(disabled))
            .touch_config(Some(
                OrbitCamTouchBinding::OneFingerOrbit
                    .with_input_gain(OrbitCamInputGain::uniform(disabled)),
            ))
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));

        assert_eq!(summary.rows, []);

        Ok(())
    }

    #[test]
    fn effective_slow_mode_keeps_tuned_blender_like_when_pinch_remains_enabled()
    -> Result<(), BindingsError> {
        let disabled = InputGain::DISABLED.0;
        let preset = OrbitCamPreset::from(
            OrbitCamBlenderLikePreset::default()
                .mouse_input_gain(OrbitCamInputGain::uniform(disabled))
                .smooth_scroll_input_gain(OrbitCamInputGain::uniform(disabled)),
        );
        let bindings = preset.to_bindings()?;
        let orbit_cam_input_mode = OrbitCamInputMode::with_preset(preset);
        let summary = describe_orbit_cam_controls(&orbit_cam_input_mode);
        let labels = summary_labels(&summary);

        assert!(effective_slow_mode(&bindings).is_some());
        assert!(labels.contains(&"Pinch out"));
        assert!(labels.contains(&"Pinch in"));
        assert!(!labels.contains(&"Middle drag"));
        assert!(!labels.contains(&"Two-finger drag"));
        Ok(())
    }

    #[test]
    fn effective_slow_mode_omits_custom_slow_mode_without_enabled_controls()
    -> Result<(), BindingsError> {
        let disabled = InputGain::DISABLED.0;
        let bindings = OrbitCamBindings::builder()
            .slow_mode(CameraSlowMode {
                toggle_key: KeyCode::KeyS,
                mod_keys:   ModKeys::ALT,
                scale:      CameraInputScalePolicy {
                    normal: InputGain::DEFAULT.0,
                    slow:   CUSTOM_SLOW_SCALE,
                },
            })
            .orbit(OrbitCamMouseDrag::new(MouseButton::Middle).with_input_gain(disabled))
            .build()?;
        assert!(effective_slow_mode(&bindings).is_none());
        let orbit_cam_input_mode = OrbitCamInputMode::Bindings(bindings);

        assert_eq!(describe_orbit_cam_controls(&orbit_cam_input_mode).rows, []);

        Ok(())
    }

    #[test]
    fn effective_free_slow_mode_omits_custom_slow_mode_without_enabled_controls()
    -> Result<(), BindingsError> {
        let disabled = InputGain::DISABLED.0;
        let bindings = FreeCamBindings::builder()
            .slow_mode(CameraSlowMode {
                toggle_key: KeyCode::KeyS,
                mod_keys:   ModKeys::ALT,
                scale:      CameraInputScalePolicy {
                    normal: InputGain::DEFAULT.0,
                    slow:   CUSTOM_SLOW_SCALE,
                },
            })
            .translate(FreeCamTranslateKeys::default().with_input_gain(disabled))
            .look(FreeCamMouseLook::button(MouseButton::Right).with_input_gain(disabled))
            .roll(
                FreeCamRollBinding::from(InputBinding::bidirectional_keys(
                    KeyCode::KeyQ,
                    KeyCode::KeyE,
                ))
                .with_input_gain(disabled),
            )
            .build()?;
        assert!(bindings.slow_mode().is_some());
        assert!(effective_free_slow_mode(&bindings).is_none());

        let summary = describe_free_cam_controls(&FreeCamInputMode::Bindings(bindings));

        assert_eq!(summary.slow_mode, SlowModeHint::NoDisplayableSlowToggle);
        assert!(
            summary
                .bindings
                .iter()
                .all(|binding| binding.kind != CameraControlBindingKind::Direct)
        );

        Ok(())
    }

    #[test]
    fn free_trackpad_look_appears_in_control_summary() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder()
            .look(FreeCamTrackpadLook::default())
            .build()?;
        let summary = describe_free_cam_controls(&FreeCamInputMode::Bindings(bindings));

        assert!(summary.bindings.iter().any(|binding| {
            binding.action == CameraControlAction::Look
                && binding.label == TRACKPAD_SOURCE_LABEL
                && binding.interaction_sources == InteractionSources::SMOOTH_SCROLL
        }));
        Ok(())
    }

    #[test]
    fn touch_summary_omits_disabled_actions_only() -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .touch_config(Some(
                OrbitCamTouchBinding::OneFingerOrbit.with_input_gain(
                    OrbitCamInputGain::new()
                        .orbit(InputGain::DISABLED.0)
                        .zoom(InputGain::DISABLED.0),
                ),
            ))
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));
        let labels = summary_labels(&summary);

        assert_eq!(summary.rows.len(), 1);
        assert_eq!(summary.rows[0].kind, OrbitCamInteractionKind::Pan);
        assert!(labels.contains(&TWO_FINGER_TOUCH_ROW_LABEL));

        Ok(())
    }

    #[test]
    fn keyboard_bindings_show_key_groups() -> Result<(), BindingsError> {
        let orbit_keys = InputBinding::cardinal_keys(
            KeyCode::ArrowUp,
            KeyCode::ArrowRight,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
        );
        let pan_keys =
            InputBinding::cardinal_keys(KeyCode::KeyW, KeyCode::KeyD, KeyCode::KeyS, KeyCode::KeyA);
        let zoom_keys = InputBinding::bidirectional_keys(KeyCode::Equal, KeyCode::Minus);
        let bindings = OrbitCamBindings::builder()
            .orbit(orbit_keys)
            .pan(pan_keys)
            .zoom(zoom_keys)
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));
        let labels = summary_labels(&summary);

        assert!(labels.contains(&"Arrow keys"));
        assert!(labels.contains(&"WASD"));
        // The bidirectional zoom key pair splits into one row per direction.
        assert!(labels.contains(&"+"));
        assert!(labels.contains(&"-"));
        assert!(!labels.contains(&"+ / -"));
        assert_eq!(row_direction(&summary, "+"), Some(ZoomDirection::In));
        assert_eq!(row_direction(&summary, "-"), Some(ZoomDirection::Out));

        Ok(())
    }

    #[test]
    fn gamepad_preset_summary_has_fast_and_slow_rows() {
        let summary =
            describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()));
        let labels = summary_labels(&summary);

        assert_eq!(summary.mode_value, "Gamepad");
        assert!(labels.contains(&"RS"));
        assert!(labels.contains(&"RB RS"));
        assert!(labels.contains(&"LS"));
        assert!(labels.contains(&"LB LS"));
        assert!(labels.contains(&"RT"));
        assert!(labels.contains(&"LT"));
        assert!(labels.contains(&"RB RT"));
        assert!(labels.contains(&"LB LT"));

        assert_eq!(row_speed(&summary, "RS"), Some(ControlSpeed::Normal));
        assert_eq!(row_speed(&summary, "RB RS"), Some(ControlSpeed::Slow));
        assert_eq!(row_speed(&summary, "LS"), Some(ControlSpeed::Normal));
        assert_eq!(row_speed(&summary, "LB LS"), Some(ControlSpeed::Slow));
        assert_eq!(row_speed(&summary, "RT"), Some(ControlSpeed::Normal));
        assert_eq!(row_speed(&summary, "LT"), Some(ControlSpeed::Normal));
        assert_eq!(row_speed(&summary, "RB RT"), Some(ControlSpeed::Slow));
        assert_eq!(row_speed(&summary, "LB LT"), Some(ControlSpeed::Slow));

        // The right triggers zoom in, the left triggers zoom out; the panel
        // highlights only the engaged direction from these tags.
        assert_eq!(row_direction(&summary, "RT"), Some(ZoomDirection::In));
        assert_eq!(row_direction(&summary, "LT"), Some(ZoomDirection::Out));
        assert_eq!(row_direction(&summary, "RB RT"), Some(ZoomDirection::In));
        assert_eq!(row_direction(&summary, "LB LT"), Some(ZoomDirection::Out));
        // Orbit and pan stick rows are not direction-specific.
        assert_eq!(row_direction(&summary, "RS"), None);
        assert_eq!(row_direction(&summary, "LS"), None);
    }

    #[test]
    fn manual_summary_wins_over_other_modes() {
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Manual);
        let labels = summary_labels(&summary);

        assert_eq!(summary.mode_label, "Input");
        assert_eq!(summary.mode_value, "Manual input");
        assert!(labels.contains(&"App controls"));
        assert!(summary.rows.iter().all(|row| {
            row.camera_interaction_sources
                .contains(InteractionSources::MANUAL)
        }));
    }

    #[test]
    fn gamepad_fallback_uses_plain_words() -> Result<(), BindingsError> {
        let bindings = OrbitCamBindings::builder()
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .build()?;
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings));
        assert!(
            summary
                .rows
                .iter()
                .any(|row| row.label == "Gamepad controls")
        );
        Ok(())
    }

    #[test]
    fn free_cam_summary_reports_normal_look_pitch_status() {
        let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(
            FreeCamPreset::keyboard_mouse(),
        ));
        let setting = single_setting_binding(&summary);

        assert_eq!(setting.action, CameraControlAction::Look);
        assert_eq!(setting.label, INVERT_Y_BINDING_LABEL);
        assert_eq!(setting.speed, ControlSpeed::Normal);
        assert_eq!(setting_value(setting), Some(INVERT_Y_STATUS_LABEL));
        assert_eq!(
            setting_activation(setting),
            Some(CameraControlActivation::Inactive)
        );
    }

    #[test]
    fn free_cam_summary_reports_inverted_look_pitch_status() {
        let preset =
            FreeCamKeyboardMousePreset::default().with_look_pitch(FreeCamLookPitch::Inverted);
        let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(preset));
        let setting = single_setting_binding(&summary);

        assert_eq!(setting.action, CameraControlAction::Look);
        assert_eq!(setting.label, INVERT_Y_BINDING_LABEL);
        assert_eq!(setting.speed, ControlSpeed::Normal);
        assert_eq!(setting_value(setting), Some(INVERT_Y_STATUS_LABEL));
        assert_eq!(
            setting_activation(setting),
            Some(CameraControlActivation::Active)
        );
    }

    #[test]
    fn free_cam_camera_summary_reports_disabled_roll() {
        let camera = FreeCam::horizon_locked();
        let summary = describe_free_cam_controls_for(
            &camera,
            &FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
        );

        assert!(free_summary_labels(&summary).contains(&ROLL_DISABLED_ROW_LABEL));
        assert!(!free_summary_labels(&summary).contains(&"Q/E"));
        assert_eq!(
            free_summary_row(&summary, ROLL_DISABLED_ROW_LABEL).map(|row| row.action),
            Some(CameraControlAction::Roll)
        );
        assert_eq!(
            free_summary_row(&summary, ROLL_DISABLED_ROW_LABEL).map(|row| row.interaction_sources),
            Some(InteractionSources::NONE)
        );
    }

    #[test]
    fn free_cam_summary_reports_keyboard_home_binding() {
        let preset = FreeCamKeyboardMousePreset::default().with_home(KeyCode::KeyH);
        let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(preset));
        let label = key_label(KeyCode::KeyH);

        assert!(free_summary_labels(&summary).contains(&label.as_str()));
        assert_eq!(
            free_summary_row(&summary, &label).map(|row| row.action),
            Some(CameraControlAction::Home)
        );
        assert_eq!(
            free_summary_row(&summary, &label).map(|row| row.interaction_sources),
            Some(InteractionSources::KEYBOARD)
        );
    }

    #[test]
    fn free_cam_gamepad_summary_decomposes_translate_and_splits_roll() {
        let summary =
            describe_free_cam_controls(&FreeCamInputMode::with_preset(FreeCamPreset::gamepad()));

        // Translate decomposes into a stick row, a boost-gated stick row, and one
        // row per vertical trigger.
        assert_eq!(
            free_summary_row(&summary, "LS").map(|row| row.action),
            Some(CameraControlAction::Translate)
        );
        assert!(
            free_summary_row(&summary, "LS")
                .is_some_and(|row| row.action_label == ActionColumnLabel::ActionName)
        );
        assert_eq!(
            free_summary_row(&summary, "L3 LS").and_then(replacement_label),
            Some(TRANSLATE_BOOST_ACTION_LABEL)
        );
        assert_eq!(
            free_summary_row(&summary, "RT").and_then(replacement_label),
            Some(TRANSLATE_UP_ACTION_LABEL)
        );
        assert_eq!(
            free_summary_row(&summary, "LT").and_then(replacement_label),
            Some(TRANSLATE_DOWN_ACTION_LABEL)
        );

        // Roll splits into one row per bumper direction.
        assert_eq!(
            free_summary_row(&summary, "RB").map(|row| row.action),
            Some(CameraControlAction::Roll)
        );
        assert_eq!(
            free_summary_row(&summary, "RB").and_then(replacement_label),
            Some(ROLL_RIGHT_ACTION_LABEL)
        );
        assert_eq!(
            free_summary_row(&summary, "LB").and_then(replacement_label),
            Some(ROLL_LEFT_ACTION_LABEL)
        );

        // Every decomposed row reports the gamepad as its source.
        for label in ["LS", "L3 LS", "RT", "LT", "RB", "LB"] {
            assert_eq!(
                free_summary_row(&summary, label).map(|row| row.interaction_sources),
                Some(InteractionSources::GAMEPAD),
                "row {label} should be gamepad-sourced",
            );
        }
    }

    #[test]
    fn free_cam_keyboard_roll_stays_single_row() -> Result<(), BindingsError> {
        let summary = describe_free_cam_controls(&FreeCamInputMode::with_preset(
            FreeCamPreset::keyboard_mouse(),
        ));
        let roll_rows = summary
            .bindings
            .iter()
            .filter(|binding| binding.action == CameraControlAction::Roll)
            .collect::<Vec<_>>();

        assert_eq!(roll_rows.len(), 1);
        assert_eq!(roll_rows[0].label, "Q / E");
        assert_eq!(roll_rows[0].action_label, ActionColumnLabel::ActionName);

        let arrow_bindings = FreeCamBindings::builder()
            .roll(FreeCamRollBinding::from(InputBinding::bidirectional_keys(
                KeyCode::ArrowUp,
                KeyCode::ArrowDown,
            )))
            .build()?;
        let arrow_summary = describe_free_cam_controls(&FreeCamInputMode::Bindings(arrow_bindings));
        let arrow_roll = arrow_summary
            .bindings
            .iter()
            .find(|binding| binding.action == CameraControlAction::Roll);
        assert_eq!(
            arrow_roll.map(|binding| binding.label.as_str()),
            Some("↑ / ↓")
        );
        Ok(())
    }

    #[test]
    fn free_cam_summary_omits_home_row_when_bindings_have_no_home() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder().build()?;
        let summary = describe_free_cam_controls(&FreeCamInputMode::Bindings(bindings));

        assert!(
            !summary
                .bindings
                .iter()
                .any(|binding| binding.action == CameraControlAction::Home)
        );

        Ok(())
    }

    #[test]
    fn orbit_camera_summary_reports_gamepad_home_binding() {
        let preset = OrbitCamGamepadPreset::default().home(GamepadButton::Select);
        let summary = describe_orbit_camera_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::from(preset),
        ));
        let label = gamepad_button_label(GamepadButton::Select);

        assert!(free_summary_labels(&summary).contains(&label.as_str()));
        assert_eq!(
            free_summary_row(&summary, &label).map(|row| row.action),
            Some(CameraControlAction::Home)
        );
        assert_eq!(
            free_summary_row(&summary, &label).map(|row| row.interaction_sources),
            Some(InteractionSources::GAMEPAD)
        );
    }

    #[test]
    fn orbit_camera_summary_omits_home_row_when_bindings_have_no_home() -> Result<(), BindingsError>
    {
        let bindings = OrbitCamBindings::builder()
            .orbit(OrbitCamMouseDrag::new(MouseButton::Right))
            .build()?;
        let summary = describe_orbit_camera_controls(&OrbitCamInputMode::Bindings(bindings));

        assert!(
            !summary
                .bindings
                .iter()
                .any(|binding| binding.action == CameraControlAction::Home)
        );

        Ok(())
    }

    fn summary_labels(summary: &OrbitCamControlSummary) -> Vec<&str> {
        summary.rows.iter().map(|row| row.label.as_str()).collect()
    }

    fn free_summary_labels(summary: &CameraControlSummary) -> Vec<&str> {
        summary
            .bindings
            .iter()
            .map(|binding| binding.label.as_str())
            .collect()
    }

    fn free_summary_row<'a>(
        summary: &'a CameraControlSummary,
        label: &str,
    ) -> Option<&'a CameraControlBinding> {
        summary
            .bindings
            .iter()
            .find(|binding| binding.label == label)
    }

    fn single_setting_binding(summary: &CameraControlSummary) -> &CameraControlBinding {
        let settings = summary
            .bindings
            .iter()
            .filter(|binding| matches!(binding.kind, CameraControlBindingKind::Setting { .. }))
            .collect::<Vec<_>>();
        assert_eq!(settings.len(), 1);
        settings[0]
    }

    fn setting_value(binding: &CameraControlBinding) -> Option<&str> {
        match &binding.kind {
            CameraControlBindingKind::Setting { value, .. } => Some(value.as_str()),
            CameraControlBindingKind::Direct => None,
        }
    }

    fn replacement_label(binding: &CameraControlBinding) -> Option<&str> {
        match &binding.action_label {
            ActionColumnLabel::ActionName => None,
            ActionColumnLabel::Replacement(label) => Some(label),
        }
    }

    fn setting_activation(binding: &CameraControlBinding) -> Option<CameraControlActivation> {
        match &binding.kind {
            CameraControlBindingKind::Setting { activation, .. } => Some(*activation),
            CameraControlBindingKind::Direct => None,
        }
    }

    fn row_speed(summary: &OrbitCamControlSummary, label: &str) -> Option<ControlSpeed> {
        summary
            .rows
            .iter()
            .find(|row| row.label == label)
            .map(|row| row.speed)
    }

    fn row_direction(summary: &OrbitCamControlSummary, label: &str) -> Option<ZoomDirection> {
        summary
            .rows
            .iter()
            .find(|row| row.label == label)
            .and_then(|row| match row.zoom_direction {
                ZoomRowFilter::Only(direction) => Some(direction),
                ZoomRowFilter::Unrestricted => None,
            })
    }
}

#[cfg(test)]
mod drawn_mouse_label_tests {
    use std::sync::mpsc;

    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::RenderPlugin;
    use bevy::render::render_resource::PollType;
    use bevy::render::render_resource::TextureFormat;
    use bevy::render::renderer::RenderDevice;
    use bevy::render::view::screenshot::Screenshot;
    use bevy::render::view::screenshot::ScreenshotCaptured;
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;
    use hana_diegetic::Anchor;
    use hana_diegetic::CascadeEntityCommandsExt as _;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticUiPlugin;
    use hana_diegetic::DisplayCompensationReadiness;
    use hana_diegetic::El;
    use hana_diegetic::LayoutBuilder;
    use hana_diegetic::Lighting;
    use hana_diegetic::Text;
    use hana_diegetic::TextStyle;
    use hana_diegetic::Unit;

    use super::OrbitCamInputMode;
    use super::describe_orbit_cam_controls;
    use crate::orbit_cam::OrbitCamPreset;

    const WIDTH: u32 = 512;
    const HEIGHT: u32 = 128;

    fn drawn_label(label: &str) -> Result<Vec<u8>, String> {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins(DiegeticUiPlugin);
        app.world_mut().insert_resource(Lighting::Unlit);
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    WIDTH,
                    HEIGHT,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Image(target.clone().into()),
            Msaa::Off,
            Transform::from_xyz(0.0, 0.0, 0.9).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let mut layout =
            LayoutBuilder::with_root(El::column().size(1.2, 0.12).background(Color::BLACK));
        layout.text(Text::new(
            label,
            TextStyle::new(0.014).with_color(Color::WHITE),
        ));
        let panel = DiegeticPanel::world()
            .font_unit(Unit::Meters)
            .size(1.2, 0.12)
            .anchor(Anchor::Center)
            .with_tree(layout.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn((panel, Transform::IDENTITY)).id();
        app.world_mut()
            .commands()
            .entity(panel)
            .override_lighting(Lighting::Unlit);
        app.finish();
        app.cleanup();
        for _ in 0..50 {
            if *app.world().resource::<DisplayCompensationReadiness>()
                == DisplayCompensationReadiness::Compensated
            {
                break;
            }
            app.update();
        }
        for _ in 0..4 {
            app.update();
        }
        let (sender, receiver) = mpsc::channel();
        app.world_mut().spawn(Screenshot::image(target)).observe(
            move |event: On<ScreenshotCaptured>| {
                let _ = sender.send(event.image.data.clone().ok_or("screenshot has no pixels"));
            },
        );
        // Bevy finishes the readback on its async compute pool, which shader
        // compilation can hold for many frames, so this waits on the capture itself.
        loop {
            app.update();
            app.world()
                .resource::<RenderDevice>()
                .wgpu_device()
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout:          None,
                })
                .map_err(|error| format!("GPU readback failed: {error}"))?;
            if let Ok(pixels) = receiver.try_recv() {
                return pixels.map_err(str::to_string);
            }
        }
    }

    #[test]
    fn blender_pan_label_draws_lowercase_middle_after_shift() -> Result<(), String> {
        let summary = describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(
            OrbitCamPreset::blender_like(),
        ));
        let label = summary
            .rows
            .iter()
            .find(|row| row.label.starts_with("Shift ") && row.label.ends_with("iddle drag"))
            .ok_or("Blender preset has no modified middle drag row")?;
        let actual = drawn_label(&label.label)?;
        let expected = drawn_label("Shift middle drag")?;
        let capitalized = drawn_label("Shift Middle drag")?;
        assert!(actual.as_chunks::<4>().0.iter().any(|pixel| pixel[0] > 80));
        assert!(
            expected != capitalized,
            "font did not distinguish letter case"
        );
        assert!(
            actual == expected,
            "Blender pan label drew the wrong letter case"
        );
        Ok(())
    }
}
