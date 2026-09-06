use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::ReflectComponent;
use bevy_math::Dir3;
use bevy_platform::collections::HashMap;
use bevy_reflect::Reflect;
use hana_kana::Orientation;
use hana_kana::Position;
use thiserror::Error;

/// A named attachment site emitted by an anchor-geometry provider.
///
/// `Vertex` and `EdgeMidpoint` indices use the provider's own stable ordering;
/// Valence assigns no cross-provider meaning to a particular number. `Center`
/// is the one whole-member site rather than a numbered vertex or edge.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Reflect)]
#[non_exhaustive]
pub enum AnchorSite {
    /// A vertex in the provider-defined ordering.
    Vertex(u32),
    /// An edge midpoint in the provider-defined ordering.
    EdgeMidpoint(u32),
    /// The provider's single whole-member site.
    #[default]
    Center,
}

/// A local position and orientation for one [`AnchorSite`].
///
/// Frames are complete: use [`Position::default`] and
/// [`Orientation::default`] for an origin/identity frame rather than omitting
/// either half. [`AnchorFrame::try_new`] rejects a non-finite position, so the
/// check runs at construction and not on every read; [`Orientation`] already
/// guarantees a finite normalized quaternion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct AnchorFrame {
    position:    Position,
    orientation: Orientation,
}

impl AnchorFrame {
    /// Creates a complete local anchor frame.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::NonFiniteAnchorPosition`] when `position`
    /// contains NaN or an infinite coordinate.
    pub fn try_new(position: Position, orientation: Orientation) -> Result<Self, GeometryError> {
        if !position.is_finite() {
            return Err(GeometryError::NonFiniteAnchorPosition);
        }

        Ok(Self {
            position,
            orientation,
        })
    }

    /// Returns this site's local position in the provider entity frame.
    #[must_use]
    pub const fn position(&self) -> Position { self.position }

    /// Returns this site's local tangent orientation in the provider entity frame.
    #[must_use]
    pub const fn orientation(&self) -> Orientation { self.orientation }
}

/// An ordered ordinary edge between two anchor sites.
///
/// Endpoint order supplies the edge-axis sign convention: reversing `start`
/// and `end` reverses [`Edge::axis`]. The value itself does not assert that an
/// edge is valid for a particular provider; [`ResolvedAnchorGeometry::try_new`]
/// does that when it owns the endpoints.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub struct Edge {
    /// Edge start in the provider-defined site ordering.
    pub start: AnchorSite,
    /// Edge end in the provider-defined site ordering.
    pub end:   AnchorSite,
}

impl Edge {
    /// Minimum endpoint separation, in authored units, that has a usable axis.
    const MIN_AXIS_LENGTH: f32 = 1e-4;

    /// Returns the native Bevy direction from `start` to `end`.
    ///
    /// `Dir3` appears at this calculation boundary only; stored geometry keeps
    /// [`Position`] and [`Orientation`] values.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::MissingAnchorSite`] when `geometry` holds no
    /// frame for one of this edge's endpoint sites, or
    /// [`GeometryError::DegenerateEdge`] when its endpoints are identical or
    /// too close to define a direction.
    pub fn axis(&self, geometry: &ResolvedAnchorGeometry) -> Result<Dir3, GeometryError> {
        if self.start == self.end {
            return Err(GeometryError::DegenerateEdge { edge: *self });
        }
        let start = geometry.frame(self.start)?.position().into_inner();
        let end = geometry.frame(self.end)?.position().into_inner();
        let separation = end - start;
        if separation.length_squared() < Self::MIN_AXIS_LENGTH * Self::MIN_AXIS_LENGTH {
            return Err(GeometryError::DegenerateEdge { edge: *self });
        }

        Dir3::new(separation).map_err(|_| GeometryError::DegenerateEdge { edge: *self })
    }
}

