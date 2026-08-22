# hana_liminal Design Improvements

Capabilities found in `bevy_mod_outline` that are not present in `hana_liminal`, evaluated for potential adoption.

## Rendering & Visual Quality

### Alpha Masking

UV-mapped texture-based alpha masking with channel selection (R, G, B, A) and threshold value. Allows outlines to follow texture opacity rather than mesh silhouette.

**Desirability: Medium** — unique visual capability, but niche use case.

### Flat/Billboard Extrusion Modes

Vertex extrusion flattened into a billboard plane, plus double-sided variants. Produces different visual results than `hana_liminal`'s world/screen hull approaches.

**Desirability: Low** — hana_liminal's three methods (JumpFlood, WorldHull, ScreenHull) cover the primary use cases well.

### Anti-Aliasing Infrastructure

Explicit support for FXAA, SMAA, TAA, and MSAA with a dedicated writeback pass built into the outline rendering pipeline.

**Desirability: Low** — Bevy's built-in AA works reasonably well for outlines already.

## Stencil & Depth

### Stencil-Based Outline Rendering

`OutlineStencil` component with fine-grained control (Always/IfVolume/Never) and offset control. Allows entities to occlude other outlines via stencil buffer. Independent volume/stencil rendering with separate controls.

**Desirability: Medium** — enables sophisticated layering and occlusion handling for complex scenes.

### Outline Plane Depth Control

`OutlinePlaneDepth` component with model-space origin and view-dependent offset. Solves z-fighting when outlines overlap, with parent-child sharing for hierarchical consistency.

**Desirability: Medium** — useful for complex overlapping outlines, but hana_liminal's depth prepass approach handles simple cases automatically.

## Animation & Performance

### Interpolation / Lerp Support

Built-in `Interpolation` trait for smooth color and offset transitions on `OutlineVolume` and `OutlineStencil`.

**Desirability: Low** — direct property mutation works, and users can implement lerp externally.

### Specialized GPU Batching

Custom instance buffers with GPU preprocessing support and fine-grained batching strategies beyond standard Bevy batching.

**Desirability: Low** — standard Bevy batching is sufficient unless profiling shows otherwise.

## Other

### Per-Entity Render Layer Control

`OutlineRenderLayers` component for layer-based visibility filtering independent of the entity's own render layers. Falls back to regular `RenderLayers` if not specified.

**Desirability: Medium** — useful for multi-camera setups (e.g., minimap, split-screen).

### Async Scene Inheritance

`AsyncSceneInheritOutline` component for automatic outline propagation into loaded scenes. Hooks-based async handling that waits for `SceneInstanceReady`.

**Desirability: Low** — hana_liminal's observer-based propagation is more idiomatic for modern Bevy and covers the same use cases.

### Feature Flags

Optional features (`flood`, `interpolation`, `reflect`, `scene`) for lighter builds when not all functionality is needed.

**Desirability: Low** — adds maintenance burden; worth considering only if the crate grows significantly.

### Warm-Up System

`OutlineWarmUp` pre-specializes shader pipelines to avoid frame stuttering when outline parameters change at runtime. Configurable warmups for render layers, stencil/volume states, transparency, and vertex offsets.

**Desirability: Deferred** — hana_liminal has fewer pipeline specialization axes than `bevy_mod_outline`, so this may not be needed. Revisit if frame stuttering is observed after implementing other improvements.

## Priority Summary

| Improvement | Desirability | Rationale |
|---|---|---|
| Stencil-based rendering | Medium | Sophisticated occlusion and layering |
| Outline plane depth control | Medium | Solves z-fighting in complex scenes |
| Alpha masking | Medium | Texture-aware outlines |
| Per-entity render layers | Medium | Multi-camera support |
| Flat/billboard extrusion | Low | Existing methods cover primary use cases |
| Anti-aliasing infrastructure | Low | Bevy's built-in AA is sufficient |
| Interpolation/lerp | Low | Can be done externally |
| Specialized GPU batching | Low | Standard batching is sufficient |
| Async scene inheritance | Low | Observer pattern is more idiomatic |
| Feature flags | Low | Maintenance cost outweighs benefit currently |
| Warm-up system | Deferred | May not be needed — hana_liminal has fewer specialization axes |
