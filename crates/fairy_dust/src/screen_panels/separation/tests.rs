use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::InheritedVisibility;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::CoordinateSpace;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::HeadlessLayoutPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::PanelAnchorGeometryParam;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelAttachmentAuthored;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelScreenBounds;
use hana_diegetic::Percent;
use hana_diegetic::Px;
use hana_diegetic::ScreenPosition;
use hana_diegetic::Sizing;
use hana_diegetic::TextDimensions;
use hana_diegetic::WidgetOf;
use hana_lagrange::FreeCam;

use super::AttachedPanel;
use super::AttachedPanelFootprint;
use super::AttachedPanelOwner;
use super::AttachmentPlacement;
use super::EdgePanel;
use super::EdgePanelHomePlacement;
use super::EdgePanelPlacementRecord;
use super::EdgePanelPlacementState;
use super::EdgePanelPlacementStatus;
use super::EdgePanelPrecedence;
use super::EdgePanelWidth;
use super::PanelFootprint;
use super::PanelPlacement;
use super::RankedEdgePanel;
use super::SCREEN_PANEL_GAP;
use super::SCREEN_PANEL_WINDOW_INSET;
use super::attached_panel_obstacles;
use super::framed_world_panel_footprints;
use super::separate_edge_panels;
use super::separate_edge_panels_with_widths;
use crate::camera_home::CameraHomeTarget;
use crate::orbit_cam::FairyDustOrbitCam;
use crate::panel_insets::CoveringPanel;
use crate::panel_insets::{self};
use crate::screen_panels;
use crate::screen_panels::DescriptionPanel;
use crate::screen_panels::constants::GPU_METER_PANEL_WIDTH_FRACTION;
use crate::screen_panels::performance::GPU_METER_PANEL_MIN_WIDTH;

