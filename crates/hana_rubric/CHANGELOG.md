# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
