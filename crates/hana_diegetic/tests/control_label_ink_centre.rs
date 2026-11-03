//! Drawn proof that single-line controls centre the cap-to-baseline span.
//! H is the flat cap/baseline probe; Open retains its real descender in the shot.

mod drawn_support;

use std::f32::consts::FRAC_PI_8;

use bevy::camera::ClearColorConfig;
use bevy::camera::Hdr;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::RunSystemOnce as _;
use bevy::prelude::*;
use bevy::render::view::DebandDither;
use bevy::render::view::Tonemapping;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;
use drawn_support::CaptureRegion;
use drawn_support::CaseApp;
use drawn_support::DrawnCase;
use drawn_support::dump_pixels;
use drawn_support::run_cases;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::Button;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::ControlPalette;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistry;
use hana_diegetic::FontWeight;
use hana_diegetic::ImeAppOwnedFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelText;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RenderCommandKind;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sizing;
use hana_diegetic::Slider;
use hana_diegetic::SliderReachableBand;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextInkMeasurement;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;

// Two physical samples per layout pixel resolve the 0.5 px acceptance gap.
const RASTER_SCALE: f32 = 2.0;
const VIEW: UVec2 = UVec2::new(2560, 1440);
const VIEW_WIDTH: f32 = 1280.0;
const VIEW_HEIGHT: f32 = 720.0;
const WORLD_WIDTH: f32 = 1.28;
const WORLD_HEIGHT: f32 = 0.72;
const PANEL_LAYER: usize = 31;
const LABEL_SIZE: f32 = 44.0 / 3.0;
const ROW_WIDTH: Px = Px(360.0);
const LABEL: &str = "H";
const LABEL_ID: &str = "measured-label";
const CONTROL_ID: &str = "measured-control";
const PANEL_FILL: Color = Color::srgb(0.025, 0.035, 0.05);
const ROW_FILL: Color = Color::srgb(0.10, 0.14, 0.20);
const BORDER: Color = Color::srgb(0.20, 0.26, 0.34);
const LABEL_INK: Color = Color::srgb(0.94, 0.97, 1.0);

#[derive(Clone, Copy, Debug)]
enum ControlKind {
    Button,
    OpenButton,
    TriggerButton,
    MenuRow,
    PaletteRow,
    TooltipTitle,
    PopUpButton,
    Switch,
    Slider,
    EditableField,
}

