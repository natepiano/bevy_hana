# hana_diegetic

Diegetic UI layout engine for Bevy -- in-world panels driven by a Clay-inspired layout algorithm.

> **Work in progress.** This crate is in active development (v0.0.1) and not
> subject to semver stability guarantees. APIs will change without notice
> between commits. Do not depend on this in production code yet.

## What it does

`hana_diegetic` renders UI panels and text directly in 3D space as regular
Bevy entities -- no screen-space overlay, no separate UI camera. Text is
rendered with the slug glyph backend, which builds one mesh per text run
from quadratic Bézier contours and computes analytic per-pixel coverage in
the shader.

- **Retained-mode layout** inspired by [Clay](https://github.com/nicbarker/clay) -- build a `LayoutTree` once, recompute only when it changes
- **Slug text rendering** -- one mesh per run from Bézier contours with analytic per-pixel coverage, and physical font sizing
- **OpenType features** -- ligatures, contextual alternates, discretionary ligatures, kerning
- **Multiple font families** -- load fonts via Bevy `AssetServer`, render with per-element font selection

## Quick start

```rust
use bevy::prelude::*;
use hana_diegetic::*;

App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(DiegeticUiPlugin)
    .add_systems(Startup, setup)
    .run();
```

## Headless widget tests

Client tests can install retained layout, widgets, tooltips, focus, and IME
behavior without creating Bevy's render sub-app:

```rust
use bevy::prelude::*;
use hana_diegetic::*;

let mut app = App::new();
app.add_plugins(MinimalPlugins)
    .insert_resource(DiegeticTextMeasurer::default())
    .add_plugins(HeadlessDiegeticUiPlugin);
```

`HeadlessLayoutPlugin` remains the layout-only entry point used by benchmarks.
`HeadlessDiegeticUiPlugin` adds widget and IME behavior to that layout path.
`WidgetInputPlugin` is not required for widget behavior. Add it when
`hana_diegetic` should translate Bevy input bindings into `WidgetInput`
requests. A test can omit the input plugin and write those requests directly.

## Text transparency

Slug renders anti-aliased glyph edges from per-pixel coverage. It emits one
mesh per text run and orders coplanar text with `depth_bias`, so blended text
composites correctly as the camera moves with a default `Camera3d`.

Coplanar `Blend` text that lies on a shared plane (a paragraph on the ground,
labels on a wall) can still show a view-angle **color shift** at grazing angles,
because `Blend` sorts view-dependently. If that matters, opt into
[stable transparency (OIT)](#stable-transparency-oit) below.

The `AlphaMode` you pick determines how that coverage is composited:

| Mode | Coverage AA? | Notes |
|---|---|---|
| `Blend` (default) | ✅ | Classic alpha compositing |
| `Premultiplied` | ✅ | Like Blend; can look better on some scenes |
| `Add` | ✅ | Coverage modulates additive contribution |
| `Multiply` | ✅ | Coverage modulates multiplicative contribution |
| `AlphaToCoverage` + MSAA | ✅ | Coverage → sub-pixel sample mask |
| `AlphaToCoverage` without MSAA | ❌ | Degrades to `Mask(0.5)` |
| `Mask(t)` | ❌ | Thresholds coverage to 0/1 — jagged edges |
| `Opaque` | ❌ | Ignores coverage — glyph rectangles |

Set the app-wide default via `TextAlpha`, per-panel via
`DiegeticPanel::text_alpha_mode`, per-label via
`TextStyle::with_alpha_mode`, or per-standalone entity via
`override_text_alpha`.

### Quick recipes

```rust
// Default — Blend, works out of the box with no camera config.
commands.spawn(Camera3d::default());
```

```rust
// MSAA scene — switch the app-wide default to AlphaToCoverage:
commands.insert_resource(TextAlpha(AlphaMode::AlphaToCoverage));
commands.spawn((Camera3d::default(), Msaa::Sample4));
```

```rust
// Mix panel-label modes for creative effects:
let neon = TextStyle::new(Pt(24.0)).with_alpha_mode(AlphaMode::Add);
let tint = TextStyle::new(Pt(14.0)).with_alpha_mode(AlphaMode::Multiply);
```

See the `text_alpha` example for an interactive walkthrough.

### Stable transparency (OIT)

Add the `StableTransparency` marker to a `Camera3d` to route that camera through
**order-independent transparency**. OIT composites `Blend` fragments by depth
regardless of draw order, so the coplanar view-angle color shift goes away
without coarsening slug's coverage AA the way `AlphaToCoverage` would.

```rust
commands.spawn((Camera3d::default(), StableTransparency));
```

It is **opt-in**: with no `StableTransparency` present, there is no OIT and MSAA
stays on. Five things to know when you enable it:

1. **OIT and MSAA are mutually exclusive** — both on one camera panics Bevy's
   OIT plugin. `StableTransparency` forces `Msaa::Off` on the camera.
2. **Every camera sharing a window must match MSAA.** A sibling camera left at
   default MSAA stalls the macOS Metal swap chain (the window only repaints on
   OS events). The marker's observers force `Msaa::Off` on every
   screen-space overlay camera, in either spawn order.
3. **The shader `oit_draw` blocks do the work.** The camera setting alone is
   inert — slug and the SDF panel shaders route `Blend`/`Premultiplied`
   fragments through `oit_draw` under the `OIT_ENABLED` shaderdef.
4. **`depth_bias` does not reach OIT fragments**, so panels apply a manual
   per-command `OIT_DEPTH_STEP` to `position.z` for coplanar layer ordering
   inside the OIT buffer.
5. **Text must be `Blend` or `Premultiplied`.** `Opaque`/`Mask` render in the
   normal passes and bypass OIT entirely.

For mesh-edge AA *with* OIT, use a current-frame post-process AA such as FXAA or
SMAA. MSAA is incompatible with OIT, while TAA can make transparent panel
surfaces flicker or disappear because they do not write the depth and motion
data its temporal reprojection requires.

### Post-process AA is optional

The SDF panel shader's `fwidth`-based edges and slug's analytic per-pixel
coverage both produce clean edges without any post-process AA. Panels look
good with no AA at all.

SMAA smooths geometric and panel edges without relying on prior-frame history,
making it the stable default for scenes containing transparent diegetic panels.
When Bevy's default features are disabled, enable its `smaa_luts` feature as
well so SMAA receives its two-dimensional lookup textures.

```rust
commands.spawn((
    Camera3d::default(),
    bevy::anti_alias::smaa::Smaa::default(),
    // ...
));
```

## Tuning text cost

Text gets more expensive to draw the more of the screen it fills — big text, or
text you're looking at almost edge-on. If it's costing too much, `AntiAlias`
lets you turn off some of the smoothing work in exchange for speed.

There are two ways text can look rough, and the setting deals with them
separately:

- **Fuzzy edges** when you view text at a steep, near edge-on angle.
- **A faint spike** sticking out of the sharp corners of letters, only at the
  most extreme angles.

The four modes are which of those two you fix:

- `Off` — fix neither. Cheapest, but you can get both the fuzzy edges and the
  corner spikes.
- `Anisotropic` — fixes the fuzzy edges, costs almost nothing. Corner spikes can
  still show.
- `Supersample` — fixes the corner spikes, but costs more, and edges can still go
  fuzzy at steep angles.
- `Both` (default) — fixes both. Costs the most.

Keep `Both` unless text is eating into your frame budget. Then drop to whichever
single fix you actually need — usually `Anisotropic`, since fuzzy edges are the
common problem and it costs almost nothing. Use `Supersample` if it's the corner
spikes that bother you, or `Off` if your text never looks rough in the first
place.

```rust
commands.insert_resource(AntiAlias::Anisotropic);
```

To see what each one looks like, run the `aa_text` example — keys `1`–`4` switch
between them, and `A` and `B` fly the camera to angles that show off each
problem. To see what they cost, run `diegetic_text_stress`, press `A` to cycle
the modes, and watch the frame time go up and down.

## Bevy compatibility

| hana_diegetic | Bevy  |
|---------------|-------|
| main          | 0.18  |

## Examples

The examples in `examples/` use `fairy_dust`, an internal shared helper library
(not published) that wraps orbit-camera, window, and panel setup so each example
stays focused. For your own app, use the public API directly (`DiegeticUiPlugin`,
`DiegeticPanel`, `WorldText`, `StableTransparency`).

## License

MIT OR Apache-2.0
