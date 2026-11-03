use bevy::prelude::Color;
use smallvec::SmallVec;

use super::VisualSlotId;
use super::VisualSlotOverride;
use super::WidgetVisualOverrides;
use super::WidgetVisualSlots;
use crate::DiegeticPanel;
use crate::cascade::Cascade;
use crate::cascade::Resolved;
use crate::layout::Dimension;
use crate::widgets::Appearance;
use crate::widgets::ResolvedWidgetStateAppearances;
use crate::widgets::StateAppearance;
use crate::widgets::VisualElementCapabilities;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetPressedAppearance;
use crate::widgets::WidgetState;
use crate::widgets::presentation::appearance::VisualChange;
use crate::widgets::presentation::appearance::WidgetStateCascades;

const DIMMED_TEXT: Color = Color::srgb(0.53, 0.53, 0.55);

/// Resolves sparse part appearances onto their retained-record recipients.
///
/// `WidgetVisualSlots::elements` and `WidgetVisualSlots::part_appearances`
/// are both ordered by element index. Advancing the part cursor only as each
/// recipient passes it visits the two lists in linear time and avoids a
/// per-recipient lookup.
pub(in crate::widgets) fn resolve_part_overrides(
    desired: &mut WidgetVisualOverrides,
    slots: &WidgetVisualSlots,
    hovered: &Resolved<WidgetHoveredAppearance>,
    pressed: &Resolved<WidgetPressedAppearance>,
    focused: &Resolved<WidgetFocusedAppearance>,
    disabled: &Resolved<WidgetDisabledAppearance>,
    authored_hovered: &Cascade<WidgetHoveredAppearance>,
    authored_disabled: &Cascade<WidgetDisabledAppearance>,
    active: &[Option<WidgetState>],
    dimmed: bool,
    panel: Option<&DiegeticPanel>,
) {
    let widget_appearances = ResolvedWidgetStateAppearances::new(
        hovered.0.appearance(),
        pressed.0.appearance(),
        focused.0.appearance(),
        disabled.0.appearance(),
    );
    let part_appearances = slots.part_appearances();
    let switch_knob_index = slots.element_index(VisualSlotId::SWITCH_KNOB);
    let switch_root_index = slots.element_index(VisualSlotId::SWITCH_ROOT);
    let mut part_cursor = 0;
    for &(element_index, capabilities) in slots.elements() {
        while part_appearances
            .get(part_cursor)
            .is_some_and(|(part_index, _)| *part_index < element_index)
        {
            part_cursor += 1;
        }
        let part_appearance =
            part_appearances
                .get(part_cursor)
                .and_then(|(part_index, appearance)| {
                    (*part_index == element_index).then_some(appearance)
                });
        if part_appearance.is_some() {
            part_cursor += 1;
        }
        let part_cascades = part_appearance.map(StateAppearance::cascades);
        let widget_level_applies = !slots.is_generated_editor_element(element_index);
        let linked_caption = slots.is_dimmed_caption(element_index);
        let keeps_authored_fill = keeps_authored_control_fill(slots, element_index);
        let merge_state = |state| {
            let mut levels = SmallVec::<[&Appearance; 2]>::new();
            if widget_level_applies && (!linked_caption || state == WidgetState::Disabled) {
                levels.push(widget_appearances.layer(state));
            }
            if let Some(part_layer) = part_cascades
                .as_ref()
                .and_then(|appearance| appearance.layer(state))
            {
                levels.push(part_layer);
            }
            let mut appearance = merge_levels(&levels);
            if state == WidgetState::Hovered {
                clear_inherited_control_hover_fill(
                    &mut appearance,
                    slots,
                    authored_hovered,
                    part_cascades.as_ref(),
                );
            }
            if switch_root_index == Some(element_index)
                && matches!(
                    state,
                    WidgetState::Hovered | WidgetState::Pressed | WidgetState::Focused
                )
            {
                appearance.background = VisualChange::Unchanged;
            }
            if keeps_authored_fill
                && (matches!(
                    state,
                    WidgetState::Hovered | WidgetState::Pressed | WidgetState::Focused
                ) || (state == WidgetState::Disabled
                    && switch_knob_index == Some(element_index)))
                && !part_cascades
                    .as_ref()
                    .and_then(|appearance| appearance.layer(state))
                    .is_some_and(|part| part.background.is_authored())
            {
                appearance.background = VisualChange::Unchanged;
            }
            appearance
        };
        let hovered = merge_state(WidgetState::Hovered);
        let pressed = merge_state(WidgetState::Pressed);
        let focused = merge_state(WidgetState::Focused);
        let disabled = merge_state(WidgetState::Disabled);
        let mut resolved =
            ResolvedWidgetStateAppearances::new(&hovered, &pressed, &focused, &disabled)
                .resolve(active, panel)
                .for_capabilities(capabilities);
        apply_disabled_part_appearance(
            &mut resolved,
            &disabled,
            slots,
            element_index,
            capabilities,
            active,
            authored_disabled,
            dimmed,
        );
        if resolved != VisualSlotOverride::default() {
            desired.set_element(element_index, resolved);
        }
    }
}

fn keeps_authored_control_fill(slots: &WidgetVisualSlots, element_index: usize) -> bool {
    [
        VisualSlotId::SWITCH_KNOB,
        VisualSlotId::SLIDER_THUMB,
        VisualSlotId::SLIDER_FILL,
        VisualSlotId::SLIDER_BAND,
    ]
    .into_iter()
    .any(|slot| slots.element_index(slot) == Some(element_index))
}

fn clear_inherited_control_hover_fill(
    appearance: &mut Appearance,
    slots: &WidgetVisualSlots,
    authored_hovered: &Cascade<WidgetHoveredAppearance>,
    part_cascades: Option<&WidgetStateCascades<'_>>,
) {
    let non_button_control = slots.element_index(VisualSlotId::SLIDER_ROOT).is_some()
        || slots.element_index(VisualSlotId::SWITCH_ROOT).is_some()
        || slots.element_index(VisualSlotId::EDITABLE_ROOT).is_some();
    let root_authored = authored_hovered
        .as_override()
        .is_some_and(|hovered| hovered.appearance().background.is_authored());
    let part_authored = part_cascades
        .and_then(|cascades| cascades.layer(WidgetState::Hovered))
        .is_some_and(|hovered| hovered.background.is_authored());
    if non_button_control && !root_authored && !part_authored {
        appearance.background = VisualChange::Unchanged;
    }
}

const fn dim_linked_caption(resolved: &mut VisualSlotOverride, disabled: &Appearance) {
    resolved.text_color = Some(match &disabled.text_color {
        VisualChange::To(color) => *color,
        VisualChange::Unchanged => DIMMED_TEXT,
    });
}

fn apply_disabled_part_appearance(
    resolved: &mut VisualSlotOverride,
    disabled: &Appearance,
    slots: &WidgetVisualSlots,
    element_index: usize,
    capabilities: VisualElementCapabilities,
    active: &[Option<WidgetState>],
    authored_disabled: &Cascade<WidgetDisabledAppearance>,
    dimmed: bool,
) {
    if dimmed && slots.is_dimmed_caption(element_index) {
        dim_linked_caption(resolved, disabled);
    }
    if !active.contains(&Some(WidgetState::Disabled)) {
        return;
    }
    let widget_level_applies = !slots.is_generated_editor_element(element_index);
    if widget_level_applies || capabilities.contains(VisualElementCapabilities::TEXT) {
        if capabilities.contains(VisualElementCapabilities::TEXT) {
            resolved.text_color = Some(match &disabled.text_color {
                VisualChange::To(color) => *color,
                VisualChange::Unchanged => DIMMED_TEXT,
            });
        }
        if widget_level_applies
            && slots.element_index(VisualSlotId::SWITCH_ROOT) != Some(element_index)
            && slots.element_index(VisualSlotId::SWITCH_KNOB) != Some(element_index)
        {
            resolved.dim_fill = !disabled.background.is_authored()
                && capabilities.contains(VisualElementCapabilities::SDF_FILL);
            resolved.dim_border = !disabled.border_color.is_authored()
                && capabilities.contains(VisualElementCapabilities::SDF_BORDER);
        }
    }
    if slots.element_index(VisualSlotId::SWITCH_ROOT) == Some(element_index)
        && !authored_disabled
            .as_override()
            .is_some_and(|appearance| appearance.appearance().background.is_authored())
    {
        resolved.fill_color = None;
    }
}

/// Reduces one state channel's authored levels into a single bundle.
///
/// `levels` is ordered lowest precedence first, so the last entry wins per
/// property. A recipient currently supplies its widget result and, when it
/// has one, its own part bundle.
fn merge_levels(levels: &[&Appearance]) -> Appearance {
    levels
        .iter()
        .rev()
        .copied()
        .fold(Appearance::new(), |lower, higher| lower.merge_over(higher))
}

impl WidgetStateCascades<'_> {
    /// Composes authored active-state layers for focused unit tests.
    #[cfg(test)]
    pub(crate) fn resolve(
        &self,
        active: &[Option<WidgetState>],
        panel: Option<&DiegeticPanel>,
    ) -> VisualSlotOverride {
        resolve_active_layers(active, panel, |state| self.layer(state))
    }
}

impl ResolvedWidgetStateAppearances<'_> {
    /// Composes the resolved active-state layers into one record override.
    pub(crate) fn resolve(
        &self,
        active: &[Option<WidgetState>],
        panel: Option<&DiegeticPanel>,
    ) -> VisualSlotOverride {
        resolve_active_layers(active, panel, |state| Some(self.layer(state)))
    }
}

