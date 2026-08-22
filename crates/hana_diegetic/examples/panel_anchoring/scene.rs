//! Scene lifecycle: which capability is active, and the eased morph that carries
//! the one persistent tile set from one mode's layout to another's.
//!
//! Every capability shares a single set of [`AnchorTile`] entities, spawned once
//! at startup. Switching capability never despawns or respawns them: the anchor
//! and spin modes share the diagonal anchor fan, and the hinge chain commits the
//! same tiles to a `hana_valence` arrangement. A switch between the fan and the
//! strip arms a [`ModeMorph`]: the tiles keep their outgoing relation and their
//! [`AnchorPose`] eases from the live pose to the pose that reproduces the
//! incoming layout. When the ease completes, fan relations or arrangement
//! membership take ownership of the incoming layout, so the tiles glide between
//! layouts from any animation state.

use bevy::prelude::ButtonInput;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::KeyCode;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use fairy_dust::CameraHomeEntity;
use fairy_dust::FairyDustOrbitCam;
use hana_diegetic::DiegeticPanelCommands;
use hana_lagrange::ZoomToFit;
use hana_valence::AnchorPose;
use hana_valence::Displacement;
use hana_valence::Orientation;

use crate::anchor_demo::AnchorChain;
use crate::anchor_demo::AnchorSelection;
use crate::anchor_demo::AnchorTile;
use crate::anchor_demo::Autofit;
use crate::anchor_demo::ShowAnchorMarkers;
use crate::anchor_demo::Spin;
use crate::anchor_demo::begin_spin;
use crate::anchor_demo::build_tile_tree;
use crate::anchor_demo::collect_tiles_by_order;
use crate::anchor_demo::freeze_spin;
use crate::anchor_demo::spawn_anchor_scene;
use crate::anchor_demo::tile_link_delta;
use crate::anchor_demo::toggle_spin;
use crate::constants::*;
use crate::hinge::ChainArrangement;
use crate::hinge::FoldDirection;
use crate::hinge::FoldTravel;
use crate::hinge::HingeChain;
use crate::presentation::AnchorPanelMaterials;

/// Which capability scene is on screen; its menu entry highlights.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveCapability {
    pub(crate) index: usize,
}

impl Default for ActiveCapability {
    fn default() -> Self {
        Self {
            index: ANCHOR_INDEX,
        }
    }
}

/// Whether `index` lays the tiles out as the diagonal anchor fan (plain anchoring
/// and spin) rather than the hinge strip. Switching between two fan
/// capabilities keeps the layout, so no morph is needed; switching to or from the
/// hinge chain crosses layouts and arms a [`ModeMorph`].
const fn is_fan(index: usize) -> bool { index == ANCHOR_INDEX || index == SPIN_INDEX }

/// An in-flight eased morph between two mode layouts. While active, capability
/// input is locked and the per-capability animations freeze; [`advance_mode_morph`]
/// owns every tile's pose, easing it from the captured outgoing pose to the pose
/// that reproduces the incoming layout on the still-attached outgoing relation.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(crate) struct ModeMorph {
    /// `true` while the ease is running.
    active:     bool,
    /// Seconds into the ease.
    timer:      f32,
    /// Capability the tiles are morphing from; its relations stay attached until
    /// the ease completes.
    from_index: usize,
    /// Capability the tiles are morphing to; its relations are pointed in once the
    /// ease completes.
    to_index:   usize,
}

impl ModeMorph {
    pub(crate) const fn active(self) -> bool { self.active }

    /// Eased progress on `[0, 1]`.
    fn progress(self) -> f32 {
        crate::presentation::smoothstep((self.timer / MODE_MORPH_SECS).clamp(0.0, 1.0))
    }
}

/// The pose a tile carried when a morph began. The morph eases from this toward
/// the pose that reproduces the incoming layout, so a tile mid-spin or mid-fold
/// glides on from where it was rather than snapping to rest.
#[derive(Component, Clone, Copy)]
pub(crate) struct TileMorph {
    start: AnchorPose,
}

