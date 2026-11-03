use bevy::prelude::Vec2;
use ttf_parser::FaceParsingError;

use super::Icon;
use super::LucideIcon;
use super::lucide_generated::ALL;
use super::lucide_generated::LUCIDE_FONT_BLAKE3;
use super::lucide_generated::LUCIDE_VERSION;
use crate::FontRegistry;
use crate::TextInkMeasurement;
use crate::Unit;
use crate::text::FontId;

const LUCIDE_FONT: &[u8] = include_bytes!("../../assets/fonts/lucide.ttf");

#[test]
fn shipped_font_matches_generated_hash() {
    let actual_hash = blake3::hash(LUCIDE_FONT).to_hex();

    assert_eq!(actual_hash.as_str(), LUCIDE_FONT_BLAKE3);
}

#[test]
fn shipped_font_contains_every_generated_icon() -> Result<(), FaceParsingError> {
    let face = ttf_parser::Face::parse(LUCIDE_FONT, 0)?;

    for icon in ALL {
        assert!(
            face.glyph_index(icon.codepoint()).is_some(),
            "shipped Lucide font is missing the {} glyph",
            icon.name(),
        );
    }

    Ok(())
}

#[test]
fn license_and_notice_match_generated_version() {
    let license_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/fonts")
        .join(format!("Lucide-{LUCIDE_VERSION}.txt"));
    let notice = include_str!("../../NOTICE");

    assert!(
        license_path.is_file(),
        "Lucide license is missing at {}",
        license_path.display(),
    );
    assert!(
        notice.contains(&format!("Lucide {LUCIDE_VERSION}")),
        "NOTICE does not name Lucide {LUCIDE_VERSION}",
    );
}

#[test]
fn icon_names_round_trip_to_canonical_names() {
    assert_eq!(LucideIcon::Play.name(), "play");
    assert_eq!(LucideIcon::AArrowDown.name(), "a-arrow-down");
    assert_eq!(LucideIcon::Axis3d.name(), "axis-3d");
}

#[test]
fn resolved_icon_uses_its_solved_square_size() -> Result<(), String> {
    let (_, style) = Icon::new(LucideIcon::CircleCheck)
        .size(64.0)
        .resolved_text_for_box(FontId(7), 48.0);

    assert_eq!(style.font_id(), 7);
    assert_eq!(style.size(), 48.0);
    let mut fonts = FontRegistry::new().map_err(|error| error.to_string())?;
    let font_id = fonts
        .register_font("lucide", LUCIDE_FONT)
        .map_err(|error| error.to_string())?;
    report_default_icon_ink_moves(&fonts, font_id)
}

/// Lists changed icon origins for the repair review. This reads the shipped
/// outlines through the same font registry used to measure the reified run.
fn report_default_icon_ink_moves(
    fonts: &crate::FontRegistry,
    font_id: FontId,
) -> Result<(), String> {
    let mut moves = Vec::new();
    for glyph in ALL {
        let icon = Icon::new(glyph);
        let side = icon.size_value();
        let (text, style) = icon.resolved_text_for_box(font_id, side);
        let TextInkMeasurement::Visible(ink) = fonts
            .measure_ink(&text, &style.as_measure())
            .map_err(|error| error.to_string())?
        else {
            return Err(format!("{} has no visible icon ink", glyph.name()));
        };
        let shift = Vec2::splat(side * 0.5)
            - Vec2::new(
                f32::midpoint(ink.left, ink.right),
                f32::midpoint(ink.top, ink.bottom),
            );
        let pixels = shift / Unit::Pixels.to_points();
        if pixels.abs().max_element() >= 1.0 {
            moves.push(format!(
                "ICON_INK_MOVE {}: x={:.3}px y={:.3}px (x={:.3}pt y={:.3}pt, default={side}pt)",
                glyph.name(),
                pixels.x,
                pixels.y,
                shift.x,
                shift.y
            ));
        }
    }
    let total = moves.len();
    moves.push(format!(
        "ICON_INK_MOVE total={total}/{}; x right, y down; scale=1",
        ALL.len()
    ));
    let audit = moves.join("\n");
    eprintln!("{audit}");
    Ok(())
}
