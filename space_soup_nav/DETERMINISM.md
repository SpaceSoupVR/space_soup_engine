# Determinism rules

Every headset must agree on the navigation data and, where it matters, on what is computed from
it. The research behind this crate (`reports/Rust navmesh design research.md`) found:

- If the host runs all monster AI and replicates waypoints (the documented practice for
  multiplayer games), **paths do not have to be bit-identical across headsets**. Only the
  *data* does.
- So the hard guarantee is on the data: a baked piece and an assembled level must be the same
  bytes everywhere. Anything computed at runtime should still be deterministic on one machine,
  run to run, so that bugs reproduce.

## Rules

1. **Baked geometry is integers.** Vertices are millimetres as `i32` in the piece's own space.
   Quarter turns are integer swaps and negations (`Turn::apply`), never `sin`/`cos`, so a
   doorway vertex lands on exactly the same integer in every rotation.
2. **No unordered iteration.** No `HashMap`/`HashSet` in anything that produces output or makes
   a decision. Use `BTreeMap`, `BTreeSet` or sorted `Vec`s. Enforced by `clippy.toml`.
3. **Transcendentals go through `libm`.** `f32::sin` and friends call the platform's maths
   library (Apple's on the Mac, Bionic on the Quest), and Rust's docs say their results vary
   by platform. Enforced by `clippy.toml`.
4. **No fused multiply-add by accident.** Plain Rust `a * b + c` is never fused (verified in the
   research). glam's NEON backend fuses in `mul_add` and `lerp`, while SSE2 does not, so those
   are banned in favour of writing the arithmetic out. Enforced by `clippy.toml`.
5. **Explicit tie-breaks.** Every sort and every "pick the best" breaks ties on a stable key
   (an index), never on input order alone.
6. **Content hash on every asset.** `NavPieceAsset::content_hash` is FNV-1a 64 over the
   canonical bytes. Assembled levels are hashed too (`net::LevelDigest`): headsets compare the
   hash on joining and refuse a level that differs, naming the room or join that does.

Check with:

```bash
cargo clippy -- -D clippy::disallowed_types -D clippy::disallowed_methods
```
