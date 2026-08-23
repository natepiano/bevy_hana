//! Interactive tour of `hana_kana`'s relationship-backed cascade engine.
//!
//! `Source A`, `Source B`, and `Leaf` each carry `Cascade<CubeScale>`.
//! `Leaf` also carries `CascadeFrom`, which selects the entity consulted when
//! `Leaf` inherits. `Resolved<CubeScale>` is the only value consumed by the
//! system that updates each cube's `Transform`.
//!
//! Controls:
//! - `G` — toggle the root `CascadeDefault<CubeScale>`.
//! - `A` — toggle `Source A` between a local override and inheritance.
//! - `L` — toggle `Leaf` between a local override and inheritance.
//! - `R` — retarget `Leaf` between `Source A` and `Source B`.
//! - `H` — home the camera.

use std::time::Duration;

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Msaa;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::StatsPanelRow;
use fairy_dust::StatsPanelSection;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_label;
use fairy_dust::diegetic_stats_sections_panel_with_integral_advance;
use fairy_dust::diegetic_stats_sections_tree_with_integral_advance;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::FontRegistry;
use hana_kana::Cascade;
use hana_kana::CascadeDefault;
use hana_kana::CascadeEntityCommandsExt;
use hana_kana::CascadeFrom;
use hana_kana::CascadePlugin;
use hana_kana::CascadeSet;
use hana_kana::Position;
use hana_kana::Resolved;
use hana_lagrange::OrbitCamPreset;

// camera
const HOME_MARGIN: f32 = 0.55;
const HOME_PITCH: f32 = 0.32;

// cascade values
const DEFAULT_SCALE_A: CubeScale = CubeScale(0.60);
const DEFAULT_SCALE_B: CubeScale = CubeScale(1.20);
const LEAF_OVERRIDE: CubeScale = CubeScale(0.85);
const SOURCE_A_OVERRIDE: CubeScale = CubeScale(1.45);

// controls
const CONTROL_FLASH_SECONDS: f32 = 0.5;
const DEFAULT_CONTROL: &str = "G Root Default";
const LEAF_CONTROL: &str = "L Leaf Override";
const RETARGET_CONTROL: &str = "R Leaf Source";
const SOURCE_A_CONTROL: &str = "A Source A Override";

// cube presentation
const CUBE_CLEARANCE: f32 = 0.05;
const CUBE_SIZE: f32 = 0.8;
const LEAF_COLOR: Color = Color::srgb(1.0, 0.62, 0.22);
const LEAF_LABEL: &str = "Leaf";
const LEAF_POSITION: Vec3 = Vec3::new(1.45, 0.0, 0.0);
const SOURCE_A_COLOR: Color = Color::srgb(0.25, 0.55, 0.95);
const SOURCE_A_LABEL: &str = "Source A";
const SOURCE_A_POSITION: Vec3 = Vec3::new(-1.45, 0.0, 0.0);
const SOURCE_B_COLOR: Color = Color::srgb(0.25, 0.85, 0.55);
const SOURCE_B_LABEL: &str = "Source B";
const SOURCE_B_POSITION: Vec3 = Vec3::ZERO;

// description panel
const DESCRIPTION_LINES: [&str; 6] = [
    "All three cubes begin by inheriting the root default.",
    "G changes the root, so every inheriting cube resizes.",
    "A toggles Source A's override; Leaf follows Source A.",
    "L toggles Leaf's own override.",
    "R retargets Leaf between Source A and Source B.",
    "Resolved<CubeScale> drives each Transform.",
];
const DESCRIPTION_TITLE: &str = "Cascade<CubeScale>";

