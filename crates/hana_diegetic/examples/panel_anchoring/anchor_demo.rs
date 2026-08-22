//! The anchor / spin capabilities (`1`/`2`): a chain of anchor tiles —
//! a root tile and the dependents that follow it down the chain — the arrow-key
//! anchor grid, the cumulative depth offset, the spin envelope, and the
//! Navigation legend glow. `+`/`-` grow and shrink the chain; every link shares
//! the same source/target anchor selection, so each adjacent pair chains.

use bevy::prelude::Alpha;
use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Gizmos;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Name;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::TitleChipActivation;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DrawOverflow;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorGeometryParam;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelCircle;
use hana_diegetic::PanelCoord;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::PanelPoint;
use hana_diegetic::PanelShape;
use hana_diegetic::ResolvedPanelAnchorGeometry;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_valence::AnchorPose;
use hana_valence::Displacement;
use hana_valence::Orientation;

use crate::constants::*;
use crate::hinge::HingeChain;
use crate::hinge::advance_hinge;
use crate::hinge::build_hinge_number_tree;
use crate::hinge::hinge_accent;
use crate::hinge::hinge_paused;
use crate::presentation::AnchorPanelMaterials;
use crate::presentation::smoothstep;
use crate::scene::ActiveCapability;
use crate::scene::ModeMorph;

/// A tile in the shared chain used by every capability. `order` is its fixed
/// position along the chain (`0` = the origin). Tile `0` is the unanchored origin
/// at [`TARGET_POSITION`]. In fan modes every later tile anchors onto the
/// previous one; in hinge mode tile `0` becomes the arrangement root and the
/// others become arrangement members. A tile carries an [`AnchorPose`] while a
/// spin, arrangement hinge, anchor transition, or mode morph is in flight.
#[derive(Component)]
pub(crate) struct AnchorTile {
    pub(crate) order: usize,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(crate) struct FanAttachmentState {
    target:   Entity,
    authored: PanelAttachment,
}

/// Number of tiles in the anchor chain (capabilities `1`/`2`); `+`/`-` change it
/// within `ANCHOR_MIN_TILES..=ANCHOR_MAX_TILES`. [`reconcile_anchor_chain`] makes
/// the live chain match by spawning or despawning tiles off the bottom end.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct AnchorChain {
    count: usize,
}

impl Default for AnchorChain {
    fn default() -> Self {
        Self {
            count: ANCHOR_MIN_TILES,
        }
    }
}

impl AnchorChain {
    /// The number of tiles currently requested.
    pub(crate) const fn count(self) -> usize { self.count }

    /// Adds one tile at the bottom (`+`), up to [`ANCHOR_MAX_TILES`].
    fn add_tile(&mut self) { self.set_count(self.count + 1); }

    /// Removes the bottom tile (`-`), down to [`ANCHOR_MIN_TILES`].
    fn remove_tile(&mut self) { self.set_count(self.count.saturating_sub(1)); }

    fn set_count(&mut self, requested: usize) {
        self.count = requested.clamp(ANCHOR_MIN_TILES, ANCHOR_MAX_TILES);
    }
}

/// Whether the anchor marker discs are drawn on the panels. `O` toggles it; on
/// by default. The title-bar `Show Anchor` chip highlights while on.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct ShowAnchorMarkers(pub(crate) bool);

impl Default for ShowAnchorMarkers {
    fn default() -> Self { Self(true) }
}

