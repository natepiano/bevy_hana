//! Generic atlas for non-text analytic paths.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::hash::Hash;

use bevy::prelude::Assets;
use bevy::render::storage::ShaderBuffer;
use hana_kana::ToU32;

use super::BandRecord;
use super::CurveRecord;
use super::PackedPathRecord;
use super::PathAtlasHandles;
use super::PathOutline;
use super::packing;
use super::packing::BandLayout;

/// Compact atlas keyed by caller-owned stable path identities.
#[derive(Debug)]
pub(crate) struct PathAtlas<K> {
    indices:      HashMap<K, u32>,
    curves:       Vec<CurveRecord>,
    bands:        Vec<BandRecord>,
    path_records: Vec<PackedPathRecord>,
    revision:     u32,
    publication:  AtlasState,
}

impl<K> Default for PathAtlas<K> {
    fn default() -> Self {
        Self {
            indices:      HashMap::new(),
            curves:       Vec::new(),
            bands:        Vec::new(),
            path_records: Vec::new(),
            revision:     0,
            publication:  AtlasState::Unpublished,
        }
    }
}

impl<K> PathAtlas<K>
where
    K: Clone + Eq + Hash,
{
    /// Rebuilds the atlas from the caller's live path set. Band counts scale
    /// per axis so each band spans about `target_band_extent` design units.
    pub(crate) fn rebuild<I>(&mut self, paths: I, target_band_extent: f32)
    where
        I: IntoIterator<Item = (K, PathOutline)>,
    {
        self.indices.clear();
        self.curves.clear();
        self.bands.clear();
        self.path_records.clear();

        for (key, path) in paths {
            // Rebase each path to a unit-scale frame before packing. At
            // REFERENCE-scale coordinates (~1e6 design units) the shader's
            // field normal loses its direction to float32 cancellation and
            // grazing lines wave; coverage is scale-invariant, so the only
            // change is restored precision.
            let (path, scale) = path.normalized();
            let minimum_feature_size = path.minimum_feature_size();
            // Straight-edge convex line marks (minimum_feature_size > 0, every contour
            // linear) pack as half-plane polygons: each contour's coverage is
            // the product of its edges' band ramps, with no radial point field
            // to bulge a grazing-corner wing. A curved contour (a Circle cap's
            // ellipse arcs) keeps the single-band curve-distance scan; a tight
            // line path packs into one band so every fragment scans every curve
            // once (a multi-band layout misses the nearest curve at grazing).
            let packed = if minimum_feature_size > 0.0 && packing::path_is_straight_polygon(&path) {
                packing::build_packed_polygons(path)
            } else {
                let layout = if minimum_feature_size > 0.0 {
                    BandLayout::uniform(1)
                } else {
                    BandLayout::for_extents(
                        path.bounds,
                        target_band_extent / scale,
                        path.curve_count(),
                    )
                };
                packing::build_packed_path_with_layout(path, layout)
            };
            let record_index = self.path_records.len().to_u32();
            let curve_start = self.curves.len().to_u32();
            let band_start = self.bands.len().to_u32();

            self.curves.extend_from_slice(packed.curves());
            self.bands
                .extend(packed.bands().iter().map(|band| BandRecord {
                    start: band.start + curve_start,
                    ..*band
                }));
            self.path_records.push(PackedPathRecord::new(
                packed.bounds(),
                band_start,
                packed.horizontal_band_count(),
                band_start + packed.horizontal_band_count(),
                packed.vertical_band_count(),
                minimum_feature_size,
            ));
            self.indices.insert(key, record_index);
        }

        self.revision = self.revision.wrapping_add(1);
        if self.path_records.is_empty() {
            self.publication = AtlasState::Unpublished;
        }
    }

    /// Atlas slot for `key`, if the last rebuild included it.
    #[must_use]
    pub(crate) fn index(&self, key: &K) -> Option<u32> { self.indices.get(key).copied() }

    /// Whether the current path indices have matching GPU buffers.
    #[must_use]
    pub(crate) const fn upload_is_current(&self) -> bool {
        matches!(&self.publication, AtlasState::Published { revision, .. } if *revision == self.revision)
    }

    /// Uploads atlas storage buffers when the compact atlas changed.
    pub(crate) fn upload(
        &mut self,
        storage_buffers: &mut Assets<ShaderBuffer>,
    ) -> AtlasPublication {
        if self.path_records.is_empty() {
            return AtlasPublication::Empty;
        }
        if let AtlasState::Published { handles, revision } = &self.publication
            && *revision == self.revision
        {
            return AtlasPublication::Unchanged(handles.clone());
        }

        let handles = PathAtlasHandles {
            curves:       storage_buffers.add(ShaderBuffer::from(self.curves.clone())),
            bands:        storage_buffers.add(ShaderBuffer::from(self.bands.clone())),
            path_records: storage_buffers.add(ShaderBuffer::from(self.path_records.clone())),
        };
        self.publication = AtlasState::Published {
            revision: self.revision,
            handles:  handles.clone(),
        };
        AtlasPublication::Replaced(handles)
    }
}

