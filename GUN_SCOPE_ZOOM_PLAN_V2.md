# Gun Scope / Zoom — Implementation Plan v2

Date: 2026-07-30
Status: Spec for review (supersedes `GUN_SCOPE_ZOOM_AND_EDITOR_IMPLEMENTATION_PLAN.md`)
Branches: `synful_zoom` on space_soup_engine, space_soup, quest_app, scene_editor_web, game
Audience: Runtime/Renderer, Editor, QA

> v1 is kept alongside this file for diffing. This document is deliberately **shorter** than v1:
> one of v1's main problems was authoring surface, and a tight spec is part of the fix.

---

## 1. What changed from v1, and why

| v1 said | v2 says | Reason |
|---|---|---|
| VR default = screen-space overlay magnification; true scope pass is an optional Phase 5 tier | **VR default = true scope render pass**; overlay is a low-tier fallback only | Overlay loses angular detail by exactly the magnification factor, and samples from the eye's viewpoint rather than the scope's. v1 §20.1 already demanded objective-lens projection, which overlay cannot do — v1 contradicted itself. |
| Scope rendered per eye | **Rendered once per frame, shared by both eyes** | A parallax-free scope's image is collimated: it does not shift with eye position within the eye box. So the image is view-independent — one render serves both eyes. Halves the cost v1 assumed. |
| `eye_box_inner_radius_m`, `eye_box_outer_radius_m` as authored sliders (0.032 / 0.050) | **Derived** from `objective_diameter / magnification` (= exit pupil) and per-optic eye relief | Those are real optical quantities, not taste. Deriving them removes two invented constants and makes any new optic correct by construction. |
| Magnification ramps 1.7x → full as alignment improves; image blurs when misaligned | **Magnification is constant. Misalignment produces eye-box vignette (crescent/shadow), not blur or zoom change** | Real glass does not change power or focus with eye alignment; it vignettes. The ramp is a mode switch in disguise, which v1 itself forbids ("zero discrete visual state jumps"). |
| `reticle_damping`, `reacquire_boost_ms` | **Cut** | These are aim assist. They contradict v1's own requirement of ≤0.35° divergence between shot basis and rendered reticle. Stability comes from the physical two-hand/stock model and the late-latched reticle instead. |
| Acquisition ring HUD cue | **Cut** | Contradicts v1 §18's "subtle optical guidance instead of HUD-heavy onboarding". |
| Server-authoritative lag compensation inside the zoom milestone | **Split into a separate project**; zoom only publishes the aim basis | Historical hitbox snapshots + rewind + telemetry is a large independent workstream. Bundling endangers both. |
| Tuning tables with 3-significant-figure defaults | **Calibration procedure + derived values** | Same failure mode we just removed from the rigging system (`wrist_roll_deg: 135`, hand-dialled and >30° wrong). Numbers we cannot justify do not belong in a spec. |

**Kept from v1, unchanged and endorsed:** the two-channel model (weapon optics vs headset abilities); lens-portal-first / no-mode-switch as the product goal; per-eye lens *visibility* evaluation; optional `#[serde(default)]` component for backward compatibility; `FireSolutionState`; late-latched reticle; soft eye-relief assist; optic-family taxonomy (v1 §20.3 — the strongest section in the original).

---

## 2. Verified codebase findings

All checked against live source on 2026-07-30 (not the graph, which predates the 07-29 commits).