// example
const AUTHORED_LABEL: &str = "Authored";
const CASCADE_FROM_LABEL: &str = "CascadeFrom";
const CASCADE_STATE_TITLE: &str = "Cascade state";
const EXAMPLE_TITLE: &str = "Cascade";
const INITIALIZING_LABEL: &str = "initializing";
const RESOLVED_LABEL: &str = "Resolved";
const RESOLVED_VALUES_LABEL: &str = "Resolved values";
const ROOT_DEFAULT_LABEL: &str = "Root default";

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
struct CubeScale(f32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CascadeSource {
    Root,
    SourceA,
    SourceB,
    Other,
}

impl CascadeSource {
    const fn cascade_from_label(self) -> &'static str {
        match self {
            Self::Root => "none -> root",
            Self::SourceA => SOURCE_A_LABEL,
            Self::SourceB => SOURCE_B_LABEL,
            Self::Other => "another entity",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ParticipantStatus {
    authored:       Cascade<CubeScale>,
    cascade_source: CascadeSource,
    resolved:       CubeScale,
}

#[derive(Clone, Copy, PartialEq)]
struct StatusSnapshot {
    default:  CubeScale,
    source_a: ParticipantStatus,
    source_b: ParticipantStatus,
    leaf:     ParticipantStatus,
}

#[derive(Resource, Clone, Copy)]
struct CascadeCubes {
    source_a: Entity,
    source_b: Entity,
    leaf:     Entity,
}

#[derive(Resource, Default)]
struct ControlFlashes {
    default:  Option<Timer>,
    source_a: Option<Timer>,
    leaf:     Option<Timer>,
    retarget: Option<Timer>,
}

impl ControlFlashes {
    fn flash_default(&mut self) { Self::start(&mut self.default); }

    fn flash_source_a(&mut self) { Self::start(&mut self.source_a); }

    fn flash_leaf(&mut self) { Self::start(&mut self.leaf); }

    fn flash_retarget(&mut self) { Self::start(&mut self.retarget); }

    fn default_activation(&self) -> ControlActivation { Self::activation(self.default.as_ref()) }

    fn source_a_activation(&self) -> ControlActivation { Self::activation(self.source_a.as_ref()) }

    fn leaf_activation(&self) -> ControlActivation { Self::activation(self.leaf.as_ref()) }

    fn retarget_activation(&self) -> ControlActivation { Self::activation(self.retarget.as_ref()) }

    fn tick(&mut self, delta: Duration) {
        Self::tick_one(&mut self.default, delta);
        Self::tick_one(&mut self.source_a, delta);
        Self::tick_one(&mut self.leaf, delta);
        Self::tick_one(&mut self.retarget, delta);
    }

    fn start(timer: &mut Option<Timer>) {
        *timer = Some(Timer::from_seconds(CONTROL_FLASH_SECONDS, TimerMode::Once));
    }

    fn activation(timer: Option<&Timer>) -> ControlActivation {
        timer.map_or(ControlActivation::Inactive, |_| ControlActivation::Active)
    }

    fn tick_one(timer: &mut Option<Timer>, delta: Duration) {
        if timer
            .as_mut()
            .is_some_and(|timer| timer.tick(delta).just_finished())
        {
            *timer = None;
        }
    }
}

#[derive(Component)]
struct CascadeStatusPanel;

fn main() {
    fairy_dust::sprinkle_example()
        .add_plugins(CascadePlugin::new(DEFAULT_SCALE_A))
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_orbit_cam_preset_bundle(
            |_| {},
            OrbitCamPreset::blender_like(),
            (Msaa::Off, TemporalAntiAliasing::default()),
        )
        .with_stable_transparency()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(DEFAULT_CONTROL)
                .control(SOURCE_A_CONTROL)
                .control(LEAF_CONTROL)
                .control(RETARGET_CONTROL),
        )
        .wire_chip_to_state::<ControlFlashes, _>(
            DEFAULT_CONTROL,
            ControlFlashes::default_activation,
        )
        .wire_chip_to_state::<ControlFlashes, _>(
            SOURCE_A_CONTROL,
            ControlFlashes::source_a_activation,
        )
        .wire_chip_to_state::<ControlFlashes, _>(LEAF_CONTROL, ControlFlashes::leaf_activation)
        .wire_chip_to_state::<ControlFlashes, _>(
            RETARGET_CONTROL,
            ControlFlashes::retarget_activation,
        )
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .init_resource::<ControlFlashes>()
        .add_systems(Startup, setup)
        .add_systems(Update, tick_control_flashes)
        .add_systems(
            Update,
            (apply_resolved_scale, refresh_status_panel)
                .chain()
                .after(CascadeSet::Propagate),
        )
        .with_shortcut(KeyCode::KeyG, toggle_default)
        .with_shortcut(KeyCode::KeyA, toggle_source_a)
        .with_shortcut(KeyCode::KeyL, toggle_leaf)
        .with_shortcut(KeyCode::KeyR, retarget_leaf)
        .run();
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_TITLE)
        .with_fit_width()
        .lines(DESCRIPTION_LINES)
}

fn setup(
    mut commands: Commands,
    fonts: Res<FontRegistry>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::from_size(Vec3::splat(CUBE_SIZE)));
    let source_a = spawn_cube(
        &mut commands,
        mesh.clone(),
        materials.add(StandardMaterial::from(SOURCE_A_COLOR)),
        SOURCE_A_POSITION,
        SOURCE_A_LABEL,
        Cascade::Inherit,
    );
    let source_b = spawn_cube(
        &mut commands,
        mesh.clone(),
        materials.add(StandardMaterial::from(SOURCE_B_COLOR)),
        SOURCE_B_POSITION,
        SOURCE_B_LABEL,
        Cascade::Inherit,
    );
    let leaf = spawn_cube(
        &mut commands,
        mesh,
        materials.add(StandardMaterial::from(LEAF_COLOR)),
        LEAF_POSITION,
        LEAF_LABEL,
        Cascade::Inherit,
    );
    commands.entity(leaf).insert(CascadeFrom::new(source_a));

    let cubes = CascadeCubes {
        source_a,
        source_b,
        leaf,
    };
    commands.insert_resource(cubes);

    match diegetic_stats_sections_panel_with_integral_advance(
        &pending_status_sections(),
        &fonts,
        &mut materials,
    ) {
        Ok(panel) => {
            commands.spawn((CascadeStatusPanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("cascade: failed to build status panel: {error}");
        },
    }
}

fn spawn_cube(
    commands: &mut Commands,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    position: impl Into<Position>,
    label: &'static str,
    authored: Cascade<CubeScale>,
) -> Entity {
    let position = position.into();
    commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(*position),
            CameraHomeTarget,
            authored,
        ))
        .with_children(|parent| {
            parent.spawn(cube_face_label(Face::Front, label, CUBE_SIZE));
        })
        .id()
}

