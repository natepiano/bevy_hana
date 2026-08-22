//! Canonical keyboard keystrokes and their parser.

mod sequence;
mod sequence_matcher;

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bitflags::bitflags;
pub use sequence::EmptyKeystrokeSequenceError;
pub use sequence::KeystrokeSequence;
pub use sequence::KeystrokeSequenceParseError;
pub use sequence_matcher::DeferredMatch;
pub use sequence_matcher::MatchOutcome;
pub use sequence_matcher::SequenceMatcher;
pub use sequence_matcher::TimeoutOutcome;

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct ModifierFlags: u8 {
        const CONTROL = 0b0001;
        const ALT = 0b0010;
        const SHIFT = 0b0100;
        const PLATFORM = 0b1000;
    }
}

/// Keyboard modifiers stored as physical key modifiers.
///
/// The platform modifier represents physical Super and remains distinct from physical Control on
/// every platform.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Modifiers(ModifierFlags);

impl Default for Modifiers {
    fn default() -> Self { Self::none() }
}

impl Modifiers {
    /// Returns the empty modifier set.
    #[must_use]
    pub const fn none() -> Self { Self(ModifierFlags::empty()) }

    /// Returns this set with control enabled.
    #[must_use]
    pub const fn with_control(self) -> Self {
        Self(ModifierFlags::from_bits_retain(
            self.0.bits() | ModifierFlags::CONTROL.bits(),
        ))
    }

    /// Returns this set with alt enabled.
    #[must_use]
    pub const fn with_alt(self) -> Self {
        Self(ModifierFlags::from_bits_retain(
            self.0.bits() | ModifierFlags::ALT.bits(),
        ))
    }

    /// Returns this set with shift enabled.
    #[must_use]
    pub const fn with_shift(self) -> Self {
        Self(ModifierFlags::from_bits_retain(
            self.0.bits() | ModifierFlags::SHIFT.bits(),
        ))
    }

    /// Returns this set with the physical Super modifier enabled.
    #[must_use]
    pub const fn with_platform(self) -> Self {
        Self(ModifierFlags::from_bits_retain(
            self.0.bits() | ModifierFlags::PLATFORM.bits(),
        ))
    }

    /// Reports whether control is enabled.
    #[must_use]
    pub const fn has_control(self) -> bool { self.0.contains(ModifierFlags::CONTROL) }

    /// Reports whether alt is enabled.
    #[must_use]
    pub const fn has_alt(self) -> bool { self.0.contains(ModifierFlags::ALT) }

    /// Reports whether shift is enabled.
    #[must_use]
    pub const fn has_shift(self) -> bool { self.0.contains(ModifierFlags::SHIFT) }

    /// Reports whether physical Super is enabled.
    #[must_use]
    pub const fn has_platform(self) -> bool { self.0.contains(ModifierFlags::PLATFORM) }

    /// Returns this modifier set's stable physical ordering parts.
    ///
    /// The bit positions intentionally follow Control, Alt, Shift, Platform rather than any
    /// platform-facing display order. Representative key bindings use this order while they are
    /// committed, so their selection never depends on formatted text.
    const fn structural_order(self) -> (u32, u8) { (self.0.bits().count_ones(), self.0.bits()) }

    /// Creates canonical modifiers from the modifier keys currently held down.
    ///
    /// Physical Control keys set control and physical Super keys set platform on every platform.
    #[must_use]
    pub fn from_pressed(pressed: &ButtonInput<KeyCode>) -> Self {
        let mut modifiers = Self::none();

        if modifier_pair_pressed(pressed, KeyCode::AltLeft, KeyCode::AltRight) {
            modifiers = modifiers.with_alt();
        }
        if modifier_pair_pressed(pressed, KeyCode::ShiftLeft, KeyCode::ShiftRight) {
            modifiers = modifiers.with_shift();
        }

        if modifier_pair_pressed(pressed, KeyCode::ControlLeft, KeyCode::ControlRight) {
            modifiers = modifiers.with_control();
        }
        if modifier_pair_pressed(pressed, KeyCode::SuperLeft, KeyCode::SuperRight) {
            modifiers = modifiers.with_platform();
        }

        modifiers
    }

    fn insert(&mut self, modifier: Modifier) {
        match modifier {
            Modifier::Control => self.0.insert(ModifierFlags::CONTROL),
            Modifier::Alt => self.0.insert(ModifierFlags::ALT),
            Modifier::Shift => self.0.insert(ModifierFlags::SHIFT),
            Modifier::Platform => *self = self.with_platform(),
        }
    }
}

