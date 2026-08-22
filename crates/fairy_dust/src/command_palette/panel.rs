//! The palette box: a screen-space panel holding the keymap failure rows, the
//! query field, and the matching command rows.

use bevy::prelude::Color;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Border;
use hana_diegetic::CornerRadius;
use hana_diegetic::EditorStateColors;
use hana_diegetic::El;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_rubric::ActiveKeymapContextState;
use hana_rubric::CommandPaletteQueryResult;
use hana_rubric::CommandPaletteRow;
use hana_rubric::DiagnosticSeverity;
use hana_rubric::PaletteBinding;
use hana_rubric::PaletteSelectionOutcome;

use super::constants::COMMAND_KEYSTROKE_COLOR;
use super::constants::COMMAND_KEYSTROKE_COLUMN_WIDTH;
use super::constants::COMMAND_KEYSTROKE_MAX_CHARS;
use super::constants::COMMAND_ROW_HEIGHT;
use super::constants::COMMAND_TEXT_SIZE;
use super::constants::COMMAND_TITLE_COLOR;
use super::constants::COMMAND_TITLE_MAX_CHARS;
use super::constants::FAILURE_ACTION_COLUMN_WIDTH;
use super::constants::FAILURE_ACTION_ID_PREFIX;
use super::constants::FAILURE_ADVISORY_COLOR;
use super::constants::FAILURE_COLOR;
use super::constants::FAILURE_LINE_MAX_CHARS;
use super::constants::FAILURE_ROW_HEIGHT;
use super::constants::FAILURE_TEXT_SIZE;
use super::constants::FIELD_BACKGROUND;
use super::constants::FIELD_BORDER;
use super::constants::FIELD_BORDER_WIDTH;
use super::constants::FIELD_CORNER_RADIUS;
use super::constants::FIELD_HEIGHT;
use super::constants::FIELD_ID;
use super::constants::FIELD_PADDING_X;
use super::constants::FIELD_PADDING_Y;
use super::constants::FIELD_SELECTION_COLOR;
use super::constants::FIELD_TEXT_COLOR;
use super::constants::FIELD_TEXT_SIZE;
use super::constants::HIDDEN_ROW_COLOR;
use super::constants::MAX_VISIBLE_COMMAND_ROWS;
use super::constants::PANEL_BACKGROUND;
use super::constants::PANEL_BORDER;
use super::constants::PANEL_BORDER_WIDTH;
use super::constants::PANEL_COLUMN_GAP;
use super::constants::PANEL_CORNER_RADIUS;
use super::constants::PANEL_EDGE_MARGIN;
use super::constants::PANEL_MAX_WIDTH;
use super::constants::PANEL_MIN_WIDTH;
use super::constants::PANEL_PADDING;
use super::constants::PANEL_ROW_GAP;
use super::constants::PANEL_TOP_RATIO;
use super::constants::PLACEHOLDER_COLOR;
use super::constants::PLACEHOLDER_TEXT;
use super::constants::SELECTED_COMMAND_BACKGROUND;
use super::constants::SELECTED_COMMAND_TITLE_COLOR;
use super::constants::SEPARATOR_COLOR;
use super::constants::SEPARATOR_HEIGHT;
use super::failure_row::KeymapFailureAction;
use super::failure_row::KeymapFailureActionLabel;
use super::failure_row::KeymapFailureRow;
use super::missing_dimension_names;
use super::palette_binding_presentation;

/// The mutually exclusive command content states one palette rebuild can render.
pub(super) enum PalettePresentationInput<'view, 'registry, 'keymap, 'context> {
    /// No `CommandRegistry` exists yet, so no shared query can run.
    AwaitingRegistryAssembly,
    /// One borrowed query result from the live registry and its complete context observation.
    AssembledQuery {
        /// What the query resolved to, which decides the highlighted row and the status line.
        selection_outcome: PaletteSelectionOutcome<'registry>,
        /// The command rows matching the query, in registry order.
        command_rows:      &'view [CommandPaletteRow<'registry, 'keymap, 'context>],
        /// The complete active state snapshot the shared query observed.
        active_context:    &'context ActiveKeymapContextState,
    },
}

/// Everything one rebuild of the palette tree renders.
pub(super) struct PaletteView<'view, 'registry, 'keymap, 'context> {
    /// Query text as the IME session currently holds it.
    pub(super) query:               &'view str,
    /// The registry-assembly state and, once assembled, every borrowed query input.
    pub(super) presentation_input:  PalettePresentationInput<'view, 'registry, 'keymap, 'context>,
    /// Keymap failures rendered above the field.
    pub(super) keymap_failure_rows: &'view [KeymapFailureRow],
    /// Width the box occupies, which the window decides.
    pub(super) panel_width:         f32,
}

