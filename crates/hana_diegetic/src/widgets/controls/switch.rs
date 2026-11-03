//! Controlled two-sided switch on the panel button click path.

use bevy::color::Color;
use bevy::ecs::schedule::ApplyDeferred;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Time;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::prelude::Without;

use super::CapsuleThumb;
use super::ControlPalette;
use super::button;
use super::metrics::DISABLED_CAPSULE_FILL;
use crate::Appearance;
use crate::ControlLength;
use crate::CornerRadius;
use crate::DiegeticPanel;
use crate::El;
use crate::PanelElementId;
use crate::Row;
use crate::Sizing;
use crate::SwitchMetrics;
use crate::WidgetBuilder;
use crate::WidgetInteractivity;
use crate::WidgetOwner;
use crate::WidgetPart;
use crate::widgets::ButtonClicked;
use crate::widgets::PanelWidget;
use crate::widgets::VisualSlotId;
use crate::widgets::VisualSlotOverride;
use crate::widgets::WidgetDimmed;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetSystems;
use crate::widgets::WidgetVisualOverrides;
use crate::widgets::WidgetVisualSlots;
use crate::widgets::presentation as visual;

const EASE_SECONDS: f32 = 0.12;
const MIN_FRAME_SECONDS: f32 = 1.0 / 60.0;
const ON_TRACK: Color = ControlPalette::system_dark().accent;
const OFF_TRACK: Color = ControlPalette::system_dark().switch_off_track;
const DISABLED_TRACK: Color = Color::srgb(0.22, 0.22, 0.23);
const DISABLED_ON_TRACK: Color = Color::srgb(0.12, 0.34, 0.56);
const DISABLED_KNOB: Color = DISABLED_CAPSULE_FILL;

impl<L: ControlLength> CapsuleThumb<L> {
    /// Builds the capsule inside a switch or slider, including its state fills.
    pub(super) fn element<W: WidgetOwner>(
        &self,
        builder: &WidgetBuilder<'_, W>,
    ) -> El<Row, WidgetPart> {
        builder
            .child(
                El::new()
                    .width(Sizing::fixed(self.width))
                    .height(Sizing::fixed(self.height))
                    .corner_radius(CornerRadius::all(self.corner_radius()))
                    .background(self.fill)
                    .border(self.border)
                    .shadow_casting(self.shadow)
                    .widget_interactivity(WidgetInteractivity::Disabled),
            )
            .hovered(Appearance::new().background(self.fill))
            .disabled(Appearance::new().background(self.disabled_fill))
    }
}

impl WidgetBuilder<'_, Switch> {
    /// Adds the shared capsule knob inside an authored switch track.
    pub fn switch_knob<L: ControlLength>(&mut self, metrics: &SwitchMetrics<L>) {
        self.with(metrics.thumb.element(self).switch_knob(), |_| {});
    }
}

/// Authored declaration for a two-sided panel switch.
///
/// [`El::switch`](crate::El::switch) creates the track. Its ordinary child
/// marked with [`El::switch_knob`](crate::El::switch_knob) supplies the knob.
/// The owner accepts [`SwitchToggleRequested`] by changing [`SwitchState`].
#[must_use]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Switch;

impl Switch {
    /// Creates a switch whose first applied side is on.
    pub const fn new() -> Self { Self }
}

/// Applied side of a panel switch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SwitchSide {
    /// The knob rests on the left and the track is unfilled.
    Off,
    /// The knob rests on the right and the track is filled.
    #[default]
    On,
}

impl SwitchSide {
    const fn other(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }

    const fn position(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::On => 1.0,
        }
    }
}

/// Applied switch side and the knob's eased position between its two stops.
#[derive(Clone, Component, Debug, PartialEq)]
pub struct SwitchState {
    /// Side last accepted by the widget owner.
    pub side: SwitchSide,
    position: f32,
}

impl Default for SwitchState {
    fn default() -> Self {
        Self {
            side:     SwitchSide::On,
            position: SwitchSide::On.position(),
        }
    }
}

impl SwitchState {
    /// Applies an initial side without easing from the default side.
    pub const fn set_initial_side(&mut self, side: SwitchSide) {
        self.side = side;
        self.position = side.position();
    }

    /// Applies the owner's decision and starts motion toward that side.
    pub const fn set_side(&mut self, side: SwitchSide) { self.side = side; }

    fn advance(&mut self, elapsed: f32) {
        let target = self.side.position();
        let distance = (elapsed / EASE_SECONDS).clamp(0.0, 1.0);
        if self.position < target {
            self.position = (self.position + distance).min(target);
        } else if self.position > target {
            self.position = (self.position - distance).max(target);
        }
    }
}

