# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-05

### Added

- Held shortcuts. `Keybindings::spawn_held_shortcut` returns a `HeldShortcut`
  with a `press` action and a `hold` action. The hold action continues until
  the primary input is released. A modifier change does not end the hold.
  Add `HeldShortcutPlugin` before you use it. `HoldLatch` connects the two
  actions.
- Toggle commands. A `command!` declaration takes `toggle_state: reader` after
  its description. `KeymapCommand::toggle_state` and
  `ReflectKeymapCommand::toggle_state` return a `CommandToggleState`: `On`,
  `Off` or `NotToggle`.
- Each toggle command's palette row shows its live On or Off state and updates while the palette is open.
- `CommandPaletteRow::title_match` returns a `PaletteTitleMatch`: the title
  characters that the query matched, for highlighting.
- `PaletteRowCursor`, `PaletteRowList` and `PaletteRowStep` keep the
  highlighted row and the visible rows of a palette list.
- `Keystroke::label` returns on-screen hint text: `secondary-p` reads `Cmd+P`
  on macOS and `Ctrl+P` elsewhere. `Modifiers::with_secondary` adds the
  platform shortcut modifier.

### Changed

- Requires Bevy 0.20.
- The command palette matches each word of a query. Title prefix matches come
  first, then word prefix matches, then other matches.
- Palette rows sort alphabetically and ignore letter case.
- A problem in a keymap file logs one plain message, "Your keyboard shortcut
  settings need attention.", with the details attached. Other diagnostics log
  at debug level.

## [0.1.0] - 2026-08-22

### Added

- Initial release. JSONC keymap loading, command registration, load
  diagnostics, keymap layering, and reload support for Bevy applications.
- The multi-dimensional state model that state-specific bindings are authored
  against, with a single registration mechanism and no alternative path.
- Keyboard input wiring, moved here from the `input` feature of `bevy_kana`
  0.3.0: the `Keybindings` builder with modifier-aware, platform-specific
  Cmd/Ctrl handling, and the `action!`, `event!`, and `bind_action_system!`
  macros. No items were renamed in the move.
