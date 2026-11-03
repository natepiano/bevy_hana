//! Compact top-left title bar for example-level controls.

use std::collections::HashMap;

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::ChildLayoutState;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

use super::PanelTextEdge;
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
use super::separation::EdgePanelPrecedence;
use super::separation::SCREEN_PANEL_WINDOW_INSET;
use crate::camera_home::CameraHomeConfig;
use crate::camera_home::HomeTitleBarControl;
use crate::constants;
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
    id:                String,
    label:             String,
    segments:          Vec<TitleBarSegment>,
    disabled_note:     DisabledNote,
    segment_separator: SegmentSeparator,
    name:              ControlName,
    help_wording:      HelpWording,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum DisabledNote {
    #[default]
    None,
    Explain(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum SegmentSeparator {
    #[default]
    Spacing,
    Neutral(String),
}

impl SegmentSeparator {
    fn text(&self) -> &str {
        match self {
            Self::Spacing => "",
            Self::Neutral(text) => text,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ControlName {
    #[default]
    Unnamed,
    Named(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum HelpWording {
    #[default]
    Inherit,
    Explicit {
        keys:   String,
        action: String,
    },
}

impl TitleBarControl {
    fn segmented_label(&self) -> String {
        match &self.name {
            ControlName::Unnamed => self.label.clone(),
            ControlName::Named(name) => format!("{} {name}:", self.label),
        }
    }

    pub(super) fn help_parts(&self) -> (String, String) {
        if let HelpWording::Explicit { keys, action } = &self.help_wording {
            return (keys.clone(), action.clone());
        }
        if let ControlName::Named(name) = &self.name {
            return (
                self.label.clone(),
                format!(
                    "{}: {}",
                    name,
                    self.segments
                        .iter()
                        .map(|segment| segment.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        let words = self.label.split_whitespace().collect::<Vec<_>>();
        let key_end = words
            .iter()
            .take_while(|word| matches!(**word, "Ctrl" | "Shift" | "Alt" | "Meta"))
            .count()
            .saturating_add(1)
            .min(words.len());
        let key_end = if words.get(key_end) == Some(&"/") && words.get(key_end + 1).is_some() {
            key_end + 2
        } else {
            key_end
        };
        let keys = words[..key_end].join(" ");
        let mut action = words[key_end..].join(" ");
        if !self.segments.is_empty() {
            let separator = match &self.segment_separator {
                SegmentSeparator::Spacing => " ".to_string(),
                SegmentSeparator::Neutral(separator) => format!("{separator} "),
            };
            let segments = self
                .segments
                .iter()
                .map(|segment| segment.label.as_str())
                .collect::<Vec<_>>()
                .join(&separator);
            action = if action.is_empty() {
                segments
            } else {
                format!("{action} {segments}")
            };
        }
        (keys, action)
    }

    /// Gives Help its own key and action wording for this control.
    #[must_use]
    pub fn with_help_wording(mut self, keys: impl Into<String>, action: impl Into<String>) -> Self {
        self.help_wording = HelpWording::Explicit {
            keys:   keys.into(),
            action: action.into(),
        };
        self
    }

    /// Names a segmented setting in the bar and Help.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = ControlName::Named(name.into());
        self
    }

    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id:                id.into(),
            label:             label.into(),
            segments:          Vec::new(),
            disabled_note:     DisabledNote::None,
            segment_separator: SegmentSeparator::Spacing,
            name:              ControlName::Unnamed,
            help_wording:      HelpWording::Inherit,
        }
    }

    fn from_label(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            id: label.clone(),
            label,
            segments: Vec::new(),
            disabled_note: DisabledNote::None,
            segment_separator: SegmentSeparator::Spacing,
            name: ControlName::Unnamed,
            help_wording: HelpWording::Inherit,
        }
    }

    /// Adds a note rendered beside the label while the control is
    /// [`ControlActivation::Disabled`], explaining why it is unavailable
    /// (for example `"(no gamepad)"`). Only plain (non-segmented) controls
    /// show the note.
    #[must_use]
    pub fn with_disabled_note(mut self, note: impl Into<String>) -> Self {
        self.disabled_note = DisabledNote::Explain(note.into());
        self
    }

    /// Draws punctuation between segments in the neutral control style.
    #[must_use]
    pub fn with_segment_separator(mut self, separator: impl Into<String>) -> Self {
        self.segment_separator = SegmentSeparator::Neutral(separator.into());
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
            id:                hint.clone(),
            label:             hint,
            segments:          segments.into_iter().collect(),
            disabled_note:     DisabledNote::None,
            segment_separator: SegmentSeparator::Spacing,
            name:              ControlName::Unnamed,
            help_wording:      HelpWording::Inherit,
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
    /// Returns the [`ControlActivation`] this value gives its chip.
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
/// Controls moved from the bar into contextual help at the current window width.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleBarOverflow {
    pub(super) dropped: Vec<String>,
}

impl TitleBarOverflow {
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.dropped.iter().any(|dropped| dropped == id)
    }
}

/// A compact top-left title bar for example-level controls.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TitleBar {
    anchor:           Anchor,
    orientation:      TitleBarOrientation,
    title:            String,
    controls:         Vec<TitleBarControl>,
    active_controls:  Vec<String>,
    background_color: TitleBarBackground,
}

#[derive(Clone, Debug, PartialEq)]
enum TitleBarBackground {
    Shared,
    Custom(Color),
}

/// Title controls listed in Help are exactly the controls that did not fit.
pub(super) fn dropped_controls<'a>(
    bar: &'a TitleBar,
    overflow: &'a TitleBarOverflow,
) -> impl Iterator<Item = &'a TitleBarControl> {
    bar.controls
        .iter()
        .filter(|control| overflow.contains(&control.id))
}

impl TitleBar {
    pub(crate) fn shows_home_control(&self) -> bool {
        self.controls
            .iter()
            .any(|control| control.id == HOME_CONTROL)
    }

    /// Creates a title bar with the visible example title.
    #[must_use]
    pub fn new() -> Self {
        Self {
            anchor:           Anchor::TopLeft,
            orientation:      TitleBarOrientation::Horizontal,
            title:            TITLE_BAR_DEFAULT_TITLE.to_string(),
            controls:         Vec::new(),
            active_controls:  Vec::new(),
            background_color: TitleBarBackground::Shared,
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
        self.background_color = TitleBarBackground::Custom(color);
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

#[derive(Component)]
pub(super) struct DrawnTitleBar {
    visible:       TitleBar,
    controls:      TitleBarControlState,
    window_width:  f32,
    window_height: f32,
}

impl DrawnTitleBar {
    pub(super) fn new(bar: &TitleBar, controls: &TitleBarControlState) -> Self {
        Self {
            visible:       bar.clone(),
            controls:      controls.clone(),
            window_width:  f32::MAX,
            window_height: f32::MAX,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct HomeTitleBarFlash {
    state: HomeFlashState,
}

#[derive(Default)]
enum HomeFlashState {
    #[default]
    Idle,
    Highlighting(Timer),
}

impl HomeTitleBarFlash {
    pub(crate) fn start(&mut self) {
        self.state = HomeFlashState::Highlighting(Timer::new(
            HOME_TITLE_BAR_HIGHLIGHT_HOLD,
            TimerMode::Once,
        ));
    }

    pub(crate) const fn cancel(&mut self) { self.state = HomeFlashState::Idle; }
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
    let HomeFlashState::Highlighting(timer) = &mut flash.state else {
        return;
    };
    if !timer.tick(time.delta()).just_finished() {
        return;
    }
    for mut bar in &mut bars {
        bar.set_active(HOME_CONTROL, ControlActivation::Inactive);
    }
    flash.state = HomeFlashState::Idle;
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
                EdgePanelPrecedence::TitleBar,
                title_bar.clone(),
                DrawnTitleBar::new(title_bar, &state),
                state,
                TitleBarOverflow::default(),
                panel,
                Transform::default(),
            ));
        },
        Err(error) => {
            error!("fairy_dust: failed to build title bar: {error}");
        },
    }
}

pub(super) fn refresh_changed_title_bar(
    mut commands: Commands,
    mut title_bars: Query<(
        Entity,
        Ref<TitleBar>,
        Ref<TitleBarControlState>,
        &TitleBarOverflow,
        Option<&mut DrawnTitleBar>,
        &mut Visibility,
    )>,
    windows: Query<&Window, With<PrimaryWindow>>,
    fonts: Res<FontRegistry>,
    measurer: Res<DiegeticTextMeasurer>,
) {
    let (window_width, window_height) = windows.single().map_or((f32::MAX, f32::MAX), |window| {
        (window.width(), window.height())
    });
    for (entity, title_bar, state, overflow, drawn, mut visibility) in &mut title_bars {
        if drawn.as_ref().is_some_and(|drawn| {
            drawn.window_width.to_bits() == window_width.to_bits()
                && drawn.window_height.to_bits() == window_height.to_bits()
                && !title_bar.is_changed()
                && drawn.controls == *state
        }) {
            continue;
        }
        let (visible, next_overflow) = visible_title_bar(
            &title_bar,
            &state,
            &fonts,
            &measurer,
            window_width,
            window_height,
        );
        let available = available_title_bar_space(window_width, window_height);
        let fits = title_bar_fits(&visible, &state, &fonts, &measurer, available);
        let next_visibility = if fits {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != next_visibility {
            *visibility = next_visibility;
        }
        if drawn
            .as_ref()
            .is_none_or(|drawn| drawn.visible != visible || drawn.controls != *state)
            && let Err(error) =
                commands.set_tree(entity, build_title_bar_tree(&visible, &state, &fonts))
        {
            warn!("failed to replace title-bar panel {entity:?} tree: {error}");
        }
        if *overflow != next_overflow {
            commands.entity(entity).insert(next_overflow);
        }
        if let Some(mut drawn) = drawn {
            drawn.visible = visible;
            drawn.controls = (*state).clone();
            drawn.window_width = window_width;
            drawn.window_height = window_height;
        } else {
            commands.entity(entity).insert(DrawnTitleBar {
                visible,
                controls: (*state).clone(),
                window_width,
                window_height,
            });
        }
    }
}

pub(super) fn visible_title_bar(
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    fonts: &FontRegistry,
    measurer: &DiegeticTextMeasurer,
    window_width: f32,
    window_height: f32,
) -> (TitleBar, TitleBarOverflow) {
    let mut visible = title_bar.clone();
    let available = available_title_bar_space(window_width, window_height);
    while !visible.controls.is_empty()
        && !title_bar_fits(&visible, state, fonts, measurer, available)
    {
        visible.controls.pop();
    }
    while !title_bar_fits(&visible, state, fonts, measurer, available) && !visible.title.is_empty()
    {
        let original = visible.title.trim_end_matches('…');
        let had_title = !original.is_empty();
        let mut shortened = original.to_string();
        shortened.pop();
        visible.title = shortened.trim_end().to_string();
        if had_title {
            visible.title.push('…');
        }
    }
    let dropped = title_bar.controls[visible.controls.len()..]
        .iter()
        .map(|control| control.id.clone())
        .collect();
    (visible, TitleBarOverflow { dropped })
}

fn available_title_bar_space(window_width: f32, window_height: f32) -> Vec2 {
    bevy::prelude::Vec2::new(window_width, window_height)
        - bevy::prelude::Vec2::splat(2.0 * SCREEN_PANEL_WINDOW_INSET)
}

fn title_bar_fits(
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    fonts: &FontRegistry,
    measurer: &DiegeticTextMeasurer,
    available: Vec2,
) -> bool {
    let size = title_bar_size(title_bar, state, fonts, measurer);
    size.x <= available.x && size.y <= available.y
}

fn title_bar_size(
    title_bar: &TitleBar,
    state: &TitleBarControlState,
    fonts: &FontRegistry,
    measurer: &DiegeticTextMeasurer,
) -> Vec2 {
    let title_size = super::integral_advance_size(fonts, TITLE_SIZE);
    let control_size = super::integral_advance_size(fonts, LABEL_SIZE);
    let text_size = |text: &str, size: Pt, edge: PanelTextEdge| {
        let bounds = (measurer.measure_fn)(text, &TextStyle::new(size).as_measure());
        let (top, bottom) = super::screen_panel_text_edge_trim(fonts, size);
        let trim = match edge {
            PanelTextEdge::Interior => 0.0,
            PanelTextEdge::First => top.0,
            PanelTextEdge::Last => bottom.0,
            PanelTextEdge::Both => top.0 + bottom.0,
        };
        Vec2::new(
            bounds.width / Unit::Pixels.to_points(),
            bounds.height / Unit::Pixels.to_points() - trim,
        )
    };
    let horizontal = title_bar.orientation == TitleBarOrientation::Horizontal;
    let text_edge = if horizontal {
        PanelTextEdge::Both
    } else {
        PanelTextEdge::Interior
    };
    let separator_height = fonts
        .font(FontId::MONOSPACE)
        .map_or(SEPARATOR_HEIGHT.0, |font| {
            font.metrics(title_size.0 * 4.0 / 3.0).cap_height
        });
    let separator = if horizontal {
        Vec2::new(SEPARATOR_WIDTH.0, separator_height)
    } else {
        Vec2::new(0.0, SEPARATOR_WIDTH.0)
    };
    let mut cells = Vec::new();
    if !title_bar.title.is_empty() {
        cells.push(text_size(
            &title_bar.title,
            title_size,
            if horizontal {
                text_edge
            } else {
                PanelTextEdge::First
            },
        ));
    }
    for control in &title_bar.controls {
        cells.push(separator);
        let label = if state.activation(&control.id) == ControlActivation::Disabled {
            match &control.disabled_note {
                DisabledNote::None => control.label.clone(),
                DisabledNote::Explain(note) => format!("{} {note}", control.label),
            }
        } else {
            control.label.clone()
        };
        let drawn_label = if control.segments.is_empty() {
            label
        } else {
            control.segmented_label()
        };
        let mut cell = text_size(&drawn_label, control_size, text_edge);
        for (index, segment) in control.segments.iter().enumerate() {
            let segment_size = text_size(&segment.label, control_size, text_edge);
            cell.x += TITLE_BAR_SEGMENT_GAP.0 + segment_size.x;
            cell.y = cell.y.max(segment_size.y);
            if index + 1 < control.segments.len() {
                cell.x += text_size(control.segment_separator.text(), control_size, text_edge).x;
            }
        }
        cells.push(cell);
    }
    if !title_bar.title.is_empty() || !title_bar.controls.is_empty() {
        cells.push(separator);
    }
    cells.push(text_size(
        HELP_CONTROL,
        control_size,
        if horizontal {
            text_edge
        } else {
            PanelTextEdge::Last
        },
    ));
    let gap = cells
        .iter()
        .skip(1)
        .map(|_| TITLE_BAR_CHILD_GAP.0)
        .sum::<f32>();
    let padding = 2.0 * constants::screen_spacing().panel_padding.0;
    if horizontal {
        Vec2::new(
            cells.iter().map(|cell| cell.x).sum::<f32>() + gap + padding,
            cells.iter().map(|cell| cell.y).fold(0.0, f32::max) + padding,
        )
    } else {
        Vec2::new(
            cells.iter().map(|cell| cell.x).fold(0.0, f32::max) + padding,
            cells.iter().map(|cell| cell.y).sum::<f32>() + gap + padding,
        )
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
    let title_size = super::integral_advance_size(fonts, TITLE_SIZE);
    let title = TextStyle::new(title_size)
        .with_color(TITLE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let controls = ControlStyles::new(fonts);

    let background = match title_bar.background_color {
        TitleBarBackground::Shared => super::default_inner_background(),
        TitleBarBackground::Custom(color) => color,
    };
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
                fonts,
                title_size,
            ),
            TitleBarOrientation::Vertical => build_title_bar_contents(
                builder,
                title_bar,
                state,
                orientation,
                El::column().width(Sizing::GROW).gap(TITLE_BAR_CHILD_GAP),
                &title,
                &controls,
                fonts,
                title_size,
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
    fonts: &FontRegistry,
    title_size: Pt,
) {
    let separator_height = fonts
        .font(FontId::MONOSPACE)
        .map_or(SEPARATOR_HEIGHT, |font| {
            Px(font.metrics(title_size.0 * 4.0 / 3.0).cap_height)
        });
    builder.with(container, |builder| {
        if !title_bar.title.is_empty() {
            builder.text(super::screen_panel_ink_text(
                &title_bar.title,
                title.clone(),
                fonts,
                title_size,
                if orientation == TitleBarOrientation::Vertical {
                    PanelTextEdge::First
                } else {
                    PanelTextEdge::Both
                },
            ));
        }
        for control in &title_bar.controls {
            title_separator(builder, orientation, separator_height);
            if control.segments.is_empty() {
                plain_control_cell(builder, control, state, controls, fonts, orientation);
            } else {
                segmented_control_cell(builder, control, state, controls, fonts, orientation);
            }
        }
        if !title_bar.title.is_empty() || !title_bar.controls.is_empty() {
            title_separator(builder, orientation, separator_height);
        }
        let help_style = controls.for_activation(state.activation(HELP_CONTROL));
        builder.text(super::screen_panel_ink_text(
            HELP_CONTROL,
            help_style,
            fonts,
            super::integral_advance_size(fonts, LABEL_SIZE),
            if orientation == TitleBarOrientation::Vertical {
                PanelTextEdge::Last
            } else {
                PanelTextEdge::Both
            },
        ));
    });
}

/// One title-bar cell holding a plain control label. A disabled control with a
/// note renders the note beside its label.
fn plain_control_cell(
    builder: &mut LayoutBuilder,
    control: &TitleBarControl,
    state: &TitleBarControlState,
    controls: &ControlStyles,
    fonts: &FontRegistry,
    orientation: TitleBarOrientation,
) {
    let activation = state.activation(&control.id);
    let style = controls.for_activation(activation);
    let size = super::integral_advance_size(fonts, LABEL_SIZE);
    let edge = if orientation == TitleBarOrientation::Vertical {
        PanelTextEdge::Interior
    } else {
        PanelTextEdge::Both
    };
    match (activation, &control.disabled_note) {
        (ControlActivation::Disabled, DisabledNote::Explain(note)) => {
            let label = format!("{} {note}", control.label);
            builder.text(super::screen_panel_ink_text(
                label, style, fonts, size, edge,
            ));
        },
        _ => {
            builder.text(super::screen_panel_ink_text(
                &control.label,
                style,
                fonts,
                size,
                edge,
            ));
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
    fonts: &FontRegistry,
    orientation: TitleBarOrientation,
) {
    let size = super::integral_advance_size(fonts, LABEL_SIZE);
    let edge = if orientation == TitleBarOrientation::Vertical {
        PanelTextEdge::Interior
    } else {
        PanelTextEdge::Both
    };
    let row = El::row().gap(TITLE_BAR_SEGMENT_GAP).align_y(AlignY::Center);
    builder.with(row, |builder| {
        builder.text(super::screen_panel_ink_text(
            control.segmented_label(),
            controls.inactive.clone(),
            fonts,
            size,
            edge,
        ));
        for (index, segment) in control.segments.iter().enumerate() {
            let style = controls.for_activation(state.activation(&segment.id));
            builder.with(El::row().align_y(AlignY::Center), |builder| {
                builder.text(super::screen_panel_ink_text(
                    &segment.label,
                    style,
                    fonts,
                    size,
                    edge,
                ));
                if index + 1 < control.segments.len()
                    && let SegmentSeparator::Neutral(separator) = &control.segment_separator
                {
                    builder.text(super::screen_panel_ink_text(
                        separator,
                        controls.inactive.clone(),
                        fonts,
                        size,
                        edge,
                    ));
                }
            });
        }
    });
}

fn title_separator(builder: &mut LayoutBuilder, orientation: TitleBarOrientation, cap_height: Px) {
    let (width, height) = match orientation {
        TitleBarOrientation::Horizontal => {
            (Sizing::fixed(SEPARATOR_WIDTH), Sizing::fixed(cap_height))
        },
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use bevy::ecs::world::CommandQueue;
    use bevy::prelude::MinimalPlugins;
    use bevy::window::PrimaryWindow;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::Unit;

    use super::*;

    #[test]
    fn explicit_home_control_is_spawned_once() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new().control(HOME_CONTROL);
        let home = CameraHomeConfig {
            yaw:               0.0,
            pitch:             0.0,
            margin:            0.0,
            anchor:            Anchor::Center,
            offset_px:         Vec2::ZERO,
            title_bar_control: HomeTitleBarControl::Shown,
        };
        let mut registry = TitleBarControlRegistry::default();
        registry.push(HOME_CONTROL);
        let mut app = crate::test_app();
        let mut materials = Assets::<StandardMaterial>::default();
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, app.world());
            spawn_title_bar_with_home_chip(
                &mut commands,
                &mut materials,
                &bar,
                &fonts,
                Some(&home),
                Some(&registry),
            );
        }
        queue.apply(app.world_mut());

        let mut bars = app.world_mut().query::<&TitleBar>();
        let spawned = bars
            .single(app.world())
            .map_err(|error| error.to_string())?;
        assert_eq!(
            spawned
                .controls
                .iter()
                .filter(|control| control.id == HOME_CONTROL)
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn shortened_title_never_puts_whitespace_before_ellipsis() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let bar = TitleBar::new().with_title("A B C D");
        let state = TitleBarControlState::from_title_bar(&bar);
        let shortened = TitleBar::new().with_title("A …");
        let shortened_width = title_bar_size(&shortened, &state, &fonts, &measurer).x;
        let (visible, _) = visible_title_bar(
            &bar,
            &state,
            &fonts,
            &measurer,
            2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, shortened_width),
            500.0,
        );
        assert!(visible.title.ends_with('…'), "{:?}", visible.title);
        assert!(!visible.title.ends_with(" …"), "{:?}", visible.title);
        let drawn = format!("{:?}", build_title_bar_tree(&visible, &state, &fonts));
        assert!(!drawn.contains(" …"), "{drawn}");
        Ok(())
    }

    #[test]
    fn title_bar_fit_chooses_first_two_axis_rung_across_window_sizes() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        for orientation in [
            TitleBarOrientation::Horizontal,
            TitleBarOrientation::Vertical,
        ] {
            let bar = TitleBar::new()
                .with_title("Widget Lab")
                .with_orientation(orientation)
                .control("H Home")
                .control("D Enable/disable");
            let state = TitleBarControlState::from_title_bar(&bar);
            let mut ladder = vec![bar.clone()];
            while !ladder.last().ok_or("empty ladder")?.controls.is_empty() {
                let mut rung = ladder.last().ok_or("empty ladder")?.clone();
                rung.controls.pop();
                ladder.push(rung);
            }
            while !ladder.last().ok_or("empty ladder")?.title.is_empty() {
                let mut rung = ladder.last().ok_or("empty ladder")?.clone();
                let original = rung.title.trim_end_matches('…');
                let had_title = !original.is_empty();
                let mut title = original.to_string();
                title.pop();
                rung.title = title.trim_end().to_string();
                if had_title {
                    rung.title.push('…');
                }
                ladder.push(rung);
            }
            let solved_sizes = solved_rung_sizes(&ladder, &state, &fonts, orientation)?;
            for (rung, solved) in ladder.iter().zip(&solved_sizes) {
                let fitted_size = title_bar_size(rung, &state, &fonts, &measurer);
                assert!(
                    (fitted_size.x - solved.x).abs() <= 0.5
                        && (fitted_size.y - solved.y).abs() <= 0.5,
                    "{orientation:?} rung {:?}: measured {fitted_size:?}, solved {solved:?}",
                    rung.title,
                );
            }
            for width in (64_u16..=1600).step_by(8) {
                for height in (64_u16..=1200).step_by(8) {
                    let available = available_title_bar_space(f32::from(width), f32::from(height));
                    let first = solved_sizes
                        .iter()
                        .position(|size| size.x <= available.x && size.y <= available.y);
                    let (visible, _) = visible_title_bar(
                        &bar,
                        &state,
                        &fonts,
                        &measurer,
                        f32::from(width),
                        f32::from(height),
                    );
                    let draws = title_bar_fits(&visible, &state, &fonts, &measurer, available);
                    assert_eq!(
                        draws,
                        first.is_some(),
                        "{orientation:?} at {width}x{height}"
                    );
                    if let Some(first) = first {
                        assert_eq!(
                            visible, ladder[first],
                            "{orientation:?} at {width}x{height}"
                        );
                        let bar_width = title_bar_size(&visible, &state, &fonts, &measurer).x;
                        let bar_height = title_bar_size(&visible, &state, &fonts, &measurer).y;
                        assert!(bar_width <= available.x && bar_height <= available.y);
                        assert!(
                            !visible.title.ends_with(" …"),
                            "{orientation:?} at {width}x{height}: {:?}",
                            visible.title
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn solved_rung_sizes(
        ladder: &[TitleBar],
        state: &TitleBarControlState,
        fonts: &FontRegistry,
        orientation: TitleBarOrientation,
    ) -> Result<Vec<Vec2>, String> {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let mut rung_entities = Vec::new();
        for rung in ladder {
            let panel = DiegeticPanel::screen()
                .size(Fit, Fit)
                .with_tree(build_title_bar_tree(rung, state, fonts))
                .build()
                .map_err(|error| error.to_string())?;
            rung_entities.push(app.world_mut().spawn(panel).id());
        }
        app.update();
        let font = fonts
            .font(FontId::MONOSPACE)
            .map_err(|error| error.to_string())?;
        let mut sizes = Vec::new();
        for (rung, entity) in ladder.iter().zip(rung_entities) {
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("title bar rung did not solve")?;
            let root = solved.computed[0].bounds;
            sizes.push(Vec2::new(root.width, root.height) / Unit::Pixels.to_points());
            for command in &solved.commands {
                let debug = format!("{:?}", command.kind);
                if !debug.starts_with("Text {") {
                    continue;
                }
                let is_title = !rung.title.is_empty()
                    && debug.starts_with(&format!("Text {{ text: {:?},", rung.title));
                let size = super::super::integral_advance_size(
                    fonts,
                    if is_title { TITLE_SIZE } else { LABEL_SIZE },
                );
                let metrics = font.metrics(size.0 * 4.0 / 3.0);
                let points_per_pixel = Unit::Pixels.to_points();
                let ink_top = (metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height)
                    .mul_add(points_per_pixel, command.bounds.y);
                let ink_bottom = metrics
                    .line_gap
                    .mul_add(0.5, metrics.descent)
                    .mul_add(-points_per_pixel, command.bounds.y + command.bounds.height);
                assert!(
                    command.bounds.x >= root.x - 0.5
                        && command.bounds.x + command.bounds.width <= root.x + root.width + 0.5
                        && ink_top >= root.y - 0.5
                        && ink_bottom <= root.y + root.height + 0.5,
                    "{orientation:?} rung {:?} clips {debug}",
                    rung.title
                );
            }
        }
        Ok(sizes)
    }

    fn help_only_threshold(
        orientation: TitleBarOrientation,
        fonts: &FontRegistry,
        measurer: &DiegeticTextMeasurer,
    ) -> f32 {
        let bar = TitleBar::new().with_title("").with_orientation(orientation);
        let extent = match orientation {
            TitleBarOrientation::Horizontal => {
                title_bar_size(&bar, &TitleBarControlState::default(), fonts, measurer).x
            },
            TitleBarOrientation::Vertical => {
                title_bar_size(&bar, &TitleBarControlState::default(), fonts, measurer).y
            },
        };
        2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, extent).ceil()
    }

    fn assert_title_bar_fits_each_update(orientation: TitleBarOrientation) -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let bar = TitleBar::new()
            .with_title("Widget Lab")
            .with_orientation(orientation)
            .control("H Home")
            .control("D Enable/disable");
        let state = TitleBarControlState::from_title_bar(&bar);
        let threshold = help_only_threshold(orientation, &fonts, &measurer);
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(fonts)
            .insert_resource(measurer)
            .insert_resource(Assets::<StandardMaterial>::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        super::super::install_title_bar_overflow_refresh(&mut app);
        app.init_resource::<HomeTitleBarFlash>();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let fonts = app.world().resource::<FontRegistry>();
        let mut panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(build_title_bar_tree(&bar, &state, fonts))
            .build()
            .map_err(|error| error.to_string())?;
        assert!(panel.set_screen_position(bevy::prelude::Vec2::splat(SCREEN_PANEL_WINDOW_INSET)));
        let entity = app
            .world_mut()
            .spawn((
                bar.clone(),
                state.clone(),
                TitleBarOverflow::default(),
                DrawnTitleBar::new(&bar, &state),
                panel,
                Visibility::Inherited,
            ))
            .id();
        let sizes = [
            600.0,
            300.0,
            200.0,
            threshold + 1.0,
            threshold,
            threshold - 1.0,
            1.0,
        ];
        for size in sizes {
            let (width, height) = match orientation {
                TitleBarOrientation::Horizontal => (size, 700.0),
                TitleBarOrientation::Vertical => (700.0, size),
            };
            app.world_mut()
                .get_mut::<Window>(window)
                .ok_or("window missing")?
                .resolution
                .set(width, height);
            app.update();
            let visibility = app
                .world()
                .get::<Visibility>(entity)
                .ok_or("visibility missing")?;
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("title bar did not solve")?;
            let extent = match orientation {
                TitleBarOrientation::Horizontal => solved.computed[0].bounds.width,
                TitleBarOrientation::Vertical => solved.computed[0].bounds.height,
            } / Unit::Pixels.to_points();
            if *visibility == Visibility::Hidden {
                assert!(size < threshold, "{orientation:?} hidden at {size}");
            } else {
                assert!(
                    2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, extent) <= size + 0.5,
                    "{orientation:?} bar extends past inset at {size}: {extent}"
                );
            }
            if (size - threshold).abs() < 0.01 {
                let drawn = format!("{:?}", solved.commands);
                assert!(drawn.contains("? Help"), "{drawn}");
                assert!(!drawn.contains("Widget Lab"), "{drawn}");
                assert!(!drawn.contains("H Home"), "{drawn}");
                assert_eq!(*visibility, Visibility::Inherited);
            }
            if orientation == TitleBarOrientation::Vertical && size <= threshold {
                let overflow = app
                    .world()
                    .get::<TitleBarOverflow>(entity)
                    .ok_or("overflow missing")?;
                assert!(overflow.contains("H Home"));
                assert!(overflow.contains("D Enable/disable"));
            }
        }
        Ok(())
    }

    #[test]
    fn horizontal_bar_keeps_insets_until_help_itself_cannot_fit() -> Result<(), String> {
        assert_title_bar_fits_each_update(TitleBarOrientation::Horizontal)
    }

    #[test]
    fn units_card_and_photo_chips_fit_or_overflow_together() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new()
            .with_title("Units")
            .control("H Home")
            .controls(["D Outlines", "R Rulers", "F Fade", "↑ / ↓ Strength"])
            .control(
                TitleBarControl::segmented(
                    "A",
                    ["Off", "Anisotropic", "Supersample", "Both"]
                        .map(|word| TitleBarSegment::new(word, word)),
                )
                .with_name("Smoothing"),
            )
            .controls([
                "P Perspective",
                "O Orthographic",
                "Click Zoom in",
                "B Card",
                "V Photo",
            ]);
        let state = TitleBarControlState::from_title_bar(&bar);
        for (width, height) in [(900.0, 800.0), (1440.0, 900.0), (1728.0, 1079.0)] {
            let (_, overflow) = visible_title_bar(
                &bar,
                &state,
                &fonts,
                &DiegeticTextMeasurer::default(),
                width,
                height,
            );
            assert_eq!(
                overflow.contains("B Card"),
                overflow.contains("V Photo"),
                "{width}×{height}"
            );
        }
        Ok(())
    }

    #[test]
    fn units_live_bar_fits_1440_window_with_complete_help() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new()
            .with_title("Units")
            .control("H Home")
            .controls(["D Outlines", "R Rulers", "F Fade", "↑ / ↓ Strength"])
            .control(
                TitleBarControl::segmented(
                    "A",
                    ["Off", "Anisotropic", "Supersample", "Both"]
                        .map(|word| TitleBarSegment::new(word, word)),
                )
                .with_name("Smoothing"),
            )
            .controls([
                "P Perspective",
                "O Orthographic",
                "Click Zoom in",
                "B Card",
                "V Photo",
            ]);
        let state = TitleBarControlState::from_title_bar(&bar);
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(fonts)
            .insert_resource(DiegeticTextMeasurer::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        super::super::install_title_bar_overflow_refresh(&mut app);
        app.init_resource::<HomeTitleBarFlash>();
        app.world_mut().spawn((
            Window {
                resolution: (1440, 900).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        let mut panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(build_title_bar_tree(
                &bar,
                &state,
                app.world().resource::<FontRegistry>(),
            ))
            .build()
            .map_err(|error| error.to_string())?;
        assert!(panel.set_screen_position(Vec2::splat(SCREEN_PANEL_WINDOW_INSET)));
        let entity = app
            .world_mut()
            .spawn((
                bar.clone(),
                state.clone(),
                TitleBarOverflow::default(),
                DrawnTitleBar::new(&bar, &state),
                panel,
                Visibility::Inherited,
            ))
            .id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("title bar did not solve")?;
        let root = solved.computed[0].bounds;
        let pixels_per_point = 1.0 / Unit::Pixels.to_points();
        let drawn = app
            .world()
            .get::<DrawnTitleBar>(entity)
            .ok_or("drawn bar missing")?;
        let predicted = title_bar_size(
            &drawn.visible,
            &state,
            app.world().resource::<FontRegistry>(),
            app.world().resource::<DiegeticTextMeasurer>(),
        );
        let drawn_width = root.width * pixels_per_point;
        assert!(
            (predicted.x - drawn_width).abs() <= 0.5,
            "fit measured {} px but drew {} px",
            predicted.x,
            drawn_width
        );
        assert!(
            2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, drawn_width) <= 1440.5,
            "drawn width is {drawn_width} px"
        );
        let help = solved
            .commands
            .iter()
            .find(|command| format!("{:?}", command.kind).starts_with("Text { text: \"? Help\","))
            .ok_or("complete ? Help is absent")?;
        assert!(help.bounds.x + help.bounds.width <= root.x + root.width + 0.5);
        Ok(())
    }

    #[test]
    fn named_segmented_control_draws_its_setting_name() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new().control(
            TitleBarControl::segmented("A", [TitleBarSegment::new("off", "Off")])
                .with_name("Smoothing"),
        );
        let tree = build_title_bar_tree(&bar, &TitleBarControlState::default(), &fonts);
        assert!(format!("{tree:?}").contains("A Smoothing:"));
        Ok(())
    }

    #[test]
    fn vertical_bar_keeps_insets_and_records_dropped_controls() -> Result<(), String> {
        assert_title_bar_fits_each_update(TitleBarOrientation::Vertical)
    }

    #[test]
    fn title_bar_refits_its_drawn_title_within_one_overflow_band() -> Result<(), String> {
        let title = "A very long title for the widget laboratory and its controls";
        let bar = TitleBar::new()
            .with_title(title)
            .control("H Home")
            .control("D Enable/disable");
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let state = TitleBarControlState::from_title_bar(&bar);
        let mut widths = Vec::new();
        for width in 180_u16..1800 {
            let (visible, overflow) =
                visible_title_bar(&bar, &state, &fonts, &measurer, f32::from(width), 500.0);
            if overflow.dropped.len() == 2
                && visible.title.len() > 3
                && widths
                    .last()
                    .is_none_or(|(_, last): &(u16, String)| *last != visible.title)
            {
                widths.push((width, visible.title));
            }
            if widths.len() == 3 {
                break;
            }
        }
        assert_eq!(
            widths.len(),
            3,
            "need three distinct fits in one overflow band"
        );

        let measure_calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&measure_calls);
        let measure = DiegeticTextMeasurer::default().measure_fn;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FontRegistry::new().map_err(|error| error.to_string())?)
            .insert_resource(DiegeticTextMeasurer {
                measure_fn: Arc::new(move |text, style| {
                    count.fetch_add(1, Ordering::Relaxed);
                    measure(text, style)
                }),
            })
            .insert_resource(Assets::<StandardMaterial>::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        super::super::install_title_bar_overflow_refresh(&mut app);
        app.init_resource::<HomeTitleBarFlash>();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(build_title_bar_tree(&bar, &state, &fonts))
            .build()
            .map_err(|error| error.to_string())?;
        let entity = app
            .world_mut()
            .spawn((
                bar.clone(),
                state,
                TitleBarOverflow::default(),
                DrawnTitleBar {
                    visible:       bar.clone(),
                    controls:      TitleBarControlState::from_title_bar(&bar),
                    window_width:  f32::MAX,
                    window_height: f32::MAX,
                },
                panel,
                Visibility::Inherited,
            ))
            .id();

        for (width, expected_title) in widths
            .into_iter()
            .chain(std::iter::once((1800, title.to_string())))
        {
            app.world_mut()
                .get_mut::<Window>(window)
                .ok_or("window missing")?
                .resolution
                .set(f32::from(width), 500.0);
            app.update();
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("title bar did not solve")?;
            let drawn = format!("{:?}", solved.commands);
            assert!(
                drawn.contains(&format!("text: {expected_title:?}")),
                "at {width}, expected {expected_title:?} in {drawn}"
            );
            let drawn_width = solved.computed[0].bounds.width / Unit::Pixels.to_points();
            assert!(
                2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, drawn_width) <= f32::from(width) + 0.5,
                "bar width {drawn_width} exceeds window width {width}"
            );
        }
        let before_idle = measure_calls.load(Ordering::Relaxed);
        app.update();
        assert_eq!(measure_calls.load(Ordering::Relaxed), before_idle);
        Ok(())
    }

    #[test]
    fn title_bar_with_controls_keeps_equal_visible_ink_margins() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        for orientation in [
            TitleBarOrientation::Horizontal,
            TitleBarOrientation::Vertical,
        ] {
            let title_bar = TitleBar::new()
                .with_title("Lab")
                .with_orientation(orientation)
                .control("G Control")
                .control(TitleBarControl::segmented(
                    "A",
                    [
                        TitleBarSegment::new("off", "Off"),
                        TitleBarSegment::new("on", "On"),
                    ],
                ));
            let tree = build_title_bar_tree(&title_bar, &TitleBarControlState::default(), &fonts);
            let panel = DiegeticPanel::screen()
                .size(Fit, Fit)
                .with_tree(tree)
                .build()
                .map_err(|error| error.to_string())?;
            let mut app = crate::test_app();
            app.add_plugins(MinimalPlugins)
                .insert_resource(DiegeticTextMeasurer::default())
                .add_plugins(HeadlessDiegeticUiPlugin);
            let entity = app.world_mut().spawn(panel).id();
            app.update();
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("title bar did not solve")?;
            let root = solved.computed[0].bounds;
            let point_per_pixel = Unit::Pixels.to_points();
            let font = fonts
                .font(FontId::MONOSPACE)
                .map_err(|error| error.to_string())?;
            let mut first_ink = f32::INFINITY;
            let mut last_ink = f32::NEG_INFINITY;
            let mut left_ink = f32::INFINITY;
            let mut right_ink = f32::NEG_INFINITY;
            let mut control_seen = false;
            let mut help_seen = false;
            for command in &solved.commands {
                let debug = format!("{:?}", command.kind);
                if !debug.starts_with("Text {") {
                    continue;
                }
                let is_title = debug.starts_with("Text { text: \"Lab\",");
                control_seen |= debug.starts_with("Text { text: \"G Control\",");
                help_seen |= debug.starts_with("Text { text: \"? Help\",");
                let size = super::super::integral_advance_size(
                    &fonts,
                    if is_title { TITLE_SIZE } else { LABEL_SIZE },
                );
                let metrics = font.metrics(size.0 * 4.0 / 3.0);
                let top_offset = metrics.line_gap.mul_add(0.5, metrics.ascent) - metrics.cap_height;
                let ink_top = top_offset.mul_add(point_per_pixel, command.bounds.y);
                let ink_bottom = metrics
                    .line_gap
                    .mul_add(0.5, metrics.descent)
                    .mul_add(-point_per_pixel, command.bounds.y + command.bounds.height);
                first_ink = first_ink.min(ink_top);
                last_ink = last_ink.max(ink_bottom);
                left_ink = left_ink.min(command.bounds.x);
                right_ink = right_ink.max(command.bounds.x + command.bounds.width);
            }
            assert!(control_seen, "real control was not measured");
            assert!(help_seen, "visible help hint was not rendered");
            let left_margin = left_ink - root.x;
            let right_margin = root.x + root.width - right_ink;
            let top_margin = first_ink - root.y;
            let bottom_margin = root.y + root.height - last_ink;
            assert!(
                (top_margin - left_margin).abs() < 2.25 && (top_margin - right_margin).abs() < 2.25,
                "{orientation:?} top {top_margin}, sides {left_margin}/{right_margin}",
            );
            assert!(
                (bottom_margin - left_margin).abs() < 2.25
                    && (bottom_margin - right_margin).abs() < 2.25,
                "{orientation:?} bottom {bottom_margin}, sides {left_margin}/{right_margin}",
            );
        }
        Ok(())
    }

    #[test]
    fn narrow_bar_keeps_title_and_help_and_overflows_controls() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let bar = TitleBar::new()
            .with_title("Widget Lab")
            .control("H Home")
            .control("D Enable/disable");
        let state = TitleBarControlState::from_title_bar(&bar);
        let (narrow, overflow) = visible_title_bar(&bar, &state, &fonts, &measurer, 400.0, 500.0);
        assert_eq!(narrow.controls.len(), 1);
        assert_eq!(narrow.controls[0].label, "H Home");
        assert!(!overflow.contains("H Home"));
        assert!(overflow.contains("D Enable/disable"));
        assert!(
            title_bar_size(&narrow, &state, &fonts, &measurer).x
                <= 2.0_f32.mul_add(-SCREEN_PANEL_WINDOW_INSET, 400.0)
        );
        let (wide, overflow) = visible_title_bar(&bar, &state, &fonts, &measurer, 1200.0, 500.0);
        assert_eq!(wide.controls.len(), 2);
        assert_eq!(overflow.dropped, Vec::<String>::new());
        let tree = build_title_bar_tree(&narrow, &state, &fonts);
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree)
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(measurer)
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("narrow bar did not solve")?;
        let width = solved.computed[0].bounds.width / Unit::Pixels.to_points();
        let left = super::super::separation::inset_anchor_top_left(
            bevy::prelude::Vec2::new(400.0, 500.0),
            bevy::prelude::Vec2::new(width, 50.0),
            Anchor::TopLeft,
        )
        .x;
        assert!(left >= SCREEN_PANEL_WINDOW_INSET - 0.5);
        assert!(left + width <= 400.0 - SCREEN_PANEL_WINDOW_INSET + 0.5);
        let commands = format!("{:?}", solved.commands);
        assert!(commands.contains("Widget Lab"));
        assert!(commands.contains("? Help"));
        assert!(commands.contains("H Home"));
        assert!(!commands.contains("D Enable/disable"));
        Ok(())
    }

    #[test]
    fn title_bar_stays_inside_both_window_insets_across_widths() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let mut cases = Vec::new();
        for title in [
            "Widget Lab",
            "A very long title for a widget example with many controls",
        ] {
            let bar = TitleBar::new()
                .with_title(title)
                .control("H Home")
                .control("D Enable/disable");
            let state = TitleBarControlState::from_title_bar(&bar);
            for width in (200_u16..=2600).step_by(20) {
                let width = f32::from(width);
                let (visible, overflow) =
                    visible_title_bar(&bar, &state, &fonts, &measurer, width, 500.0);
                let mut more = bar.clone();
                more.controls.truncate(visible.controls.len() + 1);
                if !overflow.dropped.is_empty() {
                    assert!(
                        title_bar_size(&more, &state, &fonts, &measurer).x
                            > 2.0_f32.mul_add(-SCREEN_PANEL_WINDOW_INSET, width),
                        "{title:?} at {width}: a dropped control would fit"
                    );
                }
                let tree = build_title_bar_tree(&visible, &state, &fonts);
                let panel = DiegeticPanel::screen()
                    .size(Fit, Fit)
                    .with_tree(tree)
                    .build()
                    .map_err(|error| error.to_string())?;
                let entity = app.world_mut().spawn(panel).id();
                let predicted_width = title_bar_size(&visible, &state, &fonts, &measurer).x;
                cases.push((entity, width, visible.title.clone(), predicted_width));
            }
        }
        app.update();
        for (entity, width, title, predicted_width) in cases {
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("title bar did not solve")?;
            let drawn_width = solved.computed[0].bounds.width / Unit::Pixels.to_points();
            assert!(
                (drawn_width - predicted_width).abs() <= 0.5,
                "{title:?} at {width}: predicted {predicted_width}, drawn {drawn_width}"
            );
            assert!(
                2.0_f32.mul_add(SCREEN_PANEL_WINDOW_INSET, drawn_width) <= width + 0.5,
                "{title:?} at {width}: drawn width {drawn_width}"
            );
            let text = format!("{:?}", solved.commands);
            assert!(text.contains("? Help"), "{title:?} at {width}: Help absent");
            assert!(text.contains(&title), "{title:?} at {width}: title absent");
        }
        Ok(())
    }

    #[test]
    fn narrow_long_title_ends_with_ellipsis_and_fits() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let measurer = DiegeticTextMeasurer::default();
        let bar = TitleBar::new()
            .with_title("A very long title for a widget example with many controls")
            .control("H Home")
            .control("D Enable/disable");
        let (visible, _) = visible_title_bar(
            &bar,
            &TitleBarControlState::default(),
            &fonts,
            &measurer,
            200.0,
            500.0,
        );
        assert!(visible.title.ends_with('…'), "{}", visible.title);
        assert!(
            title_bar_size(
                &visible,
                &TitleBarControlState::default(),
                &fonts,
                &measurer
            )
            .x <= 2.0_f32.mul_add(-SCREEN_PANEL_WINDOW_INSET, 200.0)
        );
        Ok(())
    }

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
    fn help_parts_joins_neutral_segment_separator_with_space() {
        let control = TitleBarControl::segmented(
            "Sweep:",
            [
                TitleBarSegment::new("behind", "B behind"),
                TitleBarSegment::new("front", "F in front"),
            ],
        )
        .with_segment_separator(",");

        let (keys, action) = control.help_parts();
        assert_eq!(keys, "Sweep:");
        assert_eq!(action, "B behind, F in front");
    }

    #[test]
    fn help_parts_joins_spacing_segments_with_single_space() {
        let control = TitleBarControl::segmented(
            "Sweep:",
            [
                TitleBarSegment::new("behind", "B behind"),
                TitleBarSegment::new("front", "F in front"),
            ],
        );

        let (keys, action) = control.help_parts();
        assert_eq!(keys, "Sweep:");
        assert_eq!(action, "B behind F in front");
    }

    #[test]
    fn segmented_separator_draws_neutral_and_touches_previous_segment() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let bar = TitleBar::new().with_title("").control(
            TitleBarControl::segmented(
                "Sweep:",
                [
                    TitleBarSegment::new("behind", "B behind"),
                    TitleBarSegment::new("front", "F in front"),
                ],
            )
            .with_segment_separator(","),
        );
        for active in ["behind", "front"] {
            let mut state = TitleBarControlState::default();
            state.set_active(active, ControlActivation::Active);
            let mut app = crate::test_app();
            app.add_plugins(MinimalPlugins)
                .insert_resource(DiegeticTextMeasurer::default())
                .add_plugins(HeadlessDiegeticUiPlugin);
            let panel = DiegeticPanel::screen()
                .size(Fit, Fit)
                .with_tree(build_title_bar_tree(&bar, &state, &fonts))
                .build()
                .map_err(|error| error.to_string())?;
            let entity = app.world_mut().spawn(panel).id();
            app.update();
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("segmented bar did not solve")?;
            let text = |label: &str| {
                solved.commands.iter().find(|command| {
                    format!("{:?}", command.kind).starts_with(&format!("Text {{ text: {label:?},"))
                })
            };
            let behind = text("B behind").ok_or("behind segment absent")?;
            let comma = text(",").ok_or("neutral comma absent")?;
            let front = text("F in front").ok_or("front segment absent")?;
            assert!(
                (behind.bounds.x + behind.bounds.width - comma.bounds.x).abs() <= 0.5,
                "comma detached from behind segment"
            );
            assert!(front.bounds.x >= comma.bounds.x + comma.bounds.width);
            let neutral = format!("{CONTROL_INACTIVE_COLOR:?}");
            let highlight = format!("{CONTROL_ACTIVE_COLOR:?}");
            assert!(format!("{:?}", comma.kind).contains(&neutral));
            let chosen = if active == "behind" { behind } else { front };
            assert!(format!("{:?}", chosen.kind).contains(&highlight));
        }
        Ok(())
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

        assert_eq!(
            control.disabled_note,
            DisabledNote::Explain("(no gamepad)".into())
        );
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