impl TitleChipActivation for ShowAnchorMarkers {
    fn activation(&self) -> ControlActivation {
        if self.0 {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

/// `O` toggles the anchor marker discs on every panel.
pub(crate) fn toggle_show_anchor_markers(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut show: ResMut<ShowAnchorMarkers>,
) {
    if keyboard.just_pressed(KeyCode::KeyO) {
        show.0 = !show.0;
    }
}

/// Whether the camera reframes the panel union when it overflows the viewport.
/// While on, the camera holds its pose until the union extent reaches the window
/// edge, then re-fits to pull it back inside (see
/// [`crate::scene::autofit_to_panels`]); it never zooms in to chase slack, so the
/// view does not oscillate. `I` toggles it; on by default. The title-bar
/// `Autofit` chip highlights while on. When off, the camera holds its current
/// pose.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct Autofit(pub(crate) bool);

impl Default for Autofit {
    fn default() -> Self { Self(true) }
}

impl TitleChipActivation for Autofit {
    fn activation(&self) -> ControlActivation {
        if self.0 {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

/// `I` toggles whether the camera reframes the panel union on viewport overflow.
pub(crate) fn toggle_autofit(keyboard: Res<ButtonInput<KeyCode>>, mut autofit: ResMut<Autofit>) {
    if keyboard.just_pressed(KeyCode::KeyI) {
        autofit.0 = !autofit.0;
    }
}

/// Title-bar flash for the `Tiles` control: `plus` lights the `+` while a tile is
/// being added, `minus` lights the `-` while one is removed. Each holds briefly
/// past key release so a single tap still registers.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct TileCountFlash {
    pub(crate) plus:  bool,
    pub(crate) minus: bool,
}

/// Maps a flash bool to a title-chip activation.
pub(crate) const fn flash_activation(lit: bool) -> ControlActivation {
    if lit {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

/// Lights the `+` / `-` segments of the title-bar Tiles control while the add /
/// remove keys are held (with a short release tail), mirroring the keys that grow
/// and shrink the tile count. Writes the resource only when a segment's lit state
/// flips, so the wired chips update on those edges alone.
pub(crate) fn advance_tile_count_flash(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    morph: Res<ModeMorph>,
    mut flash: ResMut<TileCountFlash>,
    mut plus_timer: Local<f32>,
    mut minus_timer: Local<f32>,
) {
    let dt = time.delta_secs();
    // Every capability resizes its chain with `+`/`-`, so the flash lights
    // whenever a morph is not in flight.
    let changeable = !morph.active();
    let plus_held =
        changeable && (keyboard.pressed(KeyCode::Equal) || keyboard.pressed(KeyCode::NumpadAdd));
    let minus_held = changeable
        && (keyboard.pressed(KeyCode::Minus) || keyboard.pressed(KeyCode::NumpadSubtract));
    refresh_or_decay(&mut plus_timer, plus_held, dt);
    refresh_or_decay(&mut minus_timer, minus_held, dt);
    let plus = *plus_timer > 0.0;
    let minus = *minus_timer > 0.0;
    if flash.plus != plus {
        flash.plus = plus;
    }
    if flash.minus != minus {
        flash.minus = minus;
    }
}

/// Envelope phase of the spin animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpinPhase {
    /// Spin accelerating from rest.
    Rising,
    /// Spinning at the full `SPIN_RATE_RAD`.
    Holding,
    /// Spin decelerating to rest, settling on the nearest upright orientation.
    Falling,
}

/// State of the spin animation (menu capability `2`). Depth stays under manual
/// `[`/`]` control; the animation only rotates the chain about the plane normal.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct Spin {
    /// Current phase; `None` when the animation is idle (no pose applied).
    phase:     Option<SpinPhase>,
    /// Seconds into the current `Rising`/`Falling` ease.
    timer:     f32,
    /// Accumulated spin angle about the plane normal, radians.
    angle:     f32,
    /// Spin angle when the `Falling` ease began.
    fall_from: f32,
    /// Spin angle the `Falling` ease settles on (nearest upright orientation).
    fall_to:   f32,
    /// Whether the spin is frozen mid-motion; `P` toggles it while a phase is
    /// active. Cleared whenever the capability starts or stops.
    paused:    bool,
}

/// A direction control in the info-panel legend. The key that drove the current
/// anchor transition glows yellow in the legend while the ease is in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnchorDirection {
    Left,
    Right,
    Top,
    Bottom,
    Reset,
}

impl AnchorDirection {
    /// The glyph shown for this direction in the info-panel legend.
    pub(crate) const fn glyph(self) -> &'static str {
        match self {
            Self::Left => "←",
            Self::Right => "→",
            Self::Top => "↑",
            Self::Bottom => "↓",
            Self::Reset => "R Reset",
        }
    }
}

/// In-flight ease of an anchor-selection change. The pose is offset to hold the
/// dependent at its previous resolved position, then that offset eases to zero,
/// so the panel glides to the newly selected anchor instead of snapping.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct AnchorTransition {
    /// Whether an ease is currently in flight.
    active:            bool,
    /// Seconds into the current ease.
    timer:             f32,
    /// Plane-frame offset (meters) the ease starts from and decays to zero;
    /// equals the previous resolved position minus the new one.
    from_offset:       Vec3,
    /// Source/target anchor indices when the ease began; the markers ease from
    /// these to the current selection's anchors in step with the panel slide.
    from_source_index: usize,
    from_target_index: usize,
}

impl AnchorTransition {
    /// Plane-frame slide offset at the current ease progress (zero when idle).
    fn current_offset(self) -> Vec3 {
        if !self.active {
            return Vec3::ZERO;
        }
        self.from_offset * (1.0 - self.progress())
    }

    /// Eased progress on `[0, 1]`: `0` at the start of the slide, `1` once it
    /// settles. Returns `1` when idle so markers rest at their selected anchor.
    fn progress(self) -> f32 {
        if !self.active {
            return 1.0;
        }
        smoothstep((self.timer / ANCHOR_TRANSITION_SECS).clamp(0.0, 1.0))
    }
}

/// Which navigation/title-bar controls glow this frame. `direction`/`tab` light
/// the info-panel legend; `depth_out`/`depth_in` light the title-bar `[`/`]`
/// Depth segments while their keys are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LegendGlow {
    pub(crate) direction: Option<AnchorDirection>,
    tab:                  bool,
    pub(crate) depth_out: bool,
    pub(crate) depth_in:  bool,
}

/// Per-control glow countdowns. A tapped control (arrow / `R Reset` / `Tab`)
/// holds for [`LEGEND_TAP_GLOW_SECS`]; each held depth key (`[`/`]`) keeps its own
/// timer refreshed, then fades [`LEGEND_GLOW_TAIL_SECS`] after release. The two
/// depth timers are independent, so the keys light independently.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct LegendHighlight {
    direction:       Option<AnchorDirection>,
    dir_timer:       f32,
    tab_timer:       f32,
    depth_out_timer: f32,
    depth_in_timer:  f32,
}