fn modifier_pair_pressed(pressed: &ButtonInput<KeyCode>, left: KeyCode, right: KeyCode) -> bool {
    pressed.pressed(left) || pressed.pressed(right)
}

/// A semantic modifier family that can act as a keystroke's primary trigger.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModifierFamily {
    /// The physical Control modifier family.
    Control,
    /// The physical Alt modifier family.
    Alt,
    /// The physical Shift modifier family.
    Shift,
    /// The physical Super modifier family.
    ///
    /// The parse-side `secondary` alias resolves to this family on macOS.
    Platform,
}

impl ModifierFamily {
    const fn keymap_name(self) -> &'static str {
        match self {
            Self::Control => "ctrl",
            Self::Alt => "alt",
            Self::Shift => "shift",
            // Rendered under the running OS's own name for the physical Super key. `cmd`,
            // `super`, `win`, and `platform` all parse back to this family, so the round trip
            // holds on either platform.
            Self::Platform => {
                if cfg!(target_os = "macos") {
                    "cmd"
                } else {
                    "super"
                }
            },
        }
    }

    const fn structural_ordinal(self) -> usize {
        match self {
            Self::Control => 0,
            Self::Alt => 1,
            Self::Shift => 2,
            Self::Platform => 3,
        }
    }
}

impl Display for ModifierFamily {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.keymap_name())
    }
}

impl FromStr for ModifierFamily {
    type Err = KeystrokeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        parse_modifier_family_primary(input).ok_or_else(|| KeystrokeParseError::new(input, 0))
    }
}

/// A physical non-modifier key supported by Hana keymaps.
///
/// Construction rejects physical modifier keys and `KeyCode` values without a canonical keymap
/// spelling, so every value can be routed, displayed, and parsed back.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OrdinaryKey(KeyCode);

impl OrdinaryKey {
    /// How many distinct key codes an [`OrdinaryKey`] can hold.
    ///
    /// One per entry in `CANONICAL_KEY_MAPPINGS`, which is the whole set of key codes routing
    /// can reach — every other key code fails construction.
    pub(crate) const COUNT: usize = CANONICAL_KEY_MAPPINGS.len();

    /// The `P` key, for applications that build a recovery chord in a `const` context.
    ///
    /// [`TryFrom<KeyCode>`](Self::try_from) is the general constructor, but it is fallible and
    /// non-`const`, so a `const` site cannot use it without unwrapping.
    pub const KEY_P: Self = Self(KeyCode::KeyP);

    /// Returns the validated physical key code.
    #[must_use]
    pub const fn key_code(self) -> KeyCode { self.0 }

    fn structural_ordinal(self) -> usize {
        CANONICAL_KEY_MAPPINGS
            .iter()
            .position(|(_, key_code)| *key_code == self.0)
            .unwrap_or_default()
    }
}

impl TryFrom<KeyCode> for OrdinaryKey {
    type Error = InvalidOrdinaryKeyCode;

    fn try_from(key_code: KeyCode) -> Result<Self, Self::Error> {
        ordinary_key_name(key_code).map_or_else(
            || Err(InvalidOrdinaryKeyCode(key_code)),
            |_| Ok(Self(key_code)),
        )
    }
}

impl Display for OrdinaryKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(ordinary_key_name(self.0).ok_or(fmt::Error)?)
    }
}

impl FromStr for OrdinaryKey {
    type Err = KeystrokeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        parse_ordinary_key(input).ok_or_else(|| KeystrokeParseError::new(input, 0))
    }
}

/// A physical key code that cannot serve as an ordinary Hana keymap key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidOrdinaryKeyCode(KeyCode);

impl InvalidOrdinaryKeyCode {
    /// Returns the rejected physical key code.
    #[must_use]
    pub const fn key_code(self) -> KeyCode { self.0 }
}

impl Display for InvalidOrdinaryKeyCode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "physical key code {:?} is not a supported ordinary Hana keymap key",
            self.0
        )
    }
}

impl Error for InvalidOrdinaryKeyCode {}

/// The semantic trigger that completes a keystroke after its modifier set.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PrimaryTrigger {
    /// An ordinary physical key with zero or more canonical modifiers.
    OrdinaryKey(OrdinaryKey),
    /// A bare modifier family with no other modifiers.
    ModifierFamily(ModifierFamily),
}