/// Validated provider-filled anchor geometry for one entity.
///
/// Construction owns all structural validation. Frames are keyed by unique
/// [`AnchorSite`] values and every edge references distinct, non-coincident
/// frame endpoints. The value uses opaque reflection, so dynamic component
/// editing cannot create invalid geometry after construction. [`Default`]
/// represents valid empty geometry for a provider that currently exposes no
/// sites.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component, opaque)]
pub struct ResolvedAnchorGeometry {
    frames: HashMap<AnchorSite, AnchorFrame>,
    edges:  Vec<Edge>,
}

impl ResolvedAnchorGeometry {
    /// Validates and stores one provider's frames and ordered edges.
    ///
    /// The supplied frame iterator may yield sites in any order. Frames land in
    /// a hash map, so their later iteration order is unspecified;
    /// [`Self::edges`] keeps the exact authored edge order and each edge keeps
    /// its endpoint order.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::DuplicateAnchorSite`] for a repeated frame
    /// key, [`GeometryError::MissingAnchorSite`] for an absent edge endpoint,
    /// and [`GeometryError::DegenerateEdge`] for a same-site or
    /// near-coincident edge.
    pub fn try_new(
        frames: impl IntoIterator<Item = (AnchorSite, AnchorFrame)>,
        edges: impl IntoIterator<Item = Edge>,
    ) -> Result<Self, GeometryError> {
        let mut validated_frames = HashMap::default();
        for (site, frame) in frames {
            if validated_frames.insert(site, frame).is_some() {
                return Err(GeometryError::DuplicateAnchorSite { site });
            }
        }

        let edges = edges.into_iter().collect::<Vec<_>>();
        let geometry = Self {
            frames: validated_frames,
            edges,
        };
        for edge in geometry.edges() {
            edge.axis(&geometry)?;
        }

        Ok(geometry)
    }

    /// Returns the validated frame at `site`.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::MissingAnchorSite`] when this provider did not
    /// author `site`.
    pub fn frame(&self, site: AnchorSite) -> Result<&AnchorFrame, GeometryError> {
        self.frames
            .get(&site)
            .ok_or(GeometryError::MissingAnchorSite { site })
    }

    /// Iterates over each provider-authored site and validated local frame.
    ///
    /// Hash-map iteration order is unspecified; consumers that need authored
    /// sequencing should use provider-defined indices or [`Self::edges`].
    pub fn frames(&self) -> impl Iterator<Item = (&AnchorSite, &AnchorFrame)> { self.frames.iter() }

    /// Returns the ordered edges exactly as the provider authored them.
    #[must_use]
    pub fn edges(&self) -> &[Edge] { &self.edges }
}

/// A rejected anchor-frame or geometry construction request.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GeometryError {
    /// An [`AnchorFrame`] position had a NaN or infinite coordinate.
    #[error("anchor-frame position must contain only finite coordinates")]
    NonFiniteAnchorPosition,
    /// More than one frame was supplied for the same site.
    #[error("anchor geometry contains duplicate site {site:?}")]
    DuplicateAnchorSite {
        /// The repeated provider site.
        site: AnchorSite,
    },
    /// A requested frame or edge endpoint was not present in the geometry.
    #[error("anchor geometry does not contain site {site:?}")]
    MissingAnchorSite {
        /// The absent provider site.
        site: AnchorSite,
    },
    /// An edge used the same site twice or endpoints too close for an axis.
    #[error("anchor edge {edge:?} has no usable direction")]
    DegenerateEdge {
        /// The invalid ordered edge.
        edge: Edge,
    },
}

#[cfg(test)]
mod tests {
    use bevy_math::Dir3;
    use bevy_math::Quat;
    use bevy_math::Vec3;
    use bevy_reflect::FromReflect;
    use bevy_reflect::ReflectKind;
    use bevy_reflect::Typed;
    use hana_kana::Orientation;
    use hana_kana::Position;

    use super::AnchorFrame;
    use super::AnchorSite;
    use super::Edge;
    use super::GeometryError;
    use super::ResolvedAnchorGeometry;

    fn valid<T: Default, E>(result: Result<T, E>) -> T {
        assert!(result.is_ok(), "test setup must be valid");
        result.unwrap_or_default()
    }

