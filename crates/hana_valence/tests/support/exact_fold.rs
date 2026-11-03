//! Exact continuous collision checks for spawned cell-sheet folds.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::f64::consts::TAU;

use bevy::ecs::world::World;
use bevy::math::DQuat;
use bevy::math::DVec3;
use bevy::prelude::Entity;
use bevy::prelude::Transform;
use hana_kana::Easing;
use hana_kana::EasingSample;
use hana_kana::EasingSampler;
use hana_valence::AnchoredTo;
use hana_valence::FoldSegment;
use hana_valence::FoldSequence;
use hana_valence::Hinge;
use hana_valence::ResolvedAnchorGeometry;

/// The unfold is checked over this many even intervals, split again at every
/// authored member boundary.
const SAMPLES: u32 = 256;
/// How many times a turn halves before a pair counts as crossing.
const MAX_SPLITS: u32 = 20;
/// How far the axis reaches each way when measuring a quad's distance to it.
const AXIS_REACH: f64 = 1e4;

#[derive(Clone, Copy, Debug)]
struct Pose {
    rotation:    DQuat,
    translation: DVec3,
}

impl Pose {
    fn from_transform(transform: &Transform) -> Self {
        Self {
            rotation:    transform.rotation.as_dquat(),
            translation: transform.translation.as_dvec3(),
        }
    }

    fn apply(self, point: DVec3) -> DVec3 { self.rotation * point + self.translation }

    fn then(self, local: Self) -> Self {
        Self {
            rotation:    self.rotation * local.rotation,
            translation: self.apply(local.translation),
        }
    }
}

#[derive(Clone, Debug)]
struct Joint {
    parent: usize,
    target: DVec3,
    source: DVec3,
    offset: DVec3,
    axis:   DVec3,
    pivot:  DVec3,
    base:   f64,
    folded: f64,
    begin:  f64,
    end:    f64,
    from:   f64,
    to:     f64,
    easing: Easing,
}

impl Joint {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the normalized clock fraction is bounded to zero through one"
    )]
    fn angle(&self, clock: f64) -> f64 {
        let raw = if clock <= self.begin {
            0.0_f32
        } else if clock >= self.end || self.end <= self.begin {
            1.0
        } else {
            ((clock - self.begin) / (self.end - self.begin)) as f32
        };
        let eased = match EasingSampler.sample(&self.easing, raw) {
            EasingSample::Eased(value) => f64::from(value),
            EasingSample::NonFinite => panic!("spawned fold timing produced non-finite easing"),
        };
        let fraction = self.from.mul_add(1.0 - eased, self.to * eased);
        self.base.mul_add(1.0 - fraction, self.folded * fraction)
    }

    fn moves_at(&self, clock: f64) -> bool { self.begin < clock && clock < self.end }

    fn local(&self, angle: f64) -> Pose {
        let rotation = DQuat::from_axis_angle(self.axis, angle);
        let pivot_turn = DQuat::from_axis_angle(self.axis, angle - self.base);
        Pose {
            rotation,
            translation: self.target + self.offset + self.pivot
                - pivot_turn * self.pivot
                - rotation * self.source,
        }
    }
}

#[derive(Clone, Debug)]
struct ModelSquare {
    rest:   Pose,
    joint:  Option<Joint>,
    branch: Vec<usize>,
}

/// A pure fold model sourced from the entities and components an arrangement
/// actually spawned.
pub(super) struct FoldModel {
    squares:     Vec<ModelSquare>,
    order:       Vec<usize>,
    letters:     Vec<Vec<usize>>,
    half_extent: f64,
}

impl FoldModel {
    pub(super) fn from_spawned(
        world: &World,
        squares: &[Entity],
        controllers: &[Entity],
        half_extent: f64,
    ) -> Self {
        let indices = squares
            .iter()
            .copied()
            .enumerate()
            .map(|(index, entity)| (entity, index))
            .collect::<HashMap<_, _>>();
        let (mut model_squares, letters) = spawned_squares(world, squares, controllers, &indices);
        let order = parent_first_order(&model_squares);
        attach_branches(&mut model_squares);
        Self {
            squares: model_squares,
            order,
            letters,
            half_extent,
        }
    }

    fn poses(&self, clock: f64) -> Vec<Pose> {
        let mut poses = self
            .squares
            .iter()
            .map(|square| square.rest)
            .collect::<Vec<_>>();
        for &cell in &self.order {
            if let Some(joint) = &self.squares[cell].joint {
                poses[cell] = poses[joint.parent].then(joint.local(joint.angle(clock)));
            }
        }
        poses
    }