#[test]
fn auxiliary_panels_appear_monotonically_with_window_size_and_ignore_camera_pose() {
    let mut world = World::new();
    let [title, camera, help] = std::array::from_fn(|_| world.spawn_empty().id());
    let panels = [
        EdgePanel {
            entity:       title,
            anchor:       Anchor::TopLeft,
            size:         Vec2::new(250.0, 50.0),
            precedence:   EdgePanelPrecedence::TitleBar,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       camera,
            anchor:       Anchor::BottomRight,
            size:         Vec2::new(250.0, 180.0),
            precedence:   EdgePanelPrecedence::CameraControl,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       help,
            anchor:       Anchor::BottomLeft,
            size:         Vec2::new(300.0, 220.0),
            precedence:   EdgePanelPrecedence::Description,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    let physical_sizes = [
        Vec2::new(400.0, 500.0),
        Vec2::new(900.0, 800.0),
        Vec2::new(1400.0, 1200.0),
        Vec2::new(1500.0, 2012.0),
        Vec2::new(2000.0, 1800.0),
        Vec2::new(2538.0, 2012.0),
    ];
    let mut previous = [false; 2];
    for physical_size in physical_sizes {
        let size = physical_size / 2.0;
        let base = separate_edge_panels(size, &panels, &[], &[]);
        let shifted_face = [PanelFootprint {
            top_left: size * 0.3,
            size:     size * 0.4,
        }];
        let after_camera_move = separate_edge_panels(size, &panels, &[], &shifted_face);
        assert_eq!(base, after_camera_move, "{size:?} changed with camera pose");
        assert_eq!(base[0].1, PanelPlacement::AtAnchor);
        for (index, was_shown) in previous.iter_mut().enumerate() {
            let shown = base[index + 1].1 != PanelPlacement::Hidden;
            assert!(
                !*was_shown || shown,
                "{size:?} hid a previously shown panel"
            );
            *was_shown = shown;
        }
        if physical_size.x <= 1500.0 {
            assert_eq!(previous, [false, false]);
        }
        if physical_size.x >= 2000.0 {
            assert_eq!(previous, [true, true]);
        }
    }
}

#[test]
fn auxiliary_visibility_ignores_moving_world_face_but_yields_to_host_popup() {
    let mut world = World::new();
    let [host, title, camera, keys] = std::array::from_fn(|_| world.spawn_empty().id());
    let panels = [
        EdgePanel {
            entity:       host,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(180.0, 90.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       title,
            anchor:       Anchor::TopLeft,
            size:         Vec2::new(360.0, 54.0),
            precedence:   EdgePanelPrecedence::TitleBar,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       camera,
            anchor:       Anchor::BottomRight,
            size:         Vec2::new(250.0, 180.0),
            precedence:   EdgePanelPrecedence::CameraControl,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       keys,
            anchor:       Anchor::BottomLeft,
            size:         Vec2::new(300.0, 220.0),
            precedence:   EdgePanelPrecedence::Description,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    let sizes = [
        Vec2::new(400.0, 500.0),
        Vec2::new(900.0, 800.0),
        Vec2::new(1000.0, 900.0),
        Vec2::new(1500.0, 2012.0),
        Vec2::new(2538.0, 2012.0),
    ];
    let mut previous = [false; 2];
    for size in sizes {
        let host_at = super::inset_anchor_top_left(size, panels[0].size, panels[0].anchor);
        let attachments = [AttachedPanelFootprint {
            bounds:    PanelFootprint {
                top_left: host_at + Vec2::new(-100.0, 95.0),
                size:     Vec2::new(220.0, 730.0),
            },
            placement: AttachmentPlacement::Carried {
                owner:  host,
                offset: Vec2::new(-100.0, 95.0),
            },
        }];
        let poses = [
            Vec2::new(0.0, 0.0),
            Vec2::new(size.x * 0.25, size.y * 0.35),
            Vec2::new(size.x * 0.6, size.y * 0.65),
        ];
        let mut expected = [false; 2];
        for (index, top_left) in poses.into_iter().enumerate() {
            let face = [PanelFootprint {
                top_left,
                size: size * 0.3,
            }];
            let placements = separate_edge_panels(size, &panels, &attachments, &face);
            if size == Vec2::new(1000.0, 900.0) && index == 0 {
                assert_eq!(
                    placements[0].1,
                    PanelPlacement::AtAnchor,
                    "the host and its tall attachment must obstruct the camera corner"
                );
            }
            let visible = [
                placements[2].1 != PanelPlacement::Hidden,
                placements[3].1 != PanelPlacement::Hidden,
            ];
            if index > 0 {
                if size == Vec2::new(1000.0, 900.0) {
                    assert_eq!(visible[1], expected[1], "keys changed at pose {top_left:?}");
                } else {
                    assert_eq!(visible, expected, "{size:?} changed at pose {top_left:?}");
                }
            } else {
                expected = visible;
            }
        }
        assert_eq!(
            expected[0],
            size.x >= 1000.0 && size.y >= 900.0 && size != Vec2::new(1000.0, 900.0)
        );
        assert_eq!(expected[1], size.x >= 1000.0 && size.y >= 900.0);
        for (was_shown, shown) in previous.iter_mut().zip(expected) {
            assert!(!*was_shown || shown, "{size:?} hid an auxiliary panel");
            *was_shown = shown;
        }
    }
}

#[test]
fn real_help_box_hides_when_its_natural_width_exceeds_450() -> Result<(), String> {
    let (mut app, _) = placement_app(450, 400);
    let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
    app.insert_resource(fonts)
        .insert_resource(DiegeticTextMeasurer {
            measure_fn: Arc::new(|text, measure| TextDimensions {
                width:       f32::from(u16::try_from(text.len()).unwrap_or(u16::MAX))
                    * measure.size
                    * 0.8,
                height:      measure.size,
                line_height: measure.size,
            }),
        })
        .insert_resource(Assets::<StandardMaterial>::default())
        .add_plugins(HeadlessLayoutPlugin);

    let title = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    app.world_mut()
        .entity_mut(title)
        .insert(EdgePanelPrecedence::TitleBar);
    let help = DescriptionPanel::new("Widget Lab")
        .with_fit_width()
        .line("Enter or double-click the field: edit it. Esc: stop");
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  fonts: Res<FontRegistry>,
                  mut materials: ResMut<Assets<StandardMaterial>>| {
                screen_panels::description::spawn_description_panel(
                    &mut commands,
                    &help,
                    &fonts,
                    &mut materials,
                );
            },
        )
        .map_err(|error| error.to_string())?;
    let mut query = app.world_mut().query::<(Entity, &EdgePanelPrecedence)>();
    let description = query
        .iter(app.world())
        .find_map(|(entity, precedence)| {
            (*precedence == EdgePanelPrecedence::Description).then_some(entity)
        })
        .ok_or("description panel missing")?;
    app.world_mut()
        .entity_mut(description)
        .insert(InheritedVisibility::VISIBLE);
    app.update();
    app.update();

    let panel = app
        .world()
        .get::<DiegeticPanel>(description)
        .ok_or("description panel missing")?;
    let computed = app
        .world()
        .get::<ComputedDiegeticPanel>(description)
        .ok_or("description layout missing")?;
    assert!(panel.width() <= 450.0, "surface should be clamped");
    assert!(
        computed.content_width() > 450.0,
        "help text must exceed the window's natural width"
    );
    assert_eq!(placement(&app, title), Some(PanelPlacement::AtAnchor));
    assert_eq!(placement(&app, description), Some(PanelPlacement::Hidden));
    assert_eq!(
        app.world().get::<Visibility>(description),
        Some(&Visibility::Hidden)
    );
    Ok(())
}

fn widgets_panels() -> [EdgePanel; 4] {
    let mut world = World::new();
    let entities: [Entity; 4] = std::array::from_fn(|_| world.spawn_empty().id());
    [
        EdgePanel {
            entity:       entities[0],
            anchor:       Anchor::TopRight,
            size:         Vec2::new(180.0, 90.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       entities[1],
            anchor:       Anchor::BottomRight,
            size:         Vec2::new(300.0, 463.0),
            precedence:   EdgePanelPrecedence::CameraControl,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       entities[2],
            anchor:       Anchor::TopLeft,
            size:         Vec2::new(375.0, 40.0),
            precedence:   EdgePanelPrecedence::TitleBar,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       entities[3],
            anchor:       Anchor::BottomLeft,
            size:         Vec2::new(488.0, 262.0),
            precedence:   EdgePanelPrecedence::Description,
            width_policy: EdgePanelWidth::Fixed,
        },
    ]
}

fn anchored_top_left(window: Vec2, panel: &EdgePanel) -> Vec2 {
    super::inset_anchor_top_left(window, panel.size, panel.anchor)
}

fn visible_rectangles(
    window: Vec2,
    panels: &[EdgePanel],
    obstacles: &[PanelScreenBounds],
) -> Vec<(Vec2, Vec2)> {
    let obstacles: Vec<_> = obstacles
        .iter()
        .copied()
        .map(|bounds| AttachedPanelFootprint {
            bounds:    bounds.into(),
            placement: AttachmentPlacement::Independent,
        })
        .collect();
    let placements = separate_edge_panels(window, panels, &obstacles, &[]);
    assert_eq!(placements.len(), panels.len());
    panels
        .iter()
        .zip(placements)
        .filter_map(|(panel, (entity, placement))| {
            assert_eq!(entity, panel.entity);
            let top_left = match placement {
                PanelPlacement::AtAnchor => anchored_top_left(window, panel),
                PanelPlacement::Moved { top_left } => top_left,
                PanelPlacement::Hidden => return None,
            };
            Some((top_left, panel.size))
        })
        .collect()
}

fn attachment_card_bounds(window: Vec2) -> Result<PanelScreenBounds, String> {
    // widgets.rs attaches the card's TopRight to the floating panel button's
    // BottomRight with a 24 px downward offset. In the measured example, that
    // resolves to a 216 x 42 card, 62 px from the right edge and 89 px down.
    PanelScreenBounds::new(Vec2::new(window.x - 277.0, 89.0), Vec2::new(216.0, 42.0))
        .map_err(|error| error.to_string())
}

fn carried_widgets_card(window: Vec2, owner: Entity) -> Result<AttachedPanelFootprint, String> {
    Ok(AttachedPanelFootprint {
        bounds:    attachment_card_bounds(window)?.into(),
        placement: AttachmentPlacement::Carried {
            owner,
            offset: Vec2::new(-97.0, 89.0),
        },
    })
}

fn intersects(top_left: Vec2, size: Vec2, obstacle: PanelScreenBounds) -> bool {
    let other = obstacle.top_left();
    let other_size = obstacle.size();
    top_left.x < other.x + other_size.x
        && other.x < top_left.x + size.x
        && top_left.y < other.y + other_size.y
        && other.y < top_left.y + size.y
}

#[test]
fn title_bar_clears_attached_card_at_450_by_400() -> Result<(), String> {
    let mut world = World::new();
    let floating = world.spawn_empty().id();
    let title = world.spawn_empty().id();
    let window = Vec2::new(450.0, 400.0);
    let panels = [
        EdgePanel {
            entity:       floating,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(151.0, 79.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       title,
            anchor:       Anchor::TopLeft,
            size:         Vec2::new(314.0, 54.0),
            precedence:   EdgePanelPrecedence::TitleBar,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    let obstacle = attachment_card_bounds(window)?;
    let rectangles = visible_rectangles(window, &panels, &[obstacle]);
    assert!(
        rectangles
            .iter()
            .all(|&(top_left, size)| !intersects(top_left, size, obstacle))
    );
    Ok(())
}

#[test]
fn visible_panels_fit_at_every_requested_size_and_clear_at_full_screen() -> Result<(), String> {
    let panels = widgets_panels();
    let swept = (300_u16..=2600)
        .step_by(50)
        .flat_map(|width| [500.0, 1000.0].map(move |height| Vec2::new(f32::from(width), height)));
    let full_screen = [Vec2::new(1728.0, 1117.0), Vec2::new(1728.0, 1079.0)];
    for (window, require_clearance) in swept
        .map(|window| (window, false))
        .chain(full_screen.map(|window| (window, true)))
    {
        let obstacle = attachment_card_bounds(window)?;
        let rectangles = visible_rectangles(window, &panels, &[obstacle]);
        for &(top_left, size) in &rectangles {
            let is_title = (top_left - anchored_top_left(window, &panels[2])).length() < 0.001
                && (size - panels[2].size).length() < 0.001;
            let min_inset = if is_title {
                0.0
            } else {
                SCREEN_PANEL_WINDOW_INSET
            };
            assert!(
                top_left.x >= min_inset - 0.001
                    && top_left.y >= min_inset - 0.001
                    && (is_title
                        || (top_left.x + size.x <= window.x - min_inset + 0.001
                            && top_left.y + size.y <= window.y - min_inset + 0.001)),
                "visible rectangle {top_left:?} {size:?} is outside {window:?}"
            );
            assert!(
                !intersects(top_left, size, obstacle),
                "visible rectangle {top_left:?} {size:?} intersects attachment card at {window:?}"
            );
        }
        if require_clearance {
            for (index, &(first, first_size)) in rectangles.iter().enumerate() {
                for &(second, second_size) in &rectangles[index + 1..] {
                    assert!(
                        first.x + first_size.x + SCREEN_PANEL_GAP <= second.x
                            || second.x + second_size.x + SCREEN_PANEL_GAP <= first.x
                            || first.y + first_size.y + SCREEN_PANEL_GAP <= second.y
                            || second.y + second_size.y + SCREEN_PANEL_GAP <= first.y,
                        "visible rectangles overlap or lack the gap at {window:?}: {first:?} {first_size:?}, {second:?} {second_size:?}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn camera_with_no_clear_row_never_covers_placed_stats() {
    let mut world = World::new();
    let stats = world.spawn_empty().id();
    let camera = world.spawn_empty().id();
    let panels = [
        EdgePanel {
            entity:       stats,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(302.0, 903.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       camera,
            anchor:       Anchor::BottomRight,
            size:         Vec2::new(300.0, 360.0),
            precedence:   EdgePanelPrecedence::CameraControl,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    for height in [1117.0, 1079.0] {
        let window = Vec2::new(1728.0, height);
        let placements = separate_edge_panels(window, &panels, &[], &[]);
        assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
        assert_eq!(placements[1].1, PanelPlacement::Hidden, "{window:?}");
    }
}

#[test]
fn pipeline_shrinks_beside_stats_and_regrows_at_full_screen() {
    let mut world = World::new();
    let stats = world.spawn_empty().id();
    let pipeline = world.spawn_empty().id();

    for (index, window) in [Vec2::new(1600.0, 1000.0), Vec2::new(1728.0, 1117.0)]
        .into_iter()
        .enumerate()
    {
        let authored_width = window.x * GPU_METER_PANEL_WIDTH_FRACTION;
        let panels = [
            EdgePanel {
                entity:       stats,
                anchor:       Anchor::TopRight,
                size:         Vec2::new(302.0, 903.0),
                precedence:   EdgePanelPrecedence::HostPanel,
                width_policy: EdgePanelWidth::Fixed,
            },
            EdgePanel {
                entity:       pipeline,
                anchor:       Anchor::BottomLeft,
                size:         Vec2::new(authored_width, 100.0),
                precedence:   EdgePanelPrecedence::HostPanel,
                width_policy: EdgePanelWidth::Resizable {
                    minimum: GPU_METER_PANEL_MIN_WIDTH,
                },
            },
        ];
        let placements = separate_edge_panels_with_widths(window, &panels, &[], &[]);
        assert_ne!(
            placements[1].placement,
            PanelPlacement::Hidden,
            "{window:?}"
        );
        let stats_left = anchored_top_left(window, &panels[0]).x;
        let pipeline_left = anchored_top_left(window, &panels[1]).x;
        assert!(
            pipeline_left + placements[1].width + SCREEN_PANEL_GAP <= stats_left + 0.001,
            "{window:?}: pipeline ends at {}, stats begins at {stats_left}",
            pipeline_left + placements[1].width
        );
        if index == 0 {
            assert!(placements[1].width < authored_width);
        } else {
            assert!((placements[1].width - authored_width).abs() < 0.001);
        }
    }
}

fn spawn_visible_panel(app: &mut App, panel: DiegeticPanel) -> Entity {
    app.world_mut()
        .spawn((
            panel,
            Transform::default(),
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
        ))
        .id()
}

fn position_screen_panels_for_test(
    window: Query<&Window, With<PrimaryWindow>>,
    mut panels: Query<(&mut Transform, &DiegeticPanel)>,
) {
    let Ok(window) = window.single() else { return };
    for (mut transform, panel) in &mut panels {
        if let CoordinateSpace::Screen {
            position: ScreenPosition::At(position),
            ..
        } = panel.coordinate_space()
        {
            transform.translation.x = position.x - window.width() / 2.0;
            transform.translation.y = window.height() / 2.0 - position.y;
        }
    }
}

fn panel_scene(width: u16, height: u16) -> Result<(App, Entity, [Entity; 2]), String> {
    let (mut app, window_entity) = placement_app(u32::from(width), u32::from(height));
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);

    let mut batch_panel = DiegeticPanel::screen()
        .size(Px(302.0), Px(903.0))
        .anchor(Anchor::TopRight)
        .build()
        .map_err(|error| error.to_string())?;
    // HeadlessLayoutPlugin omits the screen dimension resolver; seed its numeric results.
    batch_panel
        .set_size((Px(302.0), Px(903.0)))
        .map_err(|error| error.to_string())?;
    let batch = spawn_visible_panel(&mut app, batch_panel);

    let mut pipeline_panel = DiegeticPanel::screen()
        .size(Percent(GPU_METER_PANEL_WIDTH_FRACTION), Px(100.0))
        .anchor(Anchor::BottomLeft)
        .with_tree(
            LayoutBuilder::with_root(El::new().width(Sizing::GROW).height(Sizing::FIT)).build(),
        )
        .build()
        .map_err(|error| error.to_string())?;
    pipeline_panel
        .set_size((
            Px(f32::from(width) * GPU_METER_PANEL_WIDTH_FRACTION),
            Px(100.0),
        ))
        .map_err(|error| error.to_string())?;
    let pipeline = spawn_visible_panel(&mut app, pipeline_panel);
    app.world_mut()
        .entity_mut(pipeline)
        .insert(EdgePanelWidth::Resizable {
            minimum: GPU_METER_PANEL_MIN_WIDTH,
        });

    Ok((app, window_entity, [batch, pipeline]))
}

fn resize_panel_scene(width: u16, height: u16) -> Result<(App, Entity, [Entity; 2]), String> {
    let (mut app, window_entity, [batch, pipeline]) = panel_scene(width, height)?;
    assert_eq!(
        app.world().resource::<EdgePanelPlacementStatus>().state,
        EdgePanelPlacementState::Pending
    );
    app.add_systems(
        Update,
        position_screen_panels_for_test.after(super::place_edge_panels),
    );
    assert!(
        app.world_mut()
            .get_mut::<DiegeticPanel>(pipeline)
            .ok_or("pipeline missing before layout")?
            .set_screen_position(Vec2::new(
                SCREEN_PANEL_WINDOW_INSET,
                f32::from(height) - SCREEN_PANEL_WINDOW_INSET,
            ))
    );
    for _ in 0..16 {
        app.update();
    }
    Ok((app, window_entity, [batch, pipeline]))
}

#[test]
fn unseeded_status_attachment_does_not_hide_or_widen_pipeline() -> Result<(), String> {
    for (width, height) in [(1600_u16, 1000_u16), (1728, 1117), (1728, 1079)] {
        let (mut app, _, [_batch, pipeline]) = panel_scene(width, height)?;
        let mut status_panel = DiegeticPanel::screen()
            .size(Px(400.0), Px(100.0))
            .anchor(Anchor::TopLeft)
            .screen_position(0.0, 0.0)
            .build()
            .map_err(|error| error.to_string())?;
        status_panel
            .set_size((Px(400.0), Px(100.0)))
            .map_err(|error| error.to_string())?;
        let status = spawn_visible_panel(&mut app, status_panel);
        let (source, destination) = app
            .world_mut()
            .run_system_once(move |reader: PanelEntityReader| {
                (reader.screen(status), reader.screen(pipeline))
            })
            .map_err(|error| error.to_string())?;
        let source = source.ok_or("status handle missing")?;
        let destination = destination.ok_or("pipeline handle missing")?;
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.attach_to_panel(
                    source,
                    destination,
                    PanelAttachment::new(Anchor::BottomLeft, Anchor::TopLeft)
                        .with_offset(PanelAnchorOffset::new(Px(0.0), Px(-SCREEN_PANEL_GAP))),
                );
            })
            .map_err(|error| error.to_string())?;
        for frame in 1..=16 {
            app.update();
            assert_ne!(
                placement(&app, pipeline),
                Some(PanelPlacement::Hidden),
                "{width}x{height} frame {frame}: pipeline hidden"
            );
            assert_ne!(
                app.world().get::<Visibility>(pipeline),
                Some(&Visibility::Hidden),
                "{width}x{height} frame {frame}: pipeline visibility hidden"
            );
        }
        assert_eq!(
            app.world()
                .get::<PanelAttachmentAuthored>(status)
                .map(PanelAttachmentAuthored::target),
            Some(pipeline),
            "{width}x{height}: status did not attach before startup solve"
        );
        if width == 1728 {
            assert_status_card_whole(&mut app, status, pipeline, width, height)?;
        }
        let panel = app
            .world()
            .get::<DiegeticPanel>(pipeline)
            .ok_or("pipeline missing after startup solve")?;
        let width_fraction = f32::from(width) * GPU_METER_PANEL_WIDTH_FRACTION;
        let expected_width = if width == 1600 {
            (-2.0_f32).mul_add(
                SCREEN_PANEL_WINDOW_INSET,
                f32::from(width) - 302.0 - SCREEN_PANEL_GAP,
            )
        } else {
            width_fraction
        };
        assert!(
            (panel.width() - expected_width).abs() < 0.01,
            "{width}x{height}: pipeline width {} vs expected {expected_width}",
            panel.width()
        );
    }
    Ok(())
}

fn assert_status_card_whole(
    app: &mut App,
    status: Entity,
    pipeline: Entity,
    width: u16,
    height: u16,
) -> Result<(), String> {
    assert_ne!(
        app.world().get::<Visibility>(status),
        Some(&Visibility::Hidden),
        "{width}x{height}: status card hidden"
    );
    let pipeline_bounds = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            geometry
                .get(pipeline)
                .ok()
                .and_then(|geometry| geometry.points().screen_bounds())
        })
        .map_err(|error| error.to_string())?
        .ok_or("pipeline bounds unresolved")?;
    // HeadlessLayoutPlugin omits the screen attachment positioner.
    let status_top_left = pipeline_bounds.top_left() - Vec2::new(0.0, 100.0 + SCREEN_PANEL_GAP);
    assert!(
        app.world_mut()
            .get_mut::<DiegeticPanel>(status)
            .ok_or("status card missing")?
            .set_screen_position(status_top_left)
    );
    let bounds = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            geometry
                .get(status)
                .ok()
                .and_then(|geometry| geometry.points().screen_bounds())
        })
        .map_err(|error| error.to_string())?
        .ok_or("status card bounds unresolved")?;
    assert!(
        PanelFootprint::from(bounds).fits(Vec2::new(f32::from(width), f32::from(height))),
        "{width}x{height}: status card outside window: {bounds:?}"
    );
    Ok(())
}

fn obstacle_footprints_for_test(
    app: &mut App,
    window: Entity,
    owner: Entity,
    managed_card: Entity,
    independent_popup: Entity,
) -> Result<(Vec<AttachedPanelFootprint>, bool), String> {
    app.world_mut()
        .run_system_once(move |widget_owners: Query<&WidgetOf>, geometry: PanelAnchorGeometryParam| {
            let by_window = HashMap::from([(
                window,
                vec![RankedEdgePanel {
                    edge: EdgePanel {
                        entity: owner,
                        anchor: Anchor::BottomLeft,
                        size: Vec2::new(1280.0, 100.0),
                        precedence: EdgePanelPrecedence::HostPanel,
                        width_policy: EdgePanelWidth::Fixed,
                    },
                    precedence: 1,
                    spawn_order: 0,
                }],
            )]);
            let window_sizes = HashMap::from([(window, Vec2::new(1600.0, 1000.0))]);
            let mut attachments = [
                AttachedPanel {
                    entity: managed_card,
                    target: owner,
                    owner: AttachedPanelOwner::Independent,
                },
                AttachedPanel {
                    entity: independent_popup,
                    target: independent_popup,
                    owner: AttachedPanelOwner::Independent,
                },
            ];
            let mut obstacles = attached_panel_obstacles(
                &mut attachments,
                &by_window,
                &window_sizes,
                &widget_owners,
                &geometry,
            );
            (
                obstacles.remove(&window).unwrap_or_default(),
                matches!(attachments[0].owner, AttachedPanelOwner::EdgePanel(entity) if entity == owner),
            )
        })
        .map_err(|error| error.to_string())
}

#[test]
fn unresolved_managed_card_is_deferred_but_independent_popup_remains() -> Result<(), String> {
    let (mut app, window, [_batch, pipeline]) = resize_panel_scene(1600, 1000)?;
    let mut card = DiegeticPanel::screen()
        .size(Px(400.0), Px(100.0))
        .anchor(Anchor::TopLeft)
        .screen_position(0.0, 0.0)
        .build()
        .map_err(|error| error.to_string())?;
    card.set_size((Px(400.0), Px(100.0)))
        .map_err(|error| error.to_string())?;
    let card = spawn_visible_panel(&mut app, card);
    let mut popup = DiegeticPanel::screen()
        .size(Px(100.0), Px(80.0))
        .anchor(Anchor::TopLeft)
        .screen_position(300.0, 300.0)
        .build()
        .map_err(|error| error.to_string())?;
    popup
        .set_size((Px(100.0), Px(80.0)))
        .map_err(|error| error.to_string())?;
    let popup = spawn_visible_panel(&mut app, popup);

    for invalid_owner in [false, true] {
        if invalid_owner {
            assert!(
                app.world_mut()
                    .get_mut::<DiegeticPanel>(card)
                    .ok_or("managed card missing")?
                    .set_screen_position(Vec2::new(50.0, 50.0))
            );
            assert!(
                app.world_mut()
                    .get_mut::<DiegeticPanel>(pipeline)
                    .ok_or("pipeline missing")?
                    .set_screen_position(Vec2::ZERO)
            );
        }
        let (obstacles, owner_recognized) =
            obstacle_footprints_for_test(&mut app, window, pipeline, card, popup)?;
        assert!(
            owner_recognized,
            "managed owner lost when footprint is unresolved"
        );
        assert_eq!(
            obstacles.len(),
            1,
            "unresolved managed card became an obstacle"
        );
        assert!(matches!(
            obstacles[0].placement,
            AttachmentPlacement::Independent
        ));
        assert_eq!(obstacles[0].bounds.top_left, Vec2::new(300.0, 300.0));
        assert_eq!(obstacles[0].bounds.size, Vec2::new(100.0, 80.0));
    }
    Ok(())
}

#[test]
fn pipeline_width_recovers_after_window_regrowth() -> Result<(), String> {
    let (mut app, window, [batch, pipeline]) = resize_panel_scene(1600, 1000)?;
    let narrow_width = app
        .world()
        .get::<DiegeticPanel>(pipeline)
        .ok_or("pipeline missing")?
        .width();
    let narrow_bounds = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            [batch, pipeline].map(|entity| {
                geometry
                    .get(entity)
                    .ok()
                    .and_then(|geometry| geometry.points().screen_bounds())
            })
        })
        .map_err(|error| error.to_string())?;
    assert!(
        narrow_width < 1600.0 * GPU_METER_PANEL_WIDTH_FRACTION,
        "pipeline did not narrow: width={narrow_width}, placement={:?}, batch={:?}, bounds={narrow_bounds:?}",
        placement(&app, pipeline),
        placement(&app, batch),
    );
    assert_eq!(
        app.world().resource::<EdgePanelPlacementStatus>().state,
        EdgePanelPlacementState::Placed
    );
    let narrow_revision = app.world().resource::<EdgePanelPlacementStatus>().revision;

    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(1728.0, 1117.0);
    app.world_mut()
        .run_system_once(super::place_edge_panels)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        app.world().resource::<EdgePanelPlacementStatus>().state,
        EdgePanelPlacementState::Pending
    );
    for _ in 0..16 {
        app.update();
    }
    let wide_width = app
        .world()
        .get::<DiegeticPanel>(pipeline)
        .ok_or("pipeline missing after resize")?
        .width();
    assert!(
        1728.0_f32
            .mul_add(-GPU_METER_PANEL_WIDTH_FRACTION, wide_width)
            .abs()
            < 0.001,
        "pipeline did not regrow: {wide_width}"
    );
    assert_eq!(
        app.world().resource::<EdgePanelPlacementStatus>().state,
        EdgePanelPlacementState::Placed
    );
    let status = app.world().resource::<EdgePanelPlacementStatus>();
    assert!(
        status.revision > narrow_revision,
        "width change did not advance placement revision"
    );
    Ok(())
}

fn measured_instrumentation_fit_sizes() -> Result<(Vec2, Vec2), String> {
    let mut app = crate::test_app();
    app.add_plugins(MinimalPlugins)
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessLayoutPlugin);
    let mut materials = Assets::<StandardMaterial>::default();
    let mut meter_tree =
        LayoutBuilder::with_root(El::new().width(Sizing::GROW).height(Sizing::FIT));
    screen_panels::screen_panel_frame(
        &mut meter_tree,
        Sizing::GROW,
        Sizing::FIT,
        crate::DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(4.0),
                |builder| {
                    for _ in 0..3 {
                        builder.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(21.0)),
                            |_| {},
                        );
                    }
                },
            );
        },
    );
    let mut meter = screen_panels::gpu_meter_panel(meter_tree.build(), &mut materials)
        .map_err(|error| error.to_string())?;
    meter.set_width(1728.0 * GPU_METER_PANEL_WIDTH_FRACTION);
    let meter = app.world_mut().spawn(meter).id();

    let mut status_tree =
        LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panels::screen_panel_frame(
        &mut status_tree,
        Sizing::FIT,
        Sizing::FIT,
        crate::DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column().width(Sizing::FIT).height(Sizing::FIT).gap(2.0),
                |builder| {
                    for _ in 0..24 {
                        builder.with(
                            El::new()
                                .width(Sizing::fixed(300.0))
                                .height(Sizing::fixed(16.0)),
                            |_| {},
                        );
                    }
                },
            );
        },
    );
    let status = screen_panels::fps_stats_panel(status_tree.build(), &mut materials)
        .map_err(|error| error.to_string())?;
    let status = app.world_mut().spawn(status).id();
    app.update();
    let size = |entity| -> Result<Vec2, String> {
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .ok_or("Fit layout missing")?;
        Ok(Vec2::new(
            computed.content_width(),
            computed.content_height(),
        ))
    };
    Ok((size(meter)?, size(status)?))
}

