# Space Soup — Status for `synful_zoom_plus`

**Date:** 2026-08-02
**Branch:** `synful_zoom_plus` (branched from the Wave 1 work)
**Covers:** the zoom/scope port onto Wave 1, plus B2, C2 and part of C3.

---

## TL;DR for a reviewer

| Item | State | Verified how |
|---|---|---|
| Zoom/scope port onto Wave 1 | ✅ **Fixed — it did not build when handed over** | `cargo test` green |
| **B2** RigidBodyCard | ✅ Done | 16 new editor tests |
| **C2** Scope quality (no MSAA) | ⚠️ **Done but UNVERIFIED IN HEADSET** | 24 GPU/unit tests |
| **C3** Multiview | 🔶 **Steps 1–2 done, step 3 not started** | 5 validation tests |
| Headset verification of any of the above | ❌ **Not done** | — |

**The single most important line in this document:** C2 and C3 have never run on a
Quest. Every claim below is from headless tests on a desktop GPU. Do not treat
either as working on device until someone looks through the scope.

---

## 1. The port did not build

The zoom/scope port onto Wave 1 arrived with a compile error and several silent gaps.
All are fixed on this branch, but they are worth knowing because two of them were
caught by Wave 1's own instruments — and one was not.

| # | Gap | Caught by | Fix |
|---|---|---|---|
| 1 | `error[E0027]: pattern does not mention field 'optic'` — build broken | **A2's `schema_exhaustiveness`, at compile time** | Added `optic` to the destructure |
| 2 | `schema.json` stale (21 components, no `optic`) | A2's staleness test | Regenerated → 22 |
| 3 | Editor's `generated/schema.json` stale (19 components, `light` not `lights`) | **Nothing** — see §1.1 | Synced + new guard test |
| 4 | A4 registry missing `optic`, `spawn_point`, `teleportal`; stale `light` | A4 gate, once §3 was synced | Registered all four |
| 5 | `lobby.json` had **no optic and no grip points** on the m4a1 | Nothing | Ported from `frank_branch` |
| 6 | Optic↔lobby parity test lost in the port | Nothing | Restored |
| 7 | `run_quest.ps1` missing entirely | Nothing | Restored (incl. 88 uncommitted lines) |

**Credit where due:** A2's exhaustive destructure is a genuinely good instrument. It is
a *compile-time* guard, not the `assert_eq!(len(), 21)` count test next to it, and it is
what caught the broken build. The count test would have passed.

### 1.1 A real gap in A4, not caused by the port

Nothing regenerates the editor's copy of `schema.json`. It had drifted by four
components **before** this work started, and because A4's coverage gate validates the
registry against that copy, **the gate was passing while the editor genuinely lacked
authoring for shipped components**. A parity instrument reading a stale snapshot
reports success by construction.

Added `frontend/src/lib/schemaSync.test.js`, which fails the build when the copy
diverges from the engine's. Worth extending A4's CI job to run it.

---

## 2. B2 — RigidBodyCard ✅ DONE

`components/RigidBodyCard.jsx` + `lib/rigidBody.js`, following the `SliderJointCard`
house pattern: presets first, a plain-language consequence line, raw fields behind them.

Defaults mirror `RigidBodyDef::default()` and are pinned by test.

**The warning worth knowing about:** *"Dynamic but no grip points — a player cannot pick
this up."* That is exactly the failure that blocked the m4a1 scope test — a weapon that
rendered perfectly and could not be held, with nothing in the editor saying why.

**Verified:** editor suite 39 passed (was 22), lint clean, `vite build` clean.
**Not verified:** nobody has authored a body through the UI and run it.

---

## 3. C2 — Scope image quality ⚠️ DONE, UNVERIFIED ON DEVICE

**MSAA was dropped**, for two independent reasons. The second matters more.

1. It does not compose with SSR — the eye pass raymarches against `scene_targets[eye]`
   and needs readable per-pixel depth; multisampled depth has no meaningful resolve.
2. **It was aimed at the wrong defect.** "Magnified content aliases badly" is true of
   the *screen-space overlay* method this project rejected. With the true-portal method
   actually built, `fov = apparent / magnification` into a fixed-resolution target means
   angular sampling density *rises* with magnification — the scope image is sharper per
   world-degree than the main view.

The real defect was at the **composite**: the target was created with `mip_level_count: 1`
and sampled with `min_filter: Linear` and no mip chain, then minified 3–5× onto a
150–250 px ocular disc. That undersamples badly and crawls under head motion. MSAA would
not have fixed it — MSAA acts *inside* the target, not on how the target is sampled down.

**What was done:**
- Full mip chain on `ScopeTarget` + `mipmap_filter: Linear`; an explicit `MipBlit` pass
  fills levels 1..n (wgpu has no `generate_mipmaps`)
