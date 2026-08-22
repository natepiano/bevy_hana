//! Compact top-left title bar for example-level controls.

use std::collections::HashMap;

use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::error;
use bevy::prelude::warn;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::ChildLayoutState;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;

use super::constants::CONTROL_ACTIVE_COLOR;
use super::constants::CONTROL_DISABLED_COLOR;
use super::constants::CONTROL_INACTIVE_COLOR;
use super::constants::DIVIDER_COLOR;
use super::constants::HELP_CONTROL;
use super::constants::HOME_TITLE_BAR_HIGHLIGHT_HOLD;
use super::constants::SEPARATOR_HEIGHT;
use super::constants::SEPARATOR_WIDTH;
use super::constants::TITLE_BAR_CHILD_GAP;
use super::constants::TITLE_BAR_DEFAULT_TITLE;
use super::constants::TITLE_BAR_SEGMENT_GAP;
use super::screen_panel_frame;
use crate::camera_home::CameraHomeConfig;
use crate::camera_home::HomeTitleBarControl;
use crate::constants::HOME_CONTROL;
use crate::constants::LABEL_SIZE;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;

/// Stable identity plus visible label for a title-bar chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleChip {
    id:    &'static str,
    label: &'static str,
}

impl TitleChip {
    /// Creates a title chip with separate stable identity and visible label.
    #[must_use]
    pub const fn new(id: &'static str, label: &'static str) -> Self { Self { id, label } }

    /// Creates a title chip whose identity and visible label are the same.
    #[must_use]
    pub const fn label(label: &'static str) -> Self { Self { id: label, label } }

    /// Returns the stable chip identity.
    #[must_use]
    pub const fn id(self) -> &'static str { self.id }

    /// Returns the visible chip label.
    #[must_use]
    pub const fn label_text(self) -> &'static str { self.label }
}

/// One independently highlightable word inside a segmented control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleBarSegment {
    id:    String,
    label: String,
}

impl TitleBarSegment {
    /// Creates a segment with separate stable identity and visible label.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id:    id.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Stored title-bar control with stable identity and visible label.
pub struct TitleBarControl {
    id:            String,
    label:         String,
    segments:      Vec<TitleBarSegment>,
    disabled_note: Option<String>,
}

impl TitleBarControl {
    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id:            id.into(),
            label:         label.into(),
            segments:      Vec::new(),
            disabled_note: None,
        }
    }

    fn from_label(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            id: label.clone(),
            label,
            segments: Vec::new(),
            disabled_note: None,
        }
    }

    /// Adds a note rendered beside the label while the control is
    /// [`ControlActivation::Disabled`], explaining why it is unavailable
    /// (for example `"(no gamepad)"`). Only plain (non-segmented) controls
    /// show the note.
    #[must_use]
    pub fn with_disabled_note(mut self, note: impl Into<String>) -> Self {
        self.disabled_note = Some(note.into());
        self
    }

    /// Creates a control rendered as one cell: a key-hint label followed by
    /// words that highlight independently, each by its segment id. The hint
    /// itself never highlights.
    #[must_use]
    pub fn segmented(
        hint: impl Into<String>,
        segments: impl IntoIterator<Item = TitleBarSegment>,
    ) -> Self {
        let hint = hint.into();
        Self {
            id:            hint.clone(),
            label:         hint,
            segments:      segments.into_iter().collect(),
            disabled_note: None,
        }
    }
}

impl From<TitleChip> for TitleBarControl {
    fn from(value: TitleChip) -> Self { Self::new(value.id, value.label) }
}

impl From<TitleChip> for String {
    fn from(value: TitleChip) -> Self { value.id.to_string() }
}

impl From<&'static str> for TitleBarControl {
    fn from(value: &'static str) -> Self { Self::from_label(value) }
}

impl From<String> for TitleBarControl {
    fn from(value: String) -> Self { Self::from_label(value) }
}

/// Resource-level sugar for one-resource / one-chip title-bar activation.
pub trait TitleChipActivation {
    /// Returns whether the chip should be highlighted.
    fn activation(&self) -> ControlActivation;
}

#[derive(Resource, Default)]
pub(crate) struct TitleBarControlRegistry {
    controls: Vec<TitleBarControl>,
}

impl TitleBarControlRegistry {
    pub(crate) fn push(&mut self, control: impl Into<TitleBarControl>) {
        let control = control.into();
        if self
            .controls
            .iter()
            .any(|existing| existing.id == control.id)
        {
            return;
        }
        self.controls.push(control);
    }
}

/// Whether title-bar chips lay out in a single row or stack in a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleBarOrientation {
    /// Chips share one horizontal row separated by vertical dividers.
    #[default]
    Horizontal,
    /// Each chip occupies its own row, separated by horizontal dividers.
    Vertical,
}

