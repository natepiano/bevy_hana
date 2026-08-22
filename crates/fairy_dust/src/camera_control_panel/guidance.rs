//! Public guidance configuration types: `CameraGuidance`, `CameraGuidanceRow`.

use bevy::prelude::Component;
use hana_diegetic::Anchor;
use hana_lagrange::CameraControlAction;
use hana_lagrange::CameraControlBinding;
use hana_lagrange::CameraControlBindingKind;
use hana_lagrange::CameraControlSummary;
use hana_lagrange::ControlSpeed;
use hana_lagrange::FreeCamControlDirection;
use hana_lagrange::FreeCamInteractionKind;
use hana_lagrange::InteractionSources;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::ZoomDirection;
use hana_lagrange::describe_controls;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CameraGuidanceContent {
    Auto,
    Rows(Vec<CameraGuidanceRow>),
}

/// Data-driven camera control metadata shown by [`SprinkleBuilder`](crate::SprinkleBuilder)
/// examples.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct CameraGuidance {
    pub(super) anchor:     Anchor,
    pub(super) title:      Option<String>,
    pub(super) mode_label: Option<String>,
    pub(super) mode_value: Option<String>,
    pub(super) settings:   Vec<CameraControlBinding>,
    pub(super) content:    CameraGuidanceContent,
}

impl CameraGuidance {
    /// Builds guidance rows from the camera's actual input-mode components.
    #[must_use]
    pub const fn auto() -> Self {
        Self {
            anchor:     Anchor::BottomRight,
            title:      None,
            mode_label: None,
            mode_value: None,
            settings:   Vec::new(),
            content:    CameraGuidanceContent::Auto,
        }
    }

    /// Builds guidance rows for a built-in orbit-camera preset.
    #[must_use]
    pub fn for_preset(preset: impl Into<OrbitCamPreset>) -> Self {
        Self::from_summary(describe_controls(&OrbitCamInputMode::with_preset(preset)))
    }

    /// Builds custom camera guidance rows.
    #[must_use]
    pub fn custom(rows: impl IntoIterator<Item = CameraGuidanceRow>) -> Self {
        Self {
            anchor:     Anchor::BottomRight,
            title:      None,
            mode_label: None,
            mode_value: None,
            settings:   Vec::new(),
            content:    CameraGuidanceContent::Rows(rows.into_iter().collect()),
        }
    }

    /// Sets the panel screen anchor.
    #[must_use]
    pub const fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Replaces the panel title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Returns explicitly configured rows.
    ///
    /// Auto guidance is resolved when the panel binds to a camera.
    #[must_use]
    pub fn rows(&self) -> &[CameraGuidanceRow] {
        match &self.content {
            CameraGuidanceContent::Auto => &[],
            CameraGuidanceContent::Rows(rows) => rows,
        }
    }

    fn from_summary(summary: CameraControlSummary) -> Self {
        let (settings, rows) = split_summary_bindings(summary.bindings);
        Self {
            anchor: Anchor::BottomRight,
            title: Some(summary.camera_label),
            mode_label: Some(summary.mode_label),
            mode_value: Some(summary.mode_value),
            settings,
            content: CameraGuidanceContent::Rows(rows),
        }
    }
}

impl Default for CameraGuidance {
    fn default() -> Self { Self::auto() }
}

/// Semantic action represented by a camera guidance row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraGuidanceAction {
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
    /// Fallback for custom or future camera actions the panel cannot name yet.
    Other,
}

impl CameraGuidanceAction {
    pub(super) const fn from_orbit_interaction(kind: OrbitCamInteractionKind) -> Self {
        match kind {
            OrbitCamInteractionKind::Orbit => Self::Orbit,
            OrbitCamInteractionKind::Pan => Self::Pan,
            OrbitCamInteractionKind::Zoom => Self::Zoom,
            _ => Self::Other,
        }
    }

    pub(super) const fn from_free_interaction(kind: FreeCamInteractionKind) -> Self {
        match kind {
            FreeCamInteractionKind::Translate => Self::Translate,
            FreeCamInteractionKind::Look => Self::Look,
            FreeCamInteractionKind::Roll => Self::Roll,
            _ => Self::Other,
        }
    }

