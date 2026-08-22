//! Constants shared across font loading.

// font defaults
/// Default font family name.
pub(crate) const DEFAULT_FAMILY: &str = "JetBrains Mono";
/// Embedded `JetBrains Mono` Regular font binary (SIL Open Font License).
pub(super) const EMBEDDED_FONT: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
/// File extensions recognized by the font asset loader.
pub(super) const FONT_FILE_EXTENSIONS: &[&str] = &["ttf", "otf"];
/// Printable glyph used to read a fixed-pitch face's shared horizontal advance.
pub(super) const MONOSPACE_ADVANCE_SAMPLE: char = '0';
