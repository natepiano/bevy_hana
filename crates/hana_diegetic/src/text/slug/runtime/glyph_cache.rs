//! Glyph cache: positioned-glyph input, prepared runs, and GPU storage handles.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use bevy::prelude::Assets;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::render::storage::ShaderBuffer;
use ttf_parser::Face;

use super::BuiltTextRun;
use super::CachedGlyphOutline;
use super::FontKey;
use super::GlyphInstance;
use super::GlyphKey;
use super::GlyphOutlineCache;
use super::TextRun;
use crate::layout::ShapedGlyph;
use crate::render;
use crate::render::PathAtlasHandles;
use crate::render::PathExtendedMaterial;
use crate::render::TextRunBatchStore;
use crate::text::Font;
use crate::text::slug::glyph::OutlineError;

/// One production glyph positioned for run preparation: a `ShapedGlyph` plus
/// the `Font` and collection index needed to extract its outline.
#[derive(Clone, Copy)]
pub(crate) struct PositionedGlyph<'a> {
    pub glyph:            &'a ShapedGlyph,
    pub font:             &'a Font,
    pub collection_index: u32,
}

/// Result of preparing one text run.
#[derive(Clone, Debug)]
pub(crate) struct PreparedTextRun {
    /// Prepared text run.
    run: BuiltTextRun,
}

impl PreparedTextRun {
    /// Number of visible glyph quads in this prepared run.
    #[must_use]
    pub(crate) fn glyph_count(&self) -> usize { self.run.run.glyphs().len() }

    /// Positioned glyph instances in run order.
    #[must_use]
    pub(crate) fn glyphs(&self) -> &[GlyphInstance] { self.run.run.glyphs() }
}

/// Stable per-label run identifier, derived from the label entity so the same
/// label addresses the same batch-store slot every frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RunStorageKey(u64);

impl From<Entity> for RunStorageKey {
    fn from(label: Entity) -> Self { Self(label.to_bits()) }
}

/// `GlyphCache` resource: owns the glyph cache and the batch store.
#[derive(Debug, Default, Resource)]
pub(crate) struct GlyphCache {
    outline_cache:      GlyphOutlineCache,
    /// Per-font design-space scale (`units_per_em`), parsed from the face once
    /// per font so run preparation never re-parses font tables per glyph.
    units_per_em:       HashMap<FontKey, f32>,
    /// Batched-records routing state.
    batch_store:        TextRunBatchStore,
    preprocess_version: u32,
    atlas:              Option<PathAtlasHandles>,
    uploaded_revision:  u32,
}