/// Whether a selection result needs a presentation status line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectionStatus {
    /// Render this sentence above the command rows.
    Message(&'static str),
    /// Render nothing: the listed commands are already the whole answer.
    Hidden,
}

/// Builds the tree for one borrowed shared Rubric query without retaining that
/// result in component state.
pub(super) fn palette_tree_for_query(
    query: &str,
    query_result: &CommandPaletteQueryResult<'_, '_, '_, '_>,
    keymap_failures: &[KeymapFailureRow],
    panel_width: f32,
) -> LayoutTree {
    palette_tree(&PaletteView {
        query,
        presentation_input: PalettePresentationInput::AssembledQuery {
            selection_outcome: query_result.selection(),
            command_rows:      query_result.rows(),
            active_context:    query_result.active_context(),
        },
        keymap_failure_rows: keymap_failures,
        panel_width,
    })
}

/// Builds the distinct panel state used only before command-registry assembly.
pub(super) fn palette_tree_awaiting_assembly(
    query: &str,
    keymap_failures: &[KeymapFailureRow],
    panel_width: f32,
) -> LayoutTree {
    palette_tree(&PaletteView {
        query,
        presentation_input: PalettePresentationInput::AwaitingRegistryAssembly,
        keymap_failure_rows: keymap_failures,
        panel_width,
    })
}

/// Width the palette box occupies in this window.
///
/// The box holds [`PANEL_MAX_WIDTH`] while the window is wide enough for it and
/// its edge margins, and shrinks with the window below that rather than
/// overhanging the left edge.
pub(super) fn palette_panel_width(window: &Window) -> f32 {
    let available = PANEL_EDGE_MARGIN.mul_add(-2.0, window.width());
    available.clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH)
}

/// Top-left corner the palette box is anchored at, in window pixels.
///
/// The horizontal origin is clamped so a window narrower than
/// [`PANEL_MIN_WIDTH`] still shows the box's left edge.
pub(super) fn palette_panel_origin(window: &Window) -> Vec2 {
    let centered = (window.width() - palette_panel_width(window)) * 0.5;
    Vec2::new(centered.max(0.0), window.height() * PANEL_TOP_RATIO)
}

/// The panel-local id the failure row at `row_index` gives its action button.
pub(super) fn failure_action_id(row_index: usize) -> String {
    format!("{FAILURE_ACTION_ID_PREFIX}{row_index}")
}

/// Reads back the failure-row index an action button's panel-local id carries.
pub(super) fn failure_action_row_index(element_id: &str) -> FailureActionRow {
    element_id
        .strip_prefix(FAILURE_ACTION_ID_PREFIX)
        .and_then(|index| index.parse().ok())
        .map_or(FailureActionRow::NotAFailureAction, |row_index| {
            FailureActionRow::Row(row_index)
        })
}

/// Whether a clicked panel element is one of the palette's failure-row actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailureActionRow {
    /// The click landed on the action of the failure row at this index.
    Row(usize),
    /// The click landed on some other element, which the palette leaves alone.
    NotAFailureAction,
}

/// Builds the palette's layout tree.
pub(super) fn palette_tree(view: &PaletteView<'_, '_, '_, '_>) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::fixed(view.panel_width))
            .height(Sizing::FIT),
    );
    build_palette(&mut builder, view);
    builder.build()
}

fn build_palette(builder: &mut LayoutBuilder, view: &PaletteView<'_, '_, '_, '_>) {
    builder.with(
        El::column()
            .width(Sizing::fixed(view.panel_width))
            .height(Sizing::FIT)
            .padding(Padding::all(PANEL_PADDING))
            .gap(PANEL_ROW_GAP)
            .background(PANEL_BACKGROUND)
            .corner_radius(CornerRadius::all(PANEL_CORNER_RADIUS))
            .border(Border::all(PANEL_BORDER_WIDTH, PANEL_BORDER)),
        |builder| {
            for (row_index, keymap_failure) in view.keymap_failure_rows.iter().enumerate() {
                build_failure_row(builder, row_index, keymap_failure);
            }
            build_query_field(builder, view);
            build_separator(builder);
            build_command_rows(builder, view);
        },
    );
}

fn build_failure_row(
    builder: &mut LayoutBuilder,
    row_index: usize,
    keymap_failure: &KeymapFailureRow,
) {
    let color = match keymap_failure.severity {
        DiagnosticSeverity::Failure => FAILURE_COLOR,
        DiagnosticSeverity::Advisory => FAILURE_ADVISORY_COLOR,
    };
    let text = TextStyle::new(FAILURE_TEXT_SIZE).with_color(color);
    let line = failure_line(keymap_failure);
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(FAILURE_ROW_HEIGHT))
            .gap(PANEL_COLUMN_GAP)
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::new().width(Sizing::GROW).height(Sizing::FIT),
                |builder| {
                    builder.text(Text::new(line, text.clone()).wrap(TextWrap::None));
                },
            );
            build_failure_action(builder, row_index, &keymap_failure.action, &text);
        },
    );
}