/// Result of publishing all three atlas tables as one drawing revision.
#[derive(Clone, Debug)]
pub(crate) enum AtlasPublication {
    /// No visible paths have been packed.
    Empty,
    /// The previously published buffers already cover this revision.
    Unchanged(PathAtlasHandles),
    /// A complete new set of buffers replaces the previous drawing revision.
    Replaced(PathAtlasHandles),
}

/// Authoritative readiness of the handles bound by live path materials.
#[derive(Debug, Default)]
pub(crate) enum AtlasState {
    #[default]
    Unpublished,
    Published {
        handles:  PathAtlasHandles,
        revision: u32,
    },
}

/// Published table failures that prevent an atomic glyph-atlas replacement.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AtlasPublicationError {
    MissingBuffer { table: &'static str },
    InvalidTable { table: &'static str },
}

impl Display for AtlasPublicationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBuffer { table } => write!(f, "published {table} buffer is missing"),
            Self::InvalidTable { table } => {
                write!(f, "published {table} buffer has invalid layout or length")
            },
        }
    }
}

impl Error for AtlasPublicationError {}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;

    use super::*;
    use crate::render::analytic_paths::Bounds;

    #[test]
    fn path_atlas_publication_tracks_empty_current_and_rebuilt_tables() -> Result<(), String> {
        let mut atlas = PathAtlas::<u32>::default();
        let mut buffers = Assets::<ShaderBuffer>::default();
        assert!(!atlas.upload_is_current());
        assert!(matches!(
            atlas.upload(&mut buffers),
            AtlasPublication::Empty
        ));
        let path = PathOutline {
            bounds:   Bounds {
                min: Vec2::ZERO,
                max: Vec2::new(20.0, 12.0),
            },
            contours: vec![super::super::PathContour {
                segments:             vec![super::super::QuadraticSegment {
                    start:   Vec2::ZERO,
                    control: Vec2::new(10.0, 4.0),
                    end:     Vec2::new(20.0, 12.0),
                }],
                minimum_feature_size: 0.0,
                fade_exponent:        0.0,
                role:                 super::super::ContourRole::Ink,
            }],
        };
        atlas.rebuild([(7, path.clone())], 8.0);
        assert!(!atlas.upload_is_current());
        let AtlasPublication::Replaced(first) = atlas.upload(&mut buffers) else {
            return Err("first publication replaces all tables".to_owned());
        };
        assert!(atlas.upload_is_current());
        assert_eq!(atlas.index(&7), Some(0));
        assert_eq!(buffers.len(), 3);
        assert_eq!(
            buffers
                .get(&first.curves)
                .and_then(ShaderBuffer::cast_slice::<CurveRecord>),
            Some(atlas.curves.as_slice())
        );
        assert_eq!(
            buffers
                .get(&first.bands)
                .and_then(ShaderBuffer::cast_slice::<BandRecord>),
            Some(atlas.bands.as_slice())
        );
        assert_eq!(
            buffers
                .get(&first.path_records)
                .and_then(ShaderBuffer::cast_slice::<PackedPathRecord>),
            Some(atlas.path_records.as_slice())
        );
        assert!(
            matches!(atlas.upload(&mut buffers), AtlasPublication::Unchanged(handles) if handles.curves == first.curves && handles.bands == first.bands && handles.path_records == first.path_records)
        );
        assert_eq!(buffers.len(), 3);
        atlas.rebuild([(8, path)], 8.0);
        assert!(!atlas.upload_is_current());
        assert!(
            matches!(atlas.upload(&mut buffers), AtlasPublication::Replaced(handles) if handles.curves != first.curves)
        );
        assert!(atlas.upload_is_current());
        atlas.rebuild([], 8.0);
        assert!(!atlas.upload_is_current());
        assert!(matches!(
            atlas.upload(&mut buffers),
            AtlasPublication::Empty
        ));
        Ok(())
    }
}