fn toggle_default(
    mut default: ResMut<CascadeDefault<CubeScale>>,
    mut flashes: ResMut<ControlFlashes>,
) {
    default.0 = if default.0 == DEFAULT_SCALE_A {
        DEFAULT_SCALE_B
    } else {
        DEFAULT_SCALE_A
    };
    flashes.flash_default();
}

fn toggle_source_a(
    cubes: Res<CascadeCubes>,
    authored: Query<&Cascade<CubeScale>>,
    mut commands: Commands,
    mut flashes: ResMut<ControlFlashes>,
) {
    let Ok(current) = authored.get(cubes.source_a) else {
        return;
    };
    let next = match current {
        Cascade::Inherit => Cascade::Override(SOURCE_A_OVERRIDE),
        Cascade::Override(_) => Cascade::Inherit,
    };
    commands.entity(cubes.source_a).set_cascade(next);
    flashes.flash_source_a();
}

fn toggle_leaf(
    cubes: Res<CascadeCubes>,
    authored: Query<&Cascade<CubeScale>>,
    mut commands: Commands,
    mut flashes: ResMut<ControlFlashes>,
) {
    let Ok(current) = authored.get(cubes.leaf) else {
        return;
    };
    let next = match current {
        Cascade::Inherit => Cascade::Override(LEAF_OVERRIDE),
        Cascade::Override(_) => Cascade::Inherit,
    };
    commands.entity(cubes.leaf).set_cascade(next);
    flashes.flash_leaf();
}

fn retarget_leaf(
    cubes: Res<CascadeCubes>,
    relationships: Query<&CascadeFrom>,
    mut commands: Commands,
    mut flashes: ResMut<ControlFlashes>,
) {
    let Ok(current) = relationships.get(cubes.leaf) else {
        return;
    };
    let target = if current.target() == cubes.source_a {
        cubes.source_b
    } else {
        cubes.source_a
    };
    commands.entity(cubes.leaf).insert(CascadeFrom::new(target));
    flashes.flash_retarget();
}

fn tick_control_flashes(time: Res<Time>, mut flashes: ResMut<ControlFlashes>) {
    flashes.tick(time.delta());
}