impl PrimaryTrigger {
    fn structural_order(self) -> (u8, usize) {
        match self {
            Self::OrdinaryKey(ordinary_key) => (0, ordinary_key.structural_ordinal()),
            Self::ModifierFamily(modifier_family) => (1, modifier_family.structural_ordinal()),
        }
    }
}

impl Display for PrimaryTrigger {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::OrdinaryKey(ordinary_key) => ordinary_key.fmt(formatter),
            Self::ModifierFamily(modifier_family) => modifier_family.fmt(formatter),
        }
    }
}

impl FromStr for PrimaryTrigger {
    type Err = KeystrokeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if let Some(modifier_family) = parse_modifier_family_primary(input) {
            return Ok(Self::ModifierFamily(modifier_family));
        }

        parse_ordinary_key(input)
            .map(Self::OrdinaryKey)
            .ok_or_else(|| KeystrokeParseError::new(input, 0))
    }
}

/// A keyboard primary trigger and its canonical modifier set.
///
/// Parse a [`Keystroke`] from text such as `"platform-shift-p"`. The parser canonicalizes
/// modifier aliases and source ordering before the value is constructed. A bare `shift`, `ctrl`,
/// `alt`, or `secondary` names a [`PrimaryTrigger::ModifierFamily`]; `shift-f` instead has an
/// [`PrimaryTrigger::OrdinaryKey`] with Shift in its modifier set.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Keystroke {
    modifiers:       Modifiers,
    primary_trigger: PrimaryTrigger,
}

impl Keystroke {
    /// Creates a keystroke with an ordinary physical key as its primary trigger.
    #[must_use]
    pub const fn from_ordinary_key(modifiers: Modifiers, ordinary_key: OrdinaryKey) -> Self {
        Self {
            modifiers,
            primary_trigger: PrimaryTrigger::OrdinaryKey(ordinary_key),
        }
    }

    /// Creates a keystroke with a bare modifier family as its primary trigger.
    #[must_use]
    pub const fn from_modifier_family(modifier_family: ModifierFamily) -> Self {
        Self {
            modifiers:       Modifiers::none(),
            primary_trigger: PrimaryTrigger::ModifierFamily(modifier_family),
        }
    }

    /// Returns this keystroke's canonical modifiers.
    #[must_use]
    pub const fn modifiers(self) -> Modifiers { self.modifiers }

    /// Returns the semantic trigger that completes this keystroke.
    #[must_use]
    pub const fn primary_trigger(self) -> PrimaryTrigger { self.primary_trigger }

    /// Compares keystrokes by their platform-independent binding representation.
    fn structural_cmp(self, other: Self) -> Ordering {
        let (modifier_count, modifier_bits) = self.modifiers.structural_order();
        let (other_modifier_count, other_modifier_bits) = other.modifiers.structural_order();

        modifier_count
            .cmp(&other_modifier_count)
            .then_with(|| modifier_bits.cmp(&other_modifier_bits))
            .then_with(|| {
                self.primary_trigger
                    .structural_order()
                    .cmp(&other.primary_trigger.structural_order())
            })
    }
}

impl Display for Keystroke {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        if self.modifiers.has_platform() {
            write!(formatter, "{}-", ModifierFamily::Platform)?;
        }
        if self.modifiers.has_control() {
            write!(formatter, "{}-", ModifierFamily::Control)?;
        }
        if self.modifiers.has_alt() {
            write!(formatter, "{}-", ModifierFamily::Alt)?;
        }
        if self.modifiers.has_shift() {
            write!(formatter, "{}-", ModifierFamily::Shift)?;
        }

        self.primary_trigger.fmt(formatter)
    }
}

impl FromStr for Keystroke {
    type Err = KeystrokeParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if let Some(modifier_family) = parse_modifier_family_primary(input) {
            return Ok(Self::from_modifier_family(modifier_family));
        }

        let mut modifiers = Modifiers::none();
        let mut remaining = input;

        loop {
            let Some((token, after_token)) = remaining.split_once('-') else {
                let offset = input.len() - remaining.len();
                let Some(ordinary_key) = parse_ordinary_key(remaining) else {
                    return Err(KeystrokeParseError::new(remaining, offset));
                };
                return Ok(Self::from_ordinary_key(modifiers, ordinary_key));
            };
            if after_token.is_empty() {
                return Err(KeystrokeParseError::new("", input.len()));
            }
            let token_offset = input.len() - remaining.len();
            let Some(modifier) = parse_modifier(token) else {
                return Err(KeystrokeParseError::new(token, token_offset));
            };
            modifiers.insert(modifier);
            remaining = after_token;

            if let Some((side, after_side)) = remaining.split_once('-')
                && matches!(side, "left" | "right")
            {
                remaining = after_side;
            }
        }
    }
}