/// Spawns the one persistent tile set (in the anchor fan) and primes the active
/// capability. Called once at startup; capability switches morph these tiles
/// rather than respawning.
pub(crate) fn spawn_scene(
    commands: &mut Commands,
    selection: AnchorSelection,
    anchor_count: usize,
    show_marker: bool,
    materials: &AnchorPanelMaterials,
) {
    spawn_anchor_scene(commands, selection, anchor_count, show_marker, materials);
}

/// Number keys switch capabilities. Anchor (`1`) and Spin (`2`) share the
/// fan, so switching between them swaps mode in place: the active Spin
/// number toggles its envelope, the active Anchor number is inert. Switching to or
/// from the hinge chain (`3`) crosses layouts and arms a [`ModeMorph`]. While the
/// hinge chain is active, `A`/`C`, `F`/`B`, and `G`/`S` select the lit
/// arrangement, direction, and travel mode; `U`/`D`/`R` fold, mirror-fold, and unwrap
/// it. All input is locked while a morph is in flight.
pub(crate) fn handle_capability_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut active: ResMut<ActiveCapability>,
    selection: Res<AnchorSelection>,
    chain: Res<AnchorChain>,
    show: Res<ShowAnchorMarkers>,
    mut morph: ResMut<ModeMorph>,
    mut spin: ResMut<Spin>,
    mut hinge: ResMut<HingeChain>,
    tiles: Query<(Entity, &AnchorTile, &Transform, Option<&AnchorPose>)>,
    mut commands: Commands,
) {
    if morph.active {
        return;
    }
    if active.index == HINGE_CHAIN_INDEX {
        handle_hinge_selection(&keyboard, &mut hinge);
    }
    let Some(index) = pressed_capability(&keyboard) else {
        return;
    };
    if index == active.index {
        if index == SPIN_INDEX {
            toggle_spin(&mut spin, &tiles, &mut commands);
        }
        return;
    }
    if is_fan(active.index) && is_fan(index) {
        // Both lay the tiles out as the fan, so switch in place. Into Spin: start
        // the envelope immediately (no second press), resuming from any frozen
        // angle. Into Anchor: freeze the spin where it is rather than resetting, so
        // the chain stops at its current rotation.
        active.index = index;
        if index == SPIN_INDEX {
            toggle_spin(&mut spin, &tiles, &mut commands);
        } else {
            freeze_spin(&mut spin);
        }
        return;
    }
    arm_mode_morph(
        index,
        *selection,
        chain.count(),
        show.0,
        &tiles,
        &mut commands,
    );
    active.index = index;
    *spin = Spin::default();
    *hinge = HingeChain::default();
    morph.active = true;
    morph.timer = 0.0;
    morph.from_index = if index == HINGE_CHAIN_INDEX {
        // Leaving the fan: remember a fan index so the morph reads the fan link
        // delta as the outgoing layout.
        ANCHOR_INDEX
    } else {
        HINGE_CHAIN_INDEX
    };
    morph.to_index = index;
}

/// Reads the hinge fold-control keys while the chain is active (no movement, only
/// relighting the lit selection; `U`/`D`/`R` move the chain).
fn handle_hinge_selection(keyboard: &ButtonInput<KeyCode>, hinge: &mut HingeChain) {
    if keyboard.just_pressed(KeyCode::KeyA) {
        hinge.set_arrangement(ChainArrangement::Accordion);
    } else if keyboard.just_pressed(KeyCode::KeyC) {
        hinge.set_arrangement(ChainArrangement::Coil);
    } else if keyboard.just_pressed(KeyCode::KeyF) {
        hinge.set_direction(FoldDirection::Front);
    } else if keyboard.just_pressed(KeyCode::KeyB) {
        hinge.set_direction(FoldDirection::Back);
    } else if keyboard.just_pressed(KeyCode::KeyG) {
        hinge.set_travel(FoldTravel::Glide);
    } else if keyboard.just_pressed(KeyCode::KeyS) {
        hinge.set_travel(FoldTravel::Step);
    } else if keyboard.just_pressed(KeyCode::KeyU) {
        hinge.up();
    } else if keyboard.just_pressed(KeyCode::KeyD) {
        hinge.down();
    } else if keyboard.just_pressed(KeyCode::KeyR) {
        hinge.reset();
    }
}