fn apply_resolved_scale(
    mut cubes: Query<(&Resolved<CubeScale>, &mut Transform), Changed<Resolved<CubeScale>>>,
) {
    for (resolved, mut transform) in &mut cubes {
        transform.scale = Vec3::splat(resolved.0.0);
        transform.translation.y = CUBE_SIZE * resolved.0.0 / 2.0 + CUBE_CLEARANCE;
    }
}

fn refresh_status_panel(
    default: Res<CascadeDefault<CubeScale>>,
    cubes: Res<CascadeCubes>,
    participants: Query<(
        &Cascade<CubeScale>,
        &Resolved<CubeScale>,
        Option<&CascadeFrom>,
    )>,
    panels: Query<Entity, With<CascadeStatusPanel>>,
    mut displayed: Local<Option<StatusSnapshot>>,
    mut commands: Commands,
    fonts: Res<FontRegistry>,
) {
    let Some(snapshot) = status_snapshot(default.0, *cubes, &participants) else {
        return;
    };
    if displayed.as_ref() == Some(&snapshot) {
        return;
    }
    for panel in &panels {
        if let Err(error) = commands.set_tree(
            panel,
            diegetic_stats_sections_tree_with_integral_advance(&status_sections(snapshot), &fonts),
        ) {
            warn!("failed to replace cascade stats panel tree: {error}");
        }
    }
    *displayed = Some(snapshot);
}

fn status_snapshot(
    default: CubeScale,
    cubes: CascadeCubes,
    participants: &Query<(
        &Cascade<CubeScale>,
        &Resolved<CubeScale>,
        Option<&CascadeFrom>,
    )>,
) -> Option<StatusSnapshot> {
    Some(StatusSnapshot {
        default,
        source_a: participant_status(cubes.source_a, cubes, participants)?,
        source_b: participant_status(cubes.source_b, cubes, participants)?,
        leaf: participant_status(cubes.leaf, cubes, participants)?,
    })
}

fn participant_status(
    entity: Entity,
    cubes: CascadeCubes,
    participants: &Query<(
        &Cascade<CubeScale>,
        &Resolved<CubeScale>,
        Option<&CascadeFrom>,
    )>,
) -> Option<ParticipantStatus> {
    let (authored, resolved, relationship) = participants.get(entity).ok()?;
    Some(ParticipantStatus {
        authored:       *authored,
        cascade_source: cascade_source(relationship, cubes),
        resolved:       resolved.0,
    })
}

fn cascade_source(relationship: Option<&CascadeFrom>, cubes: CascadeCubes) -> CascadeSource {
    match relationship.map(CascadeFrom::target) {
        None => CascadeSource::Root,
        Some(target) if target == cubes.source_a => CascadeSource::SourceA,
        Some(target) if target == cubes.source_b => CascadeSource::SourceB,
        Some(_) => CascadeSource::Other,
    }
}

fn pending_status_sections() -> [StatsPanelSection; 1] {
    [StatsPanelSection::new(
        CASCADE_STATE_TITLE,
        [StatsPanelRow::new(
            RESOLVED_VALUES_LABEL,
            INITIALIZING_LABEL,
        )],
    )]
}

fn status_sections(snapshot: StatusSnapshot) -> [StatsPanelSection; 4] {
    [
        StatsPanelSection::new(
            CASCADE_STATE_TITLE,
            [StatsPanelRow::new(
                ROOT_DEFAULT_LABEL,
                scale_label(snapshot.default),
            )],
        ),
        participant_section(SOURCE_A_LABEL, snapshot.source_a),
        participant_section(SOURCE_B_LABEL, snapshot.source_b),
        participant_section(LEAF_LABEL, snapshot.leaf),
    ]
}

fn participant_section(label: &str, status: ParticipantStatus) -> StatsPanelSection {
    StatsPanelSection::new(
        label,
        [
            StatsPanelRow::new(AUTHORED_LABEL, authored_label(status.authored)),
            StatsPanelRow::new(
                CASCADE_FROM_LABEL,
                status.cascade_source.cascade_from_label(),
            ),
            StatsPanelRow::new(RESOLVED_LABEL, scale_label(status.resolved)),
        ],
    )
}

fn authored_label(authored: Cascade<CubeScale>) -> String {
    match authored {
        Cascade::Inherit => "inherit".to_string(),
        Cascade::Override(scale) => format!("override {}", scale_label(scale)),
    }
}

fn scale_label(scale: CubeScale) -> String { format!("{:.2}", scale.0) }