impl ControlKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::OpenButton => "Open button",
            Self::TriggerButton => "trigger button",
            Self::MenuRow => "menu row",
            Self::PaletteRow => "palette row",
            Self::TooltipTitle => "tooltip title",
            Self::PopUpButton => "pop-up button",
            Self::Switch => "switch",
            Self::Slider => "slider",
            Self::EditableField => "editable field",
        }
    }

    fn inner_height_and_inset(self, metrics: &ControlMetrics<Px>) -> (f32, f32) {
        match self {
            Self::Button
            | Self::OpenButton
            | Self::MenuRow
            | Self::PaletteRow
            | Self::TooltipTitle
            | Self::PopUpButton => (metrics.button.height.0, metrics.border_width.0),
            Self::TriggerButton => (metrics.switch.track_height.0, metrics.border_width.0),
            Self::Switch => (
                metrics.switch.track_height.0,
                metrics.switch.border_width.0 + metrics.switch.track_padding.0,
            ),
            Self::Slider | Self::EditableField => (metrics.field.height.0, metrics.border_width.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PixelRows {
    top:    u32,
    bottom: u32,
}

impl PixelRows {
    fn from_physical_edges(top: f32, bottom: f32) -> Result<Self, String> {
        let top = (0..VIEW.y)
            .find(|&row| pixel_centre(row) >= top)
            .ok_or_else(|| format!("inner top edge {top} is outside the capture"))?;
        let bottom = (0..VIEW.y)
            .rev()
            .find(|&row| pixel_centre(row) <= bottom)
            .ok_or_else(|| format!("inner bottom edge {bottom} is outside the capture"))?;
        if top > bottom {
            return Err(format!("inner box has no physical rows: {top}..={bottom}"));
        }
        Ok(Self { top, bottom })
    }
}

#[derive(Clone, Copy, Debug)]
struct PixelBounds {
    left:   u32,
    right:  u32,
    top:    u32,
    bottom: u32,
}

struct LabelCapture {
    labeled: Vec<u8>,
    blank:   Vec<u8>,
}

impl LabelCapture {
    fn ink_bounds(&self) -> Result<PixelBounds, String> {
        if self.labeled.len() != self.blank.len() {
            return Err(format!(
                "labeled capture has {} bytes, blank capture has {}",
                self.labeled.len(),
                self.blank.len()
            ));
        }
        let mut bounds = None::<PixelBounds>;
        for y in 0..VIEW.y {
            for x in 0..VIEW.x {
                let offset = pixel_offset(x, y)?;
                if self.labeled[offset..offset + 3] == self.blank[offset..offset + 3] {
                    continue;
                }
                let bounds = bounds.get_or_insert(PixelBounds {
                    left:   x,
                    right:  x,
                    top:    y,
                    bottom: y,
                });
                bounds.left = bounds.left.min(x);
                bounds.right = bounds.right.max(x);
                bounds.top = bounds.top.min(y);
                bounds.bottom = bounds.bottom.max(y);
            }
        }
        bounds.ok_or_else(|| "clearing the label changed no drawn pixels".to_string())
    }
}

fn pixel_centre(pixel: u32) -> f32 { f32::from(u16::try_from(pixel).unwrap_or(u16::MAX)) + 0.5 }

fn pixel_offset(x: u32, y: u32) -> Result<usize, String> {
    let pixel = y
        .checked_mul(VIEW.x)
        .and_then(|row| row.checked_add(x))
        .ok_or_else(|| format!("pixel coordinate {x},{y} overflowed"))?;
    let byte = pixel
        .checked_mul(4)
        .ok_or_else(|| format!("pixel coordinate {x},{y} overflowed its byte offset"))?;
    usize::try_from(byte).map_err(|error| format!("pixel offset conversion failed: {error}"))
}

fn metrics() -> ControlMetrics<Px> { ControlMetrics::from_label_size(Px(LABEL_SIZE)) }

fn label_text() -> Text {
    Text::new(LABEL, TextStyle::new(Px(LABEL_SIZE)).with_color(LABEL_INK))
        .measure_as(LABEL)
        .id(LABEL_ID)
}

fn add_button(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>, label: &str) {
    builder.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::fixed(metrics.button.height))
            .padding(Padding::xy(metrics.button.side_padding, Px(0.0)))
            .alignment(AlignX::Center, AlignY::Center)
            .background(ROW_FILL)
            .border(Border::all(metrics.border_width, BORDER))
            .corner_radius(CornerRadius::all(metrics.button.corner_radius))
            .button(CONTROL_ID),
        |builder| {
            let style = TextStyle::new(Px(LABEL_SIZE)).with_color(LABEL_INK);
            // The assembly's real Open control uses the bold face.
            let style = if label == "Open" {
                style.with_weight(FontWeight::BOLD)
            } else {
                style
            };
            builder.text(Text::new(label, style).measure_as("Open").id(LABEL_ID));
        },
    );
}

// Rear trigger buttons use a centred overlay of track-height square size.
// Put the flat-cap probe in that actual label slot; production Lucide icons
// retain their separate ink-centre rule in reification.
fn add_trigger_button(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    builder.with(
        El::overlay()
            .size(metrics.switch.track_height, metrics.switch.track_height)
            .alignment(AlignX::Center, AlignY::Center)
            .background(ROW_FILL)
            .border(Border::all(metrics.border_width, BORDER))
            .corner_radius(CornerRadius::all(metrics.button.corner_radius))
            .button(CONTROL_ID),
        |builder| {
            builder.text(label_text());
        },
    );
}

fn add_pop_up_button(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    builder.text(
        Button::new()
            .pop_up(CONTROL_ID, LABEL, "Rectangle", metrics)
            .id(LABEL_ID),
    );
}

fn add_switch(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    builder.with(
        El::row()
            .width(Sizing::fixed(ROW_WIDTH))
            .height(Sizing::fixed(metrics.button.height))
            .gap(metrics.row.gap)
            .alignment(AlignX::Left, AlignY::Center)
            .background(ROW_FILL),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(metrics.row.label_width))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text(label_text());
                },
            );
            builder.with(El::new().width(Sizing::GROW), |_| {});
            builder.with(
                El::new()
                    .size(metrics.switch.track_width, metrics.switch.track_height)
                    .padding(Padding::all(metrics.switch.track_padding))
                    .alignment(AlignX::Left, AlignY::Center)
                    .background(ControlPalette::system_dark().accent)
                    .border(Border::all(
                        metrics.switch.border_width,
                        ControlPalette::system_dark().accent,
                    ))
                    .corner_radius(CornerRadius::all(metrics.switch.track_radius()))
                    .switch(CONTROL_ID),
                |builder| {
                    builder.with(
                        El::new()
                            .size(metrics.switch.thumb.width, metrics.switch.thumb.height)
                            .background(ControlPalette::system_dark().knob)
                            .corner_radius(CornerRadius::all(metrics.switch.thumb.corner_radius()))
                            .switch_knob(),
                        |_| {},
                    );
                },
            );
        },
    );
}

