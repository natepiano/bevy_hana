use hana_kana::Easing;
use hana_kana::SequencePosition;
use hana_kana::SequenceStageId;
use hana_kana::SequenceStageSpan;
use hana_kana::SequenceTime;
use hana_kana::ToF32;

use super::CameraPose;
use super::normalized_camera_time;

/// One move's normalized interval, calculated from retained stage spans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CameraMoveInterval {
    pub(super) begin: f64,
    pub(super) end:   f64,
}

impl CameraMoveInterval {
    pub(super) fn from_span(span: SequenceStageSpan, total: SequenceTime) -> Self {
        Self {
            begin: normalized_camera_time(span.start(), total),
            end:   normalized_camera_time(span.end(), total),
        }
    }

    pub(super) const fn has_interior(self) -> bool { self.begin < self.end }
}

/// The captured target and authored easing for one camera move.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::animation::sequence) struct ResolvedCameraMoveEndpoint {
    pub(super) stage_id: SequenceStageId,
    pub(super) span:     SequenceStageSpan,
    pub(super) interval: CameraMoveInterval,
    pub(super) pose:     CameraPose,
    pub(super) easing:   Easing,
}

/// One move boundary, identified by the retained stage that owns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraMoveBoundary {
    Begin {
        stage_ordinal: usize,
        stage_id:      SequenceStageId,
    },
    End {
        stage_ordinal: usize,
        stage_id:      SequenceStageId,
    },
}

/// One boundary record stored in the same ordinal order playback receives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CameraBoundaryRecord {
    pub(super) boundary:         CameraMoveBoundary,
    pub(super) boundary_elapsed: SequenceTime,
    pub(super) position:         SequencePosition,
}

/// The immutable boundary records of one prepared camera sequence.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CameraBoundaryLedger {
    pub(super) records: Vec<CameraBoundaryRecord>,
}

impl From<&[ResolvedCameraMoveEndpoint]> for CameraBoundaryLedger {
    fn from(endpoints: &[ResolvedCameraMoveEndpoint]) -> Self {
        let mut records = Vec::with_capacity(endpoints.len() * 2);
        for (stage_ordinal, endpoint) in endpoints.iter().enumerate() {
            records.push(CameraBoundaryRecord {
                boundary:         CameraMoveBoundary::Begin {
                    stage_ordinal,
                    stage_id: endpoint.stage_id,
                },
                boundary_elapsed: endpoint.span.start(),
                position:         camera_boundary_position(endpoint.interval.begin),
            });
            records.push(CameraBoundaryRecord {
                boundary:         CameraMoveBoundary::End {
                    stage_ordinal,
                    stage_id: endpoint.stage_id,
                },
                boundary_elapsed: endpoint.span.end(),
                position:         camera_boundary_position(endpoint.interval.end),
            });
        }
        Self { records }
    }
}

impl CameraBoundaryLedger {
    pub(super) fn boundary_positions(&self) -> impl Iterator<Item = f64> + '_ {
        self.records
            .iter()
            .map(|record| f64::from(record.position.normalized()))
    }
}

/// Finite raw progress inside one positive-duration prepared move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::animation::sequence) struct CameraMoveProgress(f32);

impl CameraMoveProgress {
    pub(super) fn from_position(position: f64, interval: CameraMoveInterval) -> Self {
        let normalized = (position - interval.begin) / (interval.end - interval.begin);
        Self(normalized.clamp(0.0, 1.0).to_f32())
    }

    pub(super) const fn normalized(self) -> f32 { self.0 }
}

fn camera_boundary_position(normalized_position: f64) -> SequencePosition {
    match SequencePosition::try_new(normalized_position.to_f32()) {
        Ok(position) => position,
        Err(_) => SequencePosition::START,
    }
}

pub(super) const fn stage_id_from_boundary(boundary: CameraMoveBoundary) -> SequenceStageId {
    match boundary {
        CameraMoveBoundary::Begin { stage_id, .. } | CameraMoveBoundary::End { stage_id, .. } => {
            stage_id
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Vec3;

    use super::*;
    use crate::animation::sequence::CameraSequence;
    use crate::animation::sequence::support::*;

    #[test]
    fn prepared_boundaries_keep_duration_weighting_and_zero_moves_in_authored_order() {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::ZERO,
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ))
        .then(orbital_move(Vec3::ZERO, 0.1, 0.0, 1.0, Duration::ZERO))
        .then(orbital_move(
            Vec3::ZERO,
            0.2,
            0.0,
            1.0,
            Duration::from_secs(3),
        ));
        let playback = prepared_orbit(&sequence);

        let positions: Vec<_> = playback
            .boundary_ledger
            .records
            .iter()
            .map(|record| f64::from(record.position.normalized()))
            .collect();
        assert_eq!(positions, [0.0, 0.25, 0.25, 0.25, 0.25, 1.0]);
        assert_eq!(
            playback
                .boundary_ledger
                .records
                .iter()
                .map(|record| record.boundary)
                .collect::<Vec<_>>(),
            [
                CameraMoveBoundary::Begin {
                    stage_ordinal: 0,
                    stage_id:      playback.endpoints[0].stage_id,
                },
                CameraMoveBoundary::End {
                    stage_ordinal: 0,
                    stage_id:      playback.endpoints[0].stage_id,
                },
                CameraMoveBoundary::Begin {
                    stage_ordinal: 1,
                    stage_id:      playback.endpoints[1].stage_id,
                },
                CameraMoveBoundary::End {
                    stage_ordinal: 1,
                    stage_id:      playback.endpoints[1].stage_id,
                },
                CameraMoveBoundary::Begin {
                    stage_ordinal: 2,
                    stage_id:      playback.endpoints[2].stage_id,
                },
                CameraMoveBoundary::End {
                    stage_ordinal: 2,
                    stage_id:      playback.endpoints[2].stage_id,
                },
            ]
        );
    }

    #[test]
    fn duration_saturation_keeps_every_boundary_ordinal() {
        let sequence = CameraSequence::new(orbital_move(Vec3::ZERO, 0.0, 0.0, 1.0, Duration::MAX))
            .then(orbital_move(Vec3::ZERO, 0.1, 0.0, 1.0, Duration::ZERO))
            .then(orbital_move(Vec3::ZERO, 0.2, 0.0, 1.0, Duration::MAX));
        let playback = prepared_orbit(&sequence);

        assert_eq!(playback.boundary_ledger.records.len(), 6);
        assert_eq!(
            playback
                .boundary_ledger
                .boundary_positions()
                .collect::<Vec<_>>(),
            [0.0, 0.5, 0.5, 0.5, 0.5, 1.0]
        );
        assert_eq!(
            playback.playback.boundary_positions(),
            [0.0, 0.5, 0.5, 0.5, 0.5, 1.0]
        );
    }
}
