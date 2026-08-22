//! The hinge-chain capability (`3`): the shared tile set committed as a
//! `hana_valence` relationship controller that folds into a coplanar stack and
//! unwraps back (`R`), plus the local interaction and easing state for its controls.
//!
//! The tiles are the same persistent set the anchor fan uses; switching to the
//! hinge chain morphs them into the flat strip (see [`crate::scene`]). Tile `0`
//! becomes the arrangement controller, every later tile is a [`Member`], and
//! direct [`AnchoredTo`] plus [`Hinge`] values describe the physical chain.
//! `A`/`C` pick accordion vs coil, `F`/`B` the lean toward or away from the
//! camera, `G`/`S` glide vs step travel, and `U`/`D`/`R` fold, mirror-fold, and
//! unwrap. The link count is shared with the anchor fan ([`AnchorChain`]);
//! `+`/`-` grow and shrink it in every mode.

use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use fairy_dust::ControlActivation;
use fairy_dust::TitleChipActivation;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Border;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_valence::AnchorPose;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo;
use hana_valence::Angle;
use hana_valence::Arrangement;
use hana_valence::Displacement;
use hana_valence::Edge;
use hana_valence::Hinge;
use hana_valence::Member;

use crate::anchor_demo::AnchorChain;
use crate::anchor_demo::AnchorTile;
use crate::constants::*;
use crate::presentation::smoothstep;
use crate::scene::ActiveCapability;
use crate::scene::ModeMorph;

/// Direction a hinge ease is moving the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HingePhase {
    /// Easing toward the coplanar strip (`unwrap` → `1`).
    Unwrapping,
    /// Easing back to the folded chain (`unwrap` → `0`).
    Folding,
}

/// Which way the folded stack leans relative to the camera (`F`/`B`). Selecting
/// one only relights the control; the next `U`/`D`/`R` is what moves the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoldDirection {
    /// The stack leans toward the camera.
    Front,
    /// The stack leans away from the camera.
    Back,
}

/// The last fold action performed, kept lit until the next `U`/`D` (`R` relights
/// `Down` without latching itself). With a single fixed top, `Up` and `Down` fold
/// the chain toward tile `0` from opposite leans rather than pinning opposite ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoldAction {
    /// Fold leaning one way.
    Up,
    /// Fold leaning the mirror way.
    Down,
}

/// How far one `U`/`D` press carries the chain (`G`/`S`). Selecting one only
/// relights the control; the next `U`/`D` is what moves the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoldTravel {
    /// One press sweeps continuously through flat to the far collapse.
    Glide,
    /// One press advances one rest-stage (collapsed ↔ flat) and stops.
    Step,
}

/// Arrangement selected for the hinge chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChainArrangement {
    /// Adjacent hinges alternate fold direction.
    #[default]
    Accordion,
    /// Every hinge folds in the same direction.
    Coil,
}

/// The arrangement, direction, and action a fold is built from. The lit selection is
/// what the panel shows; the committed copy is what the current pose reflects, so
/// relighting a different selection while folded does not jump the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FoldSpec {
    arrangement: ChainArrangement,
    direction:   FoldDirection,
    action:      FoldAction,
}

/// State of the hinge-chain fold (capability `3`). The chain starts unwrapped as a
/// flat strip; `U`/`D` fold it into a coplanar stack and `R` unwraps it back. `lit`
/// is the panel selection set by `A`/`C`/`F`/`B`/`U`/`D`; `folded` is what the
/// committed arrangement component is built from, captured when a fold ease
/// begins. The link count lives in
/// [`AnchorChain`] — the one tile set shared with the anchor fan.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct HingeChain {
    /// The selection shown lit in the panel; only `U`/`D`/`R` act on it.
    lit:         FoldSpec,
    /// The fold the current pose reflects, captured when a fold ease begins.
    folded:      FoldSpec,
    /// Whether `U`/`D` sweep through flat to the far side or stop at each stage.
    travel:      FoldTravel,
    /// Current ease, or `None` when settled (fully folded or unwrapped).
    phase:       Option<HingePhase>,
    /// Whether the fold ease is frozen (`P`/`Space`); drives the title-bar chip.
    paused:      bool,
    /// Seconds into the current ease.
    timer:       f32,
    /// `0` fully folded .. `1` fully unwrapped; the rest value when settled.
    unwrap:      f32,
    /// `unwrap` when the current ease began.
    from_unwrap: f32,
    /// Set when an action is requested while not flat: the chain unwraps first,
    /// then commits `lit` and folds — the unfold-first / unfold-then-redo rule.
    refold:      bool,
}