/// A click's proposed new side; the switch owner decides whether to apply it.
#[derive(Clone, Debug, EntityEvent)]
pub struct SwitchToggleRequested {
    /// Live switch widget entity.
    #[event_target]
    pub entity:    Entity,
    /// Panel-local identity of the switch.
    pub id:        PanelElementId,
    /// The side opposite the currently applied side.
    pub requested: SwitchSide,
}

/// Installs state reification, click proposals, and eased presentation.
pub(in crate::widgets) struct SwitchPlugin;

impl Plugin for SwitchPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(request_toggle_from_click).add_systems(
            Update,
            (
                reconcile_switch_state,
                ApplyDeferred,
                advance_switch_knobs,
                present_switches,
            )
                .chain()
                .after(WidgetSystems::ReifyCommandsApplied)
                .after(button::present_button_state)
                .before(WidgetSystems::PresentationCommandsApplied),
        );
    }
}

fn reconcile_switch_state(
    missing: Query<(Entity, &WidgetVisualSlots), (With<PanelWidget>, Without<SwitchState>)>,
    existing: Query<(Entity, &WidgetVisualSlots), With<SwitchState>>,
    mut commands: Commands,
) {
    for (entity, slots) in &missing {
        if slots.element_index(VisualSlotId::SWITCH_ROOT).is_some() {
            commands.entity(entity).insert(SwitchState::default());
        }
    }
    for (entity, slots) in &existing {
        if slots.element_index(VisualSlotId::SWITCH_ROOT).is_none() {
            commands.entity(entity).remove::<SwitchState>();
        }
    }
}

fn request_toggle_from_click(
    click: On<ButtonClicked>,
    switches: Query<(&SwitchState, Has<WidgetDisabled>)>,
    mut commands: Commands,
) {
    let Ok((state, false)) = switches.get(click.entity) else {
        return;
    };
    commands.trigger(SwitchToggleRequested {
        entity:    click.entity,
        id:        click.id.clone(),
        requested: state.side.other(),
    });
}

fn advance_switch_knobs(mut switches: Query<&mut SwitchState>, time: Option<Res<Time>>) {
    let elapsed = time.map_or(MIN_FRAME_SECONDS, |time| {
        time.delta_secs().max(MIN_FRAME_SECONDS)
    });
    for mut state in &mut switches {
        if (state.position - state.side.position()).abs() > f32::EPSILON {
            state.advance(elapsed);
        }
    }
}

fn knob_offset(slots: &WidgetVisualSlots, position: f32) -> Option<Vec2> {
    let track = slots.content_box(VisualSlotId::SWITCH_ROOT)?;
    let track_border = slots.border_box(VisualSlotId::SWITCH_ROOT)?;
    let knob = slots.border_box(VisualSlotId::SWITCH_KNOB)?;
    let center = knob.width.mul_add(0.5, knob.x);
    let start = knob.width.mul_add(0.5, track.x);
    let end = knob.width.mul_add(-0.5, track.x + track.width);
    Some(Vec2::new(
        (end - start).mul_add(position, start) - center,
        track_border.height.mul_add(0.5, track_border.y) - knob.height.mul_add(0.5, knob.y),
    ))
}