/// Renders the row's repair action as a button, so the word the row prints is
/// the thing the reader clicks.
fn build_failure_action(
    builder: &mut LayoutBuilder,
    row_index: usize,
    action: &KeymapFailureAction,
    text: &TextStyle,
) {
    let column = El::new()
        .width(Sizing::fixed(FAILURE_ACTION_COLUMN_WIDTH))
        .height(Sizing::FIT)
        .align_x(AlignX::Right);
    match action.label() {
        KeymapFailureActionLabel::NoAction => {
            builder.with(column, |_| {});
        },
        KeymapFailureActionLabel::Verb(verb) => {
            builder.with(column.button(failure_action_id(row_index)), |builder| {
                builder.text(Text::new(verb, text.clone()).wrap(TextWrap::None));
            });
        },
    }
}

/// Composes one failure row's single line of text.
///
/// The message leads and the location trails, because the line is clipped to the
/// panel width: whatever runs off the end is lost, and a reader who cannot see
/// what went wrong has nothing to act on.
fn failure_line(keymap_failure: &KeymapFailureRow) -> String {
    clip(
        &format!("{} — {}", keymap_failure.message, keymap_failure.location),
        FAILURE_LINE_MAX_CHARS,
    )
}

/// Shortens `text` to `max_chars`, marking the cut so a clipped row never reads
/// as the whole message.
fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept = max_chars.saturating_sub(1);
    text.chars().take(kept).chain(['…']).collect()
}

/// Authors the query field as an editable panel field.
///
/// The IME session edits inside this element, so the palette draws one box and
/// `hana_diegetic` renders the live buffer into it.
fn build_query_field(builder: &mut LayoutBuilder, view: &PaletteView<'_, '_, '_, '_>) {
    let (query_text, style) = if view.query.is_empty() {
        (
            PLACEHOLDER_TEXT,
            TextStyle::new(FIELD_TEXT_SIZE).with_color(PLACEHOLDER_COLOR),
        )
    } else {
        (
            view.query,
            TextStyle::new(FIELD_TEXT_SIZE).with_color(FIELD_TEXT_COLOR),
        )
    };
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(FIELD_HEIGHT))
            .padding(Padding::xy(FIELD_PADDING_X, FIELD_PADDING_Y))
            .align_y(AlignY::Center)
            .background(FIELD_BACKGROUND)
            .corner_radius(CornerRadius::all(FIELD_CORNER_RADIUS))
            .border(Border::all(FIELD_BORDER_WIDTH, FIELD_BORDER))
            .editable_field(
                FIELD_ID,
                ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new(FIELD_ID)),
            )
            .editor_text(EditorStateColors::new().focused(FIELD_TEXT_COLOR))
            .editor_selection(EditorStateColors::new().focused(FIELD_SELECTION_COLOR))
            .editor_caret(EditorStateColors::new().focused(FIELD_TEXT_COLOR)),
        |builder| {
            builder.text(Text::new(query_text, style.clone()).wrap(TextWrap::None));
        },
    );
}

fn build_separator(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(SEPARATOR_HEIGHT))
            .background(SEPARATOR_COLOR),
        |_| {},
    );
}

fn build_command_rows(builder: &mut LayoutBuilder, view: &PaletteView<'_, '_, '_, '_>) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(PANEL_ROW_GAP),
        |builder| match &view.presentation_input {
            PalettePresentationInput::AwaitingRegistryAssembly => build_note_row(
                builder,
                String::from("Command palette is starting; declared commands will appear shortly."),
                FAILURE_ADVISORY_COLOR,
            ),
            PalettePresentationInput::AssembledQuery {
                selection_outcome,
                command_rows,
                active_context,
            } => {
                build_note_row(
                    builder,
                    active_context_label(active_context),
                    FAILURE_ADVISORY_COLOR,
                );
                if let SelectionStatus::Message(status) = selection_status(*selection_outcome) {
                    build_note_row(builder, status.to_owned(), FAILURE_ADVISORY_COLOR);
                }
                for command_row in command_rows.iter().take(MAX_VISIBLE_COMMAND_ROWS) {
                    build_command_row(builder, command_row, *selection_outcome);
                }
                let hidden = command_rows.len().saturating_sub(MAX_VISIBLE_COMMAND_ROWS);
                if hidden > 0 {
                    build_note_row(
                        builder,
                        format!("{hidden} more — keep typing to narrow the list"),
                        HIDDEN_ROW_COLOR,
                    );
                }
            },
        },
    );
}

fn build_note_row(builder: &mut LayoutBuilder, note: String, color: Color) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(COMMAND_ROW_HEIGHT)),
        |builder| {
            builder.text(
                Text::new(note, TextStyle::new(COMMAND_TEXT_SIZE).with_color(color))
                    .wrap(TextWrap::None),
            );
        },
    );
}