#[test]
fn stress_panel_group_is_whole_and_clears_independent_panels_at_full_screen() -> Result<(), String>
{
    let (meter_size, status_size) = measured_instrumentation_fit_sizes()?;
    let mut world = World::new();
    let batch = world.spawn_empty().id();
    let camera = world.spawn_empty().id();
    let pipeline = world.spawn_empty().id();
    let mut short_card_gaps = Vec::new();

    for height in [1117.0, 1079.0] {
        let window = Vec2::new(1728.0, height);
        let panels = [
            EdgePanel {
                entity:       batch,
                anchor:       Anchor::TopRight,
                size:         Vec2::new(302.0, 903.0),
                precedence:   EdgePanelPrecedence::HostPanel,
                width_policy: EdgePanelWidth::Fixed,
            },
            EdgePanel {
                entity:       camera,
                anchor:       Anchor::BottomRight,
                size:         Vec2::new(316.0, 360.0),
                precedence:   EdgePanelPrecedence::CameraControl,
                width_policy: EdgePanelWidth::Fixed,
            },
            EdgePanel {
                entity:       pipeline,
                anchor:       Anchor::BottomLeft,
                size:         Vec2::new(window.x * GPU_METER_PANEL_WIDTH_FRACTION, meter_size.y),
                precedence:   EdgePanelPrecedence::HostPanel,
                width_policy: EdgePanelWidth::Resizable {
                    minimum: GPU_METER_PANEL_MIN_WIDTH,
                },
            },
        ];
        let status = AttachedPanelFootprint {
            bounds:    PanelFootprint {
                top_left: Vec2::ZERO,
                size:     status_size,
            },
            placement: AttachmentPlacement::Carried {
                owner:  pipeline,
                offset: Vec2::new(0.0, -status_size.y - SCREEN_PANEL_GAP),
            },
        };
        let resolved = separate_edge_panels_with_widths(window, &panels, &[status], &[]);
        assert_eq!(resolved[0].placement, PanelPlacement::AtAnchor, "{height}");
        assert_eq!(resolved[1].placement, PanelPlacement::Hidden, "{height}");
        let pipeline_top_left = match resolved[2].placement {
            PanelPlacement::AtAnchor => anchored_top_left(window, &panels[2]),
            PanelPlacement::Moved { top_left } => top_left,
            PanelPlacement::Hidden => return Err(format!("pipeline hidden at {height}")),
        };
        let batch_rect = PanelFootprint {
            top_left: anchored_top_left(window, &panels[0]),
            size:     panels[0].size,
        };
        let pipeline_rect = PanelFootprint {
            top_left: pipeline_top_left,
            size:     Vec2::new(resolved[2].width, panels[2].size.y),
        };
        let status_rect = status
            .at(pipeline, pipeline_rect)
            .ok_or("carried status did not follow pipeline")?;
        let card_gap = pipeline_rect.top_left.y - status_rect.top_left.y - status_rect.size.y;
        assert!((card_gap - SCREEN_PANEL_GAP).abs() < 0.001, "{height}");
        if card_gap + 0.001 < SCREEN_PANEL_GAP {
            short_card_gaps.push((height, card_gap));
        }
        for rect in [batch_rect, pipeline_rect, status_rect] {
            assert!(
                rect.fits(window),
                "{height}: panel outside window: {:?} {:?}",
                rect.top_left,
                rect.size
            );
        }
        assert!(batch_rect.clears(pipeline_rect), "{height}");
        assert!(batch_rect.clears(status_rect), "{height}");
    }
    assert!(
        short_card_gaps.is_empty(),
        "authored status-to-pipeline gap must be at least {SCREEN_PANEL_GAP} px: {short_card_gaps:?}"
    );
    Ok(())
}

