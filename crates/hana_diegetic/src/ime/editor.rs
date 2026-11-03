//! Inline panel editing and screen fallback rendering for active IME sessions.

use std::borrow::Cow;

use bevy::math::Rect;
use bevy::prelude::Camera;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Event;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::ParamSet;
use bevy::prelude::PointerButton;
use bevy::prelude::PointerClick;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::warn;
use bevy::window::WindowRef;

use super::ActiveImeSession;
use super::ImeApplied;
use super::ImeAppliedResult;
use super::ImeBufferSnapshot;
use super::ImeCancelCause;
use super::ImeCanceled;
use super::ImeCommitCause;
use super::ImeCursorState;
use super::ImePreedit;
use super::ImePreeditBoundary;
use super::ImeRejection;
use super::ImeRequestCancel;
use super::ImeRequestCommit;
use super::ImeSelectionSnapshot;
use super::ImeSessionAnchor;
use super::ImeSessionId;
use super::ImeTarget;
use super::ImeTextChanged;
use super::ImeValidationRejected;
use super::buffer::ImeEditCommand;
use super::field::ImeBlurBehavior;
use super::field::ImeValidationPlacement;
use super::value_drag::FieldValueDrag;
use crate::AlignX;
use crate::AlignY;
use crate::Anchor;
use crate::Border;
use crate::BoundingBox;
use crate::ComputedDiegeticPanel;
use crate::ControlMetrics;
use crate::CornerRadius;
use crate::DiegeticPanel;
use crate::DiegeticPanelCommands;
use crate::DiegeticTextMeasurer;
use crate::El;
use crate::LayoutBuilder;
use crate::LayoutTree;
use crate::Padding;
use crate::PanelAnchorGeometryParam;
use crate::PanelAnchorPoints;
use crate::PanelElementId;
use crate::PanelFieldRecord;
use crate::PanelScreenBounds;
use crate::Px;
use crate::Sizing;
use crate::Text;
use crate::TextStyle;
use crate::Unit;
use crate::cascade::FontUnit;
use crate::cascade::PanelDefaults;
use crate::cascade::Resolved;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout::EditorPart;
use crate::layout::FieldDisplayTextUpdate;
use crate::panel::PanelFieldPresentation;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetKind;

const EDITOR_CAMERA_ORDER: isize = 120;
const DEFAULT_EDITOR_WIDTH: f32 = 180.0;
const DEFAULT_EDITOR_HEIGHT: f32 = 42.0;
const MIN_EDITOR_WIDTH: f32 = 72.0;
const MAX_EDITOR_WIDTH: f32 = 520.0;
const EDITOR_EXTRA_WIDTH: f32 = 0.0;
const EDITOR_FONT_SIZE: f32 = 16.0;
const EDITOR_PADDING_X: f32 = 10.0;
const EDITOR_PADDING_Y: f32 = 0.0;
const EDITOR_GAP: f32 = 3.0;
const CARET_WIDTH_TO_FONT_SIZE: f32 = 1.0 / EDITOR_FONT_SIZE;
const EDITOR_BORDER_WIDTH: f32 = 1.0;
const EDITOR_CORNER_RADIUS: f32 = 5.0;
const SOURCE_RECT_MIN_AXIS: f32 = 1.0;

const EDITOR_BACKGROUND: Color = Color::srgba(0.025, 0.028, 0.034, 0.96);
const EDITOR_BORDER: Color = Color::srgba(0.42, 0.72, 0.86, 0.92);
const EDITOR_TEXT: Color = Color::srgb(0.92, 0.94, 0.96);
const EDITOR_PREEDIT: Color = Color::srgb(0.70, 0.86, 1.0);
const EDITOR_SELECTION: Color = Color::srgba(0.18, 0.45, 0.64, 0.82);
const EDITOR_VALIDATION: Color = Color::srgb(1.0, 0.48, 0.40);
const EDITOR_CARET: Color = Color::srgb(0.86, 0.96, 1.0);

/// Field projection captured from panel picking before the session id exists.
#[derive(Resource, Clone, Debug, Default)]
pub(super) struct PendingImePanelAnchor {
    pending:      Option<ImePanelAnchorSource>,
    pointer_edit: Option<(ImeTarget, Entity, ImeEditCommand)>,
}

impl PendingImePanelAnchor {
    pub(super) fn store(
        &mut self,
        panel: Entity,
        field_id: PanelElementId,
        camera: Option<Entity>,
        window: Entity,
        bounds: BoundingBox,
        presentation: PanelFieldPresentation,
    ) {
        self.pending = Some(ImePanelAnchorSource {
            panel,
            field_id,
            camera,
            window,
            bounds,
            presentation,
        });
    }

    pub(super) fn set_pointer_edit(
        &mut self,
        target: ImeTarget,
        window: Entity,
        command: ImeEditCommand,
    ) {
        self.pointer_edit = Some((target, window, command));
    }

    pub(super) fn take_pointer_edit(
        &mut self,
        target: &ImeTarget,
        window: Entity,
    ) -> Option<ImeEditCommand> {
        let (pending_target, pending_window, _) = self.pointer_edit.as_ref()?;
        if pending_target != target || *pending_window != window {
            return None;
        }
        self.pointer_edit.take().map(|(_, _, command)| command)
    }

    fn take_for(&mut self, target: &ImeTarget, window: Entity) -> Option<ImePanelAnchorSource> {
        let pending = self.pending.as_ref()?;
        if pending.window != window || !pending.matches_target(target) {
            return None;
        }
        self.pending.take()
    }
}

#[derive(Clone, Debug)]
struct ImePanelAnchorSource {
    panel:        Entity,
    field_id:     PanelElementId,
    camera:       Option<Entity>,
    window:       Entity,
    bounds:       BoundingBox,
    presentation: PanelFieldPresentation,
}

impl ImePanelAnchorSource {
    fn matches_target(&self, target: &ImeTarget) -> bool {
        match target {
            ImeTarget::WorldPanelField { panel, field_id }
            | ImeTarget::ScreenPanelField { panel, field_id } => {
                self.panel == *panel && self.field_id == *field_id
            },
            ImeTarget::AppOwned { .. } => false,
        }
    }
}

/// Active IME presentation state.
///
/// Panel fields keep an authoritative copy of the panel tree and render the
/// live buffer inside that field. App-owned sessions retain the standalone
/// screen editor because they have no panel field to replace.
#[derive(Resource, Debug, Default)]
pub(crate) struct ImeEditorState {
    active: Option<ImeEditor>,
}

impl ImeEditorState {
    const fn active(&self) -> Option<&ImeEditor> { self.active.as_ref() }

    const fn active_mut(&mut self) -> Option<&mut ImeEditor> { self.active.as_mut() }

    fn session_id(&self) -> Option<ImeSessionId> {
        self.active.as_ref().map(|editor| editor.session_id)
    }

    pub(super) fn is_overlay_panel(&self, entity: Entity) -> bool {
        self.active.as_ref().is_some_and(|editor| {
            matches!(editor.surface, ImeEditorSurface::ScreenOverlay { panel } if panel == entity)
        })
    }

    fn cancel(&mut self, commands: &mut Commands) {
        if let Some(editor) = self.active.take() {
            match editor.surface {
                ImeEditorSurface::Inline {
                    panel,
                    authoritative_tree,
                    ..
                } => {
                    if let Err(error) = commands.set_tree(panel, authoritative_tree) {
                        warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to restore the IME source panel: {error}");
                    }
                },
                ImeEditorSurface::ScreenOverlay { panel } => {
                    commands.entity(panel).despawn();
                },
            }
        }
    }

    fn finish(&mut self, result: &ImeAppliedResult, commands: &mut Commands) {
        let Some(editor) = self.active.take() else {
            return;
        };
        match editor.surface {
            ImeEditorSurface::Inline {
                panel,
                mut authoritative_tree,
                ..
            } => {
                let ImeAppliedResult::AppOwned { display_text, .. } = result else {
                    return;
                };
                if let Some(display_text) = display_text {
                    let _ = authoritative_tree
                        .set_field_display_text(target_field_id(&editor.target), display_text);
                }
                if let Err(error) = commands.set_tree(panel, authoritative_tree) {
                    warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to finish the app-owned IME source panel: {error}");
                }
            },
            ImeEditorSurface::ScreenOverlay { panel } => {
                commands.entity(panel).despawn();
            },
        }
    }

    pub(crate) fn authoritative_tree(
        &self,
        session_id: ImeSessionId,
        panel: Entity,
    ) -> Option<&LayoutTree> {
        let editor = self.active.as_ref()?;
        if editor.session_id != session_id {
            return None;
        }
        match &editor.surface {
            ImeEditorSurface::Inline {
                panel: source_panel,
                authoritative_tree,
                ..
            } if *source_panel == panel => Some(authoritative_tree),
            ImeEditorSurface::Inline { .. } | ImeEditorSurface::ScreenOverlay { .. } => None,
        }
    }
}

#[derive(Debug)]
struct ImeEditor {
    session_id:   ImeSessionId,
    target:       ImeTarget,
    window:       Entity,
    snapshot:     ImeBufferSnapshot,
    validation:   ImeEditorValidation,
    surface:      ImeEditorSurface,
    source:       Option<ImePanelAnchorSource>,
    app_anchor:   Option<ImeSessionAnchor>,
    anchor:       Option<ImeEditorAnchor>,
    presentation: Option<ImeEditorPresentation>,
}

/// Whether the app has rejected the text currently in the editor.
///
/// Acceptance is a state of its own, not a missing message: the buffer stands as typed and the
/// editor shows no validation line. A rejection always carries the message shown to you, so no
/// reader has to decide what an empty one would mean.
#[derive(Clone, Debug, PartialEq)]
enum ImeEditorValidation {
    /// Nothing in this buffer has been rejected, so the editor shows no validation line.
    Accepted,
    /// The app rejected this buffer with the message the editor shows.
    Rejected(String),
}

/// Whether one editor element's appearance was authored by the panel or generated by the editor.
///
/// A panel may declare the style of the buffer text, the selection highlight, the caret, or the
/// validation line. Declaring nothing is the panel's own decision, not a missing value: the
/// editor builds that element itself and marks it generated so recoloring can find it again.
#[derive(Clone, Debug, PartialEq)]
enum EditorPartDeclaration {
    /// The panel authored this element's appearance.
    ///
    /// Boxed: an `EditorPart` is several hundred bytes and a presentation carries four of these.
    Authored(Box<EditorPart>),
    /// The panel authored nothing, so the editor generates the element.
    Generated,
}

impl EditorPartDeclaration {
    /// Convert one authored style attribute, which the panel API leaves optional.
    fn from_authored(authored: Option<EditorPart>) -> Self {
        authored.map_or(Self::Generated, |part| Self::Authored(Box::new(part)))
    }
}

#[derive(Debug)]
enum ImeEditorSurface {
    /// The active buffer replaces only the field descendants in this panel.
    Inline {
        panel:              Entity,
        authoritative_tree: LayoutTree,
        panel_font_unit:    Unit,
    },
    /// Standalone fallback for an app-owned target without panel geometry.
    ScreenOverlay { panel: Entity },
}

