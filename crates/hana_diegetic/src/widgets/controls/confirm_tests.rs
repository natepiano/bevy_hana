use bevy::ecs::system::RunSystemOnce;
use bevy::picking::hover::PickingInteraction;
use bevy::prelude::*;

use super::*;
use crate::Button;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::HeadlessDiegeticUiPlugin;
use crate::PanelWidgetReader;
use crate::RenderCommand;
use crate::RenderCommandKind;
use crate::Unit;
use crate::widgets::ButtonPress;
use crate::widgets::VisualOverrideIndex;

#[test]
fn confirm_labels_keep_source_contrast_at_rest_hover_and_press() -> Result<(), String> {
    let mut failures = Vec::new();
    for tone in [ConfirmTone::Normal, ConfirmTone::Destructive] {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let mut builder = LayoutBuilder::new(400.0, 60.0);
        ConfirmDialog::new(
            ConfirmChoice::new("accept", "Accept", Button::new()),
            ConfirmChoice::new("cancel", "Cancel", Button::new()),
            tone,
        )
        .append_actions(
            &mut builder,
            &ControlMetrics::from_label_size(13.0),
            &ControlPalette::system_dark(),
        );
        let panel = DiegeticPanel::world()
            .size(400.0, 60.0)
            .font_unit(Unit::Points)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        for (id, label) in [("accept", "Accept"), ("cancel", "Cancel")] {
            let widget = app
                .world_mut()
                .run_system_once(move |reader: PanelWidgetReader| {
                    reader.entity(panel, &PanelElementId::named(id))
                })
                .map_err(|error| error.to_string())?
                .ok_or("confirm widget")?;
            let mut fills = Vec::new();
            for state in ["rest", "hover", "press"] {
                if state == "hover" {
                    app.world_mut()
                        .entity_mut(widget)
                        .insert(PickingInteraction::Hovered);
                }
                if state == "press" {
                    app.world_mut().entity_mut(widget).insert(ButtonPress);
                }
                app.update();
                let (ink, fill) = source_colors(&app, panel, label)?;
                fills.push(luminance(fill));
                let ratio = contrast(ink, fill);
                if ratio < 4.5 {
                    failures.push(format!("{tone:?} {label} {state}: {ratio:.3}:1"));
                }
                let hint = match (tone, id) {
                    (ConfirmTone::Normal, "accept") => ConfirmRole::Default.hint(),
                    (ConfirmTone::Normal, _) => ConfirmRole::Cancel.hint(),
                    (ConfirmTone::Destructive, "accept") => "",
                    (ConfirmTone::Destructive, _) => ConfirmRole::SafeDefault.hint(),
                };
                if !hint.is_empty() {
                    let (ink, _) = source_colors(&app, panel, hint)?;
                    let ratio = contrast(ink, fill);
                    if ratio < 4.5 {
                        failures.push(format!("{tone:?} {label} {state} hint: {ratio:.3}:1"));
                    }
                    if tone == ConfirmTone::Normal && id == "cancel" {
                        let (label_ink, _) = source_colors(&app, panel, label)?;
                        let brightness = luminance(ink) / luminance(label_ink);
                        if brightness >= 1.0 {
                            failures.push(format!(
                                "neutral {state} hint must be dimmer than the label"
                            ));
                        } else if state != "hover" && brightness > 0.55 {
                            failures.push(format!("neutral {state} hint luminance {brightness:.3} must be at most 55% of the label"));
                        }
                    }
                }
            }
            let tinted = tone == ConfirmTone::Destructive || id == "accept";
            if tinted && fills[2] >= fills[1] {
                failures.push(format!(
                    "{tone:?} {label}: press luminance {} must be below hover luminance {}",
                    fills[2], fills[1]
                ));
            }
            app.world_mut()
                .entity_mut(widget)
                .remove::<ButtonPress>()
                .insert(PickingInteraction::None);
            app.update();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn confirm_default_ring_keeps_rest_colour() -> Result<(), String> {
    let mut failures = Vec::new();
    for tone in [ConfirmTone::Normal, ConfirmTone::Destructive] {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let dialog = ConfirmDialog::new(
            ConfirmChoice::new("accept", "Accept", Button::new()),
            ConfirmChoice::new("cancel", "Cancel", Button::new()),
            tone,
        );
        let id = dialog.default_id().clone();
        let label = match tone {
            ConfirmTone::Normal => "Accept",
            ConfirmTone::Destructive => "Cancel",
        };
        let mut builder = LayoutBuilder::new(500.0, 60.0);
        dialog.append_actions(
            &mut builder,
            &ControlMetrics::from_label_size(13.0),
            &ControlPalette::system_dark(),
        );
        let panel = DiegeticPanel::world()
            .size(500.0, 60.0)
            .font_unit(Unit::Points)
            .with_tree(builder.build())
            .build()
            .map_err(|error| error.to_string())?;
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .map_err(|error| error.to_string())?
            .ok_or("default widget")?;
        for state in ["rest", "hover", "press"] {
            if state == "hover" {
                app.world_mut()
                    .entity_mut(widget)
                    .insert(PickingInteraction::Hovered);
            }
            if state == "press" {
                app.world_mut().entity_mut(widget).insert(ButtonPress);
            }
            app.update();
            let commands = &app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .ok_or("confirm layout")?
                .layout()
                .solved()
                .ok_or("solved confirm")?
                .commands;
            let text = commands
                .iter()
                .find(|command| {
                    matches!(&command.kind,
                RenderCommandKind::Text { text, .. } if text == label)
                })
                .ok_or("default label")?;
            let (command, border) = commands
                .iter()
                .filter_map(|command| match command.kind {
                    RenderCommandKind::Border { border } if contains(command, text) => {
                        Some((command, border))
                    },
                    _ => None,
                })
                .min_by(|(a, _), (b, _)| {
                    (a.bounds.width * a.bounds.height)
                        .total_cmp(&(b.bounds.width * b.bounds.height))
                })
                .ok_or("default ring")?;
            let color = app
                .world()
                .resource::<VisualOverrideIndex>()
                .get(panel, command.element_idx)
                .and_then(|value| value.border_color)
                .unwrap_or(border.color);
            if color != ControlPalette::system_dark().knob {
                failures.push(format!(
                    "{tone:?} {state} default ring changed to {color:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn source_colors(app: &App, panel: Entity, label: &str) -> Result<(Color, Color), String> {
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or("confirm layout")?;
    let commands = &computed.layout().solved().ok_or("solved confirm")?.commands;
    let (text, ink) = commands
        .iter()
        .find_map(|command| match &command.kind {
            RenderCommandKind::Text { text, config }
                if text == label || text.starts_with(&format!("{label} ")) =>
            {
                Some((command, config.color()))
            },
            _ => None,
        })
        .ok_or("button text")?;
    let (face, color) = commands
        .iter()
        .filter_map(|command| match command.kind {
            RenderCommandKind::Rectangle { color, .. } if contains(command, text) => {
                Some((command, color))
            },
            _ => None,
        })
        .min_by(|(a, _), (b, _)| {
            (a.bounds.width * a.bounds.height).total_cmp(&(b.bounds.width * b.bounds.height))
        })
        .ok_or("button face")?;
    let overrides = app.world().resource::<VisualOverrideIndex>();
    let fill = overrides
        .get(panel, face.element_idx)
        .and_then(|value| value.fill_color)
        .unwrap_or(color);
    let ink = overrides
        .get(panel, text.element_idx)
        .and_then(|value| value.text_color)
        .unwrap_or(ink);
    Ok((ink, fill))
}

fn contains(face: &RenderCommand, text: &RenderCommand) -> bool {
    face.bounds.x <= text.bounds.x
        && face.bounds.y <= text.bounds.y
        && face.bounds.x + face.bounds.width >= text.bounds.x + text.bounds.width
        && face.bounds.y + face.bounds.height >= text.bounds.y + text.bounds.height
}

fn contrast(first: Color, second: Color) -> f32 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn luminance(color: Color) -> f32 {
    let color = color.to_linear();
    0.2126_f32.mul_add(
        color.red,
        0.7152_f32.mul_add(color.green, 0.0722 * color.blue),
    )
}