#[test]
fn pipeline_narrows_beside_independent_popup_without_clipping() {
    let mut world = World::new();
    let pipeline = world.spawn_empty().id();
    let window = Vec2::new(1728.0, 1117.0);
    let panel = EdgePanel {
        entity:       pipeline,
        anchor:       Anchor::BottomLeft,
        size:         Vec2::new(window.x * GPU_METER_PANEL_WIDTH_FRACTION, 100.0),
        precedence:   EdgePanelPrecedence::HostPanel,
        width_policy: EdgePanelWidth::Resizable {
            minimum: GPU_METER_PANEL_MIN_WIDTH,
        },
    };
    let popup = AttachedPanelFootprint {
        bounds:    PanelFootprint {
            top_left: Vec2::new(1200.0, 20.0),
            size:     Vec2::new(300.0, 1060.0),
        },
        placement: AttachmentPlacement::Independent,
    };
    let resolved = separate_edge_panels_with_widths(window, &[panel], &[popup], &[]);
    assert_ne!(resolved[0].placement, PanelPlacement::Hidden);
    assert!(resolved[0].width >= GPU_METER_PANEL_MIN_WIDTH);
    let pipeline_rect = PanelFootprint {
        top_left: super::inset_anchor_top_left(
            window,
            Vec2::new(resolved[0].width, panel.size.y),
            panel.anchor,
        ),
        size:     Vec2::new(resolved[0].width, panel.size.y),
    };
    assert!(pipeline_rect.clears(popup.bounds));
}

