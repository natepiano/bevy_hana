//! Font registry backed by parley's `FontContext`.

use std::collections::HashMap;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::Mutex;

use bevy::asset::AssetLoadError;
use bevy::asset::AssetPath;
use bevy::prelude::Event;
use bevy::prelude::Resource;
use hana_kana::ToU16;

/// How a font was loaded into the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontSource {
    /// Embedded in the library binary (e.g., `JetBrains Mono`).
    Embedded,
    /// Loaded at runtime via `AssetServer` or `Assets<Font>::add()`.
    Loaded,
}

impl Display for FontSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embedded => write!(f, "embedded"),
            Self::Loaded => write!(f, "loaded"),
        }
    }
}

/// Fired when a font is successfully registered and ready for use.
///
/// Observe this event to react when fonts become available — works for
/// both the embedded default font and fonts loaded via `AssetServer`.
///
/// ```ignore
/// app.add_observer(|trigger: On<FontRegistered>| {
///     info!("{} font ready: {} (id: {:?})", trigger.source, trigger.name, trigger.id);
/// });
/// ```
#[derive(Event, Clone, Debug)]
pub struct FontRegistered {
    /// The [`FontId`] assigned to this font.
    pub id:     FontId,
    /// The font family name.
    pub name:   String,
    /// Whether this font was embedded or loaded at runtime.
    pub source: FontSource,
}

/// Fired when a font asset fails to load, parse or register.
///
/// Covers I/O errors (file not found), parse errors (corrupt font data) and
/// a loaded font the [`FontRegistry`] refuses. Observe this event to show
/// error UI or fall back to a default font.
///
/// ```ignore
/// app.add_observer(|trigger: On<FontLoadFailed>| {
///     warn!("Font failed: {} — {}", trigger.source, trigger.error);
/// });
/// ```
#[derive(Event, Clone, Debug)]
pub struct FontLoadFailed {
    /// The font that failed.
    pub source: FontFailureSource,
    /// Why it failed.
    pub error:  FontLoadError,
}

/// Why a font asset never became a registered font.
#[derive(Clone, Debug, thiserror::Error)]
pub enum FontLoadError {
    /// The asset server could not load the font file.
    #[error(transparent)]
    Asset(AssetLoadError),
    /// The font loaded, but the [`FontRegistry`] refused it.
    #[error(transparent)]
    Registration(FontRegistrationError),
}

/// Which font a [`FontLoadFailed`] event reports.
#[derive(Clone, Debug)]
pub enum FontFailureSource {
    /// The asset path the font was loaded from.
    AssetPath(AssetPath<'static>),
    /// The family name of a font added in code, which has no asset path.
    FamilyName(String),
}

impl Display for FontFailureSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AssetPath(path) => write!(f, "{path}"),
            Self::FamilyName(name) => write!(f, "{name}"),
        }
    }
}

use parley::FontContext;
use parley::fontique::Blob;
use parley::fontique::FontInfoOverride;

use super::Font;
use super::FontParseError;
use super::constants::DEFAULT_FAMILY;
use super::constants::EMBEDDED_BOLD_FONT;
use super::constants::EMBEDDED_REGULAR_FONT;
use super::measurer;
use super::outline;
use super::outline::TextOutlineError;
use super::outline::TextOutlineMeasurement;
use crate::layout::ResolvedFontFace;
use crate::layout::TextInkMeasurement;
use crate::layout::TextMeasure;

/// Unique identifier for a loaded font family.
///
/// Used with [`TextStyle::with_font`](crate::TextStyle::with_font) to select which
/// font a text element uses.
///
/// [`MONOSPACE`](Self::MONOSPACE) (`JetBrains Mono`) is embedded in the library
/// and is the id every text element resolves to when none is set. Further ids
/// are assigned by [`FontRegistry::register_font`] and by loading a `.ttf` or
/// `.otf` file as a [`Font`] asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontId(pub u16);

impl FontId {
    /// The built-in monospace font (`JetBrains Mono`).
    ///
    /// This is the default font used by all text when no font is specified.
    pub const MONOSPACE: Self = Self(0);
}

/// A face of the embedded default family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddedFace {
    /// The regular-weight face.
    Regular,
    /// The bold face.
    Bold,
}

/// Why [`FontRegistry::new`] could not build the embedded default family.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FontRegistryError {
    /// An embedded face's font tables could not be read.
    #[error("embedded {0:?} font face is unreadable")]
    EmbeddedFaceUnreadable(EmbeddedFace),
}

/// Why [`FontRegistry::register_font`] refused a font.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FontRegistrationError {
    /// The font data does not parse.
    #[error("cannot register an unreadable font")]
    Unreadable(#[from] FontParseError),
}