impl GlyphCache {
    /// Prepares one run from already-positioned production glyphs with
    /// caller-provided X/Y placement scale.
    ///
    /// The steady-state per-glyph cost is two map hits (the packed outline /
    /// cached invisibility, and the font's `units_per_em`) plus the instance
    /// math; font tables are parsed only on each cache's first sighting.
    pub(crate) fn prepare_positioned_run_with_scale(
        &mut self,
        glyphs: &[PositionedGlyph<'_>],
        anchor: Vec2,
        layout_font_size: f32,
        placement_scale: Vec2,
        band_count: usize,
    ) -> Result<PreparedTextRun, OutlineError> {
        let mut instances = Vec::with_capacity(glyphs.len());
        for positioned in glyphs {
            let font_key = FontKey::new(positioned.glyph.font_face.blob_id);
            let key = self.glyph_key(font_key, positioned.glyph.id);
            let bounds = match self.outline_cache.get_or_insert_packed_from_face(
                key,
                positioned.font.data(),
                positioned.collection_index,
                POSITIONED_GLYPH_DIAGNOSTIC_CHAR,
                band_count,
            )? {
                CachedGlyphOutline::Invisible => continue,
                CachedGlyphOutline::Visible(outline) => outline.bounds(),
            };
            let units_per_em = self.font_units_per_em(font_key, positioned)?;
            instances.push(GlyphInstance::new_non_uniform(
                key,
                Vec2::new(
                    (positioned.glyph.x - anchor.x) * placement_scale.x,
                    -(positioned.glyph.baseline + positioned.glyph.y - anchor.y)
                        * placement_scale.y,
                ),
                bounds,
                placement_scale * (layout_font_size / units_per_em),
            ));
        }

        Ok(PreparedTextRun {
            run: BuiltTextRun {
                run: TextRun::new(instances),
            },
        })
    }

    /// Design-space scale (`units_per_em`) for `font_key`, parsed from the
    /// face on its first sighting and answered from the cache afterward.
    fn font_units_per_em(
        &mut self,
        font_key: FontKey,
        positioned: &PositionedGlyph<'_>,
    ) -> Result<f32, OutlineError> {
        match self.units_per_em.entry(font_key) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let face = Face::parse(positioned.font.data(), positioned.collection_index)
                    .map_err(|_| OutlineError::InvalidFont)?;
                Ok(*entry.insert(f32::from(face.units_per_em())))
            },
        }
    }

    /// Uploads the shared glyph atlas to its GPU buffers when new glyphs have
    /// been packed since the last upload, returning the handles every run's
    /// material binds. A grown atlas gets three NEW buffer assets and every
    /// live text material is repointed at them — `set_data` with a longer
    /// payload would re-create the wgpu buffer behind existing material bind
    /// groups, which keep reading the dead buffer (glyphs packed after a
    /// material's creation render invisible). A frame that packs no new glyph
    /// re-uploads nothing. Returns `None` before any glyph is packed, so no
    /// zero-length buffer is ever created.
    pub(crate) fn commit_glyph_atlas(
        &mut self,
        storage_buffers: &mut Assets<ShaderBuffer>,
        materials: &mut Assets<PathExtendedMaterial>,
    ) -> Option<PathAtlasHandles> {
        if self.outline_cache.atlas_glyph_records().is_empty() {
            return None;
        }
        let revision = self.outline_cache.atlas_revision();
        if let Some(handles) = self.atlas.clone()
            && self.uploaded_revision == revision
        {
            return Some(handles);
        }
        let had_atlas = self.atlas.is_some();
        let handles = PathAtlasHandles {
            curves:       storage_buffers.add(ShaderBuffer::from(
                self.outline_cache.atlas_curves().to_vec(),
            )),
            bands:        storage_buffers.add(ShaderBuffer::from(
                self.outline_cache.atlas_bands().to_vec(),
            )),
            path_records: storage_buffers.add(ShaderBuffer::from(
                self.outline_cache.atlas_glyph_records().to_vec(),
            )),
        };
        if had_atlas {
            // Repoint only text-owned batch materials. Other analytic-path
            // producers (panel lines, probes) share the PathExtendedMaterial asset
            // type but own separate atlases.
            for (_, batch) in self.batch_store.batches() {
                let Some(gpu) = &batch.gpu else {
                    continue;
                };
                let Some(mut material) = materials.get_mut(&gpu.material) else {
                    continue;
                };
                render::set_path_material_atlas(
                    &mut material,
                    handles.curves.clone(),
                    handles.bands.clone(),
                    handles.path_records.clone(),
                );
            }
        }
        self.uploaded_revision = revision;
        self.atlas = Some(handles.clone());
        Some(handles)
    }

    /// Shared-atlas record index for `key`, if the glyph has been packed.
    #[must_use]
    pub(crate) fn packed_path_index(&self, key: GlyphKey) -> Option<u32> {
        self.outline_cache.global_index(key)
    }

    /// The glyph batch store (records, batch keys, GPU handles per batch).
    #[must_use]
    pub(crate) const fn batch_store(&self) -> &TextRunBatchStore { &self.batch_store }

    /// The glyph batch store, mutable.
    pub(crate) const fn batch_store_mut(&mut self) -> &mut TextRunBatchStore {
        &mut self.batch_store
    }

    /// Stable key for a glyph in the current preprocessing profile.
    #[must_use]
    pub(crate) const fn glyph_key(&self, font: FontKey, glyph_id: u16) -> GlyphKey {
        GlyphKey::with_preprocess_version(font, glyph_id, self.preprocess_version)
    }
}

const POSITIONED_GLYPH_DIAGNOSTIC_CHAR: char = '\u{FFFD}';

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should fail loudly when fixture text runs cannot be prepared"
)]
mod tests {
    use bevy::prelude::Assets;

    use super::*;
    use crate::text::slug::support;