impl Default for HingeChain {
    /// Launches as a flat unwrapped strip with `A`/`F`/`D` lit.
    fn default() -> Self {
        let lit = FoldSpec {
            arrangement: ChainArrangement::Accordion,
            direction:   FoldDirection::Front,
            action:      FoldAction::Down,
        };
        Self {
            lit,
            folded: lit,
            travel: FoldTravel::Step,
            phase: None,
            paused: false,
            timer: 0.0,
            unwrap: 1.0,
            from_unwrap: 1.0,
            refold: false,
        }
    }
}

impl HingeChain {
    /// The lit chain arrangement (`A`/`C`).
    pub(crate) const fn arrangement(self) -> ChainArrangement { self.lit.arrangement }

    /// The lit fold direction (`F`/`B`).
    pub(crate) const fn direction(self) -> FoldDirection { self.lit.direction }

    /// The lit fold action (`U`/`D`).
    pub(crate) const fn action(self) -> FoldAction { self.lit.action }

    /// The lit travel mode (`G`/`S`).
    pub(crate) const fn travel(self) -> FoldTravel { self.travel }

    /// Selects the lit travel mode (`G`/`S`); does not move the chain.
    pub(crate) const fn set_travel(&mut self, travel: FoldTravel) { self.travel = travel; }

    /// Selects the lit chain arrangement (`A`/`C`); does not move the chain.
    pub(crate) const fn set_arrangement(&mut self, arrangement: ChainArrangement) {
        self.lit.arrangement = arrangement;
    }

    /// Selects the lit fold direction (`F`/`B`); does not move the chain.
    pub(crate) const fn set_direction(&mut self, direction: FoldDirection) {
        self.lit.direction = direction;
    }

    /// Folds the chain leaning one way (`U`).
    pub(crate) fn up(&mut self) { self.act(FoldAction::Up); }

    /// Folds the chain leaning the mirror way (`D`).
    pub(crate) fn down(&mut self) { self.act(FoldAction::Down); }

    /// Freezes or resumes the fold ease while one is in flight (`P`/`Space`).
    pub(crate) const fn toggle_pause(&mut self) {
        if self.phase.is_some() {
            self.paused = !self.paused;
        }
    }

    /// Latches the action lit and moves the chain toward it. From the flat strip
    /// both travel modes fold straight toward `action`. Off the flat strip, `Glide`
    /// unwraps then refolds to the far side in one sweep (the unfold-first /
    /// unfold-then-redo rule), while `Step` advances a single rest-stage toward
    /// `action` and stops.
    fn act(&mut self, action: FoldAction) {
        self.lit.action = action;
        self.paused = false;
        if self.is_flat() {
            self.folded = self.lit;
            self.refold = false;
            self.ease_to(HingePhase::Folding);
            return;
        }
        match self.travel {
            FoldTravel::Glide => {
                self.refold = true;
                self.ease_to(HingePhase::Unwrapping);
            },
            FoldTravel::Step => {
                self.refold = false;
                if action == self.folded.action {
                    self.ease_to(HingePhase::Folding);
                } else {
                    self.ease_to(HingePhase::Unwrapping);
                }
            },
        }
    }

    /// Unwraps the chain back to the flat strip and relights `Down` (`R`).
    pub(crate) fn reset(&mut self) {
        self.lit.action = FoldAction::Down;
        self.refold = false;
        self.paused = false;
        self.ease_to(HingePhase::Unwrapping);
    }