/// Why a [`FontRegistry`] lookup found nothing.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FontLookupError {
    /// No family is registered under this ID.
    #[error("no font family is registered as {0:?}")]
    UnknownFamilyId(FontId),
    /// No family is registered under this name.
    #[error("no font family is registered as {0:?}")]
    UnknownFamilyName(String),
    /// The shaped glyph's face comes from font data that was never registered.
    #[error("the shaped face's font data was never registered")]
    UnknownResolvedFace,
}

/// A shaped face resolves either to a selectable family or to a supplemental
/// face of a registered family.
enum RegisteredFace {
    Family(FontId),
    Supplemental(SupplementalFace),
}

/// A face that belongs to a registered family without being selectable on its
/// own, such as the embedded Bold face of [`FontId::MONOSPACE`].
struct SupplementalFace {
    family: FontId,
    face:   Font,
}

/// Resource managing font loading via parley's `FontContext`.
///
/// Fonts are registered by embedding raw TTF/OTF bytes. The registry
/// maps [`FontId`] values to [`Font`] structs that provide access to
/// font-level typographic metrics.
///
/// The embedded `JetBrains Mono` font is always available as [`FontId::MONOSPACE`].
///
/// Access the registry in Bevy systems via `Res<FontRegistry>`:
///
/// ```ignore
/// fn my_system(registry: Res<FontRegistry>) {
///     let font = registry.font(FontId::MONOSPACE).unwrap();
///     let metrics = font.metrics(48.0);
///     info!("ascent: {}", metrics.ascent);
/// }
/// ```
#[derive(Resource)]
pub struct FontRegistry {
    /// Shared font context — also held by the measurement closure.
    font_cx:    Arc<Mutex<FontContext>>,
    /// Parsed fonts indexed by [`FontId`].
    fonts:      Vec<Font>,
    font_faces: HashMap<u64, RegisteredFace>,
}

impl FontRegistry {
    /// Creates a new registry with the embedded default font.
    ///
    /// # Errors
    ///
    /// Returns [`FontRegistryError::EmbeddedFaceUnreadable`] when an embedded
    /// `JetBrains Mono` face fails to parse. Those faces are compiled into the
    /// library and parse, so the error does not occur in practice.
    pub fn new() -> Result<Self, FontRegistryError> {
        let mut font_cx = FontContext::default();

        let regular_font = Font::from_bytes(DEFAULT_FAMILY, EMBEDDED_REGULAR_FONT)
            .map_err(|_| FontRegistryError::EmbeddedFaceUnreadable(EmbeddedFace::Regular))?;
        let bold_font = Font::from_bytes(DEFAULT_FAMILY, EMBEDDED_BOLD_FONT)
            .map_err(|_| FontRegistryError::EmbeddedFaceUnreadable(EmbeddedFace::Bold))?;
        let regular_blob = Blob::from(EMBEDDED_REGULAR_FONT.to_vec());
        let regular_blob_id = regular_blob.id();
        font_cx.collection.register_fonts(
            regular_blob,
            Some(FontInfoOverride {
                family_name: Some(DEFAULT_FAMILY),
                ..Default::default()
            }),
        );
        let bold_blob = Blob::from(EMBEDDED_BOLD_FONT.to_vec());
        let bold_blob_id = bold_blob.id();
        font_cx.collection.register_fonts(
            bold_blob,
            Some(FontInfoOverride {
                family_name: Some(DEFAULT_FAMILY),
                ..Default::default()
            }),
        );

        let mut font_faces = HashMap::new();
        font_faces.insert(regular_blob_id, RegisteredFace::Family(FontId::MONOSPACE));
        font_faces.insert(
            bold_blob_id,
            RegisteredFace::Supplemental(SupplementalFace {
                family: FontId::MONOSPACE,
                face:   bold_font,
            }),
        );

        Ok(Self {
            font_cx: Arc::new(Mutex::new(font_cx)),
            fonts: vec![regular_font],
            font_faces,
        })
    }

    /// Returns the [`Font`] for a given [`FontId`].
    ///
    /// # Errors
    ///
    /// Returns [`FontLookupError::UnknownFamilyId`] when no family is
    /// registered under `id`.
    pub fn font(&self, id: impl Into<FontId>) -> Result<&Font, FontLookupError> {
        let id = id.into();
        self.fonts
            .get(usize::from(id.0))
            .ok_or(FontLookupError::UnknownFamilyId(id))
    }

