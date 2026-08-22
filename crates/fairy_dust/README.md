# fairy_dust

Workspace example helper — quiet logs and opt-in dev capabilities for bevy_hana examples.

Use `sprinkle_example()` to construct a `SprinkleBuilder` before Fairy Dust installs
its baseline plugins. The first builder operation installs `DefaultPlugins` with a
quiet log filter, then the chain opts into specific dev conveniences:

```rust,ignore
fairy_dust::sprinkle_example()
    .with_orbit_cam_preset(
        |orbit_cam| { orbit_cam.radius = Some(5.0); },
        OrbitCamPreset::blender_like(),
    )
    .with_stable_transparency()       // only callable after with_orbit_cam_*
    .with_save_window_position()
    .with_brp_extras()
    .with_camera_control_panel()
    .add_systems(Startup, setup)
    .run();
```

`.with_command_palette()` adds a searchable box over every command declared with
`hana_rubric`'s `command!` macro, opened with `Cmd+P` (`Ctrl+P` off macOS).
Typing matches a command's authored title or its raw id, and the selected row is
dispatched through `CommandRegistry`. Each row prints the keystroke the live
keymap runs that command from, or nothing when the command is unbound.

It installs `KeymapPlugin` with Fairy Dust's embedded default keymap — which
binds Fairy Dust's authored capabilities while Cmd/Ctrl+P remains a protected
direct palette recovery chord — and no application name,
so no keymap file is read from or written to the developer's config directory,
and the palette reports the unresolved configuration directory as a row above
the query field.

An application that declares its own commands owns its own document, because a
keymap naming an id the registry has not declared is rejected whole and leaves
the palette unopenable:

```rust
.with_command_palette_keymap(
    CommandPaletteKeymap::new(include_str!("my_app.keymap.jsonc"))
        .for_application("my_app"),
)
```

`.for_application` is what makes the user's own `keymap.jsonc` readable and
writable, which also clears the unresolved-configuration-directory row. Call
`with_command_palette` or `with_command_palette_keymap` once — a second call
carrying a different keymap panics rather than silently keeping the first.

When `.with_camera_home()` is installed, the home fit can be positioned in the
viewport with `.anchor(Anchor::TopLeft)` and adjusted with
`.offset_px(Vec2::new(x, y))`. The default remains centered with no pixel
offset.

## Typestate

The builder has independent baseline and orbit-camera typestates. The baseline
starts as `AssetRootPending`, where `SprinkleBuilder::with_asset_root` can configure
`AssetPlugin` before `DefaultPlugins` is installed. That method and every ordinary
builder operation return `BaselineInstalled`, where `with_asset_root` is no longer
available.

Code that needs direct access to the Bevy app before adding a Fairy Dust capability
must complete the first-step transition explicitly:

```rust,ignore
let mut builder = fairy_dust::sprinkle_example().with_default_asset_root();
let app = builder.app_mut();
```

The pending builder intentionally has no `app_mut` method, which keeps asset-root
selection ahead of any operation that can initialize Bevy's asset plugin.

The orbit-camera marker starts as `NoOrbitCam` and transitions to `WithOrbitCam`.
Methods that act on the spawned `OrbitCam` entity (such as
`SprinkleBuilder::with_stable_transparency`) are only defined on
`SprinkleBuilder<WithOrbitCam>`, so calling them before
`SprinkleBuilder::with_orbit_cam_preset` is a compile error.

## Plugin deduplication

Capabilities that share infrastructure (for example a `DiegeticUiPlugin` for HUD panels)
ensure the required plugin is registered exactly once via `ensure_plugin`, regardless of
how many capabilities pull it in.

## License

Dual-licensed under either of [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0)
or [MIT license](https://opensource.org/licenses/MIT) at your option.