#[test]
fn pipeline_never_resolves_below_its_content_minimum() {
    let mut world = World::new();
    let pipeline = world.spawn_empty().id();
    let window = Vec2::new(1728.0, 1117.0);
    let panel = EdgePanel {
        entity:       pipeline,
        anchor:       Anchor::BottomLeft,
        size:         Vec2::new(window.x * GPU_METER_PANEL_WIDTH_FRACTION, 100.0),
        precedence:   EdgePanelPrecedence::HostPanel,
        width_policy: EdgePanelWidth::Resizable {
            minimum: GPU_METER_PANEL_MIN_WIDTH,
        },
    };
    let popup = AttachedPanelFootprint {
        bounds:    PanelFootprint {
            top_left: Vec2::new(380.0, 100.0),
            size:     Vec2::new(1300.0, 980.0),
        },
        placement: AttachmentPlacement::Independent,
    };
    let resolved = separate_edge_panels_with_widths(window, &[panel], &[popup], &[]);
    assert!(
        resolved[0].placement == PanelPlacement::Hidden
            || resolved[0].width >= GPU_METER_PANEL_MIN_WIDTH
    );
}

#[test]
fn pipeline_takes_clear_row_when_horizontal_span_is_too_narrow() {
    let mut world = World::new();
    let pipeline = world.spawn_empty().id();
    let window = Vec2::new(1728.0, 1117.0);
    let authored_width = window.x * GPU_METER_PANEL_WIDTH_FRACTION;
    let panel = EdgePanel {
        entity:       pipeline,
        anchor:       Anchor::BottomLeft,
        size:         Vec2::new(authored_width, 100.0),
        precedence:   EdgePanelPrecedence::HostPanel,
        width_policy: EdgePanelWidth::Resizable {
            minimum: GPU_METER_PANEL_MIN_WIDTH,
        },
    };
    let popup = AttachedPanelFootprint {
        bounds:    PanelFootprint {
            top_left: Vec2::new(380.0, 850.0),
            size:     Vec2::new(1300.0, 230.0),
        },
        placement: AttachmentPlacement::Independent,
    };
    let resolved = separate_edge_panels_with_widths(window, &[panel], &[popup], &[]);
    assert!(matches!(
        resolved[0].placement,
        PanelPlacement::Moved { .. }
    ));
    assert_eq!(resolved[0].width, authored_width);
}

#[test]
fn gpu_meter_policy_opts_into_resizing_and_caller_size_wins() -> Result<(), String> {
    let info = DiegeticPanel::screen()
        .size(Percent(0.22), Px(100.0))
        .anchor(Anchor::BottomLeft)
        .build()
        .map_err(|error| error.to_string())?;
    assert!(super::panel_authored_width(&info, EdgePanelWidth::Fixed, None).is_none());
    let mut pipeline = DiegeticPanel::screen()
        .size(Percent(GPU_METER_PANEL_WIDTH_FRACTION), Px(100.0))
        .anchor(Anchor::BottomLeft)
        .build()
        .map_err(|error| error.to_string())?;
    assert!(super::panel_authored_width(&pipeline, EdgePanelWidth::Fixed, None).is_none());
    let policy = EdgePanelWidth::Resizable {
        minimum: GPU_METER_PANEL_MIN_WIDTH,
    };
    let mut intent =
        super::panel_authored_width(&pipeline, policy, None).ok_or("pipeline not resizable")?;
    intent.last_resolved = 1000.0;
    assert!(pipeline.set_screen_size(Sizing::fixed(1000.0), Sizing::fixed(100.0)));
    assert!(super::panel_authored_width(&pipeline, policy, Some(&intent)).is_some());
    assert!(pipeline.set_screen_size(Sizing::fixed(900.0), Sizing::fixed(100.0)));
    assert!(super::panel_authored_width(&pipeline, policy, Some(&intent)).is_none());
    Ok(())
}

#[test]
fn world_face_alone_never_hides_auxiliary_panel_at_full_screen() {
    let mut world = World::new();
    let camera = world.spawn_empty().id();
    let panel = EdgePanel {
        entity:       camera,
        anchor:       Anchor::BottomRight,
        size:         Vec2::new(300.0, 300.0),
        precedence:   EdgePanelPrecedence::CameraControl,
        width_policy: EdgePanelWidth::Fixed,
    };
    let window = Vec2::new(1728.0, 1079.0);
    let face = PanelFootprint {
        top_left: Vec2::new(1300.0, 700.0),
        size:     Vec2::new(400.0, 350.0),
    };
    assert_eq!(
        separate_edge_panels(window, &[panel], &[], &[face])[0].1,
        PanelPlacement::AtAnchor
    );
}

#[test]
fn all_widgets_panels_keep_their_anchors_at_normal_width() -> Result<(), String> {
    let panels = widgets_panels();
    let window = Vec2::new(1269.0, 1006.0);
    let card = carried_widgets_card(window, panels[0].entity)?;
    let placements = separate_edge_panels(window, &panels, &[card], &[]);
    assert!(
        placements
            .iter()
            .all(|(_, placement)| matches!(placement, PanelPlacement::AtAnchor))
    );
    Ok(())
}

#[test]
fn title_bar_stays_anchored_and_hides_floating_group_at_450_by_400() -> Result<(), String> {
    let panels = widgets_panels();
    let window = Vec2::new(450.0, 400.0);
    let card = carried_widgets_card(window, panels[0].entity)?;
    let placements = separate_edge_panels(window, &panels, &[card], &[]);
    assert_eq!(placements[2].1, PanelPlacement::AtAnchor);
    assert_eq!(placements[0].1, PanelPlacement::Hidden);
    Ok(())
}

#[test]
fn title_bar_stays_anchored_across_narrow_and_wide_windows() {
    let panels = widgets_panels();
    for width in (300_u16..=2600).step_by(50) {
        for height in [500.0, 1000.0] {
            let placements =
                separate_edge_panels(Vec2::new(f32::from(width), height), &panels, &[], &[]);
            assert_eq!(
                placements[2].1,
                PanelPlacement::AtAnchor,
                "{width} x {height}"
            );
        }
    }
}

#[test]
fn title_bar_remains_visible_when_only_one_window_inset_fits() {
    let panels = widgets_panels();
    let window = Vec2::new(400.0, 500.0);
    let placements = separate_edge_panels(window, &panels, &[], &[]);
    assert_eq!(placements[2].1, PanelPlacement::AtAnchor);
    assert!((anchored_top_left(window, &panels[2]).x - SCREEN_PANEL_WINDOW_INSET).abs() < 0.001);
}

