use bevy::color::Alpha;
use bevy::ecs::query::QueryData;
use bevy::picking::hover::PickingInteraction;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Vec2;
use bevy::prelude::With;

use super::SliderDrag;
use super::SliderPushedValue;
use super::SliderState;
use super::projection;
use super::projection::SlotTranslationUnavailable;
use crate::DiegeticPanel;
use crate::cascade::Cascade;
use crate::cascade::Resolved;
use crate::widgets::VisualSlotId;
use crate::widgets::VisualSlotOverride;
use crate::widgets::WidgetDimmed;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusVisible;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetPressedAppearance;
use crate::widgets::WidgetSpec;
use crate::widgets::WidgetState;
use crate::widgets::WidgetVisualOverrides;
use crate::widgets::WidgetVisualSlots;
use crate::widgets::presentation as visual;

/// Everything [`present_slider_state`] reads from one slider entity: the
/// authored declaration, the four resolved state-appearance bundles, the applied value,
/// the solved slots, and the live hover / focus / disabled / dragging flags.
#[derive(QueryData)]
pub(in crate::widgets) struct SliderPresentation {
    authored:            &'static WidgetSpec,
    hovered:             &'static Resolved<WidgetHoveredAppearance>,
    pressed:             &'static Resolved<WidgetPressedAppearance>,
    focused_appearance:  &'static Resolved<WidgetFocusedAppearance>,
    disabled_appearance: &'static Resolved<WidgetDisabledAppearance>,
    authored_hovered:    &'static Cascade<WidgetHoveredAppearance>,
    authored_disabled:   &'static Cascade<WidgetDisabledAppearance>,
    kind:                &'static WidgetKind,
    owner:               &'static WidgetOf,
    state:               &'static SliderState,
    slots:               &'static WidgetVisualSlots,
    interaction:         Option<&'static PickingInteraction>,
    disabled:            Has<WidgetDisabled>,
    dimmed:              Has<WidgetDimmed>,
    focused:             Has<WidgetFocusVisible>,
    dragging:            Has<SliderDrag>,
}

/// Why the signal marker is hidden rather than placed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarkerHidden {
    /// No live signal is pushed (`SliderPushedValue::AtHandle`).
    AtHandle,
    /// The pushed value has no marker slot translation.
    Unplaced(SlotTranslationUnavailable),
}

fn marker_translation(
    slots: &WidgetVisualSlots,
    state: &SliderState,
) -> Result<Vec2, MarkerHidden> {
    let SliderPushedValue::PushedTo { value, .. } = state.pushed() else {
        return Err(MarkerHidden::AtHandle);
    };
    projection::slot_translation(slots, state, VisualSlotId::SLIDER_MARKER, value)
        .map_err(MarkerHidden::Unplaced)
}

fn present_signal_surfaces(
    desired: &mut WidgetVisualOverrides,
    slots: &WidgetVisualSlots,
    state: &SliderState,
) {
    let mut fill = match state.pushed() {
        SliderPushedValue::AtHandle => VisualSlotOverride::slider_fill(
            state.direction(),
            projection::fill_extent_fraction(slots, state).unwrap_or(0.0),
        ),
        SliderPushedValue::PushedTo { value, .. } => signal_interval(
            slots,
            state,
            VisualSlotId::SLIDER_FILL,
            state.value(),
            value,
        ),
    };
    if has_visible_reach_band(slots, state) {
        fill.fill_color = Some(crate::ControlPalette::system_dark().slider_cabled_fill);
    }
    if slots.element_index(VisualSlotId::SLIDER_FILL).is_some() {
        desired.set(VisualSlotId::SLIDER_FILL, fill);
    }
    if slots.element_index(VisualSlotId::SLIDER_BAND).is_some() {
        let band = match state.pushed() {
            SliderPushedValue::AtHandle => {
                VisualSlotOverride::slider_interval(state.direction(), 0.0, 0.0)
            },
            SliderPushedValue::PushedTo {
                band_start,
                band_end,
                ..
            } => signal_interval(
                slots,
                state,
                VisualSlotId::SLIDER_BAND,
                band_start,
                band_end,
            ),
        };
        desired.set(VisualSlotId::SLIDER_BAND, band);
    }
}

fn has_visible_reach_band(slots: &WidgetVisualSlots, state: &SliderState) -> bool {
    if slots.element_index(VisualSlotId::SLIDER_BAND).is_none() {
        return false;
    }
    match state.pushed() {
        SliderPushedValue::AtHandle => false,
        SliderPushedValue::PushedTo {
            band_start,
            band_end,
            ..
        } => !signal_interval(
            slots,
            state,
            VisualSlotId::SLIDER_BAND,
            band_start,
            band_end,
        )
        .fill_extent
        .is_empty(),
    }
}

fn present_reach_band_knob_edge(
    thumb: &mut VisualSlotOverride,
    slots: &WidgetVisualSlots,
    state: &SliderState,
    points_to_world: f32,
) {
    if has_visible_reach_band(slots, state) {
        // The dark inset rim separates a knob from the near-white reach band.
        // An absent or hidden band keeps the face's original bright rim.
        thumb.border_color = Some(crate::ControlPalette::system_dark().field_fill);
        if let (Some(border), Some(content)) = (
            slots.border_box(VisualSlotId::SLIDER_THUMB),
            slots.content_box(VisualSlotId::SLIDER_THUMB),
        ) {
            // The shared capsule has no border. Add an inset rim only when
            // the reach band needs separation from the capsule fill.
            let minimum_rim = border.height / 12.0;
            let scale = 1.25 * points_to_world;
            thumb.border_widths = Some(
                [
                    content.y - border.y,
                    border.x + border.width - content.x - content.width,
                    border.y + border.height - content.y - content.height,
                    content.x - border.x,
                ]
                .map(|width| width.max(minimum_rim) * scale),
            );
        }
    }
}

/// A shadow is visible only when the Add reach interval is actually drawn.
fn present_knob_shadow(
    desired: &mut WidgetVisualOverrides,
    slots: &WidgetVisualSlots,
    state: &SliderState,
    render_offset: Option<Vec2>,
    panel: Option<&DiegeticPanel>,
) {
    if slots
        .element_index(VisualSlotId::SLIDER_KNOB_SHADOW)
        .is_none()
    {
        return;
    }
    let shown = has_visible_reach_band(slots, state);
    let drop = panel
        .and_then(|panel| {
            slots
                .border_box(VisualSlotId::SLIDER_THUMB)
                .map(|thumb| thumb.height * panel.points_to_world() * 0.08)
        })
        .unwrap_or(0.0);
    desired.set(
        VisualSlotId::SLIDER_KNOB_SHADOW,
        VisualSlotOverride {
            offset: render_offset.map(|offset| offset - Vec2::Y * drop),
            fill_color: Some(if shown {
                bevy::color::Color::BLACK.with_alpha(0.35)
            } else {
                bevy::color::Color::NONE
            }),
            ..VisualSlotOverride::default()
        },
    );
}

fn signal_interval(
    slots: &WidgetVisualSlots,
    state: &SliderState,
    surface: VisualSlotId,
    from: f32,
    to: f32,
) -> VisualSlotOverride {
    let interval = VisualSlotOverride::slider_interval;
    let (Ok(from), Ok(to)) = (
        projection::fill_extent_fraction_at_value(slots, state, surface, from),
        projection::fill_extent_fraction_at_value(slots, state, surface, to),
    ) else {
        return interval(state.direction(), 0.0, 0.0);
    };
    interval(state.direction(), from.min(to), from.max(to))
}