- Target sized per frame from the projected ocular disc instead of a fixed 768
- `supersample` plumbed but passed `1.0`; `SCOPE_TARGET_MAX` is still a constant rather
  than reading `OpticDef::quality` — the authored tier is not wired to the renderer yet

**Two things that would have silently defeated this**, noted so they are not undone:
- The composite bind group must use the **full-chain view**, not the mip-0 view.
  Binding one level makes `mipmap_filter` a no-op — implemented and doing nothing.
- Mip generation must run **inside the same encoder**, after the world pass is dropped
  and before submit, or the chain is a frame stale.

**Verified:** 24 scope tests. The load-bearing one is
`the_mip_chain_averages_away_the_minification_aliasing`: paints a 1-texel checkerboard
(worst case), builds the chain, reads back levels 0 and 1, asserts variance collapses
>50× and the mean lands at mid grey.

**NOT verified:** no frame-time measurement against the C1 baseline, and **no visual
check**. Shimmer is a *temporal* artifact — it exists only under head motion, so no
screenshot can confirm it. This needs someone wearing the headset.

**Suggested check when someone does:** add a temporary controller-button A/B that forces
LOD 0 while held. "Hold the button, does the crawl change?" is a far more reliable
observation than "does this look better than last week?"

---

## 4. C3 — Multiview 🔶 STEPS 1–2 DONE, STEP 3 NOT STARTED

### The finding that shaped it

Multiview is **not a flag flip**. `solid_pipeline`, `mesh_pipeline` and
`skinned_mesh_pipeline` are each used by *two* passes: the eye pass (wants 2 views) and
the scope world pass, which renders into a **single-layer** target where a multiview
pipeline is a validation error. The mirror reflection pass has the same shape.

So pipelines are **parameterized by view count**, not converted.

### Done

- **Vulkan capability enabled.** The `VkDevice` was created with no `pNext` chain, so
  `PhysicalDeviceMultiviewFeatures` was never on. Vulkan 1.1 is already requested, so
  multiview is core — no extension needed. Also declared in both halves of the wgpu hal
  adoption (`device_from_raw` and `create_device_from_hal`); one without the other fails
  at device creation.
- **Camera uniform is now `array<mat4x4<f32>, 2>`.** Single-view shaders read slot 0,
  multiview indexes by `@builtin(view_index)`. One shape means one bind group layout
  serves both — which is why the codebase is not half-broken mid-migration.
- **All five eye-pass pipelines have multiview variants**: solid, wire, mesh, skinned,
  particle.
- **Attachments ready (step 1).** Depth is a 2-layer texture with per-eye slices *and* a
  `D2Array` stereo view; the swapchain image has a matching `D2Array` colour view.

> Incidental fix: the eye pass previously shared **one** depth buffer between both eyes.
> Each eye now writes its own layer.

### Not done — step 3

The eye loop is **not** collapsed. The stereo attachments are deliberately
`#[allow(dead_code)]`, so collapsing it is a change to the pass body alone.

**Why it stopped here.** The eye pass body also drives the **mirror**, **SSR** and the
**scope composite**, all inherently per-eye today (per-eye reflection matrices;
`scene_targets[eye]`; per-eye occupancy and offset direction). Restructuring those is
where mistakes produce *wrong images* rather than failed validation — the one class of
bug no headless test catches, and device testing is currently deferred.

Everything landed so far is provable on a desktop GPU. Step 3 is not. It should be done
in the same session as the headset check.

---

## 5. Test status

| Repo | Result |
|---|---|
| `space_soup_engine` | **61 passed, 0 failed** |
| `space_soup` | **43 passed, 2 failed** — both pre-existing, see below |
| `scene_editor_web` | **39 passed, 0 failed**; lint + build clean |

The two renderer failures are **not** from this work:
- `real_skinned_fixtures_still_load_correctly` — `game/models/left_hand.glb` was deleted
  by commit `16f45cf`. This is task **E4**.
- `m4a1_orphan_node_promotion_end_to_end` — shells out to `python3`, which does not exist
  on Windows (it is `python`). Environmental; worth making the test tolerate both.

---

## 6. What to do next, in order

1. **Confirm grip/grab works** — everything scope-related is blocked behind being able to
   hold the weapon.
2. **Headset session**, doing three things at once: verify C2 visually, take a frame-time
   reading against the C1 baseline, and finish C3 step 3 with eyes on the result.
3. **E4** — restore or re-point `left_hand.glb` and get the renderer suite fully green.
4. Wire `OpticDef::quality` through to `SCOPE_TARGET_MAX`, and consider enabling
   `supersample = 2.0` once frame time is known.
5. Add `schemaSync.test.js` to the A4 CI job so the editor's schema copy cannot rot again.