fn add_slider(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    builder.with(
        El::row().align_y(AlignY::Center).gap(metrics.row.gap),
        |builder| {
            builder.slider(
                "slider-track",
                Slider::new(0.0..=1.0).value(0.5),
                metrics,
                SliderReachableBand::Hidden,
            );
            let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("readout"));
            builder.with(
                El::row().slider_value_field(CONTROL_ID, field, metrics),
                |builder| {
                    builder.text(
                        Text::new(
                            LABEL,
                            TextStyle::new(metrics.label_size)
                                .with_color(LABEL_INK)
                                .with_align(TextAlign::Right),
                        )
                        .id(LABEL_ID),
                    );
                },
            );
        },
    );
}

fn add_editable_field(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("measurement"));
    builder.with(
        El::row().text_value_field(CONTROL_ID, field, metrics),
        |builder| {
            builder.text(label_text());
        },
    );
}

fn add_menu_row(builder: &mut LayoutBuilder, metrics: &ControlMetrics<Px>) {
    builder.with(
        El::row()
            .size(ROW_WIDTH, metrics.button.height)
            .padding(Padding::xy(metrics.menu.text_inset, Px(0.0)))
            .align_y(AlignY::Center)
            .background(ROW_FILL)
            .button(CONTROL_ID),
        |builder| {
            builder.text(label_text());
        },
    );
}

fn add_palette_row(
    builder: &mut LayoutBuilder,
    metrics: &ControlMetrics<Px>,
) -> Result<(), String> {
    // The command palette centres already-trimmed text in its result row.
    let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
    let trims = fonts
        .font(FontId::MONOSPACE)
        .map_err(|error| error.to_string())?
        .metrics(LABEL_SIZE * Unit::Pixels.to_points())
        .shaped_line_cap_box_trims();
    builder.with(
        El::row()
            .size(ROW_WIDTH, metrics.button.height)
            .align_y(AlignY::Center)
            .background(ROW_FILL)
            .button(CONTROL_ID),
        |builder| {
            builder.text(label_text().trim_text_box(
                Px(trims.top() / Unit::Pixels.to_points()),
                Px(trims.bottom() / Unit::Pixels.to_points()),
            ));
        },
    );
    Ok(())
}