impl ImeEditorSurface {
    const fn is_inline(&self) -> bool { matches!(self, Self::Inline { .. }) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ImeEditorAnchor {
    screen_rect: Rect,
    editor_pos:  Vec2,
    editor_size: Vec2,
    caret_pos:   Vec2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TextHit {
    /// Nearest insertion boundary to the pointer.
    pub(super) insertion: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct ImeEditorPresentation {
    background:        Option<Color>,
    border:            Option<Border>,
    corner_radius:     CornerRadius,
    padding:           Padding,
    align_x:           AlignX,
    align_y:           AlignY,
    text_style:        TextStyle,
    editor_text:       EditorPartDeclaration,
    editor_selection:  EditorPartDeclaration,
    editor_caret:      EditorPartDeclaration,
    editor_validation: EditorPartDeclaration,
}

/// Last picked entity classified as outside the active editor.
#[derive(Resource, Debug, Default)]
pub(crate) struct ImeBlurIntent {
    latest: Option<ImeBlurClassification>,
}

impl ImeBlurIntent {
    const fn set(&mut self, session_id: ImeSessionId, clicked_entity: Entity) {
        self.latest = Some(ImeBlurClassification {
            session_id,
            clicked_entity,
        });
    }

    fn clear_session(&mut self, session_id: ImeSessionId) {
        if self
            .latest
            .as_ref()
            .is_some_and(|intent| intent.session_id == session_id)
        {
            self.latest = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImeBlurClassification {
    session_id:     ImeSessionId,
    clicked_entity: Entity,
}

/// Marker on the transient screen editor used by app-owned sessions.
#[derive(Component, Debug)]
struct ImeEditorPanel;

/// Validation feedback beside an inline field, owned by that editing session.
#[derive(Component, Debug)]
pub(super) struct InlineImeValidationPanel {
    session_id: ImeSessionId,
}

/// Hands a rebuilt panel tree to whichever inline IME session is editing a
/// field inside that panel.
///
/// An application that re-renders a panel while one of its fields is being
/// edited — a command palette filtering its rows as the reader types — must
/// publish the new tree through this event rather than through
/// [`DiegeticPanelCommands::set_tree`](crate::DiegeticPanelCommands::set_tree).
/// The editor holds its own copy of the panel tree and rewrites the field's
/// descendants into it to draw the caret and the selection; a direct `set_tree`
/// from the application lands after that rewrite and erases both. Sending the
/// tree here replaces the copy the editor rewrites, so the application's
/// content and the editor's caret survive together.
///
/// With no session editing that panel the event is an ordinary tree
/// replacement, so a caller never has to ask whether a session is open.
#[derive(Clone, Debug, Event)]
pub struct ImeReplacePanelTree {
    /// The panel whose tree is replaced.
    pub panel: Entity,
    /// The application's new tree for that panel, without any editing content.
    pub tree:  LayoutTree,
}

pub(super) fn replace_panel_tree(
    event: On<ImeReplacePanelTree>,
    mut editor_state: ResMut<ImeEditorState>,
    mut commands: Commands,
) {
    let event = event.event();
    let edits_this_panel = editor_state.active().is_some_and(|editor| {
        matches!(editor.surface, ImeEditorSurface::Inline { panel, .. } if panel == event.panel)
    });
    if !edits_this_panel {
        if let Err(error) = commands.set_tree(event.panel, event.tree.clone()) {
            warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to replace an unedited panel's tree: {error}");
        }
        return;
    }

    let Some(editor) = editor_state.active_mut() else {
        return;
    };
    if let ImeEditorSurface::Inline {
        authoritative_tree, ..
    } = &mut editor.surface
    {
        *authoritative_tree = event.tree.clone();
    }
    let Some(editor) = editor_state.active() else {
        return;
    };
    update_editor_tree(editor, &mut commands);
}

pub(super) fn update_editor_from_text_changed(
    event: On<ImeTextChanged>,
    active_session: Res<ActiveImeSession>,
    value_drag: Res<FieldValueDrag>,
    defaults: Option<Res<PanelDefaults>>,
    mut pending_anchor: ResMut<PendingImePanelAnchor>,
    mut editor_state: ResMut<ImeEditorState>,
    mut blur_intent: ResMut<ImeBlurIntent>,
    mut panel_queries: ParamSet<(
        Query<(&DiegeticPanel, Option<&Resolved<FontUnit>>)>,
        Query<(Entity, &InlineImeValidationPanel)>,
    )>,
    mut commands: Commands,
) {
    let event = event.event();
    if value_drag.suppresses_editor(&event.target) {
        return;
    }
    let Some(window) = active_session.active_window() else {
        return;
    };
    let source = pending_anchor.take_for(&event.target, window);
    let app_anchor = active_session.active_anchor();

    let needs_spawn = editor_state
        .active()
        .is_none_or(|editor| editor.session_id != event.session_id);
    if needs_spawn {
        editor_state.cancel(&mut commands);
        let default_font_unit = defaults
            .as_deref()
            .copied()
            .unwrap_or_default()
            .panel_font_unit;
        let Some(surface) = create_editor_surface(
            &event.target,
            window,
            &event.snapshot,
            default_font_unit,
            &panel_queries.p0(),
            &mut commands,
        ) else {
            return;
        };
        editor_state.active = Some(ImeEditor {
            session_id: event.session_id,
            target: event.target.clone(),
            window,
            snapshot: event.snapshot.clone(),
            validation: ImeEditorValidation::Accepted,
            surface,
            source,
            app_anchor,
            anchor: None,
            presentation: None,
        });
    } else if let Some(editor) = editor_state.active_mut() {
        if source.is_some() {
            editor.source = source;
        }
        editor.target = event.target.clone();
        editor.window = window;
        editor.snapshot = event.snapshot.clone();
        editor.app_anchor = app_anchor;
        editor.validation = ImeEditorValidation::Accepted;
    }

    clear_inline_validation(event.session_id, &panel_queries.p1(), &mut commands);
    blur_intent.clear_session(event.session_id);
    if let Some(editor) = editor_state.active() {
        update_editor_tree(editor, &mut commands);
    }
}

pub(super) fn update_editor_validation(
    event: On<ImeValidationRejected>,
    mut editor_state: ResMut<ImeEditorState>,
    validation_panels: Query<(Entity, &InlineImeValidationPanel)>,
    mut commands: Commands,
) {
    let event = event.event();
    let Some(editor) = editor_state.active_mut() else {
        return;
    };
    if editor.session_id != event.session_id {
        return;
    }

    clear_inline_validation(event.session_id, &validation_panels, &mut commands);
    editor.validation = ImeEditorValidation::Rejected(match &event.reason {
        ImeRejection::StaleAttempt => String::from("This edit is no longer current."),
        ImeRejection::InvalidText(message)
        | ImeRejection::OutOfRange(message)
        | ImeRejection::AppOwned(message) => message.clone(),
    });
    update_editor_tree(editor, &mut commands);
}

pub(super) fn close_editor_on_cancel(
    event: On<ImeCanceled>,
    mut editor_state: ResMut<ImeEditorState>,
    mut blur_intent: ResMut<ImeBlurIntent>,
    validation_panels: Query<(Entity, &InlineImeValidationPanel)>,
    mut commands: Commands,
) {
    let session_id = event.event().session_id;
    clear_inline_validation(session_id, &validation_panels, &mut commands);
    if editor_state.session_id() != Some(session_id) {
        return;
    }
    blur_intent.clear_session(session_id);
    editor_state.cancel(&mut commands);
}

pub(super) fn close_editor_on_apply(
    event: On<ImeApplied>,
    mut editor_state: ResMut<ImeEditorState>,
    mut blur_intent: ResMut<ImeBlurIntent>,
    validation_panels: Query<(Entity, &InlineImeValidationPanel)>,
    mut commands: Commands,
) {
    let event = event.event();
    let session_id = event.session_id;
    clear_inline_validation(session_id, &validation_panels, &mut commands);
    if editor_state.session_id() != Some(session_id) {
        return;
    }
    blur_intent.clear_session(session_id);
    editor_state.finish(&event.result, &mut commands);
}

pub(super) fn update_editor_anchor(
    mut editor_state: ResMut<ImeEditorState>,
    measurer: Res<DiegeticTextMeasurer>,
    mut panel_queries: ParamSet<(
        Query<&mut DiegeticPanel>,
        Query<(&DiegeticPanel, &ComputedDiegeticPanel, &GlobalTransform)>,
        PanelAnchorGeometryParam,
    )>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    windows: Query<&Window>,
    validation_panels: Query<(Entity, &InlineImeValidationPanel)>,
    mut commands: Commands,
) {
    let Some(editor) = editor_state.active_mut() else {
        return;
    };
    let Ok(window) = windows.get(editor.window) else {
        return;
    };

    let screen_rect = target_screen_rect(editor, &mut panel_queries, &cameras, window);
    let Some(screen_rect) = screen_rect else {
        commands.trigger(ImeRequestCancel {
            session_id: editor.session_id,
            cause:      ImeCancelCause::TargetStale,
        });
        return;
    };

    let presentation = editor
        .source
        .as_ref()
        .map(|source| projected_editor_presentation(source, screen_rect));
    let presentation_changed = editor.presentation != presentation;
    if presentation_changed {
        editor.presentation = presentation;
        if !editor.surface.is_inline() {
            update_editor_tree(editor, &mut commands);
        }
    }

    let (editor_pos, editor_size) = match &editor.surface {
        ImeEditorSurface::Inline { .. } => (screen_rect.min, screen_rect.size()),
        ImeEditorSurface::ScreenOverlay { panel } => {
            let editor_size = editor_size(screen_rect);
            let editor_pos = clamp_editor_position(screen_rect.min, editor_size, window);
            let mut panels = panel_queries.p0();
            let Ok(mut panel) = panels.get_mut(*panel) else {
                return;
            };
            let _ = panel.set_size((Px(editor_size.x), Px(editor_size.y)));
            let _ = panel.set_screen_position(editor_pos);
            (editor_pos, editor_size)
        },
    };
    let fallback = default_editor_presentation();
    let presentation = editor.presentation.as_ref().unwrap_or(&fallback);
    let horizontal_scroll = match &editor.target {
        ImeTarget::WorldPanelField { panel, field_id }
        | ImeTarget::ScreenPanelField { panel, field_id } => {
            let panels = panel_queries.p1();
            panels
                .get(*panel)
                .ok()
                .and_then(|(_, computed, _)| {
                    let field = field_record(computed, field_id)?;
                    let solved = computed
                        .layout()
                        .solved()?
                        .scrolls
                        .iter()
                        .find(|solved| solved.element_index == field.element_index)?;
                    (field.bounds.width > 0.0).then_some(
                        solved.offset_points.x * screen_rect.width() / field.bounds.width,
                    )
                })
                .unwrap_or(0.0)
        },
        ImeTarget::AppOwned { .. } => 0.0,
    };
    let caret_pos = caret_position(
        editor_pos,
        editor_size,
        &editor.snapshot,
        &measurer,
        presentation,
        horizontal_scroll,
    );
    editor.anchor = Some(ImeEditorAnchor {
        screen_rect,
        editor_pos,
        editor_size,
        caret_pos,
    });
    sync_inline_validation(
        editor,
        window,
        &measurer,
        &validation_panels,
        &mut panel_queries.p0(),
        presentation_changed,
        &mut commands,
    );
}

fn clear_inline_validation(
    session_id: ImeSessionId,
    panels: &Query<(Entity, &InlineImeValidationPanel)>,
    commands: &mut Commands,
) {
    for (panel, validation) in panels {
        if validation.session_id == session_id {
            commands.entity(panel).despawn();
        }
    }
}

fn sync_inline_validation(
    editor: &ImeEditor,
    window: &Window,
    measurer: &DiegeticTextMeasurer,
    validation_panels: &Query<(Entity, &InlineImeValidationPanel)>,
    panels: &mut Query<&mut DiegeticPanel>,
    presentation_changed: bool,
    commands: &mut Commands,
) {
    let ImeEditorValidation::Rejected(message) = &editor.validation else {
        return;
    };
    let ImeEditorSurface::Inline {
        authoritative_tree, ..
    } = &editor.surface
    else {
        return;
    };
    if validation_layout_anchor(authoritative_tree, target_field_id(&editor.target)).is_some()
        || authoritative_tree.field_can_grow_for_validation(target_field_id(&editor.target))
    {
        return;
    }
    let Some(anchor) = editor.anchor else {
        return;
    };
    let fallback = default_editor_presentation();
    let presentation = editor.presentation.as_ref().unwrap_or(&fallback);
    let mut style = presentation
        .text_style
        .clone()
        .with_color(EDITOR_VALIDATION);
    style.set_dimension(Px(style.size()));
    let feedback_dimensions = (measurer.measure_fn)(message, &style.as_measure());
    let available_width = window.width().clamp(1.0, MAX_EDITOR_WIDTH);
    let size = Vec2::new(
        (feedback_dimensions.width + 2.0)
            .ceil()
            .clamp(MIN_EDITOR_WIDTH.min(available_width), available_width),
        (feedback_dimensions.height + 2.0).ceil(),
    );
    let below = anchor.screen_rect.max.y + EDITOR_GAP;
    let y = if below + size.y <= window.height() {
        below
    } else {
        anchor.screen_rect.min.y - EDITOR_GAP - size.y
    };
    let position = clamp_editor_position(Vec2::new(anchor.screen_rect.min.x, y), size, window);
    let mut retained = false;
    for (panel, validation) in validation_panels {
        if validation.session_id != editor.session_id {
            continue;
        }
        if presentation_changed {
            commands.entity(panel).despawn();
        } else if let Ok(mut panel) = panels.get_mut(panel) {
            let _ = panel.set_screen_position(position);
            retained = true;
        }
    }
    if retained {
        return;
    }
    let mut builder = LayoutBuilder::new(size.x, size.y);
    add_text(
        &mut builder,
        message,
        &style,
        &presentation.editor_validation,
    );
    let panel = DiegeticPanel::screen()
        .size(Px(size.x), Px(size.y))
        .font_unit(Unit::Pixels)
        .anchor(Anchor::TopLeft)
        .screen_position(position.x, position.y)
        .camera_order(EDITOR_CAMERA_ORDER)
        .window(WindowRef::Entity(editor.window))
        .with_tree(builder.build())
        .build();
    match panel {
        Ok(panel) => {
            commands.spawn((
                panel,
                InlineImeValidationPanel {
                    session_id: editor.session_id,
                },
            ));
        },
        Err(error) => {
            warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to build inline IME validation feedback: {error}");
        },
    }
}

pub(super) fn update_window_ime_position(
    editor_state: Res<ImeEditorState>,
    mut windows: Query<&mut Window>,
) {
    let Some(editor) = editor_state.active() else {
        return;
    };
    let Some(anchor) = editor.anchor else {
        return;
    };
    let Ok(mut window) = windows.get_mut(editor.window) else {
        return;
    };
    window.ime_position = anchor.caret_pos;
}

pub(crate) fn handle_blur_intent(
    mut blur_intent: ResMut<ImeBlurIntent>,
    active_session: Res<ActiveImeSession>,
    mut commands: Commands,
) {
    let Some(intent) = blur_intent.latest.take() else {
        return;
    };
    if active_session.active_session_id() != Some(intent.session_id) {
        return;
    }
    if active_session.is_pending_commit() {
        return;
    }

    let Some(target) = active_session.active_target() else {
        return;
    };
    if intent.is_inside_focus_scope(target) {
        return;
    }

    match active_session.blur_behavior() {
        ImeBlurBehavior::Cancel => commands.trigger(ImeRequestCancel {
            session_id: intent.session_id,
            cause:      ImeCancelCause::Blur,
        }),
        ImeBlurBehavior::Commit => commands.trigger(ImeRequestCommit {
            session_id: intent.session_id,
            cause:      ImeCommitCause::Blur,
        }),
        ImeBlurBehavior::KeepEditing => {},
    }
}

pub(super) fn classify_non_panel_click(
    click: On<PointerClick>,
    panels: Query<(), With<DiegeticPanel>>,
    editable_widgets: Query<&WidgetKind, With<PanelWidget>>,
    editor_state: Res<ImeEditorState>,
    mut blur_intent: ResMut<ImeBlurIntent>,
) {
    if click.button != PointerButton::Primary
        || panels.contains(click.event_target())
        || editor_state.session_id().is_none()
    {
        return;
    }
    if editable_widgets
        .get(click.event_target())
        .is_ok_and(|kind| *kind == WidgetKind::EditableField)
    {
        return;
    }
    classify_widget_click(click.event_target(), &editor_state, &mut blur_intent);
}

pub(crate) fn classify_widget_click(
    clicked_panel: Entity,
    editor_state: &ImeEditorState,
    blur_intent: &mut ImeBlurIntent,
) {
    let Some(session_id) = editor_state.session_id() else {
        return;
    };
    blur_intent.set(session_id, clicked_panel);
}

pub(super) const fn pointer_command(click_count: u8, text_hit: TextHit) -> ImeEditCommand {
    match click_count {
        1 => ImeEditCommand::PlaceCursor(text_hit.insertion),
        _ => ImeEditCommand::SelectAll,
    }
}

fn create_editor_surface(
    target: &ImeTarget,
    window: Entity,
    snapshot: &ImeBufferSnapshot,
    default_font_unit: Unit,
    panels: &Query<(&DiegeticPanel, Option<&Resolved<FontUnit>>)>,
    commands: &mut Commands,
) -> Option<ImeEditorSurface> {
    match target {
        ImeTarget::WorldPanelField { panel, .. } | ImeTarget::ScreenPanelField { panel, .. } => {
            let (source, font_unit) = panels.get(*panel).ok()?;
            let authoritative_tree = source.tree().clone();
            let panel_font_unit = font_unit.map_or(default_font_unit, |resolved| resolved.0.0);
            Some(ImeEditorSurface::Inline {
                panel: *panel,
                authoritative_tree,
                panel_font_unit,
            })
        },
        ImeTarget::AppOwned { .. } => {
            let panel =
                spawn_editor_panel(window, snapshot, &ImeEditorValidation::Accepted, commands)?;
            Some(ImeEditorSurface::ScreenOverlay { panel })
        },
    }
}

fn update_editor_tree(editor: &ImeEditor, commands: &mut Commands) {
    let (panel, tree) = match &editor.surface {
        ImeEditorSurface::Inline {
            panel,
            authoritative_tree,
            panel_font_unit,
        } => {
            let Some(authored) =
                authoritative_tree.editable_field_presentation_for(target_field_id(&editor.target))
            else {
                warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to find the IME field's authored presentation");
                return;
            };
            let presentation = inline_editor_presentation(&authored);
            // A fixed field keeps its frame; validation is drawn beside it.
            // Flexible fields retain their authored inline validation parts.
            let validation_anchor =
                validation_layout_anchor(authoritative_tree, target_field_id(&editor.target));
            let validation = if validation_anchor.is_none()
                && authoritative_tree.field_can_grow_for_validation(target_field_id(&editor.target))
            {
                &editor.validation
            } else {
                &ImeEditorValidation::Accepted
            };
            let replacement =
                inline_editor_content_tree(&editor.snapshot, validation, &presentation);
            let mut tree = authoritative_tree.clone();
            let update =
                tree.set_field_editing_content(target_field_id(&editor.target), &replacement);
            if update != FieldDisplayTextUpdate::Updated {
                warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to find the IME field while updating inline content: {update:?}");
                return;
            }
            if let (Some(anchor), ImeEditorValidation::Rejected(message)) =
                (validation_anchor, &editor.validation)
            {
                let feedback = inline_validation_tree(message, &presentation, *panel_font_unit);
                if !tree.wrap_element_with_feedback(anchor, &feedback) {
                    warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to reserve inline IME validation after {anchor}");
                }
            }
            (*panel, tree)
        },
        ImeEditorSurface::ScreenOverlay { panel } => (
            *panel,
            editor_tree(
                &editor.snapshot,
                &editor.validation,
                editor.presentation.as_ref(),
            ),
        ),
    };
    if let Err(error) = commands.set_tree(panel, tree) {
        warn!(target: DIAGNOSTIC_LOG_TARGET, "failed to update IME editor content: {error}");
    }
}

fn validation_layout_anchor<'tree>(
    tree: &'tree LayoutTree,
    field_id: &PanelElementId,
) -> Option<&'tree PanelElementId> {
    let ImeValidationPlacement::AfterElement(anchor) =
        tree.field_spec_for(field_id)?.validation_placement()
    else {
        return None;
    };
    (0..tree.len())
        .any(|index| tree.element_id(index) == Some(anchor))
        .then_some(anchor)
}

fn inline_validation_tree(
    message: &str,
    presentation: &ImeEditorPresentation,
    panel_font_unit: Unit,
) -> LayoutTree {
    let step = ControlMetrics::from_label_size(presentation.text_style.size())
        .spacing
        .step();
    let step = crate::Dimension {
        value: step,
        unit:  Some(presentation.text_style.unit().resolve(panel_font_unit)),
    };
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(step)
            .padding(Padding::new(0.0, 0.0, 0.0, step)),
    );
    let style = presentation
        .text_style
        .clone()
        .with_color(EDITOR_VALIDATION);
    add_text(
        &mut builder,
        message,
        &style,
        &presentation.editor_validation,
    );
    builder.build()
}

const fn target_field_id(target: &ImeTarget) -> &PanelElementId {
    match target {
        ImeTarget::WorldPanelField { field_id, .. }
        | ImeTarget::ScreenPanelField { field_id, .. }
        | ImeTarget::AppOwned { field_id, .. } => field_id,
    }
}

fn spawn_editor_panel(
    window: Entity,
    snapshot: &ImeBufferSnapshot,
    validation: &ImeEditorValidation,
    commands: &mut Commands,
) -> Option<Entity> {
    let panel = match DiegeticPanel::screen()
        .size(Px(DEFAULT_EDITOR_WIDTH), Px(DEFAULT_EDITOR_HEIGHT))
        .anchor(Anchor::TopLeft)
        .screen_position(0.0, 0.0)
        .camera_order(EDITOR_CAMERA_ORDER)
        .window(WindowRef::Entity(window))
        .with_tree(editor_tree(snapshot, validation, None))
        .build()
    {
        Ok(panel) => panel,
        Err(error) => {
            bevy::log::error!(
                target: DIAGNOSTIC_LOG_TARGET,
                "failed to build IME editor panel: {error:?}"
            );
            return None;
        },
    };

    Some(
        commands
            .spawn((ImeEditorPanel, panel, Transform::default()))
            .id(),
    )
}

fn target_screen_rect(
    editor: &ImeEditor,
    panel_queries: &mut ParamSet<(
        Query<&mut DiegeticPanel>,
        Query<(&DiegeticPanel, &ComputedDiegeticPanel, &GlobalTransform)>,
        PanelAnchorGeometryParam,
    )>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
    window: &Window,
) -> Option<Rect> {
    match &editor.target {
        ImeTarget::WorldPanelField { panel, field_id } => {
            let panels = panel_queries.p1();
            let (panel, computed, panel_transform) = panels.get(*panel).ok()?;
            let record = field_record(computed, field_id)?;
            let source = editor.source.as_ref()?;
            let (camera, camera_transform) = cameras.get(source.camera?).ok()?;
            project_field_record(record, panel, panel_transform, camera, camera_transform)
        },
        ImeTarget::ScreenPanelField { panel, field_id } => {
            let (points_to_world, record) = {
                let panels = panel_queries.p1();
                let (panel_data, computed, _) = panels.get(*panel).ok()?;
                (
                    panel_data.points_to_world(),
                    field_record(computed, field_id)?.clone(),
                )
            };
            let geometry = panel_queries.p2().get(*panel).ok()?;
            let PanelAnchorPoints::Screen { bounds, .. } = *geometry.points() else {
                return None;
            };
            screen_field_record_rect(&record, points_to_world, bounds)
        },
        ImeTarget::AppOwned { .. } => Some(app_anchor_rect(editor.app_anchor, window)),
    }
}

impl ImeBlurClassification {
    fn is_inside_focus_scope(&self, target: &ImeTarget) -> bool {
        matches!(target, ImeTarget::AppOwned { owner, .. } if *owner == self.clicked_entity)
    }
}

fn field_record<'a>(
    computed: &'a ComputedDiegeticPanel,
    field_id: &PanelElementId,
) -> Option<&'a PanelFieldRecord> {
    computed
        .field_records()
        .iter()
        .find(|record| !record.duplicate_id && record.field_id == *field_id)
}

fn project_field_record(
    record: &PanelFieldRecord,
    panel: &DiegeticPanel,
    panel_transform: &GlobalTransform,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> Option<Rect> {
    let corners = panel_local_corners(record.bounds, panel);
    let mut points = Vec::with_capacity(corners.len());
    for corner in corners {
        let world = panel_transform.transform_point(corner);
        let viewport = camera.world_to_viewport(camera_transform, world).ok()?;
        points.push(viewport);
    }
    rect_from_points(&points)
}

fn screen_field_record_rect(
    record: &PanelFieldRecord,
    points_to_world: f32,
    bounds: PanelScreenBounds,
) -> Option<Rect> {
    let min = bounds.top_left()
        + Vec2::new(
            record.bounds.x * points_to_world,
            record.bounds.y * points_to_world,
        );
    let max = min
        + Vec2::new(
            record.bounds.width * points_to_world,
            record.bounds.height * points_to_world,
        );
    rect_from_points(&[min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)])
}

fn panel_local_corners(bounds: BoundingBox, panel: &DiegeticPanel) -> [Vec3; 4] {
    let points_to_world = panel.points_to_world();
    let (anchor_x, anchor_y) = panel.anchor_offsets();
    let left = bounds.x.mul_add(points_to_world, -anchor_x);
    let right = (bounds.x + bounds.width).mul_add(points_to_world, -anchor_x);
    let top = (-bounds.y).mul_add(points_to_world, anchor_y);
    let bottom = (-(bounds.y + bounds.height)).mul_add(points_to_world, anchor_y);
    [
        Vec3::new(left, top, 0.0),
        Vec3::new(right, top, 0.0),
        Vec3::new(right, bottom, 0.0),
        Vec3::new(left, bottom, 0.0),
    ]
}

fn rect_from_points(points: &[Vec2]) -> Option<Rect> {
    let first = *points.first()?;
    let mut min = first;
    let mut max = first;
    for point in points.iter().copied().skip(1) {
        if !point.is_finite() {
            return None;
        }
        min = min.min(point);
        max = max.max(point);
    }
    if max.x - min.x < SOURCE_RECT_MIN_AXIS || max.y - min.y < SOURCE_RECT_MIN_AXIS {
        return None;
    }
    Some(Rect { min, max })
}

fn fallback_screen_rect(window: &Window) -> Rect {
    let origin = window.cursor_position().unwrap_or(Vec2::ZERO);
    Rect {
        min: origin,
        max: origin + Vec2::new(DEFAULT_EDITOR_WIDTH, DEFAULT_EDITOR_HEIGHT),
    }
}

fn app_anchor_rect(anchor: Option<ImeSessionAnchor>, window: &Window) -> Rect {
    match anchor {
        Some(ImeSessionAnchor::ScreenRect(rect)) => rect,
        Some(ImeSessionAnchor::ScreenPoint(point)) => Rect {
            min: point,
            max: point + Vec2::new(DEFAULT_EDITOR_WIDTH, DEFAULT_EDITOR_HEIGHT),
        },
        None => fallback_screen_rect(window),
    }
}

fn editor_size(screen_rect: Rect) -> Vec2 {
    let width =
        (screen_rect.width() + EDITOR_EXTRA_WIDTH).clamp(MIN_EDITOR_WIDTH, MAX_EDITOR_WIDTH);
    Vec2::new(width, screen_rect.height())
}

fn projected_editor_presentation(
    source: &ImePanelAnchorSource,
    screen_rect: Rect,
) -> ImeEditorPresentation {
    let scale_x = screen_rect.width() / source.bounds.width;
    let scale_y = screen_rect.height() / source.bounds.height;
    let corner_scale = scale_x.min(scale_y);
    let authored = &source.presentation;
    let padding = Padding::new(
        Px(authored.padding.left.value * scale_x),
        Px(authored.padding.right.value * scale_x),
        Px(authored.padding.top.value * scale_y),
        Px(authored.padding.bottom.value * scale_y),
    );
    let border = authored.border.map(|border| {
        Border::new()
            .left(Px(border.left.value * scale_x))
            .right(Px(border.right.value * scale_x))
            .top(Px(border.top.value * scale_y))
            .bottom(Px(border.bottom.value * scale_y))
            .color(border.color)
    });
    let corner_radius = CornerRadius::new(
        Px(authored.corner_radius.top_left.value * corner_scale),
        Px(authored.corner_radius.top_right.value * corner_scale),
        Px(authored.corner_radius.bottom_right.value * corner_scale),
        Px(authored.corner_radius.bottom_left.value * corner_scale),
    );
    let mut text_style = authored.text_style.clone().scaled(corner_scale);
    text_style.set_dimension(Px(text_style.size()));

    ImeEditorPresentation {
        background: authored.background,
        border,
        corner_radius,
        padding,
        align_x: authored.align_x,
        align_y: authored.align_y,
        text_style,
        editor_text: EditorPartDeclaration::from_authored(authored.editor_text.clone()),
        editor_selection: EditorPartDeclaration::from_authored(authored.editor_selection.clone()),
        editor_caret: EditorPartDeclaration::from_authored(authored.editor_caret.clone()),
        editor_validation: EditorPartDeclaration::from_authored(authored.editor_validation.clone()),
    }
}

fn inline_editor_presentation(authored: &PanelFieldPresentation) -> ImeEditorPresentation {
    ImeEditorPresentation {
        background:        authored.background,
        border:            authored.border,
        corner_radius:     authored.corner_radius,
        padding:           authored.padding,
        align_x:           authored.align_x,
        align_y:           authored.align_y,
        text_style:        authored.text_style.clone(),
        editor_text:       EditorPartDeclaration::from_authored(authored.editor_text.clone()),
        editor_selection:  EditorPartDeclaration::from_authored(authored.editor_selection.clone()),
        editor_caret:      EditorPartDeclaration::from_authored(authored.editor_caret.clone()),
        editor_validation: EditorPartDeclaration::from_authored(authored.editor_validation.clone()),
    }
}

fn clamp_editor_position(position: Vec2, editor_size: Vec2, window: &Window) -> Vec2 {
    let max_x = (window.width() - editor_size.x).max(0.0);
    let max_y = (window.height() - editor_size.y).max(0.0);
    Vec2::new(position.x.clamp(0.0, max_x), position.y.clamp(0.0, max_y))
}

fn caret_position(
    editor_pos: Vec2,
    editor_size: Vec2,
    snapshot: &ImeBufferSnapshot,
    measurer: &DiegeticTextMeasurer,
    presentation: &ImeEditorPresentation,
    horizontal_scroll: f32,
) -> Vec2 {
    let border = presentation.border.unwrap_or_default();
    let horizontal_chrome = border.left.value
        + border.right.value
        + presentation.padding.left.value
        + presentation.padding.right.value;
    let content_width = (editor_size.x - horizontal_chrome).max(0.0);
    // Match the drawn caret's envelope, including its paint clearance.
    let text_width = (content_width - caret_envelope_width(&presentation.text_style)).max(0.0);
    let prefix = caret_prefix_text(snapshot);
    let text = editing_text(snapshot);
    let measure = presentation.text_style.as_measure();
    let measured_prefix = (measurer.measure_fn)(prefix.as_ref(), &measure).width;
    let measured_text = (measurer.measure_fn)(text.as_ref(), &measure).width;
    let aligned_offset = match presentation.align_x {
        AlignX::Left => 0.0,
        AlignX::Center => (text_width - measured_text).max(0.0) * 0.5,
        AlignX::Right => (text_width - measured_text).max(0.0),
    };
    let caret_x = border.left.value
        + presentation.padding.left.value
        + (aligned_offset + measured_prefix - horizontal_scroll).clamp(0.0, text_width);
    let vertical_chrome = border.top.value
        + border.bottom.value
        + presentation.padding.top.value
        + presentation.padding.bottom.value;
    let content_height = (editor_size.y - vertical_chrome).max(0.0);
    let caret_height = visible_caret_height(&presentation.text_style);
    let aligned_y = match presentation.align_y {
        AlignY::Top => 0.0,
        AlignY::Center => (content_height - caret_height).max(0.0) * 0.5,
        AlignY::Bottom => (content_height - caret_height).max(0.0),
    };
    let caret_y = border.top.value + presentation.padding.top.value + aligned_y;
    Vec2::new(
        (editor_pos.x + caret_x)
            .round()
            .min(editor_pos.x + border.left.value + presentation.padding.left.value + text_width),
        (editor_pos.y + caret_y).round(),
    )
}

pub(super) fn text_hit_at_x(
    text: &str,
    pointer_x: f32,
    horizontal_scroll: f32,
    record: &PanelFieldRecord,
    measurer: &DiegeticTextMeasurer,
) -> TextHit {
    let presentation = record.presentation();
    let border = presentation.border.unwrap_or_default();
    let horizontal_chrome = border.left.value
        + border.right.value
        + presentation.padding.left.value
        + presentation.padding.right.value;
    let content_width =
        (record.bounds.width - horizontal_chrome - caret_envelope_width(&presentation.text_style))
            .max(0.0);
    let text_style = presentation.text_style.clone();
    let measure = text_style.as_measure();
    let measured_text = (measurer.measure_fn)(text, &measure).width;
    let aligned_offset = match presentation.align_x {
        AlignX::Left => 0.0,
        AlignX::Center => (content_width - measured_text).max(0.0) * 0.5,
        AlignX::Right => (content_width - measured_text).max(0.0),
    };
    let text_x = pointer_x + horizontal_scroll
        - record.bounds.x
        - border.left.value
        - presentation.padding.left.value
        - aligned_offset;
    if text_x <= 0.0 {
        return TextHit { insertion: 0 };
    }

    let mut previous_width = 0.0;
    for (index, character) in text.char_indices() {
        let end = index + character.len_utf8();
        let width = (measurer.measure_fn)(&text[..end], &measure).width;
        if text_x <= width {
            let insertion = if text_x - previous_width < (width - previous_width) * 0.5 {
                index
            } else {
                end
            };
            return TextHit { insertion };
        }
        previous_width = width;
    }
    TextHit {
        insertion: text.len(),
    }
}

fn caret_prefix_text(snapshot: &ImeBufferSnapshot) -> Cow<'_, str> {
    if let Some(preedit) = &snapshot.preedit {
        let start = preedit.replacement.start.as_usize();
        let cursor = preedit
            .cursor
            .map_or(preedit.text.len(), ImePreeditBoundary::as_usize);
        let mut prefix = String::with_capacity(start + cursor);
        prefix.push_str(&snapshot.committed_text[..start]);
        prefix.push_str(&preedit.text[..cursor]);
        return Cow::Owned(prefix);
    }

    let cursor = match &snapshot.cursor {
        ImeCursorState::Insertion(boundary) => boundary.as_usize(),
        ImeCursorState::Selection(selection) => selection.focus.as_usize(),
    };
    Cow::Borrowed(&snapshot.committed_text[..cursor])
}

fn editing_text(snapshot: &ImeBufferSnapshot) -> Cow<'_, str> {
    let Some(preedit) = &snapshot.preedit else {
        return Cow::Borrowed(&snapshot.committed_text);
    };
    let start = preedit.replacement.start.as_usize();
    let end = preedit.replacement.end.as_usize();
    let mut text =
        String::with_capacity(snapshot.committed_text.len() - (end - start) + preedit.text.len());
    text.push_str(&snapshot.committed_text[..start]);
    text.push_str(&preedit.text);
    text.push_str(&snapshot.committed_text[end..]);
    Cow::Owned(text)
}