    const FONT_DATA: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf");
    const CRIMSON_TEXT_DATA: &[u8] =
        include_bytes!("../../../../assets/fonts/CrimsonText-Regular.ttf");
    const EB_GARAMOND_DATA: &[u8] =
        include_bytes!("../../../../assets/fonts/EBGaramond-Regular.ttf");
    const LIBERATION_SANS_DATA: &[u8] =
        include_bytes!("../../../../assets/fonts/LiberationSans-Regular.ttf");
    const NOTO_SANS_DATA: &[u8] = include_bytes!("../../../../assets/fonts/NotoSans-Regular.ttf");
    const NOTO_CJK_DATA: &[u8] =
        include_bytes!("../../../../assets/fonts/NotoSansCJKsc-Regular.otf");

    #[test]
    fn commit_glyph_atlas_uploads_one_buffer_each_then_reuses_them() {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "Typography");
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        let first = backend
            .commit_glyph_atlas(&mut storage_buffers, &mut materials)
            .expect("packed glyphs should produce atlas handles");
        assert_eq!(
            storage_buffers.len(),
            3,
            "the atlas is one curves, one bands, and one glyphs buffer"
        );

        let second = backend
            .commit_glyph_atlas(&mut storage_buffers, &mut materials)
            .expect("atlas handles persist across commits");
        assert_eq!(first.curves, second.curves);
        assert_eq!(first.bands, second.bands);
        assert_eq!(first.path_records, second.path_records);
        assert_eq!(
            storage_buffers.len(),
            3,
            "no newly packed glyph means no new atlas buffer"
        );
    }

    #[test]
    fn commit_glyph_atlas_growth_swaps_buffers_and_keeps_handles_fresh() {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "Typo");
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        let first = backend
            .commit_glyph_atlas(&mut storage_buffers, &mut materials)
            .expect("packed glyphs should produce atlas handles");

        // New glyphs bump the atlas revision: the commit must produce NEW
        // buffer assets (set_data on the old ones would re-create the wgpu
        // buffer behind existing material bind groups).
        prepare(&mut backend, "graphy!");
        let second = backend
            .commit_glyph_atlas(&mut storage_buffers, &mut materials)
            .expect("grown atlas should produce handles");
        assert_ne!(first.curves, second.curves);
        assert_ne!(first.bands, second.bands);
        assert_ne!(first.path_records, second.path_records);
    }

    #[test]
    fn commit_glyph_atlas_with_no_packed_glyphs_creates_no_buffer() {
        let mut backend = GlyphCache::default();
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        assert!(
            backend
                .commit_glyph_atlas(&mut storage_buffers, &mut materials)
                .is_none()
        );
        assert_eq!(storage_buffers.len(), 0);
    }

    #[test]
    fn text_runs_skip_invisible_space_glyphs() {
        let mut backend = GlyphCache::default();
        let prepared = prepare(&mut backend, "A A");

        assert_eq!(prepared.glyph_count(), 2);
    }

    #[test]
    fn space_only_text_run_is_invisible_not_failed() {
        let mut backend = GlyphCache::default();
        let prepared = prepare(&mut backend, " ");

        assert_eq!(prepared.glyph_count(), 0);
    }

    #[test]
    fn font_matrix_supports_quadratic_and_cubic_outlines() {
        let latin_fonts = [
            (FONT_DATA, 101_u64),
            (NOTO_SANS_DATA, 102),
            (EB_GARAMOND_DATA, 103),
            (CRIMSON_TEXT_DATA, 104),
            (LIBERATION_SANS_DATA, 105),
        ];

        for (font_data, font_key) in latin_fonts {
            let mut backend = GlyphCache::default();
            let prepared =
                support::prepare_fixture_run(&mut backend, font_data, font_key, "Typography")
                    .expect("Latin font fixture should prepare");
            assert_eq!(prepared.glyph_count(), 10);
        }

        let mut backend = GlyphCache::default();
        let prepared = support::prepare_fixture_run(&mut backend, NOTO_CJK_DATA, 106, "漢")
            .expect("CFF cubic font fixture should prepare through quadratic conversion");
        assert_eq!(prepared.glyph_count(), 1);
    }

    fn prepare(backend: &mut GlyphCache, text: &str) -> PreparedTextRun {
        support::prepare_fixture_run(backend, FONT_DATA, 11, text)
            .expect("fixture text should prepare")
    }
}
