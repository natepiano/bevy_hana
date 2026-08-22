//! `FreeCam` setting controls owned by the camera control panel.

use bevy::prelude::App;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamLookPitch;
use hana_lagrange::FreeCamPreset;
use hana_lagrange::ResolvedCameraInputRoute;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;

use super::CameraGuidancePanel;
use crate::ensure_plugin;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FreeCamLookPitchPreference(FreeCamLookPitch);

impl FreeCamLookPitchPreference {
    pub(super) const fn look_pitch(self) -> FreeCamLookPitch { self.0 }

    const fn with_look_pitch(mut self, look_pitch: FreeCamLookPitch) -> Self {
        self.0 = look_pitch;
        self
    }
}

impl Default for FreeCamLookPitchPreference {
    fn default() -> Self { Self(FreeCamLookPitch::Inverted) }
}

command! {
    action:      ToggleFreeCamLookPitch,
    event:       ToggleFreeCamLookPitchEvent,
    id:          "fairy_dust::toggle_free_cam_look_pitch",
    title:       "Toggle Free Camera Look Pitch",
    description: "Swap the free camera between inverted and standard mouse pitch.",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LookPitchUpdate {
    Applied,
    Unsupported,
}

pub(super) fn install(app: &mut App) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.init_resource::<FreeCamLookPitchPreference>();
    app.add_observer(toggle_look_pitch);
}

pub(super) const fn preset_with_look_pitch(
    preset: FreeCamPreset,
    look_pitch: FreeCamLookPitch,
) -> FreeCamPreset {
    match preset {
        FreeCamPreset::KeyboardMouse(preset) => {
            FreeCamPreset::KeyboardMouse(preset.with_look_pitch(look_pitch))
        },
        _ => preset,
    }
}

fn toggle_look_pitch(
    _: On<ToggleFreeCamLookPitchEvent>,
    route: Res<ResolvedCameraInputRoute>,
    panels: Query<&CameraGuidancePanel>,
    mut preference: ResMut<FreeCamLookPitchPreference>,
    mut modes: Query<&mut FreeCamInputMode>,
) {
    let Some(camera) = active_camera(&route, &panels) else {
        return;
    };
    let Ok(mut mode) = modes.get_mut(camera) else {
        return;
    };
    let Some(look_pitch) = mode_look_pitch(&mode) else {
        return;
    };

    let next = look_pitch.toggled();
    if set_look_pitch(&mut mode, next) == LookPitchUpdate::Applied {
        *preference = preference.with_look_pitch(next);
    }
}

fn active_camera(
    route: &ResolvedCameraInputRoute,
    panels: &Query<&CameraGuidancePanel>,
) -> Option<Entity> {
    route
        .routed_camera()
        .or_else(|| panels.single().ok()?.bound_camera)
}

const fn mode_look_pitch(mode: &FreeCamInputMode) -> Option<FreeCamLookPitch> {
    match mode {
        FreeCamInputMode::Preset(FreeCamPreset::KeyboardMouse(preset)) => Some(preset.look_pitch()),
        FreeCamInputMode::Bindings(bindings) => Some(bindings.look_pitch()),
        _ => None,
    }
}

fn set_look_pitch(mode: &mut FreeCamInputMode, look_pitch: FreeCamLookPitch) -> LookPitchUpdate {
    match mode {
        FreeCamInputMode::Preset(FreeCamPreset::KeyboardMouse(preset)) => {
            *preset = (*preset).with_look_pitch(look_pitch);
            LookPitchUpdate::Applied
        },
        FreeCamInputMode::Bindings(bindings) => {
            *bindings = bindings.clone().with_look_pitch(look_pitch);
            LookPitchUpdate::Applied
        },
        _ => LookPitchUpdate::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use hana_lagrange::BindingsError;
    use hana_lagrange::FreeCamBindings;
    use hana_lagrange::FreeCamPreset;

    use super::*;

    #[test]
    fn preference_defaults_to_inverted() {
        assert_eq!(
            FreeCamLookPitchPreference::default().look_pitch(),
            FreeCamLookPitch::Inverted
        );
    }

    #[test]
    fn preset_look_pitch_can_be_updated() {
        let mut mode = FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse());

        assert_eq!(
            set_look_pitch(&mut mode, FreeCamLookPitch::Inverted),
            LookPitchUpdate::Applied
        );
        assert_eq!(mode_look_pitch(&mode), Some(FreeCamLookPitch::Inverted));
    }

    #[test]
    fn binding_look_pitch_can_be_updated() -> Result<(), BindingsError> {
        let mut mode = FreeCamInputMode::Bindings(FreeCamBindings::builder().build()?);

        assert_eq!(
            set_look_pitch(&mut mode, FreeCamLookPitch::Inverted),
            LookPitchUpdate::Applied
        );
        assert_eq!(mode_look_pitch(&mode), Some(FreeCamLookPitch::Inverted));
        Ok(())
    }

    #[test]
    fn manual_mode_does_not_claim_a_setting() {
        let mut mode = FreeCamInputMode::Manual;

        assert_eq!(mode_look_pitch(&mode), None);
        assert_eq!(
            set_look_pitch(&mut mode, FreeCamLookPitch::Inverted),
            LookPitchUpdate::Unsupported
        );
    }
}
