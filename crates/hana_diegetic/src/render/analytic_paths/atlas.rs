//! Generic atlas for non-text analytic paths.

use std::collections::HashMap;
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
    indices:           HashMap<K, u32>,
    curves:            Vec<CurveRecord>,
    bands:             Vec<BandRecord>,
    path_records:      Vec<PackedPathRecord>,
    revision:          u32,
    uploaded_revision: u32,
    handles:           Option<PathAtlasHandles>,
}

impl<K> Default for PathAtlas<K> {
    fn default() -> Self {
        Self {
            indices:           HashMap::new(),
            curves:            Vec::new(),
            bands:             Vec::new(),
            path_records:      Vec::new(),
            revision:          0,
            uploaded_revision: u32::MAX,
            handles:           None,
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
            let min_feature = path.min_feature();
            // Straight-edge convex line marks (min_feature > 0, every contour
            // linear) pack as half-plane polygons: each contour's coverage is
            // the product of its edges' band ramps, with no radial point field
            // to bulge a grazing-corner wing. A curved contour (a Circle cap's
            // ellipse arcs) keeps the single-band curve-distance scan; a tight
            // line path packs into one band so every fragment scans every curve
            // once (a multi-band layout misses the nearest curve at grazing).
            let packed = if min_feature > 0.0 && packing::path_is_straight_polygon(&path) {
                packing::build_packed_polygons(path)
            } else {
                let layout = if min_feature > 0.0 {
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
                min_feature,
            ));
            self.indices.insert(key, record_index);
        }

        self.revision = self.revision.wrapping_add(1);
        if self.path_records.is_empty() {
            self.handles = None;
            self.uploaded_revision = u32::MAX;
        }
    }

    /// Atlas slot for `key`, if the last rebuild included it.
    #[must_use]
    pub(crate) fn index(&self, key: &K) -> Option<u32> { self.indices.get(key).copied() }

    /// Uploads atlas storage buffers when the compact atlas changed.
    pub(crate) fn upload(
        &mut self,
        storage_buffers: &mut Assets<ShaderBuffer>,
    ) -> Option<(PathAtlasHandles, bool)> {
        if self.path_records.is_empty() {
            return None;
        }
        if let Some(handles) = self.handles.clone()
            && self.uploaded_revision == self.revision
        {
            return Some((handles, false));
        }

        let handles = PathAtlasHandles {
            curves:       storage_buffers.add(ShaderBuffer::from(self.curves.clone())),
            bands:        storage_buffers.add(ShaderBuffer::from(self.bands.clone())),
            path_records: storage_buffers.add(ShaderBuffer::from(self.path_records.clone())),
        };
        self.uploaded_revision = self.revision;
        self.handles = Some(handles.clone());
        Some((handles, true))
    }
}