/// Captures each tile's current pose as the morph start, rebuilds every tile tree
/// for the incoming mode, and seeds a [`AnchorPose`] on every anchored tile
/// so [`advance_mode_morph`] has a pose to drive. The outgoing relations stay
/// attached; only the trees and the pose change here.
fn arm_mode_morph(
    to_index: usize,
    selection: AnchorSelection,
    count: usize,
    show_marker: bool,
    tiles: &Query<(Entity, &AnchorTile, &Transform, Option<&AnchorPose>)>,
    commands: &mut Commands,
) {
    for (entity, tile, _, pose) in tiles {
        if let Err(error) = commands.set_tree(
            entity,
            build_tile_tree(to_index, tile.order, count, selection, show_marker),
        ) {
            error!("failed to replace anchor tile {entity:?} tree: {error}");
        }
        if tile.order == 0 {
            continue;
        }
        let start = pose.copied().unwrap_or_default();
        commands.entity(entity).insert((TileMorph { start }, start));
    }
}

/// The capability index for the number key pressed this frame, if any.
fn pressed_capability(keyboard: &ButtonInput<KeyCode>) -> Option<usize> {
    if keyboard.just_pressed(KeyCode::Digit1) {
        Some(ANCHOR_INDEX)
    } else if keyboard.just_pressed(KeyCode::Digit2) {
        Some(SPIN_INDEX)
    } else if keyboard.just_pressed(KeyCode::Digit3) {
        Some(HINGE_CHAIN_INDEX)
    } else {
        None
    }
}

/// Advances an in-flight mode morph. Each anchored tile's pose eases from its
/// captured start toward the pose that, on the still-attached outgoing relation,
/// reproduces the incoming mode's resting link delta — so the chain glides into
/// the incoming layout. When the ease completes, the relations are re-pointed to
/// the incoming mode (the tile is already sitting at that layout, so the swap is
/// invisible) and the morph poses are cleared. [`autofit_to_panels`] reframes the
/// new layout only if it overflows the viewport.
pub(crate) fn advance_mode_morph(
    time: Res<Time>,
    mut morph: ResMut<ModeMorph>,
    mut spin: ResMut<Spin>,
    selection: Res<AnchorSelection>,
    all_tiles: Query<(Entity, &AnchorTile)>,
    mut poses: Query<(Option<&TileMorph>, &mut AnchorPose), With<AnchorTile>>,
    mut commands: Commands,
) {
    if !morph.active {
        return;
    }
    morph.timer += time.delta_secs();
    let progress = morph.progress();
    // End pose (on the outgoing relation): no rotation, and a translation that
    // turns the outgoing link delta into the incoming one, so the chain lands on
    // the incoming layout.
    let end_translation = Displacement::from(
        tile_link_delta(morph.to_index, *selection) - tile_link_delta(morph.from_index, *selection),
    );
    for (tile_morph, mut pose) in &mut poses {
        let Some(tile_morph) = tile_morph else {
            continue;
        };
        if let Ok(rotation) = tile_morph
            .start
            .rotation
            .slerp(Orientation::default(), progress)
        {
            pose.rotation = rotation;
        }
        pose.translation = tile_morph.start.translation.lerp(end_translation, progress);
    }
    if morph.timer < MODE_MORPH_SECS {
        return;
    }
    finalize_mode_morph(morph.to_index, &all_tiles, &mut commands);
    if morph.to_index == SPIN_INDEX {
        // Landing on Spin from another mode starts the envelope immediately;
        // `finalize_mode_morph` already left each anchored tile a pose to drive.
        begin_spin(&mut spin);
    }
    morph.active = false;
}

