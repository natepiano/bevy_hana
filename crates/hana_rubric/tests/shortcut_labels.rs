//! Platform shortcut hints preserve key words and use the standard modifier symbols.

use hana_rubric::Keystroke;
use hana_rubric::KeystrokeLabelPart;
use hana_rubric::KeystrokeLabelPart::Symbol;
use hana_rubric::KeystrokeLabelPart::Word;
use hana_rubric::KeystrokeParseError;
use hana_rubric::ModifierSymbol;

#[test]
fn shortcut_labels_use_standard_control_alt_and_shift_glyphs() -> Result<(), KeystrokeParseError> {
    for (shortcut, symbol) in [
        ("ctrl-home", ModifierSymbol::Control),
        ("alt-home", ModifierSymbol::Alt),
        ("shift-home", ModifierSymbol::Shift),
    ] {
        assert_shortcut_label(shortcut, &[Symbol(symbol), Word("Home".to_owned())])?;
    }
    Ok(())
}

#[test]
fn shortcut_labels_name_the_platform_modifier_correctly() -> Result<(), KeystrokeParseError> {
    assert_shortcut_label(
        "super-home",
        &[platform_modifier_part(), Word("Home".to_owned())],
    )
}

#[test]
fn shortcut_labels_put_all_modifiers_in_platform_order() -> Result<(), KeystrokeParseError> {
    // Deliberately author the chord in a different order from its on-screen hint.
    assert_shortcut_label(
        "super-shift-alt-ctrl-f11",
        &[
            Symbol(ModifierSymbol::Control),
            Symbol(ModifierSymbol::Alt),
            Symbol(ModifierSymbol::Shift),
            platform_modifier_part(),
            Word("F11".to_owned()),
        ],
    )
}

#[test]
fn shortcut_labels_omit_absent_modifiers_without_reordering_the_rest()
-> Result<(), KeystrokeParseError> {
    assert_shortcut_label(
        "super-alt-home",
        &[
            Symbol(ModifierSymbol::Alt),
            platform_modifier_part(),
            Word("Home".to_owned()),
        ],
    )
}

#[test]
fn shortcut_labels_keep_home_and_function_keys_as_words() -> Result<(), KeystrokeParseError> {
    for (shortcut, word) in [
        ("home", "Home"),
        ("f11", "F11"),
        ("up", "Up"),
        ("down", "Down"),
        ("left", "Left"),
        ("right", "Right"),
        ("equal", "Equal"),
        ("minus", "Minus"),
    ] {
        assert_shortcut_label(shortcut, &[Word(word.to_owned())])?;
    }
    Ok(())
}

#[test]
fn shortcut_labels_use_the_same_symbols_for_bare_modifier_triggers()
-> Result<(), KeystrokeParseError> {
    for (shortcut, symbol) in [
        ("ctrl", ModifierSymbol::Control),
        ("alt", ModifierSymbol::Alt),
        ("shift", ModifierSymbol::Shift),
    ] {
        assert_shortcut_label(shortcut, &[Symbol(symbol)])?;
    }
    assert_shortcut_label("super", &[platform_modifier_part()])
}

fn platform_modifier_part() -> KeystrokeLabelPart {
    if cfg!(target_os = "macos") {
        Symbol(ModifierSymbol::Command)
    } else if cfg!(target_os = "windows") {
        Word("Win".to_owned())
    } else {
        Word("Super".to_owned())
    }
}

fn assert_shortcut_label(
    shortcut: &str,
    expected: &[KeystrokeLabelPart],
) -> Result<(), KeystrokeParseError> {
    let label = shortcut.parse::<Keystroke>()?.label();
    assert_eq!(label.parts(), expected, "shortcut {shortcut:?}");
    Ok(())
}