impl LegendHighlight {
    /// Snapshot of which controls are lit (timer still positive) this frame.
    pub(crate) fn glow(self) -> LegendGlow {
        LegendGlow {
            direction: (self.dir_timer > 0.0).then_some(self.direction).flatten(),
            tab:       self.tab_timer > 0.0,
            depth_out: self.depth_out_timer > 0.0,
            depth_in:  self.depth_in_timer > 0.0,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct AnchorSelection {
    pub(crate) source_index: usize,
    pub(crate) target_index: usize,
    depth_mm:                f32,
}

impl Default for AnchorSelection {
    fn default() -> Self {
        Self {
            source_index: DEFAULT_SOURCE_INDEX,
            target_index: DEFAULT_TARGET_INDEX,
            depth_mm:     0.0,
        }
    }
}

impl AnchorSelection {
    const fn source_anchor(self) -> Anchor { ANCHOR_POINTS[self.source_index] }

    const fn target_anchor(self) -> Anchor { ANCHOR_POINTS[self.target_index] }

    pub(crate) const fn source_label(self) -> &'static str { ANCHOR_NAMES[self.source_index] }

    pub(crate) const fn target_label(self) -> &'static str { ANCHOR_NAMES[self.target_index] }
}

/// Which panel the arrow keys move. `Tab` toggles it; the selected panel's
/// section title shows at full strength, the other is dimmed.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectedPanel {
    /// The dependent (green) panel — the arrows move its source anchor.
    #[default]
    Anchored,
    /// The target (blue) panel — the arrows move its anchor, dragging the
    /// dependent along.
    Target,
}

impl SelectedPanel {
    const fn toggled(self) -> Self {
        match self {
            Self::Anchored => Self::Target,
            Self::Target => Self::Anchored,
        }
    }
}

/// Moves a 3×3-grid anchor index one step in `direction` without wrapping, so
/// the selected panel's anchor stops at the grid edges. `Reset` is a no-op here.
fn grid_move(index: usize, direction: AnchorDirection) -> usize {
    let last = INFO_GRID_SIDE - 1;
    let row = index / INFO_GRID_SIDE;
    let col = index % INFO_GRID_SIDE;
    let (row, col) = match direction {
        AnchorDirection::Left => (row, col.saturating_sub(1)),
        AnchorDirection::Right => (row, (col + 1).min(last)),
        AnchorDirection::Top => (row.saturating_sub(1), col),
        AnchorDirection::Bottom => ((row + 1).min(last), col),
        AnchorDirection::Reset => (row, col),
    };
    row * INFO_GRID_SIDE + col
}

/// Spawns the one persistent tile set in the anchor fan: `count` tiles, the first
/// the unanchored chain origin at [`TARGET_POSITION`] and each later one anchored
/// onto the previous. Capability switches morph these same tiles between layouts
/// rather than respawning them.
pub(crate) fn spawn_anchor_scene(
    commands: &mut Commands,
    selection: AnchorSelection,
    count: usize,
    show_marker: bool,
    materials: &AnchorPanelMaterials,
) {
    let mut parent = None;
    let link_delta = tile_link_delta(ANCHOR_INDEX, selection);
    let mut next_position = TARGET_POSITION;
    for order in 0..count {
        let Some(tile) = spawn_tile(
            commands,
            ANCHOR_INDEX,
            order,
            count,
            next_position,
            parent,
            selection,
            show_marker,
            false,
            materials,
        ) else {
            return;
        };
        parent = Some(tile);
        next_position += link_delta;
    }
}

/// Spawns one tile for `mode`. The origin (`order == 0`, `parent` `None`) carries
/// no relation and rests at [`TARGET_POSITION`]. A later fan tile anchors onto
/// `parent` and receives an initial [`AnchorPose`] when a fan animation is in
/// flight. Hinge tiles receive no relation here;
/// [`crate::hinge::reconcile_hinge_arrangement`] adds them through
/// [`hana_diegetic::ArrangedPanel`] membership.
fn spawn_tile(
    commands: &mut Commands,
    mode: usize,
    order: usize,
    count: usize,
    initial_position: Vec3,
    parent: Option<Entity>,
    selection: AnchorSelection,
    show_marker: bool,
    animating: bool,
    materials: &AnchorPanelMaterials,
) -> Option<Entity> {
    let tree = build_tile_tree(mode, order, count, selection, show_marker);
    let panel = match build_anchor_panel(tree, materials) {
        Ok(panel) => panel,
        Err(error) => {
            error!(
                "panel_anchoring: failed to build tile {}: {error}",
                order + 1
            );
            return None;
        },
    };
    let mut tile = commands.spawn((
        Name::new(format!("Tile {}", order + 1)),
        AnchorTile { order },
        CameraHomeTarget,
        panel,
        Transform::from_translation(initial_position),
        Visibility::default(),
    ));
    if parent.is_some() && animating {
        tile.insert(AnchorPose::default());
    }
    Some(tile.id())
}

/// One tile's layout tree for `mode`: the hinge chain shows its link number; the
/// fan modes show the anchor markers (at their resting positions) and the
/// cumulative depth label.
pub(crate) fn build_tile_tree(
    mode: usize,
    order: usize,
    count: usize,
    selection: AnchorSelection,
    show_marker: bool,
) -> LayoutTree {
    if mode == HINGE_CHAIN_INDEX {
        build_hinge_number_tree(order, count)
    } else {
        build_anchor_tile_tree(
            order,
            count,
            anchor_center(selection.source_anchor()),
            anchor_center(selection.target_anchor()),
            selection.depth_mm,
            show_marker,
        )
    }
}

/// The resting local displacement (meters, plane frame) from a tile to its parent
/// for `mode`: the anchor fan glues the selected source anchor to the parent's
/// target anchor plus the depth offset; the hinge strip stacks each link one panel
/// height below its parent, edge to edge.
pub(crate) fn tile_link_delta(mode: usize, selection: AnchorSelection) -> Vec3 {
    if mode == HINGE_CHAIN_INDEX {
        Vec3::new(0.0, -PANEL_HEIGHT_M, 0.0)
    } else {
        let in_plane = anchor_plane_offset(selection.target_anchor())
            - anchor_plane_offset(selection.source_anchor());
        Vec3::new(in_plane.x, in_plane.y, selection.depth_mm * 0.001)
    }
}

/// Starts or reverses the spin envelope. A rising/holding spin starts falling; an
/// idle or falling spin starts rising and the dependent gains its
/// `AnchorPose`.
pub(crate) fn toggle_spin(
    spin: &mut Spin,
    tiles: &Query<(Entity, &AnchorTile, &Transform, Option<&AnchorPose>)>,
    commands: &mut Commands,
) {
    let spinning = matches!(spin.phase, Some(SpinPhase::Rising | SpinPhase::Holding));
    if spinning {
        spin.phase = Some(SpinPhase::Falling);
        spin.timer = 0.0;
        spin.fall_from = spin.angle;
        spin.fall_to = nearest_full_turn(spin.angle);
        spin.paused = false;
    } else {
        begin_spin(spin);
        // Every anchored tile gets a pose; each spins relative to its parent, so
        // the rotation composes cumulatively down the chain.
        for (tile, anchor, _, _) in tiles {
            if anchor.order >= 1 {
                commands.entity(tile).insert(AnchorPose::default());
            }
        }
    }
}

/// Arms the spin envelope at its rising edge. `angle` is left untouched, so a
/// spin resumed after a Spin→Anchor freeze continues from where it stopped rather
/// than snapping upright; a fresh spin starts from the default zero angle. The
/// caller owns whether the anchored tiles already carry a [`AnchorPose`] for
/// the spin to drive (`toggle_spin` inserts them; the mode morph leaves them in
/// place).
pub(crate) const fn begin_spin(spin: &mut Spin) {
    spin.phase = Some(SpinPhase::Rising);
    spin.timer = 0.0;
    spin.paused = false;
}

/// Freezes the spin where it is: stops accumulating angle but keeps the current
/// `angle`, so the chain holds its rotation instead of snapping back to upright.
/// [`drive_anchor_pose`] keeps writing the held rotation while it is non-identity.
pub(crate) const fn freeze_spin(spin: &mut Spin) {
    spin.phase = None;
    spin.paused = false;
}

/// Advances every capability animation by this frame's `dt`, in `Update` before
/// the panel rebuild. The marker rebuild, fan pose writer, and arrangement
/// component reconciliation then read one shared, already-advanced progress per
/// frame, so the marker stays glued to the pin instead of lagging the slide by a
/// frame. All animations freeze while a capability switch is in flight, so the
/// outgoing scene recedes rigidly.
pub(crate) fn advance_animations(
    time: Res<Time>,
    morph: Res<ModeMorph>,
    mut spin: ResMut<Spin>,
    mut hinge: ResMut<HingeChain>,
    mut transition: ResMut<AnchorTransition>,
) {
    if morph.active() {
        return;
    }
    let dt = time.delta_secs();
    if !spin.paused {
        advance_spin(&mut spin, dt);
    }
    if !hinge_paused(&hinge) {
        advance_hinge(&mut hinge, dt);
    }
    advance_anchor_transition(&mut transition, dt);
}

/// `P` or `Space` pauses or resumes the active capability's animation while it is
/// in flight — the spin envelope or the hinge fold ease; it does
/// nothing when idle or while a capability switch is running.
pub(crate) fn toggle_animation_pause(
    keyboard: Res<ButtonInput<KeyCode>>,
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    mut spin: ResMut<Spin>,
    mut hinge: ResMut<HingeChain>,
) {
    if morph.active() {
        return;
    }
    let pause_pressed =
        keyboard.just_pressed(KeyCode::KeyP) || keyboard.just_pressed(KeyCode::Space);
    if !pause_pressed {
        return;
    }
    if active.index == HINGE_CHAIN_INDEX {
        hinge.toggle_pause();
    } else if spin.phase.is_some() {
        spin.paused = !spin.paused;
    }
}

impl TitleChipActivation for Spin {
    /// Highlights the title-bar `Pause` word while the spin animation is
    /// frozen mid-motion.
    fn activation(&self) -> ControlActivation {
        if self.paused {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

/// Fan-mode writer of each anchored tile's [`AnchorPose`], composing both
/// animations from their already-advanced state each frame in
/// `PanelSystems::AnimateAnchorPose`, before the valence resolver, so every offset
/// lands that frame and the relation stays the sole transform writer.
///
/// - Spin (menu capability `2`): a spin about the pinned anchor; `Rising`/`Falling` share
///   `SPIN_EASE_SECS` so the spin accelerates from and decelerates back to rest. Depth is not
///   touched — it stays under manual `[`/`]` control.
/// - Anchor transition: an in-plane (plus depth) slide that eases each tile from its previous
///   resolved position to the newly selected anchor.
///
/// Every tile gets the same local pose; because the relations compose down the
/// chain, the spin and slide accumulate so each tile moves relative to its
/// parent. The poses are removed once both animations are idle.
pub(crate) fn drive_anchor_pose(
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    spin: Res<Spin>,
    transition: Res<AnchorTransition>,
    mut poses: Query<(Entity, &mut AnchorPose), With<AnchorTile>>,
    mut commands: Commands,
) {
    // `ModeMorph` owns every pose while it runs, and the arrangement hinge
    // systems own hinge-mode poses; this system drives fan modes only.
    if morph.active() || active.index == HINGE_CHAIN_INDEX {
        return;
    }
    let Ok(rotation) = Orientation::try_from(Quat::from_rotation_z(spin.angle)) else {
        return;
    };
    // Idle, no slide, and the held rotation has unwound to upright: drop the poses
    // so the tiles rest on their relation. A spin frozen at a non-identity angle
    // (Spin→Anchor) keeps its pose, so it stops where it was instead of resetting.
    if spin.phase.is_none() && !transition.active && rotation.into_inner().is_near_identity() {
        for (tile, _) in &poses {
            commands.entity(tile).remove::<AnchorPose>();
        }
        return;
    }
    let translation = Displacement::from(transition.current_offset());
    for (_, mut pose) in &mut poses {
        pose.rotation = rotation;
        pose.translation = translation;
    }
}

/// Advances the spin envelope by `dt`: accumulates the spin angle and steps the
/// phase.
fn advance_spin(spin: &mut Spin, dt: f32) {
    let Some(phase) = spin.phase else {
        return;
    };
    match phase {
        SpinPhase::Rising => {
            spin.timer += dt;
            let ease = smoothstep((spin.timer / SPIN_EASE_SECS).clamp(0.0, 1.0));
            spin.angle = (SPIN_RATE_RAD * ease).mul_add(dt, spin.angle);
            if spin.timer >= SPIN_EASE_SECS {
                spin.phase = Some(SpinPhase::Holding);
            }
        },
        SpinPhase::Holding => {
            spin.angle = SPIN_RATE_RAD.mul_add(dt, spin.angle);
        },
        SpinPhase::Falling => {
            spin.timer += dt;
            let ease = smoothstep((spin.timer / SPIN_EASE_SECS).clamp(0.0, 1.0));
            spin.angle = (spin.fall_to - spin.fall_from).mul_add(ease, spin.fall_from);
            if spin.timer >= SPIN_EASE_SECS {
                spin.phase = None;
            }
        },
    }
}

/// Advances the anchor-transition ease by `dt`, clearing `active` once the ease
/// completes. The slide offset is read back from the advanced state by
/// [`AnchorTransition::current_offset`].
fn advance_anchor_transition(transition: &mut AnchorTransition, dt: f32) {
    if !transition.active {
        return;
    }
    transition.timer += dt;
    if transition.timer >= ANCHOR_TRANSITION_SECS {
        transition.active = false;
    }
}

/// Plane-frame displacement (meters) of the dependent's resolved position when
/// the selection changes `from` → `to`: the difference of the followed target
/// anchor minus the difference of the following source anchor, plus the depth
/// change along the plane normal. Both panels share size and plane, so the
/// in-plane axes coincide. The world resolver shifts the resolved position by
/// the pose translation as `right·x + up·y + normal·z`.
///
/// `spin_angle` is the dependent's current Spin angle about the plane
/// normal. The resolver places the body as `target_point − R_spin · source_offset`,
/// so a source-anchor change swings the body by the source delta *rotated* by
/// `R_spin`; the target delta is not rotated (the target panel does not spin).
/// At `spin_angle == 0` this reduces to the plain in-plane difference.
fn anchor_transition_delta(from: AnchorSelection, to: AnchorSelection, spin_angle: f32) -> Vec3 {
    let target =
        anchor_plane_offset(to.target_anchor()) - anchor_plane_offset(from.target_anchor());
    let source =
        anchor_plane_offset(to.source_anchor()) - anchor_plane_offset(from.source_anchor());
    let in_plane = target - Vec2::from_angle(spin_angle).rotate(source);
    let depth = (to.depth_mm - from.depth_mm) * 0.001;
    Vec3::new(in_plane.x, in_plane.y, depth)
}

/// Plane-local position of `anchor` on a panel of the demo's size, in meters,
/// with x toward the plane's right and y toward its up.
fn anchor_plane_offset(anchor: Anchor) -> Vec2 {
    let (right, up) = match anchor {
        Anchor::TopLeft => (-0.5, 0.5),
        Anchor::TopCenter => (0.0, 0.5),
        Anchor::TopRight => (0.5, 0.5),
        Anchor::CenterLeft => (-0.5, 0.0),
        Anchor::Center => (0.0, 0.0),
        Anchor::CenterRight => (0.5, 0.0),
        Anchor::BottomLeft => (-0.5, -0.5),
        Anchor::BottomCenter => (0.0, -0.5),
        Anchor::BottomRight => (0.5, -0.5),
    };
    Vec2::new(right * PANEL_WIDTH, up * PANEL_HEIGHT) * 0.001
}

/// Nearest multiple of a full turn to `angle` — the upright orientation the
/// `Falling` ease settles on, so removing the pose leaves no rotation snap.
fn nearest_full_turn(angle: f32) -> f32 {
    use core::f32::consts::TAU;
    (angle / TAU).round() * TAU
}

pub(crate) fn cycle_anchor_selection(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    spin: Res<Spin>,
    mut selection: ResMut<AnchorSelection>,
    selected: Res<SelectedPanel>,
    mut transition: ResMut<AnchorTransition>,
    tiles: Query<(Entity, &AnchorTile)>,
    mut commands: Commands,
) {
    if morph.active() || active.index == HINGE_CHAIN_INDEX {
        return;
    }
    let mut next = *selection;
    let reset = keyboard.just_pressed(KeyCode::KeyR);
    // The arrow keys move the Tab-selected role's anchor on its 3×3 grid; they
    // do not wrap, so each press steps one cell and stops at the grid edges. The
    // selection is shared by every link, so a move re-chains the whole strip.
    if let Some(direction) = pressed_direction(&keyboard, reset)
        && !matches!(direction, AnchorDirection::Reset)
    {
        match *selected {
            SelectedPanel::Target => next.target_index = grid_move(next.target_index, direction),
            SelectedPanel::Anchored => next.source_index = grid_move(next.source_index, direction),
        }
    }
    let fast = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let depth_rate = DEPTH_RATE_MM_PER_SEC * if fast { DEPTH_FAST_MULTIPLIER } else { 1.0 };
    let depth_move = depth_rate * time.delta_secs();
    if keyboard.pressed(KeyCode::BracketLeft) {
        next.depth_mm -= depth_move;
    }
    if keyboard.pressed(KeyCode::BracketRight) {
        next.depth_mm += depth_move;
    }
    if reset {
        next = AnchorSelection::default();
    }
    if next == *selection {
        return;
    }

    let (by_order, live) = collect_tiles_by_order(&tiles);
    if live < 2 {
        return;
    }

    // Arrow keys and reset are discrete jumps that ease to the new position; a
    // held depth change ([/]) moves continuously and is not eased. The same local
    // slide offset is applied to every link, and the relations compose it down the
    // chain, so the tiles slide together to their new anchors.
    let eased = pressed_direction(&keyboard, reset).is_some();
    if eased {
        let delta = anchor_transition_delta(*selection, next, spin.angle);
        transition.from_offset = transition.current_offset() - delta;
        transition.timer = 0.0;
        transition.active = transition.from_offset.length_squared() > ANCHOR_TRANSITION_EPS_SQ;
        transition.from_source_index = selection.source_index;
        transition.from_target_index = selection.target_index;
    }

    *selection = next;
    // Re-anchor every dependent onto its predecessor with the new selection. The
    // info panel rebuilds in `reconcile_info_panel`; the tile trees (markers +
    // depth labels) rebuild in `reconcile_panels`, which eases the markers.
    for entity in by_order.iter().take(live).skip(1) {
        if eased && transition.active {
            commands.entity(*entity).insert(AnchorPose::default());
        }
    }
}

/// Keeps the fan's checked same-space attachments synchronized with the current
/// mode and anchor selection.
pub(crate) fn reconcile_fan_attachments(
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    selection: Res<AnchorSelection>,
    tiles: Query<(Entity, &AnchorTile, Option<&FanAttachmentState>)>,
    panel_entities: PanelEntityReader,
    mut commands: Commands,
) {
    if morph.active() {
        return;
    }
    let mut by_order = [Entity::PLACEHOLDER; ANCHOR_MAX_TILES];
    let mut states = [None; ANCHOR_MAX_TILES];
    for (entity, tile, state) in &tiles {
        if tile.order < ANCHOR_MAX_TILES {
            by_order[tile.order] = entity;
            states[tile.order] = state.copied();
        }
    }
    let fan_active = active.index != HINGE_CHAIN_INDEX;
    for order in 0..ANCHOR_MAX_TILES {
        let entity = by_order[order];
        if entity == Entity::PLACEHOLDER {
            continue;
        }
        let desired = (fan_active && order > 0).then(|| FanAttachmentState {
            target:   by_order[order - 1],
            authored: fan_attachment(*selection),
        });
        if states[order] == desired {
            continue;
        }
        let Some(source) = panel_entities.world(entity) else {
            continue;
        };
        match desired {
            Some(desired) => {
                let Some(target) = panel_entities.world(desired.target) else {
                    continue;
                };
                commands.attach_to_panel(source, target, desired.authored);
                commands.entity(entity).insert(desired);
            },
            None => {
                if states[order].is_some() {
                    commands.detach(source);
                    commands.entity(entity).remove::<FanAttachmentState>();
                }
            },
        }
    }
}

/// Collects the live anchor tiles into an array indexed by `order`, returning the
/// array and the tile count. Orders are contiguous from `0` (the root).
pub(crate) fn collect_tiles_by_order(
    tiles: &Query<(Entity, &AnchorTile)>,
) -> ([Entity; ANCHOR_MAX_TILES], usize) {
    let mut by_order = [Entity::PLACEHOLDER; ANCHOR_MAX_TILES];
    let mut live = 0;
    for (entity, tile) in tiles {
        if tile.order < ANCHOR_MAX_TILES {
            by_order[tile.order] = entity;
            live = live.max(tile.order + 1);
        }
    }
    (by_order, live)
}

/// `+`/`-` grow and shrink the tile chain from the bottom in every mode. A held
/// key auto-repeats after [`TILE_COUNT_REPEAT_DELAY`], then every
/// [`TILE_COUNT_REPEAT_RATE`]. The count is clamped in
/// [`AnchorChain::add_tile`]/[`AnchorChain::remove_tile`]; [`reconcile_anchor_chain`]
/// realizes it.
pub(crate) fn handle_anchor_count_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    morph: Res<ModeMorph>,
    mut chain: ResMut<AnchorChain>,
    mut cooldown: Local<f32>,
) {
    if morph.active() {
        return;
    }
    let plus = keyboard.pressed(KeyCode::Equal) || keyboard.pressed(KeyCode::NumpadAdd);
    let minus = keyboard.pressed(KeyCode::Minus) || keyboard.pressed(KeyCode::NumpadSubtract);
    let just_plus =
        keyboard.just_pressed(KeyCode::Equal) || keyboard.just_pressed(KeyCode::NumpadAdd);
    let just_minus =
        keyboard.just_pressed(KeyCode::Minus) || keyboard.just_pressed(KeyCode::NumpadSubtract);
    if just_plus {
        chain.add_tile();
        *cooldown = TILE_COUNT_REPEAT_DELAY;
    } else if just_minus {
        chain.remove_tile();
        *cooldown = TILE_COUNT_REPEAT_DELAY;
    } else if plus || minus {
        *cooldown -= time.delta_secs();
        if *cooldown <= 0.0 {
            if plus {
                chain.add_tile();
            } else {
                chain.remove_tile();
            }
            *cooldown = TILE_COUNT_REPEAT_RATE;
        }
    }
}

/// Makes the live tile chain match [`AnchorChain`] by spawning or despawning tiles
/// off the bottom end, in whichever mode is active. Growth anchors each new tile
/// onto the current last tile in fan modes, while hinge growth leaves relation
/// setup to [`crate::hinge::reconcile_hinge_arrangement`]. Fan growth adds a pose when an
/// animation is in flight; shrink despawns the surplus. Survivors keep their
/// current ownership — only the chain's tail changes — and the color wheel
/// recolor of every tile for the new count is handled by [`reconcile_panels`],
/// which rebuilds on a count change.
pub(crate) fn reconcile_anchor_chain(
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    chain: Res<AnchorChain>,
    selection: Res<AnchorSelection>,
    spin: Res<Spin>,
    transition: Res<AnchorTransition>,
    show: Res<ShowAnchorMarkers>,
    materials: Res<AnchorPanelMaterials>,
    tiles: Query<(Entity, &AnchorTile)>,
    mut commands: Commands,
) {
    if morph.active() {
        return;
    }
    let (mut by_order, live) = collect_tiles_by_order(&tiles);
    if live == 0 {
        return;
    }
    let desired = chain.count;
    if live == desired {
        return;
    }
    for entity in by_order.iter().take(live).skip(desired) {
        commands.entity(*entity).despawn();
    }
    let animating = spin.phase.is_some() || transition.active;
    for order in live..desired {
        let Some(tile) = spawn_tile(
            &mut commands,
            active.index,
            order,
            desired,
            TARGET_POSITION + tile_link_offset(active.index, *selection, order),
            Some(by_order[order - 1]),
            *selection,
            show.0,
            animating,
            &materials,
        ) else {
            return;
        };
        by_order[order] = tile;
    }
}

/// `Tab` toggles which panel the arrow keys move; `reconcile_info_panel` then
/// rebuilds the info panel so the selected panel's section title shows at full
/// strength.
pub(crate) fn cycle_selected_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut selected: ResMut<SelectedPanel>,
) {
    if keyboard.just_pressed(KeyCode::Tab) {
        *selected = selected.toggled();
    }
}

/// Sole rebuilder of the tile trees. Every link shares the eased source/target
/// marker positions and the per-link depth (the configured `depth_mm`), so the
/// whole chain rebuilds together whenever a marker moves at least
/// [`MARKER_STEP_MM`], the per-link depth moves at least [`DEPTH_LABEL_STEP_MM`],
/// the tile count changes (recoloring every tile around the wheel), or the marker
/// toggle flips. Each tile then renders its own markers (a source disc if it is
/// anchored, a target disc if it has a child) and its cumulative depth label.
pub(crate) fn reconcile_panels(
    active: Res<ActiveCapability>,
    selection: Res<AnchorSelection>,
    transition: Res<AnchorTransition>,
    show: Res<ShowAnchorMarkers>,
    chain: Res<AnchorChain>,
    tiles: Query<(Entity, &AnchorTile)>,
    mut last: Local<Option<(Vec2, Vec2, f32, usize, bool, usize)>>,
    mut commands: Commands,
) {
    let count = chain.count;
    let mode = active.index;
    let source_marker = eased_anchor_center(
        ANCHOR_POINTS[transition.from_source_index],
        selection.source_anchor(),
        *transition,
    );
    let target_marker = eased_anchor_center(
        ANCHOR_POINTS[transition.from_target_index],
        selection.target_anchor(),
        *transition,
    );
    let per_link_depth = selection.depth_mm;
    let changed = match *last {
        Some((last_source, last_target, last_depth, last_count, last_show, last_mode)) => {
            marker_moved(last_source, source_marker)
                || marker_moved(last_target, target_marker)
                || (last_depth - per_link_depth).abs() >= DEPTH_LABEL_STEP_MM
                || last_count != count
                || last_show != show.0
                || last_mode != mode
        },
        None => true,
    };
    if !changed {
        return;
    }
    *last = Some((
        source_marker,
        target_marker,
        per_link_depth,
        count,
        show.0,
        mode,
    ));
    for (entity, tile) in &tiles {
        let tree = if mode == HINGE_CHAIN_INDEX {
            build_hinge_number_tree(tile.order, count)
        } else {
            build_anchor_tile_tree(
                tile.order,
                count,
                source_marker,
                target_marker,
                per_link_depth,
                show.0,
            )
        };
        if let Err(error) = commands.set_tree(entity, tree) {
            error!("failed to replace anchor demo panel {entity:?} tree: {error}");
        }
    }
}

/// Marker center (mm) eased from the `from` anchor to the `to` anchor at the
/// transition's current progress; the resting `to` center when idle.
fn eased_anchor_center(from: Anchor, to: Anchor, transition: AnchorTransition) -> Vec2 {
    if transition.active {
        anchor_center(from).lerp(anchor_center(to), transition.progress())
    } else {
        anchor_center(to)
    }
}

/// Center (mm) of `anchor`'s point in the panel's full box: the marker disc is
/// centered here, so it sits exactly on the anchor and overflows the frame at
/// every edge anchor.
fn anchor_center(anchor: Anchor) -> Vec2 {
    let (align_x, align_y) = anchor_alignment(anchor);
    let x = match align_x {
        AlignX::Left => 0.0,
        AlignX::Center => PANEL_WIDTH * 0.5,
        AlignX::Right => PANEL_WIDTH,
    };
    let y = match align_y {
        AlignY::Top => 0.0,
        AlignY::Center => PANEL_HEIGHT * 0.5,
        AlignY::Bottom => PANEL_HEIGHT,
    };
    Vec2::new(x, y)
}

/// Whether the marker has moved at least [`MARKER_STEP_MM`] since the last build.
fn marker_moved(a: Vec2, b: Vec2) -> bool {
    a.distance_squared(b) >= MARKER_STEP_MM * MARKER_STEP_MM
}

/// The direction control for the key pressed this frame, if any. Arrow keys map
/// to their legend glyph; `reset` (R) maps to `Reset`.
fn pressed_direction(keyboard: &ButtonInput<KeyCode>, reset: bool) -> Option<AnchorDirection> {
    if keyboard.just_pressed(KeyCode::ArrowLeft) {
        Some(AnchorDirection::Left)
    } else if keyboard.just_pressed(KeyCode::ArrowRight) {
        Some(AnchorDirection::Right)
    } else if keyboard.just_pressed(KeyCode::ArrowUp) {
        Some(AnchorDirection::Top)
    } else if keyboard.just_pressed(KeyCode::ArrowDown) {
        Some(AnchorDirection::Bottom)
    } else if reset {
        Some(AnchorDirection::Reset)
    } else {
        None
    }
}

/// Tracks which Navigation controls glow yellow. A tapped control (arrow,
/// `R Reset`, or `Tab`) lights for [`LEGEND_TAP_GLOW_SECS`]; each held depth key
/// keeps its own timer refreshed while down. When a key is up its timer counts
/// down so the flash lingers [`LEGEND_GLOW_TAIL_SECS`] past release.
pub(crate) fn advance_legend_highlight(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut highlight: ResMut<LegendHighlight>,
) {
    let dt = time.delta_secs();
    let reset = keyboard.just_pressed(KeyCode::KeyR);
    if let Some(direction) = pressed_direction(&keyboard, reset) {
        highlight.direction = Some(direction);
        highlight.dir_timer = LEGEND_TAP_GLOW_SECS;
    } else if highlight.dir_timer > 0.0 {
        highlight.dir_timer -= dt;
    }
    if keyboard.just_pressed(KeyCode::Tab) {
        highlight.tab_timer = LEGEND_TAP_GLOW_SECS;
    } else if highlight.tab_timer > 0.0 {
        highlight.tab_timer -= dt;
    }
    refresh_or_decay(
        &mut highlight.depth_out_timer,
        keyboard.pressed(KeyCode::BracketLeft),
        dt,
    );
    refresh_or_decay(
        &mut highlight.depth_in_timer,
        keyboard.pressed(KeyCode::BracketRight),
        dt,
    );
}

/// Refreshes a held key's glow timer to the release tail while it is down, else
/// counts it down toward zero.
fn refresh_or_decay(timer: &mut f32, held: bool, dt: f32) {
    if held {
        *timer = LEGEND_GLOW_TAIL_SECS;
    } else if *timer > 0.0 {
        *timer -= dt;
    }
}

/// Draws a gradient gizmo for every link in the chain, from each parent tile's
/// target anchor to the child tile's source anchor, colored from the parent's
/// accent to the child's so the line reads in the same color wheel as the tiles.
/// Each endpoint eases between its old and new anchor in step with the slide.
pub(crate) fn draw_anchor_link(
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    selection: Res<AnchorSelection>,
    transition: Res<AnchorTransition>,
    geometry: PanelAnchorGeometryParam,
    tiles: Query<(Entity, &AnchorTile)>,
    mut gizmos: Gizmos,
) {
    // The link gizmos belong to the anchor fan; the hinge strip and the morph
    // between layouts draw none.
    if morph.active() || active.index == HINGE_CHAIN_INDEX {
        return;
    }
    let (by_order, live) = collect_tiles_by_order(&tiles);
    if live < 2 {
        return;
    }
    let progress = transition.progress();
    for order in 1..live {
        let (Ok(parent_geometry), Ok(child_geometry)) = (
            geometry.get(by_order[order - 1]),
            geometry.get(by_order[order]),
        ) else {
            continue;
        };
        let (Some(target_point), Some(source_point)) = (
            eased_anchor_world(
                &parent_geometry,
                ANCHOR_POINTS[transition.from_target_index],
                selection.target_anchor(),
                transition.active,
                progress,
            ),
            eased_anchor_world(
                &child_geometry,
                ANCHOR_POINTS[transition.from_source_index],
                selection.source_anchor(),
                transition.active,
                progress,
            ),
        ) else {
            continue;
        };
        if target_point.distance_squared(source_point)
            <= ANCHOR_LINK_MIN_LENGTH * ANCHOR_LINK_MIN_LENGTH
        {
            continue;
        }
        gizmos.line_gradient(
            target_point,
            source_point,
            hinge_accent(order - 1, live),
            hinge_accent(order, live),
        );
    }
}

/// World position of a panel's anchor-link endpoint, eased from the `from` anchor
/// to the `to` anchor at the transition `progress` so the gizmo endpoint tracks
/// the marker disc; the resting `to` point when no ease is in flight.
fn eased_anchor_world(
    geometry: &ResolvedPanelAnchorGeometry,
    from: Anchor,
    to: Anchor,
    active: bool,
    progress: f32,
) -> Option<Vec3> {
    let to_point = geometry.point(to).as_world()?;
    if !active {
        return Some(to_point);
    }
    let from_point = geometry.point(from).as_world()?;
    Some(from_point.lerp(to_point, progress))
}

/// The fan relation gluing a tile onto its parent: the selected source anchor is
/// pinned to the parent's target anchor with the current depth offset.
/// [`crate::hinge::reconcile_hinge_arrangement`] emits direct hinge relations instead.
fn fan_attachment(selection: AnchorSelection) -> PanelAttachment {
    PanelAttachment::new(selection.source_anchor(), selection.target_anchor())
        .with_offset(PanelAnchorOffset::ZERO.with_z(Mm(selection.depth_mm)))
}

fn build_anchor_panel(
    tree: LayoutTree,
    materials: &AnchorPanelMaterials,
) -> Result<DiegeticPanel, hana_diegetic::PanelBuildError> {
    DiegeticPanel::world()
        .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
        .font_unit(Unit::Millimeters)
        .material(materials.panel.clone())
        .text_material(materials.text.clone())
        .anchor(Anchor::Center)
        .with_tree(tree)
        .build()
}

/// One anchor tile's layout, bordered in its color-wheel accent. A non-root tile
/// (`order >= 1`) shows a source marker at `source_marker` where it attaches to
/// its parent; a tile with a child (`order + 1 < count`) shows a target marker at
/// `target_marker` where that child attaches. Non-root tiles also show their
/// cumulative depth — `order` times `per_link_depth` (the configured depth) —
/// pinned at bottom-center. The markers are omitted when `show_marker` is off
/// (`O` toggle).
fn build_anchor_tile_tree(
    order: usize,
    count: usize,
    source_center: Vec2,
    target_center: Vec2,
    per_link_depth: f32,
    show_marker: bool,
) -> LayoutTree {
    let accent = hinge_accent(order, count);
    let show_source = order >= 1;
    let show_target = order + 1 < count;
    let depth_mm = (order >= 1).then_some(tile_depth_mm(order, per_link_depth));
    let mut markers = Vec::new();
    if show_marker {
        if show_source {
            markers.push(marker_circle(source_center, accent));
        }
        if show_target {
            markers.push(marker_circle(target_center, accent));
        }
    }
    let mut overlay = El::overlay()
        .width(Sizing::GROW)
        .height(Sizing::GROW)
        .background(TILE_BACKGROUND)
        .border(Border::all(BORDER_WIDTH, accent));
    if !markers.is_empty() {
        overlay = overlay.draw(PanelDraw::shapes(markers).overflow(DrawOverflow::Visible));
    }
    let mut builder = LayoutBuilder::new(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT));
    builder.with(overlay, |builder| {
        if let Some(depth_mm) = depth_mm {
            depth_label_overlay(builder, accent, depth_mm);
        }
    });
    builder.build()
}

/// An anchor marker disc centered on `center` (mm in the panel's full box,
/// eased between anchors), in the tile's accent. Authored as an overflow-visible
/// [`PanelDraw`] shape so it spills past the frame at every edge anchor without
/// affecting layout or the panel border.
fn marker_circle(center: Vec2, accent: Color) -> PanelShape {
    PanelShape::Circle(
        PanelCircle::new(
            PanelPoint::new(PanelCoord::start(center.x), PanelCoord::start(center.y)),
            ANCHOR_MARKER_SIZE * 0.25,
        )
        .color(accent.with_alpha(MARKER_ALPHA)),
    )
}

/// The tile's cumulative depth label, pinned at bottom-center where the depth
/// offset is applied.
fn depth_label_overlay(builder: &mut LayoutBuilder, accent: Color, depth_mm: f32) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .alignment(AlignX::Center, AlignY::Bottom)
            .padding(Padding::new(0.0, 0.0, 0.0, DEPTH_LABEL_BOTTOM_PAD)),
        |builder| {
            builder.text((depth_label(depth_mm), depth_label_style(accent, depth_mm)));
        },
    );
}