/// Hands every tile from the morph to the incoming mode. Fan modes install their
/// authored relation here. Hinge mode leaves relation construction to
/// `reconcile_hinge_arrangement`, which commits arrangement membership after the
/// morph. Spin keeps an identity pose for its envelope; plain anchoring drops the
/// pose so each tile rests on its relation.
fn finalize_mode_morph(
    to_index: usize,
    all_tiles: &Query<(Entity, &AnchorTile)>,
    commands: &mut Commands,
) {
    let (by_order, live) = collect_tiles_by_order(all_tiles);
    for &entity in by_order.iter().take(live).skip(1) {
        let mut tile = commands.entity(entity);
        tile.remove::<TileMorph>();
        if to_index == HINGE_CHAIN_INDEX {
            continue;
        }
        if to_index == SPIN_INDEX {
            tile.insert(AnchorPose::default());
        } else {
            tile.remove::<AnchorPose>();
        }
    }
}

/// While [`Autofit`] is on, frames the union of every panel only once that union
/// overflows the viewport. The hidden home cube tracks the union of all panels
/// each frame; this projects the cube's eight corners to NDC and, when every
/// corner still lands inside the `[-1, 1]` view square, leaves the camera alone —
/// so the camera holds still while everything stays on screen instead of chasing
/// every small layout change. The frame a corner crosses the window edge it
/// re-issues an instant [`ZoomToFit`] (view angle preserved) with [`HOME_MARGIN`]
/// padding, pulling the whole extent back inside with slack so it does not
/// immediately re-trigger. With autofit off the fit never fires.
pub(crate) fn autofit_to_panels(
    autofit: Res<Autofit>,
    home: Option<Res<CameraHomeEntity>>,
    home_cube: Query<&Transform>,
    cameras: Query<(Entity, &Camera, &GlobalTransform), With<FairyDustOrbitCam>>,
    mut commands: Commands,
) {
    if !autofit.0 {
        return;
    }
    let (Some(home), Ok((camera_entity, camera, camera_global))) = (home, cameras.single()) else {
        return;
    };
    let Ok(cube) = home_cube.get(home.0) else {
        return;
    };
    if union_within_view(camera, camera_global, cube) {
        return;
    }
    // Instant (zero-duration) fit: sets the orbit target directly and completes
    // this frame, so re-issuing it while the camera damps toward the target never
    // strands a timed animation mid-ease (which would freeze the camera and lock
    // out manual orbit). The `OrbitCam` damps toward the target, so the recentering
    // stays smooth.
    commands.trigger(ZoomToFit::new(camera_entity, home.0).margin(HOME_MARGIN));
}

/// The eight corners of the unit home-cube mesh (a [`Cuboid`] of side `1`),
/// walked through the cube's [`Transform`] to recover the panel-union box in
/// world space.
const HOME_CUBE_CORNERS: [Vec3; 8] = [
    Vec3::new(-0.5, -0.5, -0.5),
    Vec3::new(0.5, -0.5, -0.5),
    Vec3::new(-0.5, 0.5, -0.5),
    Vec3::new(0.5, 0.5, -0.5),
    Vec3::new(-0.5, -0.5, 0.5),
    Vec3::new(0.5, -0.5, 0.5),
    Vec3::new(-0.5, 0.5, 0.5),
    Vec3::new(0.5, 0.5, 0.5),
];

/// Whether every corner of the panel-union box projects inside the camera's
/// viewport: each corner is in view only when its NDC `x`/`y` land within
/// `[-1, 1]` and its NDC `z` within `[0, 1]` (in front of the camera). The union
/// has overflowed the window the moment any one corner falls outside.
fn union_within_view(camera: &Camera, camera_global: &GlobalTransform, cube: &Transform) -> bool {
    HOME_CUBE_CORNERS.iter().all(|&corner| {
        let world = cube.transform_point(corner);
        camera
            .world_to_ndc(camera_global, world)
            .is_some_and(|ndc| {
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z)
            })
    })
}