    fn axis(&self, hinge: usize, parent: Pose) -> Axis {
        let joint = self.squares[hinge]
            .joint
            .as_ref()
            .expect("only a hinged square has an axis");
        Axis {
            point:     parent.apply(joint.target + joint.offset + joint.pivot),
            direction: parent.rotation * joint.axis,
        }
    }

    fn quad(&self, pose: Pose) -> Quad {
        let half = self.half_extent;
        [(-half, half), (half, half), (half, -half), (-half, -half)]
            .map(|(x, y)| pose.apply(DVec3::new(x, y, 0.0)))
    }

    /// The first pair of squares that may cross from `from` to `to`, within
    /// which each letter turns at most one hinge.
    pub(super) fn crossing(&self, from: f64, to: f64) -> Option<(usize, usize)> {
        let poses = self.poses(from);
        let quads = poses
            .iter()
            .map(|&pose| self.quad(pose))
            .collect::<Vec<_>>();
        let boxes = quads.iter().map(|quad| bounds(quad)).collect::<Vec<_>>();
        let branches = self.active_branches(&poses, from, to);
        let owner = branch_owners(&branches, quads.len());
        for (index, branch) in branches.iter().enumerate() {
            let corners = branch
                .squares
                .iter()
                .flat_map(|&cell| quads[cell])
                .collect::<Vec<_>>();
            let whole = branch.axis.polar(&corners);
            let reach = grown(bounds(&corners), branch.turn.abs() * whole.outer);
            for still in 0..quads.len() {
                let nearby = match owner[still] {
                    Some(other) if other > index => {
                        overlaps(swept_box(&branches[other], still, &quads, &boxes), reach)
                    },
                    None => overlaps(boxes[still], reach),
                    _ => false,
                };
                if !nearby {
                    continue;
                }
                match owner[still] {
                    Some(other) if other == index => {},
                    Some(other) => {
                        if other < index {
                            continue;
                        }
                        let other_branch = &branches[other];
                        for &cell in &branch.squares {
                            if overlaps(
                                swept_box(branch, cell, &quads, &boxes),
                                swept_box(other_branch, still, &quads, &boxes),
                            ) && !both_clear(
                                self,
                                [(branch, &quads[cell]), (other_branch, &quads[still])],
                                from,
                                (from, to),
                                0,
                            ) {
                                return Some((cell, still));
                            }
                        }
                    },
                    None => {
                        let polar = branch.axis.polar(&quads[still]);
                        let inner = branch.axis.distance(&quads[still]);
                        if sweep_clears(&whole, &polar, inner, branch.turn) {
                            continue;
                        }
                        for &cell in &branch.squares {
                            if overlaps(swept_box(branch, cell, &quads, &boxes), boxes[still])
                                && !sweep_clears(
                                    &branch.axis.polar(&quads[cell]),
                                    &polar,
                                    inner,
                                    branch.turn,
                                )
                                && !turn_clears(
                                    &quads[cell],
                                    &quads[still],
                                    branch.axis,
                                    (0.0, branch.turn),
                                    0,
                                )
                            {
                                return Some((cell, still));
                            }
                        }
                    },
                }
            }
        }
        None
    }

    fn active_branches(&self, poses: &[Pose], from: f64, to: f64) -> Vec<Branch> {
        let middle = f64::midpoint(from, to);
        self.letters
            .iter()
            .flat_map(|turns| {
                turns.iter().copied().filter_map(|hinge| {
                    self.squares[hinge]
                        .joint
                        .as_ref()
                        .filter(|joint| joint.moves_at(middle))
                        .map(|joint| Branch {
                            hinge,
                            squares: self.squares[hinge].branch.clone(),
                            axis: self.axis(hinge, poses[joint.parent]),
                            turn: joint.angle(to) - joint.angle(from),
                        })
                })
            })
            .collect()
    }

    pub(super) fn assert_no_crossings(&self, fixture: &str) {
        let mut stops = (0..=SAMPLES)
            .map(|sample| f64::from(sample) / f64::from(SAMPLES))
            .collect::<Vec<_>>();
        for square in &self.squares {
            if let Some(joint) = &square.joint {
                stops.extend([joint.begin, joint.end]);
            }
        }
        stops.sort_by(f64::total_cmp);
        stops.dedup();
        for stop in stops.windows(2) {
            let (from, to) = (stop[0], stop[1]);
            if let Some((first, second)) = self.crossing(from, to) {
                panic!("{fixture}: squares {first} and {second} may cross between {from} and {to}");
            }
        }
    }
}