/// One command row: its authored title on the left and its keyboard state on the right.
fn build_command_row(
    builder: &mut LayoutBuilder,
    command_row: &CommandPaletteRow<'_, '_, '_>,
    selection: PaletteSelectionOutcome<'_>,
) {
    let command = command_row.command();
    let is_selected = matches!(selection, PaletteSelectionOutcome::Selected(selected)
        if selected.id() == command.id());
    let mut row = El::row()
        .width(Sizing::GROW)
        .height(Sizing::fixed(COMMAND_ROW_HEIGHT))
        .gap(PANEL_COLUMN_GAP)
        .align_y(AlignY::Center);
    let title_color = if is_selected {
        row = row
            .background(SELECTED_COMMAND_BACKGROUND)
            .corner_radius(CornerRadius::all(FIELD_CORNER_RADIUS));
        SELECTED_COMMAND_TITLE_COLOR
    } else {
        COMMAND_TITLE_COLOR
    };

    builder.with(row, |builder| {
        builder.with(
            El::new().width(Sizing::GROW).height(Sizing::FIT),
            |builder| {
                builder.text(
                    Text::new(
                        clip(command.title(), COMMAND_TITLE_MAX_CHARS),
                        TextStyle::new(COMMAND_TEXT_SIZE).with_color(title_color),
                    )
                    .wrap(TextWrap::None),
                );
            },
        );
        builder.with(
            El::new()
                .width(Sizing::fixed(COMMAND_KEYSTROKE_COLUMN_WIDTH))
                .height(Sizing::FIT)
                .align_x(AlignX::Right),
            |builder| {
                if !matches!(command_row.binding(), PaletteBinding::Unbound) {
                    let presentation = palette_binding_presentation(command_row.binding());
                    let binding_text =
                        if matches!(command_row.binding(), PaletteBinding::BoundTo(_)) {
                            clip(presentation.text(), COMMAND_KEYSTROKE_MAX_CHARS)
                        } else {
                            presentation.to_string()
                        };
                    build_binding_state(builder, &binding_text);
                }
            },
        );
    });
}

/// Renders a typed binding state instead of pretending it is an authored
/// unbound command.
fn build_binding_state(builder: &mut LayoutBuilder, state: &str) {
    builder.text(
        Text::new(
            state,
            TextStyle::new(COMMAND_TEXT_SIZE).with_color(COMMAND_KEYSTROKE_COLOR),
        )
        .wrap(TextWrap::None),
    );
}