#[test]
fn moved_panels_align_with_their_anchor_and_keep_exact_gaps() -> Result<(), String> {
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();
    let panels = [
        EdgePanel {
            entity:       first,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(180.0, 90.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       second,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(180.0, 90.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    let window = Vec2::new(300.0, 240.0);
    let placements = separate_edge_panels(window, &panels, &[], &[]);
    assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
    assert_eq!(
        placements[1].1,
        PanelPlacement::Moved {
            top_left: Vec2::new(
                anchored_top_left(window, &panels[0]).x,
                SCREEN_PANEL_WINDOW_INSET + 90.0 + SCREEN_PANEL_GAP
            ),
        }
    );

    let obstacle = PanelScreenBounds::new(Vec2::new(120.0, 0.0), Vec2::new(180.0, 90.0))
        .map_err(|error| error.to_string())?;
    let obstacle = AttachedPanelFootprint {
        bounds:    PanelFootprint::from(obstacle),
        placement: AttachmentPlacement::Independent,
    };
    let placements = separate_edge_panels(window, &panels[..1], &[obstacle], &[]);
    assert_eq!(
        placements[0].1,
        PanelPlacement::Moved {
            top_left: Vec2::new(
                anchored_top_left(window, &panels[0]).x,
                90.0 + SCREEN_PANEL_GAP
            ),
        }
    );

    let carried_card = AttachedPanelFootprint {
        bounds:    PanelFootprint {
            top_left: Vec2::new(
                300.0 - 180.0 - SCREEN_PANEL_WINDOW_INSET - 90.0,
                SCREEN_PANEL_WINDOW_INSET + 89.0,
            ),
            size:     Vec2::new(216.0, 42.0),
        },
        placement: AttachmentPlacement::Carried {
            owner:  first,
            offset: Vec2::new(-90.0, 89.0),
        },
    };
    let placements = separate_edge_panels(Vec2::new(300.0, 270.0), &panels, &[carried_card], &[]);
    assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
    assert_eq!(
        placements[1].1,
        PanelPlacement::Moved {
            top_left: Vec2::new(
                anchored_top_left(Vec2::new(300.0, 270.0), &panels[0]).x,
                SCREEN_PANEL_WINDOW_INSET + 89.0 + 42.0 + SCREEN_PANEL_GAP
            ),
        }
    );
    Ok(())
}

#[test]
fn card_attached_to_hidden_panel_hides_and_returns() -> Result<(), String> {
    let (mut app, window) = placement_app(240, 100);
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);
    let _blocker = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let target = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let card = spawn_edge_panel(&mut app, Anchor::Center)?;
    app.update();
    let (source, destination) = app
        .world_mut()
        .run_system_once(move |reader: PanelEntityReader| {
            (reader.screen(card), reader.screen(target))
        })
        .map_err(|error| error.to_string())?;
    let source = source.ok_or("card handle missing")?;
    let destination = destination.ok_or("target handle missing")?;
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.attach_to_panel(
                source,
                destination,
                PanelAttachment::new(Anchor::TopLeft, Anchor::TopLeft),
            );
        })
        .map_err(|error| error.to_string())?;
    app.update();
    assert_eq!(placement(&app, target), Some(PanelPlacement::Hidden));
    assert_eq!(placement(&app, card), None);
    assert_eq!(
        app.world().get::<Visibility>(card),
        Some(&Visibility::Hidden)
    );

    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(240.0, 400.0);
    app.update();
    assert!(matches!(
        placement(&app, target),
        Some(PanelPlacement::Moved { .. })
    ));
    assert_eq!(
        app.world().get::<Visibility>(card),
        Some(&Visibility::Hidden)
    );
    app.update();
    assert!(matches!(
        placement(&app, target),
        Some(PanelPlacement::Moved { .. })
    ));
    assert_eq!(placement(&app, card), None);
    assert_eq!(
        app.world().get::<Visibility>(card),
        Some(&Visibility::Inherited)
    );
    Ok(())
}

#[test]
fn compact_window_hides_camera_and_help_panels() -> Result<(), String> {
    let panels = widgets_panels();
    let window = Vec2::new(750.0, 1006.0);
    let card = carried_widgets_card(window, panels[0].entity)?;
    let placements = separate_edge_panels(window, &panels, &[card], &[]);
    assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
    assert_eq!(placements[1].1, PanelPlacement::Hidden);
    assert_eq!(placements[2].1, PanelPlacement::AtAnchor);
    assert_eq!(placements[3].1, PanelPlacement::Hidden);
    Ok(())
}

#[test]
fn world_panel_hides_a_lesser_panel_when_no_edge_row_clears() {
    let mut world = World::new();
    let title = world.spawn_empty().id();
    let lesser = world.spawn_empty().id();
    let panels = [
        EdgePanel {
            entity:       title,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(100.0, 100.0),
            precedence:   EdgePanelPrecedence::TitleBar,
            width_policy: EdgePanelWidth::Fixed,
        },
        EdgePanel {
            entity:       lesser,
            anchor:       Anchor::TopRight,
            size:         Vec2::new(100.0, 100.0),
            precedence:   EdgePanelPrecedence::HostPanel,
            width_policy: EdgePanelWidth::Fixed,
        },
    ];
    let footprint = PanelFootprint {
        top_left: Vec2::new(190.0, 0.0),
        size:     Vec2::new(110.0, 200.0),
    };
    let placements = separate_edge_panels(Vec2::new(300.0, 200.0), &panels, &[], &[footprint]);
    assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
    assert_eq!(placements[1].1, PanelPlacement::Hidden);
}

#[test]
fn lesser_panel_stays_at_anchor_when_world_panel_clears() {
    let mut world = World::new();
    let lesser = world.spawn_empty().id();
    let panels = [EdgePanel {
        entity:       lesser,
        anchor:       Anchor::TopRight,
        size:         Vec2::new(100.0, 100.0),
        precedence:   EdgePanelPrecedence::HostPanel,
        width_policy: EdgePanelWidth::Fixed,
    }];
    let footprint = PanelFootprint {
        top_left: Vec2::new(0.0, 0.0),
        size:     Vec2::new(100.0, 200.0),
    };
    let placements = separate_edge_panels(Vec2::new(300.0, 200.0), &panels, &[], &[footprint]);
    assert_eq!(placements[0].1, PanelPlacement::AtAnchor);
}

#[test]
fn auxiliary_panels_keep_window_placements_when_the_camera_moves() -> Result<(), String> {
    let panels = widgets_panels();
    let narrow = Vec2::new(750.0, 1006.0);
    let narrow_card = carried_widgets_card(narrow, panels[0].entity)?;
    let narrow_scene = PanelFootprint {
        top_left: Vec2::new(230.0, 230.0),
        size:     Vec2::new(290.0, 560.0),
    };
    let narrow_placements = separate_edge_panels(narrow, &panels, &[narrow_card], &[narrow_scene]);
    assert_eq!(narrow_placements[0].1, PanelPlacement::AtAnchor);
    assert_eq!(narrow_placements[1].1, PanelPlacement::Hidden);
    assert_eq!(narrow_placements[2].1, PanelPlacement::AtAnchor);
    assert_eq!(narrow_placements[3].1, PanelPlacement::Hidden);

    let normal = Vec2::new(1269.0, 1006.0);
    let normal_card = carried_widgets_card(normal, panels[0].entity)?;
    let normal_scene = PanelFootprint {
        top_left: Vec2::new(500.0, 230.0),
        size:     Vec2::new(270.0, 560.0),
    };
    let normal_placements = separate_edge_panels(normal, &panels, &[normal_card], &[normal_scene]);
    assert!(
        normal_placements[..3]
            .iter()
            .all(|(_, placement)| *placement == PanelPlacement::AtAnchor)
    );
    assert_eq!(normal_placements[3].1, PanelPlacement::AtAnchor);
    Ok(())
}

#[test]
fn projected_world_panel_and_home_insets_settle_with_still_camera() -> Result<(), String> {
    let (mut app, _) = placement_app(300, 200);
    let title = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    app.world_mut()
        .entity_mut(title)
        .insert(EdgePanelPrecedence::TitleBar);
    let lesser = spawn_edge_panel(&mut app, Anchor::TopRight)?;

    let mut world_panel = DiegeticPanel::world()
        .size(Px(100.0), Px(100.0))
        .world_width(4.0)
        .anchor(Anchor::TopLeft)
        .build()
        .map_err(|error| error.to_string())?;
    world_panel.set_width(100.0);
    world_panel.set_height(100.0);
    let root = app.world_mut().spawn(CameraHomeTarget).id();
    let panel_entity = app
        .world_mut()
        .spawn((
            world_panel,
            GlobalTransform::from_translation(Vec3::new(-2.0, 2.0, 0.0)),
        ))
        .id();
    app.world_mut().entity_mut(root).add_child(panel_entity);

    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(300, 200),
        scale_factor:  1.0,
    });
    camera.computed.clip_from_view = bevy::math::proj::perspective_infinite_reverse(1.0, 1.5, 0.1);
    app.world_mut().spawn((
        camera,
        RenderTarget::Window(WindowRef::Primary),
        GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
        FairyDustOrbitCam,
    ));

    let mut settled_insets = None;
    for _ in 0..6 {
        app.update();
        assert_eq!(placement(&app, title), Some(PanelPlacement::AtAnchor));
        assert_eq!(placement(&app, lesser), Some(PanelPlacement::Hidden));
        let insets = panel_insets::covering_panel_insets(
            Vec2::new(300.0, 200.0),
            [CoveringPanel {
                anchor:   Anchor::TopLeft,
                top_left: Vec2::ZERO,
                size:     Vec2::new(180.0, 90.0),
            }],
        );
        if let Some(previous) = settled_insets {
            assert_eq!(insets, previous);
        }
        settled_insets = Some(insets);
    }
    Ok(())
}

#[test]
fn projected_face_and_edge_visibility_stay_clear_at_requested_widths() -> Result<(), String> {
    for (width, height) in [(450_u16, 400_u16), (750, 1006), (1269, 1006)] {
        let (mut app, window, edges) = projected_face_scene(width, height)?;
        let mut settled = None;
        for _ in 0..16 {
            app.update();
            let state: Vec<_> = edges
                .iter()
                .map(|&entity| {
                    (
                        placement(&app, entity),
                        app.world().get::<Visibility>(entity).copied(),
                    )
                })
                .collect();
            if let Some(previous) = &settled {
                assert_eq!(&state, previous, "{width} x {height} edge panels changed");
            }
            settled = Some(state);
            assert_projected_face_clear(
                &mut app,
                window,
                edges,
                Vec2::new(f32::from(width), f32::from(height)),
            )?;
        }
    }
    Ok(())
}