/// A keystroke parse failure with the source token and its byte offset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeystrokeParseError {
    token:  String,
    offset: usize,
}

impl KeystrokeParseError {
    fn new(token: &str, offset: usize) -> Self {
        Self {
            token: token.to_owned(),
            offset,
        }
    }

    /// Returns the unrecognized source token.
    #[must_use]
    pub fn token(&self) -> &str { &self.token }

    /// Returns the offending token's byte offset in the parsed string.
    #[must_use]
    pub const fn offset(&self) -> usize { self.offset }

    pub(super) const fn at_offset(mut self, offset: usize) -> Self {
        self.offset += offset;
        self
    }
}

impl Display for KeystrokeParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid keystroke token {:?} at byte offset {}",
            self.token, self.offset
        )
    }
}

impl Error for KeystrokeParseError {}

#[derive(Clone, Copy)]
enum Modifier {
    Control,
    Alt,
    Shift,
    Platform,
}

fn parse_modifier(token: &str) -> Option<Modifier> {
    match token {
        "ctrl" | "control" | "ctrlleft" | "ctrlright" | "controlleft" | "controlright" => {
            Some(Modifier::Control)
        },
        "alt" | "opt" | "option" | "altleft" | "altright" | "optleft" | "optright"
        | "optionleft" | "optionright" => Some(Modifier::Alt),
        "shift" | "shiftleft" | "shiftright" => Some(Modifier::Shift),
        "secondary" => Some(if cfg!(target_os = "macos") {
            Modifier::Platform
        } else {
            Modifier::Control
        }),
        "cmd" | "command" | "super" | "win" | "platform" | "cmdleft" | "cmdright"
        | "commandleft" | "commandright" | "superleft" | "superright" | "winleft" | "winright"
        | "platformleft" | "platformright" => Some(Modifier::Platform),
        _ => None,
    }
}

fn parse_modifier_family_primary(token: &str) -> Option<ModifierFamily> {
    match token {
        "ctrl" | "control" => Some(ModifierFamily::Control),
        "alt" | "opt" | "option" => Some(ModifierFamily::Alt),
        "shift" => Some(ModifierFamily::Shift),
        "secondary" => Some(if cfg!(target_os = "macos") {
            ModifierFamily::Platform
        } else {
            ModifierFamily::Control
        }),
        "cmd" | "command" | "super" | "win" | "platform" => Some(ModifierFamily::Platform),
        _ => None,
    }
}

macro_rules! define_canonical_key_mappings {
    ($(($name:literal, $key_code:path)),+ $(,)?) => {
        const CANONICAL_KEY_MAPPINGS: &[(&str, KeyCode)] = &[
            $(($name, $key_code),)+
        ];

        const fn ordinary_key_name(key_code: KeyCode) -> Option<&'static str> {
            match key_code {
                $($key_code => Some($name),)+
                _ => None,
            }
        }

    };
}

