//! Tooltip appearance observed through materialized panels and layout commands.
#![allow(
    clippy::expect_used,
    reason = "invalid headless test fixtures should fail immediately"
)]

use std::time::Duration;

use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::TransformPlugin;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlPalette;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::Dimension;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Tooltip;

const VIEW: Vec2 = Vec2::new(800.0, 600.0);
const TIP_TEXT: &str = "tooltip look";

#[derive(Clone, Copy)]
enum HostPanelScale {
    PointSized,
    MeterSized,
}

struct Scene {
    app:     App,
    window:  Entity,
    control: Entity,
    host:    Entity,
    tooltip: Entity,
}

impl Scene {
    fn new(declaration: Tooltip) -> Self {
        Self::with_host_scale(declaration, HostPanelScale::PointSized)
    }

    fn with_host_scale(declaration: Tooltip, scale: HostPanelScale) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<Assets<Mesh>>()
            .add_plugins(HeadlessDiegeticUiPlugin);

        let mut window_component = Window::default();
        window_component.resolution.set(VIEW.x, VIEW.y);
        let window = app
            .world_mut()
            .spawn((window_component, PrimaryWindow))
            .id();
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: VIEW.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(VIEW.x, VIEW.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let camera_transform = Transform::from_xyz(0.0, 0.0, 5.0);
        app.world_mut().spawn((
            camera,
            RenderTarget::Window(WindowRef::Entity(window)),
            camera_transform,
            GlobalTransform::from(camera_transform),
        ));

        let (width, height, label_size) = match scale {
            HostPanelScale::PointSized => (80.0, 40.0, Dimension::from(fairy_dust::LABEL_SIZE)),
            HostPanelScale::MeterSized => (0.8, 0.4, Dimension::from(hana_diegetic::Mm(38.0))),
        };
        let mut tree = LayoutBuilder::new(width, height);
        tree.with(
            El::new()
                .size(width, height)
                .button("control")
                .tooltip(declaration),
            |tree| {
                // The point-sized fixture matches the widgets example; the
                // meter-sized fixture exercises physical tooltip text size.
                tree.text(Text::new("Row", TextStyle::new(label_size)));
            },
        );
        let panel_tree = tree.build();
        let panel = match scale {
            HostPanelScale::PointSized => DiegeticPanel::world()
                .size(Pt(80.0), Pt(40.0))
                .world_width(0.8)
                .with_tree(panel_tree)
                .build(),
            HostPanelScale::MeterSized => DiegeticPanel::world()
                .size(0.8, 0.4)
                .world_width(0.8)
                .with_tree(panel_tree)
                .build(),
        }
        .expect("host panel");
        let host = app
            .world_mut()
            .spawn((panel, Transform::from_xyz(-2.2, 0.0, 0.0)))
            .id();
        for _ in 0..4 {
            app.update();
        }
        let control = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(host, &hana_diegetic::PanelElementId::named("control"))
            })
            .expect("widget query")
            .expect("control widget");
        let tooltip = app
            .world()
            .get::<hana_diegetic::Tooltips>(control)
            .and_then(|controllers| controllers.iter().next())
            .expect("tooltip controller");
        Self {
            app,
            window,
            control,
            host,
            tooltip,
        }
    }

    fn focus(&mut self) {
        self.app.world_mut().trigger(RequestWidgetFocus {
            window: self.window,
            widget: self.control,
        });
        for _ in 0..30 {
            self.app.update();
        }
    }

    fn command(&self, panel: Entity, prefix: &str) -> String {
        let world = self.app.world();
        let layout = world
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel")
            .layout()
            .solved()
            .expect("solved panel layout");
        layout
            .commands
            .iter()
            .map(|command| format!("{:?}", command.kind))
            .find(|kind| kind.starts_with(prefix))
            .expect("matching layout command")
    }

    fn tooltip_width(&self) -> f32 {
        self.app
            .world()
            .get::<DiegeticPanel>(self.tooltip)
            .expect("materialized tooltip")
            .width()
    }

    fn border_command(&self) -> String { self.command(self.tooltip, "Border {") }

    fn look(&self) -> TooltipLook {
        let text = self.command(self.tooltip, "Text { text: \"alpha");
        TooltipLook {
            style:  text
                .split_once("config: ")
                .expect("text style")
                .1
                .to_owned(),
            border: self.border_command(),
        }
    }

    fn screen_panel(&mut self, position: Vec2, size: Vec2) -> Entity {
        let panel = DiegeticPanel::screen()
            .size(Px(size.x), Px(size.y))
            .anchor(Anchor::Center)
            .screen_position(position.x, position.y)
            .window_entity(self.window)
            .camera_order(1)
            .with_tree(LayoutBuilder::new(size.x, size.y).build())
            .build()
            .expect("screen blocker");
        self.app.world_mut().spawn(panel).id()
    }
}