fn present_switches(
    switches: Query<(
        Entity,
        &SwitchState,
        &WidgetVisualSlots,
        &WidgetOf,
        Has<WidgetDisabled>,
        Has<WidgetDimmed>,
    )>,
    panels: Query<&DiegeticPanel>,
    mut overrides: Query<&mut WidgetVisualOverrides>,
    mut commands: Commands,
) {
    for (entity, state, slots, owner, disabled, _) in &switches {
        let mut desired = overrides
            .get(entity)
            .map_or_else(|_| WidgetVisualOverrides::default(), Clone::clone);
        let track_fill = if disabled {
            match state.side {
                SwitchSide::Off => DISABLED_TRACK,
                SwitchSide::On => DISABLED_ON_TRACK,
            }
        } else {
            match state.side {
                SwitchSide::Off => OFF_TRACK,
                SwitchSide::On => ON_TRACK,
            }
        };
        desired.set(
            VisualSlotId::SWITCH_ROOT,
            VisualSlotOverride {
                fill_color: Some(track_fill),
                border_color: (state.side == SwitchSide::On).then_some(track_fill),
                ..Default::default()
            },
        );
        let render_offset = panels.get(owner.panel()).ok().and_then(|panel| {
            knob_offset(slots, state.position).and_then(|offset| {
                visual::layout_delta_to_render_offset(offset, panel.points_to_world())
            })
        });
        desired.set(
            VisualSlotId::SWITCH_KNOB,
            VisualSlotOverride {
                offset: render_offset,
                fill_color: disabled.then_some(DISABLED_KNOB),
                ..Default::default()
            },
        );
        visual::write_widget_overrides(entity, desired, &mut overrides, &mut commands);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::time::TimeUpdateStrategy;

    use super::*;
    use crate::AlignX;
    use crate::AlignY;
    use crate::Appearance;
    use crate::Border;
    use crate::ButtonActivationSource;
    use crate::ComputedDiegeticPanel;
    use crate::ControlMetrics;
    use crate::CornerRadius;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessDiegeticUiPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::Padding;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::TextDimensions;
    use crate::WidgetDisabledAppearance;
    use crate::WidgetPressedAppearance;
    use crate::layout::BoundingBox;
    use crate::layout::LayoutEngine;
    use crate::layout::RenderCommandKind;
    use crate::widgets::ButtonPress;
    use crate::widgets::ComputedVisualSlot;
    use crate::widgets::VisualOverrideIndex;

    #[derive(Default, Resource)]
    struct RecordedToggle(Option<SwitchSide>);

    fn record_toggle(event: On<SwitchToggleRequested>, mut recorded: ResMut<RecordedToggle>) {
        recorded.0 = Some(event.requested);
    }

    #[test]
    fn track_colors_follow_the_system_dark_palette() {
        let palette = ControlPalette::system_dark();
        assert_eq!(ON_TRACK, palette.accent);
        assert_eq!(OFF_TRACK, palette.switch_off_track);
        assert_ne!(OFF_TRACK, palette.switch_border);
        let luminance = |color: Color| {
            let rgb = color.to_linear();
            0.2126_f32.mul_add(rgb.red, 0.7152_f32.mul_add(rgb.green, 0.0722 * rgb.blue))
        };
        let contrast = |a: Color, b: Color| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let drawn_knob = Color::srgb(220.0 / 255.0, 218.0 / 255.0, 214.0 / 255.0);
        assert!(contrast(drawn_knob, OFF_TRACK) >= 3.0);
        for stock in [
            Color::srgb(46.0 / 255.0, 52.0 / 255.0, 64.0 / 255.0),
            Color::srgb(11.0 / 255.0, 25.0 / 255.0, 46.0 / 255.0),
            Color::srgb(0.19, 0.215, 0.26),
            // The group stock after the measured loss from requested to drawn colour.
            Color::srgb(0.154, 0.177, 0.224),
        ] {
            assert!(contrast(palette.switch_border, stock) >= 3.0);
            assert!(contrast(ON_TRACK, stock) >= 3.0);
        }
    }

    #[test]
    fn a_click_requests_the_other_side() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SwitchPlugin)
            .init_resource::<RecordedToggle>()
            .add_observer(record_toggle);
        let entity = app.world_mut().spawn(SwitchState::default()).id();
        app.world_mut().trigger(ButtonClicked {
            entity,
            id: PanelElementId::named("power"),
            source: ButtonActivationSource::Semantic,
        });
        app.world_mut().flush();
        assert_eq!(
            app.world().resource::<RecordedToggle>().0,
            Some(SwitchSide::Off)
        );
        assert_eq!(
            app.world()
                .get::<SwitchState>(entity)
                .map(|state| state.side),
            Some(SwitchSide::On)
        );
    }

    #[test]
    fn a_reified_switch_carries_its_knob_slot() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let mut builder = LayoutBuilder::new(60.0, 25.0);
        builder.with(El::new().size(24.0, 12.0).switch("power"), |builder| {
            builder.with(El::new().size(10.0, 10.0).switch_knob(), |_| {});
        });
        let panel = DiegeticPanel::world()
            .size(Mm(60.0), Mm(25.0))
            .with_tree(builder.build())
            .build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else { return };
        app.world_mut().spawn(panel);
        app.update();

        let world = app.world_mut();
        let mut switches = world.query_filtered::<&WidgetVisualSlots, With<SwitchState>>();
        let slots: Vec<_> = switches.iter(world).collect();
        assert_eq!(slots.len(), 1);
        assert!(
            slots
                .iter()
                .all(|slots| slots.element_index(VisualSlotId::SWITCH_KNOB).is_some())
        );
    }

    #[test]
    fn set_side_moves_the_knob() {
        let slots = WidgetVisualSlots::new(vec![
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_ROOT,
                element_index: 0,
                border_box:    BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  30.0,
                    height: 12.0,
                },
                content_box:   BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  30.0,
                    height: 12.0,
                },
            },
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_KNOB,
                element_index: 1,
                border_box:    BoundingBox {
                    x:      0.0,
                    y:      1.0,
                    width:  10.0,
                    height: 10.0,
                },
                content_box:   BoundingBox {
                    x:      0.0,
                    y:      1.0,
                    width:  10.0,
                    height: 10.0,
                },
            },
        ]);
        let mut state = SwitchState::default();
        let on = knob_offset(&slots, state.position);
        assert!(on.is_some());
        let Some(on) = on else { return };
        state.set_side(SwitchSide::Off);
        assert_eq!(state.side, SwitchSide::Off);
        state.advance(EASE_SECONDS * 0.5);
        assert!(state.position > 0.0 && state.position < 1.0);
        state.advance(EASE_SECONDS);
        assert!(state.position.abs() < f32::EPSILON);
        assert_eq!(state.side, SwitchSide::Off);
        let off = knob_offset(&slots, state.position);
        assert!(off.is_some());
        let Some(off) = off else { return };
        assert!(on.x > off.x);
    }

    #[test]
    fn switch_knob_offset_centers_an_oversized_content_child_at_both_stops() {
        let track = BoundingBox {
            x:      0.0,
            y:      0.0,
            width:  26.0,
            height: 15.0,
        };
        // Two points of padding plus a one-point border on each side leave
        // less content height than the knob. Layout pins that child to the
        // content top, so presentation must correct the drawn centre.
        let content = BoundingBox {
            x:      3.0,
            y:      3.0,
            width:  20.0,
            height: 9.0,
        };
        let knob = BoundingBox {
            x:      3.0,
            y:      3.0,
            width:  11.0,
            height: 11.0,
        };
        let slots = WidgetVisualSlots::new(vec![
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_ROOT,
                element_index: 0,
                border_box:    track,
                content_box:   content,
            },
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_KNOB,
                element_index: 1,
                border_box:    knob,
                content_box:   knob,
            },
        ]);

        for position in [0.0, 1.0] {
            let offset = knob_offset(&slots, position);
            assert!(offset.is_some(), "switch slots should solve");
            let Some(offset) = offset else { return };
            let track_center = track.height.mul_add(0.5, track.y);
            let drawn_knob_center = knob.height.mul_add(0.5, knob.y + offset.y);
            assert!((drawn_knob_center - track_center).abs() < 1e-6);
            let drawn_knob_left = knob.x + offset.x;
            let expected_left = if position == 0.0 {
                content.x
            } else {
                content.x + content.width - knob.width
            };
            assert!((drawn_knob_left - expected_left).abs() < 1e-6);
        }
    }

    #[test]
    fn switch_presentation_keeps_button_interaction_appearance() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let panel = DiegeticPanel::world().size(Mm(60.0), Mm(25.0)).build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else { return };
        let panel = app.world_mut().spawn(panel).id();
        let slots = WidgetVisualSlots::new(vec![
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_ROOT,
                element_index: 0,
                border_box:    BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  30.0,
                    height: 12.0,
                },
                content_box:   BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  30.0,
                    height: 12.0,
                },
            },
            ComputedVisualSlot {
                slot:          VisualSlotId::SWITCH_KNOB,
                element_index: 1,
                border_box:    BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  10.0,
                    height: 10.0,
                },
                content_box:   BoundingBox {
                    x:      0.0,
                    y:      0.0,
                    width:  10.0,
                    height: 10.0,
                },
            },
        ]);
        let interaction = VisualSlotOverride {
            border_color: Some(Color::srgb(1.0, 0.5, 0.0)),
            ..Default::default()
        };
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(VisualSlotId::BUTTON_ROOT, interaction.clone());
        let widget = app
            .world_mut()
            .spawn((
                SwitchState::default(),
                slots,
                WidgetOf::new(panel),
                overrides,
            ))
            .id();
        assert!(app.world_mut().run_system_once(present_switches).is_ok());
        assert_eq!(
            app.world()
                .get::<WidgetVisualOverrides>(widget)
                .and_then(|overrides| overrides.get(VisualSlotId::BUTTON_ROOT)),
            Some(&interaction),
        );
    }

    #[test]
    fn on_switch_keeps_accent_during_hover_press_and_focus() {
        const HOVER_FILL: Color = Color::srgb(0.2, 0.4, 0.8);
        const PRESS_FILL: Color = Color::srgb(0.1, 0.3, 0.6);
        const KNOB_FILL: Color = Color::srgb(0.9, 0.9, 0.85);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let window = app.world_mut().spawn(bevy::prelude::Window::default()).id();
        app.insert_resource(WidgetPressedAppearance::new(
            Appearance::new().background(PRESS_FILL),
        ));
        let mut builder = LayoutBuilder::new(60.0, 25.0);
        builder.with(
            El::new()
                .size(26.0, 15.0)
                .background(OFF_TRACK)
                .switch("power")
                .hovered(Appearance::new().background(HOVER_FILL))
                .focused(Appearance::new().background(HOVER_FILL)),
            |builder| {
                builder.with(
                    El::new()
                        .size(11.0, 11.0)
                        .background(KNOB_FILL)
                        .switch_knob(),
                    |_| {},
                );
            },
        );
        let panel = DiegeticPanel::world()
            .size(Mm(60.0), Mm(25.0))
            .with_tree(builder.build())
            .build();
        assert!(panel.is_ok());
        let Ok(panel) = panel else { return };
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("power"))
            });
        assert!(widget.is_ok());
        let Ok(Some(widget)) = widget else { return };
        assert!(app.world().get::<WidgetVisualSlots>(widget).is_some());
        let Some(slots) = app.world().get::<WidgetVisualSlots>(widget) else {
            return;
        };
        assert!(slots.element_index(VisualSlotId::SWITCH_ROOT).is_some());
        let Some(track_index) = slots.element_index(VisualSlotId::SWITCH_ROOT) else {
            return;
        };
        assert!(slots.element_index(VisualSlotId::SWITCH_KNOB).is_some());
        let Some(knob_index) = slots.element_index(VisualSlotId::SWITCH_KNOB) else {
            return;
        };

        for side in [SwitchSide::Off, SwitchSide::On] {
            assert!(app.world().get::<SwitchState>(widget).is_some());
            if let Some(mut state) = app.world_mut().get_mut::<SwitchState>(widget) {
                state.set_initial_side(side);
            }
            app.world_mut()
                .entity_mut(widget)
                .insert(PickingInteraction::Hovered);
            app.update();
            let index = app.world().resource::<VisualOverrideIndex>();
            assert_switch_track_fill(&app, panel, track_index, side);
            assert_eq!(
                index
                    .get(panel, knob_index)
                    .and_then(|value| value.fill_color),
                None,
            );

            app.world_mut().entity_mut(widget).insert(ButtonPress);
            app.update();
            let index = app.world().resource::<VisualOverrideIndex>();
            assert_switch_track_fill(&app, panel, track_index, side);
            assert_eq!(
                index
                    .get(panel, knob_index)
                    .and_then(|value| value.fill_color),
                None,
            );
            app.world_mut()
                .entity_mut(widget)
                .remove::<ButtonPress>()
                .insert(PickingInteraction::None);
            app.update();
            app.world_mut()
                .trigger(crate::RequestWidgetFocus { window, widget });
            app.update();
            assert!(app.world().get::<crate::WidgetFocused>(widget).is_some());
            assert_switch_track_fill(&app, panel, track_index, side);
        }
    }

    fn assert_switch_track_fill(app: &App, panel: Entity, track_index: usize, side: SwitchSide) {
        let actual = app
            .world()
            .resource::<VisualOverrideIndex>()
            .get(panel, track_index)
            .and_then(|value| value.fill_color);
        let expected = match side {
            SwitchSide::On => ON_TRACK,
            SwitchSide::Off => OFF_TRACK,
        };
        assert_eq!(actual, Some(expected));
    }

    fn switch_visual_fixture(disabled_fill: Color) -> Option<(App, Entity, Entity, usize, usize)> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin)
            .init_resource::<RecordedToggle>()
            .add_observer(record_toggle)
            .insert_resource(WidgetDisabledAppearance::new(
                Appearance::new().background(disabled_fill),
            ));
        let mut builder = LayoutBuilder::new(60.0, 25.0);
        builder.with(
            El::new()
                .size(26.0, 15.0)
                .background(OFF_TRACK)
                .border(Border::all(
                    ControlMetrics::from_label_size(Px(13.0))
                        .switch
                        .border_width,
                    ControlPalette::system_dark().switch_border,
                ))
                .switch("power"),
            |builder| {
                builder.with(
                    El::new()
                        .size(11.0, 11.0)
                        .background(ControlPalette::system_dark().knob)
                        .switch_knob(),
                    |_| {},
                );
            },
        );
        let panel = DiegeticPanel::world()
            .size(Mm(60.0), Mm(25.0))
            .with_tree(builder.build())
            .build()
            .ok()?;
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("power"))
            })
            .ok()??;
        let slots = app.world().get::<WidgetVisualSlots>(widget)?;
        let track_index = slots.element_index(VisualSlotId::SWITCH_ROOT)?;
        let knob_index = slots.element_index(VisualSlotId::SWITCH_KNOB)?;
        Some((app, panel, widget, track_index, knob_index))
    }

    #[test]
    fn on_switch_edge_matches_track_while_off_keeps_its_outline() {
        let fixture = switch_visual_fixture(Color::BLACK);
        assert!(fixture.is_some(), "switch fixture");
        let Some((mut app, panel, widget, track_index, _)) = fixture else {
            return;
        };
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        let authored_edge = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| computed.layout().solved())
            .and_then(|layout| {
                layout
                    .commands
                    .iter()
                    .find_map(|command| match &command.kind {
                        RenderCommandKind::Border { border } => Some(border.color),
                        _ => None,
                    })
            });
        assert_eq!(
            authored_edge,
            Some(ControlPalette::system_dark().switch_border)
        );

        let edge = |app: &App| {
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, track_index)
                .and_then(|visual| visual.border_color)
        };
        assert_eq!(edge(&app), Some(ON_TRACK));

        let state = app.world_mut().get_mut::<SwitchState>(widget);
        assert!(state.is_some(), "switch state");
        if let Some(mut state) = state {
            state.set_side(SwitchSide::Off);
        }
        app.update();
        assert_eq!(edge(&app), None, "Off retains the authored edge");
        assert_ne!(authored_edge, Some(OFF_TRACK));
        assert_eq!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, track_index)
                .and_then(|visual| visual.fill_color),
            Some(OFF_TRACK)
        );
        assert!(
            app.world()
                .get::<SwitchState>(widget)
                .is_some_and(|state| state.position > 0.0)
        );

        let state = app.world_mut().get_mut::<SwitchState>(widget);
        assert!(state.is_some(), "switch state");
        if let Some(mut state) = state {
            state.set_initial_side(SwitchSide::Off);
            state.set_side(SwitchSide::On);
        }
        app.update();
        assert_eq!(edge(&app), Some(ON_TRACK));
        assert!(
            app.world()
                .get::<SwitchState>(widget)
                .is_some_and(|state| state.position < 1.0)
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(WidgetDisabled::test_marker());
        app.update();
        assert_eq!(edge(&app), Some(DISABLED_ON_TRACK));
    }

    #[test]
    fn dimmed_on_switch_keeps_accent_and_activation() {
        const DISABLED_FILL: Color = Color::srgb(0.18, 0.18, 0.19);

        let fixture = switch_visual_fixture(DISABLED_FILL);
        assert!(fixture.is_some());
        let Some((mut app, panel, widget, track_index, knob_index)) = fixture else {
            return;
        };
        let index = app.world().resource::<VisualOverrideIndex>();
        assert_eq!(
            index
                .get(panel, track_index)
                .and_then(|part| part.fill_color),
            Some(ON_TRACK)
        );

        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());
        let dimmed_overrides = app.world().get::<WidgetVisualOverrides>(widget);
        assert_eq!(
            dimmed_overrides
                .and_then(|overrides| overrides.get(VisualSlotId::SWITCH_ROOT))
                .and_then(|part| part.fill_color),
            Some(ON_TRACK),
        );
        let dimmed_index = app.world().resource::<VisualOverrideIndex>();
        let dimmed_track = dimmed_index
            .get(panel, track_index)
            .and_then(|part| part.fill_color);
        let dimmed_knob = dimmed_index
            .get(panel, knob_index)
            .and_then(|part| part.fill_color);
        app.world_mut().trigger(ButtonClicked {
            entity: widget,
            id:     PanelElementId::named("power"),
            source: ButtonActivationSource::Semantic,
        });
        app.world_mut().flush();
        assert_eq!(
            app.world().resource::<RecordedToggle>().0,
            Some(SwitchSide::Off)
        );

        app.world_mut().entity_mut(widget).remove::<WidgetDimmed>();
        app.world_mut().resource_mut::<RecordedToggle>().0 = None;
        app.world_mut()
            .entity_mut(widget)
            .insert(WidgetDisabled::test_marker());
        app.update();
        let overrides = app.world().get::<WidgetVisualOverrides>(widget);
        assert_eq!(
            overrides
                .and_then(|overrides| overrides.get(VisualSlotId::SWITCH_ROOT))
                .and_then(|part| part.fill_color),
            Some(DISABLED_ON_TRACK),
        );
        let index = app.world().resource::<VisualOverrideIndex>();
        assert_eq!(
            index
                .get(panel, track_index)
                .and_then(|part| part.fill_color),
            Some(DISABLED_ON_TRACK),
        );
        assert_eq!(
            index
                .get(panel, knob_index)
                .and_then(|part| part.fill_color),
            Some(DISABLED_KNOB),
        );
        assert_eq!(dimmed_track, Some(ON_TRACK));
        assert_eq!(dimmed_knob, None);
        app.world_mut().trigger(ButtonClicked {
            entity: widget,
            id:     PanelElementId::named("power"),
            source: ButtonActivationSource::Semantic,
        });
        app.world_mut().flush();
        assert_eq!(app.world().resource::<RecordedToggle>().0, None);

        app.world_mut()
            .entity_mut(widget)
            .remove::<WidgetDisabled>();
        app.update();
        let index = app.world().resource::<VisualOverrideIndex>();
        assert_eq!(
            index
                .get(panel, track_index)
                .and_then(|part| part.fill_color),
            Some(ON_TRACK)
        );
        assert_eq!(
            index
                .get(panel, knob_index)
                .and_then(|part| part.fill_color),
            None
        );
    }

    fn switch_face_slots(label_size: Px) -> WidgetVisualSlots {
        let metrics = ControlMetrics::from_label_size(label_size);
        let mut builder = LayoutBuilder::new(80.0, 40.0);
        builder.with(
            El::new()
                .size(metrics.switch.track_width, metrics.switch.track_height)
                .padding(Padding::xy(
                    metrics.switch.track_padding,
                    metrics.switch.track_padding,
                ))
                .alignment(AlignX::Left, AlignY::Center)
                .background(Color::BLACK)
                .border(Border::all(metrics.switch.border_width, Color::BLACK))
                .corner_radius(CornerRadius::all(metrics.switch.track_radius()))
                .switch("power"),
            |builder| {
                builder.with(
                    El::new()
                        .size(metrics.switch.thumb.width, metrics.switch.thumb.height)
                        .background(Color::WHITE)
                        .corner_radius(CornerRadius::all(metrics.switch.thumb.corner_radius()))
                        .switch_knob(),
                    |_| {},
                );
            },
        );
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions {
            width:       0.0,
            height:      0.0,
            line_height: 0.0,
        }));
        let tree = builder.build();
        let result = engine.compute(&tree, 80.0, 40.0, 1.0);
        let records = tree.computed_widget_records(&result);
        WidgetVisualSlots::new(records[0].visual_slots().to_vec())
    }

    fn switch_knob_at_side(slots: &WidgetVisualSlots, side: SwitchSide) -> Option<BoundingBox> {
        let mut knob = slots.border_box(VisualSlotId::SWITCH_KNOB)?;
        let offset = knob_offset(slots, side.position())?;
        knob.x += offset.x;
        knob.y += offset.y;
        Some(knob)
    }

    #[test]
    fn switch_thumb_is_wider_than_tall_by_the_measured_macos_ratio() {
        let slots = switch_face_slots(Px(13.0));
        let knob = switch_knob_at_side(&slots, SwitchSide::Off);
        assert!(knob.is_some(), "solved switch knob and offset");
        let Some(knob) = knob else { return };
        assert!(knob.width > knob.height);
        assert!((knob.width / knob.height - 52.0 / 32.0).abs() < 1e-4);
    }

    #[test]
    fn switch_thumb_end_radius_is_half_its_height() {
        let switch = ControlMetrics::from_label_size(Px(13.0)).switch;
        assert!(
            switch
                .thumb
                .height
                .0
                .mul_add(-0.5, switch.thumb.corner_radius().0)
                .abs()
                < 1e-4
        );
        assert!(
            (switch.thumb.corner_radius().0 / switch.track_height.0 - 16.0 / 40.0).abs() < 1e-4
        );
        assert!(switch.thumb.width.0 > 2.0 * switch.thumb.corner_radius().0);
    }

    #[test]
    fn switch_off_and_on_are_mirrors_with_equal_measured_resting_gaps() {
        let slots = switch_face_slots(Px(13.0));
        let track = slots.border_box(VisualSlotId::SWITCH_ROOT);
        assert!(track.is_some(), "solved track");
        let Some(track) = track else { return };
        let off = switch_knob_at_side(&slots, SwitchSide::Off);
        assert!(off.is_some(), "solved switch knob and offset");
        let Some(off) = off else { return };
        let on = switch_knob_at_side(&slots, SwitchSide::On);
        assert!(on.is_some(), "solved switch knob and offset");
        let Some(on) = on else { return };
        let left_gap = off.x - track.x;
        let right_gap = track.x + track.width - on.x - on.width;
        assert!((left_gap - right_gap).abs() < 1e-4);
        assert!((left_gap / track.height - 4.0 / 40.0).abs() < 1e-4);
        assert!((track.x.mul_add(-2.0, off.x + on.x + on.width) - track.width).abs() < 1e-4);
        assert!((off.y - on.y).abs() < 1e-4);
    }

    #[test]
    fn off_switch_thumb_clear_band_is_at_least_the_pre_capsule_gap() {
        for label_size in [Px(6.5), Px(13.0), Px(26.0)] {
            let slots = switch_face_slots(label_size);
            let track = slots.border_box(VisualSlotId::SWITCH_ROOT);
            assert!(track.is_some(), "solved track");
            let Some(track) = track else { return };
            let knob = switch_knob_at_side(&slots, SwitchSide::Off);
            assert!(knob.is_some(), "solved switch knob and offset");
            let Some(knob) = knob else { return };
            let ring = ControlMetrics::from_label_size(label_size)
                .switch
                .border_width
                .0;
            let end_band = knob.x - track.x - ring;
            let top_band = knob.y - track.y - ring;
            let bottom_band = track.y + track.height - knob.y - knob.height - ring;
            for band in [end_band, top_band, bottom_band] {
                assert!(band / track.height >= 0.066);
            }
        }
    }

    #[test]
    fn switch_thumb_outer_edge_distance_is_equal_in_on_and_off_with_half_width_ring() {
        for label_size in [Px(6.5), Px(13.0), Px(26.0)] {
            let metrics = ControlMetrics::from_label_size(label_size);
            assert!(
                metrics
                    .border_width
                    .0
                    .mul_add(-0.5, metrics.switch.border_width.0)
                    .abs()
                    < 1e-4
            );
            let slots = switch_face_slots(label_size);
            let track = slots.border_box(VisualSlotId::SWITCH_ROOT);
            assert!(track.is_some(), "solved track");
            let Some(track) = track else { return };
            let off = switch_knob_at_side(&slots, SwitchSide::Off);
            assert!(off.is_some(), "solved switch knob and offset");
            let Some(off) = off else { return };
            let on = switch_knob_at_side(&slots, SwitchSide::On);
            assert!(on.is_some(), "solved switch knob and offset");
            let Some(on) = on else { return };
            let end_gaps = [off.x - track.x, track.x + track.width - on.x - on.width];
            assert!((end_gaps[0] - end_gaps[1]).abs() < 1e-4);
            for (knob, end_gap) in [(off, end_gaps[0]), (on, end_gaps[1])] {
                let top_gap = knob.y - track.y;
                let bottom_gap = track.y + track.height - knob.y - knob.height;
                for gap in [end_gap, top_gap, bottom_gap] {
                    assert!((gap / track.height - 0.1).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    fn switch_travel_equals_track_width_minus_thumb_width_and_both_gaps() {
        let slots = switch_face_slots(Px(13.0));
        let track = slots.border_box(VisualSlotId::SWITCH_ROOT);
        assert!(track.is_some(), "solved track");
        let Some(track) = track else { return };
        let off = switch_knob_at_side(&slots, SwitchSide::Off);
        assert!(off.is_some(), "solved switch knob and offset");
        let Some(off) = off else { return };
        let on = switch_knob_at_side(&slots, SwitchSide::On);
        assert!(on.is_some(), "solved switch knob and offset");
        let Some(on) = on else { return };
        let left_gap = off.x - track.x;
        let right_gap = track.x + track.width - on.x - on.width;
        let travel = on.x - off.x;
        assert!((travel - (track.width - off.width - left_gap - right_gap)).abs() < 1e-4);
        assert!((travel / track.height - 28.0 / 40.0).abs() < 1e-4);
    }

    #[test]
    fn switch_thumb_stays_inside_track_at_both_stops_for_small_and_large_labels() {
        for label_size in [Px(6.5), Px(26.0)] {
            let slots = switch_face_slots(label_size);
            let track = slots.border_box(VisualSlotId::SWITCH_ROOT);
            assert!(track.is_some(), "solved track");
            let Some(track) = track else { return };
            // Direct engine solves retain the tree's authored numeric layout units.
            assert!((track.height - label_size.0 * 15.0 / 13.0).abs() < 1e-4);
            for side in [SwitchSide::Off, SwitchSide::On] {
                let knob = switch_knob_at_side(&slots, side);
                assert!(knob.is_some(), "solved switch knob and offset");
                let Some(knob) = knob else { return };
                assert!(knob.x >= track.x);
                assert!(knob.y >= track.y);
                assert!(knob.x + knob.width <= track.x + track.width);
                assert!(knob.y + knob.height <= track.y + track.height);
                let top_gap = knob.y - track.y;
                let bottom_gap = track.y + track.height - knob.y - knob.height;
                assert!((top_gap - bottom_gap).abs() < 1e-4);
                assert!((top_gap / track.height - 4.0 / 40.0).abs() < 1e-4);
                assert!((knob.width / track.height - 52.0 / 40.0).abs() < 1e-4);
                assert!((knob.height / track.height - 32.0 / 40.0).abs() < 1e-4);
            }
        }
    }
}