define_canonical_key_mappings! {
    ("a", KeyCode::KeyA),
    ("b", KeyCode::KeyB),
    ("c", KeyCode::KeyC),
    ("d", KeyCode::KeyD),
    ("e", KeyCode::KeyE),
    ("f", KeyCode::KeyF),
    ("g", KeyCode::KeyG),
    ("h", KeyCode::KeyH),
    ("i", KeyCode::KeyI),
    ("j", KeyCode::KeyJ),
    ("k", KeyCode::KeyK),
    ("l", KeyCode::KeyL),
    ("m", KeyCode::KeyM),
    ("n", KeyCode::KeyN),
    ("o", KeyCode::KeyO),
    ("p", KeyCode::KeyP),
    ("q", KeyCode::KeyQ),
    ("r", KeyCode::KeyR),
    ("s", KeyCode::KeyS),
    ("t", KeyCode::KeyT),
    ("u", KeyCode::KeyU),
    ("v", KeyCode::KeyV),
    ("w", KeyCode::KeyW),
    ("x", KeyCode::KeyX),
    ("y", KeyCode::KeyY),
    ("z", KeyCode::KeyZ),
    ("0", KeyCode::Digit0),
    ("1", KeyCode::Digit1),
    ("2", KeyCode::Digit2),
    ("3", KeyCode::Digit3),
    ("4", KeyCode::Digit4),
    ("5", KeyCode::Digit5),
    ("6", KeyCode::Digit6),
    ("7", KeyCode::Digit7),
    ("8", KeyCode::Digit8),
    ("9", KeyCode::Digit9),
    ("f1", KeyCode::F1),
    ("f2", KeyCode::F2),
    ("f3", KeyCode::F3),
    ("f4", KeyCode::F4),
    ("f5", KeyCode::F5),
    ("f6", KeyCode::F6),
    ("f7", KeyCode::F7),
    ("f8", KeyCode::F8),
    ("f9", KeyCode::F9),
    ("f10", KeyCode::F10),
    ("f11", KeyCode::F11),
    ("f12", KeyCode::F12),
    ("f13", KeyCode::F13),
    ("f14", KeyCode::F14),
    ("f15", KeyCode::F15),
    ("f16", KeyCode::F16),
    ("f17", KeyCode::F17),
    ("f18", KeyCode::F18),
    ("f19", KeyCode::F19),
    ("f20", KeyCode::F20),
    ("f21", KeyCode::F21),
    ("f22", KeyCode::F22),
    ("f23", KeyCode::F23),
    ("f24", KeyCode::F24),
    ("f25", KeyCode::F25),
    ("f26", KeyCode::F26),
    ("f27", KeyCode::F27),
    ("f28", KeyCode::F28),
    ("f29", KeyCode::F29),
    ("f30", KeyCode::F30),
    ("f31", KeyCode::F31),
    ("f32", KeyCode::F32),
    ("f33", KeyCode::F33),
    ("f34", KeyCode::F34),
    ("f35", KeyCode::F35),
    ("up", KeyCode::ArrowUp),
    ("down", KeyCode::ArrowDown),
    ("left", KeyCode::ArrowLeft),
    ("right", KeyCode::ArrowRight),
    ("escape", KeyCode::Escape),
    ("enter", KeyCode::Enter),
    ("tab", KeyCode::Tab),
    ("space", KeyCode::Space),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
    ("insert", KeyCode::Insert),
    ("capslock", KeyCode::CapsLock),
    ("contextmenu", KeyCode::ContextMenu),
    ("printscreen", KeyCode::PrintScreen),
    ("scrolllock", KeyCode::ScrollLock),
    ("pause", KeyCode::Pause),
    ("numlock", KeyCode::NumLock),
    ("backquote", KeyCode::Backquote),
    ("backslash", KeyCode::Backslash),
    ("bracketleft", KeyCode::BracketLeft),
    ("bracketright", KeyCode::BracketRight),
    ("comma", KeyCode::Comma),
    ("equal", KeyCode::Equal),
    ("minus", KeyCode::Minus),
    ("period", KeyCode::Period),
    ("quote", KeyCode::Quote),
    ("semicolon", KeyCode::Semicolon),
    ("slash", KeyCode::Slash),
}