    /// Whether the chain is settled flat (the spread-out strip), the only state a
    /// fold may start straight from.
    fn is_flat(self) -> bool { self.phase.is_none() && self.unwrap >= 1.0 }

    /// Starts an ease toward `phase`'s rest state from the current `unwrap`, ignored
    /// if that ease is already running.
    fn ease_to(&mut self, phase: HingePhase) {
        if self.phase == Some(phase) {
            return;
        }
        self.phase = Some(phase);
        self.from_unwrap = self.unwrap;
        self.timer = 0.0;
    }

    /// Arrangement type committed to the current fold pose.
    const fn committed_arrangement(self) -> ChainArrangement { self.folded.arrangement }

    /// Fold amount committed to the arrangement root.
    fn committed_fold(self) -> f32 { 1.0 - self.unwrap }

    /// Full-fold angle committed to the arrangement root.
    fn committed_lean(self) -> f32 {
        let action_sign = match self.folded.action {
            FoldAction::Up => 1.0,
            FoldAction::Down => -1.0,
        };
        HINGE_FOLD_ANGLE_RAD * fold_lean(self.folded.direction) * action_sign
    }
}

impl TitleChipActivation for HingeChain {
    /// Highlights the title-bar `Pause` word while a hinge fold is frozen.
    fn activation(&self) -> ControlActivation {
        if self.paused {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

/// Whether the stack leans toward the camera (`Front`) or away (`Back`), negating
/// the fold angle so the same creases close to the opposite side.
const fn fold_lean(direction: FoldDirection) -> f32 {
    match direction {
        FoldDirection::Back => 1.0,
        FoldDirection::Front => -1.0,
    }
}

/// Advances the hinge unwrap/fold ease by `dt`, lerping `unwrap` from its value
/// when the ease began toward the target rest state. When an unwrap that was queued
/// for a refold reaches the flat strip, it commits the lit selection and folds;
/// otherwise the phase clears.
pub(crate) fn advance_hinge(hinge: &mut HingeChain, dt: f32) {
    let Some(phase) = hinge.phase else {
        return;
    };
    hinge.timer += dt;
    let progress = smoothstep((hinge.timer / HINGE_EASE_SECS).clamp(0.0, 1.0));
    let target = match phase {
        HingePhase::Unwrapping => 1.0,
        HingePhase::Folding => 0.0,
    };
    hinge.unwrap = (target - hinge.from_unwrap).mul_add(progress, hinge.from_unwrap);
    if hinge.timer < HINGE_EASE_SECS {
        return;
    }
    hinge.unwrap = target;
    hinge.phase = None;
    if phase == HingePhase::Unwrapping && hinge.refold {
        hinge.refold = false;
        hinge.folded = hinge.lit;
        hinge.ease_to(HingePhase::Folding);
    }
}

/// Whether the hinge fold ease is currently frozen by `P`/`Space`.
pub(crate) const fn hinge_paused(hinge: &HingeChain) -> bool { hinge.paused }

/// Reconciles the shared tiles with direct arrangement membership and physical attachments.
///
/// While capability `3` is active and no [`ModeMorph`] owns the poses, tile
/// `0` is an [`Arrangement`] controller and each later live tile receives a
/// [`Member`], an [`AnchoredTo`] relation to its preceding tile, and a current
/// [`Hinge`] angle. Outside that state, these direct values are removed so the
/// fan and morph systems retain exclusive ownership of relations and poses.
pub(crate) fn reconcile_hinge_arrangement(
    active: Res<ActiveCapability>,
    morph: Res<ModeMorph>,
    hinge: Res<HingeChain>,
    chain: Res<AnchorChain>,
    tiles: Query<(Entity, &AnchorTile, Option<&Member>, Option<&AnchorPose>)>,
    mut commands: Commands,
) {
    let mut by_order = [Entity::PLACEHOLDER; ANCHOR_MAX_TILES];
    for (entity, tile, _, _) in &tiles {
        if tile.order < ANCHOR_MAX_TILES {
            by_order[tile.order] = entity;
        }
    }

    if morph.active() || active.index != HINGE_CHAIN_INDEX {
        for (entity, _, member, _) in &tiles {
            if member.is_some() {
                commands
                    .entity(entity)
                    .remove::<(Member, AnchoredTo, AnchorPose, Hinge)>();
            } else {
                commands.entity(entity).remove::<Hinge>();
            }
        }
        let root = by_order[0];
        if root != Entity::PLACEHOLDER {
            commands.entity(root).remove::<Arrangement>();
        }
        return;
    }

    let root = by_order[0];
    if root == Entity::PLACEHOLDER {
        return;
    }
    let fold = hinge.committed_fold();
    let lean = hinge.committed_lean();
    commands
        .entity(root)
        .insert(Arrangement)
        .remove::<(Hinge, Member, AnchoredTo, AnchorPose)>();

    for order in 1..chain.count() {
        let entity = by_order[order];
        let parent = by_order[order - 1];
        if entity == Entity::PLACEHOLDER || parent == Entity::PLACEHOLDER {
            continue;
        }
        let Ok((_, _, member, anchor_pose)) = tiles.get(entity) else {
            continue;
        };
        let sign = hinge_sign(order, hinge.committed_arrangement());
        let edge = Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        };
        let Ok(angle) = Angle::from_radians(lean * fold * sign) else {
            continue;
        };
        let Ok(hinge) = Hinge::try_new(edge, angle, angle, Displacement::default()) else {
            continue;
        };
        let mut tile_commands = commands.entity(entity);
        if member.is_some_and(|member| member.arrangement_entity != root) {
            tile_commands.remove::<Member>();
        }
        tile_commands.insert((
            Member::new(root),
            AnchoredTo::new(
                parent,
                AnchorSite::EdgeMidpoint(0),
                AnchorSite::EdgeMidpoint(2),
            ),
            hinge,
        ));
        if anchor_pose.is_none() {
            tile_commands.insert(AnchorPose::default());
        }
    }
}

const fn hinge_sign(order: usize, arrangement: ChainArrangement) -> f32 {
    match arrangement {
        ChainArrangement::Accordion if order.is_multiple_of(2) => -1.0,
        ChainArrangement::Accordion | ChainArrangement::Coil => 1.0,
    }
}

/// A hinge-chain tile's layout tree: its link number centered, accented per
/// `order`/`count` on the shared color wheel. Built at the shared tile size.
pub(crate) fn build_hinge_number_tree(order: usize, count: usize) -> LayoutTree {
    let accent = hinge_accent(order, count);
    let mut builder = LayoutBuilder::new(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT));
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(HINGE_BACKGROUND)
            .border(Border::all(HINGE_BORDER_WIDTH, accent))
            .alignment(AlignX::Center, AlignY::Center),
        |builder| {
            builder.text(((order + 1).to_string(), hinge_label_style(accent)));
        },
    );
    builder.build()
}