fn control_tree(kind: ControlKind) -> Result<LayoutTree, String> {
    let metrics = metrics();
    let mut builder = LayoutBuilder::with_root(
        El::overlay()
            .size(Px(VIEW_WIDTH), Px(VIEW_HEIGHT))
            .alignment(AlignX::Center, AlignY::Center)
            .background(PANEL_FILL),
    );
    match kind {
        ControlKind::Button => add_button(&mut builder, &metrics, LABEL),
        ControlKind::OpenButton => add_button(&mut builder, &metrics, "Open"),
        ControlKind::TriggerButton => add_trigger_button(&mut builder, &metrics),
        ControlKind::MenuRow => add_menu_row(&mut builder, &metrics),
        ControlKind::PaletteRow => add_palette_row(&mut builder, &metrics)?,
        ControlKind::TooltipTitle => {
            builder.text(
                label_text().layout(
                    El::new()
                        .size(ROW_WIDTH, metrics.button.height)
                        .alignment(AlignX::Left, AlignY::Center)
                        .background(ROW_FILL)
                        .border(Border::all(metrics.border_width, BORDER)),
                ),
            );
        },
        ControlKind::PopUpButton => add_pop_up_button(&mut builder, &metrics),
        ControlKind::Switch => add_switch(&mut builder, &metrics),
        ControlKind::Slider => add_slider(&mut builder, &metrics),
        ControlKind::EditableField => add_editable_field(&mut builder, &metrics),
    }
    Ok(builder.build())
}

fn spawn_cameras(app: &mut App, window: Entity, target: Handle<Image>, region: CaptureRegion) {
    app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(window)),
        Hdr,
        Tonemapping::KhronosPbrNeutral,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let mut camera = Camera {
        order: 110,
        clear_color: ClearColorConfig::Custom(Color::BLACK),
        ..default()
    };
    region.apply_to_camera(&mut camera);
    let distance = WORLD_HEIGHT * 0.5 / FRAC_PI_8.tan();
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        RenderTarget::Image(target.into()),
        Hdr,
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, distance).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(PANEL_LAYER),
    ));
}