/// A compact top-left title bar for example-level controls.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TitleBar {
    anchor:           Anchor,
    orientation:      TitleBarOrientation,
    title:            String,
    controls:         Vec<TitleBarControl>,
    active_controls:  Vec<String>,
    background_color: Option<Color>,
}

impl TitleBar {
    /// Creates a title bar with the visible example title.
    #[must_use]
    pub fn new() -> Self {
        Self {
            anchor:           Anchor::TopLeft,
            orientation:      TitleBarOrientation::Horizontal,
            title:            TITLE_BAR_DEFAULT_TITLE.to_string(),
            controls:         Vec::new(),
            active_controls:  Vec::new(),
            background_color: None,
        }
    }

    /// Overrides the rendered title. Strings render literally — pass the
    /// case you want displayed.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the title bar screen anchor.
    #[must_use]
    pub const fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets whether chips lay out in a row (default) or stack in a column.
    #[must_use]
    pub const fn with_orientation(mut self, orientation: TitleBarOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Overrides the inner background color (including alpha) for this
    /// title bar. Defaults to the crate's `INNER_BACKGROUND` constant.
    #[must_use]
    pub const fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Adds a compact control label such as `H Home`.
    #[must_use]
    pub fn control(mut self, control: impl Into<TitleBarControl>) -> Self {
        self.controls.push(control.into());
        self
    }

    /// Adds a compact control label that starts highlighted.
    #[must_use]
    pub fn active_control(mut self, control: impl Into<TitleBarControl>) -> Self {
        let control = control.into();
        self.active_controls.push(control.id.clone());
        self.controls.push(control);
        self
    }

    /// Adds multiple compact control labels.
    #[must_use]
    pub fn controls(
        mut self,
        controls: impl IntoIterator<Item = impl Into<TitleBarControl>>,
    ) -> Self {
        self.controls.extend(controls.into_iter().map(Into::into));
        self
    }
}

impl Default for TitleBar {
    fn default() -> Self { Self::new() }
}

/// How a control label should be styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlActivation {
    /// Highlight the control.
    Active,
    /// Clear the control's highlight.
    Inactive,
    /// Grey the control out to show it cannot be used right now. A control's
    /// disabled note (see [`TitleBarControl::with_disabled_note`]) renders
    /// beside its label in this state.
    Disabled,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleBarControlState {
    activations: HashMap<String, ControlActivation>,
}

impl TitleBarControlState {
    fn from_title_bar(title_bar: &TitleBar) -> Self {
        Self {
            activations: title_bar
                .active_controls
                .iter()
                .map(|control| (control.clone(), ControlActivation::Active))
                .collect(),
        }
    }

    /// Sets a control's activation. Repeated calls with the same value are
    /// no-ops. [`ControlActivation::Inactive`] is the default and is stored as
    /// the absence of an entry.
    pub(crate) fn set_active(&mut self, control: &str, activation: ControlActivation) {
        match activation {
            ControlActivation::Inactive => {
                self.activations.remove(control);
            },
            ControlActivation::Active | ControlActivation::Disabled => {
                self.activations.insert(control.to_string(), activation);
            },
        }
    }

    /// Returns a control's activation, defaulting to
    /// [`ControlActivation::Inactive`] for controls that were never set.
    #[must_use]
    pub(crate) fn activation(&self, control: &str) -> ControlActivation {
        self.activations
            .get(control)
            .copied()
            .unwrap_or(ControlActivation::Inactive)
    }
}

#[derive(Component)]
struct TitleBarMarker;

#[derive(Resource, Default)]
pub(crate) struct HomeTitleBarFlash {
    timer: Option<Timer>,
}

impl HomeTitleBarFlash {
    pub(crate) fn start(&mut self) {
        self.timer = Some(Timer::new(HOME_TITLE_BAR_HIGHLIGHT_HOLD, TimerMode::Once));
    }

    pub(crate) const fn cancel(&mut self) { self.timer = None; }
}

/// The three label styles a control cell picks between by its
/// [`ControlActivation`].
struct ControlStyles {
    active:   TextStyle,
    inactive: TextStyle,
    disabled: TextStyle,
}

impl ControlStyles {
    fn new(fonts: &FontRegistry) -> Self {
        let size = super::integral_advance_size(fonts, LABEL_SIZE);
        let control_style = |color| {
            TextStyle::new(size)
                .with_color(color)
                .with_shadow_mode(GlyphShadowMode::None)
        };
        Self {
            active:   control_style(CONTROL_ACTIVE_COLOR),
            inactive: control_style(CONTROL_INACTIVE_COLOR),
            disabled: control_style(CONTROL_DISABLED_COLOR),
        }
    }