    #[cfg(test)]
    pub(super) const fn from_orbit_zoom_direction(direction: Option<ZoomDirection>) -> Self {
        match direction {
            Some(ZoomDirection::In) => Self::ZoomIn,
            Some(ZoomDirection::Out) => Self::ZoomOut,
            None => Self::Zoom,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Orbit => "Orbit",
            Self::Pan => "Pan",
            Self::Zoom => "Zoom",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::Look => "Look",
            Self::Translate => "Translate",
            Self::Roll => "Roll",
            Self::Home => "Home",
            Self::Other => "Action",
        }
    }

    pub(super) const fn zoom_direction(self) -> Option<ZoomDirection> {
        match self {
            Self::ZoomIn => Some(ZoomDirection::In),
            Self::ZoomOut => Some(ZoomDirection::Out),
            _ => None,
        }
    }
}

impl From<CameraControlAction> for CameraGuidanceAction {
    fn from(action: CameraControlAction) -> Self {
        match action {
            CameraControlAction::Orbit => Self::Orbit,
            CameraControlAction::Pan => Self::Pan,
            CameraControlAction::Zoom => Self::Zoom,
            CameraControlAction::ZoomIn => Self::ZoomIn,
            CameraControlAction::ZoomOut => Self::ZoomOut,
            CameraControlAction::Look => Self::Look,
            CameraControlAction::Translate => Self::Translate,
            CameraControlAction::Roll => Self::Roll,
            CameraControlAction::Home => Self::Home,
            _ => Self::Other,
        }
    }
}

/// A single camera guidance row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraGuidanceRow {
    action:                     CameraGuidanceAction,
    label:                      String,
    camera_interaction_sources: InteractionSources,
    speed:                      ControlSpeed,
    action_label:               Option<String>,
    direction:                  Option<FreeCamControlDirection>,
}

impl CameraGuidanceRow {
    /// Creates a row for a camera action.
    #[must_use]
    pub fn new(action: CameraGuidanceAction, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            camera_interaction_sources: InteractionSources::NONE,
            speed: ControlSpeed::Normal,
            action_label: None,
            direction: None,
        }
    }

    /// Sets the binding speed variant.
    #[must_use]
    pub const fn with_speed(mut self, speed: ControlSpeed) -> Self {
        self.speed = speed;
        self
    }

    /// Sets the right-column label shown in place of the action's default name.
    #[must_use]
    pub fn with_action_label(mut self, action_label: impl Into<String>) -> Self {
        self.action_label = Some(action_label.into());
        self
    }

    /// Tags this row with the decomposed `FreeCam` direction it drives, so the
    /// panel lights it only while that direction is engaged.
    #[must_use]
    pub const fn with_direction(mut self, direction: FreeCamControlDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// Highlights this row only when the active sources intersect `sources`.
    #[must_use]
    pub const fn with_camera_interaction_sources(
        mut self,
        camera_interaction_sources: InteractionSources,
    ) -> Self {
        self.camera_interaction_sources = camera_interaction_sources;
        self
    }

    /// Returns the camera action represented by this row.
    #[must_use]
    pub const fn action(&self) -> CameraGuidanceAction { self.action }

    /// Returns this row's camera-interaction source metadata.
    #[must_use]
    pub const fn camera_interaction_sources(&self) -> InteractionSources {
        self.camera_interaction_sources
    }

    /// Returns the display label.
    #[must_use]
    pub fn label(&self) -> &str { &self.label }

    /// Returns the right-column label override, when set.
    #[must_use]
    pub fn action_label(&self) -> Option<&str> { self.action_label.as_deref() }

    /// Returns the decomposed direction this row drives, when set.
    #[must_use]
    pub const fn direction(&self) -> Option<FreeCamControlDirection> { self.direction }

    /// Returns the binding speed variant.
    #[must_use]
    pub const fn speed(&self) -> ControlSpeed { self.speed }
}

impl CameraGuidanceRow {
    fn from_direct(binding: CameraControlBinding) -> Self {
        let mut row = Self::new(CameraGuidanceAction::from(binding.action), binding.label)
            .with_camera_interaction_sources(binding.interaction_sources)
            .with_speed(binding.speed);
        row.action_label = binding.action_label;
        row.direction = binding.direction;
        row
    }
}

pub(super) fn split_summary_bindings(
    bindings: Vec<CameraControlBinding>,
) -> (Vec<CameraControlBinding>, Vec<CameraGuidanceRow>) {
    let mut settings = Vec::new();
    let mut rows = Vec::new();
    for binding in bindings {
        if matches!(&binding.kind, CameraControlBindingKind::Direct) {
            rows.push(CameraGuidanceRow::from_direct(binding));
        } else {
            settings.push(binding);
        }
    }
    (settings, rows)
}