#[derive(Debug, PartialEq)]
struct TooltipLook {
    style:  String,
    border: String,
}

fn default_tooltip(text: impl Into<Text>) -> Tooltip {
    let mut tooltip = Tooltip::new(El::new())
        .with_label_size(fairy_dust::LABEL_SIZE)
        .show_after(Duration::from_secs(5));
    tooltip.text(text);
    tooltip
}

fn style_size(command: &str) -> f32 {
    command
        .split_once("size: ")
        .expect("text size")
        .1
        .split_once(',')
        .expect("size terminator")
        .0
        .parse()
        .expect("numeric text size")
}

#[test]
fn unstyled_text_uses_the_drawn_row_label_size_and_palette() {
    let mut scene = Scene::new(default_tooltip(TIP_TEXT));
    scene.focus();
    let row_size = style_size(&scene.command(scene.host, "Text { text: \"Row\""));
    let text = scene.command(scene.tooltip, "Text { text: \"tooltip look\"");
    let tooltip_size = style_size(&text);
    let ratio = tooltip_size / row_size;
    assert!(
        (0.8..=0.9).contains(&ratio),
        "tooltip size {tooltip_size} / drawn row-label size {row_size} = {ratio}"
    );
    let palette = ControlPalette::system_dark();
    assert!(
        text.contains(&format!("color: {:?}", palette.knob)),
        "{text}"
    );
    let border = scene.border_command();
    assert!(
        border.contains(&format!("color: {:?}", palette.tooltip_border)),
        "tooltip border differs from palette: {border}"
    );
}

#[test]
fn world_tooltip_text_matches_source_label_in_world_units() {
    let mut scene = Scene::new(default_tooltip(TIP_TEXT));
    scene.focus();

    let world_text_height = |entity, prefix| {
        let panel = scene
            .app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("materialized panel");
        let transform = scene
            .app
            .world()
            .get::<GlobalTransform>(entity)
            .expect("panel world transform");
        let font_size = style_size(&scene.command(entity, prefix));
        font_size * panel.points_to_world() * transform.affine().transform_vector3(Vec3::Y).length()
    };
    let source = world_text_height(scene.host, "Text { text: \"Row\"");
    let tooltip = world_text_height(scene.tooltip, "Text { text: \"tooltip look\"");
    let ratio = tooltip / source;
    assert!(
        (0.8..=0.9).contains(&ratio),
        "world tooltip text height {tooltip} / source label height {source} = {ratio}; local {:?}; global {:?}",
        scene.app.world().get::<Transform>(scene.tooltip),
        scene.app.world().get::<GlobalTransform>(scene.tooltip)
    );
}

#[test]
fn world_tooltip_uses_the_cube_label_size_in_meters() {
    const CUBE_TOOLTIP_LABEL_SIZE_METERS: f32 = 0.038;
    let mut tooltip = Tooltip::new(El::column())
        .with_label_size(hana_diegetic::Mm(CUBE_TOOLTIP_LABEL_SIZE_METERS * 1000.0))
        .show_after(Duration::ZERO);
    assert!(
        format!("{tooltip:?}")
            .contains("drawn_label_size: Dimension { value: 38.0, unit: Some(Millimeters) }")
    );
    tooltip.text("Widget Lab");
    let mut scene = Scene::with_host_scale(tooltip, HostPanelScale::MeterSized);
    scene.focus();
    let controller = scene.tooltip;
    let panel = scene
        .app
        .world()
        .get::<DiegeticPanel>(controller)
        .expect("world tooltip panel");
    let transform = scene
        .app
        .world()
        .get::<GlobalTransform>(controller)
        .expect("world tooltip transform");
    let font_size = style_size(&scene.command(controller, "Text { text: \"Widget Lab\""));
    let world_size = font_size
        * panel.points_to_world()
        * transform.affine().transform_vector3(Vec3::Y).length();
    let expected_text_height = CUBE_TOOLTIP_LABEL_SIZE_METERS * 0.85;
    assert!(
        (world_size - expected_text_height).abs() < 0.0001,
        "cube tooltip text height {world_size} m; expected {expected_text_height} m"
    );
}

