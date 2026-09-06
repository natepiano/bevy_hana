//! Snapshot resolution: derives a `CameraGuidanceSnapshot` (panel-ready rows
//! and labels) from the `CameraGuidance` config plus the camera's input-mode
//! components, and supplies the label helpers used by the layout pass.

use bevy::prelude::Component;
use bevy::prelude::Name;
use hana_lagrange::CameraControlBinding;
use hana_lagrange::CameraControlSummary;
use hana_lagrange::ControlSpeed;
use hana_lagrange::FreeCam;
use hana_lagrange::FreeCamActiveDirections;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::InteractionSources;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::ZoomDirection;
use hana_lagrange::describe_controls;
use hana_lagrange::describe_controls_for;

use super::guidance;
use super::guidance::CameraGuidance;
use super::guidance::CameraGuidanceContent;
use super::guidance::CameraGuidanceRow;

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(super) struct CameraGuidanceSnapshot {
    pub(super) camera_label:            String,
    pub(super) mode_label:              String,
    pub(super) mode_value:              String,
    pub(super) slow_mode_binding_label: Option<String>,
    pub(super) settings:                Vec<CameraControlBinding>,
    pub(super) rows:                    Vec<CameraGuidanceRow>,
}

pub(super) fn resolve_guidance_snapshot(
    name: Option<&Name>,
    guidance: Option<&CameraGuidance>,
    orbit_mode: Option<&OrbitCamInputMode>,
    free_cam: Option<&FreeCam>,
    free_mode: Option<&FreeCamInputMode>,
) -> CameraGuidanceSnapshot {
    let name_label = name.map(|n| n.as_str().to_string());
    let title_label = guidance.and_then(|g| g.title.clone());
    let explicit = guidance.and_then(|g| match &g.content {
        CameraGuidanceContent::Auto => None,
        CameraGuidanceContent::Rows(rows) => Some((g, rows.clone())),
    });

    if let Some((guidance, rows)) = explicit {
        let summary = resolve_camera_summary(orbit_mode, free_cam, free_mode);
        let (summary_settings, _) = guidance::split_summary_bindings(summary.bindings.clone());
        let settings = if guidance.settings.is_empty() {
            summary_settings
        } else {
            guidance.settings.clone()
        };
        return CameraGuidanceSnapshot {
            camera_label: name_label
                .or(title_label)
                .unwrap_or_else(|| summary.camera_label.clone()),
            mode_label: guidance
                .mode_label
                .clone()
                .unwrap_or_else(|| summary.mode_label.clone()),
            mode_value: guidance
                .mode_value
                .clone()
                .unwrap_or_else(|| summary.mode_value.clone()),
            slow_mode_binding_label: summary.slow_mode_binding_label,
            settings,
            rows,
        };
    }

    let summary = resolve_camera_summary(orbit_mode, free_cam, free_mode);
    let (settings, rows) = guidance::split_summary_bindings(summary.bindings);
    CameraGuidanceSnapshot {
        camera_label: name_label.or(title_label).unwrap_or(summary.camera_label),
        mode_label: summary.mode_label,
        mode_value: summary.mode_value,
        slow_mode_binding_label: summary.slow_mode_binding_label,
        settings,
        rows,
    }
}

fn resolve_camera_summary(
    orbit_mode: Option<&OrbitCamInputMode>,
    free_cam: Option<&FreeCam>,
    free_mode: Option<&FreeCamInputMode>,
) -> CameraControlSummary {
    if let Some(mode) = free_mode {
        if let Some(camera) = free_cam {
            return describe_controls_for(camera, mode);
        }
        return describe_controls(mode);
    }
    match orbit_mode {
        Some(mode) => describe_controls(mode),
        None => describe_controls(&OrbitCamInputMode::default()),
    }
}

pub(super) fn row_active(
    row: &CameraGuidanceRow,
    sources: InteractionSources,
    live_zoom_direction: Option<ZoomDirection>,
    live_free_directions: FreeCamActiveDirections,
) -> bool {
    if sources.is_empty() || !sources.intersects(row.camera_interaction_sources()) {
        return false;
    }
    // A decomposed `FreeCam` row lights only while its own direction is engaged,
    // so pressing one affordance never lights the whole action's rows.
    if let Some(direction) = row.direction() {
        return live_free_directions.contains(direction);
    }
    match row.action().zoom_direction() {
        // Non-directional action rows match on source alone.
        None => true,
        // A directional zoom row lights only when the live zoom matches it. Until
        // a direction has been reported (no zoom engaged yet) fall back to
        // source-only so the row can still light rather than going dark.
        Some(direction) => live_zoom_direction.is_none_or(|live| live == direction),
    }
}