/// The link's accent, hue-walked around the color wheel by its index over the
/// current count, so a fuller chain spreads its accents across more of the wheel.
/// Shared with the anchor fan, whose tiles follow the same color wheel.
pub(crate) fn hinge_accent(order: usize, count: usize) -> Color {
    let index = steps_to_f32(order);
    let scale = steps_to_f32(count.max(1)).max(f32::EPSILON);
    let hue = (HINGE_ACCENT_HUE_START + 360.0 * index / scale).rem_euclid(360.0);
    Color::hsl(hue, HINGE_ACCENT_SATURATION, HINGE_ACCENT_LIGHTNESS)
}

fn steps_to_f32(count: usize) -> f32 {
    let mut steps = 0.0;
    for _ in 0..count {
        steps += 1.0;
    }
    steps
}

fn hinge_label_style(accent: Color) -> TextStyle {
    TextStyle::new(HINGE_LABEL_SIZE)
        .with_color(accent)
        .with_shadow_mode(GlyphShadowMode::None)
}

#[cfg(test)]
mod tests {
    use hana_valence::AnchorPose;
    use hana_valence::AnchoredTo;
    use hana_valence::Arrangement;
    use hana_valence::Hinge;
    use hana_valence::Member;
    use hana_valence::Members;

    use super::HingeChain;
    use super::reconcile_hinge_arrangement;
    use crate::anchor_demo::AnchorChain;
    use crate::anchor_demo::AnchorTile;
    use crate::constants::ANCHOR_INDEX;
    use crate::constants::HINGE_CHAIN_INDEX;
    use crate::scene::ActiveCapability;
    use crate::scene::ModeMorph;