fn parse_ordinary_key(token: &str) -> Option<OrdinaryKey> {
    if let Some((_, key)) = CANONICAL_KEY_MAPPINGS
        .iter()
        .find(|(name, _)| *name == token)
    {
        return Some(OrdinaryKey(*key));
    }

    let canonical_name = match token {
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "esc" => "escape",
        "return" => "enter",
        "del" => "delete",
        "pgup" => "pageup",
        "pgdown" => "pagedown",
        "ins" => "insert",
        "prtsc" => "printscreen",
        "grave" => "backquote",
        "lbracket" => "bracketleft",
        "rbracket" => "bracketright",
        "equals" => "equal",
        "dot" => "period",
        "apostrophe" => "quote",
        _ => return None,
    };

    if let Some((_, key)) = CANONICAL_KEY_MAPPINGS
        .iter()
        .find(|(name, _)| *name == canonical_name)
    {
        return Some(OrdinaryKey(*key));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;

    use super::CANONICAL_KEY_MAPPINGS;
    use super::InvalidOrdinaryKeyCode;
    use super::Keystroke;
    use super::KeystrokeParseError;
    use super::ModifierFamily;
    use super::Modifiers;
    use super::OrdinaryKey;
    use super::PrimaryTrigger;

    fn parsed(input: &str) -> Result<Keystroke, KeystrokeParseError> { input.parse() }

    fn parse_error(input: &str) -> Result<KeystrokeParseError, KeystrokeParseError> {
        match parsed(input) {
            Ok(_) => Err(KeystrokeParseError::new(input, 0)),
            Err(error) => Ok(error),
        }
    }

    fn pressed(keys: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut pressed = ButtonInput::default();
        for key in keys {
            pressed.press(*key);
        }
        pressed
    }

    fn ordinary_keystroke(
        modifiers: Modifiers,
        key_code: KeyCode,
    ) -> Result<Keystroke, super::InvalidOrdinaryKeyCode> {
        OrdinaryKey::try_from(key_code)
            .map(|ordinary_key| Keystroke::from_ordinary_key(modifiers, ordinary_key))
    }

    #[test]
    fn modifier_order_and_platform_aliases_are_canonical() -> Result<(), KeystrokeParseError> {
        let canonical = parsed("cmd-shift-p")?;

        assert_eq!(canonical, parsed("shift-cmd-p")?);
        assert_eq!(canonical, parsed("shift-super-p")?);
        assert_eq!(canonical, parsed("shift-win-p")?);
        assert_eq!(canonical, parsed("shift-platform-p")?);

        Ok(())
    }

    #[test]
    fn control_and_platform_are_distinct_on_every_platform() -> Result<(), KeystrokeParseError> {
        assert_ne!(parsed("ctrl-p")?, parsed("platform-p")?);

        Ok(())
    }

    #[test]
    fn constructed_platform_keystroke_equals_parsed_command_on_every_platform()
    -> Result<(), Box<dyn Error>> {
        let constructed = ordinary_keystroke(
            Modifiers::none().with_platform().with_shift(),
            KeyCode::KeyP,
        )?;
        let parsed = parsed("cmd-shift-p")?;

        assert_eq!(constructed, parsed);
        assert_eq!(parsed, constructed);

        Ok(())
    }

    #[test]
    fn supported_ordinary_key_constructs_and_round_trips() -> Result<(), Box<dyn Error>> {
        let ordinary_key = OrdinaryKey::try_from(KeyCode::KeyP)?;
        let keystroke =
            Keystroke::from_ordinary_key(Modifiers::none().with_control(), ordinary_key);

        assert_eq!(ordinary_key.key_code(), KeyCode::KeyP);
        assert_eq!(keystroke, parsed("ctrl-p")?);
        assert_eq!(keystroke.to_string().parse::<Keystroke>()?, keystroke);

        Ok(())
    }

    #[test]
    fn ordinary_key_construction_rejects_modifier_key_codes() {
        assert_eq!(
            OrdinaryKey::try_from(KeyCode::ShiftLeft).map(OrdinaryKey::key_code),
            Err(InvalidOrdinaryKeyCode(KeyCode::ShiftLeft))
        );
    }

    #[test]
    fn ordinary_key_construction_rejects_unsupported_key_codes() {
        assert_eq!(
            OrdinaryKey::try_from(KeyCode::AudioVolumeUp).map(OrdinaryKey::key_code),
            Err(InvalidOrdinaryKeyCode(KeyCode::AudioVolumeUp))
        );
    }

    #[test]
    fn left_and_right_modifier_spellings_are_canonical() -> Result<(), KeystrokeParseError> {
        assert_eq!(parsed("ctrl-left-p")?, parsed("control-right-p")?);
        assert_eq!(parsed("alt-left-p")?, parsed("option-right-p")?);
        assert_eq!(parsed("shift-left-p")?, parsed("shiftright-p")?);
        assert_eq!(parsed("cmd-left-p")?, parsed("superright-p")?);

        Ok(())
    }

    #[test]
    fn physical_control_g_matches_ctrl_g_on_every_platform() -> Result<(), Box<dyn Error>> {
        let keys = pressed(&[KeyCode::ControlLeft, KeyCode::KeyG]);
        let physical = ordinary_keystroke(Modifiers::from_pressed(&keys), KeyCode::KeyG)?;

        assert_eq!(physical, parsed("ctrl-g")?);
        assert!(physical.modifiers.has_control());
        assert!(!physical.modifiers.has_platform());
        assert_ne!(physical, parsed("platform-g")?);

        Ok(())
    }

    #[test]
    fn physical_control_and_super_g_preserve_both_modifiers() -> Result<(), Box<dyn Error>> {
        let keys = pressed(&[KeyCode::ControlLeft, KeyCode::SuperLeft, KeyCode::KeyG]);
        let physical = ordinary_keystroke(Modifiers::from_pressed(&keys), KeyCode::KeyG)?;

        assert_eq!(physical, parsed("platform-ctrl-g")?);
        assert!(physical.modifiers.has_control());
        assert!(physical.modifiers.has_platform());
        assert_ne!(physical, parsed("ctrl-g")?);
        assert_ne!(physical, parsed("platform-g")?);

        Ok(())
    }

    #[test]
    fn secondary_g_matches_the_platform_shortcut_modifier() -> Result<(), Box<dyn Error>> {
        let keys = if cfg!(target_os = "macos") {
            pressed(&[KeyCode::SuperLeft, KeyCode::KeyG])
        } else {
            pressed(&[KeyCode::ControlLeft, KeyCode::KeyG])
        };
        let physical = ordinary_keystroke(Modifiers::from_pressed(&keys), KeyCode::KeyG)?;
        let secondary = parsed("secondary-g")?;
        let display = secondary.to_string();

        assert_eq!(physical, secondary);
        if cfg!(target_os = "macos") {
            assert_eq!(display, "cmd-g");
        } else {
            assert_eq!(display, "ctrl-g");
        }
        assert!(!display.contains("secondary-"));
        assert_eq!(secondary, parsed(&display)?);

        Ok(())
    }

    #[test]
    fn pressed_alt_and_shift_match_parsed_modifiers() -> Result<(), KeystrokeParseError> {
        let keys = pressed(&[KeyCode::AltLeft, KeyCode::ShiftRight]);

        assert_eq!(
            Modifiers::from_pressed(&keys),
            parsed("alt-shift-p")?.modifiers()
        );

        Ok(())
    }

    #[test]
    fn physical_super_g_matches_platform_aliases_without_matching_ctrl_or_bare_g()
    -> Result<(), Box<dyn Error>> {
        let keys = pressed(&[KeyCode::SuperLeft, KeyCode::KeyG]);
        let physical = ordinary_keystroke(Modifiers::from_pressed(&keys), KeyCode::KeyG)?;

        assert_eq!(physical, parsed("cmd-g")?);
        assert_eq!(physical, parsed("super-g")?);
        assert_eq!(physical, parsed("win-g")?);
        assert_ne!(physical, parsed("ctrl-g")?);
        assert_ne!(physical, parsed("g")?);

        Ok(())
    }

    #[test]
    fn every_constructible_primary_trigger_round_trips() -> Result<(), Box<dyn Error>> {
        for &(name, key) in CANONICAL_KEY_MAPPINGS {
            let ordinary_key = OrdinaryKey::try_from(key)?;
            let primary_trigger = PrimaryTrigger::OrdinaryKey(ordinary_key);
            let keystroke = parsed(name)?;

            assert_eq!(
                ordinary_key.to_string().parse::<OrdinaryKey>()?,
                ordinary_key
            );
            assert_eq!(
                primary_trigger.to_string().parse::<PrimaryTrigger>()?,
                primary_trigger
            );
            assert_eq!(keystroke.primary_trigger(), primary_trigger);
            assert_eq!(keystroke, parsed(&keystroke.to_string())?);
        }

        for modifier_family in [
            ModifierFamily::Control,
            ModifierFamily::Alt,
            ModifierFamily::Shift,
            ModifierFamily::Platform,
        ] {
            let primary_trigger = PrimaryTrigger::ModifierFamily(modifier_family);

            assert_eq!(
                modifier_family.to_string().parse::<ModifierFamily>()?,
                modifier_family
            );
            assert_eq!(
                primary_trigger.to_string().parse::<PrimaryTrigger>()?,
                primary_trigger
            );
            assert_eq!(
                Keystroke::from_modifier_family(modifier_family)
                    .to_string()
                    .parse::<Keystroke>()?
                    .primary_trigger(),
                primary_trigger
            );
        }

        Ok(())
    }

    #[test]
    fn physical_modifier_keys_cannot_be_constructed_as_ordinary_primary_keys() -> Result<(), String>
    {
        for key_code in [
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ] {
            let Err(error) = OrdinaryKey::try_from(key_code) else {
                return Err(format!(
                    "physical modifier {key_code:?} was accepted as an ordinary key"
                ));
            };
            assert_eq!(error.key_code(), key_code);
        }

        Ok(())
    }

    #[test]
    fn invalid_tokens_preserve_their_offsets() -> Result<(), KeystrokeParseError> {
        let empty = parse_error("")?;
        assert_eq!(empty.token(), "");
        assert_eq!(empty.offset(), 0);

        let unknown_key = parse_error("cmd-unknown")?;
        assert_eq!(unknown_key.token(), "unknown");
        assert_eq!(unknown_key.offset(), 4);

        let unknown_modifier = parse_error("cmd-unknown-p")?;
        assert_eq!(unknown_modifier.token(), "unknown");
        assert_eq!(unknown_modifier.offset(), 4);

        let empty_leading_token = parse_error("-p")?;
        assert_eq!(empty_leading_token.token(), "");
        assert_eq!(empty_leading_token.offset(), 0);

        let trailing_separator = parse_error("p-")?;
        assert_eq!(trailing_separator.token(), "");
        assert_eq!(trailing_separator.offset(), 2);

        let trailing_separator_after_modifier = parse_error("cmd-p-")?;
        assert_eq!(trailing_separator_after_modifier.token(), "");
        assert_eq!(trailing_separator_after_modifier.offset(), 6);

        let trailing_separator_after_control = parse_error("ctrl-")?;
        assert_eq!(trailing_separator_after_control.token(), "");
        assert_eq!(trailing_separator_after_control.offset(), 5);

        Ok(())
    }

    #[test]
    fn side_token_is_a_key_only_when_no_key_follows() -> Result<(), Box<dyn Error>> {
        let control = Modifiers::none().with_control();

        assert_eq!(
            parsed("ctrl-left")?,
            ordinary_keystroke(control, KeyCode::ArrowLeft)?
        );
        assert_eq!(
            parsed("ctrl-left-p")?,
            ordinary_keystroke(control, KeyCode::KeyP)?
        );

        Ok(())
    }

    #[test]
    fn bare_modifier_families_are_distinct_from_modified_keys() -> Result<(), KeystrokeParseError> {
        assert_eq!(
            parsed("shift")?.primary_trigger(),
            PrimaryTrigger::ModifierFamily(ModifierFamily::Shift)
        );
        assert_eq!(parsed("shift")?.modifiers(), Modifiers::none());
        assert_eq!(
            parsed("shift-f")?.primary_trigger(),
            PrimaryTrigger::OrdinaryKey(
                OrdinaryKey::try_from(KeyCode::KeyF)
                    .map_err(|error| KeystrokeParseError::new(&error.to_string(), 0))?
            )
        );
        assert!(parsed("shift-f")?.modifiers().has_shift());
        assert_ne!(parsed("shift")?, parsed("shift-f")?);
        assert_eq!(parsed("shift")?.to_string(), "shift");
        assert_eq!(parsed("ctrl")?.to_string(), "ctrl");
        assert_eq!(parsed("alt")?.to_string(), "alt");
        assert_eq!(
            parsed("secondary")?.to_string(),
            if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            }
        );

        let platform_name = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "super"
        };
        for alias in ["cmd", "super", "win", "platform"] {
            let rendered = parsed(alias)?.to_string();
            assert_eq!(rendered, platform_name);
            assert_eq!(parsed(&rendered)?, parsed(alias)?);
        }

        Ok(())
    }

    #[test]
    fn platform_modified_keys_render_under_the_running_platform_name() -> Result<(), Box<dyn Error>>
    {
        let platform_p = Keystroke::from_ordinary_key(
            Modifiers::none().with_platform(),
            OrdinaryKey::try_from(KeyCode::KeyP)?,
        );
        let rendered = platform_p.to_string();

        assert_eq!(
            rendered,
            if cfg!(target_os = "macos") {
                "cmd-p"
            } else {
                "super-p"
            }
        );
        assert_eq!(parsed(&rendered)?, platform_p);

        let secondary_p = parsed("secondary-p")?;
        let secondary_rendered = secondary_p.to_string();
        assert_eq!(
            secondary_rendered,
            if cfg!(target_os = "macos") {
                "cmd-p"
            } else {
                "ctrl-p"
            }
        );
        assert_eq!(parsed(&secondary_rendered)?, secondary_p);

        Ok(())
    }

    #[test]
    fn key_p_constant_matches_the_fallible_conversion() -> Result<(), Box<dyn Error>> {
        assert_eq!(OrdinaryKey::KEY_P, OrdinaryKey::try_from(KeyCode::KeyP)?);
        assert_eq!(OrdinaryKey::KEY_P.key_code(), KeyCode::KeyP);
        assert_eq!(OrdinaryKey::KEY_P.to_string(), "p");

        Ok(())
    }
}