fn visible_caret_height(style: &TextStyle) -> f32 {
    let line_height = style.line_height_raw();
    if line_height > 0.0 {
        line_height
    } else {
        style.size()
    }
}

const fn caret_envelope_width(style: &TextStyle) -> f32 {
    // SDF antialiasing and the camera's edge filter paint beyond the stroke's
    // layout bounds. Reserve another stroke width before the field padding.
    style.size() * CARET_WIDTH_TO_FONT_SIZE * 2.0
}

fn editor_text_style() -> TextStyle { TextStyle::new(EDITOR_FONT_SIZE) }

fn editor_tree(
    snapshot: &ImeBufferSnapshot,
    validation: &ImeEditorValidation,
    presentation: Option<&ImeEditorPresentation>,
) -> LayoutTree {
    let fallback = default_editor_presentation();
    let presentation = presentation.unwrap_or(&fallback);
    let mut root = El::column()
        .width(Sizing::GROW)
        .height(Sizing::GROW)
        .padding(presentation.padding)
        .gap(EDITOR_GAP)
        .alignment(presentation.align_x, presentation.align_y)
        .corner_radius(presentation.corner_radius);
    if let Some(background) = presentation.background {
        root = root.background(background);
    }
    if let Some(border) = presentation.border {
        root = root.border(border);
    }
    let mut builder = LayoutBuilder::with_root(root);

    append_editor_rows(&mut builder, snapshot, validation, presentation);
    builder.build()
}