    fn for_activation(&self, activation: ControlActivation) -> TextStyle {
        match activation {
            ControlActivation::Active => self.active.clone(),
            ControlActivation::Inactive => self.inactive.clone(),
            ControlActivation::Disabled => self.disabled.clone(),
        }
    }
}

pub(super) fn tick_home_title_bar_flash(
    time: Res<Time>,
    mut flash: ResMut<HomeTitleBarFlash>,
    mut bars: Query<&mut TitleBarControlState>,
) {
    let Some(timer) = flash.timer.as_mut() else {
        return;
    };
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    for mut bar in &mut bars {
        bar.set_active(HOME_CONTROL, ControlActivation::Inactive);
    }
    flash.timer = None;
}

pub(super) fn spawn_title_bar_with_home_chip(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    title_bar: &TitleBar,
    fonts: &FontRegistry,
    home: Option<&CameraHomeConfig>,
    registry: Option<&TitleBarControlRegistry>,
) {
    let mut title_bar = title_bar.clone();
    if home.is_some_and(|home| matches!(home.title_bar_control, HomeTitleBarControl::Shown))
        && !title_bar
            .controls
            .iter()
            .any(|control| control.id == HOME_CONTROL)
    {
        title_bar
            .controls
            .insert(0, TitleBarControl::from(HOME_CONTROL));
    }
    if let Some(registry) = registry {
        for control in &registry.controls {
            if !title_bar
                .controls
                .iter()
                .any(|existing| existing.id == control.id)
            {
                title_bar.controls.push(control.clone());
            }
        }
    }
    spawn_title_bar(commands, materials, &title_bar, fonts);
}

fn spawn_title_bar(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    title_bar: &TitleBar,
    fonts: &FontRegistry,
) {
    let state = TitleBarControlState::from_title_bar(title_bar);
    let unlit = super::screen_panel_material_handle(materials);
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(title_bar.anchor)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_title_bar_tree(title_bar, &state, fonts))
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((
                TitleBarMarker,
                title_bar.clone(),
                state,
                panel,
                Transform::default(),
            ));
        },
        Err(error) => {
            error!("fairy_dust: failed to build title bar: {error}");
        },
    }
}

pub(crate) fn refresh_changed_title_bar(
    mut commands: Commands,
    title_bars: Query<
        (Entity, &TitleBar, &TitleBarControlState),
        Or<(Changed<TitleBar>, Changed<TitleBarControlState>)>,
    >,
    fonts: Res<FontRegistry>,
) {
    for (entity, title_bar, state) in &title_bars {
        if let Err(error) =
            commands.set_tree(entity, build_title_bar_tree(title_bar, state, &fonts))
        {
            warn!("failed to replace title-bar panel {entity:?} tree: {error}");
        }
    }
}

fn build_title_bar_tree(
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    fonts: &FontRegistry,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_title_bar_layout(&mut builder, title_bar, state, fonts);
    builder.build()
}

