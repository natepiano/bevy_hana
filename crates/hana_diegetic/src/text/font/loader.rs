//! Asset loader for `.ttf` and `.otf` font files.

use std::io::Error as IoError;

use bevy::asset::AssetLoader;
use bevy::asset::LoadContext;
use bevy::asset::io::Reader;
use bevy::reflect::TypePath;

use super::Font;
use super::constants::FONT_FILE_EXTENSIONS;

/// Loads `.ttf` and `.otf` font files into [`Font`] assets.
///
/// Registered automatically by [`DiegeticUiPlugin`](crate::DiegeticUiPlugin).
/// When a font asset finishes loading, the plugin registers it with
/// [`FontRegistry`](crate::FontRegistry) and fires a
/// [`FontRegistered`](crate::FontRegistered) event.
#[derive(Default, TypePath)]
pub(in crate::text) struct FontLoader;

impl AssetLoader for FontLoader {
    type Asset = Font;
    type Settings = ();
    type Error = FontLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        (): &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        // Extract family name from the font's name table.
        let path_str = load_context.path().to_string();
        let name = ttf_parser::Face::parse(&bytes, 0)
            .ok()
            .and_then(|face| {
                face.names()
                    .into_iter()
                    .filter(|n| n.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
                    .find_map(|n| n.to_string())
                    .or_else(|| {
                        face.names()
                            .into_iter()
                            .filter(|n| n.name_id == ttf_parser::name_id::FAMILY)
                            .find_map(|n| n.to_string())
                    })
            })
            .unwrap_or(path_str);

        Font::from_bytes(&name, &bytes).ok_or(FontLoaderError::ParseFailed)
    }

    fn extensions(&self) -> &[&str] { FONT_FILE_EXTENSIONS }
}

/// Errors that can occur when loading a font file.
#[derive(thiserror::Error, Debug)]
pub(in crate::text) enum FontLoaderError {
    /// I/O error reading the font file.
    #[error("failed to read font file: {0}")]
    Io(#[from] IoError),
    /// The font data could not be parsed.
    #[error("failed to parse font data")]
    ParseFailed,
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::Error as IoError;
    use std::io::ErrorKind;

    use super::FontLoaderError;

    #[test]
    fn font_loader_error_messages_are_stable() {
        let cases = [
            (
                FontLoaderError::from(IoError::other("font device unavailable")),
                "failed to read font file: font device unavailable",
            ),
            (FontLoaderError::ParseFailed, "failed to parse font data"),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn io_conversion_preserves_source() {
        let error = FontLoaderError::from(IoError::new(
            ErrorKind::UnexpectedEof,
            "font data ended early",
        ));

        assert!(matches!(
            error.source().and_then(|source| source.downcast_ref::<IoError>()),
            Some(source)
                if source.kind() == ErrorKind::UnexpectedEof
                    && source.to_string() == "font data ended early"
        ));
    }
}