fn inline_editor_content_tree(
    snapshot: &ImeBufferSnapshot,
    validation: &ImeEditorValidation,
    presentation: &ImeEditorPresentation,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(presentation.text_style.size() * EDITOR_GAP / EDITOR_FONT_SIZE)
            .alignment(presentation.align_x, presentation.align_y),
    );
    append_editor_rows(&mut builder, snapshot, validation, presentation);
    builder.build()
}

fn append_editor_rows(
    builder: &mut LayoutBuilder,
    snapshot: &ImeBufferSnapshot,
    validation: &ImeEditorValidation,
    presentation: &ImeEditorPresentation,
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(0.0)
            .alignment(presentation.align_x, presentation.align_y),
        |builder| append_buffer(builder, snapshot, presentation),
    );

    if let ImeEditorValidation::Rejected(message) = validation {
        let validation_style = presentation
            .text_style
            .clone()
            .with_color(EDITOR_VALIDATION);
        add_text(
            builder,
            message,
            &validation_style,
            &presentation.editor_validation,
        );
    }
}

fn default_editor_presentation() -> ImeEditorPresentation {
    ImeEditorPresentation {
        background:        Some(EDITOR_BACKGROUND),
        border:            Some(Border::all(EDITOR_BORDER_WIDTH, EDITOR_BORDER)),
        corner_radius:     CornerRadius::all(EDITOR_CORNER_RADIUS),
        padding:           Padding::xy(EDITOR_PADDING_X, EDITOR_PADDING_Y),
        align_x:           AlignX::Left,
        align_y:           AlignY::Center,
        text_style:        editor_text_style().with_color(EDITOR_TEXT),
        editor_text:       EditorPartDeclaration::Generated,
        editor_selection:  EditorPartDeclaration::Generated,
        editor_caret:      EditorPartDeclaration::Generated,
        editor_validation: EditorPartDeclaration::Generated,
    }
}

