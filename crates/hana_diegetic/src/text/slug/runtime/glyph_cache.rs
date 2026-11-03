//! Glyph cache: positioned-glyph input, prepared runs, and GPU storage handles.

use bevy::prelude::Assets;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::render::storage::ShaderBuffer;
#[cfg(test)]
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
use crate::render::AtlasPublication;
use crate::render::AtlasPublicationError;
use crate::render::AtlasState;
use crate::render::PackedPathRecord;
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
    /// Batched-records routing state.
    batch_store:        TextRunBatchStore,
    preprocess_version: u32,
    atlas:              AtlasState,
}

impl GlyphCache {
    /// Drops cached glyphs and removes the three shader-buffer assets owned by
    /// the shared atlas, leaving the resource as it was before text was drawn.
    pub(crate) fn reset_to_post_startup(&mut self, storage_buffers: &mut Assets<ShaderBuffer>) {
        if let AtlasState::Published { handles: atlas, .. } = std::mem::take(&mut self.atlas) {
            storage_buffers.remove(&atlas.curves);
            storage_buffers.remove(&atlas.bands);
            storage_buffers.remove(&atlas.path_records);
        }
        *self = Self::default();
    }

    /// Prepares one run from already-positioned production glyphs with
    /// caller-provided X/Y placement scale.
    ///
    /// The shared outline cache retains both parsed faces and immutable packed
    /// results. Repeated glyphs need only map lookups and instance math.
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
            let font_key = FontKey::for_face(
                positioned.glyph.font_face.blob_id,
                positioned.collection_index,
            );
            let key = GlyphKey::for_packing(
                font_key,
                positioned.glyph.id,
                self.preprocess_version,
                band_count,
            );
            let units_per_em = self
                .outline_cache
                .font_units_per_em(font_key, positioned.font.data())?;
            let bounds = match self.outline_cache.get_or_insert_packed_from_face(
                key,
                positioned.font.data(),
                positioned.collection_index,
                POSITIONED_GLYPH_DIAGNOSTIC_CHAR,
                band_count,
                units_per_em,
            )? {
                CachedGlyphOutline::Invisible => continue,
                CachedGlyphOutline::Visible(outline) => outline.bounds(),
            };
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