#[test]
fn unstyled_text_without_a_label_size_uses_the_default_text_style_baseline() {
    let mut tooltip = Tooltip::new(El::new()).show_after(Duration::from_secs(5));
    tooltip.text(TIP_TEXT);
    let mut scene = Scene::new(tooltip);
    scene.focus();

    let text = scene.command(scene.tooltip, "Text { text: \"tooltip look\"");
    let baseline = TextStyle::default().size();
    let tooltip_size = style_size(&text);
    let ratio = tooltip_size / baseline;
    assert!(
        (0.8..=0.9).contains(&ratio),
        "tooltip size {tooltip_size} / default TextStyle size {baseline} = {ratio}"
    );
}

#[test]
fn authored_text_keeps_its_size_color_and_weight() {
    let authored = TextStyle::new(Pt(17.0))
        .with_color(Color::srgb(0.8, 0.3, 0.2))
        .bold();
    let mut scene = Scene::new(default_tooltip(Text::new(TIP_TEXT, authored.clone())));
    scene.focus();
    let text = scene.command(scene.tooltip, "Text { text: \"tooltip look\"");
    assert!(
        (style_size(&text) - authored.size()).abs() < 0.001,
        "{text}"
    );
    assert!(
        text.contains(&format!("color: {:?}", authored.color())),
        "{text}"
    );
    assert!(
        text.contains(&format!("weight: {:?}", authored.weight())),
        "{text}"
    );
}

#[test]
fn authored_root_background_and_padding_keep_default_border_and_radius() {
    let mut tooltip = Tooltip::new(
        El::column()
            .background(Color::srgb(1.0, 0.0, 0.0))
            .padding(Padding::all(Px(20.0))),
    )
    .show_after(Duration::from_secs(5));
    tooltip.text(TIP_TEXT);
    let mut scene = Scene::new(tooltip);
    scene.focus();
    let materialized = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.tooltip)
        .expect("materialized tooltip");
    assert_eq!(
        materialized.tree().element_corner_radius(0),
        CornerRadius::all(Px(6.0))
    );
    let fill = scene.command(scene.tooltip, "Rectangle {");
    assert!(
        fill.contains(&format!("color: {:?}", Color::srgb(1.0, 0.0, 0.0))),
        "{fill}"
    );
    let border = scene.border_command();
    assert!(
        border.contains(&format!(
            "color: {:?}",
            ControlPalette::system_dark().tooltip_border
        )),
        "{border}"
    );

    let mut default_tooltip = Tooltip::new(El::column()).show_after(Duration::from_secs(5));
    default_tooltip.text(TIP_TEXT);
    let mut default_scene = Scene::new(default_tooltip);
    default_scene.focus();
    let added_width = scene.tooltip_width() - default_scene.tooltip_width();
    let expected_width = Dimension::from(Px(32.0)).to_points(1.0);
    assert!(
        (added_width - expected_width).abs() < 0.5,
        "20 px authored padding must replace 4 px defaults on both sides: {added_width} versus {expected_width}"
    );
}

#[test]
fn side_room_reflow_and_restore_preserve_the_default_look() {
    let tooltip = default_tooltip(Text::from("alpha ".repeat(8)).wrap(TextWrap::Words))
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut wide_scene = Scene::new(tooltip.clone());
    wide_scene.focus();
    let before = wide_scene.look();
    let wide = wide_scene.tooltip_width();

    let mut scene = Scene::new(tooltip);
    for y in [245.0, 355.0] {
        scene.screen_panel(Vec2::new(100.0, y), Vec2::new(100.0, 80.0));
    }
    let overlay = scene.screen_panel(Vec2::new(400.0, 300.0), Vec2::new(100.0, 160.0));
    scene.app.update();
    scene.focus();
    let narrow = scene.tooltip_width();
    assert!(
        narrow < wide * 0.8,
        "tooltip did not narrow: {wide} -> {narrow}; visibility {:?}; transform {:?}; panel world width {}",
        scene.app.world().get::<Visibility>(scene.tooltip),
        scene.app.world().get::<GlobalTransform>(scene.tooltip),
        scene
            .app
            .world()
            .get::<DiegeticPanel>(scene.tooltip)
            .expect("tooltip panel")
            .world_width()
    );
    assert_eq!(scene.look(), before, "reflow changed the default look");

    assert!(
        scene
            .app
            .world_mut()
            .get_mut::<DiegeticPanel>(overlay)
            .expect("overlay")
            .set_screen_position(Vec2::new(1000.0, 300.0))
    );
    for _ in 0..30 {
        scene.app.update();
    }
    let restored = scene.tooltip_width();
    assert!(
        restored > narrow * 1.2,
        "tooltip did not restore: {wide} -> {narrow} -> {restored}; visibility {:?}",
        scene.app.world().get::<Visibility>(scene.tooltip)
    );
    assert_eq!(
        scene.look(),
        before,
        "restoring width changed the default look"
    );
}