/// Maps each changed slider's state onto its retained visual overrides.
///
/// Runs after `WidgetSystems::FocusCommandsApplied`, so application and
/// keyboard-traversal indicator commands, as well as pointer-driven indicator
/// removal, are visible in the same frame. Each `Changed` query and
/// [`RemovedComponents`] stream is consumed here, so a quiet frame never walks
/// the live sliders. Hover reads the all-pointer
/// [`PickingInteraction`] aggregate and pressed reads the private [`SliderDrag`]
/// marker; [`SliderCaptures`] stays lifecycle authority and is never read for
/// presentation. Writes go through [`visual::write_widget_overrides`],
/// which compares immutably first, so an unchanged state never marks
/// [`WidgetVisualOverrides`] changed. The thumb slot receives only its
/// value-derived translation; the thumb element's state appearance supplies
/// its focus border.
pub(in crate::widgets) fn present_slider_state(
    changed: Query<
        (Entity, &WidgetKind),
        (
            With<WidgetOf>,
            Or<(
                Changed<WidgetSpec>,
                Changed<Resolved<WidgetHoveredAppearance>>,
                Changed<Resolved<WidgetPressedAppearance>>,
                Changed<Resolved<WidgetFocusedAppearance>>,
                Changed<Resolved<WidgetDisabledAppearance>>,
                Changed<WidgetVisualSlots>,
                Changed<PickingInteraction>,
                Changed<WidgetFocusVisible>,
                Changed<WidgetDisabled>,
                Changed<WidgetDimmed>,
                Changed<SliderState>,
                Changed<SliderDrag>,
            )>,
        ),
    >,
    sliders: Query<SliderPresentation, With<WidgetOf>>,
    kinds: Query<&WidgetKind, With<WidgetOf>>,
    panels: Query<&DiegeticPanel>,
    mut removed_interactions: RemovedComponents<PickingInteraction>,
    mut removed_focus: RemovedComponents<WidgetFocusVisible>,
    mut removed_disabled: RemovedComponents<WidgetDisabled>,
    mut removed_dimmed: RemovedComponents<WidgetDimmed>,
    mut removed_drags: RemovedComponents<SliderDrag>,
    mut overrides: Query<&mut WidgetVisualOverrides>,
    mut commands: Commands,
) {
    let dirty = visual::dirty_widgets(
        changed.iter().map(|(entity, kind)| (entity, *kind)),
        removed_interactions
            .read()
            .chain(removed_focus.read())
            .chain(removed_disabled.read())
            .chain(removed_dimmed.read())
            .chain(removed_drags.read()),
        &kinds,
        WidgetKind::Slider,
    );
    for entity in dirty {
        let Ok(present) = sliders.get(entity) else {
            continue;
        };
        if *present.kind != WidgetKind::Slider {
            continue;
        }
        if !matches!(present.authored, WidgetSpec::Slider(_)) {
            continue;
        }
        let (state, slots, visually_disabled, focused) = (
            present.state,
            present.slots,
            present.disabled,
            present.focused,
        );
        let mut desired = WidgetVisualOverrides::default();
        let active = [
            focused.then_some(WidgetState::Focused),
            matches!(
                present.interaction,
                Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
            )
            .then_some(WidgetState::Hovered),
            present.dragging.then_some(WidgetState::Pressed),
            visually_disabled.then_some(WidgetState::Disabled),
        ];
        let panel = panels.get(present.owner.panel()).ok();
        visual::resolve_part_overrides(
            &mut desired,
            slots,
            present.hovered,
            present.pressed,
            present.focused_appearance,
            present.disabled_appearance,
            present.authored_hovered,
            present.authored_disabled,
            &active,
            present.dimmed,
            panel,
        );
        // Always resolve the thumb slot so a same-id slider re-authored from a
        // marked thumb to no thumb clears its stale translation through the
        // shared `write_widget_overrides` replacement path. The layout-frame
        // delta is converted once through the owning `DiegeticPanel` to the
        // panel-local render frame the retained routes consume; an absent
        // thumb, a missing panel, or an invalid scale yields the default
        // override, which the immutable-before-mutable writer treats as a
        // clear rather than a manufactured offset.
        let render_offset =
            projection::thumb_translation(slots, state)
                .ok()
                .and_then(|layout_delta| {
                    panel.and_then(|panel| {
                        visual::layout_delta_to_render_offset(layout_delta, panel.points_to_world())
                    })
                });
        let mut thumb_override =
            render_offset.map_or_else(VisualSlotOverride::default, |offset| VisualSlotOverride {
                offset: Some(offset),
                ..VisualSlotOverride::default()
            });
        if let Some(panel) = panel {
            present_reach_band_knob_edge(
                &mut thumb_override,
                slots,
                state,
                panel.points_to_world(),
            );
        }
        if thumb_override != VisualSlotOverride::default() {
            desired.set(VisualSlotId::SLIDER_THUMB, thumb_override);
        }
        present_knob_shadow(&mut desired, slots, state, render_offset, panel);
        present_signal_surfaces(&mut desired, slots, state);
        let marker_override = match marker_translation(slots, state)
            .ok()
            .and_then(|layout_delta| {
                panel.and_then(|panel| {
                    visual::layout_delta_to_render_offset(layout_delta, panel.points_to_world())
                })
            }) {
            Some(offset) => VisualSlotOverride {
                offset: Some(offset),
                ..Default::default()
            },
            None => VisualSlotOverride {
                fill_color: Some(bevy::color::Color::NONE),
                ..Default::default()
            },
        };
        if slots.element_index(VisualSlotId::SLIDER_MARKER).is_some() {
            desired.set(VisualSlotId::SLIDER_MARKER, marker_override);
        }
        visual::write_widget_overrides(entity, desired, &mut overrides, &mut commands);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::change_detection::Tick;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::EntityEvent;
    use bevy::prelude::Handle;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;

    use super::super::*;
    use crate::AlignX;
    use crate::AlignY;
    use crate::Appearance;
    use crate::Border;
    use crate::BorderColor;
    use crate::ClearWidgetFocus;
    use crate::ComputedDiegeticPanel;
    use crate::ControlMetrics;
    use crate::ControlPalette;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelBuildError;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::RequestWidgetFocus;
    use crate::Sizing;
    use crate::Text;
    use crate::TextStyle;
    use crate::WidgetHoveredAppearance;
    use crate::WidgetInteractivity;
    use crate::WidgetPressedAppearance;
    use crate::cascade::Cascade;
    use crate::layout::BoundingBox;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ButtonPress;
    use crate::widgets::StateAppearance;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetDimmed;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::controls::slider::projection;
    use crate::widgets::controls::slider::projection::ProjectionUnavailable;

    fn slider0() -> Slider { Slider::new(0.0..=1.0).value(0.5) }

    fn button_and_slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .background(Color::WHITE)
                .button("act")
                .hovered(Appearance::new().background(STATE_HOVER_FILL))
                .pressed(Appearance::new().background(STATE_PRESS_FILL)),
            |_| {},
        );
        // The slider's state layers override background and border color, so the
        // element authors both a normal background and a normal border for the
        // surface those overrides patch.
        builder.with(
            apply_state_appearance(
                El::new()
                    .size(20.0, 10.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget("level", slider0()),
            ),
            |_| {},
        );
        builder.with(
            apply_state_appearance(
                El::new()
                    .size(20.0, 10.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget("peer", slider0()),
            ),
            |_| {},
        );
        builder.build()
    }

    fn button_press_isolation_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .background(Color::WHITE)
                .button("act")
                .pressed(Appearance::new().background(STATE_PRESS_FILL)),
            |_| {},
        );
        builder.with(
            apply_state_appearance(
                El::new()
                    .size(20.0, 10.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget("level", slider0()),
            ),
            |children| {
                children.with(El::new().background(Color::WHITE), |_| {});
            },
        );
        let mut tree = builder.build();
        let pressed_part_index = tree.len() - 1;
        assert!(tree.set_element_state_appearance(
            pressed_part_index,
            StateAppearance {
                hovered: Cascade::Override(WidgetHoveredAppearance::new(
                    Appearance::new().background(STATE_HOVER_FILL),
                )),
                pressed: Cascade::Override(WidgetPressedAppearance::new(
                    Appearance::new().background(STATE_PRESS_FILL),
                )),
                ..StateAppearance::default()
            },
        ));
        tree
    }

    /// The same unrelated button paired with a state-layered slider that marks
    /// one thumb, so a peer test can watch both the slider's root and thumb
    /// records react while the button stays untouched.
    fn button_and_thumb_slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(20.0, 10.0).button("act"), |_| {});
        builder.with(
            apply_state_appearance(
                El::overlay()
                    .size(40.0, 16.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .alignment(AlignX::Left, AlignY::Center)
                    .widget("level", plain_slider(0.5)),
            ),
            |builder| {
                builder.with(El::new().size(8.0, 8.0).id(THUMB_ID).slider_thumb(), |_| {});
            },
        );
        builder.build()
    }

    #[derive(Default, Resource)]
    struct RecordedProposals(Vec<(Entity, f32, bool, SliderProposalSource)>);

    fn record_proposal(change: On<SliderChangeRequested>, mut recorded: ResMut<RecordedProposals>) {
        recorded.0.push((
            change.event_target(),
            change.value.get(),
            change.is_final,
            change.source,
        ));
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .init_resource::<RecordedProposals>()
            .add_observer(record_proposal);
        app
    }

    fn range(start: f32, end: f32) -> SliderRange {
        SliderRange::new(start, end).expect("range should validate")
    }
    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &str) -> Entity {
        let id = PanelElementId::named(id);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
            .expect("widget should be reified")
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    const STATE_DISABLED_FILL: Color = Color::srgb(0.2, 0.3, 0.4);
    const STATE_DISABLED_LABEL_TEXT: Color = Color::srgb(0.7, 0.5, 0.2);
    const STATE_FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const STATE_HOVER_FILL: Color = Color::srgb(0.2, 0.4, 0.6);
    const STATE_THUMB_FOCUS_BORDER: Color = Color::srgb(0.95, 0.75, 0.1);
    const STATE_SLIDER_LABEL_SLOT: VisualSlotId = VisualSlotId::new(1);
    const STATE_SLIDER_TRACK_SLOT: VisualSlotId = VisualSlotId::new(2);
    const THUMB_ID: &str = "thumb";

    fn plain_slider(value: f32) -> Slider {
        Slider::new(0.0..=1.0)
            .value(value)
            .direction(SliderDirection::LeftToRight)
    }

    fn thumb_slider_tree(id: &str, slider: Slider, thumb_size: f32) -> LayoutTree {
        thumb_slider_tree_with(id, slider, thumb_size, |element| element)
    }

    fn thumb_slider_tree_with(
        id: &str,
        slider: Slider,
        thumb_size: f32,
        configure: impl FnOnce(SliderElement<Overlay>) -> SliderElement<Overlay>,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            configure(
                El::overlay()
                    .size(40.0, 16.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .alignment(AlignX::Left, AlignY::Center)
                    .widget(id, slider),
            ),
            |builder| {
                builder.with(
                    El::new()
                        .size(thumb_size, 8.0)
                        .background(Color::BLACK)
                        .border(Border::all(1.0, Color::BLACK))
                        .id(THUMB_ID)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn disabled_slider_state_tree(slider: Slider) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::column()
                .size(40.0, 16.0)
                .background(Color::srgb(0.15, 0.25, 0.35))
                .widget("level", slider)
                .disabled(
                    Appearance::new()
                        .background(STATE_DISABLED_FILL)
                        .text_color(STATE_DISABLED_LABEL_TEXT),
                ),
            |builder| {
                builder.text(
                    Text::new("Level", TextStyle::new(10.0).with_color(Color::WHITE))
                        .layout(El::new().visual_slot(STATE_SLIDER_LABEL_SLOT)),
                );
                builder.with(
                    El::overlay()
                        .size(32.0, 2.0)
                        .background(Color::srgb(0.1, 0.7, 0.9))
                        .visual_slot(STATE_SLIDER_TRACK_SLOT),
                    |builder| {
                        builder.with(
                            El::new()
                                .size(32.0, 2.0)
                                .background(ControlPalette::system_dark().accent)
                                .slider_fill(),
                            |_| {},
                        );
                    },
                );
                builder.with(
                    El::new()
                        .size(8.0, 8.0)
                        .background(Color::srgb(0.9, 0.3, 0.1))
                        .border(Border::all(Px(1.0), Color::srgb(0.1, 0.2, 0.3)))
                        .id(THUMB_ID)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn build_world_panel(tree: LayoutTree) -> Result<DiegeticPanel, PanelBuildError> {
        DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
    }

    fn slots_of(app: &App, widget: Entity) -> WidgetVisualSlots {
        app.world()
            .get::<WidgetVisualSlots>(widget)
            .cloned()
            .expect("widget should carry visual slots")
    }

    fn root_override(app: &App, widget: Entity) -> Option<VisualSlotOverride> {
        let slots = app.world().get::<WidgetVisualSlots>(widget)?;
        let element_index = slots.element_index(VisualSlotId::SLIDER_ROOT)?;
        app.world()
            .get::<WidgetVisualOverrides>(widget)
            .and_then(|overrides| {
                overrides
                    .element_overrides()
                    .iter()
                    .find(|(index, _)| *index == element_index)
                    .map(|(_, value)| value.clone())
            })
    }

    fn thumb_offset(app: &App, widget: Entity) -> Option<Vec2> {
        app.world()
            .get::<WidgetVisualOverrides>(widget)
            .and_then(|overrides| overrides.get(VisualSlotId::SLIDER_THUMB).cloned())
            .and_then(|value| value.offset)
    }

    fn marker_slider_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.25)),
            |builder| {
                builder.with(
                    El::new()
                        .size(8.0, 8.0)
                        .background(Color::BLACK)
                        .slider_thumb(),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .size(2.0, 8.0)
                        .background(Color::WHITE)
                        .slider_marker(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    #[test]
    fn logarithmic_slider_thumb_fill_and_reachable_band_use_real_value_positions() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(200.0, 40.0);
        builder.slider(
            "size",
            Slider::new(0.01..=10.0).logarithmic().value(0.1),
            &ControlMetrics::from_label_size(13.0_f32),
            SliderReachableBand::Drawn,
        );
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "size");
        let slots = slots_of(&app, widget);
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("root content");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb bounds");
        let travel = slots
            .border_box(VisualSlotId::SLIDER_THUMB_HIT)
            .unwrap_or(thumb);
        let center_at = |fraction: f32| {
            (content.width - travel.width).mul_add(fraction, travel.width.mul_add(0.5, content.x))
        };
        let thumb_before = thumb_offset(&app, widget).expect("placed log thumb");
        assert_close(
            thumb_before.x,
            (center_at(1.0 / 3.0) - thumb.center().0) * panel_points_to_world(&app, panel),
        );
        let fill = slots
            .border_box(VisualSlotId::SLIDER_FILL)
            .expect("fill bounds");
        let overrides = app
            .world()
            .get::<WidgetVisualOverrides>(widget)
            .expect("thumb and fill overrides");
        let interval = overrides
            .get(VisualSlotId::SLIDER_FILL)
            .expect("authored fill")
            .fill_interval()
            .ok()
            .expect("authored value fill interval");
        assert_close(interval.start_fraction, 0.0);
        assert_close(
            interval.end_fraction,
            (center_at(1.0 / 3.0) - fill.x) / fill.width,
        );
        app.world_mut()
            .get_mut::<SliderState>(widget)
            .expect("log state")
            .set_pushed(SliderPushedValue::PushedTo {
                value:      1.0,
                band_start: 0.01,
                band_end:   10.0,
            });
        app.update();
        assert_eq!(
            thumb_offset(&app, widget),
            Some(thumb_before),
            "live signal leaves the authored real-value knob in place"
        );
        let overrides = app
            .world()
            .get::<WidgetVisualOverrides>(widget)
            .expect("pushed overrides");
        for (slot, start, end) in [
            (VisualSlotId::SLIDER_FILL, 1.0 / 3.0, 2.0 / 3.0),
            (VisualSlotId::SLIDER_BAND, 0.0, 1.0),
        ] {
            let surface = slots.border_box(slot).expect("signal surface");
            let interval = overrides
                .get(slot)
                .expect("signal override")
                .fill_interval()
                .ok()
                .expect("signal interval");
            assert_close(
                interval.start_fraction,
                (center_at(start) - surface.x) / surface.width,
            );
            assert_close(
                interval.end_fraction,
                (center_at(end) - surface.x) / surface.width,
            );
        }
    }

    #[test]
    fn unavailable_live_values_do_not_draw_a_segment_from_zero() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(200.0, 40.0);
        builder.slider(
            "level",
            slider0(),
            &ControlMetrics::from_label_size(13.0_f32),
            SliderReachableBand::Drawn,
        );
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        app.world_mut()
            .get_mut::<SliderState>(widget)
            .expect("slider state")
            .set_pushed(SliderPushedValue::PushedTo {
                value:      f32::NAN,
                band_start: 0.0,
                band_end:   1.0,
            });
        app.update();
        let overrides = app
            .world()
            .get::<WidgetVisualOverrides>(widget)
            .expect("visual overrides");
        assert!(
            overrides
                .get(VisualSlotId::SLIDER_FILL)
                .expect("fill override")
                .fill_extent
                .is_empty()
        );
    }

    #[test]
    fn a_pushed_value_places_the_marker_and_leaves_the_thumb() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, marker_slider_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let thumb_before = thumb_offset(&app, widget);
        app.world_mut()
            .get_mut::<SliderState>(widget)
            .expect("slider state")
            .set_pushed(SliderPushedValue::PushedTo {
                value:      0.75,
                band_start: 0.0,
                band_end:   1.0,
            });
        app.update();
        let overrides = app
            .world()
            .get::<WidgetVisualOverrides>(widget)
            .expect("visual overrides");
        assert_eq!(thumb_offset(&app, widget), thumb_before);
        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("visual slots");
        let state = app
            .world()
            .get::<SliderState>(widget)
            .expect("slider state");
        let marker_center = slots
            .border_box(VisualSlotId::SLIDER_MARKER)
            .expect("marker bounds")
            .center()
            .0
            + super::marker_translation(slots, state)
                .expect("marker translation")
                .x;
        let mut pushed_thumb = state.clone();
        pushed_thumb
            .set_value(0.75)
            .expect("pushed value should apply");
        let thumb_center = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb bounds")
            .center()
            .0
            + super::super::projection::thumb_translation(slots, &pushed_thumb)
                .expect("thumb translation")
                .x;
        assert_close(marker_center, thumb_center);
        assert!(
            overrides
                .get(VisualSlotId::SLIDER_MARKER)
                .and_then(|value| value.offset)
                .is_some()
        );
        assert_close(
            app.world()
                .get::<SliderState>(widget)
                .expect("slider state")
                .value(),
            0.25,
        );
    }

    #[test]
    fn at_handle_hides_the_marker() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, marker_slider_tree());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let override_value = app
            .world()
            .get::<WidgetVisualOverrides>(widget)
            .expect("visual overrides")
            .get(VisualSlotId::SLIDER_MARKER)
            .expect("marker override");
        assert_eq!(override_value.fill_color, Some(Color::NONE));
        assert_eq!(override_value.offset, None);
    }

    fn set_slider_value(app: &mut App, widget: Entity, value: f32) {
        let mut state = app
            .world_mut()
            .get_mut::<SliderState>(widget)
            .expect("widget should carry slider state");
        state.set_value(value).expect("value should apply");
    }

    #[test]
    fn thumb_translation_tracks_applied_value_without_relayout() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, thumb_slider_tree("level", plain_slider(0.0), 8.0));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        // At the directed range start the thumb sits at its authored center.
        let start = thumb_offset(&app, widget).expect("thumb override present");
        assert_close(start.x, 0.0);
        assert_close(start.y, 0.0);

        let computed_before = app
            .world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed());

        // Applying the range end moves the thumb across the full directed
        // travel: content extent minus the thumb extent.
        set_slider_value(&mut app, widget, 1.0);
        app.update();
        let end = thumb_offset(&app, widget).expect("thumb override present");
        let slots = slots_of(&app, widget);
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("root content box");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb border box");
        let scale = panel_points_to_world(&app, panel);
        let expected_end = to_render_offset(Vec2::new(content.width - thumb.width, 0.0), scale);
        assert_close(end.x, expected_end.x);
        assert_close(end.y, expected_end.y);

        // No relayout ran for the value change: the panel's computed output
        // kept its change tick.
        let computed_after = app
            .world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed());
        assert_eq!(computed_after, computed_before);
    }

    #[test]
    fn metric_sized_slider_presented_thumb_reaches_both_track_ends_inside_slider_box() {
        let metrics = ControlMetrics::from_label_size(13.0_f32);
        let mut builder = LayoutBuilder::new(200.0, 40.0);
        builder.with(
            El::overlay()
                .size(metrics.slider.width, metrics.slider.thumb.height)
                .alignment(AlignX::Left, AlignY::Center)
                .widget("level", plain_slider(1.0)),
            |builder| {
                builder.with(
                    El::overlay()
                        .width(Sizing::GROW)
                        .height(Sizing::fixed(metrics.slider.thumb.height))
                        .alignment(AlignX::Center, AlignY::Center),
                    |builder| {
                        builder.with(
                            El::overlay()
                                .width(Sizing::GROW)
                                .height(Sizing::fixed(metrics.slider.track_height))
                                .id("track")
                                .scroll_x(0.0),
                            |builder| {
                                builder.with(El::new().size(1.0, 1.0), |_| {});
                            },
                        );
                    },
                );
                builder.with(
                    El::overlay()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .alignment(AlignX::Left, AlignY::Center),
                    |builder| {
                        builder.with(
                            El::new()
                                .size(metrics.slider.thumb.width, metrics.slider.thumb.height)
                                .id(THUMB_ID)
                                .slider_thumb(),
                            |_| {},
                        );
                    },
                );
            },
        );
        let mut app = test_app();
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("panel should solve");
        let slider = computed.content_bounds().expect("slider box should solve");
        let track = computed
            .scroll_records()
            .iter()
            .find(|record| record.id == PanelElementId::named("track"))
            .expect("track should solve")
            .bounds;
        let slots = slots_of(&app, widget);
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb box should solve");
        let thumb_index = slots
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("thumb element should solve");
        let scale = panel_points_to_world(&app, panel);

        for (value, track_end) in [(0.0, track.x), (1.0, track.x + track.width)] {
            set_slider_value(&mut app, widget, value);
            app.update();
            // The render index is what the retained draw routes consume. Its
            // offset is in world units, while solved boxes use layout points.
            let offset = indexed_offset(&app, panel, thumb_index).unwrap_or(Vec2::ZERO);
            let presented_x = thumb.x + offset.x / scale;
            let presented_y = thumb.y - offset.y / scale;
            let presented_edge = if value == 0.0 {
                presented_x
            } else {
                presented_x + thumb.width
            };
            assert!(
                (presented_edge - track_end).abs() < 1e-3,
                "value {value}: presented thumb edge {presented_edge} misses track end {track_end}",
            );
            assert!(presented_x >= slider.x - 1e-3);
            assert!(presented_x + thumb.width <= slider.x + slider.width + 1e-3);
            assert!(presented_y >= slider.y - 1e-3);
            assert!(presented_y + thumb.height <= slider.y + slider.height + 1e-3);
        }
    }

    #[test]
    fn thumb_element_hover_fill_composes_with_the_value_derived_offset() {
        let mut app = test_app();
        let mut tree = thumb_slider_tree_with("level", plain_slider(0.25), 8.0, |element| element);
        let authored_thumb_index = tree.len() - 1;
        assert!(tree.set_element_state_appearance(
            authored_thumb_index,
            StateAppearance {
                hovered: Cascade::Override(WidgetHoveredAppearance::new(
                    Appearance::new().background(STATE_HOVER_FILL),
                )),
                ..StateAppearance::default()
            },
        ));
        let panel = spawn_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let thumb_index = slots_of(&app, widget)
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("thumb element index");
        assert_eq!(thumb_index, authored_thumb_index);
        let initial_offset = thumb_offset(&app, widget).expect("initial thumb slot offset");
        assert_ne!(
            initial_offset,
            Vec2::ZERO,
            "the authored initial slider value must move the thumb",
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let hovered = indexed_root(&app, panel, thumb_index).expect("hovered thumb index entry");
        assert_eq!(hovered.offset, Some(initial_offset));
        assert_eq!(hovered.fill_color, Some(STATE_HOVER_FILL));

        set_slider_value(&mut app, widget, 0.75);
        app.update();

        let moved_offset = thumb_offset(&app, widget).expect("moved thumb slot offset");
        assert_ne!(
            moved_offset, initial_offset,
            "changing the applied slider value must move the thumb offset",
        );
        let moved = indexed_root(&app, panel, thumb_index).expect("moved thumb index entry");
        assert_eq!(moved.offset, Some(moved_offset));
        assert_eq!(moved.fill_color, Some(STATE_HOVER_FILL));
    }

    #[test]
    fn slider_root_hover_fill_does_not_repaint_unstyled_thumb() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, |element| {
                element.hovered(Appearance::new().background(STATE_HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        let root_index = slots
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("slider root slot");
        let thumb_index = slots
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("slider thumb slot");

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            indexed_root(&app, panel, root_index).and_then(|value| value.fill_color),
            Some(STATE_HOVER_FILL),
        );
        assert_eq!(
            indexed_root(&app, panel, thumb_index).and_then(|value| value.fill_color),
            None,
            "the thumb keeps its authored fill during root hover",
        );
    }

    #[test]
    fn focused_thumb_border_layers_with_the_value_offset() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .background(Color::WHITE)
                .border(Border::all(1.0, Color::WHITE))
                .alignment(AlignX::Left, AlignY::Center)
                .widget("level", plain_slider(0.5)),
            |builder| {
                builder.with(
                    builder
                        .child(El::new())
                        .size(8.0, 8.0)
                        .background(Color::BLACK)
                        .border(Border::all(1.0, Color::BLACK))
                        .id(THUMB_ID)
                        .slider_thumb()
                        .focused(BorderColor(STATE_THUMB_FOCUS_BORDER)),
                    |_| {},
                );
            },
        );
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let thumb_index = slots_of(&app, widget)
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("thumb element index");
        let idle = indexed_root(&app, panel, thumb_index).expect("thumb override present");
        assert!(
            idle.offset.is_some(),
            "the slider presenter must provide the thumb's value-derived offset",
        );
        assert_eq!(idle.border_color, None);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();

        let focused =
            indexed_root(&app, panel, thumb_index).expect("focused thumb override present");
        assert_eq!(focused.offset, idle.offset);
        assert_eq!(focused.border_color, Some(STATE_THUMB_FOCUS_BORDER));

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();

        let unfocused =
            indexed_root(&app, panel, thumb_index).expect("unfocused thumb override present");
        assert_eq!(unfocused.offset, idle.offset);
        assert_eq!(
            unfocused.border_color, None,
            "clearing keyboard focus must drop the focused thumb border",
        );
    }

    #[test]
    fn disabled_slider_appearance_routes_each_property_to_its_authored_part() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, disabled_slider_state_tree(plain_slider(0.5)));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        let root_index = slots
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("slider root should expose its visual slot");
        let label_index = slots
            .element_index(STATE_SLIDER_LABEL_SLOT)
            .expect("slider label should expose its visual slot");
        let track_index = slots
            .element_index(STATE_SLIDER_TRACK_SLOT)
            .expect("slider track should expose its visual slot");
        let fill_index = slots
            .element_index(VisualSlotId::SLIDER_FILL)
            .expect("slider fill should expose its visual slot");
        let thumb_index = slots
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("slider thumb should expose its visual slot");

        app.world_mut()
            .entity_mut(widget)
            .insert(WidgetDisabled::test_marker());
        app.update();

        let root = indexed_root(&app, panel, root_index).expect("disabled root override");
        assert_eq!(root.fill_color, Some(STATE_DISABLED_FILL));
        let track = indexed_root(&app, panel, track_index).expect("disabled track override");
        assert_eq!(track.fill_color, Some(STATE_DISABLED_FILL));
        let fill = indexed_root(&app, panel, fill_index).expect("disabled fill override");
        assert_eq!(fill.fill_color, Some(STATE_DISABLED_FILL));
        let thumb = indexed_root(&app, panel, thumb_index).expect("disabled thumb override");
        assert_eq!(thumb.fill_color, Some(STATE_DISABLED_FILL));
        let label = indexed_root(&app, panel, label_index).expect("disabled label override");
        assert_eq!(label.text_color, Some(STATE_DISABLED_LABEL_TEXT));

        app.world_mut()
            .entity_mut(widget)
            .remove::<WidgetDisabled>();
        app.update();

        assert_eq!(
            indexed_root(&app, panel, root_index)
                .and_then(|override_value| override_value.fill_color),
            None,
        );
        assert_eq!(
            indexed_root(&app, panel, track_index)
                .and_then(|override_value| override_value.fill_color),
            None,
        );
        assert_eq!(
            indexed_root(&app, panel, fill_index)
                .and_then(|override_value| override_value.fill_color),
            None,
        );
        assert_eq!(
            indexed_root(&app, panel, thumb_index)
                .and_then(|override_value| override_value.fill_color),
            None,
        );
        assert_eq!(
            indexed_root(&app, panel, label_index)
                .and_then(|override_value| override_value.text_color),
            None,
        );
    }

    #[test]
    fn empty_disabled_appearance_dims_track_fill_and_thumb() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .background(ControlPalette::system_dark().track)
                .widget("level", plain_slider(0.5))
                .disabled(Appearance::new()),
            |builder| {
                builder.with(
                    El::new()
                        .size(32.0, 2.0)
                        .background(ControlPalette::system_dark().accent)
                        .slider_fill(),
                    |_| {},
                );
                builder.with(
                    El::new()
                        .size(8.0, 8.0)
                        .background(ControlPalette::system_dark().knob)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        let panel = spawn_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);

        app.world_mut()
            .entity_mut(widget)
            .insert(WidgetDisabled::test_marker());
        app.update();

        for slot in [
            VisualSlotId::SLIDER_ROOT,
            VisualSlotId::SLIDER_FILL,
            VisualSlotId::SLIDER_THUMB,
        ] {
            let index = slots.element_index(slot).expect("slider slot should exist");
            let visual = indexed_root(&app, panel, index).expect("disabled visual override");
            assert!(visual.dim_fill, "disabled slider part should dim its fill");
        }

        app.world_mut()
            .entity_mut(widget)
            .remove::<WidgetDisabled>();
        app.update();
        let root = slots
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("slider root should exist");
        assert_eq!(indexed_root(&app, panel, root), None);
    }

    #[test]
    fn dimmed_slider_fill_keeps_enabled_appearance_and_input() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, disabled_slider_state_tree(plain_slider(0.5)));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let fill_index = slots_of(&app, widget)
            .element_index(VisualSlotId::SLIDER_FILL)
            .expect("slider fill slot");

        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());
        assert_eq!(
            indexed_root(&app, panel, fill_index).and_then(|visual| visual.fill_color),
            None,
        );

        app.world_mut().entity_mut(widget).remove::<WidgetDimmed>();
        app.update();
        assert_eq!(
            indexed_root(&app, panel, fill_index).and_then(|visual| visual.fill_color),
            None,
        );
    }

    #[test]
    fn root_state_override_follows_hover_and_clears() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, |element| {
                element
                    .hovered(Appearance::new().background(STATE_HOVER_FILL))
                    .focused(Appearance::new().border_color(STATE_FOCUS_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_HOVER_FILL),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::None);
        app.update();
        assert_eq!(root_override(&app, widget), None);
    }

    #[test]
    fn oversized_thumb_centers_on_content_and_blocks_projection() {
        let mut app = test_app();
        // A 60-unit thumb inside a 38-unit content box exceeds the content.
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree("level", plain_slider(0.5), 60.0),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("root content box");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb border box");
        assert!(
            thumb.width >= content.width,
            "the thumb must exceed the content for this case",
        );

        // The thumb centers on the content box's active axis.
        let offset = thumb_offset(&app, widget).expect("thumb override present");
        let scale = panel_points_to_world(&app, panel);
        let expected =
            to_render_offset(Vec2::new(content.center().0 - thumb.center().0, 0.0), scale);
        assert_close(offset.x, expected.x);
        assert_close(offset.y, expected.y);

        // The same geometry leaves pointer projection unavailable.
        assert_eq!(
            projection::project_pointer_value(
                Vec2::new(content.center().0, content.center().1),
                content,
                projection::ThumbExtent::Marked(thumb.width),
                range(0.0, 1.0),
                SliderDirection::LeftToRight,
            ),
            Err(ProjectionUnavailable),
        );
    }

    #[test]
    fn orphan_thumb_is_rejected_with_its_id() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(8.0, 8.0).id("stray").slider_thumb(), |_| {});
        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::SliderThumbOutsideSlider(id))
                if id == PanelElementId::named("stray")
        ));
    }

    #[test]
    fn second_thumb_is_rejected_with_the_slider_id() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.5)),
            |builder| {
                builder.with(El::new().size(8.0, 8.0).id("a").slider_thumb(), |_| {});
                builder.with(El::new().size(8.0, 8.0).id("b").slider_thumb(), |_| {});
            },
        );
        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::SliderHasMultipleThumbs(id))
                if id == PanelElementId::named("level")
        ));
    }

    #[test]
    fn orphan_fill_is_rejected_with_its_id() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(8.0, 8.0).id("stray").slider_fill(), |_| {});
        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::SliderFillOutsideSlider(id))
                if id == PanelElementId::named("stray")
        ));
    }

    #[test]
    fn second_fill_is_rejected_with_the_slider_id() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.5)),
            |builder| {
                builder.with(El::new().size(8.0, 8.0).id("a").slider_fill(), |_| {});
                builder.with(El::new().size(8.0, 8.0).id("b").slider_fill(), |_| {});
            },
        );
        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::SliderHasMultipleFills(id))
                if id == PanelElementId::named("level")
        ));
    }

    #[test]
    fn slider_state_builders_default_missing_surfaces() {
        let mut background = LayoutBuilder::new(100.0, 50.0);
        background.with(
            El::new()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.5))
                .hovered(Appearance::new().background(Color::WHITE)),
            |_| {},
        );
        assert!(build_world_panel(background.build()).is_ok());

        let mut border = LayoutBuilder::new(100.0, 50.0);
        border.with(
            El::new()
                .size(40.0, 16.0)
                .background(Color::WHITE)
                .widget("level", plain_slider(0.5))
                .hovered(Appearance::new().border_color(Color::WHITE)),
            |_| {},
        );
        assert!(build_world_panel(border.build()).is_ok());

        let mut border_width = LayoutBuilder::new(100.0, 50.0);
        border_width.with(
            El::new()
                .size(40.0, 16.0)
                .background(Color::WHITE)
                .widget("level", plain_slider(0.5))
                .focused(Appearance::new().border_width(Px(2.0))),
            |_| {},
        );
        assert!(build_world_panel(border_width.build()).is_ok());

        let mut material = LayoutBuilder::new(100.0, 50.0);
        material.with(
            El::new()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.5))
                .hovered(Appearance::new().material(Handle::<StandardMaterial>::default())),
            |_| {},
        );
        assert!(build_world_panel(material.build()).is_ok());
    }

    /// Hover, press, and focus layers shared by the peer-isolation trees.
    fn apply_state_appearance<L>(element: SliderElement<L>) -> SliderElement<L> {
        element
            .hovered(Appearance::new().background(STATE_HOVER_FILL))
            .pressed(Appearance::new().background(STATE_PRESS_FILL))
            .focused(Appearance::new().border_color(STATE_FOCUS_BORDER))
    }

    /// A slider element in child layout `L`.
    type SliderElement<L> = El<L, crate::WidgetElement<Slider>>;
    type Overlay = crate::Overlay;
    type Row = crate::Row;

    const STATE_FOCUS_FILL: Color = Color::srgb(0.15, 0.15, 0.4);
    const STATE_PRESS_FILL: Color = Color::srgb(0.6, 0.3, 0.1);
    const STATE_DISABLED_BORDER: Color = Color::srgb(0.35, 0.35, 0.4);

    fn directed_slider(direction: SliderDirection, value: f32) -> Slider {
        Slider::new(0.0..=1.0).value(value).direction(direction)
    }

    /// Every state layer a precedence test needs, authored on the slider
    /// element.
    fn apply_all_state_layers<L>(element: SliderElement<L>) -> SliderElement<L> {
        element
            .focused(
                Appearance::new()
                    .background(STATE_FOCUS_FILL)
                    .border_color(STATE_FOCUS_BORDER),
            )
            .hovered(Appearance::new().background(STATE_HOVER_FILL))
            .pressed(Appearance::new().background(STATE_PRESS_FILL))
            .disabled(Appearance::new().border_color(STATE_DISABLED_BORDER))
    }

    /// A slider whose marked thumb is authored at the content-box center — away
    /// from every directed range start — so each expected presentation
    /// translation is computed from the solved authored center rather than read
    /// back as a zero offset. The root is non-square, so an active-axis swap
    /// yields a wrong translation.
    fn centered_thumb_tree(slider: Slider, thumb_width: f32, thumb_height: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 24.0)
                .background(Color::WHITE)
                .border(Border::all(1.0, Color::WHITE))
                .alignment(AlignX::Center, AlignY::Center)
                .widget("level", slider),
            |builder| {
                builder.with(
                    El::new()
                        .size(thumb_width, thumb_height)
                        .id(THUMB_ID)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    /// Like [`centered_thumb_tree`] but authors the thumb at the top-left corner
    /// so an oversized thumb keeps a non-zero centering delta on both axes.
    fn corner_thumb_tree(slider: Slider, thumb_width: f32, thumb_height: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 24.0)
                .background(Color::WHITE)
                .border(Border::all(1.0, Color::WHITE))
                .alignment(AlignX::Left, AlignY::Top)
                .widget("level", slider),
            |builder| {
                builder.with(
                    El::new()
                        .size(thumb_width, thumb_height)
                        .id(THUMB_ID)
                        .slider_thumb(),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    fn headless_slider_tree(id: &str, slider: Slider) -> LayoutTree {
        headless_slider_tree_with(id, slider, |element| element)
    }

    fn headless_slider_tree_with(
        id: &str,
        slider: Slider,
        configure: impl FnOnce(SliderElement<Row>) -> SliderElement<Row>,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            configure(
                El::new()
                    .size(40.0, 16.0)
                    .background(Color::WHITE)
                    .border(Border::all(1.0, Color::WHITE))
                    .widget(id, slider),
            ),
            |_| {},
        );
        builder.build()
    }

    /// Active-axis start, extent, thumb extent, and authored thumb center for
    /// `direction`, read from solved slot boxes.
    fn active_axis_metrics(
        content: BoundingBox,
        thumb: BoundingBox,
        direction: SliderDirection,
    ) -> (f32, f32, f32, f32) {
        match direction {
            SliderDirection::LeftToRight | SliderDirection::RightToLeft => {
                (content.x, content.width, thumb.width, thumb.center().0)
            },
            SliderDirection::BottomToTop | SliderDirection::TopToBottom => {
                (content.y, content.height, thumb.height, thumb.center().1)
            },
        }
    }

    /// First-principles expected thumb translation for a normalized value,
    /// independent of the production endpoint math under test.
    fn expected_thumb_offset(
        content: BoundingBox,
        thumb: BoundingBox,
        direction: SliderDirection,
        fraction: f32,
    ) -> Vec2 {
        let (content_start, content_extent, thumb_extent, authored_center) =
            active_axis_metrics(content, thumb, direction);
        let start_coordinate = thumb_extent.mul_add(0.5, content_start);
        let end_coordinate = thumb_extent.mul_add(-0.5, content_start + content_extent);
        let (travel_start, travel_end) = match direction {
            SliderDirection::LeftToRight | SliderDirection::TopToBottom => {
                (start_coordinate, end_coordinate)
            },
            SliderDirection::RightToLeft | SliderDirection::BottomToTop => {
                (end_coordinate, start_coordinate)
            },
        };
        let desired_center = fraction.mul_add(travel_end - travel_start, travel_start);
        let delta = desired_center - authored_center;
        match direction {
            SliderDirection::LeftToRight | SliderDirection::RightToLeft => Vec2::new(delta, 0.0),
            SliderDirection::BottomToTop | SliderDirection::TopToBottom => Vec2::new(0.0, delta),
        }
    }

    fn active_offset(offset: Vec2, direction: SliderDirection) -> f32 {
        match direction {
            SliderDirection::LeftToRight | SliderDirection::RightToLeft => offset.x,
            SliderDirection::BottomToTop | SliderDirection::TopToBottom => offset.y,
        }
    }

    fn cross_offset(offset: Vec2, direction: SliderDirection) -> f32 {
        match direction {
            SliderDirection::LeftToRight | SliderDirection::RightToLeft => offset.y,
            SliderDirection::BottomToTop | SliderDirection::TopToBottom => offset.x,
        }
    }

    fn thumb_override(app: &App, widget: Entity) -> Option<VisualSlotOverride> {
        app.world()
            .get::<WidgetVisualOverrides>(widget)
            .and_then(|overrides| overrides.get(VisualSlotId::SLIDER_THUMB).cloned())
    }

    /// The owning panel's live layout-points-to-world scale, the same factor
    /// the presentation writer converts thumb deltas through.
    fn panel_points_to_world(app: &App, panel: Entity) -> f32 {
        app.world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should carry its role component")
            .points_to_world()
    }

    /// Converts a layout-frame thumb delta into the panel-local render frame
    /// independently of the production conversion: X scales, Y scales and
    /// inverts.
    fn to_render_offset(layout_delta: Vec2, points_to_world: f32) -> Vec2 {
        Vec2::new(
            layout_delta.x * points_to_world,
            -layout_delta.y * points_to_world,
        )
    }

    fn indexed_offset(app: &App, panel: Entity, element_index: usize) -> Option<Vec2> {
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .and_then(|value| value.offset)
    }

    fn indexed_root(app: &App, panel: Entity, element_index: usize) -> Option<VisualSlotOverride> {
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .cloned()
    }

    fn computed_tick(app: &App, panel: Entity) -> Option<Tick> {
        app.world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed())
    }

    fn override_tick(app: &App, widget: Entity) -> Option<Tick> {
        app.world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed())
    }

    fn assert_override_quiet(app: &mut App, widget: Entity, tick: Option<Tick>) {
        app.update();
        assert_eq!(
            override_tick(app, widget),
            tick,
            "a quiet frame must not write the slider override component",
        );
    }

    const ALL_DIRECTIONS: [SliderDirection; 4] = [
        SliderDirection::LeftToRight,
        SliderDirection::RightToLeft,
        SliderDirection::TopToBottom,
        SliderDirection::BottomToTop,
    ];

    #[test]
    fn thumb_tracks_applied_value_at_both_endpoints_in_all_directions() {
        for direction in ALL_DIRECTIONS {
            let mut app = test_app();
            let panel = spawn_panel(
                &mut app,
                centered_thumb_tree(directed_slider(direction, 0.0), 8.0, 8.0),
            );
            app.update();
            let widget = resolve_widget(&mut app, panel, "level");
            let slots = slots_of(&app, widget);
            let content = slots
                .content_box(VisualSlotId::SLIDER_ROOT)
                .expect("root content box");
            let thumb = slots
                .border_box(VisualSlotId::SLIDER_THUMB)
                .expect("thumb border box");
            let thumb_index = slots
                .element_index(VisualSlotId::SLIDER_THUMB)
                .expect("thumb element index");
            let root_box_before = slots
                .border_box(VisualSlotId::SLIDER_ROOT)
                .expect("root border box");
            let transform_before = *app
                .world()
                .get::<Transform>(widget)
                .expect("widget transform");
            let computed_before = computed_tick(&app, panel);

            // The authored center is not the directed start, so value 0 already
            // computes a non-zero translation back to the range start.
            let at_start = thumb_offset(&app, widget).expect("thumb override present");
            let scale = panel_points_to_world(&app, panel);
            let expected_start =
                to_render_offset(expected_thumb_offset(content, thumb, direction, 0.0), scale);
            assert_close(at_start.x, expected_start.x);
            assert_close(at_start.y, expected_start.y);
            assert!(
                active_offset(at_start, direction).abs() > f32::EPSILON,
                "{direction:?} value-0 translation must be computed from the authored center",
            );

            set_slider_value(&mut app, widget, 1.0);
            app.update();

            let at_end = thumb_offset(&app, widget).expect("thumb override present");
            let expected_end =
                to_render_offset(expected_thumb_offset(content, thumb, direction, 1.0), scale);
            assert_close(at_end.x, expected_end.x);
            assert_close(at_end.y, expected_end.y);
            assert_close(cross_offset(at_end, direction), 0.0);
            assert_eq!(
                indexed_offset(&app, panel, thumb_index),
                Some(at_end),
                "{direction:?} thumb translation must reach the visual override index",
            );

            // The value change moved only the thumb record: slider hit geometry
            // and the panel's computed layout stayed put.
            let slots_after = slots_of(&app, widget);
            assert_eq!(
                slots_after.border_box(VisualSlotId::SLIDER_ROOT),
                Some(root_box_before),
                "{direction:?} slider hit geometry must not move",
            );
            assert_eq!(
                *app.world()
                    .get::<Transform>(widget)
                    .expect("widget transform"),
                transform_before,
            );
            assert_eq!(
                computed_tick(&app, panel),
                computed_before,
                "{direction:?} value change must not relayout",
            );
        }
    }

    #[test]
    fn same_id_reauthor_adopts_new_direction_without_recreating_entity() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            centered_thumb_tree(directed_slider(SliderDirection::LeftToRight, 0.0), 8.0, 8.0),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert_eq!(
            app.world()
                .get::<SliderState>(widget)
                .map(SliderState::direction),
            Some(SliderDirection::LeftToRight),
        );
        let ltr_offset = thumb_offset(&app, widget).expect("thumb override present");

        // Re-author the same id with a reversed direction; the live value (0.0)
        // is preserved and the retained entity adopts the new direction.
        app.world_mut()
            .commands()
            .set_tree(
                panel,
                centered_thumb_tree(directed_slider(SliderDirection::RightToLeft, 0.0), 8.0, 8.0),
            )
            .expect("reversed tree should be accepted");
        app.update();
        assert_eq!(
            resolve_widget(&mut app, panel, "level"),
            widget,
            "the retained widget keeps its entity across the direction change",
        );
        assert_eq!(
            app.world()
                .get::<SliderState>(widget)
                .map(SliderState::direction),
            Some(SliderDirection::RightToLeft),
        );

        let slots = slots_of(&app, widget);
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("root content box");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb border box");
        let rtl_offset = thumb_offset(&app, widget).expect("thumb override present");
        let scale = panel_points_to_world(&app, panel);
        let expected_rtl = to_render_offset(
            expected_thumb_offset(content, thumb, SliderDirection::RightToLeft, 0.0),
            scale,
        );
        assert_close(rtl_offset.x, expected_rtl.x);
        assert_close(rtl_offset.y, expected_rtl.y);
        assert!(
            (rtl_offset.x - ltr_offset.x).abs() > f32::EPSILON,
            "the reversed direction must move the value-0 thumb to the opposite end",
        );
    }

    #[test]
    fn oversized_thumb_centers_in_all_directions_preserving_geometry_and_projection() {
        for direction in ALL_DIRECTIONS {
            let mut app = test_app();
            // A 50x40 thumb exceeds the 38x22 content box on both axes.
            let panel = spawn_panel(
                &mut app,
                corner_thumb_tree(directed_slider(direction, 0.3), 50.0, 40.0),
            );
            app.update();
            let widget = resolve_widget(&mut app, panel, "level");
            let slots = slots_of(&app, widget);
            let content = slots
                .content_box(VisualSlotId::SLIDER_ROOT)
                .expect("root content box");
            let thumb = slots
                .border_box(VisualSlotId::SLIDER_THUMB)
                .expect("thumb border box");
            let (content_start, content_extent, thumb_extent, authored_center) =
                active_axis_metrics(content, thumb, direction);
            assert!(
                thumb_extent >= content_extent,
                "{direction:?} thumb must meet or exceed the content on the active axis",
            );
            let expected_delta = content_extent.mul_add(0.5, content_start) - authored_center;
            let scale = panel_points_to_world(&app, panel);
            let expected_layout = match direction {
                SliderDirection::LeftToRight | SliderDirection::RightToLeft => {
                    Vec2::new(expected_delta, 0.0)
                },
                SliderDirection::BottomToTop | SliderDirection::TopToBottom => {
                    Vec2::new(0.0, expected_delta)
                },
            };
            let expected_active =
                active_offset(to_render_offset(expected_layout, scale), direction);

            // Active-axis centering with the cross axis preserved, and only an
            // offset is written so the authored draw depth and material survive.
            let value = thumb_override(&app, widget).expect("thumb override present");
            let offset = value.offset.expect("offset present");
            assert_close(active_offset(offset, direction), expected_active);
            assert_close(cross_offset(offset, direction), 0.0);
            assert_eq!(value.fill_color, None);
            assert_eq!(value.border_color, None);
            assert!(value.material.is_none());
            assert!(value.texture.is_none());

            // The same geometry leaves pointer projection unavailable.
            assert_eq!(
                projection::project_pointer_value(
                    Vec2::new(content.center().0, content.center().1),
                    content,
                    projection::ThumbExtent::Marked(thumb_extent),
                    range(0.0, 1.0),
                    direction,
                ),
                Err(ProjectionUnavailable),
                "{direction:?} oversized thumb must block projection",
            );

            // A value change keeps the thumb centered, the hit geometry fixed,
            // and runs no relayout.
            let root_box_before = slots
                .border_box(VisualSlotId::SLIDER_ROOT)
                .expect("root border box");
            let computed_before = computed_tick(&app, panel);
            set_slider_value(&mut app, widget, 0.9);
            app.update();
            let after = thumb_offset(&app, widget).expect("thumb override present");
            assert_close(active_offset(after, direction), expected_active);
            assert_close(cross_offset(after, direction), 0.0);
            assert_eq!(
                slots_of(&app, widget).border_box(VisualSlotId::SLIDER_ROOT),
                Some(root_box_before),
            );
            assert_eq!(
                computed_tick(&app, panel),
                computed_before,
                "{direction:?} value change on an oversized thumb must not relayout",
            );
        }
    }

    #[test]
    fn reauthoring_from_thumb_to_no_thumb_clears_the_stale_thumb_override() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, thumb_slider_tree("level", plain_slider(0.5), 8.0));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert!(thumb_offset(&app, widget).is_some());
        let thumb_index = slots_of(&app, widget).element_index(VisualSlotId::SLIDER_THUMB);
        assert!(thumb_index.is_some());

        // Re-author the same id slider without a marked thumb; the widget is
        // retained and its stale thumb translation is cleared through the shared
        // `write_widget_overrides` replacement path.
        app.world_mut()
            .commands()
            .set_tree(panel, headless_slider_tree("level", plain_slider(0.5)))
            .expect("headless tree should be accepted");
        app.update();
        app.update();
        assert_eq!(resolve_widget(&mut app, panel, "level"), widget);
        assert_eq!(
            thumb_offset(&app, widget),
            None,
            "the stale thumb override must be cleared",
        );
        assert_eq!(thumb_override(&app, widget), None);
        if let Some(index) = thumb_index {
            assert!(indexed_root(&app, panel, index).is_none());
        }

        // A later quiet frame leaves the cleared override untouched.
        let tick_after_clear = app
            .world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed());
        app.update();
        assert_eq!(thumb_offset(&app, widget), None);
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<WidgetVisualOverrides>()
                .map(|overrides| overrides.last_changed()),
            tick_after_clear,
            "a quiet frame must not re-touch the cleared override",
        );
    }

    #[test]
    fn slider_value_changes_and_drags_do_not_rebuild_peer_overrides() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, button_and_slider_tree());
        app.update();
        let slider = resolve_widget(&mut app, panel, "level");
        let peer_slider = resolve_widget(&mut app, panel, "peer");
        let button = resolve_widget(&mut app, panel, "act");

        // Both peers receive distinct non-default overrides before removal.
        // Removing them exposes any cross-widget presenter wake-up because
        // neither peer can restore its override without becoming dirty.
        app.world_mut()
            .entity_mut(button)
            .insert(PickingInteraction::Hovered);
        app.world_mut()
            .entity_mut(peer_slider)
            .insert(PickingInteraction::Hovered);
        app.update();
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(button).is_some());
        assert!(
            app.world()
                .get::<WidgetVisualOverrides>(peer_slider)
                .is_some()
        );

        app.world_mut()
            .entity_mut(button)
            .remove::<WidgetVisualOverrides>();
        app.world_mut()
            .entity_mut(peer_slider)
            .remove::<WidgetVisualOverrides>();
        set_slider_value(&mut app, slider, 0.75);
        app.update();

        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_none(),
            "a slider value change must not rebuild the peer button override",
        );
        assert!(
            app.world()
                .get::<WidgetVisualOverrides>(peer_slider)
                .is_none(),
            "a slider value change must not rebuild the peer slider override",
        );

        app.world_mut().entity_mut(slider).insert(SliderDrag);
        app.update();

        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_none(),
            "a slider drag must not rebuild the peer button override",
        );
        assert!(
            app.world()
                .get::<WidgetVisualOverrides>(peer_slider)
                .is_none(),
            "a slider drag must not rebuild the peer slider override",
        );
    }

    #[test]
    fn button_edges_and_slider_drags_do_not_rebuild_cross_kind_overrides() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, button_and_slider_tree());
        app.update();
        let slider = resolve_widget(&mut app, panel, "level");
        let button = resolve_widget(&mut app, panel, "act");

        app.world_mut().entity_mut(slider).insert(SliderDrag);
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(slider).is_some());

        app.world_mut()
            .entity_mut(slider)
            .remove::<WidgetVisualOverrides>();
        app.world_mut()
            .entity_mut(button)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(slider).is_none(),
            "a button interaction change must not rebuild a live slider override",
        );

        app.world_mut()
            .entity_mut(button)
            .remove::<PickingInteraction>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(slider).is_none(),
            "a button interaction removal must not rebuild a live slider override",
        );

        app.world_mut()
            .entity_mut(button)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(button).is_some());

        app.world_mut()
            .entity_mut(button)
            .remove::<WidgetVisualOverrides>();
        app.world_mut().entity_mut(slider).remove::<SliderDrag>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_none(),
            "a slider drag removal must not rebuild a live button override",
        );

        app.world_mut().entity_mut(slider).insert(SliderDrag);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_none(),
            "a slider drag insertion must not rebuild a live button override",
        );
    }

    #[test]
    fn button_press_edges_do_not_rebuild_slider_overrides() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, button_press_isolation_tree());
        app.update();
        let slider = resolve_widget(&mut app, panel, "level");
        let button = resolve_widget(&mut app, panel, "act");
        assert!(app.world().get::<WidgetVisualOverrides>(slider).is_none());

        app.world_mut()
            .entity_mut(slider)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(slider).is_some());
        app.world_mut()
            .entity_mut(slider)
            .remove::<WidgetVisualOverrides>();

        app.world_mut().entity_mut(button).insert(ButtonPress);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_some(),
            "a real button press proves the button presenter ran",
        );

        app.world_mut().entity_mut(slider).insert(ButtonPress);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(slider).is_none(),
            "a slider ButtonPress must not build a button-presenter override",
        );

        app.world_mut().entity_mut(slider).remove::<ButtonPress>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(slider).is_none(),
            "removing a slider ButtonPress must not build a button-presenter override",
        );
    }

    #[test]
    fn slider_drag_and_value_edges_update_overrides_once_then_leave_quiet_frames_unchanged() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, apply_state_appearance),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let idle_tick = override_tick(&app, widget);
        assert!(idle_tick.is_some());
        assert_override_quiet(&mut app, widget, idle_tick);

        app.world_mut().entity_mut(widget).insert(SliderDrag);
        app.update();
        let drag_tick = override_tick(&app, widget);
        assert!(drag_tick.is_some());
        assert_ne!(drag_tick, idle_tick);
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(STATE_PRESS_FILL),
        );
        assert_override_quiet(&mut app, widget, drag_tick);

        app.world_mut().entity_mut(widget).remove::<SliderDrag>();
        app.update();
        let released_tick = override_tick(&app, widget);
        assert!(released_tick.is_some());
        assert_ne!(released_tick, drag_tick);
        assert_eq!(root_override(&app, widget), None);
        assert_override_quiet(&mut app, widget, released_tick);

        set_slider_value(&mut app, widget, 0.75);
        app.update();
        let changed_value_tick = override_tick(&app, widget);
        assert!(changed_value_tick.is_some());
        assert_ne!(changed_value_tick, released_tick);
        assert_override_quiet(&mut app, widget, changed_value_tick);
    }

    #[test]
    fn slider_state_change_leaves_the_peer_button_untouched() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, button_and_thumb_slider_tree());
        app.update();
        let slider = resolve_widget(&mut app, panel, "level");
        let button = resolve_widget(&mut app, panel, "act");

        let slider_root = slots_of(&app, slider)
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("slider root element index");
        let thumb_index = slots_of(&app, slider)
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("slider thumb element index");
        let button_root = slots_of(&app, button)
            .element_index(VisualSlotId::BUTTON_ROOT)
            .expect("button root element index");

        // The idle slider owns no root override; its marked thumb already carries
        // a translation override for the authored default value.
        assert_eq!(root_override(&app, slider), None);
        assert_eq!(indexed_root(&app, panel, slider_root), None);
        let initial_thumb_offset = thumb_offset(&app, slider).expect("initial thumb offset");
        assert_eq!(
            indexed_offset(&app, panel, thumb_index),
            Some(initial_thumb_offset)
        );

        // The untouched peer button owns no override records.
        assert!(app.world().get::<WidgetVisualOverrides>(button).is_none());
        assert_eq!(indexed_root(&app, panel, button_root), None);

        // Change the slider's value and drive it pressed through the private drag
        // marker; only the slider's own records react.
        set_slider_value(&mut app, slider, 1.0);
        app.world_mut().entity_mut(slider).insert(SliderDrag);
        app.update();

        assert!(
            root_override(&app, slider).is_some(),
            "the slider's pressed state writes its root override",
        );
        assert!(
            indexed_root(&app, panel, slider_root).is_some(),
            "the slider's root override reaches the retained index",
        );
        let moved_thumb_offset = thumb_offset(&app, slider).expect("moved thumb offset");
        assert_ne!(
            moved_thumb_offset, initial_thumb_offset,
            "the applied-value change moves the thumb override to the new value",
        );
        assert_eq!(
            indexed_offset(&app, panel, thumb_index),
            Some(moved_thumb_offset),
            "the moved thumb override reaches the retained index",
        );

        // The peer button gains no override component and no retained index entry.
        assert!(
            app.world().get::<WidgetVisualOverrides>(button).is_none(),
            "the peer button must gain no visual override component",
        );
        assert_eq!(
            indexed_root(&app, panel, button_root),
            None,
            "the peer button must retain no visual override index entry",
        );
    }

    #[test]
    fn slider_state_precedence_layers_independently_per_property() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, apply_all_state_layers),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        // Keyboard focus alone: focused fill plus focused border.
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_FOCUS_FILL),
                border_color: Some(STATE_FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        // Hover replaces the fill; the border falls through to focus.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_HOVER_FILL),
                border_color: Some(STATE_FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "hover replaces the focused fill while the border falls through to focus",
        );

        // Pressed is driven by `SliderDrag` and replaces the fill; the border
        // still falls through to focus.
        app.world_mut().entity_mut(widget).insert(SliderDrag);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_PRESS_FILL),
                border_color: Some(STATE_FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "the private drag marker drives the pressed fill",
        );

        // Disabled replaces the border; the fill falls through to press.
        let result =
            app.world_mut()
                .run_system_once(move |mut writer: crate::PanelWidgetWriter| {
                    writer.override_interactivity(widget, WidgetInteractivity::Disabled)
                });
        assert_eq!(result.ok(), Some(true));
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_PRESS_FILL),
                border_color: Some(STATE_DISABLED_BORDER),
                dim_fill: true,
                ..VisualSlotOverride::default()
            }),
            "disabled replaces the border while the fill falls through to press",
        );
    }

    #[test]
    fn slider_pressed_presentation_is_driven_by_slider_drag() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, |element| {
                element.pressed(Appearance::new().background(STATE_PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        assert_eq!(root_override(&app, widget), None);

        // The all-pointer aggregate reporting `Pressed` does not drive the
        // slider's pressed layer; only the private `SliderDrag` marker does.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Pressed);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "the pointer-pressed aggregate alone must not show pressed state",
        );

        app.world_mut().entity_mut(widget).insert(SliderDrag);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(STATE_PRESS_FILL),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut().entity_mut(widget).remove::<SliderDrag>();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "removing the drag returns to normal",
        );
    }

    #[test]
    fn slider_state_removal_edges_return_to_prior_and_normal() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, |element| {
                element
                    .hovered(Appearance::new().background(STATE_HOVER_FILL))
                    .pressed(Appearance::new().background(STATE_PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget).and_then(|value| value.fill_color),
            Some(STATE_HOVER_FILL),
        );
        app.world_mut().entity_mut(widget).insert(SliderDrag);
        app.update();
        assert_eq!(
            root_override(&app, widget).and_then(|value| value.fill_color),
            Some(STATE_PRESS_FILL),
        );

        // Removing the drag returns to the prior hovered fill.
        app.world_mut().entity_mut(widget).remove::<SliderDrag>();
        app.update();
        assert_eq!(
            root_override(&app, widget).and_then(|value| value.fill_color),
            Some(STATE_HOVER_FILL),
            "removing the drag returns to the prior hover state",
        );

        // Removing the hover aggregate returns to normal.
        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "removing the hover aggregate returns to normal",
        );
    }

    #[test]
    fn slider_first_override_insertion_reaches_dispatch_through_the_fence() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            headless_slider_tree_with("level", plain_slider(0.5), |element| {
                element.hovered(Appearance::new().background(STATE_HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        // A headless slider writes no thumb translation, so the hover override is
        // genuinely the first insertion onto a widget without the component.
        assert!(app.world().get::<WidgetVisualOverrides>(widget).is_none());
        let root_index = slots_of(&app, widget)
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("root element index");
        assert!(indexed_root(&app, panel, root_index).is_none());

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let expected = VisualSlotOverride {
            fill_color: Some(STATE_HOVER_FILL),
            ..VisualSlotOverride::default()
        };
        assert_eq!(root_override(&app, widget), Some(expected.clone()));
        assert_eq!(
            indexed_root(&app, panel, root_index),
            Some(expected),
            "the first override insertion reaches dispatch through the presentation fence",
        );
    }

    #[test]
    fn slider_repeated_identical_value_and_state_leave_the_override_untouched() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            thumb_slider_tree_with("level", plain_slider(0.5), 8.0, |element| {
                element.hovered(Appearance::new().background(STATE_HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let tick = app
            .world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed());
        assert!(tick.is_some());

        // Re-touching `SliderState` and re-inserting the same hovered state both
        // re-arm the walk, but the requested overrides are unchanged, so the
        // component change tick and its retained records stay put.
        set_slider_value(&mut app, widget, 0.5);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<WidgetVisualOverrides>()
                .map(|overrides| overrides.last_changed()),
            tick,
            "an unchanged value and state must not dirty the override component",
        );
    }

    #[test]
    fn set_tree_rejects_invalid_slider_thumbs_and_surfaces_preserving_the_live_tree() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, thumb_slider_tree("level", plain_slider(0.5), 8.0));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");

        let mut orphan = LayoutBuilder::new(100.0, 50.0);
        orphan.with(El::new().size(8.0, 8.0).id("stray").slider_thumb(), |_| {});
        assert!(matches!(
            app.world_mut().commands().set_tree(panel, orphan.build()),
            Err(PanelBuildError::SliderThumbOutsideSlider(id))
                if id == PanelElementId::named("stray")
        ));

        let mut duplicate = LayoutBuilder::new(100.0, 50.0);
        duplicate.with(
            El::overlay()
                .size(40.0, 16.0)
                .widget("level", plain_slider(0.5)),
            |builder| {
                builder.with(El::new().size(8.0, 8.0).id("a").slider_thumb(), |_| {});
                builder.with(El::new().size(8.0, 8.0).id("b").slider_thumb(), |_| {});
            },
        );
        assert!(matches!(
            app.world_mut().commands().set_tree(panel, duplicate.build()),
            Err(PanelBuildError::SliderHasMultipleThumbs(id))
                if id == PanelElementId::named("level")
        ));

        // Every rejected replacement left the current tree and widget live.
        app.update();
        assert_eq!(
            resolve_widget(&mut app, panel, "level"),
            widget,
            "rejected trees must leave the current widget live",
        );
        assert!(app.world().get::<SliderState>(widget).is_some());
    }

    #[test]
    fn zero_thumb_headless_slider_reifies_with_a_distinct_current_root_slot() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, headless_slider_tree("level", plain_slider(0.5)));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        assert!(slots.element_index(VisualSlotId::SLIDER_ROOT).is_some());
        assert!(slots.content_box(VisualSlotId::SLIDER_ROOT).is_some());
        assert!(slots.border_box(VisualSlotId::SLIDER_ROOT).is_some());
        assert!(slots.element_index(VisualSlotId::SLIDER_THUMB).is_none());
        assert!(slots.border_box(VisualSlotId::SLIDER_THUMB).is_none());
        assert_eq!(
            thumb_offset(&app, widget),
            None,
            "a headless slider writes no thumb translation",
        );
    }

    #[test]
    fn slider_root_and_thumb_slots_are_distinct_and_carry_current_bounds() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, thumb_slider_tree("level", plain_slider(0.5), 8.0));
        app.update();
        let widget = resolve_widget(&mut app, panel, "level");
        let slots = slots_of(&app, widget);
        let root_index = slots
            .element_index(VisualSlotId::SLIDER_ROOT)
            .expect("root slot");
        let thumb_index = slots
            .element_index(VisualSlotId::SLIDER_THUMB)
            .expect("thumb slot");
        assert_ne!(
            root_index, thumb_index,
            "the root and thumb slots resolve to distinct elements",
        );
        let root = slots
            .border_box(VisualSlotId::SLIDER_ROOT)
            .expect("root border box");
        let thumb = slots
            .border_box(VisualSlotId::SLIDER_THUMB)
            .expect("thumb border box");
        let content = slots
            .content_box(VisualSlotId::SLIDER_ROOT)
            .expect("root content box");
        assert!(thumb.width > 0.0 && thumb.height > 0.0);
        assert!(thumb.width < root.width && thumb.height < root.height);
        assert!(content.width < root.width && content.height < root.height);
    }
}