fn resolve_active_layers<'a>(
    active: &[Option<WidgetState>],
    panel: Option<&DiegeticPanel>,
    layer: impl Fn(WidgetState) -> Option<&'a Appearance>,
) -> VisualSlotOverride {
    let mut background = None;
    let mut border_color = None;
    let mut border_width = None;
    let mut text_color = None;
    let mut path_color = None;
    let mut tint = None;
    let mut material = None;
    for state in WidgetState::LAYER_ORDER {
        if active.contains(&Some(state))
            && let Some(layer) = layer(state)
        {
            if let VisualChange::To(value) = &layer.background {
                background = Some(value);
            }
            if let VisualChange::To(value) = &layer.border_color {
                border_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.border_width {
                border_width = Some(value);
            }
            if let VisualChange::To(value) = &layer.text_color {
                text_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.path_color {
                path_color = Some(value);
            }
            if let VisualChange::To(value) = &layer.tint {
                tint = Some(value);
            }
            if let VisualChange::To(value) = &layer.material {
                material = Some(value);
            }
        }
    }
    let border_widths = border_width
        .zip(panel)
        .and_then(|(width, panel)| render_border_widths(*width, panel));
    VisualSlotOverride {
        fill_color: background.copied(),
        border_color: border_color.copied(),
        border_widths,
        text_color: text_color.copied(),
        path_color: path_color.copied(),
        tint: tint.copied(),
        material: material.cloned(),
        ..VisualSlotOverride::default()
    }
}

/// Converts an authored state border width into the panel-local world widths
/// retained SDF records carry.
///
/// Mirrors the authored border path: the dimension resolves to layout points
/// against the panel's layout unit, then scales by the panel's points-to-world
/// factor. Returns `None` for a negative or non-finite width, or a non-finite
/// or non-positive panel scale, so an unresolvable width leaves the authored
/// widths in place instead of writing a manufactured one.
fn render_border_widths(width: Dimension, panel: &DiegeticPanel) -> Option<[f32; 4]> {
    let points = width.to_points(panel.layout_unit().to_points());
    let points_to_world = panel.points_to_world();
    if !points.is_finite() || points < 0.0 || !points_to_world.is_finite() || points_to_world <= 0.0
    {
        return None;
    }
    Some([points * points_to_world; 4])
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::App;
    use bevy::prelude::Assets;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::Handle;
    use bevy::prelude::Image;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Window;
    use bevy::window::WindowRef;

    use super::merge_levels;
    use crate::Appearance;
    use crate::Border;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::EditorStateColors;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::WidgetFocusedAppearance;
    use crate::WidgetHoveredAppearance;
    use crate::WidgetPressedAppearance;
    use crate::cascade::Cascade;
    use crate::cascade::Resolved;
    use crate::ime::ImePlugin;
    use crate::layout::PanelCircle;
    use crate::layout::PanelDraw;
    use crate::layout::Text;
    use crate::layout::TextStyle;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ButtonPress;
    use crate::widgets::SemanticWidgetIntent;
    use crate::widgets::Slider;
    use crate::widgets::StateAppearance;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::WidgetDimmed;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetState;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::presentation::visual::VisualOverrideIndex;
    use crate::widgets::presentation::visual::VisualSlotId;
    use crate::widgets::presentation::visual::VisualSlotOverride;
    use crate::widgets::presentation::visual::WidgetVisualOverrides;
    use crate::widgets::presentation::visual::WidgetVisualSlots;

    const PART_HOVER_FILL: Color = Color::srgb(0.3, 0.7, 0.2);
    const TEXT_OVERRIDE_COLOR: Color = Color::srgb(0.3, 0.2, 0.7);
    const PATH_OVERRIDE_COLOR: Color = Color::srgb(0.7, 0.2, 0.3);
    const TINT_OVERRIDE_COLOR: Color = Color::srgb(0.5, 0.7, 0.1);
    const PART_NORMAL_FILL: Color = Color::srgb(0.08, 0.12, 0.16);
    const PART_NORMAL_BORDER: Color = Color::srgb(0.18, 0.24, 0.30);
    const WIDGET_PROPERTY_COLOR: Color = Color::srgb(0.82, 0.16, 0.28);
    const PART_PROPERTY_COLOR: Color = Color::srgb(0.12, 0.68, 0.42);
    const GLOBAL_PROPERTY_COLOR: Color = Color::srgb(0.14, 0.30, 0.74);
    const PANEL_PROPERTY_COLOR: Color = Color::srgb(0.72, 0.38, 0.12);

    #[derive(Clone, Copy)]
    enum MergeProperty {
        Background,
        BorderColor,
        BorderWidth,
        TextColor,
        PathColor,
        Tint,
        Material,
    }

    impl MergeProperty {
        const ALL: [Self; 7] = [
            Self::Background,
            Self::BorderColor,
            Self::BorderWidth,
            Self::TextColor,
            Self::PathColor,
            Self::Tint,
            Self::Material,
        ];

        fn widget_appearance(self, material: Handle<StandardMaterial>) -> Appearance {
            match self {
                Self::Background => Appearance::new().background(WIDGET_PROPERTY_COLOR),
                Self::BorderColor => Appearance::new().border_color(WIDGET_PROPERTY_COLOR),
                Self::BorderWidth => Appearance::new().border_width(Px(2.0)),
                Self::TextColor => Appearance::new().text_color(WIDGET_PROPERTY_COLOR),
                Self::PathColor => Appearance::new().path_color(WIDGET_PROPERTY_COLOR),
                Self::Tint => Appearance::new().tint(WIDGET_PROPERTY_COLOR),
                Self::Material => Appearance::new().material(material),
            }
        }

        fn part_appearance(self, material: Handle<StandardMaterial>) -> Appearance {
            match self {
                Self::Background => Appearance::new().background(PART_PROPERTY_COLOR),
                Self::BorderColor => Appearance::new().border_color(PART_PROPERTY_COLOR),
                Self::BorderWidth => Appearance::new().border_width(Px(4.0)),
                Self::TextColor => Appearance::new().text_color(PART_PROPERTY_COLOR),
                Self::PathColor => Appearance::new().path_color(PART_PROPERTY_COLOR),
                Self::Tint => Appearance::new().tint(PART_PROPERTY_COLOR),
                Self::Material => Appearance::new().material(material),
            }
        }

        fn expected_override(
            self,
            widget_names_property: bool,
            part_names_property: bool,
            points_to_world: f32,
            widget_material: Handle<StandardMaterial>,
            part_material: Handle<StandardMaterial>,
        ) -> Option<VisualSlotOverride> {
            let part_wins = part_names_property;
            let widget_wins = widget_names_property && !part_names_property;
            if !part_wins && !widget_wins {
                return None;
            }
            Some(match self {
                Self::Background => VisualSlotOverride {
                    fill_color: Some(if part_wins {
                        PART_PROPERTY_COLOR
                    } else {
                        WIDGET_PROPERTY_COLOR
                    }),
                    ..VisualSlotOverride::default()
                },
                Self::BorderColor => VisualSlotOverride {
                    border_color: Some(if part_wins {
                        PART_PROPERTY_COLOR
                    } else {
                        WIDGET_PROPERTY_COLOR
                    }),
                    ..VisualSlotOverride::default()
                },
                Self::BorderWidth => {
                    let points = if part_wins { 4.0 } else { 2.0 };
                    VisualSlotOverride {
                        border_widths: Some([points * 0.75 * points_to_world; 4]),
                        ..VisualSlotOverride::default()
                    }
                },
                Self::TextColor => VisualSlotOverride {
                    text_color: Some(if part_wins {
                        PART_PROPERTY_COLOR
                    } else {
                        WIDGET_PROPERTY_COLOR
                    }),
                    ..VisualSlotOverride::default()
                },
                Self::PathColor => VisualSlotOverride {
                    path_color: Some(if part_wins {
                        PART_PROPERTY_COLOR
                    } else {
                        WIDGET_PROPERTY_COLOR
                    }),
                    ..VisualSlotOverride::default()
                },
                Self::Tint => VisualSlotOverride {
                    tint: Some(if part_wins {
                        PART_PROPERTY_COLOR
                    } else {
                        WIDGET_PROPERTY_COLOR
                    }),
                    ..VisualSlotOverride::default()
                },
                Self::Material => VisualSlotOverride {
                    material: Some(if part_wins {
                        part_material
                    } else {
                        widget_material
                    }),
                    ..VisualSlotOverride::default()
                },
            })
        }
    }

    const HOVER_FILL: Color = Color::srgb(0.1, 0.2, 0.3);
    const FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const PRESS_FILL: Color = Color::srgb(0.4, 0.5, 0.6);
    const HOVER_TEXT: Color = Color::srgb(0.7, 0.5, 0.3);
    const FOCUS_PATH: Color = Color::srgb(0.3, 0.5, 0.7);
    fn panel() -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .build()
            .expect("a sized world panel builds")
    }

    fn state_appearance() -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().background(HOVER_FILL),
            )),
            pressed: Cascade::Override(WidgetPressedAppearance::new(
                Appearance::new().background(PRESS_FILL),
            )),
            focused: Cascade::Override(WidgetFocusedAppearance::new(
                Appearance::new()
                    .border_color(FOCUS_BORDER)
                    .border_width(Px(2.0)),
            )),
            ..StateAppearance::default()
        }
    }

    #[test]
    fn merge_levels_accepts_zero_through_three_authored_levels() {
        let mut materials = Assets::<StandardMaterial>::default();
        let lowest_material = materials.add(StandardMaterial::default());
        let middle_material = materials.add(StandardMaterial::default());
        let highest_material = materials.add(StandardMaterial::default());
        let lowest = Appearance::new()
            .background(Color::BLACK)
            .border_color(Color::WHITE)
            .border_width(Px(1.0))
            .text_color(Color::BLACK)
            .path_color(Color::WHITE)
            .tint(Color::srgb(0.2, 0.3, 0.4))
            .material(lowest_material.clone());
        let middle = Appearance::new()
            .background(Color::WHITE)
            .border_color(Color::BLACK)
            .border_width(Px(2.0))
            .text_color(Color::WHITE)
            .path_color(Color::BLACK)
            .tint(Color::srgb(0.4, 0.3, 0.2))
            .material(middle_material.clone());
        let highest = Appearance::new()
            .background(Color::srgb(0.2, 0.4, 0.6))
            .border_color(Color::srgb(0.6, 0.4, 0.2))
            .border_width(Px(3.0))
            .text_color(Color::srgb(0.3, 0.5, 0.7))
            .path_color(Color::srgb(0.7, 0.5, 0.3))
            .tint(Color::srgb(0.5, 0.7, 0.3))
            .material(highest_material.clone());

        assert_eq!(merge_levels(&[]), Appearance::new());
        assert_eq!(
            merge_levels(&[&lowest]),
            Appearance::new()
                .background(Color::BLACK)
                .border_color(Color::WHITE)
                .border_width(Px(1.0))
                .text_color(Color::BLACK)
                .path_color(Color::WHITE)
                .tint(Color::srgb(0.2, 0.3, 0.4))
                .material(lowest_material),
        );
        assert_eq!(
            merge_levels(&[&lowest, &middle]),
            Appearance::new()
                .background(Color::WHITE)
                .border_color(Color::BLACK)
                .border_width(Px(2.0))
                .text_color(Color::WHITE)
                .path_color(Color::BLACK)
                .tint(Color::srgb(0.4, 0.3, 0.2))
                .material(middle_material),
        );
        assert_eq!(
            merge_levels(&[&lowest, &middle, &highest]),
            Appearance::new()
                .background(Color::srgb(0.2, 0.4, 0.6))
                .border_color(Color::srgb(0.6, 0.4, 0.2))
                .border_width(Px(3.0))
                .text_color(Color::srgb(0.3, 0.5, 0.7))
                .path_color(Color::srgb(0.7, 0.5, 0.3))
                .tint(Color::srgb(0.5, 0.7, 0.3))
                .material(highest_material),
        );
    }

    fn widgets_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn spawn_widget_panel(app: &mut App, tree: crate::LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("widget panel should build");
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named(id))
            })
            .ok()
            .flatten()
            .expect("widget should reify")
    }

    fn reauthor_tree(app: &mut App, panel: Entity, tree: crate::LayoutTree) {
        app.world_mut()
            .commands()
            .set_tree(panel, tree)
            .expect("replacement tree should be accepted");
        app.update();
        app.update();
    }

    fn indexed_fill(app: &App, panel: Entity, element_index: usize) -> Option<Color> {
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .and_then(|override_value| override_value.fill_color)
    }

    fn assert_no_unowned_slider_part_overrides(
        app: &App,
        panel: Entity,
        widget: Entity,
        styled_unauthored_index: usize,
        structural_container_index: usize,
    ) {
        let visual_override_index = app.world().resource::<VisualOverrideIndex>();
        assert!(
            visual_override_index
                .get(panel, styled_unauthored_index)
                .is_none(),
            "a visual recipient without an authored state appearance receives no element override",
        );
        assert!(
            visual_override_index
                .get(panel, structural_container_index)
                .is_none(),
            "a pure slider layout container creates no visual override index entry",
        );
        assert!(
            !app.world()
                .get::<WidgetVisualSlots>(widget)
                .expect("slider widget visual slots")
                .elements()
                .iter()
                .any(|(element_index, _)| *element_index == structural_container_index),
            "a pure slider layout container is not a retained-record recipient",
        );
    }

    fn hovered_part_appearance() -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().background(PART_HOVER_FILL),
            )),
            ..StateAppearance::default()
        }
    }

    fn text_part_tree(appearance: Appearance) -> (crate::LayoutTree, usize) {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.text(
                Text::new("label", TextStyle::new(10.0))
                    .layout(children.child(El::new()).hovered(appearance)),
            );
        });
        let tree = builder.build();
        (tree.clone(), tree.len() - 1)
    }

    fn text_and_draw_part_tree(appearance: Appearance) -> (crate::LayoutTree, usize) {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.text(
                Text::new("label", TextStyle::new(10.0)).layout(
                    children
                        .child(
                            El::new()
                                .draw(PanelDraw::shapes([PanelCircle::new((10.0, 10.0), 5.0)])),
                        )
                        .hovered(appearance),
                ),
            );
        });
        let tree = builder.build();
        (tree.clone(), tree.len() - 1)
    }

    fn button_part_tree(stateful_part: bool, prepended_part: bool) -> (crate::LayoutTree, usize) {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().background(Color::WHITE).button("button"),
            |children| {
                if prepended_part {
                    children.with(El::new().background(Color::WHITE), |_| {});
                }
                children.with(El::new().background(Color::WHITE), |_| {});
            },
        );
        let mut tree = builder.build();
        let part_index = tree.len() - 1;
        if stateful_part {
            assert!(tree.set_element_state_appearance(part_index, hovered_part_appearance()));
        }
        (tree, part_index)
    }

    fn editable_part_tree(stateful_part: bool, prepended_part: bool) -> (crate::LayoutTree, usize) {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        builder.with(
            El::new().background(Color::WHITE).editable_field(
                "editable",
                field,
                crate::TextStyle::new(10.0),
            ),
            |children| {
                if prepended_part {
                    children.with(El::new().background(Color::WHITE), |_| {});
                }
                children.with(El::new().background(Color::WHITE), |_| {});
            },
        );
        let mut tree = builder.build();
        let part_index = tree.len() - 1;
        if stateful_part {
            assert!(tree.set_element_state_appearance(part_index, hovered_part_appearance()));
        }
        (tree, part_index)
    }

    fn slider_part_tree(
        stateful_part: bool,
        prepended_part: bool,
    ) -> (crate::LayoutTree, usize, usize, usize) {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::overlay()
                .size(40.0, 16.0)
                .background(Color::WHITE)
                .widget("slider", Slider::new(0.0..=1.0)),
            |children| {
                if prepended_part {
                    children.with(El::new().background(Color::WHITE), |_| {});
                }
                children.with(El::new().background(Color::WHITE), |_| {});
                children.with(El::new().background(Color::WHITE), |_| {});
                children.with(El::new(), |_| {});
            },
        );
        let mut tree = builder.build();
        let structural_container_index = tree.len() - 1;
        let styled_unauthored_index = structural_container_index - 1;
        let part_index = styled_unauthored_index - 1;
        if stateful_part {
            assert!(tree.set_element_state_appearance(part_index, hovered_part_appearance()));
        }
        (
            tree,
            part_index,
            styled_unauthored_index,
            structural_container_index,
        )
    }

    fn state_channel_appearance(
        active_state: WidgetState,
        channel: WidgetState,
        appearance: &Appearance,
    ) -> Appearance {
        if active_state == channel {
            appearance.clone()
        } else {
            Appearance::new()
        }
    }

    fn rich_part<W: crate::WidgetOwner>(
        builder: &crate::WidgetBuilder<'_, W>,
        active_state: WidgetState,
        appearance: Appearance,
    ) -> crate::El<crate::Row, crate::PressedPart> {
        builder
            .child(
                El::new()
                    .background(PART_NORMAL_FILL)
                    .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                    .material(Handle::<StandardMaterial>::default())
                    .draw(PanelDraw::shapes([PanelCircle::new((10.0, 10.0), 5.0)])),
            )
            .focused(state_channel_appearance(
                active_state,
                WidgetState::Focused,
                &appearance,
            ))
            .hovered(state_channel_appearance(
                active_state,
                WidgetState::Hovered,
                &appearance,
            ))
            .pressed(state_channel_appearance(
                active_state,
                WidgetState::Pressed,
                &appearance,
            ))
            .disabled(state_channel_appearance(
                active_state,
                WidgetState::Disabled,
                &appearance,
            ))
    }

    fn image_part<W: crate::WidgetOwner>(
        builder: &crate::WidgetBuilder<'_, W>,
        active_state: WidgetState,
        appearance: Appearance,
    ) -> crate::El<crate::Row, crate::PressedPart> {
        builder
            .child(El::new())
            .focused(state_channel_appearance(
                active_state,
                WidgetState::Focused,
                &appearance,
            ))
            .hovered(state_channel_appearance(
                active_state,
                WidgetState::Hovered,
                &appearance,
            ))
            .pressed(state_channel_appearance(
                active_state,
                WidgetState::Pressed,
                &appearance,
            ))
            .disabled(state_channel_appearance(
                active_state,
                WidgetState::Disabled,
                &appearance,
            ))
    }

    fn activate_widget_state(app: &mut App, widget: Entity, state: WidgetState) {
        match state {
            WidgetState::Hovered => {
                app.world_mut()
                    .entity_mut(widget)
                    .insert(PickingInteraction::Hovered);
            },
            WidgetState::Pressed => {
                app.world_mut().entity_mut(widget).insert(ButtonPress);
            },
            WidgetState::Focused => {
                let window = app.world_mut().spawn(Window::default()).id();
                app.world_mut()
                    .trigger(crate::RequestWidgetFocus { window, widget });
                app.world_mut().flush();
            },
            WidgetState::Disabled => {
                app.world_mut()
                    .entity_mut(widget)
                    .insert(WidgetDisabled::test_marker());
            },
        }
    }

    fn part_override(app: &App, panel: Entity, element_index: usize) -> Option<VisualSlotOverride> {
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .cloned()
    }

    #[test]
    fn panel_hover_fill_only_reaches_buttons_without_control_authoring() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 80.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .background(PART_NORMAL_FILL)
                .button("button"),
            |_| {},
        );
        builder.with(
            El::new()
                .size(40.0, 12.0)
                .widget("slider", Slider::new(0.0..=1.0)),
            |children| {
                children.with(
                    El::new().size(35.0, 4.0).background(PART_NORMAL_FILL),
                    |_| {},
                );
            },
        );
        builder.with(El::new().size(20.0, 12.0).switch("switch"), |children| {
            children.with(El::new().size(9.0, 9.0).switch_knob(), |_| {});
        });
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        builder.with(
            El::new()
                .size(40.0, 12.0)
                .background(PART_NORMAL_FILL)
                .editable_field("field", field, TextStyle::new(10.0)),
            |_| {},
        );
        let tree = builder.build();
        let slider_track_index = tree.children_of(tree.children_of(0)[1])[0];
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(80.0))
            .widget_hovered_appearance(Appearance::new().background(PANEL_PROPERTY_COLOR))
            .with_tree(tree)
            .build()
            .expect("hover panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();

        for id in ["button", "slider", "switch", "field"] {
            let widget = resolve_widget(&mut app, panel, id);
            app.world_mut()
                .entity_mut(widget)
                .insert(PickingInteraction::Hovered);
        }
        app.update();
        for (id, slot) in [
            ("slider", VisualSlotId::SLIDER_ROOT),
            ("field", VisualSlotId::EDITABLE_ROOT),
        ] {
            let widget = resolve_widget(&mut app, panel, id);
            let index = app
                .world()
                .get::<WidgetVisualSlots>(widget)
                .and_then(|slots| slots.element_index(slot))
                .expect("control root slot");
            assert_eq!(
                part_override(&app, panel, index).and_then(|part| part.fill_color),
                None,
                "{id} must not inherit panel hover fill",
            );
        }
        assert_eq!(
            part_override(&app, panel, slider_track_index).and_then(|part| part.fill_color),
            None,
        );
        let switch = resolve_widget(&mut app, panel, "switch");
        let switch_index = app
            .world()
            .get::<WidgetVisualSlots>(switch)
            .and_then(|slots| slots.element_index(VisualSlotId::SWITCH_ROOT))
            .expect("switch root slot");
        assert_eq!(
            part_override(&app, panel, switch_index).and_then(|part| part.fill_color),
            Some(crate::ControlPalette::system_dark().accent),
        );
        let button = resolve_widget(&mut app, panel, "button");
        let button_index = app
            .world()
            .get::<WidgetVisualSlots>(button)
            .and_then(|slots| slots.element_index(VisualSlotId::BUTTON_ROOT))
            .expect("button root slot");
        assert_eq!(
            part_override(&app, panel, button_index).and_then(|part| part.fill_color),
            Some(PANEL_PROPERTY_COLOR),
        );
    }

    fn inline_editor_visual_app(tree: crate::LayoutTree) -> (App, Entity, Entity) {
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
            .expect("editor panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        (app, window, panel)
    }

    fn editable_tree_with_widget_focus_and_optional_caret(
        text: &str,
        caret_colors: Option<EditorStateColors>,
    ) -> crate::LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let field = El::new()
            .background(PART_NORMAL_FILL)
            .editable_field("field", field, crate::TextStyle::new(10.0))
            .focused(Appearance::new().background(WIDGET_PROPERTY_COLOR));
        let field = match caret_colors {
            Some(colors) => field.editor_caret(colors),
            None => field,
        };
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(field, |children| {
            children.text((text, TextStyle::new(10.0)));
        });
        builder.build()
    }

    fn editable_tree_with_text_colors(text: &str, colors: EditorStateColors) -> crate::LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let field = El::new()
            .background(PART_NORMAL_FILL)
            .editable_field("field", field, crate::TextStyle::new(10.0))
            .focused(Appearance::new().background(WIDGET_PROPERTY_COLOR))
            .editor_text(colors);
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(field, |children| {
            children.text((text, TextStyle::new(10.0)));
        });
        builder.build()
    }

    fn activate_inline_editor(app: &mut App, window: Entity, field: Entity) {
        app.world_mut().trigger(crate::RequestWidgetFocus {
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

    #[test]
    fn widget_to_part_merge_matrix_covers_each_property_and_state() {
        // The upper cascade hops are covered by
        // `global_state_appearance_defaults_reach_every_widget_without_state_authoring`,
        // `panel_state_appearances_merge_with_globals_in_the_reification_frame`, and
        // `panel_hovered_appearance_preserves_global_properties_through_the_cascade`.
        for property in MergeProperty::ALL {
            for state in WidgetState::LAYER_ORDER {
                for (widget_names_property, part_names_property) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    let mut app = widgets_test_app();
                    let mut materials = Assets::<StandardMaterial>::default();
                    let widget_material = materials.add(StandardMaterial::default());
                    let part_material = materials.add(StandardMaterial::default());
                    let widget_appearance = if widget_names_property {
                        property.widget_appearance(widget_material.clone())
                    } else {
                        Appearance::new()
                    };
                    let part_appearance = if part_names_property {
                        property.part_appearance(part_material.clone())
                    } else {
                        Appearance::new()
                    };
                    let mut builder = LayoutBuilder::new(100.0, 50.0);
                    builder.with(
                        El::new()
                            .background(PART_NORMAL_FILL)
                            .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                            .material(Handle::<StandardMaterial>::default())
                            .button("button")
                            .focused(state_channel_appearance(
                                state,
                                WidgetState::Focused,
                                &widget_appearance,
                            ))
                            .hovered(state_channel_appearance(
                                state,
                                WidgetState::Hovered,
                                &widget_appearance,
                            ))
                            .pressed(state_channel_appearance(
                                state,
                                WidgetState::Pressed,
                                &widget_appearance,
                            ))
                            .disabled(state_channel_appearance(
                                state,
                                WidgetState::Disabled,
                                &widget_appearance,
                            )),
                        |children| {
                            if matches!(property, MergeProperty::Tint) {
                                children.image(
                                    image_part(children, state, part_appearance),
                                    Handle::default(),
                                    Color::WHITE,
                                );
                            } else {
                                children.text(
                                    Text::new("part", TextStyle::new(10.0)).layout(rich_part(
                                        children,
                                        state,
                                        part_appearance,
                                    )),
                                );
                            }
                        },
                    );
                    let tree = builder.build();
                    let part_index = tree.len() - 1;
                    let panel = spawn_widget_panel(&mut app, tree);
                    app.update();
                    let widget = resolve_widget(&mut app, panel, "button");
                    activate_widget_state(&mut app, widget, state);
                    app.update();

                    let points_to_world = app
                        .world()
                        .get::<DiegeticPanel>(panel)
                        .map_or(0.0, DiegeticPanel::points_to_world);
                    let mut expected = property.expected_override(
                        widget_names_property,
                        part_names_property,
                        points_to_world,
                        widget_material,
                        part_material,
                    );
                    if state == WidgetState::Disabled && !matches!(property, MergeProperty::Tint) {
                        let visual = expected.get_or_insert_with(VisualSlotOverride::default);
                        visual.text_color.get_or_insert(super::DIMMED_TEXT);
                        let property_is_authored = widget_names_property || part_names_property;
                        visual.dim_fill =
                            !property_is_authored || !matches!(property, MergeProperty::Background);
                        visual.dim_border = !property_is_authored
                            || !matches!(property, MergeProperty::BorderColor);
                    }
                    assert_eq!(part_override(&app, panel, part_index), expected);
                }
            }
        }
    }

    #[test]
    fn widget_focus_appearance_excludes_generated_editor_selection() {
        let (mut app, window, panel) = inline_editor_visual_app(
            editable_tree_with_widget_focus_and_optional_caret("display", None),
        );
        let field = resolve_widget(&mut app, panel, "field");
        activate_inline_editor(&mut app, window, field);

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field visual slots");
        let root_index = slots
            .element_index(VisualSlotId::EDITABLE_ROOT)
            .expect("editable field root index");
        assert_eq!(
            part_override(&app, panel, root_index),
            Some(VisualSlotOverride {
                fill_color: Some(WIDGET_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
        let selection_index = slots
            .elements()
            .iter()
            .find_map(|(element_index, capabilities)| {
                (slots
                    .generated_editor_elements
                    .binary_search(element_index)
                    .is_ok()
                    && capabilities.contains(VisualElementCapabilities::SDF_FILL))
                .then_some(*element_index)
            })
            .expect("selected editor text should generate an SDF-fill selection recipient");
        assert_eq!(
            part_override(&app, panel, selection_index),
            None,
            "the generated selection has no widget-level VisualOverrideIndex entry",
        );
    }

    #[test]
    fn widget_focus_appearance_excludes_generated_editor_caret() {
        let (mut app, window, panel) =
            inline_editor_visual_app(editable_tree_with_widget_focus_and_optional_caret("", None));
        let field = resolve_widget(&mut app, panel, "field");
        activate_inline_editor(&mut app, window, field);

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field visual slots");
        let root_index = slots
            .element_index(VisualSlotId::EDITABLE_ROOT)
            .expect("editable field root index");
        assert_eq!(
            part_override(&app, panel, root_index),
            Some(VisualSlotOverride {
                fill_color: Some(WIDGET_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
        let caret_index = slots
            .elements()
            .iter()
            .find_map(|(element_index, capabilities)| {
                (slots
                    .generated_editor_elements
                    .binary_search(element_index)
                    .is_ok()
                    && capabilities.contains(VisualElementCapabilities::SDF_FILL))
                .then_some(*element_index)
            })
            .expect("empty editor text should generate an SDF-fill caret recipient");
        assert_eq!(
            part_override(&app, panel, caret_index),
            None,
            "the generated caret has no widget-level VisualOverrideIndex entry",
        );
    }

    #[test]
    fn declared_editor_caret_part_still_overrides_the_generated_caret() {
        let (mut app, window, panel) =
            inline_editor_visual_app(editable_tree_with_widget_focus_and_optional_caret(
                "",
                Some(EditorStateColors::new().focused(PART_PROPERTY_COLOR)),
            ));
        let field = resolve_widget(&mut app, panel, "field");
        activate_inline_editor(&mut app, window, field);

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field visual slots");
        let caret_index = slots
            .part_appearances()
            .iter()
            .find_map(|(element_index, _)| {
                slots
                    .generated_editor_elements
                    .binary_search(element_index)
                    .is_ok()
                    .then_some(*element_index)
            })
            .expect("the declared editor caret should generate a styled part");
        assert_eq!(
            part_override(&app, panel, caret_index),
            Some(VisualSlotOverride {
                fill_color: Some(PART_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn editor_state_colors_apply_to_text_glyphs_and_caret_fill() {
        let colors = EditorStateColors::new().focused(PART_PROPERTY_COLOR);
        let (mut text_app, text_window, text_panel) =
            inline_editor_visual_app(editable_tree_with_text_colors("display", colors.clone()));
        let text_field = resolve_widget(&mut text_app, text_panel, "field");
        activate_inline_editor(&mut text_app, text_window, text_field);
        let text_slots = text_app
            .world()
            .get::<WidgetVisualSlots>(text_field)
            .expect("editable field visual slots");
        let text_index = text_slots
            .part_appearances()
            .iter()
            .find_map(|(element_index, _)| {
                text_slots
                    .elements()
                    .iter()
                    .find(|(visual_index, capabilities)| {
                        *visual_index == *element_index
                            && capabilities.contains(VisualElementCapabilities::TEXT)
                    })
                    .map(|_| *element_index)
            })
            .expect("the declared editor text should generate a text recipient");
        assert_eq!(
            part_override(&text_app, text_panel, text_index),
            Some(VisualSlotOverride {
                text_color: Some(PART_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );

        let (mut caret_app, caret_window, caret_panel) = inline_editor_visual_app(
            editable_tree_with_widget_focus_and_optional_caret("", Some(colors)),
        );
        let caret_field = resolve_widget(&mut caret_app, caret_panel, "field");
        activate_inline_editor(&mut caret_app, caret_window, caret_field);
        let caret_slots = caret_app
            .world()
            .get::<WidgetVisualSlots>(caret_field)
            .expect("editable field visual slots");
        let caret_index = caret_slots
            .part_appearances()
            .iter()
            .find_map(|(element_index, _)| {
                caret_slots
                    .generated_editor_elements
                    .binary_search(element_index)
                    .is_ok()
                    .then_some(*element_index)
            })
            .expect("the declared editor caret should generate a styled part");
        assert_eq!(
            part_override(&caret_app, caret_panel, caret_index),
            Some(VisualSlotOverride {
                fill_color: Some(PART_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn dimmed_open_editor_keeps_enabled_text() {
        for colors in [
            EditorStateColors::new(),
            EditorStateColors::new().disabled(PART_PROPERTY_COLOR),
        ] {
            let (mut app, window, panel) =
                inline_editor_visual_app(editable_tree_with_text_colors("display", colors));
            let field = resolve_widget(&mut app, panel, "field");
            activate_inline_editor(&mut app, window, field);
            app.world_mut().entity_mut(field).insert(WidgetDimmed);
            app.update();

            let slots = app
                .world()
                .get::<WidgetVisualSlots>(field)
                .expect("editable field visual slots");
            let text_index = slots
                .elements()
                .iter()
                .find_map(|(element_index, capabilities)| {
                    (slots.is_generated_editor_element(*element_index)
                        && capabilities.contains(VisualElementCapabilities::TEXT))
                    .then_some(*element_index)
                })
                .expect("open editor text recipient");
            assert_eq!(
                part_override(&app, panel, text_index).and_then(|visual| visual.text_color),
                None,
            );
        }
    }

    #[test]
    fn later_editor_focused_color_replaces_the_earlier_color() {
        let colors = EditorStateColors::new()
            .focused(WIDGET_PROPERTY_COLOR)
            .focused(PART_PROPERTY_COLOR);
        let (mut app, window, panel) = inline_editor_visual_app(
            editable_tree_with_widget_focus_and_optional_caret("", Some(colors)),
        );
        let field = resolve_widget(&mut app, panel, "field");
        activate_inline_editor(&mut app, window, field);

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .expect("editable field visual slots");
        let caret_index = slots
            .part_appearances()
            .iter()
            .find_map(|(element_index, _)| {
                slots
                    .generated_editor_elements
                    .binary_search(element_index)
                    .is_ok()
                    .then_some(*element_index)
            })
            .expect("the declared editor caret should generate a styled part");
        assert_eq!(
            part_override(&app, panel, caret_index),
            Some(VisualSlotOverride {
                fill_color: Some(PART_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn part_hovered_border_preserves_the_widget_hovered_background() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                .button("button")
                .hovered(
                    Appearance::new()
                        .background(WIDGET_PROPERTY_COLOR)
                        .border_color(WIDGET_PROPERTY_COLOR),
                ),
            |children| {
                children.with(
                    children
                        .child(
                            El::new()
                                .background(PART_NORMAL_FILL)
                                .border(Border::all(Px(1.0), PART_NORMAL_BORDER)),
                        )
                        .hovered(Appearance::new().border_color(PART_PROPERTY_COLOR)),
                    |_| {},
                );
            },
        );
        let tree = builder.build();
        let part_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            part_override(&app, panel, part_index),
            Some(VisualSlotOverride {
                fill_color: Some(WIDGET_PROPERTY_COLOR),
                border_color: Some(PART_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn part_hovered_background_preserves_the_part_normal_fill_against_the_widget_bundle() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .button("button")
                .hovered(Appearance::new().background(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.with(
                    children
                        .child(El::new().background(PART_NORMAL_FILL))
                        .hovered(Appearance::new().background(PART_NORMAL_FILL)),
                    |_| {},
                );
            },
        );
        let tree = builder.build();
        let part_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            part_override(&app, panel, part_index),
            Some(VisualSlotOverride {
                fill_color: Some(PART_NORMAL_FILL),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn slider_fill_keeps_authored_color_across_hover_press_and_focus() {
        for part_authors_background in [false, true] {
            let mut app = widgets_test_app();
            let mut builder = LayoutBuilder::new(100.0, 50.0);
            builder.with(
                El::overlay()
                    .size(40.0, 16.0)
                    .background(PART_NORMAL_FILL)
                    .widget("slider", Slider::new(0.0..=1.0))
                    .hovered(Appearance::new().background(WIDGET_PROPERTY_COLOR))
                    .pressed(Appearance::new().background(WIDGET_PROPERTY_COLOR))
                    .focused(Appearance::new().background(WIDGET_PROPERTY_COLOR)),
                |children| {
                    let fill = children.child(
                        El::new()
                            .size(40.0, 4.0)
                            .background(PART_NORMAL_FILL)
                            .slider_fill(),
                    );
                    if part_authors_background {
                        children.with(
                            fill.hovered(Appearance::new().background(PART_PROPERTY_COLOR))
                                .pressed(Appearance::new().background(PART_PROPERTY_COLOR))
                                .focused(Appearance::new().background(PART_PROPERTY_COLOR)),
                            |_| {},
                        );
                    } else {
                        children.with(fill, |_| {});
                    }
                },
            );
            let panel = spawn_widget_panel(&mut app, builder.build());
            app.update();
            let widget = resolve_widget(&mut app, panel, "slider");
            let world = app.world();
            let slots = world
                .get::<WidgetVisualSlots>(widget)
                .expect("slider slots");
            let root_index = slots
                .element_index(VisualSlotId::SLIDER_ROOT)
                .expect("root slot");
            let fill_index = slots
                .element_index(VisualSlotId::SLIDER_FILL)
                .expect("fill slot");
            let hovered = world
                .get::<Resolved<WidgetHoveredAppearance>>(widget)
                .expect("hover appearance");
            let pressed = world
                .get::<Resolved<WidgetPressedAppearance>>(widget)
                .expect("press appearance");
            let focused = world
                .get::<Resolved<WidgetFocusedAppearance>>(widget)
                .expect("focus appearance");
            let disabled = world
                .get::<Resolved<crate::WidgetDisabledAppearance>>(widget)
                .expect("disabled appearance");
            for state in [
                WidgetState::Hovered,
                WidgetState::Pressed,
                WidgetState::Focused,
            ] {
                let mut desired = WidgetVisualOverrides::default();
                super::resolve_part_overrides(
                    &mut desired,
                    slots,
                    hovered,
                    pressed,
                    focused,
                    disabled,
                    world
                        .get::<Cascade<WidgetHoveredAppearance>>(widget)
                        .expect("authored hovered appearance"),
                    world
                        .get::<Cascade<crate::WidgetDisabledAppearance>>(widget)
                        .expect("authored disabled appearance"),
                    &[Some(state)],
                    false,
                    world.get::<DiegeticPanel>(panel),
                );
                let color_for = |element_index| {
                    desired
                        .element_overrides()
                        .iter()
                        .find(|(index, _)| *index == element_index)
                        .and_then(|(_, visual)| visual.fill_color)
                };
                assert_eq!(color_for(root_index), Some(WIDGET_PROPERTY_COLOR));
                assert_eq!(
                    color_for(fill_index),
                    part_authors_background.then_some(PART_PROPERTY_COLOR),
                    "{state:?} must preserve the fill's authored color"
                );
            }
        }
    }

    #[test]
    fn sibling_caption_inherits_only_its_widgets_dim_appearance() {
        let mut app = widgets_test_app();
        app.insert_resource(crate::WidgetHoveredAppearance::new(
            Appearance::new().text_color(WIDGET_PROPERTY_COLOR),
        ));
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.with(El::row().size(70.0, 16.0), |builder| {
            builder.text(
                Text::new("Power", TextStyle::new(4.0)).layout(El::new().dim_with_widget("power")),
            );
            builder.with(El::new().size(20.0, 12.0).switch("power"), |builder| {
                builder.with(El::new().size(9.0, 9.0).switch_knob(), |_| {});
            });
        });
        let panel = spawn_widget_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "power");
        let world = app.world();
        let slots = world
            .get::<WidgetVisualSlots>(widget)
            .expect("switch slots");
        let caption_index = slots
            .element_index(VisualSlotId::DIMMED_CAPTION)
            .expect("linked caption text");
        let hovered = world
            .get::<Resolved<WidgetHoveredAppearance>>(widget)
            .expect("hover appearance");
        let pressed = world
            .get::<Resolved<WidgetPressedAppearance>>(widget)
            .expect("press appearance");
        let focused = world
            .get::<Resolved<WidgetFocusedAppearance>>(widget)
            .expect("focus appearance");
        let disabled = world
            .get::<Resolved<crate::WidgetDisabledAppearance>>(widget)
            .expect("disabled appearance");
        let caption_text = |state| {
            let mut desired = WidgetVisualOverrides::default();
            super::resolve_part_overrides(
                &mut desired,
                slots,
                hovered,
                pressed,
                focused,
                disabled,
                world
                    .get::<Cascade<WidgetHoveredAppearance>>(widget)
                    .expect("authored hovered appearance"),
                world
                    .get::<Cascade<crate::WidgetDisabledAppearance>>(widget)
                    .expect("authored disabled appearance"),
                &[Some(state)],
                false,
                world.get::<DiegeticPanel>(panel),
            );
            desired
                .element_overrides()
                .iter()
                .find(|(index, _)| *index == caption_index)
                .and_then(|(_, visual)| visual.text_color)
        };
        assert_eq!(caption_text(WidgetState::Hovered), None);
        assert_eq!(
            caption_text(WidgetState::Disabled),
            Some(super::DIMMED_TEXT)
        );
        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert_eq!(
            part_override(&app, panel, caption_index).and_then(|visual| visual.text_color),
            Some(super::DIMMED_TEXT),
        );

        app.world_mut().entity_mut(widget).remove::<WidgetDimmed>();
        app.update();
        assert_eq!(part_override(&app, panel, caption_index), None);
    }

    #[test]
    fn linked_caption_uses_switch_authored_disabled_text_color() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.with(El::row().size(70.0, 16.0), |builder| {
            builder.text(
                Text::new("No device", TextStyle::new(4.0))
                    .layout(El::new().dim_with_widget("power")),
            );
            builder.with(
                El::new()
                    .size(20.0, 12.0)
                    .switch("power")
                    .disabled(Appearance::new().text_color(PART_PROPERTY_COLOR)),
                |builder| {
                    builder.with(El::new().size(9.0, 9.0).switch_knob(), |_| {});
                },
            );
        });
        let panel = spawn_widget_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "power");
        let caption_index = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(VisualSlotId::DIMMED_CAPTION))
            .expect("switch should own its linked caption");

        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert_eq!(
            part_override(&app, panel, caption_index).and_then(|visual| visual.text_color),
            Some(PART_PROPERTY_COLOR),
        );
    }

    #[test]
    fn switch_root_keeps_authored_disabled_fill() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(80.0, 30.0);
        builder.with(
            El::new()
                .size(20.0, 12.0)
                .background(PART_NORMAL_FILL)
                .switch("power")
                .disabled(Appearance::new().background(PART_PROPERTY_COLOR)),
            |builder| {
                builder.with(El::new().size(9.0, 9.0).switch_knob(), |_| {});
            },
        );
        let panel = spawn_widget_panel(&mut app, builder.build());
        app.update();
        let widget = resolve_widget(&mut app, panel, "power");
        let root_index = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(VisualSlotId::SWITCH_ROOT))
            .expect("switch root slot");
        app.world_mut()
            .entity_mut(widget)
            .insert(WidgetDisabled::test_marker());
        app.update();
        assert_eq!(
            part_override(&app, panel, root_index).and_then(|visual| visual.fill_color),
            Some(PART_PROPERTY_COLOR),
        );
    }

    #[test]
    fn explicit_empty_part_bundle_inherits_the_widget_bundle_like_no_part_entry() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .button("button")
                .hovered(Appearance::new().background(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.with(
                    children
                        .child(El::new().background(PART_NORMAL_FILL))
                        .hovered(Appearance::new()),
                    |_| {},
                );
                children.with(El::new().background(PART_NORMAL_FILL), |_| {});
            },
        );
        let tree = builder.build();
        let explicit_empty_index = tree.len() - 2;
        let undeclared_part_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("button visual slots");
        assert!(
            slots
                .part_appearances()
                .iter()
                .any(|(element_index, _)| *element_index == explicit_empty_index),
            "the explicit empty bundle must reach the post-inversion part channel",
        );
        assert!(
            !slots
                .part_appearances()
                .iter()
                .any(|(element_index, _)| *element_index == undeclared_part_index),
            "the comparison recipient must have no part entry",
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let expected = Some(VisualSlotOverride {
            fill_color: Some(WIDGET_PROPERTY_COLOR),
            ..VisualSlotOverride::default()
        });
        assert_eq!(part_override(&app, panel, explicit_empty_index), expected);
        assert_eq!(part_override(&app, panel, undeclared_part_index), expected);
    }

    #[test]
    fn global_panel_widget_and_part_hovered_appearances_compose_at_a_part() {
        let mut app = widgets_test_app();
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(GLOBAL_PROPERTY_COLOR));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                .button("button")
                .hovered(Appearance::new().text_color(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.text(
                    Text::new("part", TextStyle::new(10.0)).layout(
                        children
                            .child(
                                El::new()
                                    .background(PART_NORMAL_FILL)
                                    .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                                    .draw(PanelDraw::shapes([PanelCircle::new((10.0, 10.0), 5.0)])),
                            )
                            .hovered(Appearance::new().path_color(PART_PROPERTY_COLOR)),
                    ),
                );
            },
        );
        let tree = builder.build();
        let part_index = tree.len() - 1;
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(Appearance::new().border_color(PANEL_PROPERTY_COLOR))
            .with_tree(tree)
            .build()
            .expect("appearance composition panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            part_override(&app, panel, part_index),
            Some(VisualSlotOverride {
                fill_color: Some(GLOBAL_PROPERTY_COLOR),
                text_color: Some(WIDGET_PROPERTY_COLOR),
                path_color: Some(PART_PROPERTY_COLOR),
                border_color: Some(PANEL_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn widget_border_color_stays_dormant_on_a_text_only_label() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .button("button")
                .hovered(Appearance::new().border_color(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.text(Text::new("label", TextStyle::new(10.0)).layout(El::new()));
            },
        );
        let tree = builder.build();
        let label_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let label_capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| {
                slots
                    .elements()
                    .iter()
                    .find(|(element_index, _)| *element_index == label_index)
                    .map(|(_, capabilities)| *capabilities)
            })
            .expect("text label visual recipient");
        assert!(!label_capabilities.contains(VisualElementCapabilities::SDF_BORDER));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(part_override(&app, panel, label_index), None);
    }

    #[test]
    fn widget_material_stays_dormant_on_an_image_only_element() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("button")
                .hovered(Appearance::new().material(Handle::<StandardMaterial>::default())),
            |children| {
                children.image(El::new(), Handle::<Image>::default(), Color::WHITE);
            },
        );
        let tree = builder.build();
        let image_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let image_capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| {
                slots
                    .elements()
                    .iter()
                    .find(|(element_index, _)| *element_index == image_index)
                    .map(|(_, capabilities)| *capabilities)
            })
            .expect("image visual recipient");
        assert!(!image_capabilities.contains(VisualElementCapabilities::SDF_MATERIAL));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(part_override(&app, panel, image_index), None);
    }

    #[test]
    fn widget_tint_reaches_image_parts_and_skips_text_parts() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("button")
                .hovered(Appearance::new().tint(TINT_OVERRIDE_COLOR)),
            |children| {
                children.text(Text::new("label", TextStyle::new(10.0)).layout(El::new()));
                children.image(El::new(), Handle::default(), Color::WHITE);
            },
        );
        let tree = builder.build();
        let text_index = tree.len() - 2;
        let image_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");

        let slots = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("widget visual slots");
        let text_capabilities = slots
            .elements()
            .iter()
            .find_map(|(element_index, capabilities)| {
                (*element_index == text_index).then_some(*capabilities)
            })
            .expect("text visual recipient");
        let image_capabilities = slots
            .elements()
            .iter()
            .find_map(|(element_index, capabilities)| {
                (*element_index == image_index).then_some(*capabilities)
            })
            .expect("image visual recipient");
        assert!(!text_capabilities.contains(VisualElementCapabilities::IMAGE));
        assert!(image_capabilities.contains(VisualElementCapabilities::IMAGE));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(part_override(&app, panel, text_index), None);
        assert_eq!(
            part_override(&app, panel, image_index),
            Some(VisualSlotOverride {
                tint: Some(TINT_OVERRIDE_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn widget_background_stays_dormant_on_a_text_only_label_without_background() {
        let mut app = widgets_test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .button("button")
                .hovered(Appearance::new().background(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.text(Text::new("label", TextStyle::new(10.0)).layout(El::new()));
            },
        );
        let tree = builder.build();
        let label_index = tree.len() - 1;
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let label_capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| {
                slots
                    .elements()
                    .iter()
                    .find(|(element_index, _)| *element_index == label_index)
                    .map(|(_, capabilities)| *capabilities)
            })
            .expect("text label visual recipient");
        assert!(!label_capabilities.contains(VisualElementCapabilities::SDF_FILL));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(part_override(&app, panel, label_index), None);
    }

    #[test]
    fn state_layer_order_wins_across_global_panel_widget_and_part_levels() {
        let mut app = widgets_test_app();
        *app.world_mut()
            .resource_mut::<crate::WidgetDisabledAppearance>() =
            crate::WidgetDisabledAppearance::new(
                Appearance::new().background(GLOBAL_PROPERTY_COLOR),
            );
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .button("button")
                .hovered(Appearance::new().background(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.with(
                    children
                        .child(El::new().background(PART_NORMAL_FILL))
                        .focused(Appearance::new()),
                    |_| {},
                );
            },
        );
        let tree = builder.build();
        let part_index = tree.len() - 1;
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_pressed_appearance(Appearance::new().background(PANEL_PROPERTY_COLOR))
            .with_tree(tree)
            .build()
            .expect("layered state panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        activate_widget_state(&mut app, widget, WidgetState::Focused);
        app.world_mut().entity_mut(widget).insert((
            PickingInteraction::Hovered,
            ButtonPress,
            WidgetDisabled::test_marker(),
        ));
        app.update();

        assert_eq!(
            part_override(&app, panel, part_index),
            // Resolving levels first keeps the global disabled value; the interleaved
            // level-first state algorithm would instead produce `WIDGET_PROPERTY_COLOR`.
            Some(VisualSlotOverride {
                fill_color: Some(GLOBAL_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn runtime_global_and_panel_hover_mutations_repaint_live_widget_kinds() {
        let mut app = widgets_test_app();
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().background(PART_NORMAL_FILL).button("button"),
            |_| {},
        );
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .widget("slider", Slider::new(0.0..=1.0)),
            |_| {},
        );
        builder.with(
            El::new().background(PART_NORMAL_FILL).editable_field(
                "field",
                field,
                crate::TextStyle::new(10.0),
            ),
            |_| {},
        );
        let panel = spawn_widget_panel(&mut app, builder.build());
        app.update();
        let button = resolve_widget(&mut app, panel, "button");
        let slider = resolve_widget(&mut app, panel, "slider");
        let editable = resolve_widget(&mut app, panel, "field");
        for widget in [button, slider, editable] {
            app.world_mut()
                .entity_mut(widget)
                .insert(PickingInteraction::Hovered);
        }
        app.update();

        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(GLOBAL_PROPERTY_COLOR));
        app.update();
        for (widget, slot, fill) in [
            (
                button,
                VisualSlotId::BUTTON_ROOT,
                Some(GLOBAL_PROPERTY_COLOR),
            ),
            (slider, VisualSlotId::SLIDER_ROOT, None),
            (editable, VisualSlotId::EDITABLE_ROOT, None),
        ] {
            let root_index = app
                .world()
                .get::<WidgetVisualSlots>(widget)
                .and_then(|slots| slots.element_index(slot))
                .expect("widget root index");
            assert_eq!(
                part_override(&app, panel, root_index).and_then(|part| part.fill_color),
                fill,
            );
        }

        *app.world_mut()
            .get_mut::<Cascade<WidgetHoveredAppearance>>(panel)
            .expect("panel hovered appearance cascade") = Cascade::Override(
            WidgetHoveredAppearance::new(Appearance::new().background(PANEL_PROPERTY_COLOR)),
        );
        app.update();
        for (widget, slot, fill) in [
            (
                button,
                VisualSlotId::BUTTON_ROOT,
                Some(PANEL_PROPERTY_COLOR),
            ),
            (slider, VisualSlotId::SLIDER_ROOT, None),
            (editable, VisualSlotId::EDITABLE_ROOT, None),
        ] {
            let root_index = app
                .world()
                .get::<WidgetVisualSlots>(widget)
                .and_then(|slots| slots.element_index(slot))
                .expect("widget root index");
            assert_eq!(
                part_override(&app, panel, root_index).and_then(|part| part.fill_color),
                fill,
            );
        }
    }

    #[test]
    fn editable_field_never_presents_a_pressed_appearance() {
        let mut app = widgets_test_app();
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(GLOBAL_PROPERTY_COLOR));
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .editable_field("field", field, crate::TextStyle::new(10.0))
                .hovered(Appearance::new().background(GLOBAL_PROPERTY_COLOR)),
            |_| {},
        );
        let panel = spawn_widget_panel(&mut app, builder.build());
        app.update();
        let field = resolve_widget(&mut app, panel, "field");
        let root_index = app
            .world()
            .get::<WidgetVisualSlots>(field)
            .and_then(|slots| slots.element_index(VisualSlotId::EDITABLE_ROOT))
            .expect("editable field root index");

        app.world_mut()
            .entity_mut(field)
            .insert((ButtonPress, PickingInteraction::Hovered));
        *app.world_mut().resource_mut::<WidgetPressedAppearance>() =
            WidgetPressedAppearance::new(Appearance::new().background(WIDGET_PROPERTY_COLOR));
        app.update();

        assert_eq!(
            part_override(&app, panel, root_index),
            Some(VisualSlotOverride {
                fill_color: Some(GLOBAL_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
            "the hovered override proves present_editable_state ran after the pressed root changed",
        );
        assert_ne!(
            part_override(&app, panel, root_index),
            Some(VisualSlotOverride {
                fill_color: Some(WIDGET_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
            "an editable field must not present the pressed appearance",
        );
    }

    #[test]
    fn first_update_reifies_global_panel_widget_and_part_hovered_appearances() {
        let mut app = widgets_test_app();
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(GLOBAL_PROPERTY_COLOR));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(PART_NORMAL_FILL)
                .button("button")
                .hovered(Appearance::new().text_color(WIDGET_PROPERTY_COLOR)),
            |children| {
                children.text(
                    Text::new("part", TextStyle::new(10.0)).layout(
                        children
                            .child(
                                El::new()
                                    .background(PART_NORMAL_FILL)
                                    .border(Border::all(Px(1.0), PART_NORMAL_BORDER))
                                    .draw(PanelDraw::shapes([PanelCircle::new((10.0, 10.0), 5.0)])),
                            )
                            .hovered(Appearance::new().path_color(PART_PROPERTY_COLOR)),
                    ),
                );
            },
        );
        let tree = builder.build();
        let part_index = tree.len() - 1;
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(Appearance::new().border_color(PANEL_PROPERTY_COLOR))
            .with_tree(tree)
            .build()
            .expect("first-update panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "button");
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetHoveredAppearance>>(widget)
                .map(|resolved| resolved.0.appearance()),
            Some(
                &Appearance::new()
                    .background(GLOBAL_PROPERTY_COLOR)
                    .border_color(PANEL_PROPERTY_COLOR)
                    .text_color(WIDGET_PROPERTY_COLOR),
            ),
        );
        let world = app.world();
        let slots = world
            .get::<WidgetVisualSlots>(widget)
            .expect("first update widget visual slots");
        let hovered = world
            .get::<Resolved<WidgetHoveredAppearance>>(widget)
            .expect("resolved hovered appearance");
        let pressed = world
            .get::<Resolved<WidgetPressedAppearance>>(widget)
            .expect("press");
        let focused = world
            .get::<Resolved<crate::WidgetFocusedAppearance>>(widget)
            .expect("focus");
        let disabled = world
            .get::<Resolved<crate::WidgetDisabledAppearance>>(widget)
            .expect("disabled");
        let panel_component = world.get::<DiegeticPanel>(panel);
        let mut desired = WidgetVisualOverrides::default();
        super::resolve_part_overrides(
            &mut desired,
            slots,
            hovered,
            pressed,
            focused,
            disabled,
            world
                .get::<Cascade<WidgetHoveredAppearance>>(widget)
                .expect("hover state"),
            world
                .get::<Cascade<crate::WidgetDisabledAppearance>>(widget)
                .expect("authored disabled appearance"),
            &[Some(WidgetState::Hovered)],
            false,
            panel_component,
        );
        assert_eq!(
            desired
                .element_overrides()
                .iter()
                .find(|(element_index, _)| *element_index == part_index)
                .map(|(_, override_value)| override_value),
            Some(&VisualSlotOverride {
                fill_color: Some(GLOBAL_PROPERTY_COLOR),
                text_color: Some(WIDGET_PROPERTY_COLOR),
                path_color: Some(PART_PROPERTY_COLOR),
                border_color: Some(PANEL_PROPERTY_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn text_only_part_state_color_never_creates_a_fill_capability_or_override() {
        let mut app = widgets_test_app();
        let (tree, text_index) = text_part_tree(Appearance::new().text_color(TEXT_OVERRIDE_COLOR));
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("button should carry visual slots")
            .elements()
            .iter()
            .find(|(element_index, _)| *element_index == text_index)
            .map(|(_, capabilities)| *capabilities)
            .expect("text recipient should remain in visual slots");
        assert!(capabilities.contains(VisualElementCapabilities::TEXT));
        assert!(!capabilities.contains(VisualElementCapabilities::SDF_FILL));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let override_value = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, text_index)
            .expect("hovered text recipient should receive an override");
        assert_eq!(override_value.text_color, Some(TEXT_OVERRIDE_COLOR));
        assert_eq!(override_value.fill_color, None);
    }

    #[test]
    fn global_text_color_default_stays_dormant_on_textless_widget_part() {
        let mut app = widgets_test_app();
        app.insert_resource(WidgetHoveredAppearance::new(
            Appearance::new().text_color(TEXT_OVERRIDE_COLOR),
        ));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(El::new().id("container"), |_| {});
        });
        let tree = builder.build();
        let container_index = tree.len() - 1;
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
            .expect("global text-color default should not reject a textless widget part");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetHoveredAppearance>>(widget)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetHoveredAppearance::new(
                Appearance::new().text_color(TEXT_OVERRIDE_COLOR),
            )),
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, container_index)
                .and_then(|override_value| override_value.text_color),
            None,
        );
    }

    #[test]
    fn text_and_draw_part_state_colors_stay_role_isolated() {
        let mut app = widgets_test_app();
        let (tree, text_index) =
            text_and_draw_part_tree(Appearance::new().text_color(TEXT_OVERRIDE_COLOR));
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        let capabilities = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .expect("button should carry visual slots")
            .elements()
            .iter()
            .find(|(element_index, _)| *element_index == text_index)
            .map(|(_, capabilities)| *capabilities)
            .expect("text and draw recipient should remain in visual slots");
        assert!(capabilities.contains(VisualElementCapabilities::TEXT));
        assert!(capabilities.contains(VisualElementCapabilities::DRAW));

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let text_override = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, text_index)
            .expect("hovered text and draw recipient should receive an override");
        assert_eq!(text_override.text_color, Some(TEXT_OVERRIDE_COLOR));
        assert_eq!(text_override.path_color, None);

        let (tree, path_index) =
            text_and_draw_part_tree(Appearance::new().path_color(PATH_OVERRIDE_COLOR));
        assert_eq!(path_index, text_index);
        reauthor_tree(&mut app, panel, tree);
        let path_override = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, path_index)
            .expect("re-authored text and draw recipient should receive an override");
        assert_eq!(path_override.text_color, None);
        assert_eq!(path_override.path_color, Some(PATH_OVERRIDE_COLOR));
    }

    #[test]
    fn non_root_part_appearance_reaches_its_owned_element() {
        let mut app = widgets_test_app();
        let (tree, part_index) = button_part_tree(true, false);
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(indexed_fill(&app, panel, part_index), Some(PART_HOVER_FILL));
    }

    #[test]
    fn button_part_reauthoring_clears_and_moves_element_overrides() {
        let mut app = widgets_test_app();
        let (tree, original_part_index) = button_part_tree(true, false);
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "button");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );

        let (tree, moved_part_index) = button_part_tree(true, true);
        assert_ne!(moved_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(resolve_widget(&mut app, panel, "button"), widget);
        assert_eq!(
            indexed_fill(&app, panel, moved_part_index),
            Some(PART_HOVER_FILL)
        );
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);

        let (tree, restored_part_index) = button_part_tree(true, false);
        assert_eq!(restored_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );

        let (tree, cleared_part_index) = button_part_tree(false, false);
        assert_eq!(cleared_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);
    }

    #[test]
    fn editable_part_reauthoring_clears_and_moves_element_overrides() {
        let mut app = widgets_test_app();
        let (tree, original_part_index) = editable_part_tree(true, false);
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "editable");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );

        let (tree, moved_part_index) = editable_part_tree(true, true);
        assert_ne!(moved_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(resolve_widget(&mut app, panel, "editable"), widget);
        assert_eq!(
            indexed_fill(&app, panel, moved_part_index),
            Some(PART_HOVER_FILL)
        );
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);

        let (tree, restored_part_index) = editable_part_tree(true, false);
        assert_eq!(restored_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );

        let (tree, cleared_part_index) = editable_part_tree(false, false);
        assert_eq!(cleared_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);
    }

    #[test]
    fn slider_part_reauthoring_clears_and_moves_only_part_element_overrides() {
        let mut app = widgets_test_app();
        let (tree, original_part_index, styled_unauthored_index, structural_container_index) =
            slider_part_tree(true, false);
        let panel = spawn_widget_panel(&mut app, tree);
        app.update();
        let widget = resolve_widget(&mut app, panel, "slider");
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );
        assert_no_unowned_slider_part_overrides(
            &app,
            panel,
            widget,
            styled_unauthored_index,
            structural_container_index,
        );

        let (
            tree,
            moved_part_index,
            moved_styled_unauthored_index,
            moved_structural_container_index,
        ) = slider_part_tree(true, true);
        assert_ne!(moved_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(
            indexed_fill(&app, panel, moved_part_index),
            Some(PART_HOVER_FILL)
        );
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);
        assert_no_unowned_slider_part_overrides(
            &app,
            panel,
            widget,
            moved_styled_unauthored_index,
            moved_structural_container_index,
        );

        let (
            tree,
            restored_part_index,
            restored_styled_unauthored_index,
            restored_structural_container_index,
        ) = slider_part_tree(true, false);
        assert_eq!(restored_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(
            indexed_fill(&app, panel, original_part_index),
            Some(PART_HOVER_FILL)
        );
        assert_no_unowned_slider_part_overrides(
            &app,
            panel,
            widget,
            restored_styled_unauthored_index,
            restored_structural_container_index,
        );

        let (
            tree,
            cleared_part_index,
            cleared_styled_unauthored_index,
            cleared_structural_container_index,
        ) = slider_part_tree(false, false);
        assert_eq!(cleared_part_index, original_part_index);
        reauthor_tree(&mut app, panel, tree);
        assert_eq!(resolve_widget(&mut app, panel, "slider"), widget);
        assert_eq!(indexed_fill(&app, panel, original_part_index), None);
        assert_no_unowned_slider_part_overrides(
            &app,
            panel,
            widget,
            cleared_styled_unauthored_index,
            cleared_structural_container_index,
        );
    }

    #[test]
    fn properties_layer_independently() {
        // Focus authors only the border and hover only the fill, so both
        // survive when the two states are live together.
        let state_appearance = state_appearance();
        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Focused), Some(WidgetState::Hovered)],
            None,
        );
        assert_eq!(resolved.fill_color, Some(HOVER_FILL));
        assert_eq!(resolved.border_color, Some(FOCUS_BORDER));
    }

    #[test]
    fn later_layers_replace_earlier_ones_regardless_of_active_order() {
        // Pressed sits after hovered in the layer order, so it wins the fill
        // even when the caller lists hovered last.
        let state_appearance = state_appearance();
        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Pressed), Some(WidgetState::Hovered)],
            None,
        );
        assert_eq!(resolved.fill_color, Some(PRESS_FILL));
    }

    #[test]
    fn text_and_path_colors_layer_independently() {
        let state_appearance = StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().text_color(HOVER_TEXT),
            )),
            focused: Cascade::Override(WidgetFocusedAppearance::new(
                Appearance::new().path_color(FOCUS_PATH),
            )),
            ..StateAppearance::default()
        };

        let resolved = state_appearance.cascades().resolve(
            &[Some(WidgetState::Focused), Some(WidgetState::Hovered)],
            None,
        );

        assert_eq!(resolved.text_color, Some(HOVER_TEXT));
        assert_eq!(resolved.path_color, Some(FOCUS_PATH));
        assert_eq!(resolved.fill_color, None);
        assert_eq!(resolved.border_color, None);
    }

    #[test]
    fn inactive_states_author_nothing() {
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[None, None, None, None], None);
        assert_eq!(resolved.fill_color, None);
        assert_eq!(resolved.border_color, None);
        assert_eq!(resolved.border_widths, None);
    }

    #[test]
    fn border_width_resolves_through_the_owning_panel_scale() {
        let panel = panel();
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[Some(WidgetState::Focused)], Some(&panel));
        let widths = resolved
            .border_widths
            .expect("a focused width and a live panel resolve to render widths");
        // Two logical pixels resolve to points through the fixed 96 DPI
        // convention, then scale to panel world units.
        let expected = 2.0 * 0.75 * panel.points_to_world();
        for width in widths {
            assert!(
                (width - expected).abs() < 1e-6,
                "each side carries the same resolved width",
            );
        }
    }

    #[test]
    fn border_width_without_a_panel_leaves_the_authored_width() {
        let state_appearance = state_appearance();
        let resolved = state_appearance
            .cascades()
            .resolve(&[Some(WidgetState::Focused)], None);
        assert_eq!(resolved.border_widths, None);
        assert_eq!(
            resolved.border_color,
            Some(FOCUS_BORDER),
            "a missing panel must not suppress the other properties",
        );
    }
}