- **OpenXR FOV is pass-through and must stay that way.** `Camera::xr_projection(ev.fov, 0.01, 1000.0)` at `space_soup/src/renderer/xr_renderer.rs:462`; the composition layer submits `.fov(ev.fov)` unchanged at :723. Submitting a narrower FOV than was rendered would make the compositor reproject incorrectly. **No app-level FOV override in VR.** (v1 got this right.)
- **The second-viewpoint machinery already exists.** `mirror.rs` provides `reflection_matrix`, `world_plane_equation`, `plane_to_eye_space`, `oblique_near_clip`, `MirrorTarget` (`create_target(device, format, w, h)` → colour + depth views) and a separate reflected view-proj uniform. `xr_renderer.rs` holds `mirror_targets: [MirrorTarget; 2]`, renders them in their own pass (:483) and samples them in the composite pass (:700). **A scope is the same shape of object as a mirror.**
- **Per-eye offscreen scene targets already exist**: `scene_targets: [SceneTarget; 2]` (colour + depth) for SSR, created via `ssr_pipelines.create_scene_target(...)` at :172.
- **No multiview, no MSAA, no foveated rendering** anywhere: `multiview: None` and `sample_count: 1` throughout (`mesh_pipeline.rs:111,268`, `mirror.rs:149,217,233`, `mod.rs:503`). Good news for adding a pass; but it means **magnified content will alias**, so scope-target resolution and filtering matter.
- **`GameObject` optional-component pattern is established**: `light: Option<LightDef>`, `sound`, `particle_emitter`, `laser`, `terrain_collider`, all `#[serde(default)]`, plus a `grip_pose_legacy` migration precedent (`space_soup_engine/src/scene.rs:640+`).
- **Input vocabulary is fixed and small**: `InputFrame` is in `space_soup_engine/src/events.rs:28` (v1 mislocated this in `quest_app/src/lib.rs`), consumed in `runtime.rs:554`; bindable buttons are `BINDING_BUTTONS = ["btn_a","btn_b","btn_x","btn_y","trigger","grip"]` (`scene.rs:171`).
- **Editor card pattern is trivial to extend**: `<div className="card"><div className="section-label">Light</div>…` in `Inspector.jsx:89`.
- **Assets to author against**: `game/models/m4a1.glb` and `game/models/ar15`.
- **No zoom, scope, magnification or FOV-authoring code exists** in the engine or renderer today. Editor camera FOV is hardcoded (`viewport.js:125`).

---

## 3. The physical model this design is built on

Three real optical facts drive everything below.

**3.1 Exit pupil.** `exit_pupil_mm = objective_diameter_mm / magnification`. A 40 mm objective at 4× gives a 10 mm exit pupil; at 16× it gives 2.5 mm. The exit pupil *is* the eye-box diameter. This is why high magnification is physically harder to get behind — and it means **eye-box tightness at high power is free**, not something we tune.

**3.2 Eye box and eye relief.** The eye box is a cone behind the ocular: its diameter is set by the exit pupil, its length by eye relief. Rifle scopes need ≥5 cm eye relief to clear recoil; 9–12 cm is common on high-power optics. Outside the eye box you get **scope shadow** — a black crescent eating into the circle, growing until the image blacks out.

**3.3 Collimation → the image is view-independent.** In a parallax-free optic the target image and reticle sit in the same focal plane, and the field is collimated (focused at infinity). The practical test riflemen use: *move your head behind the scope without moving the rifle — the reticle does not move relative to the target.*

**That third fact is the key architectural lever.** Because the image does not depend on eye position, we render it **once**, not once per eye. Eye position affects only *vignetting*, which is a cheap per-eye mask term.

Consequences for authoring: an optic is described by physical numbers a real spec sheet has — objective diameter, magnification range, true field of view, eye relief — and everything the renderer needs falls out of them.

---

## 4. Why not screen-space overlay (the quantitative argument)

Quest 3 is 2064×2208 per eye across ~104° horizontal → **~20 px per display degree** (Meta cites 25 PPD centrally). Consider an optic presenting a ~20° apparent field at magnification **M = 8**, so it shows 2.5° of world:

| Approach | Real world detail inside the lens |
|---|---|
| Screen-space overlay | 2.5° × 20 px/° ≈ **50 px** of source data, upscaled ~8× to fill the lens circle |
| True scope pass into a 400 px target | **160 px per world degree** — the full 8× gain the optic promises |

**The overlay discards a factor of exactly M.** At 8× you present one-eighth of the detail the display can physically show: magnified blur, not magnification. Two further defects: the image is sampled from the eye's viewpoint, so parallax and occlusion around the barrel are wrong; and objective-lens projection (v1 §20.1) is impossible.

This matches shipped practice — Pavlov renders the scene a second time inside the scope, gating the second camera on eye proximity for cost.

Overlay is retained only as a **fallback tier** for when the lens covers a trivial pixel area or the device is thermally throttled, where the detail loss is not perceptible anyway.

---

## 5. Architecture

### 5.1 Render model

Per frame, in this order:

1. **Scope pass (once, not per eye)** — for each *active* optic:
   - Virtual camera at the **objective lens** centre, oriented along the **scope axis** (from the weapon transform, not the head).
   - Projection: symmetric perspective with `fov_y = true_field_of_view` (= apparent field / M).
   - Render into a `ScopeTarget` (colour + depth), sized per quality tier.
   - Reuse the `mirror.rs` target/pipeline shape; this is a narrow-FOV forward render, not a reflection.