/// Formats Rubric's already-resolved context snapshot without consumer-side state lookup.
fn active_context_label(active_context: &ActiveKeymapContextState) -> String {
    match active_context {
        ActiveKeymapContextState::GlobalRouting => String::from("Context: global"),
        ActiveKeymapContextState::AwaitingStateDimensions => {
            String::from("Context: state dimensions loading")
        },
        ActiveKeymapContextState::StateDimensionsUnavailable { missing } => {
            format!("Context unavailable: {}", missing_dimension_names(missing))
        },
        ActiveKeymapContextState::Resolved(snapshot) => format!(
            "Context: {}",
            snapshot
                .values()
                .map(|(dimension, value)| format!("{}={}", dimension.as_str(), value.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Returns the line rendered above the command rows when the query resolved to
/// something the palette cannot dispatch.
const fn selection_status(selection: PaletteSelectionOutcome<'_>) -> SelectionStatus {
    match selection {
        PaletteSelectionOutcome::Selected(_) | PaletteSelectionOutcome::EmptyQuery => {
            SelectionStatus::Hidden
        },
        PaletteSelectionOutcome::NoMatch => {
            SelectionStatus::Message("No command matches this text.")
        },
        PaletteSelectionOutcome::NotPaletteInvocable => SelectionStatus::Message(
            "That command is hold-to-act, so it runs from its binding rather than here.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bevy::prelude::App;
    use bevy::prelude::States;
    use bevy::prelude::Window;
    use bevy::state::app::AppExtStates;
    use bevy::state::app::StatesPlugin;
    use hana_diegetic::LayoutTree;
    use hana_diegetic::LayoutTreeChange;
    use hana_rubric::ActiveKeymapContext;
    use hana_rubric::DiagnosticSeverity;
    use hana_rubric::EffectiveKeymapStatus;
    use hana_rubric::KeymapBindingUnavailability;
    use hana_rubric::KeymapBindings;
    use hana_rubric::KeymapPlugin;
    use hana_rubric::PaletteBinding;
    use hana_rubric::PaletteSelectionOutcome;
    use hana_rubric::query_command_palette;
    use strum::AsRefStr;
    use strum::EnumIter;
    use strum::EnumMessage;

    use super::FAILURE_LINE_MAX_CHARS;
    use super::FailureActionRow;
    use super::KeymapFailureAction;
    use super::KeymapFailureActionLabel;
    use super::KeymapFailureRow;
    use super::PANEL_EDGE_MARGIN;
    use super::PANEL_MAX_WIDTH;
    use super::PANEL_MIN_WIDTH;
    use super::SelectionStatus;
    use super::clip;
    use super::failure_action_id;
    use super::failure_action_row_index;
    use super::failure_line;
    use super::palette_panel_origin;
    use super::palette_panel_width;
    use super::palette_tree_awaiting_assembly;
    use super::palette_tree_for_query;
    use super::selection_status;
    use crate::command_palette;
    use crate::command_palette::test_support;
    use crate::command_palette::test_support::DISPATCH_COMMAND_TITLE;
    use crate::command_palette::test_support::HELD_COMMAND_ID;

    /// Pixel counts compare to a tenth of a pixel, which is finer than anything
    /// the layout can render and coarser than f32 rounding.
    const PIXEL_TOLERANCE: f32 = 0.1;
    /// Window dimensions for the axis a geometry test is not varying.
    const WINDOW_HEIGHT: f32 = 1080.0;
    const WINDOW_WIDTH: f32 = 1920.0;

    /// A context whose state is deliberately absent, exercising Rubric's
    /// public `ContextUnavailable` snapshot for the consumer renderer.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum AbsentPaletteContext {
        #[strum(message = "Only present when the panel test installs it")]
        Available,
    }

    /// A second absent dimension makes the renderer prove it preserves Rubric's sort order.
    #[derive(AsRefStr, Clone, Copy, Debug, EnumIter, EnumMessage, Eq, Hash, PartialEq, States)]
    #[strum(serialize_all = "snake_case")]
    enum AnotherAbsentPaletteContext {
        #[strum(message = "Only present when the panel test installs it")]
        Available,
    }

    /// The first resolved dimension rendered in the palette context note.
    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum PaletteApplicationContext {
        #[default]
        #[strum(message = "While the palette application is ready")]
        Ready,
    }

    /// The second resolved dimension rendered in the palette context note.
    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum PaletteInteractionContext {
        #[default]
        #[strum(message = "While the palette interaction is editing")]
        Editing,
    }

    /// A window of exactly this logical size, which is all
    /// [`palette_panel_width`] and [`palette_panel_origin`] read.
    fn window(width: f32, height: f32) -> Window {
        let mut window = Window::default();
        window.resolution.set(width, height);
        window
    }

    fn panel_width(window_width: f32) -> f32 {
        palette_panel_width(&window(window_width, WINDOW_HEIGHT))
    }

    fn panel_origin_x(window_width: f32) -> f32 {
        palette_panel_origin(&window(window_width, WINDOW_HEIGHT)).x
    }

    fn panel_origin_y(window_height: f32) -> f32 {
        palette_panel_origin(&window(WINDOW_WIDTH, window_height)).y
    }

    fn assert_pixels_eq(measured: f32, expected: f32) {
        assert!(
            (measured - expected).abs() < PIXEL_TOLERANCE,
            "expected {expected} pixels, measured {measured}"
        );
    }

    #[test]
    fn a_wide_window_holds_the_box_at_its_full_width_and_centers_it() {
        assert_pixels_eq(panel_width(1920.0), PANEL_MAX_WIDTH);
        assert_pixels_eq(panel_origin_x(1920.0), (1920.0 - PANEL_MAX_WIDTH) * 0.5);
    }

    #[test]
    fn a_narrow_window_shrinks_the_box_instead_of_pushing_it_off_the_left_edge() {
        let window_width = 700.0;

        assert_pixels_eq(
            panel_width(window_width),
            PANEL_EDGE_MARGIN.mul_add(-2.0, window_width),
        );
        assert_pixels_eq(panel_origin_x(window_width), PANEL_EDGE_MARGIN);
    }

    #[test]
    fn a_window_narrower_than_the_minimum_still_shows_the_left_edge() {
        assert_pixels_eq(panel_width(200.0), PANEL_MIN_WIDTH);
        assert_pixels_eq(panel_origin_x(200.0), 0.0);
    }

    #[test]
    fn the_box_sits_below_the_top_of_the_window_without_reaching_its_middle() {
        let window_height = 1080.0;

        let origin_y = panel_origin_y(window_height);

        assert!(origin_y > 0.0);
        assert!(origin_y < window_height * 0.5);
    }

    #[test]
    fn an_action_id_round_trips_to_its_row_index() {
        assert_eq!(
            failure_action_row_index(&failure_action_id(3)),
            FailureActionRow::Row(3)
        );
        assert_eq!(
            failure_action_row_index("query"),
            FailureActionRow::NotAFailureAction
        );
        assert_eq!(
            failure_action_row_index("keymap-failure-action-not-a-number"),
            FailureActionRow::NotAFailureAction
        );
    }

    #[test]
    fn only_an_actionable_failure_prints_a_clickable_verb() {
        assert_eq!(
            KeymapFailureAction::OpenFile(PathBuf::from("/tmp/keymap.jsonc")).label(),
            KeymapFailureActionLabel::Verb("Open")
        );
        assert_eq!(
            KeymapFailureAction::RevealDirectory(PathBuf::from("/tmp")).label(),
            KeymapFailureActionLabel::Verb("Reveal")
        );
        assert_eq!(
            KeymapFailureAction::NoAction.label(),
            KeymapFailureActionLabel::NoAction
        );
    }

    #[test]
    fn a_clipped_line_is_marked_as_cut() {
        assert_eq!(clip("abcdef", 6), "abcdef");
        assert_eq!(clip("abcdef", 4), "abc…");
    }

    #[test]
    fn only_a_rejection_the_reader_can_act_on_prints_a_status_line() {
        assert_eq!(
            selection_status(PaletteSelectionOutcome::EmptyQuery),
            SelectionStatus::Hidden
        );
        assert!(matches!(
            selection_status(PaletteSelectionOutcome::NoMatch),
            SelectionStatus::Message(_)
        ));
        assert!(matches!(
            selection_status(PaletteSelectionOutcome::NotPaletteInvocable),
            SelectionStatus::Message(_)
        ));
    }

    fn keymap_failure(severity: DiagnosticSeverity) -> KeymapFailureRow {
        KeymapFailureRow {
            severity,
            location: String::from("embedded defaults"),
            message: String::from("Unrecognized keymap block member."),
            action: KeymapFailureAction::NoAction,
        }
    }

    /// A row long enough to be clipped: a real advisory message next to a real
    /// `line:column` location.
    #[test]
    fn a_clipped_failure_row_keeps_its_whole_message_and_elides_only_the_location() {
        let mut long_failure = keymap_failure(DiagnosticSeverity::Advisory);
        long_failure.message =
            String::from("Unrecognized keymap block member `contxt`; did you mean `context`?");
        long_failure.location = String::from("command_palette.keymap.jsonc:27:8");

        let line = failure_line(&long_failure);

        assert!(
            line.chars().count() <= FAILURE_LINE_MAX_CHARS,
            "the row is one clipped line: {line}"
        );
        assert!(
            line.starts_with(&long_failure.message),
            "the message must survive the clip whole: {line}"
        );
        assert!(line.ends_with('…'), "only the location is elided: {line}");
    }

    /// The tree that proves app-owned failure rows can render before Rubric has
    /// assembled a registry to query.
    fn awaiting_tree(keymap_failures: &[KeymapFailureRow]) -> LayoutTree {
        palette_tree_awaiting_assembly("", keymap_failures, PANEL_MAX_WIDTH)
    }

    /// Builds one tree from the public borrowed query result, keeping it local
    /// to this renderer call just as production rebuilding does.
    fn shared_tree(query: &str, keymap_bindings: &KeymapBindings) -> LayoutTree {
        let app = test_support::palette_test_app();
        let query_result = query_command_palette(
            app.world().resource(),
            app.world().resource(),
            app.world().resource(),
            keymap_bindings,
            query,
        );
        palette_tree_for_query(query, &query_result, &[], PANEL_MAX_WIDTH)
    }

    fn awaiting_context() -> ActiveKeymapContext {
        let mut app = App::new();
        app.add_plugins(
            KeymapPlugin::new().with_state_dimension::<AbsentPaletteContext>("palette"),
        );
        app.finish();
        app.world_mut()
            .remove_resource::<ActiveKeymapContext>()
            .unwrap_or_default()
    }

    fn unavailable_context() -> ActiveKeymapContext {
        let mut app = App::new();
        app.add_plugins(
            KeymapPlugin::new()
                .with_state_dimension::<AbsentPaletteContext>("zebra")
                .with_state_dimension::<AnotherAbsentPaletteContext>("alpha"),
        );
        app.finish();
        app.update();
        app.world_mut()
            .remove_resource::<ActiveKeymapContext>()
            .unwrap_or_default()
    }

    fn resolved_context() -> ActiveKeymapContext {
        let mut app = App::new();
        app.add_plugins(StatesPlugin)
            .insert_state(PaletteApplicationContext::Ready)
            .insert_state(PaletteInteractionContext::Editing)
            .add_plugins(
                KeymapPlugin::new()
                    .with_state_dimension::<PaletteApplicationContext>("application")
                    .with_state_dimension::<PaletteInteractionContext>("interaction"),
            );
        app.finish();
        app.update();
        app.world_mut()
            .remove_resource::<ActiveKeymapContext>()
            .unwrap_or_default()
    }

    #[test]
    fn a_failure_row_colors_itself_by_its_severity_without_moving_anything() {
        let advisory = awaiting_tree(&[keymap_failure(DiagnosticSeverity::Advisory)]);
        let failure = awaiting_tree(&[keymap_failure(DiagnosticSeverity::Failure)]);

        assert_eq!(
            advisory.classify_change(&failure),
            LayoutTreeChange::VisualOnly
        );
        assert_eq!(
            advisory.classify_change(&awaiting_tree(&[keymap_failure(
                DiagnosticSeverity::Advisory
            )])),
            LayoutTreeChange::Identical
        );
    }

    #[test]
    fn each_keymap_failure_adds_its_own_row_above_the_field() {
        let none = awaiting_tree(&[]);
        let one = awaiting_tree(&[keymap_failure(DiagnosticSeverity::Advisory)]);
        let two = awaiting_tree(&[
            keymap_failure(DiagnosticSeverity::Advisory),
            keymap_failure(DiagnosticSeverity::Advisory),
        ]);

        assert!(one.len() > none.len());
        assert_eq!(two.len() - one.len(), one.len() - none.len());
    }

    #[test]
    fn unavailable_command_rows_render_their_distinct_reasons() {
        let app = test_support::palette_test_app();

        for (unavailability, label) in [
            (
                KeymapBindingUnavailability::AwaitingInitialLoad,
                "keymap loading",
            ),
            (
                KeymapBindingUnavailability::Unconfigured,
                "keymap unconfigured",
            ),
            (
                KeymapBindingUnavailability::MissingDefault,
                "embedded defaults missing",
            ),
            (
                KeymapBindingUnavailability::InvalidDefault,
                "embedded defaults invalid",
            ),
        ] {
            let unavailable = shared_tree(
                "Open Command Palette",
                &KeymapBindings::from(unavailability),
            );
            let presentation = format!("{unavailable:?}");

            assert_eq!(
                command_palette::keymap_unavailability_label(unavailability),
                label
            );
            assert!(presentation.contains(label));
        }

        let global = format!(
            "{:?}",
            shared_tree(
                "Open Command Palette",
                app.world().resource::<KeymapBindings>(),
            )
        );
        assert!(global.contains("Context: global"));
    }

    #[test]
    fn a_selected_shared_query_rebuilds_the_renderer_from_its_typed_outcome() {
        let app = test_support::palette_test_app();
        let keymap_bindings = app.world().resource::<KeymapBindings>();
        let unselected = shared_tree("", keymap_bindings);
        let selected = shared_tree("Open Command Palette", keymap_bindings);

        assert_ne!(
            unselected.classify_change(&selected),
            LayoutTreeChange::Identical
        );
    }

    #[test]
    fn awaiting_context_renders_a_state_instead_of_an_authored_unbound_row() {
        let app = test_support::palette_test_app();
        let awaiting_condition = awaiting_context();
        let query_result = query_command_palette(
            app.world().resource(),
            &awaiting_condition,
            app.world().resource(),
            app.world().resource(),
            "Open Command Palette",
        );
        let awaiting =
            palette_tree_for_query("Open Command Palette", &query_result, &[], PANEL_MAX_WIDTH);
        let unbound = shared_tree(
            "Open Command Palette",
            app.world().resource::<KeymapBindings>(),
        );

        assert_eq!(awaiting.len(), unbound.len() + 1);
        let presentation = format!("{awaiting:?}");
        assert!(presentation.contains("Context: state dimensions loading"));
        assert!(presentation.contains("state loading"));
    }

    #[test]
    fn unavailable_context_has_a_distinct_consumer_presentation() {
        let app = test_support::palette_test_app();
        let unavailable_context = unavailable_context();
        let unavailable_query = query_command_palette(
            app.world().resource(),
            &unavailable_context,
            app.world().resource(),
            app.world().resource(),
            DISPATCH_COMMAND_TITLE,
        );
        assert!(matches!(
            unavailable_query.rows()[0].binding(),
            PaletteBinding::StateDimensionsUnavailable(missing)
                if missing.iter().map(hana_rubric::ContextDimensionName::as_str).eq(["alpha", "zebra"])
        ));
        let unavailable_tree = palette_tree_for_query(
            DISPATCH_COMMAND_TITLE,
            &unavailable_query,
            &[],
            PANEL_MAX_WIDTH,
        );

        let unavailable_presentation = format!("{unavailable_tree:?}");
        assert!(unavailable_presentation.contains("Context unavailable: alpha, zebra"));
        assert!(unavailable_presentation.contains("state unavailable: alpha, zebra"));
    }

    #[test]
    fn a_keymap_unavailability_and_missing_state_dimensions_share_one_presentation() {
        let app = test_support::palette_test_app();
        let unavailable_context = unavailable_context();
        let unavailable_bindings =
            KeymapBindings::from(KeymapBindingUnavailability::InvalidDefault);
        let query_result = query_command_palette(
            app.world().resource(),
            &unavailable_context,
            app.world().resource(),
            &unavailable_bindings,
            DISPATCH_COMMAND_TITLE,
        );
        let presentation = format!(
            "{:?}",
            palette_tree_for_query(DISPATCH_COMMAND_TITLE, &query_result, &[], PANEL_MAX_WIDTH,)
        );

        assert!(matches!(
            query_result.rows()[0].binding(),
            PaletteBinding::KeymapUnavailable(KeymapBindingUnavailability::InvalidDefault)
        ));
        assert!(presentation.contains("embedded defaults invalid"));
        assert!(presentation.contains("Context unavailable: alpha, zebra"));
    }

    #[test]
    fn reflected_snapshot_failure_renders_its_context_and_typed_binding_state() {
        let app = test_support::palette_test_app();
        let unmaterializable_status = EffectiveKeymapStatus::UnmaterializableStateDimensions;
        let query_result = query_command_palette(
            app.world().resource(),
            app.world().resource(),
            &unmaterializable_status,
            app.world().resource(),
            "Open Command Palette",
        );
        let presentation = format!(
            "{:?}",
            palette_tree_for_query("Open Command Palette", &query_result, &[], PANEL_MAX_WIDTH,)
        );

        assert!(presentation.contains("Context: global"));
        assert!(presentation.contains("invalid state snapshot"));
    }

    #[test]
    fn resolved_two_dimension_context_values_render_in_dimension_name_order() {
        let app = test_support::palette_test_app();
        let resolved_context = resolved_context();
        let query_result = query_command_palette(
            app.world().resource(),
            &resolved_context,
            app.world().resource(),
            app.world().resource(),
            DISPATCH_COMMAND_TITLE,
        );
        let presentation = format!(
            "{:?}",
            palette_tree_for_query(DISPATCH_COMMAND_TITLE, &query_result, &[], PANEL_MAX_WIDTH,)
        );

        assert!(presentation.contains("Context: application=ready, interaction=editing"));
    }

    #[test]
    fn application_recovery_bound_and_unbound_rows_keep_distinct_presentations() {
        let app = test_support::palette_test_app();
        let bound = format!(
            "{:?}",
            shared_tree("Restart Example", app.world().resource::<KeymapBindings>())
        );
        let unbound = format!(
            "{:?}",
            shared_tree(
                "Open Command Palette",
                app.world().resource::<KeymapBindings>(),
            )
        );

        let mut application_recovery_app = App::new();
        application_recovery_app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(crate::keymap::FAIRY_DUST_DEFAULT_KEYMAP)
                .with_protected_command_binding(
                    crate::command_palette_recovery_command_id(),
                    crate::command_palette_recovery_keystroke(),
                ),
        );
        application_recovery_app.finish();
        let application_recovery = format!(
            "{:?}",
            shared_tree(
                "Open Command Palette",
                application_recovery_app
                    .world()
                    .resource::<KeymapBindings>(),
            )
        );

        assert!(bound.contains("ctrl-shift-r"));
        assert!(unbound.contains("Open Command Palette"));
        assert!(!unbound.contains("ctrl-shift-r"));
        assert!(application_recovery.contains(&format!(
            "protected {}",
            crate::command_palette_recovery_keystroke()
        )));
    }

    #[test]
    fn every_selection_outcome_has_its_own_consumer_presentation() {
        let app = test_support::palette_test_app();
        let keymap_bindings = app.world().resource::<KeymapBindings>();
        let empty = format!("{:?}", shared_tree("", keymap_bindings));
        let selected = format!("{:?}", shared_tree(DISPATCH_COMMAND_TITLE, keymap_bindings));
        let no_match = format!(
            "{:?}",
            shared_tree("this command does not exist", keymap_bindings)
        );
        let held = format!("{:?}", shared_tree(HELD_COMMAND_ID, keymap_bindings));

        assert!(empty.contains("Open Command Palette"));
        assert!(selected.contains(DISPATCH_COMMAND_TITLE));
        assert!(no_match.contains("No command matches this text."));
        assert!(held.contains("That command is hold-to-act"));
        assert_ne!(empty, selected);
        assert_ne!(selected, no_match);
        assert_ne!(no_match, held);
    }

    #[test]
    fn awaiting_registry_assembly_is_visibly_distinct_from_an_assembled_registry() {
        let awaiting = format!("{:?}", awaiting_tree(&[]));
        let app = test_support::palette_test_app();
        let assembled = format!(
            "{:?}",
            shared_tree("", app.world().resource::<KeymapBindings>())
        );

        assert!(awaiting.contains("Command palette is starting"));
        assert!(assembled.contains("Open Command Palette"));
        assert_ne!(awaiting, assembled);
    }
}