fn depth_label(depth_mm: f32) -> String { format!("depth: {}", format_depth_mm(depth_mm)) }

fn depth_label_style(accent: Color, depth_mm: f32) -> TextStyle {
    let color = if depth_mm == 0.0 { BODY_COLOR } else { accent };
    TextStyle::new(DEPTH_LABEL_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

const fn anchor_alignment(anchor: Anchor) -> (AlignX, AlignY) {
    match anchor {
        Anchor::TopLeft => (AlignX::Left, AlignY::Top),
        Anchor::TopCenter => (AlignX::Center, AlignY::Top),
        Anchor::TopRight => (AlignX::Right, AlignY::Top),
        Anchor::CenterLeft => (AlignX::Left, AlignY::Center),
        Anchor::Center => (AlignX::Center, AlignY::Center),
        Anchor::CenterRight => (AlignX::Right, AlignY::Center),
        Anchor::BottomLeft => (AlignX::Left, AlignY::Bottom),
        Anchor::BottomCenter => (AlignX::Center, AlignY::Bottom),
        Anchor::BottomRight => (AlignX::Right, AlignY::Bottom),
    }
}

fn format_depth_mm(depth_mm: f32) -> String {
    if depth_mm == 0.0 {
        "0 mm".to_owned()
    } else {
        format!("{depth_mm:+.0} mm")
    }
}

fn tile_link_offset(mode: usize, selection: AnchorSelection, order: usize) -> Vec3 {
    let mut offset = Vec3::ZERO;
    let step = tile_link_delta(mode, selection);
    for _ in 0..order {
        offset += step;
    }
    offset
}

fn tile_depth_mm(order: usize, per_link_depth: f32) -> f32 {
    let mut depth = 0.0;
    for _ in 0..order {
        depth += per_link_depth;
    }
    depth
}