2. **Per-eye composite** — in each eye's existing composite pass, for each visible optic:
   - Determine the ocular lens disc's projected region for **that eye**.
   - Sample the shared scope texture by **angle within the apparent field** (collimated content ⇒ sample by ray angle, not eye position).
   - Apply the **eye-box vignette** computed from that eye's position relative to the exit-pupil cone: full circle inside, crescent shadow as it exits, black beyond.
   - Composite the reticle (see 5.3).
3. **Headset Ability Channel** (unchanged from v1) — player-centric post effects, after the lens channel, must not alter lens mask boundaries.

**Gating (performance-critical):** run the scope pass only when the ocular is visible to at least one eye *and* covers more than a threshold pixel area. Otherwise skip entirely — a holstered or low-ready weapon costs nothing.

### 5.2 Known interactions to decide explicitly

- **SSR.** Reflections are screen-space, computed from `scene_targets[eye]`. The scope pass is a different view and will have **no SSR** unless we run it again there. **v1 decision: no SSR inside the scope**, documented and accepted; revisit if it reads as wrong on reflective surfaces.
- **Aliasing.** No MSAA anywhere (`sample_count: 1`). Magnification makes edge aliasing more visible. Mitigate with scope-target resolution + linear filtering first; only consider AA if measurement says it's needed.
- **Lightmaps / DDGI (ADR-007).** A local scope pass is compatible with server-streamed view-independent lighting, since probes are view-independent by construction. But it does add local geometry cost in exactly the frames that are already busy — hence the gating above.
- **Polyrepo enum breakage.** Cortex anti-pattern, learned on `LightKind::Directional`: adding enum variants breaks non-exhaustive matches in downstream crates with E0004 that only surface when *that* crate builds. Every new enum here must be swept across quest_app, editor bridge, server convert, protocol.

### 5.3 Reticle

- Composited in the scope pass output, at the focal plane (so it is parallax-free with the target image by construction).
- **FFP vs SFP** is an authored per-optic choice: first focal plane scales with magnification (subtensions stay valid); second focal plane keeps constant apparent size.
- Drawn with the **freshest available head pose** (late latch) so it does not swim during fast turns. This is the one v1 stability feature that is honest — it improves temporal accuracy rather than filtering the player's aim.

### 5.4 Data model (minimal)

Add one optional field to `GameObject`, matching the existing pattern:

```rust
#[serde(default)]
pub optic: Option<OpticDef>,
```

`OpticDef` carries **physical description + presentation**, and nothing that can be derived:

- `class: OpticClass` — `ReflexRedDot | Holographic | Lpvo | FixedPrism | PrecisionScope | Binocular`
- `magnification: MagnificationDef` — `Fixed(f32)` or `Variable { min, max, steps: Option<Vec<f32>> }`
- `objective_diameter_mm: f32`
- `true_fov_deg_at_1x: f32`
- `eye_relief_mm: f32`
- `lens: LensGeometryDef` — ocular mesh/node reference + clip shape + edge feather
- `reticle: ReticleDef` — texture/style, focal plane, colour, brightness
- `zero: Option<ZeroDef>` — zero distance, height-over-bore
- `quality: OpticQualityTier` — `Low | Balanced | Ultra` (resolution + gating thresholds)

**Derived at load, never authored:** exit pupil, eye-box diameter, eye-box cone length, apparent field, scope-camera FOV, per-magnification eye-box tightening.

That is ~9 authored fields versus v1's ~15 nested structs and 60+ tunables.

### 5.5 Runtime ownership

A single `OpticController` per local player in `space_soup_engine`:

- Resolves the active optic from held-object context.
- Computes, per eye per frame: ocular visibility, eye-box occupancy (→ vignette term), projected pixel area (→ gating).
- Publishes a compact `OpticViewState` for the renderer and a `FireSolutionState` snapshotting the aim basis at trigger time.
- Owns variable-magnification state; exposes it to script.

No gameplay or script code mutates camera matrices directly.

Non-VR path: keep it simple — apply the effective FOV to `camera.fov_y` with a smooth blend, plus lens mask and reticle overlay. Desktop genuinely can override FOV; VR cannot.

### 5.6 Script API (small)

`set_optic_magnification(level)`, `cycle_optic_magnification(step)`, `set_optic_zero(distance_m)`, `optic_state()` read-back. Ability-channel commands stay as v1 specified.

---

## 6. Editor authoring: derive, don't slider

Principle carried over from the rigging fix: **measure from the asset instead of exposing a knob.**

