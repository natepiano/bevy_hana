# Future Creative Effects: hana_liminal

Ideas for post-v1.0 exploration. None of these are committed — they're here so we don't lose them.

---

## Edge Detection (Geometric Feature Lines)

Detects and renders interior geometric edges: silhouette edges (front-face meets back-face), crease edges (normals diverge beyond threshold), and boundary edges (single-face edges). Complements outer-contour hull/JFA outlines with interior structural detail.

**Use cases**: NPR/toon shading, technical illustration, mechanical/architectural visualization.

**Rendering approach (CPU path)**:
1. Adjacency cache: one-time per mesh asset. Vertex welding (spatial hashing ~1e-4) → edge map → face adjacency.
2. Classification: per frame per entity. Camera-dependent dot product tests for silhouette/crease/boundary.
3. Line geometry: classified edges → thin billboard quads with per-edge width.
4. Render: dedicated shader pass with depth testing.

**Future GPU path**: compute shader classification + line geometry generation to eliminate CPU→GPU upload.

**Reference implementation**: `/Users/natemccoy/rust/bevy_silhouette/` has a working gizmo-based prototype.

---

## Animated Line Styles (Dashed, Dotted, Marching Ants)

Fragment shader modification in the hull pass. UV-based distance calculation creates gaps in the outline. Time uniform drives animation. Low implementation cost, high user value — marching ants useful for selection feedback in editors.

---

## Distance-Based Outline Scaling

Width that scales based on camera distance. Modify width during extraction using linear/logarithmic/custom falloff. Works with all backends.

---

## Depth/Normal Post-Process Outlines

Sobel or Roberts edge detection on the depth and normals buffers. Produces a stylized, uniform-width outline across the entire scene (or per-entity with stencil masking). Very different aesthetic from hull — thinner, more organic, catches internal silhouettes that hull misses.

**Inspiration**: Return of the Obra Dinn, Okami, Borderlands.

**Why it's different from hull**: Hull extrudes geometry outward. Post-process finds edges in the rendered image. They catch different features — hull only finds the outer silhouette, post-process finds any discontinuity in depth or normals.

---

## Sketch / Hand-Drawn Outlines

Jittered, wobbly lines that simulate hand-drawing. Could be implemented as a hull fragment shader effect that displaces the outline edge using noise (Perlin, simplex, or a pre-baked texture). Animation over time gives a "breathing" or "boiling" line effect common in hand-drawn animation.

**Inspiration**: Ni no Kuni, Sable, A Short Hike.

---

## Double / Triple Outlines

Multiple outline layers at different widths and colors on the same entity. Common in anime rendering — e.g., a thin dark inner outline with a thicker colored outer glow. Could be implemented as a `Vec<OutlineLayer>` or by allowing multiple `Outline` components via relations.

**Inspiration**: Guilty Gear Strive, Dragon Ball FighterZ.

---

## Gradient / Falloff Outlines

Outline color or opacity that fades from inner edge to outer edge rather than being solid. Creates a softer, glow-like appearance without needing HDR bloom. Could be a fragment shader effect using distance from the mesh surface.

---

## Occlusion-Only Outlines (X-Ray)

Render the outline only where the mesh is occluded by other geometry — the "see through walls" effect. Useful for showing selected objects behind cover, teammate positions in games, or hidden parts in technical visualization.

**Implementation idea**: Reverse the depth test in the hull pass — only draw hull fragments that fail the depth test against the scene.

---

## Animated Pulse / Breathe

Outline width or intensity that oscillates over time. Already partially possible with systems that mutate `Outline` each frame, but a built-in `AnimationCurve` or `Animatable` impl would let Bevy's animation system drive it natively via keyframes.

---

## Per-Edge Width Variation

Outline width that varies along the silhouette based on surface curvature, view angle, or artistic weight maps. Thicker lines on convex silhouettes, thinner on concave. Produces a more natural, illustrative look.

**Inspiration**: Traditional ink illustration, where line weight conveys form.

---

## Inner Glow / Outer Glow

Separate from the outline itself — a soft colored region that extends inward from the mesh edge (inner glow) or outward beyond the outline (outer glow). Different from bloom because it's controlled per-entity and doesn't depend on HDR intensity.