fn projected_face_scene(width: u16, height: u16) -> Result<(App, Entity, [Entity; 4]), String> {
    let (mut app, window) = placement_app(u32::from(width), u32::from(height));
    let title = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    app.world_mut()
        .entity_mut(title)
        .insert(EdgePanelPrecedence::TitleBar);
    let edges = [
        title,
        spawn_edge_panel(&mut app, Anchor::TopRight)?,
        spawn_edge_panel(&mut app, Anchor::BottomRight)?,
        spawn_edge_panel(&mut app, Anchor::BottomLeft)?,
    ];
    let mut world_panel = DiegeticPanel::world()
        .size(Px(100.0), Px(100.0))
        .world_width(2.0)
        .anchor(Anchor::TopLeft)
        .build()
        .map_err(|error| error.to_string())?;
    world_panel.set_width(100.0);
    world_panel.set_height(100.0);
    let home = app.world_mut().spawn(CameraHomeTarget).id();
    let panel = app
        .world_mut()
        .spawn((
            world_panel,
            GlobalTransform::from_translation(Vec3::new(-1.0, 1.0, 0.0)),
        ))
        .id();
    app.world_mut().entity_mut(home).add_child(panel);
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(u32::from(width), u32::from(height)),
        scale_factor:  1.0,
    });
    camera.computed.clip_from_view = bevy::math::proj::perspective_infinite_reverse(
        1.0,
        f32::from(width) / f32::from(height),
        0.1,
    );
    app.world_mut().spawn((
        camera,
        RenderTarget::Window(WindowRef::Primary),
        GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
        FairyDustOrbitCam,
    ));
    Ok((app, window, edges))
}

fn assert_projected_face_clear(
    app: &mut App,
    window: Entity,
    edges: [Entity; 4],
    window_size: Vec2,
) -> Result<(), String> {
    let obstacles = app
        .world_mut()
        .run_system_once(
            move |targets: Query<Entity, With<CameraHomeTarget>>,
                  children: Query<&Children>,
                  panels: Query<(&DiegeticPanel, &GlobalTransform)>,
                  cameras: Query<
                (&Camera, &RenderTarget, &GlobalTransform),
                Or<(With<FairyDustOrbitCam>, With<FreeCam>)>,
            >| {
                framed_world_panel_footprints(&targets, &children, &panels, &cameras, Some(window))
            },
        )
        .map_err(|error| error.to_string())?;
    let obstacles = obstacles.get(&window).ok_or("projected face missing")?;
    assert!(!obstacles.is_empty());
    for (entity, anchor) in edges.into_iter().zip([
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
    ]) {
        assert!(app.world().get::<EdgePanelHomePlacement>(entity).is_some());
        let rect = match placement(app, entity).ok_or("edge placement missing")? {
            PanelPlacement::Hidden => continue,
            PanelPlacement::AtAnchor => PanelFootprint {
                top_left: anchored_top_left(
                    window_size,
                    &EdgePanel {
                        entity,
                        anchor,
                        size: Vec2::new(180.0, 90.0),
                        precedence: EdgePanelPrecedence::HostPanel,
                        width_policy: EdgePanelWidth::Fixed,
                    },
                ),
                size:     Vec2::new(180.0, 90.0),
            },
            PanelPlacement::Moved { top_left } => PanelFootprint {
                top_left,
                size: Vec2::new(180.0, 90.0),
            },
        };
        assert!(
            obstacles.iter().all(|&obstacle| rect.clears(obstacle)),
            "{window_size:?}: visible edge panel covers projected face"
        );
    }
    Ok(())
}

fn placement_app(width: u32, height: u32) -> (App, Entity) {
    let mut app = crate::test_app();
    app.add_plugins(MinimalPlugins);
    screen_panels::install_overlay_picking(&mut app);
    let window = app
        .world_mut()
        .spawn((
            Window {
                resolution: (width, height).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ))
        .id();
    (app, window)
}

fn spawn_edge_panel(app: &mut App, anchor: Anchor) -> Result<Entity, String> {
    let mut panel = DiegeticPanel::screen()
        .size(Px(180.0), Px(90.0))
        .anchor(anchor)
        .build()
        .map_err(|error| error.to_string())?;
    panel.set_width(180.0);
    panel.set_height(90.0);
    Ok(app
        .world_mut()
        .spawn((panel, Visibility::Inherited, InheritedVisibility::VISIBLE))
        .id())
}

fn placement(app: &App, entity: Entity) -> Option<PanelPlacement> {
    app.world()
        .get::<EdgePanelPlacementRecord>(entity)
        .map(|record| record.placement)
}

#[test]
fn attached_screen_panel_blocks_a_moved_edge_panel() -> Result<(), String> {
    let (mut app, _) = placement_app(300, 220);
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);
    let _blocker = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let edge = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    let target = spawn_edge_panel(&mut app, Anchor::Center)?;
    let card = spawn_edge_panel(&mut app, Anchor::Center)?;
    app.update();

    let (source, destination) = app
        .world_mut()
        .run_system_once(move |reader: PanelEntityReader| {
            (reader.screen(card), reader.screen(target))
        })
        .map_err(|error| error.to_string())?;
    let source = source.ok_or("card handle missing")?;
    let destination = destination.ok_or("target handle missing")?;
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.attach_to_panel(
                source,
                destination,
                PanelAttachment::new(Anchor::TopLeft, Anchor::TopLeft),
            );
        })
        .map_err(|error| error.to_string())?;
    app.update();
    app.update();

    assert!(app.world().get::<PanelAttachmentAuthored>(card).is_some());
    assert_eq!(placement(&app, card), None);
    let (edge_bounds, card_bounds) = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            (
                geometry
                    .get(edge)
                    .ok()
                    .and_then(|geometry| geometry.points().screen_bounds()),
                geometry
                    .get(card)
                    .ok()
                    .and_then(|geometry| geometry.points().screen_bounds()),
            )
        })
        .map_err(|error| error.to_string())?;
    let card_bounds = card_bounds.ok_or("attached card bounds missing")?;
    let edge_bounds = edge_bounds.ok_or("edge bounds missing")?;
    assert!(
        placement(&app, edge) == Some(PanelPlacement::Hidden)
            || !intersects(edge_bounds.top_left(), edge_bounds.size(), card_bounds),
        "visible edge panel intersects attached card: {edge_bounds:?}, {card_bounds:?}"
    );
    Ok(())
}

#[test]
fn auxiliary_panel_yields_to_edge_filling_popup_and_returns_when_it_closes() -> Result<(), String> {
    let (mut app, _) = placement_app(1000, 900);
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);
    let camera = spawn_edge_panel(&mut app, Anchor::BottomRight)?;
    app.world_mut()
        .entity_mut(camera)
        .insert(EdgePanelPrecedence::CameraControl);
    let target = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    let popup_width = (-2.0_f32).mul_add(SCREEN_PANEL_WINDOW_INSET, 1000.0);
    let popup_height = (-2.0_f32).mul_add(SCREEN_PANEL_WINDOW_INSET, 900.0);
    let mut popup_panel = DiegeticPanel::screen()
        .size(Px(popup_width), Px(popup_height))
        .anchor(Anchor::TopLeft)
        .build()
        .map_err(|error| error.to_string())?;
    popup_panel.set_width(popup_width);
    popup_panel.set_height(popup_height);
    assert!(popup_panel.set_screen_position(Vec2::splat(SCREEN_PANEL_WINDOW_INSET)));
    let popup = app
        .world_mut()
        .spawn((
            popup_panel,
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
        ))
        .id();
    app.update();
    assert_eq!(placement(&app, camera), Some(PanelPlacement::AtAnchor));

    let (source, destination) = app
        .world_mut()
        .run_system_once(move |reader: PanelEntityReader| {
            (reader.screen(popup), reader.screen(target))
        })
        .map_err(|error| error.to_string())?;
    let source = source.ok_or("popup handle missing")?;
    let destination = destination.ok_or("target handle missing")?;
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.attach_to_panel(
                source,
                destination,
                PanelAttachment::new(Anchor::TopLeft, Anchor::TopLeft),
            );
        })
        .map_err(|error| error.to_string())?;
    app.update();
    let popup_bounds = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            geometry
                .get(popup)
                .ok()
                .and_then(|geometry| geometry.points().screen_bounds())
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(
        placement(&app, camera),
        Some(PanelPlacement::Hidden),
        "popup bounds: {popup_bounds:?}, popup visibility: {:?}, target placement: {:?}",
        app.world().get::<Visibility>(popup),
        placement(&app, target)
    );
    assert_eq!(
        app.world().get::<Visibility>(camera),
        Some(&Visibility::Hidden)
    );

    app.world_mut().despawn(popup);
    app.update();
    assert_eq!(placement(&app, camera), Some(PanelPlacement::AtAnchor));
    assert_eq!(
        app.world().get::<Visibility>(camera),
        Some(&Visibility::Inherited)
    );
    Ok(())
}