fn capture_label(kind: ControlKind) -> Result<LabelCapture, String> {
    let mut app = CaseApp::new();
    let mut resolution = WindowResolution::new(VIEW.x, VIEW.y);
    resolution.set_scale_factor_override(Some(1.0));
    let window = app
        .world_mut()
        .spawn((
            Window {
                resolution,
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    let region = CaptureRegion::new(VIEW, UVec2::ZERO, VIEW);
    let target = app.target(region);
    spawn_cameras(&mut app, window, target.clone(), region);
    let panel = DiegeticPanel::world()
        .size(Px(VIEW_WIDTH), Px(VIEW_HEIGHT))
        .world_width(WORLD_WIDTH)
        .font_unit(Unit::Pixels)
        .anchor(Anchor::Center)
        .shadow_casting(ShadowCasting::Off)
        .with_tree(control_tree(kind)?)
        .build()
        .map_err(|error| format!("invalid {} panel: {error}", kind.name()))?;
    let panel = app
        .world_mut()
        .spawn((panel, Transform::IDENTITY, RenderLayers::layer(PANEL_LAYER)))
        .id();
    app.settle()?;
    let labeled = app.capture(target.clone())?;
    dump_pixels::<true>(
        "control_label_ink_centre",
        "",
        kind.name(),
        region,
        CaptureRegion::new(VIEW, UVec2::new(1120, 640), UVec2::new(320, 160)),
        &labeled,
    )?;

    if let Some(root) = std::env::var_os("HANA_CAP_SHOTS") {
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        image::save_buffer(
            std::path::Path::new(&root).join(format!("{}.png", kind.name().replace(' ', "_"))),
            &labeled,
            VIEW.x,
            VIEW.y,
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    }
    let label_id = PanelElementId::named(LABEL_ID);
    // A flat H exposes the cap top and baseline without counting Open's p.
    if matches!(kind, ControlKind::OpenButton) {
        let open_y = label_origin_y(&app, panel, "Open")?;
        let probe_id = label_id.clone();
        app.world_mut()
            .run_system_once(move |mut text: PanelText| text.set_text(panel, &probe_id, "H"))
            .map_err(|error| error.to_string())?;
        for _ in 0..4 {
            app.update();
        }
        let probe_y = label_origin_y(&app, panel, "H")?;
        if (open_y - probe_y).abs() > 0.001 {
            return Err(format!(
                "flat-cap probe moved Open's baseline: {open_y} -> {probe_y}"
            ));
        }
    }
    let labeled = app.capture(target.clone())?;
    let cleared = app
        .world_mut()
        .run_system_once(move |mut text: PanelText| text.set_text(panel, &label_id, ""))
        .map_err(|error| format!("clear {} label: {error}", kind.name()))?;
    if !cleared {
        return Err(format!("{} label run was not found", kind.name()));
    }
    for _ in 0..4 {
        app.update();
    }
    let blank = app.capture(target)?;
    Ok(LabelCapture { labeled, blank })
}

fn label_origin_y(app: &App, panel: Entity, label: &str) -> Result<f32, String> {
    app.world()
        .get::<ComputedDiegeticPanel>(panel)
        .and_then(|panel| panel.layout().solved())
        .and_then(|layout| {
            layout
                .commands
                .iter()
                .find_map(|command| match &command.kind {
                    RenderCommandKind::Text { text, .. } if text == label => Some(command.bounds.y),
                    _ => None,
                })
        })
        .ok_or_else(|| format!("no solved command for {label:?}"))
}

fn check_control_label(kind: ControlKind) -> Result<(), String> {
    let metrics = metrics();
    let (height, inset) = kind.inner_height_and_inset(&metrics);
    let inner = PixelRows::from_physical_edges(
        (VIEW_HEIGHT - height).mul_add(0.5, inset) * RASTER_SCALE,
        (VIEW_HEIGHT + height).mul_add(0.5, -inset) * RASTER_SCALE,
    )?;
    let ink = capture_label(kind)?.ink_bounds()?;
    let above = i32::try_from(ink.top)
        .and_then(|top| i32::try_from(inner.top).map(|inner_top| top - inner_top))
        .map_err(|error| format!("top-gap conversion failed: {error}"))?;
    let below = i32::try_from(inner.bottom)
        .and_then(|inner_bottom| i32::try_from(ink.bottom).map(|bottom| inner_bottom - bottom))
        .map_err(|error| format!("bottom-gap conversion failed: {error}"))?;
    let offset = f64::from(above - below) / (2.0 * f64::from(RASTER_SCALE));
    eprintln!(
        "measure: {} cap gap {:.2} px, baseline gap {:.2} px, offset {offset:.2} px",
        kind.name(),
        f64::from(above) / f64::from(RASTER_SCALE),
        f64::from(below) / f64::from(RASTER_SCALE)
    );
    if above < 0 || below < 0 {
        return Err(format!(
            "{} ink {ink:?} leaves its inner rows {inner:?}",
            kind.name()
        ));
    }
    if (above - below).abs() > 1 {
        return Err(format!(
            "{} ink {ink:?} is offset {offset:.1} px in inner rows {inner:?}",
            kind.name()
        ));
    }
    Ok(())
}

fn check_open_button() -> Result<(), String> { check_control_label(ControlKind::OpenButton) }
fn check_trigger_button() -> Result<(), String> { check_control_label(ControlKind::TriggerButton) }

fn check_menu_row() -> Result<(), String> { check_control_label(ControlKind::MenuRow) }
fn check_palette_row() -> Result<(), String> { check_control_label(ControlKind::PaletteRow) }
fn check_tooltip_title() -> Result<(), String> { check_control_label(ControlKind::TooltipTitle) }

fn check_button() -> Result<(), String> { check_control_label(ControlKind::Button) }

fn check_pop_up_button() -> Result<(), String> { check_control_label(ControlKind::PopUpButton) }

fn check_switch() -> Result<(), String> { check_control_label(ControlKind::Switch) }

fn check_slider() -> Result<(), String> { check_control_label(ControlKind::Slider) }

fn check_editable_field() -> Result<(), String> { check_control_label(ControlKind::EditableField) }

fn check_surrogate_text_edit(initial: &str, replacements: &[&str]) -> Result<(), String> {
    let mut app = CaseApp::new();
    let style = TextStyle::new(Pt(11.0));
    let mut builder = LayoutBuilder::with_root(El::column().size(Pt(240.0), Pt(120.0)));
    builder.text(
        Text::new(initial, style.clone())
            .measure_as("H")
            .id("edited-label")
            .layout(El::new().size(Pt(100.0), Pt(40.0)).align_y(AlignY::Center)),
    );
    let panel = DiegeticPanel::world()
        .size(Pt(240.0), Pt(120.0))
        .font_unit(Unit::Points)
        .with_tree(builder.build())
        .build()
        .map_err(|error| error.to_string())?;
    let panel = app.world_mut().spawn(panel).id();
    // No camera is needed for this command-placement check. Run the layout
    // and real text reification without waiting for display compensation.
    for _ in 0..4 {
        app.update();
    }
    let cap = match app
        .world()
        .resource::<FontRegistry>()
        .measure_ink("H", &style.as_measure())
        .map_err(|error| error.to_string())?
    {
        TextInkMeasurement::Visible(ink) => ink,
        TextInkMeasurement::NoVisibleInk => return Err("H must have visible ink".into()),
    };
    for &replacement in replacements {
        let next = replacement.to_owned();
        let changed = app
            .world_mut()
            .run_system_once(move |mut text: PanelText| {
                text.set_text(panel, &PanelElementId::named("edited-label"), next.as_str())
            })
            .map_err(|error| format!("cannot edit label: {error}"))?;
        if !changed {
            return Err("named label must accept the text edit".into());
        }
        for _ in 0..4 {
            app.update();
        }
        let dimensions = (app
            .world()
            .resource::<hana_diegetic::DiegeticTextMeasurer>()
            .measure_fn)(replacement, &style.as_measure());
        let expected = if replacement == "H" {
            20.0 - f32::midpoint(cap.top, cap.bottom)
        } else {
            // Preserve the line-box rule: overflowing text starts at the top
            // of its fixed content box instead of shifting above it.
            (40.0 - dimensions.height).max(0.0) * 0.5
        };
        let result = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|panel| panel.layout().solved())
            .ok_or("edited panel must be solved")?;
        let actual = result
            .commands
            .iter()
            .find_map(|command| match &command.kind {
                RenderCommandKind::Text { text, .. } if text == replacement => Some(command.bounds),
                _ => None,
            })
            .ok_or("edited label command must exist")?;
        if (actual.y - expected).abs() >= 0.001 {
            return Err(format!(
                "{initial:?} -> {replacement:?}: edited block starts at {}, expected {expected}",
                actual.y
            ));
        }
    }
    Ok(())
}

#[test]
fn control_cap_height_text_edits_enter_multiline_with_a_measurement_surrogate() -> Result<(), String>
{
    for separator in ["\u{2028}", "\u{2029}", "\n", "\r\n"] {
        check_surrogate_text_edit("H", &[&format!("H{separator}H"), "H"])?;
    }
    Ok(())
}

#[test]
fn control_cap_height_text_edits_return_to_single_line_with_a_measurement_surrogate()
-> Result<(), String> {
    for separator in ["\u{2028}", "\u{2029}", "\n", "\r\n"] {
        check_surrogate_text_edit(&format!("H{separator}H"), &["H"])?;
    }
    Ok(())
}

#[test]
fn every_control_label_centres_cap_height_above_baseline() -> Result<(), String> {
    run_cases(&[
        DrawnCase {
            name: "open_button",
            run:  check_open_button,
        },
        DrawnCase {
            name: "trigger_button",
            run:  check_trigger_button,
        },
        DrawnCase {
            name: "menu_row",
            run:  check_menu_row,
        },
        DrawnCase {
            name: "palette_row",
            run:  check_palette_row,
        },
        DrawnCase {
            name: "tooltip_title",
            run:  check_tooltip_title,
        },
        DrawnCase {
            name: "button",
            run:  check_button,
        },
        DrawnCase {
            name: "pop_up_button",
            run:  check_pop_up_button,
        },
        DrawnCase {
            name: "switch",
            run:  check_switch,
        },
        DrawnCase {
            name: "slider",
            run:  check_slider,
        },
        DrawnCase {
            name: "editable_field",
            run:  check_editable_field,
        },
    ])
}