fn append_buffer(
    builder: &mut LayoutBuilder,
    snapshot: &ImeBufferSnapshot,
    presentation: &ImeEditorPresentation,
) {
    if let Some(preedit) = &snapshot.preedit {
        append_preedit_buffer(builder, snapshot, preedit, presentation);
        return;
    }

    match &snapshot.cursor {
        ImeCursorState::Insertion(boundary) => {
            let index = boundary.as_usize();
            add_text(
                builder,
                &snapshot.committed_text[..index],
                &presentation.text_style,
                &presentation.editor_text,
            );
            add_caret(builder, &presentation.text_style, presentation);
            add_text(
                builder,
                &snapshot.committed_text[index..],
                &presentation.text_style,
                &presentation.editor_text,
            );
        },
        ImeCursorState::Selection(selection) => {
            let (start, end) = selection_range(selection);
            add_text(
                builder,
                &snapshot.committed_text[..start],
                &presentation.text_style,
                &presentation.editor_text,
            );
            if selection.focus.as_usize() == start {
                add_caret(builder, &presentation.text_style, presentation);
            }
            add_selected_text(
                builder,
                &snapshot.committed_text[start..end],
                &presentation.text_style,
                presentation,
            );
            if selection.focus.as_usize() == end {
                add_caret(builder, &presentation.text_style, presentation);
            }
            add_text(
                builder,
                &snapshot.committed_text[end..],
                &presentation.text_style,
                &presentation.editor_text,
            );
        },
    }
}

fn append_preedit_buffer(
    builder: &mut LayoutBuilder,
    snapshot: &ImeBufferSnapshot,
    preedit: &ImePreedit,
    presentation: &ImeEditorPresentation,
) {
    let start = preedit.replacement.start.as_usize();
    let end = preedit.replacement.end.as_usize();
    let cursor = preedit
        .cursor
        .map_or(preedit.text.len(), ImePreeditBoundary::as_usize);

    add_text(
        builder,
        &snapshot.committed_text[..start],
        &presentation.text_style,
        &presentation.editor_text,
    );
    let preedit_style = presentation.text_style.clone().with_color(EDITOR_PREEDIT);
    add_text(
        builder,
        &preedit.text[..cursor],
        &preedit_style,
        &presentation.editor_text,
    );
    add_caret(builder, &presentation.text_style, presentation);
    add_text(
        builder,
        &preedit.text[cursor..],
        &preedit_style,
        &presentation.editor_text,
    );
    add_text(
        builder,
        &snapshot.committed_text[end..],
        &presentation.text_style,
        &presentation.editor_text,
    );
}

fn selection_range(selection: &ImeSelectionSnapshot) -> (usize, usize) {
    let anchor = selection.anchor.as_usize();
    let focus = selection.focus.as_usize();
    (anchor.min(focus), anchor.max(focus))
}

fn add_text(
    builder: &mut LayoutBuilder,
    text: &str,
    style: &TextStyle,
    declaration: &EditorPartDeclaration,
) {
    if text.is_empty() {
        return;
    }
    match declaration {
        EditorPartDeclaration::Authored(declaration) => {
            builder.text(declaration.as_ref().clone().into_text(text, style))
        },
        EditorPartDeclaration::Generated => {
            builder.text(Text::new(text, style.clone()).generated_editor_part())
        },
    };
}

fn add_selected_text(
    builder: &mut LayoutBuilder,
    text: &str,
    style: &TextStyle,
    presentation: &ImeEditorPresentation,
) {
    if text.is_empty() {
        return;
    }
    match &presentation.editor_selection {
        EditorPartDeclaration::Authored(declaration) => declaration
            .as_ref()
            .clone()
            .with_background_if_unset(EDITOR_SELECTION)
            .with_width(Sizing::FIT)
            .with_height(Sizing::FIT)
            .with_children(builder, |builder| {
                add_text(builder, text, style, &presentation.editor_text);
            }),
        EditorPartDeclaration::Generated => {
            builder.with(
                El::new()
                    .generated_editor_part()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .background(EDITOR_SELECTION)
                    .padding(Padding::xy(0.0, 0.0)),
                |builder| add_text(builder, text, style, &presentation.editor_text),
            );
        },
    }
}

fn add_caret(
    builder: &mut LayoutBuilder,
    text_style: &TextStyle,
    presentation: &ImeEditorPresentation,
) {
    let caret_height = visible_caret_height(text_style);
    let caret_width = text_style.size() * CARET_WIDTH_TO_FONT_SIZE;
    builder.with(
        El::column()
            .caret_marker()
            .width(Sizing::fixed(0.0))
            .height(Sizing::GROW)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::fixed(caret_envelope_width(text_style)))
                    .height(Sizing::fixed(caret_height))
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| append_caret_stroke(builder, caret_width, caret_height, presentation),
            );
        },
    );
}