    /// Returns the family name for a given [`FontId`].
    ///
    /// # Errors
    ///
    /// Returns [`FontLookupError::UnknownFamilyId`] when no family is
    /// registered under `id`.
    pub fn family_name(&self, id: FontId) -> Result<&str, FontLookupError> {
        self.font(id).map(Font::name)
    }

    /// Returns the concrete font and collection index for a shaped glyph face.
    ///
    /// # Errors
    ///
    /// Returns [`FontLookupError::UnknownResolvedFace`] when the face's font
    /// data was never registered.
    pub fn font_for_face(&self, face: ResolvedFontFace) -> Result<(&Font, u32), FontLookupError> {
        let font = match self
            .font_faces
            .get(&face.blob_id)
            .ok_or(FontLookupError::UnknownResolvedFace)?
        {
            RegisteredFace::Family(font_id) => self.font(*font_id)?,
            RegisteredFace::Supplemental(supplemental) => {
                // A supplemental face draws only as part of its registered family.
                self.font(supplemental.family)?;
                &supplemental.face
            },
        };
        Ok((font, face.collection_index))
    }

    /// Registers an additional font from raw TTF/OTF bytes and returns the
    /// [`FontId`] assigned to it.
    ///
    /// The font is immediately available for use in [`TextStyle`](crate::TextStyle)
    /// via `.with_font(id.0)`. Glyph outlines are built on demand by the slug
    /// renderer when text using this font is first rendered.
    ///
    /// # Example
    ///
    /// ```ignore
    /// const NOTO_SANS: &[u8] = include_bytes!("NotoSans-Regular.ttf");
    ///
    /// fn setup(mut registry: ResMut<FontRegistry>) {
    ///     let id = registry.register_font("Noto Sans", NOTO_SANS)
    ///         .expect("font should parse");
    ///     // Use `id.0` with `TextStyle::new(size).with_font(id.0)`.
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`FontRegistrationError::Unreadable`] when `data` does not
    /// parse as a font face; the registry is unchanged.
    pub fn register_font(
        &mut self,
        name: &str,
        data: &[u8],
    ) -> Result<FontId, FontRegistrationError> {
        let font = Font::from_bytes(name, data)?;
        let blob = Blob::from(data.to_vec());
        let blob_id = blob.id();

        // Register with parley's font collection.
        let mut font_cx = self
            .font_cx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        font_cx.collection.register_fonts(
            blob,
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        drop(font_cx);

        let id = FontId(self.fonts.len().to_u16());
        self.fonts.push(font);
        self.font_faces.insert(blob_id, RegisteredFace::Family(id));
        Ok(id)
    }

    /// Returns the [`FontId`] for a font with the given family name.
    ///
    /// # Errors
    ///
    /// Returns [`FontLookupError::UnknownFamilyName`] when no family is
    /// registered under `name`.
    pub fn font_id_by_name(&self, name: &str) -> Result<FontId, FontLookupError> {
        self.fonts
            .iter()
            .position(|f| f.name() == name)
            .map(|i| FontId(i.to_u16()))
            .ok_or_else(|| FontLookupError::UnknownFamilyName(name.to_string()))
    }

    /// Measures the ink `text` draws when laid out in `measure`'s style:
    /// [`TextInkMeasurement::NoVisibleInk`] when no glyph draws any (empty or
    /// blank text).
    ///
    /// Shapes with the same font context and families as the parley measurer
    /// and the panel-text renderer, so the bounds sit on the drawn glyphs.
    ///
    /// # Errors
    ///
    /// Returns [`FontLookupError::UnknownFamilyId`] when `measure`'s font ID
    /// names no registered family.
    pub fn measure_ink(
        &self,
        text: &str,
        measure: &TextMeasure,
    ) -> Result<TextInkMeasurement, FontLookupError> {
        let family_name = self.family_name(FontId(measure.font_id))?;
        Ok(measurer::measure_ink(
            &self.font_cx,
            family_name,
            text,
            measure,
        ))
    }

    /// Shapes `text` in `measure`'s style (the parley setup [`measure_ink`](Self::measure_ink)
    /// and the panel renderer use) and returns every visible glyph's outline
    /// along the run: [`TextOutlineMeasurement::NoVisibleInk`] when no glyph
    /// draws any.
    ///
    /// Units are layout units at `measure.size`, x right and y up, with the
    /// origin at the left end of the first line's baseline.
    ///
    /// # Errors
    ///
    /// Returns [`TextOutlineError::Lookup`] when `measure`'s font ID names no
    /// registered family, and [`TextOutlineError::UnreadableFace`] or
    /// [`TextOutlineError::MissingGlyphId`] when a shaped glyph's face cannot
    /// give its outline.
    pub fn text_outline(
        &self,
        text: &str,
        measure: &TextMeasure,
    ) -> Result<TextOutlineMeasurement, TextOutlineError> {
        let family_name = self.family_name(FontId(measure.font_id))?;
        outline::text_outline(&self.font_cx, family_name, text, measure)
    }

    /// Returns the shared font context for use by the measurement closure.
    #[must_use]
    pub fn font_context(&self) -> Arc<Mutex<FontContext>> { Arc::clone(&self.font_cx) }

    /// Returns a cloned list of family names for the measurement closure.
    #[must_use]
    pub fn family_names(&self) -> Vec<String> {
        self.fonts
            .iter()
            .map(Font::name)
            .map(String::from)
            .collect()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
mod tests {
    use parley::Layout;
    use parley::LayoutContext;
    use parley::layout::PositionedLayoutItem;
    use parley::style::FontFamily;
    use parley::style::FontWeight;
    use parley::style::StyleProperty;

    use super::DEFAULT_FAMILY;
    use super::EMBEDDED_BOLD_FONT;
    use super::EMBEDDED_REGULAR_FONT;
    use super::FontId;
    use super::FontLookupError;
    use super::FontParseError;
    use super::FontRegistrationError;
    use super::FontRegistry;
    use super::RegisteredFace;
    use super::SupplementalFace;
    use crate::layout::ResolvedFontFace;

    fn resolved_face_for_weight(registry: &FontRegistry, weight: f32) -> ResolvedFontFace {
        let font_context = registry.font_context();
        let mut font_cx = font_context.lock().unwrap();
        let mut layout_cx = LayoutContext::default();
        let mut layout: Layout<()> = Layout::new();
        let mut builder = layout_cx.ranged_builder(&mut font_cx, "Bold", 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named(DEFAULT_FAMILY)));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
        builder.build_into(&mut layout, "Bold");
        drop(font_cx);
        layout.break_all_lines(None);

        layout
            .lines()
            .flat_map(|line| line.items())
            .find_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => {
                    let font = run.run().font();
                    Some(ResolvedFontFace {
                        requested_font_id: FontId::MONOSPACE.0,
                        blob_id:           font.data.id(),
                        collection_index:  font.index,
                    })
                },
                PositionedLayoutItem::InlineBox(_) => None,
            })
            .unwrap()
    }

    #[test]
    fn embedded_default_family_resolves_regular_and_bold_faces() {
        let registry = FontRegistry::new().unwrap();
        assert_eq!(
            registry.font_id_by_name(DEFAULT_FAMILY),
            Ok(FontId::MONOSPACE)
        );

        let regular = resolved_face_for_weight(&registry, 400.0);
        let bold = resolved_face_for_weight(&registry, 700.0);
        assert_ne!(regular.blob_id, bold.blob_id);
        assert_eq!(
            registry.font_for_face(regular).unwrap().0.data(),
            EMBEDDED_REGULAR_FONT
        );
        assert_eq!(
            registry.font_for_face(bold).unwrap().0.data(),
            EMBEDDED_BOLD_FONT
        );
    }

    #[test]
    fn bold_supplemental_face_names_the_monospace_family() {
        let registry = FontRegistry::new().unwrap();
        let bold = resolved_face_for_weight(&registry, 700.0);

        assert!(matches!(
            registry.font_faces.get(&bold.blob_id),
            Some(RegisteredFace::Supplemental(SupplementalFace { family, .. }))
                if *family == FontId::MONOSPACE
        ));
    }

    #[test]
    fn extra_default_face_does_not_take_a_family_id() {
        let mut registry = FontRegistry::new().unwrap();
        let next = registry
            .register_font("Another family", EMBEDDED_REGULAR_FONT)
            .unwrap();

        assert_eq!(next, FontId(1));
        assert_eq!(
            registry.font_id_by_name(DEFAULT_FAMILY),
            Ok(FontId::MONOSPACE)
        );
    }

    #[test]
    fn unreadable_font_data_returns_named_registration_error_without_changing_registry() {
        let mut registry = FontRegistry::new().unwrap();
        let previous_family_count = registry.family_names().len();
        let result = registry.register_font("Unreadable", b"not a font");

        assert_eq!(
            result,
            Err(FontRegistrationError::Unreadable(
                FontParseError::Unreadable {
                    name: "Unreadable".to_string(),
                }
            ))
        );
        assert_eq!(registry.family_names().len(), previous_family_count);
        assert_eq!(
            registry.font_id_by_name("Unreadable"),
            Err(FontLookupError::UnknownFamilyName("Unreadable".to_string()))
        );
    }
}