**Optic card** (`Inspector.jsx`, following the existing Light card):
1. Enable toggle.
2. `class` picker — sets sane physical defaults for that family (a red dot gets huge eye relief and 1×; a precision scope gets a small exit pupil).
3. Magnification (fixed or range/steps).
4. Physical spec: objective diameter, true FOV, eye relief — the four numbers off a real spec sheet.
5. Lens geometry: pick the ocular mesh/node; clip shape + feather. **Auto-detect** the ocular node from the mesh where possible, the way bone roles are auto-detected.
6. Reticle: style, focal plane, brightness.
7. Zero: distance + height-over-bore.
8. Quality tier.
9. **"Derived" read-only panel** showing exit pupil, eye-box diameter and length, apparent field, scope-camera FOV — so the author can see the consequences of their physical numbers immediately.

**Preview mode:** show the actual lens image in the editor viewport with a movable virtual eye, so the author can see the eye box and crescent behaviour without a headset. This replaces v1's "Quick-Scope Trainer" and "Player Feel metrics" panels, which measured tuning values we are no longer exposing.

---

## 7. Performance

Budget targets from v1 are retained (72 Hz → 13.89 ms; 90 Hz → 11.11 ms) but the cost model changes:

- Scope pass is **one** narrow-FOV render, not two, and is **skipped entirely** when no ocular is visible or the lens is sub-threshold in pixels.
- Narrow FOV means aggressive frustum culling — a 3° cone contains very little geometry, so this is far cheaper than a full second scene render.
- Resolution by tier; dynamic step-down under sustained GPU pressure, max one tier per second (v1's anti-pumping rule is good, keep it).
- **Measure before defending any number.** The v1 figures (CPU ≤0.35 ms, GPU ≤0.90 ms median) are plausible targets but were not measured on this codebase; treat them as goals to verify, not commitments.

---

## 8. Verification

Cortex anti-patterns that apply directly here:

- **WGSL is validated at pipeline creation, not `cargo build`.** Every new scope shader must go through the existing `gpu_smoke` path.
- **Pipeline build is not proof of correct rendering.** The cuboid-winding bug passed compile and pipeline creation while every cuboid rendered inside-out. The lens quad needs a **render-and-readback test**: render a known scene through a known optic to an offscreen target, map it, and assert on pixels.

Measurable acceptance criteria (all machine-checkable):

| Property | Test |
|---|---|
| Magnification is real, not upscaled | Render a known test pattern at 1× and M×; assert resolvable detail scales with M |
| Image is view-independent | Move the virtual eye within the eye box; assert the target/reticle relationship is stable |
| Eye box matches physics | Assert vignette onset radius equals `objective/M` within tolerance |
| Reticle trust | Divergence between `FireSolutionState` basis and rendered reticle centre ≤0.35° p95 |
| No mode switching | Sweep the optic across the view; assert no discontinuity in the output beyond the gating threshold |
| Cost | Scope pass GPU/CPU time within tier budget; zero cost when ocular not visible |

---

## 9. Phasing

- **Phase 0** — `OpticDef` schema + serde round-trip tests; derived-quantity unit tests (exit pupil, eye box). No behaviour change when `optic` is absent. Sweep downstream crates for E0004.
- **Phase 1** — Editor Optic card + derived read-only panel + persistence. Author a scope on `m4a1.glb`.
- **Phase 2** — `OpticController`: visibility, eye-box occupancy, gating, `FireSolutionState`. Unit-tested headless.
- **Phase 3** — Desktop path (real FOV blend + mask + reticle). Fast validation loop with no headset.
- **Phase 4** — VR scope pass: `ScopeTarget`, objective-lens camera, shared-texture per-eye composite, eye-box vignette, late-latched reticle. Render-and-readback tests + on-device measurement.
- **Phase 5** — Variable magnification, FFP/SFP, zeroing, optic families breadth.
- **Deferred, separate projects** — server-authoritative lag compensation; headset ability channel; zoom-state replication for spectators.

## 10. Open questions

1. **Both-eyes-open behaviour at the eye.** A rifle scope's exit pupil is small, so physically only one eye can be behind it at a time; the other eye is blocked by the scope body. Our per-eye visibility test should produce that naturally — but it needs on-device confirmation that the transition to one-eye dominance feels right rather than flickery.
2. **Binoculars/spotting scopes** have two optical paths and genuinely serve both eyes. Do we need them in v1, or is `Binocular` class deferred?
3. **Variable-magnification input** in VR when trigger and grip are already taken — a physical zoom ring on the weapon model is the immersive answer but needs a hand-interaction design.
4. Whether ranked/competitive modes may use a wider-than-physical eye box as an accessibility option.