fn append_caret_stroke(
    builder: &mut LayoutBuilder,
    caret_width: f32,
    caret_height: f32,
    presentation: &ImeEditorPresentation,
) {
    match &presentation.editor_caret {
        EditorPartDeclaration::Authored(declaration) => declaration
            .as_ref()
            .clone()
            .with_background_if_unset(EDITOR_CARET)
            .with_width(Sizing::fixed(caret_width))
            .with_height(Sizing::fixed(caret_height))
            .with_children(builder, |_| {}),
        EditorPartDeclaration::Generated => {
            builder.with(
                El::new()
                    .generated_editor_part()
                    .width(Sizing::fixed(caret_width))
                    .height(Sizing::fixed(caret_height))
                    .background(EDITOR_CARET),
                |_| {},
            );
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::InputPlugin;
    use bevy::math::Rect;
    use bevy::math::Vec2;
    use bevy::math::Vec3;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerClick;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::Ime;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;

    use super::EDITOR_SELECTION;
    use super::ImeBlurIntent;
    use super::ImeEditor;
    use super::ImeEditorPanel;
    use super::ImeEditorState;
    use super::ImeEditorSurface;
    use super::ImeEditorValidation;
    use super::PendingImePanelAnchor;
    use super::caret_position;
    use super::caret_prefix_text;
    use super::clamp_editor_position;
    use super::classify_non_panel_click;
    use super::classify_widget_click;
    use super::editor_size;
    use super::pointer_command;
    use super::screen_field_record_rect;
    use super::text_hit_at_x;
    use crate::AlignX;
    use crate::BoundingBox;
    use crate::DiegeticPanel;
    use crate::DiegeticTextMeasurer;
    use crate::EditorStateColors;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeBufferBoundary;
    use crate::ImeBufferRange;
    use crate::ImeBufferSnapshot;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeCancelCause;
    use crate::ImeCursorState;
    use crate::ImeEditableFieldSpec;
    use crate::ImeOpenSession;
    use crate::ImePreedit;
    use crate::ImePreeditBoundary;
    use crate::ImeRequestCancel;
    use crate::ImeRequestCommit;
    use crate::ImeSelectionSnapshot;
    use crate::ImeSessionId;
    use crate::ImeTarget;
    use crate::ImeTextChanged;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelFieldRecord;
    use crate::PanelScreenBounds;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::RequestWidgetFocus;
    use crate::TextStyle;
    use crate::Unit;
    use crate::constants::MONOSPACE_WIDTH_RATIO;
    use crate::ime::ActiveImeSession;
    use crate::ime::ImeCommitCause;
    use crate::ime::ImePlugin;
    use crate::ime::buffer::ImeEditCommand;
    use crate::ime::pointer::FieldSelectionDrag;
    use crate::ime::pointer::click as classify_panel_click;
    use crate::ime::value_drag::FieldValueDrag;
    use crate::layout::LayoutEngine;
    use crate::layout::LayoutTreeChange;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommandKind;
    use crate::panel::PanelFieldPresentation;
    use crate::widgets::SemanticWidgetIntent;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetState;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;

    #[derive(Default, Resource)]
    struct PropagatedPanelClicks(Vec<bool>);

    const EDITOR_CARET_FOCUSED_FILL: Color = Color::srgb(0.20, 0.80, 0.40);
    const EDITOR_TEXT_DISABLED_COLOR: Color = Color::srgb(0.35, 0.45, 0.55);
    const EDITOR_SELECTION_FOCUSED_FILL: Color = Color::srgb(0.90, 0.30, 0.20);
    const EDITOR_TEXT_FOCUSED_COLOR: Color = Color::srgb(0.20, 0.50, 0.90);
    const EDITOR_VALIDATION_FOCUSED_COLOR: Color = Color::srgb(0.80, 0.30, 0.80);

    fn record_propagated_panel_click(
        click: On<PointerClick>,
        mut clicks: ResMut<PropagatedPanelClicks>,
    ) {
        clicks.0.push(click.get_propagate());
    }

    fn assert_float_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn insertion_snapshot(text: &str, cursor: usize) -> ImeBufferSnapshot {
        ImeBufferSnapshot {
            committed_text: text.to_owned(),
            cursor:         ImeCursorState::Insertion(ImeBufferBoundary::new(cursor)),
            preedit:        None,
        }
    }

    fn editable_tree(text: &str) -> LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(
            El::new().editable_field("field", field, crate::TextStyle::new(10.0)),
            |builder| {
                builder.text((text, TextStyle::new(10.0)));
            },
        );
        builder.build()
    }

    fn styled_editable_tree() -> LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Float {
                min: None,
                max: None,
            }));
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(
            El::new()
                .editable_field("field", field, crate::TextStyle::new(10.0))
                .editor_text(EditorStateColors::new().focused(EDITOR_TEXT_FOCUSED_COLOR))
                .editor_selection(EditorStateColors::new().focused(EDITOR_SELECTION_FOCUSED_FILL))
                .editor_caret(EditorStateColors::new().focused(EDITOR_CARET_FOCUSED_FILL))
                .editor_validation(
                    EditorStateColors::new().focused(EDITOR_VALIDATION_FOCUSED_COLOR),
                ),
            |builder| {
                builder.text(("display", TextStyle::new(10.0)));
            },
        );
        builder.build()
    }

    fn disabled_text_editable_tree() -> LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Float {
                min: None,
                max: None,
            }));
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(
            El::new()
                .editable_field("field", field, crate::TextStyle::new(10.0))
                .editor_text(EditorStateColors::new().disabled(EDITOR_TEXT_DISABLED_COLOR))
                .editor_validation(EditorStateColors::new().disabled(EDITOR_TEXT_DISABLED_COLOR)),
            |builder| {
                builder.text(("display", TextStyle::new(10.0)));
            },
        );
        builder.build()
    }

    fn hover_only_selection_tree() -> LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(
            El::new()
                .editable_field("field", field, crate::TextStyle::new(10.0))
                .editor_selection(EditorStateColors::new().hovered(Color::srgb(0.9, 0.2, 0.1))),
            |builder| {
                builder.text(("display", TextStyle::new(10.0)));
            },
        );
        builder.build()
    }

    fn inline_editor_app(text: &str) -> (App, Entity, Entity, LayoutTree) {
        let mut app = App::new();
        app.add_plugins(ImePlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let tree = editable_tree(text);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(40.0))
            .with_tree(tree.clone())
            .build()
            .expect("the editable panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.world_mut().flush();
        (app, window, panel, tree)
    }

    fn interactive_inline_editor_app(text: &str) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(InputPlugin)
            .add_message::<Ime>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .add_plugins((HeadlessLayoutPlugin, ImePlugin));
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(40.0))
            .with_tree(editable_tree(text))
            .build()
            .expect("the interactive editable panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        (app, window, panel)
    }

    fn inline_editor_visual_app(tree: LayoutTree) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ImePlugin));
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = DiegeticPanel::screen()
            .size(Px(100.0), Px(40.0))
            .window(WindowRef::Entity(window))
            .with_tree(tree)
            .build()
            .expect("the styled editable panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        (app, window, panel)
    }

    fn resolve_field_widget(app: &mut App, panel: Entity) -> Entity {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("field"))
            })
            .expect("field reader should run")
            .expect("field should reify")
    }

    fn assert_focused_editor_parts(
        app: &App,
        panel: Entity,
        widget: Entity,
        expected_overrides: &[VisualSlotOverride],
    ) -> Vec<usize> {
        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("editable field should carry visual slots");
        let parts = slots.part_appearances();
        assert_eq!(parts.len(), expected_overrides.len());
        assert!(
            parts.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "generated editor part appearances must remain element-index ordered",
        );
        for ((element_index, appearance), expected_override) in parts.iter().zip(expected_overrides)
        {
            assert_eq!(
                appearance
                    .cascades()
                    .resolve(&[Some(WidgetState::Focused)], None),
                expected_override.clone(),
            );
            assert_eq!(
                app.world()
                    .resource::<VisualOverrideIndex>()
                    .get(panel, *element_index),
                Some(expected_override),
            );
        }
        parts
            .iter()
            .map(|(element_index, _)| *element_index)
            .collect()
    }

    fn assert_disabled_editor_text_parts(
        app: &App,
        panel: Entity,
        widget: Entity,
        expected_count: usize,
    ) {
        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("editable field should carry visual slots");
        let parts = slots.part_appearances();
        assert_eq!(parts.len(), expected_count);
        for (element_index, appearance) in parts {
            assert!(slots.elements().iter().any(|(visual_index, capabilities)| {
                *visual_index == *element_index
                    && capabilities.contains(VisualElementCapabilities::TEXT)
            }));
            assert_eq!(
                appearance
                    .cascades()
                    .resolve(&[Some(WidgetState::Disabled)], None)
                    .text_color,
                Some(EDITOR_TEXT_DISABLED_COLOR),
            );
            let override_value = app
                .world()
                .resource::<VisualOverrideIndex>()
                .get(panel, *element_index)
                .expect("disabled editor text recipient should receive an override");
            assert_eq!(override_value.text_color, Some(EDITOR_TEXT_DISABLED_COLOR));
            assert_eq!(override_value.fill_color, None);
        }
    }

    fn open_inline_editor(app: &mut App, window: Entity, panel: Entity, text: &str) {
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: "field".into(),
            },
            window,
            initial_text: text.to_owned(),
            field_spec: ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            anchor: None,
        });
        app.world_mut().flush();
    }

    fn active_session_id(app: &App) -> ImeSessionId {
        app.world()
            .resource::<ActiveImeSession>()
            .active_session_id()
            .expect("the IME session should be active")
    }

    fn activate_styled_inline_editor(app: &mut App, window: Entity, field: Entity) {
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: field,
        });
        app.world_mut().flush();
        app.update();

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: field,
            window,
        });
        app.world_mut().flush();
        app.update();
    }

    fn move_inline_editor_cursor(app: &mut App, position: usize) {
        let input_blocker = app.world().resource::<crate::ImeInputBlocker>().clone();
        let changed = app
            .world_mut()
            .resource_mut::<ActiveImeSession>()
            .apply_edit_command(ImeEditCommand::PlaceCursor(position), &input_blocker)
            .expect("active editor should accept a cursor placement");
        app.world_mut().trigger(changed);
        app.world_mut().flush();
    }

    fn apply_inline_editor_preedit(
        app: &mut App,
        window: Entity,
        text: &str,
        cursor: Option<(usize, usize)>,
    ) {
        let input_blocker = app.world().resource::<crate::ImeInputBlocker>().clone();
        let changed = app
            .world_mut()
            .resource_mut::<ActiveImeSession>()
            .apply_preedit(window, text, cursor, &input_blocker)
            .expect("active editor should accept a preedit update");
        app.world_mut().trigger(changed);
        app.world_mut().flush();
    }

    fn field_word_position(app: &App, panel: Entity, prefix: &str, character: &str) -> Vec3 {
        let computed = app
            .world()
            .get::<crate::ComputedDiegeticPanel>(panel)
            .expect("the source panel should have computed field bounds");
        let record = computed
            .field_records()
            .first()
            .expect("the source panel should have an editable field");
        let text_style = record.presentation().text_style.clone();
        let measurer = app.world().resource::<DiegeticTextMeasurer>();
        let measure = text_style.as_measure();
        let prefix_width = (measurer.measure_fn)(prefix, &measure).width;
        let character_width = (measurer.measure_fn)(character, &measure).width;
        let panel_local = Vec2::new(
            character_width.mul_add(0.5, record.bounds.x + prefix_width),
            record.bounds.height.mul_add(0.5, record.bounds.y),
        );
        panel_local_position(app, panel, panel_local)
    }

    fn panel_local_position(app: &App, panel: Entity, panel_local: Vec2) -> Vec3 {
        let panel_data = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("the source panel should exist");
        let transform = app
            .world()
            .get::<GlobalTransform>(panel)
            .expect("the source panel should have a global transform");
        let points_to_world = panel_data.points_to_world();
        let (anchor_x, anchor_y) = panel_data.anchor_offsets();
        let local = Vec3::new(
            panel_local.x.mul_add(points_to_world, -anchor_x),
            (-panel_local.y).mul_add(points_to_world, anchor_y),
            0.0,
        );
        transform.transform_point(local)
    }

    fn click_panel_field(app: &mut App, window: Entity, panel: Entity, position: Vec3, count: u8) {
        let window_ref = WindowRef::Entity(window)
            .normalize(None)
            .expect("the entity window reference should normalize");
        app.world_mut().trigger(PointerClick {
            entity: panel,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button: PointerButton::Primary,
            hit: HitData::new(Entity::PLACEHOLDER, 0.0, Some(position), None),
            duration: std::time::Duration::ZERO,
            count,
        });
        app.world_mut().flush();
    }

    fn editor_state(target: ImeTarget) -> ImeEditorState {
        ImeEditorState {
            active: Some(ImeEditor {
                session_id: ImeSessionId::new(1),
                target,
                window: Entity::PLACEHOLDER,
                snapshot: insertion_snapshot("", 0),
                validation: ImeEditorValidation::Accepted,
                surface: ImeEditorSurface::ScreenOverlay {
                    panel: Entity::PLACEHOLDER,
                },
                source: None,
                app_anchor: None,
                anchor: None,
                presentation: None,
            }),
        }
    }

    #[test]
    fn panel_field_edits_inline_without_spawning_an_editor_panel() {
        let (mut app, window, panel, authored_tree) = inline_editor_app("before");

        open_inline_editor(&mut app, window, panel, "before");

        let panel_tree = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("the source panel should remain")
            .tree();
        assert_eq!(
            authored_tree.classify_change(panel_tree),
            LayoutTreeChange::LayoutAffecting,
            "selection presentation should be rendered in a derived source-panel tree",
        );
        let editor_panels = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<ImeEditorPanel>>();
            query.iter(world).count()
        };
        assert_eq!(editor_panels, 0);
    }

    #[test]
    fn anchored_world_field_draws_typed_text_inline() {
        let (mut app, window, panel, _) = inline_editor_app("0");
        let field_id = PanelElementId::named("field");
        app.world_mut()
            .resource_mut::<PendingImePanelAnchor>()
            .store(
                panel,
                field_id.clone(),
                Some(Entity::PLACEHOLDER),
                window,
                BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  80.0,
                    height: 20.0,
                },
                PanelFieldPresentation {
                    text_style: TextStyle::new(10.0),
                    ..default()
                },
            );
        open_inline_editor(&mut app, window, panel, "0");
        let session_id = active_session_id(&app);
        app.world_mut().trigger(ImeTextChanged {
            session_id,
            target: ImeTarget::WorldPanelField { panel, field_id },
            snapshot: insertion_snapshot("1.25", 4),
        });
        app.world_mut().flush();

        let editor = app.world().resource::<ImeEditorState>();
        let editor_panel = editor
            .active()
            .and_then(|editor| match editor.surface {
                ImeEditorSurface::Inline { panel, .. } => Some(panel),
                ImeEditorSurface::ScreenOverlay { .. } => None,
            })
            .expect("a picked world field edits its panel inline");
        assert_eq!(editor_panel, panel);
        let surface = app
            .world()
            .get::<DiegeticPanel>(editor_panel)
            .expect("editor surface");
        assert!(!surface.coordinate_space().is_screen());
        let measurer = DiegeticTextMeasurer::default();
        let layout = LayoutEngine::new(measurer.measure_fn).compute(
            surface.tree(),
            surface.width(),
            surface.height(),
            1.0,
        );
        assert!(layout.commands.iter().any(|command| {
            matches!(&command.kind, RenderCommandKind::Text { text, .. } if text == "1.25")
                && command.bounds.x >= 0.0
                && command.bounds.y >= 0.0
                && command.bounds.x + command.bounds.width <= surface.width()
                && command.bounds.y + command.bounds.height <= surface.height()
        }));
    }

    #[test]
    fn textless_world_field_edits_at_its_authored_font_size_inside_its_slot() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, ImePlugin));
        let window = app.world_mut().spawn(Window::default()).id();
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let mut builder = LayoutBuilder::new(0.1, 0.04);
        builder.with(
            El::row()
                .size(0.1, 0.04)
                .editable_field("field", field, TextStyle::new(0.008)),
            |_| {},
        );
        let panel = DiegeticPanel::world()
            .font_unit(Unit::Meters)
            .size(Mm(100.0), Mm(40.0))
            .with_tree(builder.build())
            .build()
            .expect("world field panel");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        open_inline_editor(&mut app, window, panel, "value");
        let panel = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("inline panel");
        let layout = LayoutEngine::new(DiegeticTextMeasurer::default().measure_fn).compute(
            panel.tree(),
            panel.width(),
            panel.height(),
            1.0,
        );
        assert!(layout.commands.iter().any(|command| {
            matches!(&command.kind, RenderCommandKind::Text { text, config }
                if text == "value" && (config.size() - 0.008).abs() < f32::EPSILON)
                && command.bounds.x >= 0.0
                && command.bounds.y >= 0.0
                && command.bounds.x + command.bounds.width <= 0.1
                && command.bounds.y + command.bounds.height <= 0.04
        }));
    }

    fn fitted_panel_height_points(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<crate::ComputedDiegeticPanel>(panel)
            .expect("fitted panel layout")
            .layout()
            .solved()
            .expect("fitted panel solves")
            .computed
            .first()
            .expect("fitted panel root")
            .bounds
            .height
    }

    #[test]
    fn ime_validation_after_uses_inherited_font_unit() {
        for (font_unit, label_size) in [(Unit::Pixels, 14.0), (Unit::Meters, 0.004)] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(DiegeticTextMeasurer::default())
                .add_plugins((InputPlugin, crate::HeadlessDiegeticUiPlugin))
                .add_observer(
                    |event: On<crate::ImeCommitRequested>, mut commands: Commands| {
                        commands.trigger(crate::ImeRejectCommit {
                            session_id: event.session_id,
                            attempt_id: event.attempt_id,
                            reason:     crate::ImeRejection::AppOwned("Invalid value.".to_owned()),
                        });
                    },
                );
            let window = app.world_mut().spawn(Window::default()).id();
            let field = ImeEditableFieldSpec::AppOwned(
                crate::ImeAppOwnedFieldSpec::new("field").validation_after("frame"),
            );
            let style = TextStyle::new(label_size);
            assert_eq!(
                style.unit(),
                crate::cascade::Cascade::Inherit,
                "the field must inherit its font unit",
            );
            let mut builder = LayoutBuilder::with_root(
                El::column()
                    .width(crate::Sizing::GROW)
                    .height(crate::Sizing::FIT),
            );
            builder.with(
                El::row()
                    .id("frame")
                    .width(crate::Sizing::GROW)
                    .height(crate::Sizing::fixed(crate::Pt(32.0)))
                    .editable_field("field", field.clone(), style.clone()),
                |builder| {
                    builder.text(("value", style));
                },
            );
            let panel = DiegeticPanel::screen()
                .size(crate::Pt(240.0), crate::Fit)
                .font_unit(font_unit)
                .window(WindowRef::Entity(window))
                .with_tree(builder.build())
                .build()
                .expect("validation panel");
            let panel = app.world_mut().spawn(panel).id();
            app.update();
            app.world_mut().trigger(ImeOpenSession {
                target: ImeTarget::ScreenPanelField {
                    panel,
                    field_id: "field".into(),
                },
                window,
                initial_text: "value".to_owned(),
                field_spec: field,
                anchor: None,
            });
            app.update();
            let normal_height = fitted_panel_height_points(&app, panel);
            let session_id = active_session_id(&app);
            app.world_mut().trigger(ImeRequestCommit {
                session_id,
                cause: ImeCommitCause::Enter,
            });
            app.update();
            let rejected = app
                .world()
                .get::<crate::ComputedDiegeticPanel>(panel)
                .expect("rejected panel layout");
            let message = rejected
                .layout()
                .solved()
                .expect("validation layout solves")
                .commands
                .iter()
                .find(|command| {
                    matches!(&command.kind, RenderCommandKind::Text { text, .. }
                        if text == "Invalid value.")
                })
                .expect("validation line");
            let step = crate::ControlMetrics::from_label_size(label_size)
                .spacing
                .step()
                * font_unit.to_points();
            let growth = fitted_panel_height_points(&app, panel) - normal_height;
            assert!(
                (-2.0_f32)
                    .mul_add(step, growth - message.bounds.height)
                    .abs()
                    < 0.0001,
                "{font_unit:?}: growth {growth} must equal line {} plus two {step} steps",
                message.bounds.height,
            );
        }
    }

    #[test]
    fn generated_editor_parts_resolve_through_display_editor_display() {
        let (mut app, window, panel) = inline_editor_visual_app(styled_editable_tree());
        let field = resolve_field_widget(&mut app, panel);
        activate_styled_inline_editor(&mut app, window, field);
        let selection_indices = assert_focused_editor_parts(
            &app,
            panel,
            field,
            &[
                VisualSlotOverride {
                    fill_color: Some(EDITOR_SELECTION_FOCUSED_FILL),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    text_color: Some(EDITOR_TEXT_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    fill_color: Some(EDITOR_CARET_FOCUSED_FILL),
                    ..VisualSlotOverride::default()
                },
            ],
        );

        move_inline_editor_cursor(&mut app, "display".len() / 2);
        app.update();
        let insertion_indices = assert_focused_editor_parts(
            &app,
            panel,
            field,
            &[
                VisualSlotOverride {
                    text_color: Some(EDITOR_TEXT_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    fill_color: Some(EDITOR_CARET_FOCUSED_FILL),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    text_color: Some(EDITOR_TEXT_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
            ],
        );

        let session_id = active_session_id(&app);
        app.world_mut().trigger(ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Request,
        });
        app.world_mut().flush();
        app.update();
        let validation_indices = assert_focused_editor_parts(
            &app,
            panel,
            field,
            &[
                VisualSlotOverride {
                    text_color: Some(EDITOR_TEXT_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    fill_color: Some(EDITOR_CARET_FOCUSED_FILL),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    text_color: Some(EDITOR_TEXT_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
                VisualSlotOverride {
                    text_color: Some(EDITOR_VALIDATION_FOCUSED_COLOR),
                    ..VisualSlotOverride::default()
                },
            ],
        );

        let session_id = active_session_id(&app);
        app.world_mut().trigger(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Request,
        });
        app.world_mut().flush();
        app.update();

        let restored_slots = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field should retain visual slots after closing the editor");
        assert_eq!(restored_slots.part_appearances(), []);
        for element_index in selection_indices
            .into_iter()
            .chain(insertion_indices)
            .chain(validation_indices)
        {
            assert!(
                app.world()
                    .resource::<VisualOverrideIndex>()
                    .get(panel, element_index)
                    .is_none(),
                "closing the editor must remove its generated part override",
            );
        }
    }

    #[test]
    fn disabled_editor_text_recolors_every_generated_text_recipient() {
        let (mut app, window, panel) = inline_editor_visual_app(disabled_text_editable_tree());
        let field = resolve_field_widget(&mut app, panel);
        activate_styled_inline_editor(&mut app, window, field);
        app.world_mut()
            .entity_mut(field)
            .insert(WidgetDisabled::test_marker());
        app.update();
        assert_disabled_editor_text_parts(&app, panel, field, 1);

        move_inline_editor_cursor(&mut app, "display".len() / 2);
        app.update();
        assert_disabled_editor_text_parts(&app, panel, field, 2);

        apply_inline_editor_preedit(&mut app, window, "xy", Some((1, 1)));
        app.update();
        assert_disabled_editor_text_parts(&app, panel, field, 4);

        apply_inline_editor_preedit(&mut app, window, "", None);
        app.update();
        let session_id = active_session_id(&app);
        app.world_mut().trigger(ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Request,
        });
        app.world_mut().flush();
        app.update();
        assert_disabled_editor_text_parts(&app, panel, field, 3);
    }

    #[test]
    fn hover_only_editor_selection_declaration_keeps_builtin_fill_at_rest() {
        let (mut app, window, panel) = inline_editor_visual_app(hover_only_selection_tree());
        let field = resolve_field_widget(&mut app, panel);

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: field,
            window,
        });
        app.world_mut().flush();
        app.update();

        let selection_index = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field should carry visual slots")
            .part_appearances()
            .first()
            .map(|(element_index, _)| *element_index)
            .expect("generated selection should retain its hovered appearance");
        let result = app
            .world()
            .get::<crate::ComputedDiegeticPanel>(panel)
            .map(crate::ComputedDiegeticPanel::layout)
            .and_then(crate::PanelLayout::solved)
            .expect("editable panel should retain its layout result");
        assert!(result.commands.iter().any(|command| {
            command.element_idx == selection_index
                && matches!(
                    &command.kind,
                    RenderCommandKind::Rectangle {
                        color,
                        source: RectangleSource::Background,
                    } if *color == EDITOR_SELECTION
                )
        }));
    }

    #[test]
    fn editor_text_id_is_removed_from_split_fragments() {
        let (mut app, window, panel) = inline_editor_visual_app(styled_editable_tree());
        let field = resolve_field_widget(&mut app, panel);
        activate_styled_inline_editor(&mut app, window, field);

        move_inline_editor_cursor(&mut app, "display".len() / 2);
        app.update();

        assert!(
            !app.world()
                .get::<DiegeticPanel>(panel)
                .expect("editable panel should remain")
                .tree()
                .contains_text_id(&PanelElementId::named("editor-text")),
        );
    }

    #[test]
    fn canceling_inline_edit_restores_the_authoritative_tree() {
        let (mut app, window, panel, authored_tree) = inline_editor_app("before");
        open_inline_editor(&mut app, window, panel, "before");
        let session_id = active_session_id(&app);

        app.world_mut().trigger(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Request,
        });
        app.world_mut().flush();

        let restored = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("the source panel should remain")
            .tree();
        assert_eq!(
            authored_tree.classify_change(restored),
            LayoutTreeChange::Identical,
        );
    }

    #[test]
    fn built_in_inline_commit_uses_the_authoritative_tree() {
        let (mut app, window, panel, authored_tree) = inline_editor_app("before");
        open_inline_editor(&mut app, window, panel, "after");
        let session_id = active_session_id(&app);

        app.world_mut().trigger(ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Request,
        });
        app.world_mut().flush();

        let committed = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("the source panel should remain")
            .tree();
        assert_eq!(committed.field_display_text(1), Some("after"));
        assert_eq!(committed.len(), authored_tree.len());
    }

    #[test]
    fn caret_prefix_uses_snapshot_cursor_without_splitting_utf8() {
        let snapshot = insertion_snapshot("aé日", "aé".len());

        assert_eq!(caret_prefix_text(&snapshot), "aé");
    }

    #[test]
    fn caret_prefix_includes_preedit_text_before_cursor() {
        let snapshot = ImeBufferSnapshot {
            committed_text: "abcdef".to_owned(),
            cursor:         ImeCursorState::Selection(ImeSelectionSnapshot {
                anchor: ImeBufferBoundary::new(1),
                focus:  ImeBufferBoundary::new(4),
            }),
            preedit:        Some(ImePreedit {
                text:        "xy".to_owned(),
                replacement: ImeBufferRange {
                    start: ImeBufferBoundary::new(1),
                    end:   ImeBufferBoundary::new(4),
                },
                cursor:      Some(ImePreeditBoundary::new(1)),
            }),
        };

        assert_eq!(caret_prefix_text(&snapshot), "ax");
    }

    #[test]
    fn editor_position_clamps_to_window_bounds() {
        let mut window = Window::default();
        window.resolution.set(320.0, 160.0);
        let position =
            clamp_editor_position(Vec2::new(300.0, 150.0), Vec2::new(80.0, 40.0), &window);

        assert_eq!(position, Vec2::new(240.0, 120.0));
    }

    #[test]
    fn caret_position_tracks_editor_width() {
        let measurer = DiegeticTextMeasurer::default();
        let snapshot = insertion_snapshot("abcd", 2);
        let caret = caret_position(
            Vec2::ZERO,
            Vec2::new(104.0, 34.0),
            &snapshot,
            &measurer,
            &super::default_editor_presentation(),
            0.0,
        );
        let expected_x = (super::EDITOR_FONT_SIZE * MONOSPACE_WIDTH_RATIO)
            .mul_add(2.0, super::EDITOR_BORDER_WIDTH + super::EDITOR_PADDING_X);

        assert_float_eq(caret.x, expected_x.round());
        assert_float_eq(caret.y, 9.0);
    }

    #[test]
    fn caret_position_reserves_its_width_before_the_right_padding() {
        let measurer = DiegeticTextMeasurer::default();
        let mut presentation = super::default_editor_presentation();
        presentation.align_x = AlignX::Right;
        let border = presentation.border.unwrap_or_default();
        let caret_width = presentation.text_style.size() * super::CARET_WIDTH_TO_FONT_SIZE;
        let right_edge = 104.0 - border.right.value - presentation.padding.right.value;
        for text in ["999", "12345678901234567890"] {
            let snapshot = insertion_snapshot(text, text.len());
            let caret = caret_position(
                Vec2::ZERO,
                Vec2::new(104.0, 34.0),
                &snapshot,
                &measurer,
                &presentation,
                0.0,
            );
            assert!(
                caret.x + caret_width <= right_edge,
                "{text}: caret {caret:?} must end before the padded edge {right_edge}"
            );
        }
    }

    #[test]
    fn caret_position_subtracts_applied_horizontal_scroll() {
        let measurer = DiegeticTextMeasurer::default();
        let snapshot = insertion_snapshot("abcdefghijklmnop", 8);
        let presentation = super::default_editor_presentation();
        let unscrolled = caret_position(
            Vec2::ZERO,
            Vec2::new(104.0, 34.0),
            &snapshot,
            &measurer,
            &presentation,
            0.0,
        );
        let scrolled = caret_position(
            Vec2::ZERO,
            Vec2::new(104.0, 34.0),
            &snapshot,
            &measurer,
            &presentation,
            25.0,
        );
        assert!(
            scrolled.x < unscrolled.x,
            "scrolled={scrolled:?}, unscrolled={unscrolled:?}"
        );
        assert!(scrolled.x >= super::EDITOR_BORDER_WIDTH + super::EDITOR_PADDING_X);
    }

    #[test]
    fn editor_size_is_bounded_from_source_rect() {
        let size = editor_size(Rect::from_corners(Vec2::ZERO, Vec2::new(900.0, 20.0)));

        assert_float_eq(size.x, super::MAX_EDITOR_WIDTH);
    }

    #[test]
    fn screen_panel_field_rect_uses_resolved_screen_bounds() {
        let record = PanelFieldRecord {
            field_id:      PanelElementId::named("title"),
            bounds:        BoundingBox {
                x:      20.0,
                y:      10.0,
                width:  60.0,
                height: 15.0,
            },
            field_spec:    ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            display_text:  String::new(),
            element_index: 0,
            duplicate_id:  false,
            presentation:  crate::panel::PanelFieldPresentation::default(),
        };
        let bounds = PanelScreenBounds::new(Vec2::new(100.0, 50.0), Vec2::new(200.0, 100.0))
            .expect("screen bounds are valid");

        let rect = screen_field_record_rect(&record, 2.0, bounds)
            .expect("field bounds produce a visible rect");

        assert_eq!(rect.min, Vec2::new(140.0, 70.0));
        assert_eq!(rect.max, Vec2::new(260.0, 100.0));
    }

    #[test]
    fn text_hit_uses_the_character_half_nearest_the_pointer() {
        let text_style = TextStyle::new(10.0);
        let measurer = DiegeticTextMeasurer::default();
        let text_width = (measurer.measure_fn)("abcd", &text_style.as_measure()).width;
        let character_width = (measurer.measure_fn)("a", &text_style.as_measure()).width;
        let record = PanelFieldRecord {
            field_id:      PanelElementId::named("title"),
            bounds:        BoundingBox {
                x:      20.0,
                y:      10.0,
                width:  100.0,
                height: 20.0,
            },
            field_spec:    ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            display_text:  "abcd".to_owned(),
            element_index: 0,
            duplicate_id:  false,
            presentation:  PanelFieldPresentation {
                align_x: AlignX::Center,
                text_style,
                ..default()
            },
        };
        let text_start = (record.bounds.width
            - super::caret_envelope_width(&record.presentation.text_style)
            - text_width)
            .mul_add(0.5, record.bounds.x);

        let left_half = text_hit_at_x(
            "abcd",
            character_width.mul_add(1.25, text_start),
            0.0,
            &record,
            &measurer,
        );
        let right_half = text_hit_at_x(
            "abcd",
            character_width.mul_add(1.75, text_start),
            0.0,
            &record,
            &measurer,
        );

        assert_eq!(left_half.insertion, 1);

        assert_eq!(right_half.insertion, 2);
    }

    #[test]
    fn click_near_scrolled_field_left_edge_hits_visible_character() {
        let text_style = TextStyle::new(10.0);
        let measurer = DiegeticTextMeasurer::default();
        let character_width = (measurer.measure_fn)("a", &text_style.as_measure()).width;
        let record = PanelFieldRecord {
            field_id:      PanelElementId::named("field"),
            bounds:        BoundingBox {
                x:      20.0,
                y:      10.0,
                width:  72.0,
                height: 20.0,
            },
            field_spec:    ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            display_text:  "abcdefghijklmnop".to_owned(),
            element_index: 0,
            duplicate_id:  false,
            presentation:  PanelFieldPresentation {
                align_x: AlignX::Left,
                text_style,
                ..default()
            },
        };
        let pointer_x = character_width.mul_add(0.25, record.bounds.x);
        let hit = text_hit_at_x(
            &record.display_text,
            pointer_x,
            character_width * 8.0,
            &record,
            &measurer,
        );
        assert!(
            hit.insertion >= 7,
            "visible start should be near index 8: {hit:?}"
        );
    }

    #[test]
    fn pointer_click_count_selects_cursor_or_all() {
        let text_hit = super::TextHit { insertion: 4 };

        assert!(matches!(
            pointer_command(1, text_hit),
            ImeEditCommand::PlaceCursor(4)
        ));
        assert!(matches!(
            pointer_command(2, text_hit),
            ImeEditCommand::SelectAll
        ));
        assert!(matches!(
            pointer_command(3, text_hit),
            ImeEditCommand::SelectAll
        ));
    }

    #[test]
    fn panel_pointer_clicks_place_cursor_and_select_all() {
        let text = "alpha beta";
        let (mut app, window, panel) = interactive_inline_editor_app(text);
        let position = field_word_position(&app, panel, "al", "p");
        open_inline_editor(&mut app, window, panel, text);

        click_panel_field(&mut app, window, panel, position, 1);
        let snapshot = &app
            .world()
            .resource::<ImeEditorState>()
            .active()
            .expect("the editor should remain active")
            .snapshot;
        assert!(matches!(
            snapshot.cursor,
            ImeCursorState::Insertion(boundary) if boundary.as_usize() == "alp".len()
        ));

        click_panel_field(&mut app, window, panel, position, 2);
        let snapshot = &app
            .world()
            .resource::<ImeEditorState>()
            .active()
            .expect("the editor should remain active")
            .snapshot;
        assert!(matches!(
            snapshot.cursor,
            ImeCursorState::Selection(ImeSelectionSnapshot { anchor, focus })
                if anchor.as_usize() == 0 && focus.as_usize() == text.len()
        ));

        click_panel_field(&mut app, window, panel, position, 3);
        let snapshot = &app
            .world()
            .resource::<ImeEditorState>()
            .active()
            .expect("the editor should remain active")
            .snapshot;
        assert!(matches!(
            snapshot.cursor,
            ImeCursorState::Selection(ImeSelectionSnapshot { anchor, focus })
                if anchor.as_usize() == 0 && focus.as_usize() == text.len()
        ));
    }

    #[test]
    fn clicking_outside_the_inline_field_commits_and_removes_selection() {
        let text = "alpha beta";
        let (mut app, window, panel) = interactive_inline_editor_app(text);
        let outside_field = panel_local_position(&app, panel, Vec2::new(99.0, 39.0));
        open_inline_editor(&mut app, window, panel, text);

        click_panel_field(&mut app, window, panel, outside_field, 1);
        let result = app.world_mut().run_system_once(super::handle_blur_intent);
        assert!(result.is_ok());
        app.world_mut().flush();

        assert!(
            app.world().resource::<ImeEditorState>().active().is_none(),
            "the inline editor should close after the outside click commits",
        );
        let committed = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("the source panel should remain")
            .tree();
        assert_eq!(committed.field_display_text(1), Some(text));
        assert_eq!(committed.len(), editable_tree(text).len());
    }

    #[test]
    fn clicking_outside_a_cancel_on_blur_field_restores_its_old_value() {
        let text = "0.400";
        let (mut app, window, panel) = interactive_inline_editor_app(text);
        let outside_field = panel_local_position(&app, panel, Vec2::new(99.0, 39.0));
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: "field".into(),
            },
            window,
            initial_text: text.to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(
                crate::ImeAppOwnedFieldSpec::new("field").cancel_on_blur(),
            ),
            anchor: None,
        });
        app.world_mut().flush();
        click_panel_field(&mut app, window, panel, outside_field, 1);
        app.world_mut()
            .run_system_once(super::handle_blur_intent)
            .expect("blur system should run");
        app.world_mut().flush();

        assert!(
            app.world()
                .resource::<ActiveImeSession>()
                .active_session_id()
                .is_none()
        );
        let restored = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("source panel should remain")
            .tree();
        assert_eq!(restored.field_display_text(1), Some(text));
    }

    #[test]
    fn a_keep_editing_field_ignores_a_click_outside_it() {
        let text = "alpha beta";
        let (mut app, window, panel) = interactive_inline_editor_app(text);
        let outside = panel_local_position(&app, panel, Vec2::new(99.0, 39.0));
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: "field".into(),
            },
            window,
            initial_text: text.to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(
                crate::ImeAppOwnedFieldSpec::new("field").keep_editing_on_blur(),
            ),
            anchor: None,
        });
        app.world_mut().flush();
        let session = app
            .world()
            .resource::<ActiveImeSession>()
            .active_session_id();
        click_panel_field(&mut app, window, panel, outside, 1);
        app.world_mut()
            .run_system_once(super::handle_blur_intent)
            .expect("blur system runs");
        app.world_mut().flush();
        let active = app.world().resource::<ActiveImeSession>();
        assert_eq!(active.active_session_id(), session);
        assert!(!active.is_pending_commit());
        assert!(app.world().resource::<ImeEditorState>().active().is_some());
    }

    #[test]
    fn widget_click_classification_ends_panel_field_editing() {
        let source_panel = Entity::from_raw_u32(1).expect("test entity index is valid");
        let other_panel = Entity::from_raw_u32(2).expect("test entity index is valid");
        let target = ImeTarget::WorldPanelField {
            panel:    source_panel,
            field_id: PanelElementId::named("field"),
        };
        let editor_state = editor_state(target.clone());
        let mut blur_intent = ImeBlurIntent::default();

        classify_widget_click(source_panel, &editor_state, &mut blur_intent);
        assert!(
            blur_intent
                .latest
                .as_ref()
                .is_some_and(|classification| !classification.is_inside_focus_scope(&target))
        );

        classify_widget_click(other_panel, &editor_state, &mut blur_intent);
        assert!(
            blur_intent
                .latest
                .as_ref()
                .is_some_and(|classification| !classification.is_inside_focus_scope(&target))
        );
    }

    #[test]
    fn non_panel_click_classifies_as_outside_the_inline_editor() {
        let source_panel = Entity::from_raw_u32(1).expect("test entity index is valid");
        let target = ImeTarget::WorldPanelField {
            panel:    source_panel,
            field_id: PanelElementId::named("field"),
        };
        let mut app = App::new();
        app.insert_resource(editor_state(target.clone()))
            .init_resource::<ImeBlurIntent>()
            .add_observer(classify_non_panel_click);
        let clicked_entity = app.world_mut().spawn_empty().id();

        app.world_mut().trigger(PointerClick {
            entity:   clicked_entity,
            pointer:  Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::None {
                        width:  1,
                        height: 1,
                    },
                    position: Vec2::ZERO,
                },
            ),
            button:   PointerButton::Primary,
            hit:      HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            duration: std::time::Duration::ZERO,
            count:    1,
        });

        assert!(
            app.world()
                .resource::<ImeBlurIntent>()
                .latest
                .as_ref()
                .is_some_and(|classification| !classification.is_inside_focus_scope(&target))
        );
    }

    #[test]
    fn panel_click_propagates_without_an_active_editor() {
        let mut app = App::new();
        app.init_resource::<ImeEditorState>()
            .init_resource::<ImeBlurIntent>()
            .init_resource::<ActiveImeSession>()
            .init_resource::<crate::ImeInputBlocker>()
            .init_resource::<DiegeticTextMeasurer>()
            .init_resource::<PropagatedPanelClicks>()
            .init_resource::<PendingImePanelAnchor>()
            .init_resource::<FieldSelectionDrag>()
            .init_resource::<FieldValueDrag>();
        let panel = app
            .world_mut()
            .spawn_empty()
            .observe(classify_panel_click)
            .observe(record_propagated_panel_click)
            .id();
        app.world_mut().flush();

        app.world_mut().trigger(PointerClick {
            entity:   panel,
            pointer:  Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::None {
                        width:  1,
                        height: 1,
                    },
                    position: Vec2::ZERO,
                },
            ),
            button:   PointerButton::Primary,
            hit:      HitData::new(Entity::PLACEHOLDER, 0.0, None, None),
            duration: std::time::Duration::ZERO,
            count:    1,
        });

        assert_eq!(app.world().resource::<PropagatedPanelClicks>().0, [true]);
    }
}
