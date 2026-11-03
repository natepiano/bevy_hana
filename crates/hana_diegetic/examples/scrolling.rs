//! Vertical scroll views: fitting content, a long list, and a following log.

use bevy::prelude::App;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::PluginGroup;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelSystems;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_kana::ToF32;

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;
const WINDOW_MARGIN: f32 = 80.0;
const FOOTER_HEIGHT: f32 = 20.0;
const FOOTER_GAP: f32 = 20.0;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Scroll views".into(),
                    resolution: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
                    ..default()
                }),
                ..default()
            }),
            DiegeticUiPlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, update_example.after(PanelSystems::ComputeLayout))
        .run();
}

fn list_tree(log_rows: usize) -> LayoutTree {
    let mut builder =
        LayoutBuilder::with_root(El::row().width(Sizing::GROW).height(Sizing::GROW).gap(20.0));
    for (title, id, rows) in [
        ("Fits: wheel does nothing", "short", 5),
        ("Long list: wheel or trackpad", "long", 100),
        ("Log: scroll up to pin", "log", log_rows),
    ] {
        builder.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .gap(12.0),
            |column| {
                column.text((title, TextStyle::new(Pt(12.0)).with_color(Color::WHITE)));
                let view = El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::all(8.0))
                    .gap(4.0)
                    .background(Color::srgb(0.12, 0.13, 0.15))
                    .scroll_line(TextStyle::new(Pt(12.0)));
                let view = if id == "log" {
                    view.scroll_view_from_end(id)
                } else {
                    view.scroll_view(id)
                };
                column.with(view, |body| {
                    for index in 0..rows {
                        body.text((
                            format!(
                                "{} {:03}",
                                if id == "log" { "Event" } else { "Row" },
                                index + 1
                            ),
                            TextStyle::new(Pt(12.0)).with_color(Color::WHITE),
                        ));
                    }
                });
            },
        );
    }
    builder.build()
}

fn stats_tree(compute_ms: f32) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::row().width(Sizing::GROW));
    builder.text((
        format!("Layout compute: {compute_ms:.3} ms    Log appends once per second"),
        TextStyle::new(Pt(12.0)).with_color(Color::srgb(0.6, 0.62, 0.65)),
    ));
    builder.build()
}

#[derive(Resource)]
struct ExamplePanels {
    lists:           Entity,
    stats:           Entity,
    rows:            usize,
    append:          Timer,
    readout:         Timer,
    peak_compute_ms: f32,
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let width = 2.0_f32.mul_add(-WINDOW_MARGIN, WINDOW_WIDTH.to_f32());
    let bottom = WINDOW_HEIGHT.to_f32() - WINDOW_MARGIN;
    let lists = DiegeticPanel::screen()
        .size(
            Px(width),
            Px(bottom - WINDOW_MARGIN - FOOTER_HEIGHT - FOOTER_GAP),
        )
        .anchor(Anchor::TopLeft)
        .screen_position(WINDOW_MARGIN, WINDOW_MARGIN)
        .with_tree(list_tree(30))
        .build();
    let stats = DiegeticPanel::screen()
        .size(Px(width), Px(FOOTER_HEIGHT))
        .anchor(Anchor::TopLeft)
        .screen_position(WINDOW_MARGIN, bottom - FOOTER_HEIGHT)
        .with_tree(stats_tree(0.0))
        .build();
    let (Ok(lists), Ok(stats)) = (lists, stats) else {
        error!("could not build scrolling example panels");
        return;
    };
    let lists = commands.spawn(lists).id();
    let stats = commands.spawn(stats).id();
    commands.insert_resource(ExamplePanels {
        lists,
        stats,
        rows: 30,
        append: Timer::from_seconds(1.0, TimerMode::Repeating),
        readout: Timer::from_seconds(0.25, TimerMode::Repeating),
        peak_compute_ms: 0.0,
    });
}

fn update_example(
    time: Res<Time>,
    perf: Res<DiegeticPerfStats>,
    mut panels: ResMut<ExamplePanels>,
    mut commands: Commands,
) {
    panels.peak_compute_ms = panels.peak_compute_ms.max(perf.compute_ms);
    if panels.append.tick(time.delta()).just_finished() {
        panels.rows += 1;
        if let Err(error) = commands.set_tree(panels.lists, list_tree(panels.rows)) {
            error!("could not append log row: {error}");
        }
    }
    if panels.readout.tick(time.delta()).just_finished() {
        if let Err(error) = commands.set_tree(panels.stats, stats_tree(panels.peak_compute_ms)) {
            error!("could not update compute readout: {error}");
        }
        panels.peak_compute_ms = 0.0;
    }
}