    fn frame(position: Vec3) -> AnchorFrame {
        valid(AnchorFrame::try_new(
            Position::from(position),
            Orientation::default(),
        ))
    }

    #[test]
    fn frame_rejects_non_finite_position() {
        assert_eq!(
            AnchorFrame::try_new(
                Position::from(Vec3::new(f32::NAN, 0.0, 0.0)),
                Orientation::default(),
            ),
            Err(GeometryError::NonFiniteAnchorPosition),
        );
    }

    #[test]
    fn frame_retains_complete_semantic_values() {
        let orientation = valid(Orientation::try_from(Quat::from_rotation_z(0.5)));
        let frame = valid(AnchorFrame::try_new(Position::from(Vec3::X), orientation));

        assert_eq!(frame.position(), Position::from(Vec3::X));
        assert_eq!(frame.orientation(), orientation);
    }

    #[test]
    fn geometry_rejects_duplicate_site() {
        assert!(matches!(
            ResolvedAnchorGeometry::try_new(
                [
                    (AnchorSite::Center, frame(Vec3::ZERO)),
                    (AnchorSite::Center, frame(Vec3::X))
                ],
                [],
            ),
            Err(GeometryError::DuplicateAnchorSite {
                site: AnchorSite::Center,
            }),
        ));
    }

    #[test]
    fn geometry_rejects_missing_edge_endpoint() {
        let edge = Edge {
            start: AnchorSite::Center,
            end:   AnchorSite::Vertex(0),
        };

        assert!(matches!(
            ResolvedAnchorGeometry::try_new([(AnchorSite::Center, frame(Vec3::ZERO))], [edge]),
            Err(GeometryError::MissingAnchorSite {
                site: AnchorSite::Vertex(0),
            }),
        ));
    }

    #[test]
    fn geometry_rejects_same_site_edge() {
        let edge = Edge {
            start: AnchorSite::Center,
            end:   AnchorSite::Center,
        };

        assert!(matches!(
            ResolvedAnchorGeometry::try_new([(AnchorSite::Center, frame(Vec3::ZERO))], [edge]),
            Err(GeometryError::DegenerateEdge { edge: _ }),
        ));
    }

    #[test]
    fn geometry_rejects_near_coincident_edge() {
        let edge = Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        };

        assert!(matches!(
            ResolvedAnchorGeometry::try_new(
                [
                    (AnchorSite::Vertex(0), frame(Vec3::ZERO)),
                    (AnchorSite::Vertex(1), frame(Vec3::X * 0.000_01)),
                ],
                [edge],
            ),
            Err(GeometryError::DegenerateEdge { edge: _ }),
        ));
    }

    #[test]
    fn lookup_reports_missing_site() {
        let geometry = valid(ResolvedAnchorGeometry::try_new([], []));

        assert_eq!(
            geometry.frame(AnchorSite::Center),
            Err(GeometryError::MissingAnchorSite {
                site: AnchorSite::Center,
            }),
        );
    }

    #[test]
    fn edge_axis_respects_endpoint_order() {
        let forward = Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        };
        let backward = Edge {
            start: forward.end,
            end:   forward.start,
        };
        let geometry = ResolvedAnchorGeometry::try_new(
            [
                (AnchorSite::Vertex(0), frame(Vec3::ZERO)),
                (AnchorSite::Vertex(1), frame(Vec3::X)),
            ],
            [forward, backward],
        );
        let geometry = valid(geometry);

        let forward_axis = forward.axis(&geometry);
        let backward_axis = backward.axis(&geometry);
        assert!(forward_axis.is_ok());
        assert!(backward_axis.is_ok());
        assert_eq!(forward_axis.unwrap_or(Dir3::X).as_vec3(), Vec3::X);
        assert_eq!(backward_axis.unwrap_or(Dir3::X).as_vec3(), Vec3::NEG_X);
    }

    #[test]
    fn opaque_geometry_reflection_has_no_mutable_fields() {
        assert_eq!(
            ResolvedAnchorGeometry::type_info().kind(),
            ReflectKind::Opaque
        );
        assert!(ResolvedAnchorGeometry::from_reflect(&()).is_none());
    }
}