fn spawned_squares(
    world: &World,
    squares: &[Entity],
    controllers: &[Entity],
    indices: &HashMap<Entity, usize>,
) -> (Vec<ModelSquare>, Vec<Vec<usize>>) {
    let mut modeled = squares
        .iter()
        .copied()
        .map(|entity| ModelSquare {
            rest:   Pose::from_transform(
                world
                    .get::<Transform>(entity)
                    .expect("every modeled square has a transform"),
            ),
            joint:  None,
            branch: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut letters = Vec::with_capacity(controllers.len());
    for &controller in controllers {
        let sequence = world
            .get::<FoldSequence>(controller)
            .expect("every modeled controller carries its fold sequence");
        let mut turns = Vec::with_capacity(sequence.tracks().len());
        for track in sequence.tracks() {
            let entity = track.member_entity();
            let &index = indices
                .get(&entity)
                .expect("every sequence member is a modeled square");
            let [segment] = track.segments() else {
                panic!("each strip member should move in exactly one segment");
            };
            modeled[index].joint = Some(spawned_joint(world, entity, indices, segment));
            turns.push(index);
        }
        turns.sort_by(|first, second| {
            let begin = |index: usize| {
                modeled[index]
                    .joint
                    .as_ref()
                    .expect("a sequence member has a joint")
                    .begin
            };
            begin(*first).total_cmp(&begin(*second))
        });
        letters.push(turns);
    }
    (modeled, letters)
}

fn spawned_joint(
    world: &World,
    entity: Entity,
    indices: &HashMap<Entity, usize>,
    segment: &FoldSegment,
) -> Joint {
    let relation = world
        .get::<AnchoredTo>(entity)
        .expect("every sequence member is anchored");
    let hinge = world
        .get::<Hinge>(entity)
        .expect("every sequence member carries its applied hinge");
    let geometry = world
        .get::<ResolvedAnchorGeometry>(entity)
        .expect("every sequence member carries cell geometry");
    let target_geometry = world
        .get::<ResolvedAnchorGeometry>(relation.target())
        .expect("every hinge target carries cell geometry");
    Joint {
        parent: indices[&relation.target()],
        target: target_geometry
            .frame(relation.target_anchor())
            .expect("the target anchor is in cell geometry")
            .position()
            .into_inner()
            .as_dvec3(),
        source: geometry
            .frame(relation.source_anchor())
            .expect("the source anchor is in cell geometry")
            .position()
            .into_inner()
            .as_dvec3(),
        offset: relation.offset().into_inner().as_dvec3(),
        axis:   hinge
            .edge()
            .axis(geometry)
            .expect("a cell hinge edge has an axis")
            .as_dvec3(),
        pivot:  hinge.pivot_offset().into_inner().as_dvec3(),
        base:   f64::from(hinge.base_angle().radians()),
        folded: f64::from(hinge.folded_angle().radians()),
        begin:  segment.normalized_begin(),
        end:    segment.normalized_end(),
        from:   f64::from(segment.from().fraction()),
        to:     f64::from(segment.to().fraction()),
        easing: segment.timing().easing.clone(),
    }
}

fn parent_first_order(squares: &[ModelSquare]) -> Vec<usize> {
    let mut order = (0..squares.len())
        .filter(|&index| squares[index].joint.is_none())
        .collect::<Vec<_>>();
    while order.len() < squares.len() {
        let before = order.len();
        for (index, square) in squares.iter().enumerate() {
            if order.contains(&index) {
                continue;
            }
            let parent = square
                .joint
                .as_ref()
                .expect("a non-root square has a joint")
                .parent;
            if order.contains(&parent) {
                order.push(index);
            }
        }
        assert!(
            order.len() > before,
            "spawned hinge relations must be acyclic"
        );
    }
    order
}

fn attach_branches(squares: &mut [ModelSquare]) {
    for hinge in 0..squares.len() {
        if squares[hinge].joint.is_none() {
            continue;
        }
        squares[hinge].branch = (0..squares.len())
            .filter(|&square| is_descendant(square, hinge, squares))
            .collect();
    }
}

const fn is_descendant(square: usize, ancestor: usize, squares: &[ModelSquare]) -> bool {
    let mut current = square;
    loop {
        if current == ancestor {
            return true;
        }
        let Some(joint) = &squares[current].joint else {
            return false;
        };
        current = joint.parent;
    }
}

type Quad = [DVec3; 4];

fn edges(quad: &Quad) -> [(DVec3, DVec3); 4] {
    [0, 1, 2, 3].map(|index| (quad[index], quad[(index + 1) % 4]))
}

fn normal(quad: &Quad) -> DVec3 { (quad[1] - quad[0]).cross(quad[3] - quad[0]).normalize() }

fn inside(quad: &Quad, normal: DVec3, point: DVec3) -> bool {
    edges(quad)
        .iter()
        .all(|&(start, end)| (end - start).cross(point - start).dot(normal) >= 0.0)
}

fn pierces(quad: &Quad, start: DVec3, end: DVec3) -> bool {
    let normal = normal(quad);
    let from_start = (start - quad[0]).dot(normal);
    let from_end = (end - quad[0]).dot(normal);
    from_start * from_end < 0.0
        && inside(
            quad,
            normal,
            start + (end - start) * (from_start / (from_start - from_end)),
        )
}

fn segment_distance(first: (DVec3, DVec3), second: (DVec3, DVec3)) -> f64 {
    let along_first = first.1 - first.0;
    let along_second = second.1 - second.0;
    let between = first.0 - second.0;
    let first_length = along_first.length_squared();
    let second_length = along_second.length_squared();
    let shared = along_first.dot(along_second);
    let first_offset = along_first.dot(between);
    let second_offset = along_second.dot(between);
    let denominator = first_length.mul_add(second_length, -shared * shared);
    let mut s = if denominator > f64::EPSILON {
        (shared.mul_add(second_offset, -first_offset * second_length) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = shared.mul_add(s, second_offset) / second_length;
    if t < 0.0 {
        t = 0.0;
        s = (-first_offset / first_length).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((shared - first_offset) / first_length).clamp(0.0, 1.0);
    }
    ((first.0 + along_first * s) - (second.0 + along_second * t)).length()
}

fn vertex_face(quad: &Quad, point: DVec3) -> f64 {
    let normal = normal(quad);
    let height = (point - quad[0]).dot(normal);
    if inside(quad, normal, point - normal * height) {
        height.abs()
    } else {
        f64::INFINITY
    }
}

fn quad_distance(first: &Quad, second: &Quad) -> f64 {
    if edges(first)
        .iter()
        .any(|&(start, end)| pierces(second, start, end))
        || edges(second)
            .iter()
            .any(|&(start, end)| pierces(first, start, end))
    {
        return 0.0;
    }
    let mut least = f64::INFINITY;
    for first_edge in edges(first) {
        for second_edge in edges(second) {
            least = least.min(segment_distance(first_edge, second_edge));
        }
    }
    for &corner in first {
        least = least.min(vertex_face(second, corner));
    }
    for &corner in second {
        least = least.min(vertex_face(first, corner));
    }
    least
}

fn bounds(points: &[DVec3]) -> (DVec3, DVec3) {
    points.iter().fold(
        (DVec3::INFINITY, DVec3::NEG_INFINITY),
        |(low, high), &point| (low.min(point), high.max(point)),
    )
}

fn grown((low, high): (DVec3, DVec3), by: f64) -> (DVec3, DVec3) { (low - by, high + by) }

fn overlaps(first: (DVec3, DVec3), second: (DVec3, DVec3)) -> bool {
    first.0.cmple(second.1).all() && second.0.cmple(first.1).all()
}

#[derive(Clone, Copy, Debug)]
struct Axis {
    point:     DVec3,
    direction: DVec3,
}

impl Axis {
    fn radius(self, point: DVec3) -> f64 {
        (point - self.point)
            .reject_from_normalized(self.direction)
            .length()
    }

    fn turned(self, quad: &Quad, angle: f64) -> Quad {
        let rotation = DQuat::from_axis_angle(self.direction, angle);
        quad.map(|corner| self.point + rotation * (corner - self.point))
    }

    fn distance(self, quad: &Quad) -> f64 {
        let far = self.direction * AXIS_REACH;
        let line = (self.point - far, self.point + far);
        if pierces(quad, line.0, line.1) {
            return 0.0;
        }
        edges(quad)
            .iter()
            .map(|&edge| segment_distance(line, edge))
            .fold(f64::INFINITY, f64::min)
    }

    fn polar(self, points: &[DVec3]) -> Polar {
        let across = self.direction.any_orthonormal_vector();
        let up = self.direction.cross(across);
        let mut axial = (f64::INFINITY, f64::NEG_INFINITY);
        let mut outer = 0.0_f64;
        let mut turned = (f64::INFINITY, f64::NEG_INFINITY);
        let mut reference = None;
        let mut on_axis = false;
        for &point in points {
            let offset = point - self.point;
            let along = offset.dot(self.direction);
            axial = (axial.0.min(along), axial.1.max(along));
            let (x, y) = (offset.dot(across), offset.dot(up));
            let radius = x.hypot(y);
            outer = outer.max(radius);
            on_axis |= radius < f64::EPSILON;
            let angle = y.atan2(x);
            let from = *reference.get_or_insert(angle);
            let relative = (angle - from + PI).rem_euclid(TAU) - PI;
            turned = (turned.0.min(relative), turned.1.max(relative));
        }
        let arc = reference
            .filter(|_| !on_axis && turned.1 - turned.0 < PI)
            .map(|from| (from + turned.0, turned.1 - turned.0));
        Polar { axial, outer, arc }
    }
}

struct Polar {
    axial: (f64, f64),
    outer: f64,
    arc:   Option<(f64, f64)>,
}

fn sweep_clears(moving: &Polar, still: &Polar, inner: f64, turn: f64) -> bool {
    if moving.axial.1 < still.axial.0 || still.axial.1 < moving.axial.0 || moving.outer < inner {
        return true;
    }
    let (Some((moving_start, moving_span)), Some((still_start, still_span))) =
        (moving.arc, still.arc)
    else {
        return false;
    };
    let start = moving_start + turn.min(0.0);
    let span = moving_span + turn.abs();
    let ahead = (still_start - start).rem_euclid(TAU);
    span < TAU && ahead > span && ahead + still_span < TAU
}

fn turn_clears(
    mover: &Quad,
    still: &Quad,
    axis: Axis,
    (from, to): (f64, f64),
    splits: u32,
) -> bool {
    let apart = quad_distance(&axis.turned(mover, from), still);
    if apart == 0.0 {
        return false;
    }
    let radius = mover
        .iter()
        .map(|&corner| axis.radius(corner))
        .fold(0.0, f64::max);
    if apart > (to - from).abs() * radius {
        return true;
    }
    if splits == MAX_SPLITS {
        return false;
    }
    let middle = f64::midpoint(from, to);
    turn_clears(mover, still, axis, (from, middle), splits + 1)
        && turn_clears(mover, still, axis, (middle, to), splits + 1)
}

struct Branch {
    hinge:   usize,
    squares: Vec<usize>,
    axis:    Axis,
    turn:    f64,
}

impl Branch {
    fn angle(&self, model: &FoldModel, clock: f64, from: f64) -> f64 {
        model.squares[self.hinge]
            .joint
            .as_ref()
            .map_or(0.0, |joint| joint.angle(clock) - joint.angle(from))
    }
}

fn both_clear(
    model: &FoldModel,
    pair: [(&Branch, &Quad); 2],
    start: f64,
    (from, to): (f64, f64),
    splits: u32,
) -> bool {
    let at = |(branch, quad): (&Branch, &Quad), clock: f64| {
        branch.axis.turned(quad, branch.angle(model, clock, start))
    };
    let apart = quad_distance(&at(pair[0], from), &at(pair[1], from));
    if apart == 0.0 {
        return false;
    }
    let reach = pair
        .iter()
        .map(|&(branch, quad)| {
            let radius = quad
                .iter()
                .map(|&corner| branch.axis.radius(corner))
                .fold(0.0, f64::max);
            (branch.angle(model, to, start) - branch.angle(model, from, start)).abs() * radius
        })
        .sum::<f64>();
    if apart > reach {
        return true;
    }
    if splits == MAX_SPLITS {
        return false;
    }
    let middle = f64::midpoint(from, to);
    both_clear(model, pair, start, (from, middle), splits + 1)
        && both_clear(model, pair, start, (middle, to), splits + 1)
}

fn swept_box(
    branch: &Branch,
    cell: usize,
    quads: &[Quad],
    boxes: &[(DVec3, DVec3)],
) -> (DVec3, DVec3) {
    let radius = quads[cell]
        .iter()
        .map(|&corner| branch.axis.radius(corner))
        .fold(0.0, f64::max);
    grown(boxes[cell], branch.turn.abs() * radius)
}

fn branch_owners(branches: &[Branch], count: usize) -> Vec<Option<usize>> {
    let mut owner = vec![None; count];
    for (index, branch) in branches.iter().enumerate() {
        for &cell in &branch.squares {
            owner[cell] = Some(index);
        }
    }
    owner
}