    /// Uploads the shared glyph atlas to its GPU buffers when new glyphs have
    /// been packed since the last upload, returning the handles every run's
    /// material binds. A grown atlas gets three NEW buffer assets and every
    /// live text material is repointed at them — a longer
    /// payload would re-create the wgpu buffer behind existing material bind
    /// groups, which keep reading the dead buffer (glyphs packed after a
    /// material's creation render invisible). A frame that packs no new glyph
    /// re-uploads nothing. Returns `AtlasPublication::Empty` before any glyph
    /// is packed, so no zero-length buffer is ever created.
    pub(crate) fn commit_glyph_atlas(
        &mut self,
        storage_buffers: &mut Assets<ShaderBuffer>,
        materials: &mut Assets<PathExtendedMaterial>,
    ) -> Result<AtlasPublication, AtlasPublicationError> {
        if self.outline_cache.atlas_glyph_records().is_empty() {
            return Ok(AtlasPublication::Empty);
        }
        let revision = self.outline_cache.atlas_revision();
        // Validate and clone every published table before mutating assets or
        // materials. A failed table leaves the last drawing revision intact.
        let had_atlas = matches!(self.atlas, AtlasState::Published { .. });
        let (mut curves, mut bands, mut path_records) = match &self.atlas {
            AtlasState::Unpublished => (
                ShaderBuffer::default(),
                ShaderBuffer::default(),
                ShaderBuffer::default(),
            ),
            AtlasState::Published {
                handles,
                revision: uploaded_revision,
            } => {
                let extent = if *uploaded_revision == revision {
                    PublishedTableExtent::Complete
                } else {
                    PublishedTableExtent::Prefix
                };
                let curves = published_table::<render::CurveRecord>(
                    storage_buffers,
                    &handles.curves,
                    self.outline_cache.atlas_curves().len(),
                    extent,
                    "curves",
                )?;
                let bands = published_table::<render::BandRecord>(
                    storage_buffers,
                    &handles.bands,
                    self.outline_cache.atlas_bands().len(),
                    extent,
                    "bands",
                )?;
                let paths = published_table::<PackedPathRecord>(
                    storage_buffers,
                    &handles.path_records,
                    self.outline_cache.atlas_glyph_records().len(),
                    extent,
                    "paths",
                )?;
                if *uploaded_revision == revision {
                    return Ok(AtlasPublication::Unchanged(handles.clone()));
                }
                (curves.clone(), bands.clone(), paths.clone())
            },
        };
        append_unpublished(&mut curves, self.outline_cache.atlas_curves());
        append_unpublished(&mut bands, self.outline_cache.atlas_bands());
        append_unpublished(&mut path_records, self.outline_cache.atlas_glyph_records());
        let handles = PathAtlasHandles {
            curves:       storage_buffers.add(curves),
            bands:        storage_buffers.add(bands),
            path_records: storage_buffers.add(path_records),
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
        self.atlas = AtlasState::Published {
            handles: handles.clone(),
            revision,
        };
        Ok(AtlasPublication::Replaced(handles))
    }

    /// Whether the current glyph indices have matching GPU buffers.
    #[must_use]
    pub(crate) const fn atlas_is_current(&self) -> bool {
        matches!(&self.atlas, AtlasState::Published { revision, .. }
            if *revision == self.outline_cache.atlas_revision())
    }

    /// Shared-atlas record index for `key`, if the glyph has been packed.
    #[must_use]
    pub(crate) fn packed_path_index(&self, key: GlyphKey) -> Option<u32> {
        self.outline_cache.global_index(key)
    }

    /// Packed font metrics and bounds used to fit glyph quads at rest.
    #[must_use]
    pub(crate) fn packed_path_records(&self) -> &[PackedPathRecord] {
        self.outline_cache.atlas_glyph_records()
    }

    /// The glyph batch store (records, batch keys, GPU handles per batch).
    #[must_use]
    pub(crate) const fn batch_store(&self) -> &TextRunBatchStore { &self.batch_store }

    /// The glyph batch store, mutable.
    pub(crate) const fn batch_store_mut(&mut self) -> &mut TextRunBatchStore {
        &mut self.batch_store
    }
}

/// Record extent required of an already-published atlas table.
#[derive(Clone, Copy)]
enum PublishedTableExtent {
    /// Every record of the current revision must already be present.
    Complete,
    /// A previous revision may contain only a prefix of the current records.
    Prefix,
}

/// Clone a validated asset only when constructing the next complete revision.
fn published_table<'a, T: bytemuck::Pod>(
    buffers: &'a Assets<ShaderBuffer>,
    handle: &Handle<ShaderBuffer>,
    total_records: usize,
    extent: PublishedTableExtent,
    table: &'static str,
) -> Result<&'a ShaderBuffer, AtlasPublicationError> {
    let buffer = buffers
        .get(handle)
        .ok_or(AtlasPublicationError::MissingBuffer { table })?;
    let bytes = buffer
        .cast_slice::<u8>()
        .ok_or(AtlasPublicationError::InvalidTable { table })?;
    let records = bytemuck::try_cast_slice::<u8, T>(bytes)
        .map_err(|_| AtlasPublicationError::InvalidTable { table })?;
    let valid_extent = match extent {
        PublishedTableExtent::Complete => records.len() == total_records,
        PublishedTableExtent::Prefix => records.len() <= total_records,
    };
    if !valid_extent {
        return Err(AtlasPublicationError::InvalidTable { table });
    }
    Ok(buffer)
}