pub(super) const fn speed_label(speed: ControlSpeed) -> &'static str {
    match speed {
        ControlSpeed::Normal => "Normal",
        ControlSpeed::Slow => "Slow",
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;
    use bevy::prelude::MouseButton;
    use bevy_enhanced_input::prelude::ModKeys;
    use hana_lagrange::BindingsError;
    use hana_lagrange::CameraControlActivation;
    use hana_lagrange::CameraControlBindingKind;
    use hana_lagrange::CameraInputScalePolicy;
    use hana_lagrange::CameraSlowMode;
    use hana_lagrange::FreeCamControlDirection;
    use hana_lagrange::FreeCamKeyboardMousePreset;
    use hana_lagrange::FreeCamLookPitch;
    use hana_lagrange::FreeCamPreset;
    use hana_lagrange::InputGain;
    use hana_lagrange::OrbitCamBindings;
    use hana_lagrange::OrbitCamBlenderLikePreset;
    use hana_lagrange::OrbitCamInputGain;
    use hana_lagrange::OrbitCamMouseDrag;
    use hana_lagrange::OrbitCamPreset;

    use super::*;
    use crate::camera_control_panel::guidance::CameraGuidanceAction;

    const CUSTOM_SLOW_SCALE: f32 = 0.25;

    fn zoom_row(direction: Option<ZoomDirection>) -> CameraGuidanceRow {
        let action = CameraGuidanceAction::from_orbit_zoom_direction(direction);
        CameraGuidanceRow::new(action, "rt")
            .with_camera_interaction_sources(InteractionSources::GAMEPAD)
    }

    #[test]
    fn directional_zoom_row_lights_only_on_matching_live_direction() {
        let row = zoom_row(Some(ZoomDirection::In));
        assert!(row_active(
            &row,
            InteractionSources::GAMEPAD,
            Some(ZoomDirection::In),
            FreeCamActiveDirections::NONE,
        ));
        assert!(!row_active(
            &row,
            InteractionSources::GAMEPAD,
            Some(ZoomDirection::Out),
            FreeCamActiveDirections::NONE,
        ));
    }

    #[test]
    fn directional_zoom_row_lights_when_live_direction_unknown() {
        let row = zoom_row(Some(ZoomDirection::Out));
        assert!(row_active(
            &row,
            InteractionSources::GAMEPAD,
            None,
            FreeCamActiveDirections::NONE,
        ));
    }

    #[test]
    fn non_directional_row_lights_regardless_of_live_direction() {
        let row = zoom_row(None);
        assert!(row_active(
            &row,
            InteractionSources::GAMEPAD,
            Some(ZoomDirection::In),
            FreeCamActiveDirections::NONE,
        ));
        assert!(row_active(
            &row,
            InteractionSources::GAMEPAD,
            Some(ZoomDirection::Out),
            FreeCamActiveDirections::NONE,
        ));
    }

    #[test]
    fn row_inactive_without_source_overlap() {
        let row = zoom_row(Some(ZoomDirection::In));
        assert!(!row_active(
            &row,
            InteractionSources::WHEEL,
            Some(ZoomDirection::In),
            FreeCamActiveDirections::NONE,
        ));
        assert!(!row_active(
            &row,
            InteractionSources::NONE,
            Some(ZoomDirection::In),
            FreeCamActiveDirections::NONE,
        ));
    }

    #[test]
    fn free_direction_row_lights_only_when_its_direction_is_engaged() {
        let row = CameraGuidanceRow::new(CameraGuidanceAction::Translate, "rt")
            .with_camera_interaction_sources(InteractionSources::GAMEPAD)
            .with_direction(FreeCamControlDirection::Up);
        let up = FreeCamActiveDirections::NONE.with(FreeCamControlDirection::Up);
        let down = FreeCamActiveDirections::NONE.with(FreeCamControlDirection::Down);

        assert!(row_active(&row, InteractionSources::GAMEPAD, None, up));
        assert!(!row_active(&row, InteractionSources::GAMEPAD, None, down));
        assert!(!row_active(
            &row,
            InteractionSources::GAMEPAD,
            None,
            FreeCamActiveDirections::NONE,
        ));
    }

    #[test]
    fn blender_like_snapshot_includes_slow_mode_binding_hint() {
        let snapshot = resolve_guidance_snapshot(
            None,
            None,
            Some(&OrbitCamInputMode::with_preset(
                OrbitCamPreset::blender_like(),
            )),
            None,
            None,
        );

        assert_eq!(snapshot.slow_mode_binding_label.as_deref(), Some("alt-s"));
    }

    #[test]
    fn tuned_blender_like_snapshot_keeps_preset_label_and_slow_mode_hint() {
        let disabled = InputGain::DISABLED.0;
        let preset = OrbitCamBlenderLikePreset::default()
            .mouse_input_gain(OrbitCamInputGain::uniform(disabled))
            .smooth_scroll_input_gain(OrbitCamInputGain::uniform(disabled));
        let snapshot = resolve_guidance_snapshot(
            None,
            None,
            Some(&OrbitCamInputMode::with_preset(preset)),
            None,
            None,
        );
        let labels = row_labels(&snapshot);

        assert_eq!(snapshot.mode_label, "Preset");
        assert_eq!(snapshot.mode_value, "BlenderLike");
        assert_eq!(snapshot.slow_mode_binding_label.as_deref(), Some("alt-s"));
        assert!(labels.contains(&"pinch out"));
        assert!(labels.contains(&"pinch in"));
        assert!(!labels.contains(&"mmb drag"));
        assert!(!labels.contains(&"smooth-scroll"));
    }

    #[test]
    fn custom_slow_mode_snapshot_omits_hint_when_all_controls_are_disabled()
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
        let snapshot = resolve_guidance_snapshot(
            None,
            None,
            Some(&OrbitCamInputMode::Bindings(bindings)),
            None,
            None,
        );

        assert_eq!(snapshot.slow_mode_binding_label, None);
        assert!(snapshot.rows.is_empty());
        Ok(())
    }

    #[test]
    fn simple_mouse_snapshot_omits_slow_mode_hint() {
        let snapshot = resolve_guidance_snapshot(
            None,
            None,
            Some(&OrbitCamInputMode::with_preset(
                OrbitCamPreset::simple_mouse(),
            )),
            None,
            None,
        );

        assert_eq!(snapshot.slow_mode_binding_label, None);
    }

    #[test]
    fn free_cam_snapshot_uses_free_actions() {
        let preset = FreeCamKeyboardMousePreset::default().with_home(KeyCode::KeyH);
        let mode = FreeCamInputMode::with_preset(preset);
        let snapshot = resolve_guidance_snapshot(None, None, None, None, Some(&mode));

        assert_eq!(
            row_actions(&snapshot),
            vec![
                CameraGuidanceAction::Look,
                CameraGuidanceAction::Translate,
                CameraGuidanceAction::Roll,
                CameraGuidanceAction::Home,
            ]
        );
    }

    #[test]
    fn free_cam_snapshot_includes_invert_y_status() {
        let mode = FreeCamInputMode::with_preset(
            FreeCamKeyboardMousePreset::default().with_look_pitch(FreeCamLookPitch::Inverted),
        );
        let snapshot = resolve_guidance_snapshot(None, None, None, None, Some(&mode));
        let setting = single_setting_binding(&snapshot);

        assert_eq!(
            CameraGuidanceAction::from(setting.action),
            CameraGuidanceAction::Look
        );
        assert_eq!(setting.label, "alt-i");
        assert_eq!(setting_value(setting), Some("Invert Y"));
        assert_eq!(setting.speed, ControlSpeed::Normal);
        assert_eq!(
            setting_activation(setting),
            Some(CameraControlActivation::Active)
        );
    }

    #[test]
    fn horizon_locked_free_cam_snapshot_reports_disabled_roll() {
        let free_cam = FreeCam::horizon_locked();
        let mode = FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse());
        let snapshot = resolve_guidance_snapshot(None, None, None, Some(&free_cam), Some(&mode));
        let labels = row_labels(&snapshot);

        assert!(labels.contains(&"Roll disabled"));
        assert!(!labels.contains(&"q/e"));
    }

    fn row_labels(snapshot: &CameraGuidanceSnapshot) -> Vec<&str> {
        snapshot.rows.iter().map(CameraGuidanceRow::label).collect()
    }

    fn row_actions(snapshot: &CameraGuidanceSnapshot) -> Vec<CameraGuidanceAction> {
        snapshot
            .rows
            .iter()
            .map(CameraGuidanceRow::action)
            .collect()
    }

    fn single_setting_binding(snapshot: &CameraGuidanceSnapshot) -> &CameraControlBinding {
        assert_eq!(snapshot.settings.len(), 1);
        &snapshot.settings[0]
    }

    fn setting_value(binding: &CameraControlBinding) -> Option<&str> {
        match &binding.kind {
            CameraControlBindingKind::Setting { value, .. } => Some(value.as_str()),
            CameraControlBindingKind::Direct => None,
        }
    }

    fn setting_activation(binding: &CameraControlBinding) -> Option<CameraControlActivation> {
        match &binding.kind {
            CameraControlBindingKind::Setting { activation, .. } => Some(*activation),
            CameraControlBindingKind::Direct => None,
        }
    }
}