fn build_title_bar_layout(
    builder: &mut LayoutBuilder,
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    fonts: &FontRegistry,
) {
    let title = TextStyle::new(super::integral_advance_size(fonts, TITLE_SIZE))
        .with_color(TITLE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let controls = ControlStyles::new(fonts);

    let background = title_bar
        .background_color
        .unwrap_or_else(super::default_inner_background);
    let orientation = title_bar.orientation;
    screen_panel_frame(
        builder,
        Sizing::FIT,
        Sizing::FIT,
        background,
        |builder| match orientation {
            TitleBarOrientation::Horizontal => build_title_bar_contents(
                builder,
                title_bar,
                state,
                orientation,
                El::row()
                    .width(Sizing::GROW)
                    .gap(TITLE_BAR_CHILD_GAP)
                    .align_y(AlignY::Center),
                &title,
                &controls,
            ),
            TitleBarOrientation::Vertical => build_title_bar_contents(
                builder,
                title_bar,
                state,
                orientation,
                El::column().width(Sizing::GROW).gap(TITLE_BAR_CHILD_GAP),
                &title,
                &controls,
            ),
        },
    );
}

fn build_title_bar_contents<L: ChildLayoutState>(
    builder: &mut LayoutBuilder,
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    orientation: TitleBarOrientation,
    container: El<L>,
    title: &TextStyle,
    controls: &ControlStyles,
) {
    builder.with(container, |builder| {
        builder.text((&title_bar.title, title.clone()));
        for control in &title_bar.controls {
            title_separator(builder, orientation);
            if control.segments.is_empty() {
                plain_control_cell(builder, control, state, controls);
            } else {
                segmented_control_cell(builder, control, state, controls);
            }
        }
        title_separator(builder, orientation);
        let help_style = controls.for_activation(state.activation(HELP_CONTROL));
        builder.text((HELP_CONTROL, help_style));
    });
}

/// One title-bar cell holding a plain control label. A disabled control with a
/// note renders the note beside its label.
fn plain_control_cell(
    builder: &mut LayoutBuilder,
    control: &TitleBarControl,
    state: &TitleBarControlState,
    controls: &ControlStyles,
) {
    let activation = state.activation(&control.id);
    let style = controls.for_activation(activation);
    match (activation, control.disabled_note.as_deref()) {
        (ControlActivation::Disabled, Some(note)) => {
            let label = format!("{} {note}", control.label);
            builder.text((label.as_str(), style));
        },
        _ => {
            builder.text((&control.label, style));
        },
    }
}

/// One title-bar cell holding a segmented control: the key hint followed by
/// its segments, each styled by its own activation state.
fn segmented_control_cell(
    builder: &mut LayoutBuilder,
    control: &TitleBarControl,
    state: &TitleBarControlState,
    controls: &ControlStyles,
) {
    let row = El::row().gap(TITLE_BAR_SEGMENT_GAP).align_y(AlignY::Center);
    builder.with(row, |builder| {
        builder.text((&control.label, controls.inactive.clone()));
        for segment in &control.segments {
            let style = controls.for_activation(state.activation(&segment.id));
            builder.text((&segment.label, style));
        }
    });
}

fn title_separator(builder: &mut LayoutBuilder, orientation: TitleBarOrientation) {
    let (width, height) = match orientation {
        TitleBarOrientation::Horizontal => (
            Sizing::fixed(SEPARATOR_WIDTH),
            Sizing::fixed(SEPARATOR_HEIGHT),
        ),
        TitleBarOrientation::Vertical => (Sizing::GROW, Sizing::fixed(SEPARATOR_WIDTH)),
    };
    builder.with(
        El::new()
            .width(width)
            .height(height)
            .background(DIVIDER_COLOR),
        |_| {},
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_bar_control_state_tracks_active_labels() {
        let mut state = TitleBarControlState::default();

        state.set_active("H Home", ControlActivation::Active);
        assert_eq!(state.activation("H Home"), ControlActivation::Active);

        // Repeat Active must be a no-op — otherwise a duplicate entry could
        // survive the single Inactive below.
        state.set_active("H Home", ControlActivation::Active);
        state.set_active("H Home", ControlActivation::Inactive);
        assert_eq!(state.activation("H Home"), ControlActivation::Inactive);

        // Repeat Inactive on a control with no entry must be a no-op.
        state.set_active("H Home", ControlActivation::Inactive);
        assert_eq!(state.activation("H Home"), ControlActivation::Inactive);
    }

    #[test]
    fn segmented_control_keeps_hint_id_and_segment_ids() {
        let control = TitleBarControl::segmented(
            "A",
            [
                TitleBarSegment::new("aa-off", "Off"),
                TitleBarSegment::new("aa-both", "Both"),
            ],
        );
        assert_eq!(control.id, "A");
        assert_eq!(control.label, "A");
        assert_eq!(control.segments.len(), 2);

        // Segment highlight uses the same id-based state as whole chips.
        let mut state = TitleBarControlState::default();
        state.set_active("aa-both", ControlActivation::Active);
        assert_eq!(state.activation("aa-both"), ControlActivation::Active);
        assert_eq!(state.activation("aa-off"), ControlActivation::Inactive);
        assert_eq!(state.activation("A"), ControlActivation::Inactive);
    }

    #[test]
    fn disabled_control_reads_as_disabled() {
        let mut state = TitleBarControlState::default();

        state.set_active("G Cycle Input", ControlActivation::Disabled);
        assert_eq!(
            state.activation("G Cycle Input"),
            ControlActivation::Disabled
        );

        // Clearing to Inactive drops the entry back to the default.
        state.set_active("G Cycle Input", ControlActivation::Inactive);
        assert_eq!(
            state.activation("G Cycle Input"),
            ControlActivation::Inactive
        );
    }

    #[test]
    fn with_disabled_note_stores_the_note() {
        let control = TitleBarControl::from("G Cycle Input").with_disabled_note("(no gamepad)");

        assert_eq!(control.disabled_note.as_deref(), Some("(no gamepad)"));
    }

    #[test]
    fn title_bar_can_seed_active_controls() {
        let title_bar = TitleBar::new()
            .with_title("Demo")
            .control("A Action")
            .active_control("T Toggle");
        let state = TitleBarControlState::from_title_bar(&title_bar);

        assert_eq!(state.activation("A Action"), ControlActivation::Inactive);
        assert_eq!(state.activation("T Toggle"), ControlActivation::Active);
    }
}