/// Publication progress comes from the buffer itself, never a second counter.
fn append_unpublished<T: bytemuck::Pod>(buffer: &mut ShaderBuffer, records: &[T]) {
    let published = buffer
        .cast_slice::<u8>()
        .map_or(0, |bytes| bytes.len() / std::mem::size_of::<T>());
    buffer.extend_from_slice(&records[published..]);
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
    fn mixed_font_atlas_keeps_each_glyphs_design_space_em() {
        let mut atlas = GlyphOutlineCache::default();
        for (font_key, bytes, expected_em) in [
            (FontKey::new(1), FONT_DATA, 1000.0),
            (FontKey::new(2), LIBERATION_SANS_DATA, 2048.0),
        ] {
            let face = Face::parse(bytes, 0).expect("fixture face");
            let glyph_id = face.glyph_index('A').expect("fixture A").0;
            let key = GlyphKey::with_preprocess_version(font_key, glyph_id, 0);
            atlas
                .get_or_insert_packed_from_face(
                    key,
                    bytes,
                    0,
                    'A',
                    render::DEFAULT_BAND_COUNT,
                    f32::from(face.units_per_em()),
                )
                .expect("packed glyph");
            let index = atlas.global_index(key).expect("atlas slot") as usize;
            assert_eq!(atlas.atlas_glyph_records()[index].text_em_size, expected_em);
        }
        let records = atlas.atlas_glyph_records();
        assert!(records[0].text_em_size / 100.0 < 20.0);
        assert!(records[1].text_em_size / 100.0 >= 20.0);
    }

    #[test]
    fn commit_glyph_atlas_uploads_one_buffer_each_then_reuses_them()
    -> Result<(), AtlasPublicationError> {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "Typography");
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        let first =
            published_handles(backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?);
        assert_eq!(
            storage_buffers.len(),
            3,
            "the atlas is one curves, one bands, and one glyphs buffer"
        );

        let second =
            published_handles(backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?);
        assert_eq!(first.curves, second.curves);
        assert_eq!(first.bands, second.bands);
        assert_eq!(first.path_records, second.path_records);
        assert_eq!(
            storage_buffers.len(),
            3,
            "no newly packed glyph means no new atlas buffer"
        );
        Ok(())
    }

    #[test]
    fn commit_glyph_atlas_growth_swaps_buffers_and_keeps_handles_fresh()
    -> Result<(), AtlasPublicationError> {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "Typo");
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        let first =
            published_handles(backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?);

        // New glyphs bump the atlas revision: the commit must produce NEW
        // buffer assets (re-publishing the old ones would re-create the wgpu
        // buffer behind existing material bind groups).
        prepare(&mut backend, "graphy!");
        let second =
            published_handles(backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?);
        assert_ne!(first.curves, second.curves);
        assert_ne!(first.bands, second.bands);
        assert_ne!(first.path_records, second.path_records);
        assert_eq!(
            storage_buffers
                .get(&second.curves)
                .expect("curves")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_curves())
                    .expect("canonical records encode")
                    .as_slice()
            ),
            "suffix publication preserves the full array layout including stride padding",
        );
        assert_eq!(
            storage_buffers
                .get(&second.bands)
                .expect("bands")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_bands())
                    .expect("canonical records encode")
                    .as_slice()
            ),
        );
        assert_eq!(
            storage_buffers
                .get(&second.path_records)
                .expect("paths")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_glyph_records())
                    .expect("canonical records encode")
                    .as_slice()
            ),
        );

        backend.reset_to_post_startup(&mut storage_buffers);
        prepare(&mut backend, "A");
        let reset =
            published_handles(backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?);
        assert_eq!(
            storage_buffers
                .get(&reset.curves)
                .expect("reset curves")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_curves())
                    .expect("canonical records encode")
                    .as_slice()
            ),
            "reset retires published prefixes as well as atlas handles",
        );
        Ok(())
    }

    #[test]
    fn commit_glyph_atlas_with_no_packed_glyphs_creates_no_buffer()
    -> Result<(), AtlasPublicationError> {
        let mut backend = GlyphCache::default();
        let mut storage_buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();

        assert!(matches!(
            backend.commit_glyph_atlas(&mut storage_buffers, &mut materials)?,
            AtlasPublication::Empty
        ));
        assert_eq!(storage_buffers.len(), 0);
        Ok(())
    }

    #[test]
    fn glyph_publication_failure_keeps_previous_revision_and_recovers()
    -> Result<(), AtlasPublicationError> {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "A");
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();
        let first = published_handles(backend.commit_glyph_atlas(&mut buffers, &mut materials)?);
        let material =
            render::glyph_atlas_test_batch(&mut backend.batch_store, &mut materials, &first)
                .expect("live batch");
        let curves_before = buffers
            .get(&first.curves)
            .expect("published curves")
            .cast_slice::<u8>()
            .expect("initialized")
            .to_vec();
        let bands_before = buffers.remove(&first.bands).expect("published bands");
        prepare(&mut backend, "B");
        let asset_count = buffers.len();
        assert_eq!(
            backend
                .commit_glyph_atlas(&mut buffers, &mut materials)
                .expect_err("missing table prevents partial publication"),
            AtlasPublicationError::MissingBuffer { table: "bands" }
        );
        assert_eq!(
            buffers.len(),
            asset_count,
            "failure publishes no partial table"
        );
        assert!(!backend.atlas_is_current());
        assert!(
            matches!(&backend.atlas, AtlasState::Published { handles, .. } if handles.curves == first.curves && handles.bands == first.bands && handles.path_records == first.path_records)
        );
        assert_eq!(
            buffers
                .get(&first.curves)
                .expect("last drawing curves retained")
                .cast_slice::<u8>(),
            Some(curves_before.as_slice())
        );
        assert_material_atlas(&materials, &material, &first);
        buffers
            .insert(first.bands.id(), bands_before)
            .expect("restore published bands");
        let recovered = backend.commit_glyph_atlas(&mut buffers, &mut materials)?;
        assert!(matches!(recovered, AtlasPublication::Replaced(_)));
        let handles = published_handles(recovered);
        assert!(backend.atlas_is_current());
        assert_ne!(first.curves, handles.curves);
        assert_ne!(first.bands, handles.bands);
        assert_ne!(first.path_records, handles.path_records);
        assert_material_atlas(&materials, &material, &handles);
        assert_eq!(
            buffers
                .get(&handles.curves)
                .expect("complete recovered curves")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_curves())
                    .expect("canonical records encode")
                    .as_slice()
            )
        );
        assert_eq!(
            buffers
                .get(&handles.bands)
                .expect("complete recovered bands")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_bands())
                    .expect("canonical records encode")
                    .as_slice()
            )
        );
        assert_eq!(
            buffers
                .get(&handles.path_records)
                .expect("complete recovered paths")
                .cast_slice::<u8>(),
            Some(
                crate::render::encase_storage_bytes(backend.outline_cache.atlas_glyph_records())
                    .expect("canonical records encode")
                    .as_slice()
            )
        );
        Ok(())
    }

    #[test]
    fn glyph_publication_rejects_invalid_table_without_replacing_drawing()
    -> Result<(), AtlasPublicationError> {
        let mut backend = GlyphCache::default();
        prepare(&mut backend, "A");
        let mut buffers = Assets::<ShaderBuffer>::default();
        let mut materials = Assets::<PathExtendedMaterial>::default();
        let handles = published_handles(backend.commit_glyph_atlas(&mut buffers, &mut materials)?);
        let material =
            render::glyph_atlas_test_batch(&mut backend.batch_store, &mut materials, &handles)
                .expect("live batch");
        let original = buffers.get(&handles.path_records).expect("paths").clone();
        buffers
            .insert(handles.path_records.id(), ShaderBuffer::from(vec![1_u32]))
            .expect("inject malformed paths");
        assert_eq!(
            backend
                .commit_glyph_atlas(&mut buffers, &mut materials)
                .expect_err("invalid record stride"),
            AtlasPublicationError::InvalidTable { table: "paths" }
        );
        assert_eq!(buffers.len(), 3, "no partial replacement on failure");
        assert_material_atlas(&materials, &material, &handles);
        buffers
            .insert(handles.path_records.id(), original)
            .expect("restore paths");
        assert!(matches!(
            backend.commit_glyph_atlas(&mut buffers, &mut materials)?,
            AtlasPublication::Unchanged(_)
        ));
        prepare(&mut backend, "B");
        let recovered = backend.commit_glyph_atlas(&mut buffers, &mut materials)?;
        assert!(matches!(recovered, AtlasPublication::Replaced(_)));
        let recovered = published_handles(recovered);
        assert_ne!(handles.curves, recovered.curves);
        assert_ne!(handles.bands, recovered.bands);
        assert_ne!(handles.path_records, recovered.path_records);
        assert_material_atlas(&materials, &material, &recovered);
        Ok(())
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

    fn assert_material_atlas(
        materials: &Assets<PathExtendedMaterial>,
        material: &Handle<PathExtendedMaterial>,
        expected: &PathAtlasHandles,
    ) {
        let actual = materials
            .get(material)
            .expect("live batch material")
            .extension
            .atlas_handles();
        assert_eq!(actual.curves, expected.curves);
        assert_eq!(actual.bands, expected.bands);
        assert_eq!(actual.path_records, expected.path_records);
    }

    fn published_handles(publication: AtlasPublication) -> PathAtlasHandles {
        match publication {
            AtlasPublication::Empty => None,
            AtlasPublication::Unchanged(handles) | AtlasPublication::Replaced(handles) => {
                Some(handles)
            },
        }
        .expect("packed glyphs should produce atlas handles")
    }

    fn prepare(backend: &mut GlyphCache, text: &str) -> PreparedTextRun {
        support::prepare_fixture_run(backend, FONT_DATA, 11, text)
            .expect("fixture text should prepare")
    }
}