#[test]
fn camera_panel_yields_to_keys_popup_and_returns_when_it_closes() -> Result<(), String> {
    let (mut app, _) = placement_app(1000, 900);
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);
    let camera = spawn_edge_panel(&mut app, Anchor::BottomRight)?;
    app.world_mut()
        .entity_mut(camera)
        .insert(EdgePanelPrecedence::CameraControl);
    let target = spawn_edge_panel(&mut app, Anchor::TopLeft)?;
    app.world_mut()
        .entity_mut(target)
        .insert(EdgePanelPrecedence::Description);
    let popup_width = (-2.0_f32).mul_add(SCREEN_PANEL_WINDOW_INSET, 1000.0);
    let popup_height = (-2.0_f32).mul_add(SCREEN_PANEL_WINDOW_INSET, 900.0);
    let mut popup_panel = DiegeticPanel::screen()
        .size(Px(popup_width), Px(popup_height))
        .anchor(Anchor::TopLeft)
        .build()
        .map_err(|error| error.to_string())?;
    popup_panel.set_width(popup_width);
    popup_panel.set_height(popup_height);
    assert!(popup_panel.set_screen_position(Vec2::splat(SCREEN_PANEL_WINDOW_INSET)));
    let popup = app
        .world_mut()
        .spawn((
            popup_panel,
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
        ))
        .id();
    app.update();
    assert_eq!(placement(&app, camera), Some(PanelPlacement::AtAnchor));

    let (source, destination) = app
        .world_mut()
        .run_system_once(move |reader: PanelEntityReader| {
            (reader.screen(popup), reader.screen(target))
        })
        .map_err(|error| error.to_string())?;
    let source = source.ok_or("popup handle missing")?;
    let destination = destination.ok_or("target handle missing")?;
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.attach_to_panel(
                source,
                destination,
                PanelAttachment::new(Anchor::TopLeft, Anchor::TopLeft),
            );
        })
        .map_err(|error| error.to_string())?;
    app.update();
    let popup_bounds = app
        .world_mut()
        .run_system_once(move |geometry: PanelAnchorGeometryParam| {
            geometry
                .get(popup)
                .ok()
                .and_then(|geometry| geometry.points().screen_bounds())
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(
        placement(&app, camera),
        Some(PanelPlacement::Hidden),
        "popup bounds: {popup_bounds:?}, popup visibility: {:?}, target placement: {:?}",
        app.world().get::<Visibility>(popup),
        placement(&app, target)
    );
    assert_eq!(
        app.world().get::<Visibility>(camera),
        Some(&Visibility::Hidden)
    );
    assert_eq!(placement(&app, target), Some(PanelPlacement::AtAnchor));

    app.world_mut().despawn(popup);
    app.update();
    assert_eq!(placement(&app, camera), Some(PanelPlacement::AtAnchor));
    assert_eq!(
        app.world().get::<Visibility>(camera),
        Some(&Visibility::Inherited)
    );
    Ok(())
}

#[test]
fn stable_edge_panel_does_not_change_panel_or_visibility_ticks() -> Result<(), String> {
    let (mut app, _) = placement_app(300, 220);
    let panel = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    app.update();
    app.update();
    assert_eq!(placement(&app, panel), Some(PanelPlacement::AtAnchor));

    let panel_tick = app
        .world()
        .entity(panel)
        .get_change_ticks::<DiegeticPanel>()
        .ok_or("panel change ticks missing")?
        .changed;
    let visibility_tick = app
        .world()
        .entity(panel)
        .get_change_ticks::<Visibility>()
        .ok_or("visibility change ticks missing")?
        .changed;

    app.update();

    let entity = app.world().entity(panel);
    assert_eq!(
        entity
            .get_change_ticks::<DiegeticPanel>()
            .ok_or("panel change ticks missing")?
            .changed,
        panel_tick,
    );
    assert_eq!(
        entity
            .get_change_ticks::<Visibility>()
            .ok_or("visibility change ticks missing")?
            .changed,
        visibility_tick,
    );
    Ok(())
}

#[test]
fn window_resize_marks_real_move_hide_and_reveal() -> Result<(), String> {
    let (mut app, window) = placement_app(300, 240);
    let _blocker = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let panel = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    app.update();
    assert!(matches!(
        placement(&app, panel),
        Some(PanelPlacement::Moved { .. })
    ));
    let panel_tick = app
        .world()
        .entity(panel)
        .get_change_ticks::<DiegeticPanel>()
        .ok_or("panel change ticks missing")?
        .changed;

    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(250.0, 240.0);
    app.update();
    assert_ne!(
        app.world()
            .entity(panel)
            .get_change_ticks::<DiegeticPanel>()
            .ok_or("panel change ticks missing")?
            .changed,
        panel_tick,
    );
    assert!(matches!(
        placement(&app, panel),
        Some(PanelPlacement::Moved { .. })
    ));

    let visibility_tick = app
        .world()
        .entity(panel)
        .get_change_ticks::<Visibility>()
        .ok_or("visibility change ticks missing")?
        .changed;
    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(170.0, 240.0);
    app.update();
    assert_eq!(placement(&app, panel), Some(PanelPlacement::Hidden));
    assert_ne!(
        app.world()
            .entity(panel)
            .get_change_ticks::<Visibility>()
            .ok_or("visibility change ticks missing")?
            .changed,
        visibility_tick,
    );

    let hidden_tick = app
        .world()
        .entity(panel)
        .get_change_ticks::<Visibility>()
        .ok_or("visibility change ticks missing")?
        .changed;
    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(250.0, 240.0);
    app.update();
    assert!(matches!(
        placement(&app, panel),
        Some(PanelPlacement::Moved { .. })
    ));
    assert_ne!(
        app.world()
            .entity(panel)
            .get_change_ticks::<Visibility>()
            .ok_or("visibility change ticks missing")?
            .changed,
        hidden_tick,
    );
    Ok(())
}

#[test]
fn owner_hide_of_separation_hidden_panel_survives_window_growth() -> Result<(), String> {
    let (mut app, window) = placement_app(150, 100);
    let panel = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    app.update();
    assert_eq!(placement(&app, panel), Some(PanelPlacement::Hidden));
    *app.world_mut()
        .get_mut::<Visibility>(panel)
        .ok_or("panel visibility missing")? = Visibility::Hidden;
    app.world_mut()
        .get_mut::<Window>(window)
        .ok_or("window missing")?
        .resolution
        .set(300.0, 100.0);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(panel),
        Some(&Visibility::Hidden)
    );
    assert_eq!(placement(&app, panel), None);
    assert!(app.world().get::<EdgePanelHomePlacement>(panel).is_none());
    Ok(())
}

#[test]
fn owner_show_of_separation_hidden_panel_is_rechecked_for_space() -> Result<(), String> {
    let (mut app, _) = placement_app(150, 100);
    let panel = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    app.update();
    assert_eq!(placement(&app, panel), Some(PanelPlacement::Hidden));
    *app.world_mut()
        .get_mut::<Visibility>(panel)
        .ok_or("panel visibility missing")? = Visibility::Inherited;
    app.update();
    assert_eq!(placement(&app, panel), Some(PanelPlacement::Hidden));
    assert_eq!(
        app.world().get::<Visibility>(panel),
        Some(&Visibility::Hidden)
    );
    Ok(())
}

#[test]
fn spawn_order_survives_a_reused_lower_entity_index() -> Result<(), String> {
    let (mut app, _) = placement_app(240, 130);
    let freed = app.world_mut().spawn_empty().id();
    let first = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let reused = app
        .world_mut()
        .despawn_no_free(freed)
        .ok_or("freeing entity failed")?;
    let mut panel = DiegeticPanel::screen()
        .size(Px(180.0), Px(90.0))
        .anchor(Anchor::TopRight)
        .build()
        .map_err(|error| error.to_string())?;
    panel.set_width(180.0);
    panel.set_height(90.0);
    let second = app
        .world_mut()
        .spawn_at(
            reused,
            (panel, Visibility::Inherited, InheritedVisibility::VISIBLE),
        )
        .map_err(|error| error.to_string())?
        .id();
    assert!(second.index() < first.index());
    app.update();
    assert_eq!(placement(&app, first), Some(PanelPlacement::AtAnchor));
    assert_eq!(placement(&app, second), Some(PanelPlacement::Hidden));
    Ok(())
}

#[test]
fn attachment_releases_separation_hide_and_detach_restores_edge_placement() -> Result<(), String> {
    let (mut app, _) = placement_app(240, 130);
    app.insert_resource(DiegeticTextMeasurer::default());
    app.add_plugins(HeadlessLayoutPlugin);
    let blocker = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let attached = spawn_edge_panel(&mut app, Anchor::TopRight)?;
    let target = spawn_edge_panel(&mut app, Anchor::Center)?;
    app.update();
    assert_eq!(placement(&app, attached), Some(PanelPlacement::Hidden));
    let (source, destination) = app
        .world_mut()
        .run_system_once(move |reader: PanelEntityReader| {
            (reader.screen(attached), reader.screen(target))
        })
        .map_err(|error| error.to_string())?;
    let source = source.ok_or("source handle missing")?;
    let destination = destination.ok_or("target handle missing")?;
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.attach_to_panel(
                source,
                destination,
                PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight),
            );
        })
        .map_err(|error| error.to_string())?;
    assert!(
        app.world()
            .get::<PanelAttachmentAuthored>(attached)
            .is_some()
    );
    app.update();
    assert_eq!(placement(&app, attached), None);
    assert_eq!(
        app.world().get::<Visibility>(attached),
        Some(&Visibility::Inherited)
    );
    // The newly revealed attachment occupies the blocker's anchor this frame.
    assert_eq!(placement(&app, blocker), Some(PanelPlacement::Hidden));
    app.world_mut()
        .run_system_once(move |mut commands: Commands| {
            commands.detach(source);
        })
        .map_err(|error| error.to_string())?;
    app.update();
    assert_eq!(placement(&app, attached), Some(PanelPlacement::Hidden));
    assert_eq!(placement(&app, blocker), Some(PanelPlacement::AtAnchor));
    Ok(())
}