    #[test]
    fn arrangement_membership_tracks_commit_switch_and_reentry_in_tile_order() {
        let mut chain = AnchorChain::default();
        chain.add_tile();
        let mut app = App::new();
        app.insert_resource(chain)
            .insert_resource(HingeChain::default())
            .insert_resource(ActiveCapability {
                index: HINGE_CHAIN_INDEX,
            })
            .insert_resource(ModeMorph::default())
            .add_systems(Update, reconcile_hinge_arrangement);

        let last = app.world_mut().spawn(AnchorTile { order: 2 }).id();
        let root = app.world_mut().spawn(AnchorTile { order: 0 }).id();
        let first = app.world_mut().spawn(AnchorTile { order: 1 }).id();
        app.update();

        assert!(app.world().get::<Arrangement>(root).is_some());
        assert_eq!(
            app.world()
                .get::<Member>(first)
                .map(|member| member.arrangement_entity),
            Some(root)
        );
        assert_eq!(
            app.world()
                .get::<Member>(last)
                .map(|member| member.arrangement_entity),
            Some(root)
        );
        assert_eq!(
            app.world()
                .get::<Members>(root)
                .map(|members| members.iter().collect::<Vec<_>>()),
            Some(vec![first, last])
        );
        assert_eq!(
            app.world().get::<AnchoredTo>(first).map(AnchoredTo::target),
            Some(root)
        );
        assert_eq!(
            app.world().get::<AnchoredTo>(last).map(AnchoredTo::target),
            Some(first)
        );
        assert!(app.world().get::<Hinge>(first).is_some());
        assert!(app.world().get::<Hinge>(last).is_some());
        let first_anchor_pose_tick = app
            .world()
            .entity(first)
            .get_ref::<AnchorPose>()
            .map(|anchor_pose| anchor_pose.last_changed());
        assert!(first_anchor_pose_tick.is_some());

        app.update();
        assert_eq!(
            app.world()
                .entity(first)
                .get_ref::<AnchorPose>()
                .map(|anchor_pose| anchor_pose.last_changed()),
            first_anchor_pose_tick,
            "an unchanged hinge arrangement must not reseed AnchorPose"
        );

        app.world_mut().resource_mut::<HingeChain>().down();
        app.update();
        assert!(app.world().get::<Arrangement>(root).is_some());
        assert!(app.world().get::<Hinge>(first).is_some());
        assert!(app.world().get::<Hinge>(last).is_some());

        app.world_mut().resource_mut::<ActiveCapability>().index = ANCHOR_INDEX;
        app.update();
        assert!(app.world().get::<Arrangement>(root).is_none());
        assert!(app.world().get::<Member>(first).is_none());
        assert!(app.world().get::<Member>(last).is_none());
        assert!(app.world().get::<Hinge>(first).is_none());
        assert!(app.world().get::<Hinge>(last).is_none());
        assert!(app.world().get::<AnchorPose>(first).is_none());
        assert!(app.world().get::<AnchorPose>(last).is_none());

        app.world_mut().resource_mut::<ActiveCapability>().index = HINGE_CHAIN_INDEX;
        app.update();
        assert!(app.world().get::<Arrangement>(root).is_some());
        assert_eq!(
            app.world()
                .get::<Members>(root)
                .map(|members| members.iter().collect::<Vec<_>>()),
            Some(vec![first, last])
        );
        assert!(
            app.world()
                .get::<AnchorPose>(first)
                .is_some_and(|anchor_pose| *anchor_pose == AnchorPose::default())
        );
        assert!(
            app.world()
                .get::<AnchorPose>(last)
                .is_some_and(|anchor_pose| *anchor_pose == AnchorPose::default())
        );
    }
}
