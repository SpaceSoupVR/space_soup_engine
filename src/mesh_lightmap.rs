//! Where a MESH's baked lighting lives in its atlas.
//!
//! The same contract as [`crate::brush_lightmap`], for the other half of the
//! world. The baker has to know which point in space every texel stands for so
//! it can shoot rays from there; the renderer has to know which texel every
//! vertex samples. Two pieces of code deriving that mapping separately is how
//! baked lighting ends up slightly displaced -- a symptom that reads as "the
//! baker is buggy" rather than as a layout mismatch -- so it is derived once,
//! here, and both sides are handed the result.
//!
//! WHY A CHART PER TRIANGLE
//!
//! A proper unwrapper (xatlas and its relatives) grows charts across a mesh
//! until the stretch gets too large, then cuts a seam. That packs far tighter
//! than this does, and it is a large dependency, a nondeterministic one, and a
//! source of seams that then have to be reconciled. A chart per triangle needs
//! none of that: every triangle is planar by definition, so there is no stretch
//! to bound and no seam to cut, and the layout is a pure function of the
//! geometry -- reorder nothing, and re-running the bake reproduces it exactly.
//!
//! What it costs is atlas space. A triangle fills half its bounding rectangle,
//! and every chart carries its own gutter, so a mesh uses roughly three times
//! the texels a seamed unwrap would. For props -- which is what meshes are here;
//! levels are brushes -- that trade is worth taking, and [`MAX_TRIANGLES`] is
//! what stops it being taken on something that is not a prop.
//!
//! UV2 IS PER CORNER, SO THE MESH MUST BE SPLIT
//!
//! Adjacent triangles land in unrelated parts of the atlas, so a vertex shared
//! between them has no single uv2. The renderer therefore de-indexes: one
//! vertex per triangle corner. That is what every unwrapper does at a seam;
//! here every edge is a seam.
//!
//! MESH-LOCAL UNITS
//!
//! Density is metres per texel in the mesh's OWN space, before the object's
//! scale. It has to be: the geometry is loaded once per asset and the uv2 lives
//! in the vertex buffer, so a layout that depended on an object's scale would
//! need a second copy of the mesh per instance. An object that scales its mesh
//! up gets a proportionally coarser lightmap, which is a quality knob rather
//! than a correctness one -- the mapping still lands where it should.

/// Texels of empty space kept around every chart, to stop bilinear bleed.
pub const GUTTER: u32 = 1;

/// The largest atlas one mesh may claim, per side.
pub const MAX_ATLAS: u32 = 1024;

/// Smallest chart, so a sliver triangle still gets somewhere to put its light.
const MIN_CHART: u32 = 2;

/// Metres per texel, in mesh-local space.
pub const DEFAULT_TEXEL: f32 = 0.04;

/// Above this many triangles a mesh gets no per-texel lightmap at all.
///
/// Not a performance guess: with a chart per triangle the atlas area grows
/// linearly with the triangle count, so a 500k-triangle skydome would ask for
/// an atlas it cannot have and get one scaled down until every chart is the
/// 2x2 minimum -- which is a per-triangle lightmap costing 512x512 texels and
/// three times the vertices, to say something a single flat value already says.
/// Past this point the flat tint is not a degraded result, it is the right one.
pub const MAX_TRIANGLES: usize = 40_000;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        mul(a, 1.0 / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// One triangle's patch of the atlas, and how it maps back to the mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshChart {
    /// Texel rectangle, gutter excluded.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Mesh-local position at the CENTRE of this chart's texel (0, 0).
    ///
    /// Centre rather than corner for the same reason as the brush atlas: a ray
    /// fired from a texel's corner sits exactly on a boundary and lands on
    /// either side depending on rounding.
    pub origin: [f32; 3],
    /// Mesh-local step between horizontally and vertically adjacent texels.
    pub du: [f32; 3],
    pub dv: [f32; 3],
    /// The triangle's geometric normal.
    pub normal: [f32; 3],
    /// The triangle's three corners, in this chart's texel coordinates.
    ///
    /// Carried rather than recomputed because both consumers need them and for
    /// opposite purposes: the renderer turns them into uv2, and the baker uses
    /// them to tell which texels are actually on the triangle and to blend the
    /// vertex normals across it.
    pub corners: [[f32; 2]; 3],
}

impl MeshChart {
    /// The mesh-local point a texel of this chart samples.
    pub fn texel_local(&self, tx: f32, ty: f32) -> [f32; 3] {
        add(self.origin, add(mul(self.du, tx), mul(self.dv, ty)))
    }

    /// Where corner `i` lands in the atlas, in 0..1.
    pub fn uv2(&self, corner: usize, atlas_w: u32, atlas_h: u32) -> [f32; 2] {
        let c = self.corners[corner];
        // +0.5 puts texel (0,0)'s centre at its centre, matching `origin`.
        [
            (self.x as f32 + c[0] + 0.5) / atlas_w as f32,
            (self.y as f32 + c[1] + 0.5) / atlas_h as f32,
        ]
    }

    /// Barycentric weights of a texel centre within the triangle.
    ///
    /// Negative in a component means the texel is off that edge. The baker
    /// needs both facts -- whether to shade the texel at all, and how to blend
    /// the three vertex normals if it does -- and they are the same
    /// computation, so it is done once and returned whole.
    pub fn barycentric(&self, tx: u32, ty: u32) -> [f32; 3] {
        let p = [tx as f32, ty as f32];
        let [a, b, c] = self.corners;
        let v0 = [b[0] - a[0], b[1] - a[1]];
        let v1 = [c[0] - a[0], c[1] - a[1]];
        let v2 = [p[0] - a[0], p[1] - a[1]];
        let den = v0[0] * v1[1] - v1[0] * v0[1];
        if den.abs() < 1e-12 {
            // A degenerate triangle covers no area; call every texel the first
            // corner rather than dividing by zero.
            return [1.0, 0.0, 0.0];
        }
        let v = (v2[0] * v1[1] - v1[0] * v2[1]) / den;
        let w = (v0[0] * v2[1] - v2[0] * v0[1]) / den;
        [1.0 - v - w, v, w]
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshLightmapLayout {
    pub width: u32,
    pub height: u32,
    /// One per triangle, in packing order. Address them through [`Self::part`]
    /// rather than by a bare index: which triangles belong to which primitive
    /// is what `parts` records.
    pub charts: Vec<MeshChart>,
    /// Where each primitive's triangles sit in `charts`.
    pub parts: Vec<PartRange>,
    /// Multiplier applied to the requested density to fit `MAX_ATLAS`.
    /// 1.0 when nothing had to be given up.
    pub density_scale: f32,
}

/// One primitive's slice of the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange {
    /// glTF node index -- see [`mesh_lightmap_layout`] on why this and not
    /// traversal order.
    pub node: usize,
    /// Index of the primitive within that node's mesh.
    pub primitive: usize,
    /// Index into `charts` of this primitive's first triangle.
    pub first: usize,
    pub count: usize,
}

impl MeshLightmapLayout {
    /// The chart for triangle `tri` of one primitive, or `None` if that
    /// primitive was not laid out.
    pub fn chart(&self, node: usize, primitive: usize, tri: usize) -> Option<&MeshChart> {
        let r = self
            .parts
            .iter()
            .find(|p| p.node == node && p.primitive == primitive)?;
        if tri >= r.count {
            return None;
        }
        self.charts.get(r.first + tri)
    }
}

/// One primitive's geometry, already baked into the mesh's own space.
#[derive(Debug, Clone, Default)]
pub struct MeshPart {
    pub node: usize,
    pub primitive: usize,
    /// Positions with the node's world transform applied, which is what the
    /// renderer uploads for a static mesh and therefore what the lightmap has
    /// to be measured against.
    pub positions: Vec<[f32; 3]>,
    /// Per-vertex normals, likewise transformed. Used by the baker to shade a
    /// smooth surface smoothly instead of faceting it.
    pub normals: Vec<[f32; 3]>,
    /// Per-vertex texture coordinates (`TEXCOORD_0`), for a texture that says
    /// where a part glows (see [`GlowMap`]); empty when the primitive has none.
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Pack every triangle of every primitive of one mesh asset into one atlas.
///
/// KEYED BY NODE INDEX, NOT BY TRAVERSAL ORDER
///
/// Two pieces of code read this asset: the renderer, walking the node tree it
/// already walks for skinning, and the baker, walking it for geometry. If a
/// primitive's charts were identified by "the nth primitive I visited", the two
/// walks would have to stay in lockstep forever -- and the failure when they
/// drifted would not be a crash, it would be one part of a model wearing
/// another part's lighting.
///
/// A glTF node index is a property of the FILE. Both sides sort by it here, so
/// the packing is identical whatever order either of them happened to visit.
pub fn mesh_lightmap_layout(parts: &[MeshPart], texel: f32) -> Option<MeshLightmapLayout> {
    let mut order: Vec<&MeshPart> = parts.iter().filter(|p| p.indices.len() >= 3).collect();
    order.sort_by_key(|p| (p.node, p.primitive));

    let total: usize = order.iter().map(|p| p.indices.len() / 3).sum();
    if total == 0 || total > MAX_TRIANGLES {
        return None;
    }

    // Concatenated into one triangle soup, with the vertex indices rebased, so
    // the packer sees exactly what it sees for a single primitive.
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges: Vec<PartRange> = Vec::new();
    for p in &order {
        let base = positions.len() as u32;
        let first = indices.len() / 3;
        positions.extend_from_slice(&p.positions);
        // Whole triangles only: a trailing index or two describes nothing, and
        // carrying it through would shift every following chart by one.
        let tri_count = p.indices.len() / 3;
        indices.extend(p.indices[..tri_count * 3].iter().map(|i| i + base));
        ranges.push(PartRange {
            node: p.node,
            primitive: p.primitive,
            first,
            count: tri_count,
        });
    }

    let mut layout = pack_triangles(&positions, &indices, texel)?;
    layout.parts = ranges;
    Some(layout)
}

/// One triangle, measured in its own plane, before anything is packed.
struct Measured {
    u: [f32; 3],
    v: [f32; 3],
    normal: [f32; 3],
    min_u: f32,
    min_v: f32,
    /// The triangle plane's own offset: points on it satisfy dot(n, p) == d.
    plane_d: f32,
    /// Extent of the triangle's bounding box in the (u, v) frame, metres.
    w: f32,
    h: f32,
    /// Corner offsets from the bounding box minimum, in metres along (u, v).
    corners_m: [[f32; 2]; 3],
}

/// Pack every triangle of a mesh into one atlas.
///
/// `None` when the mesh is not a candidate -- no triangles, or more than
/// [`MAX_TRIANGLES`] of them. A caller that gets `None` should keep whatever it
/// did before per-texel lightmaps existed; it is not an error.
///
/// Triangles are shelf-packed in index order without sorting. Sorting packs
/// tighter and would make the layout depend on the relative sizes of unrelated
/// triangles, so editing one corner of a model could move every other chart and
/// invalidate a bake that nothing else had changed.
pub fn pack_triangles(
    positions: &[[f32; 3]],
    indices: &[u32],
    texel: f32,
) -> Option<MeshLightmapLayout> {
    let tri_count = indices.len() / 3;
    if tri_count == 0 || tri_count > MAX_TRIANGLES {
        return None;
    }
    let texel = if texel > 0.0 { texel } else { DEFAULT_TEXEL };

    let mut measured = Vec::with_capacity(tri_count);
    for t in 0..tri_count {
        let p: [[f32; 3]; 3] = [
            *positions.get(indices[t * 3] as usize)?,
            *positions.get(indices[t * 3 + 1] as usize)?,
            *positions.get(indices[t * 3 + 2] as usize)?,
        ];
        let e0 = sub(p[1], p[0]);
        let e1 = sub(p[2], p[0]);
        let n = cross(e0, e1);
        // A degenerate triangle still gets a chart, so `charts[t]` stays
        // addressable by triangle index. It renders nothing, so what its texels
        // hold does not matter -- only that looking it up does not panic.
        let normal = norm(n);
        let u = if dot(e0, e0) > 0.0 { norm(e0) } else { [1.0, 0.0, 0.0] };
        let v = norm(cross(normal, u));

        let mut proj = [[0.0f32; 2]; 3];
        let (mut min_u, mut min_v) = (f32::MAX, f32::MAX);
        let (mut max_u, mut max_v) = (f32::MIN, f32::MIN);
        for (i, q) in p.iter().enumerate() {
            let (pu, pv) = (dot(*q, u), dot(*q, v));
            proj[i] = [pu, pv];
            min_u = min_u.min(pu);
            max_u = max_u.max(pu);
            min_v = min_v.min(pv);
            max_v = max_v.max(pv);
        }
        measured.push(Measured {
            u,
            v,
            normal,
            min_u,
            min_v,
            plane_d: dot(normal, p[0]),
            w: (max_u - min_u).max(0.0),
            h: (max_v - min_v).max(0.0),
            corners_m: [
                [proj[0][0] - min_u, proj[0][1] - min_v],
                [proj[1][0] - min_u, proj[1][1] - min_v],
                [proj[2][0] - min_u, proj[2][1] - min_v],
            ],
        });
    }

    // Halve the density until it fits, exactly as the brush packer does. A
    // huge mesh should cost a blurry lightmap, not a failed bake.
    let mut density_scale = 1.0f32;
    for _ in 0..12 {
        if let Some(layout) = try_pack(&measured, texel, density_scale) {
            return Some(layout);
        }
        density_scale *= 0.5;
    }
    None
}

/// How many texels an extent of `metres` needs at `per_metre` density.
///
/// The epsilon keeps a span that is an exact multiple of the texel size from
/// rounding up to one texel more than it needs, which is a pure space saving --
/// the chart is sized from its own extent below, so an extra texel would be
/// correct, just wasteful.
fn texels_across(metres: f32, per_metre: f32) -> u32 {
    let raw = metres * per_metre;
    ((raw - 1e-4).ceil().max(1.0)) as u32
}

/// The world span one texel covers, given the extent it has to fill.
///
/// The nominal density is NOT used: the texel count was rounded up, always, by
/// `ceil` or by the `MIN_CHART` clamp, so the nominal size would make the chart
/// cover more than the triangle -- a strip of texel centres hanging off the
/// edge in open air, shaded as if they were surface, which bilinear filtering
/// then pulls back in as a bright rim.
fn span_per_texel(metres: f32, count: u32) -> f32 {
    if count == 0 || metres <= 0.0 {
        return 0.0;
    }
    metres / count as f32
}

fn try_pack(measured: &[Measured], texel: f32, density_scale: f32) -> Option<MeshLightmapLayout> {
    let per_metre = density_scale / texel;
    let sized: Vec<(u32, u32)> = measured
        .iter()
        .map(|m| {
            (
                texels_across(m.w, per_metre).clamp(MIN_CHART, MAX_ATLAS),
                texels_across(m.h, per_metre).clamp(MIN_CHART, MAX_ATLAS),
            )
        })
        .collect();

    let area: u64 = sized
        .iter()
        .map(|(w, h)| ((w + 2 * GUTTER) as u64) * ((h + 2 * GUTTER) as u64))
        .sum();
    let mut width = (area as f64).sqrt().ceil().max(4.0) as u32;
    width = width.next_power_of_two().min(MAX_ATLAS);

    let mut charts = Vec::with_capacity(measured.len());
    let (mut shelf_y, mut shelf_h, mut cursor_x) = (0u32, 0u32, 0u32);
    for (m, &(w, h)) in measured.iter().zip(sized.iter()) {
        let (pw, ph) = (w + 2 * GUTTER, h + 2 * GUTTER);
        if pw > width {
            return None;
        }
        if cursor_x + pw > width {
            shelf_y += shelf_h;
            shelf_h = 0;
            cursor_x = 0;
        }
        if shelf_y + ph > MAX_ATLAS {
            return None;
        }

        // Texel size from the triangle's OWN extent divided by the texels it
        // got, rather than from the nominal density. The two disagree whenever
        // the count was rounded -- always upward, by `ceil` or by the
        // `MIN_CHART` clamp -- and using the nominal size then makes the chart
        // cover MORE than the triangle: a strip of texel centres hanging off
        // the edge in open air, shaded as if they were surface, which bilinear
        // filtering pulls back in as a bright rim.
        //
        let step_u = span_per_texel(m.w, w);
        let step_v = span_per_texel(m.h, h);
        let du = mul(m.u, step_u);
        let dv = mul(m.v, step_v);
        // The chart's texel (0,0) centre, half a texel in from the triangle's
        // own minimum corner in its plane.
        let origin = add(
            add(mul(m.u, m.min_u), mul(m.v, m.min_v)),
            add(mul(du, 0.5), mul(dv, 0.5)),
        );
        // `u` and `v` span the triangle's DIRECTION but say nothing about how
        // far along the normal it sits, so the point built from them lies on
        // the parallel plane through the mesh origin. Lifting it onto the
        // triangle's own plane is what makes `texel_local` return points that
        // are actually on the surface -- and for a triangle through the origin
        // it would look perfectly correct without this.
        let along = dot(origin, m.normal);
        let origin = add(origin, mul(m.normal, m.plane_d - along));

        let (cx, cy) = (cursor_x + GUTTER, shelf_y + GUTTER);
        // Corner offsets are metres from the bounding box minimum, and texel
        // (0,0)'s centre sits half a texel in from it -- hence the -0.5, which
        // is what keeps `corners`, `origin` and `uv2` describing one mapping
        // rather than three that nearly agree.
        let to_texels = |c: [f32; 2]| {
            [
                if step_u > 0.0 { c[0] / step_u - 0.5 } else { 0.0 },
                if step_v > 0.0 { c[1] / step_v - 0.5 } else { 0.0 },
            ]
        };
        charts.push(MeshChart {
            x: cx,
            y: cy,
            w,
            h,
            origin,
            du,
            dv,
            normal: m.normal,
            corners: [
                to_texels(m.corners_m[0]),
                to_texels(m.corners_m[1]),
                to_texels(m.corners_m[2]),
            ],
        });

        cursor_x += pw;
        shelf_h = shelf_h.max(ph);
    }

    let height = (shelf_y + shelf_h).max(1).next_power_of_two().min(MAX_ATLAS);
    if shelf_y + shelf_h > height {
        return None;
    }
    Some(MeshLightmapLayout { width, height, charts, parts: Vec::new(), density_scale })
}

/// Read one glTF's static geometry, in the form the layout wants.
///
/// `None` when the asset is not a per-texel lightmap candidate:
///
/// * it will not load, or has no triangles;
/// * it carries a SKIN or an ANIMATION. Both mean the geometry moves, and a
///   lightmap baked against where it happened to be at export time would then
///   be wrong everywhere else. The renderer already refuses to bake node
///   transforms into a skinned primitive's vertices for the same reason, so
///   this is the same rule stated on the other side. Lighting for those is the
///   dynamic-object problem, not this one.
///
/// Positions and normals come back with the node hierarchy's transform applied,
/// because that is what the renderer puts in the vertex buffer for a static
/// mesh -- and the lightmap has to be measured against the geometry that is
/// actually drawn, not against the untransformed source.
pub fn load_mesh_parts(path: &std::path::Path) -> Option<Vec<MeshPart>> {
    // Buffers only. `gltf::import` also DECODES every image, which for a mesh
    // with 2K textures is most of the load time and all of the memory -- and
    // this asks a question about geometry. The client calls this on the headset
    // for every static mesh in a level.
    let gltf = gltf::Gltf::open(path).ok()?;
    let base = path.parent();
    let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob.clone()).ok()?;
    mesh_parts(&gltf.document, &buffers)
}

/// The triangles of a model that BLOCK LIGHT, in model space (node transforms
/// applied): every primitive except glass and bulbs.
///
/// A light fixture's housing shadows its own lamp and every other -- a sconce's
/// back plate keeps its light off the wall behind it -- but its glass lets the
/// light through, and its bulb IS the light. Glass is a material with
/// `KHR_materials_transmission`; a bulb is a material that glows as a whole
/// (an emissive factor and no emissive texture). A housing that carries its
/// bulb in an emissive TEXTURE -- black everywhere but the filament -- still
/// blocks; the bake keeps shadow rays clear of the last few centimetres around
/// a lamp for that case. Read from the file's own JSON because those material
/// properties are extensions this crate's glTF reader is not built with.
pub fn light_blocking_triangles(path: &std::path::Path) -> Option<Vec<[glam::Vec3; 3]>> {
    fixture_light_geometry(path).map(|g| g.blocking)
}

/// A fixture model's light-blocking triangles (see [`light_blocking_triangles`])
/// and whether its bulb was among what was dropped.
pub struct FixtureLightGeometry {
    pub blocking: Vec<[glam::Vec3; 3]>,
    /// The bulb was dropped -- a primitive that glows as a whole (the
    /// sconce's), or the triangles an emissive texture paints the glow on
    /// (the hanging lamp's) -- so nothing left in `blocking` is the bulb:
    /// every triangle there, however close to the lamp, is housing that
    /// really shades it.
    pub bulb_dropped: bool,
}

/// A triangle of a housing whose emissive texture glows at its middle past
/// this share of the texture's brightest is the bulb painted there. Relative,
/// so a JPEG mask's noise in the black never counts.
const PAINTED_BULB_SHARE: f32 = 0.1;

/// [`light_blocking_triangles`], and whether the bulb was its own primitive.
pub fn fixture_light_geometry(path: &std::path::Path) -> Option<FixtureLightGeometry> {
    let gltf = gltf::Gltf::open(path).ok()?;
    let base = path.parent();
    let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob.clone()).ok()?;
    let parts = mesh_parts(&gltf.document, &buffers)?;
    let json: serde_json::Value = serde_json::from_slice(&gltf_json_bytes(path)?).ok()?;
    let passes_light = |material: Option<usize>| material_passes_light(&json, material);
    let mut out = Vec::new();
    let mut bulb_dropped = false;
    let mut glow_maps: std::collections::HashMap<Option<usize>, Option<(GlowMap, f32)>> = std::collections::HashMap::new();
    for part in &parts {
        let material = gltf
            .document
            .nodes()
            .nth(part.node)
            .and_then(|n| n.mesh())
            .and_then(|m| m.primitives().nth(part.primitive))
            .and_then(|p| p.material().index());
        let (passes, bulb) = passes_light(material);
        if passes {
            bulb_dropped |= bulb && !part.indices.is_empty();
            continue;
        }
        // A BULB PAINTED INTO THE HOUSING'S TEXTURE, as the hanging lamp's is:
        // the triangles its emissive texture lights. Dropped as a bulb
        // primitive is, so the housing round it shades the lamp to within
        // the last few centimetres -- with the bulb kept, a 12 cm clearance
        // had to keep shadow rays off it, and once the lamp sat in its bulb
        // the outside of the bell's crown, inside those 12 cm, saw the lamp
        // through the metal: a glowing ring round the neck (2026-10-01).
        let glow = glow_maps
            .entry(material)
            .or_insert_with(|| {
                let map = material_glow_map(&json, material, &gltf.document, &buffers, path)?;
                let brightest = map.texels.iter().map(|t| t.max_element()).fold(0.0f32, f32::max);
                (brightest > 0.0).then_some((map, brightest))
            })
            .as_ref();
        for tri in part.indices.chunks_exact(3) {
            if let Some((map, brightest)) = glow {
                let uv = |i: u32| part.uvs.get(i as usize).copied().unwrap_or([0.0, 0.0]);
                let (a, b, c) = (uv(tri[0]), uv(tri[1]), uv(tri[2]));
                let middle = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
                if map.sample(middle).max_element() > PAINTED_BULB_SHARE * brightest {
                    bulb_dropped = true;
                    continue;
                }
            }
            let v = |i: u32| glam::Vec3::from(part.positions[i as usize]);
            out.push([v(tri[0]), v(tri[1]), v(tri[2])]);
        }
    }
    Some(FixtureLightGeometry { blocking: out, bulb_dropped })
}

/// Whether a material lets light through, and whether it is a bulb: glass is
/// `KHR_materials_transmission`; a bulb glows as a whole (an emissive factor
/// and no emissive texture). See [`light_blocking_triangles`].
fn material_passes_light(json: &serde_json::Value, material: Option<usize>) -> (bool, bool) {
    let Some(m) = material.and_then(|i| json["materials"].get(i)) else { return (false, false) };
    let transmissive = m["extensions"]["KHR_materials_transmission"]["transmissionFactor"]
        .as_f64()
        .is_some_and(|t| t > 0.5);
    let glows_whole = m["emissiveFactor"]
        .as_array()
        .is_some_and(|f| f.iter().filter_map(|v| v.as_f64()).any(|v| v > 0.0))
        && m.get("emissiveTexture").is_none();
    (transmissive || glows_whole, glows_whole)
}

/// A MODEL'S MEAN SURFACE COLOUR, in linear light: each light-blocking
/// primitive's base colour -- its factor times its texture's mean -- weighted
/// by the primitive's area. Bulbs and glass are left out: one glows, the other
/// shows what lies behind it. `None` when the model cannot be read.
///
/// For the reflection trace, where a reflection meets a model somewhere no
/// probe photographed: the top of a lamp, when every photograph is taken from
/// below it. Read from the nearest photograph instead, that direction showed
/// the lamp's glowing mouth, and the ceiling above each sconce reflected a
/// bright ghost of it (headset, 2026-09-29).
pub fn model_albedo(path: &std::path::Path) -> Option<glam::Vec3> {
    let gltf = gltf::Gltf::open(path).ok()?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone()).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&gltf_json_bytes(path)?).ok()?;
    let mut texture_means: std::collections::HashMap<usize, Option<glam::Vec3>> = std::collections::HashMap::new();
    // A SKINNED model -- a character -- has no static geometry to weigh by
    // area (`mesh_parts` declines it): each primitive counts once instead.
    let Some(parts) = mesh_parts(&gltf.document, &buffers) else {
        let (mut sum, mut n) = (glam::Vec3::ZERO, 0u32);
        for primitive in gltf.document.meshes().flat_map(|m| m.primitives()) {
            let material = primitive.material().index();
            if material_passes_light(&json, material).0 {
                continue;
            }
            sum += material_base_colour(&json, material, &gltf.document, &buffers, path, &mut texture_means);
            n += 1;
        }
        return (n > 0).then(|| sum / n as f32);
    };
    let (mut sum, mut total) = (glam::Vec3::ZERO, 0.0f32);
    for part in &parts {
        let material = gltf
            .document
            .nodes()
            .nth(part.node)
            .and_then(|n| n.mesh())
            .and_then(|m| m.primitives().nth(part.primitive))
            .and_then(|p| p.material().index());
        if material_passes_light(&json, material).0 {
            continue;
        }
        let area: f32 = part
            .indices
            .chunks_exact(3)
            .map(|t| {
                let v = |i: u32| glam::Vec3::from(part.positions[i as usize]);
                0.5 * (v(t[1]) - v(t[0])).cross(v(t[2]) - v(t[0])).length()
            })
            .sum();
        if area <= 0.0 {
            continue;
        }
        sum += material_base_colour(&json, material, &gltf.document, &buffers, path, &mut texture_means) * area;
        total += area;
    }
    (total > 0.0).then(|| sum / total)
}

/// One material's triangles in a model, in model space (node transforms
/// applied), with that material's base colour, linear, and the glow it gives
/// off at a drive of one (see `scene_light::emissive_drive`).
pub struct AlbedoPart {
    pub triangles: Vec<[glam::Vec3; 3]>,
    pub albedo: glam::Vec3,
    /// The material's `emissiveFactor` where it glows whole -- no emissive
    /// texture, the glTF way of saying "this part is the bulb". Zero for a
    /// part that does not glow, and for one whose glow a texture places (the
    /// hanging lamp's housing): a whole-part mean of the mask would light the
    /// housing. The renderer draws the glow as factor x mask x drive
    /// (`mesh_pipeline`).
    pub emissive: glam::Vec3,
    /// Where a TEXTURE places the glow -- the hanging lamp's one material
    /// covers housing and bulb alike, and its emissive texture is black but
    /// for the bulb -- the texture and each triangle's coordinates on it.
    /// `None` for a part that glows whole or not at all.
    pub glow_map: Option<GlowMap>,
}

/// A glow placed by an emissive texture: the texture times the material's
/// `emissiveFactor`, linear, as the renderer draws it at a drive of one
/// (`mesh::texture`: factor x mask; `KHR_materials_emissive_strength` is left
/// to the light's intensity, as there), and each triangle's texture
/// coordinates in its part's `triangles` order.
pub struct GlowMap {
    pub width: u32,
    pub height: u32,
    /// Row by row from the image's top, as glTF's `v` runs.
    pub texels: Vec<glam::Vec3>,
    pub uvs: Vec<[[f32; 2]; 3]>,
}

impl GlowMap {
    /// The glow on triangle `triangle` at barycentric `u`, `v` -- the weights
    /// of its second and third corners, as a ray-triangle test returns them --
    /// read bilinearly, the texture repeating as glTF's default sampler does.
    pub fn at(&self, triangle: usize, u: f32, v: f32) -> glam::Vec3 {
        let Some(t) = self.uvs.get(triangle) else { return glam::Vec3::ZERO };
        if self.width == 0 || self.height == 0 {
            return glam::Vec3::ZERO;
        }
        let w = 1.0 - u - v;
        let s = w * t[0][0] + u * t[1][0] + v * t[2][0];
        let r = w * t[0][1] + u * t[1][1] + v * t[2][1];
        self.sample([s, r])
    }

    /// The glow at texture coordinates `uv`, read as [`Self::at`] reads it.
    pub fn sample(&self, uv: [f32; 2]) -> glam::Vec3 {
        if self.width == 0 || self.height == 0 {
            return glam::Vec3::ZERO;
        }
        let x = uv[0] * self.width as f32 - 0.5;
        let y = uv[1] * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |xi: f32, yi: f32| {
            let xi = (xi as i64).rem_euclid(self.width as i64) as usize;
            let yi = (yi as i64).rem_euclid(self.height as i64) as usize;
            self.texels[yi * self.width as usize + xi]
        };
        let top = texel(x0, y0) * (1.0 - fx) + texel(x0 + 1.0, y0) * fx;
        let bottom = texel(x0, y0 + 1.0) * (1.0 - fx) + texel(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// A MODEL AS A REFLECTION SEES IT, part by part: every triangle but glass's,
/// grouped by material, each with that material's base colour and glow.
/// [`model_albedo`] is one colour for the whole model; the baker's reflection
/// cards (`reflection_cards`) colour a lamp's dark shade and its pale plate
/// apart. BULBS INCLUDED, unlike the light-blocking triangles
/// ([`light_blocking_triangles`]): a bulb must not shadow its own lamp, but a
/// reflection sees it -- left off the cards, a sconce's bulb reflected as a
/// dark hole in its glowing mouth, "the bulb showing as a shadow" (user,
/// 2026-09-30). `None` when the model cannot be read, or is skinned.
pub fn model_albedo_parts(path: &std::path::Path) -> Option<Vec<AlbedoPart>> {
    let gltf = gltf::Gltf::open(path).ok()?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone()).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&gltf_json_bytes(path)?).ok()?;
    let parts = mesh_parts(&gltf.document, &buffers)?;
    let mut texture_means: std::collections::HashMap<usize, Option<glam::Vec3>> = std::collections::HashMap::new();
    let mut by_material: Vec<(Option<usize>, AlbedoPart)> = Vec::new();
    for part in &parts {
        let material = gltf
            .document
            .nodes()
            .nth(part.node)
            .and_then(|n| n.mesh())
            .and_then(|m| m.primitives().nth(part.primitive))
            .and_then(|p| p.material().index());
        // Glass shows what lies behind it; a bulb is seen.
        let (passes_light, glows_whole) = material_passes_light(&json, material);
        if passes_light && !glows_whole {
            continue;
        }
        let at = match by_material.iter().position(|(m, _)| *m == material) {
            Some(at) => at,
            None => {
                let albedo = material_base_colour(&json, material, &gltf.document, &buffers, path, &mut texture_means);
                let emissive = if glows_whole { material_emissive_factor(&json, material) } else { glam::Vec3::ZERO };
                let glow_map = if glows_whole { None } else { material_glow_map(&json, material, &gltf.document, &buffers, path) };
                by_material.push((material, AlbedoPart { triangles: Vec::new(), albedo, emissive, glow_map }));
                by_material.len() - 1
            }
        };
        let v = |i: u32| glam::Vec3::from(part.positions[i as usize]);
        let into = &mut by_material[at].1;
        into.triangles.extend(part.indices.chunks_exact(3).map(|t| [v(t[0]), v(t[1]), v(t[2])]));
        if let Some(map) = into.glow_map.as_mut() {
            // Aligned with the triangles; a primitive with no coordinates is
            // read at the texture's corner, which is what the renderer does.
            let uv = |i: u32| part.uvs.get(i as usize).copied().unwrap_or([0.0, 0.0]);
            map.uvs.extend(part.indices.chunks_exact(3).map(|t| [uv(t[0]), uv(t[1]), uv(t[2])]));
        }
    }
    Some(by_material.into_iter().map(|(_, p)| p).filter(|p| !p.triangles.is_empty()).collect())
}

/// A material's emissive texture times its `emissiveFactor`, linear -- the
/// glow a texture places -- with no triangles yet; `None` without an emissive
/// texture, or with a black factor (glTF's default: no glow whatever the
/// texture says).
fn material_glow_map(
    json: &serde_json::Value,
    material: Option<usize>,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path: &std::path::Path,
) -> Option<GlowMap> {
    let m = &json["materials"][material?];
    let factor = material_emissive_factor(json, material);
    if factor.max_element() <= 0.0 {
        return None;
    }
    let texture = m["emissiveTexture"]["index"].as_u64()?;
    let image = json["textures"][texture as usize]["source"].as_u64()? as usize;
    let rgb = gltf_image_decode(doc, buffers, path, image)?.to_rgb8();
    let linear = |b: u8| {
        let c = b as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let texels = rgb.pixels().map(|p| glam::Vec3::new(linear(p.0[0]), linear(p.0[1]), linear(p.0[2])) * factor).collect();
    Some(GlowMap { width: rgb.width(), height: rgb.height(), texels, uvs: Vec::new() })
}

/// A material's `emissiveFactor`, or zero.
fn material_emissive_factor(json: &serde_json::Value, material: Option<usize>) -> glam::Vec3 {
    let Some(f) = material.and_then(|i| json["materials"][i]["emissiveFactor"].as_array()) else { return glam::Vec3::ZERO };
    let c = |k: usize| f.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    glam::Vec3::new(c(0), c(1), c(2))
}

/// A material's base colour, linear: its factor times its texture's mean.
fn material_base_colour(
    json: &serde_json::Value,
    material: Option<usize>,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path: &std::path::Path,
    texture_means: &mut std::collections::HashMap<usize, Option<glam::Vec3>>,
) -> glam::Vec3 {
    let pbr = material.map(|i| &json["materials"][i]["pbrMetallicRoughness"]);
    let factor = pbr
        .and_then(|p| p["baseColorFactor"].as_array())
        .map(|f| {
            glam::Vec3::new(
                f.first().and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                f.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                f.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            )
        })
        .unwrap_or(glam::Vec3::ONE);
    let texture = pbr
        .and_then(|p| p["baseColorTexture"]["index"].as_u64())
        .and_then(|t| json["textures"][t as usize]["source"].as_u64())
        .map(|s| s as usize);
    let mean = match texture {
        Some(image) => *texture_means.entry(image).or_insert_with(|| gltf_image_mean(doc, buffers, path, image)),
        None => None,
    };
    factor * mean.unwrap_or(glam::Vec3::ONE)
}

/// The mean of image `index` of a glTF, in linear light: every fourth texel
/// each way, which for a 1K colour map is 65 thousand samples and a fraction
/// of the decode. From its file beside the model or from the binary buffer.
fn gltf_image_mean(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path: &std::path::Path,
    index: usize,
) -> Option<glam::Vec3> {
    let rgb = gltf_image_decode(doc, buffers, path, index)?.to_rgb8();
    let linear = |b: u8| {
        let c = b as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (mut sum, mut n) = (glam::Vec3::ZERO, 0u32);
    for y in (0..rgb.height()).step_by(4) {
        for x in (0..rgb.width()).step_by(4) {
            let p = rgb.get_pixel(x, y).0;
            sum += glam::Vec3::new(linear(p[0]), linear(p[1]), linear(p[2]));
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Image `index` of a glTF, decoded: from its file beside the model or from
/// the binary buffer.
fn gltf_image_decode(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path: &std::path::Path,
    index: usize,
) -> Option<image::DynamicImage> {
    let image = doc.images().nth(index)?;
    match image.source() {
        gltf::image::Source::Uri { uri, .. } => {
            let file = path.parent()?.join(percent_decode(uri));
            image::load_from_memory(&std::fs::read(file).ok()?).ok()
        }
        gltf::image::Source::View { view, .. } => {
            let data = &buffers.get(view.buffer().index())?.0;
            image::load_from_memory(data.get(view.offset()..view.offset() + view.length())?).ok()
        }
    }
}

/// A glTF uri's `%20` and friends, as a file name.
fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The JSON of a `.gltf` (the file) or a `.glb` (its first chunk).
fn gltf_json_bytes(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() >= 20 && &bytes[0..4] == b"glTF" {
        let len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        return bytes.get(20..20 + len).map(|b| b.to_vec());
    }
    Some(bytes)
}

/// As [`load_mesh_parts`], for a caller that has already parsed the document.
///
/// The renderer has: it is standing in the middle of building vertex buffers
/// from the very primitives this measures. Re-importing a 27MB glb to ask the
/// same question of the same bytes is not a rounding error on a headset.
pub fn mesh_parts(doc: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Option<Vec<MeshPart>> {
    if doc.skins().len() > 0 || doc.animations().len() > 0 {
        return None;
    }

    fn walk(
        node: gltf::Node,
        parent: [[f32; 4]; 4],
        buffers: &[gltf::buffer::Data],
        out: &mut Vec<MeshPart>,
    ) {
        let local = node.transform().matrix();
        let world = mat_mul(parent, local);
        if let Some(mesh) = node.mesh() {
            for (pi, prim) in mesh.primitives().enumerate() {
                let reader = prim.reader(|b| Some(&buffers[b.index()]));
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<[f32; 3]> =
                    positions.map(|p| transform_point(world, p)).collect();
                if positions.is_empty() {
                    continue;
                }
                let normals: Vec<[f32; 3]> = match reader.read_normals() {
                    Some(n) => n.map(|n| norm(transform_vector(world, n))).collect(),
                    None => vec![[0.0, 1.0, 0.0]; positions.len()],
                };
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .filter(|t: &Vec<[f32; 2]>| t.len() == positions.len())
                    .unwrap_or_default();
                out.push(MeshPart {
                    node: node.index(),
                    primitive: pi,
                    positions,
                    normals,
                    uvs,
                    indices,
                });
            }
        }
        for child in node.children() {
            walk(child, world, buffers, out);
        }
    }

    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut out = Vec::new();
    for scene in doc.scenes() {
        for node in scene.nodes() {
            walk(node, identity, buffers, &mut out);
        }
        // The FIRST scene only. glTF allows several and names one as the
        // default; walking them all would lay out geometry that is never drawn
        // together, and every extra chart is atlas space taken from the mesh
        // that is.
        break;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// Column-major 4x4 multiply, matching `gltf`'s `transform().matrix()` layout.
///
/// Written out rather than pulled from glam so this module has no linear
/// algebra dependency of its own -- it is handed arrays by both consumers and
/// hands arrays back.
fn mat_mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

fn transform_point(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

fn transform_vector(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2],
    ]
}

/// The uv2 a renderer needs, as one array per primitive.
///
/// The layout's own form is charts -- rectangles and world axes -- because that
/// is what the BAKER needs: which point in space each texel stands for. A
/// renderer needs the inverse and nothing else: which atlas coordinate each
/// triangle corner samples. Converting here rather than exporting `MeshChart`
/// keeps the renderer free of the concept, which matters because the renderer
/// is a separate, standalone crate that has no business knowing what a bake is.
///
/// Each entry is `((node, primitive), uv2 per corner)`, one uv2 for every index
/// in that primitive -- so the consumer can check the length against its own
/// index buffer and refuse a set that describes different geometry.
pub fn corner_uv2(layout: &MeshLightmapLayout) -> Vec<((usize, usize), Vec<[f32; 2]>)> {
    layout
        .parts
        .iter()
        .map(|part| {
            let mut uv = Vec::with_capacity(part.count * 3);
            for t in 0..part.count {
                let chart = &layout.charts[part.first + t];
                for k in 0..3 {
                    uv.push(chart.uv2(k, layout.width, layout.height));
                }
            }
            ((part.node, part.primitive), uv)
        })
        .collect()
}

/// Everything a renderer needs for one asset, or `None` if it is not a
/// candidate. The one call a client should make.
pub fn lightmap_uv_for_asset(
    path: &std::path::Path,
) -> Option<Vec<((usize, usize), Vec<[f32; 2]>)>> {
    let parts = load_mesh_parts(path)?;
    let layout = mesh_lightmap_layout(&parts, DEFAULT_TEXEL)?;
    Some(corner_uv2(&layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit right triangle in the XZ plane, plus a tilted one, plus a sliver.
    fn sample_mesh() -> (Vec<[f32; 3]>, Vec<u32>) {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            // Tilted, and nowhere near the origin -- a chart whose plane passes
            // through the origin hides an entire class of mistake.
            [5.0, 2.0, -3.0],
            [5.7, 2.4, -3.0],
            [5.0, 2.9, -3.6],
            // A sliver: 40cm long and a tenth of a millimetre wide.
            [0.0, 4.0, 0.0],
            [0.4, 4.0, 0.0],
            [0.4, 4.0001, 0.0],
        ];
        let indices = vec![0, 1, 2, 3, 4, 5, 6, 7, 8];
        (positions, indices)
    }

    fn tri_of(positions: &[[f32; 3]], indices: &[u32], t: usize) -> [[f32; 3]; 3] {
        [
            positions[indices[t * 3] as usize],
            positions[indices[t * 3 + 1] as usize],
            positions[indices[t * 3 + 2] as usize],
        ]
    }

    /// THE invariant: the chart and the triangle describe one mapping.
    ///
    /// For every texel, the barycentric weights the chart reports are blended
    /// into the triangle's real corner positions and compared with the point
    /// the chart says that texel samples. If `origin`, `du`, `dv` and `corners`
    /// disagree about anything -- the half-texel offset, which corner the box
    /// is anchored on, whether the plane offset was applied -- the two answers
    /// separate and the bake lands somewhere the renderer never samples.
    #[test]
    fn every_texel_agrees_with_the_triangle_it_belongs_to() {
        let (positions, indices) = sample_mesh();
        let layout = pack_triangles(&positions, &indices, DEFAULT_TEXEL).unwrap();
        let mut checked = 0;
        for (t, chart) in layout.charts.iter().enumerate() {
            let p = tri_of(&positions, &indices, t);
            for ty in 0..chart.h {
                for tx in 0..chart.w {
                    let b = chart.barycentric(tx, ty);
                    let blended = add(
                        add(mul(p[0], b[0]), mul(p[1], b[1])),
                        mul(p[2], b[2]),
                    );
                    let direct = chart.texel_local(tx as f32, ty as f32);
                    let d = sub(blended, direct);
                    assert!(
                        dot(d, d).sqrt() < 1e-3,
                        "triangle {t} texel ({tx},{ty}): barycentric says {blended:?} \
                         but the chart samples {direct:?}",
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked >= 12, "only {checked} texels checked; the mesh got no charts");
    }

    /// A corner's uv2 must address the atlas where that corner actually is.
    #[test]
    fn a_corner_uv2_lands_on_its_own_chart() {
        let (positions, indices) = sample_mesh();
        let layout = pack_triangles(&positions, &indices, DEFAULT_TEXEL).unwrap();
        for (t, chart) in layout.charts.iter().enumerate() {
            for corner in 0..3 {
                let uv = chart.uv2(corner, layout.width, layout.height);
                let px = uv[0] * layout.width as f32;
                let py = uv[1] * layout.height as f32;
                // Within the chart plus its gutter. A corner sits ON the chart
                // boundary by construction (it is half a texel outside the
                // outermost texel centre), which is what the gutter is for.
                assert!(
                    px >= chart.x as f32 - GUTTER as f32
                        && px <= (chart.x + chart.w + GUTTER) as f32
                        && py >= chart.y as f32 - GUTTER as f32
                        && py <= (chart.y + chart.h + GUTTER) as f32,
                    "triangle {t} corner {corner} at ({px}, {py}) is outside chart \
                     {:?}",
                    (chart.x, chart.y, chart.w, chart.h),
                );
            }
        }
    }

    #[test]
    fn charts_do_not_overlap_and_stay_inside_the_atlas() {
        let (positions, indices) = sample_mesh();
        let layout = pack_triangles(&positions, &indices, DEFAULT_TEXEL).unwrap();
        let mut used = vec![false; (layout.width * layout.height) as usize];
        for (t, c) in layout.charts.iter().enumerate() {
            assert!(
                c.x + c.w + GUTTER <= layout.width && c.y + c.h + GUTTER <= layout.height,
                "triangle {t}'s chart runs off a {}x{} atlas",
                layout.width,
                layout.height,
            );
            for y in c.y..c.y + c.h {
                for x in c.x..c.x + c.w {
                    let i = (y * layout.width + x) as usize;
                    assert!(!used[i], "triangle {t} reuses texel ({x}, {y})");
                    used[i] = true;
                }
            }
        }
    }

    /// A texel outside the triangle must be reported as outside.
    ///
    /// Half of every chart is the other side of the hypotenuse. Shading it as
    /// if it were surface is what puts light on a texel the triangle cannot
    /// see, and bilinear filtering then drags that into the visible half.
    #[test]
    fn texels_off_the_triangle_read_as_negative() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let layout = pack_triangles(&positions, &[0, 1, 2], 0.1).unwrap();
        let c = &layout.charts[0];
        let inside = |tx, ty| c.barycentric(tx, ty).iter().all(|w| *w >= 0.0);
        // The right-angle corner is on the triangle; the opposite corner of the
        // bounding box is the far side of the hypotenuse.
        assert!(inside(0, 0), "the right-angle corner should be inside");
        assert!(
            !inside(c.w - 1, c.h - 1),
            "the far bounding-box corner is across the hypotenuse and should be outside",
        );
    }

    #[test]
    fn a_mesh_past_the_triangle_budget_gets_no_layout() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let indices: Vec<u32> = (0..(MAX_TRIANGLES + 1) * 3).map(|i| (i % 3) as u32).collect();
        assert!(
            pack_triangles(&positions, &indices, DEFAULT_TEXEL).is_none(),
            "a mesh past MAX_TRIANGLES must fall back rather than pack an atlas",
        );
        assert!(pack_triangles(&positions, &[], DEFAULT_TEXEL).is_none());
    }

    /// Density is given up rather than the bake failing.
    #[test]
    fn a_dense_mesh_scales_its_density_down_to_fit() {
        // Enough triangles that MIN_CHART alone would overflow the atlas is not
        // reachable under MAX_TRIANGLES, so this checks the other end: big
        // triangles that cannot fit at full density.
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for i in 0..64u32 {
            let z = i as f32 * 10.0;
            positions.extend_from_slice(&[
                [0.0, 0.0, z],
                [20.0, 0.0, z],
                [0.0, 20.0, z],
            ]);
            indices.extend_from_slice(&[i * 3, i * 3 + 1, i * 3 + 2]);
        }
        let layout = pack_triangles(&positions, &indices, 0.01).unwrap();
        assert!(
            layout.density_scale < 1.0,
            "2000-texel-wide charts fit at full density? density_scale = {}",
            layout.density_scale,
        );
        assert!(layout.width <= MAX_ATLAS && layout.height <= MAX_ATLAS);
    }

    /// The real asset this feature was built for.
    #[test]
    fn a_nine_thousand_triangle_prop_gets_a_sane_atlas() {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        // 1cm triangles, the density of the hanging lamp.
        for i in 0..9530u32 {
            let x = (i % 100) as f32 * 0.01;
            let y = (i / 100) as f32 * 0.01;
            positions.extend_from_slice(&[
                [x, y, 0.0],
                [x + 0.01, y, 0.0],
                [x, y + 0.01, 0.0],
            ]);
            indices.extend_from_slice(&[i * 3, i * 3 + 1, i * 3 + 2]);
        }
        let layout = pack_triangles(&positions, &indices, DEFAULT_TEXEL).unwrap();
        assert_eq!(layout.charts.len(), 9530);
        assert!(
            layout.width * layout.height <= 1024 * 1024,
            "{}x{} atlas for one prop",
            layout.width,
            layout.height,
        );
    }
}

#[cfg(test)]
mod asset_tests {
    //! Against the real hanging lamp, which is the asset this was built for.
    //!
    //! Skipped rather than failed when the model is absent: the crate is built
    //! in checkouts that do not carry `game/`, and a test that cannot run is
    //! not a test that failed.
    use super::*;
    use std::path::PathBuf;

    fn lamp() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../game/models/lights/hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        p.is_file().then_some(p)
    }

    #[test]
    fn the_hanging_lamp_lays_out_into_one_atlas() {
        let Some(path) = lamp() else { return };
        let parts = load_mesh_parts(&path).expect("lamp has static geometry");
        let tris: usize = parts.iter().map(|p| p.indices.len() / 3).sum();
        let layout = mesh_lightmap_layout(&parts, DEFAULT_TEXEL).expect("lamp fits an atlas");
        eprintln!(
            "lamp: {} parts, {tris} triangles -> {}x{} atlas, density {}",
            parts.len(),
            layout.width,
            layout.height,
            layout.density_scale,
        );
        assert_eq!(layout.charts.len(), tris);
        assert_eq!(layout.parts.len(), parts.len());
        // Every part addressable, and every chart on the triangle it names.
        for part in &parts {
            for t in 0..part.indices.len() / 3 {
                let chart = layout
                    .chart(part.node, part.primitive, t)
                    .unwrap_or_else(|| panic!("node {} prim {} tri {t} has no chart", part.node, part.primitive));
                let p = [
                    part.positions[part.indices[t * 3] as usize],
                    part.positions[part.indices[t * 3 + 1] as usize],
                    part.positions[part.indices[t * 3 + 2] as usize],
                ];
                let b = chart.barycentric(0, 0);
                let blended = add(add(mul(p[0], b[0]), mul(p[1], b[1])), mul(p[2], b[2]));
                let d = sub(blended, chart.texel_local(0.0, 0.0));
                assert!(dot(d, d).sqrt() < 1e-3, "chart and triangle disagree");
            }
        }
    }
    /// Why the renderer must not let glass cast a shadow.
    ///
    /// This measures the ASSET, not the renderer: from the hanging lamp's bulb
    /// socket, the transmissive glass envelope blocks every downward ray while
    /// the opaque housing around it blocks almost none. So a shadow pass that
    /// treats transmission as opaque seals the bulb inside its own fixture and
    /// the room goes black -- which is exactly what `test_room` did, with the
    /// lamp still visibly glowing, so it read as a lighting bug rather than a
    /// shadow one.
    ///
    /// It lives here rather than beside the fix because the fix is a threshold
    /// in another crate and would pass on any asset. This is the evidence the
    /// threshold exists for, and it fails if someone re-exports the lamp with
    /// its glass opaque.
    #[test]
    fn the_lamps_glass_would_seal_its_own_bulb_in_shadow() {
        let Some(path) = lamp() else { return };
        let parts = load_mesh_parts(&path).unwrap();
        // The socket from fixture.json.
        let origin = [0.0f32, -1.18, 0.0];
        let mut by_part: Vec<Vec<[[f32; 3]; 3]>> = Vec::new();
        for p in &parts {
            let mut tris = Vec::new();
            for t in 0..p.indices.len() / 3 {
                tris.push([
                    p.positions[p.indices[t * 3] as usize],
                    p.positions[p.indices[t * 3 + 1] as usize],
                    p.positions[p.indices[t * 3 + 2] as usize],
                ]);
            }
            by_part.push(tris);
        }
        let tris: Vec<[[f32; 3]; 3]> = by_part.iter().flatten().copied().collect();
        // Rays over a 64-degree full cone pointing straight down (-Y).
        let half = 32.0f32.to_radians();
        let (mut blocked, mut total) = (0, 0);
        let mut per_part = vec![0usize; by_part.len()];
        for i in 0..40 {
            for j in 0..40 {
                let u = (i as f32 + 0.5) / 40.0;
                let v = (j as f32 + 0.5) / 40.0;
                let theta = half * u.sqrt();
                let phi = v * std::f32::consts::TAU;
                let d = [
                    theta.sin() * phi.cos(),
                    -theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                total += 1;
                if tris.iter().any(|t| hits(origin, d, t)) {
                    blocked += 1;
                }
                for (pi, part) in by_part.iter().enumerate() {
                    if part.iter().any(|t| hits(origin, d, t)) {
                        per_part[pi] += 1;
                    }
                }
            }
        }
        eprintln!(
            "FIXTURE SELF-OCCLUSION: {blocked}/{total} downward rays blocked ({:.0}%)",
            100.0 * blocked as f32 / total as f32,
        );
        for (pi, n) in per_part.iter().enumerate() {
            eprintln!(
                "   part {pi} (node {}, prim {}) alone blocks {n}/{total} ({:.0}%)",
                parts[pi].node,
                parts[pi].primitive,
                100.0 * *n as f32 / total as f32,
            );
        }
        // Primitive 1 is the glass envelope; primitive 0 is the housing.
        let glass = per_part
            .iter()
            .position(|n| *n * 2 > total)
            .expect("no part blocks most of the bulb, so this asset no longer motivates the fix");
        assert_eq!(
            parts[glass].primitive, 1,
            "the part sealing the bulb is not the glass envelope any more",
        );
        let housing: usize = per_part
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != glass)
            .map(|(_, n)| *n)
            .sum();
        assert!(
            housing * 4 < total,
            "the OPAQUE housing blocks {housing}/{total} on its own; if the fixture \
             genuinely blocks its own bulb, exempting the glass will not fix the room",
        );
    }

    /// Moller-Trumbore, forward hits only.
    fn hits(o: [f32; 3], d: [f32; 3], t: &[[f32; 3]; 3]) -> bool {
        let e1 = sub(t[1], t[0]);
        let e2 = sub(t[2], t[0]);
        let p = cross(d, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-9 {
            return false;
        }
        let inv = 1.0 / det;
        let tv = sub(o, t[0]);
        let u = dot(tv, p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross(tv, e1);
        let v = dot(d, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        // 1cm bias so the ray does not hit geometry it starts on.
        dot(e2, q) * inv > 0.01
    }

}

#[cfg(test)]
mod light_blocking_tests {
    use super::*;

    fn model(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/models/lights").join(rel)
    }

    /// The hanging lamp's glass dome lets its light through; its shade and
    /// cap block it. The sconce's bulb is its light; its cage and back plate
    /// block it.
    #[test]
    fn a_fixture_blocks_with_its_housing_not_its_glass_or_bulb() {
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let sconce = model("industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let (Some(lamp_tris), Some(sconce_tris)) = (light_blocking_triangles(&lamp), light_blocking_triangles(&sconce)) else {
            eprintln!("skipping: fixture models not present");
            return;
        };
        // Exactly the housing: the lamp's 8812 (its cage wraps the glass, so
        // the glass cannot be told apart by position) without its 718-triangle
        // glass, and without the 318 its emissive texture paints its bulb on;
        // the sconce's 7130 without its 2696-triangle bulb.
        assert_eq!(lamp_tris.len(), 8494, "the lamp's glass or bulb was kept, or its housing dropped");
        assert_eq!(sconce_tris.len(), 7130, "the sconce's bulb was kept, or its housing dropped");
    }

    /// The sconce models its bulb on its own, so its whole housing shades the
    /// lamp; the hanging lamp paints its filament into the housing's texture,
    /// so the glass round its lamp is still in the housing.
    #[test]
    fn a_fixture_says_whether_its_bulb_is_its_own_part() {
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let sconce = model("industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let (Some(lamp), Some(sconce)) = (fixture_light_geometry(&lamp), fixture_light_geometry(&sconce)) else {
            eprintln!("skipping: fixture models not present");
            return;
        };
        assert!(sconce.bulb_dropped, "the sconce's bulb primitive was not recognised");
        assert!(lamp.bulb_dropped, "the hanging lamp's painted bulb was kept");
    }

    /// The hanging lamp's bulb is PAINTED into its housing's emissive
    /// texture, and its triangles go with it: from the bulb's middle, every
    /// level ray runs past the bulb's 4.5 cm radius before it meets housing.
    /// With the bulb kept, each stopped on the bulb's own glass -- and the 12
    /// cm clearance that had to excuse that let the outside of the bell's
    /// crown see the lamp through the metal (2026-10-01).
    #[test]
    fn a_painted_bulb_leaves_nothing_round_its_lamp() {
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let Some(lamp) = fixture_light_geometry(&lamp) else {
            eprintln!("skipping: fixture models not present");
            return;
        };
        // The middle of its emissive faces (Blender, 2026-10-01).
        let bulb = glam::Vec3::new(0.0, -1.0647, 0.0);
        let hit = |dir: glam::Vec3| {
            lamp.blocking
                .iter()
                .filter_map(|t| {
                    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
                    let p = dir.cross(e2);
                    let det = e1.dot(p);
                    if det.abs() < 1e-12 {
                        return None;
                    }
                    let s = bulb - t[0];
                    let u = s.dot(p) / det;
                    let q = s.cross(e1);
                    let v = dir.dot(q) / det;
                    let d = e2.dot(q) / det;
                    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && d > 0.0).then_some(d)
                })
                .fold(f32::INFINITY, f32::min)
        };
        for i in 0..16 {
            let a = i as f32 * std::f32::consts::TAU / 16.0;
            let d = hit(glam::Vec3::new(a.cos(), 0.0, a.sin()));
            assert!(d > 0.047, "{i}/16 round: housing {d:.3} m from the bulb's middle -- the bulb's glass was kept");
        }
    }

    /// The fixtures' mean colours are read from their textures, not their
    /// factors alone, and the bulb is not in them: a black iron sconce comes
    /// out dark, where a white bulb counted in would have lifted it.
    #[test]
    fn a_fixture_has_the_mean_colour_of_its_housing() {
        let sconce = model("industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let (Some(s), Some(l)) = (model_albedo(&sconce), model_albedo(&lamp)) else {
            eprintln!("skipping: fixture models not present");
            return;
        };
        eprintln!("sconce albedo {s:?}, hanging lamp albedo {l:?}");
        for a in [s, l] {
            assert!(a.min_element() > 0.0 && a.max_element() < 1.0, "not a surface colour: {a:?}");
        }
        // A texture read as all-white (the decode failing into the factor)
        // would make both exactly their factors, which glTF writes as 1.
        assert!(s.max_element() < 0.9 && l.max_element() < 0.9, "the textures were not read: {s:?} {l:?}");
    }

    /// A REFLECTION SEES THE BULB: the sconce's parts for its reflection cards
    /// are its housing and its bulb -- the bulb glowing its own warm white at a
    /// drive of one, the housing not at all -- and the hanging lamp's glass is
    /// left out (it shows what lies behind it). Left off the cards, the bulb
    /// reflected as a dark hole in the glowing mouth.
    #[test]
    fn a_fixtures_parts_for_reflections_include_its_glowing_bulb() {
        let sconce = model("industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let (Some(s), Some(l)) = (model_albedo_parts(&sconce), model_albedo_parts(&lamp)) else {
            eprintln!("skipping: fixture models not present");
            return;
        };
        let tris = |parts: &[AlbedoPart]| parts.iter().map(|p| p.triangles.len()).sum::<usize>();
        assert_eq!(tris(&s), 7130 + 2696, "the sconce's housing and bulb");
        let glowing: Vec<&AlbedoPart> = s.iter().filter(|p| p.emissive != glam::Vec3::ZERO).collect();
        assert_eq!(glowing.len(), 1, "one glowing part, the bulb");
        assert_eq!(glowing[0].triangles.len(), 2696);
        assert!((glowing[0].emissive - glam::Vec3::new(1.0, 0.86, 0.62)).length() < 1e-5, "{:?}", glowing[0].emissive);
        assert!(s.iter().all(|p| p.glow_map.is_none()), "the sconce's glow is whole, not placed by a texture");
        assert_eq!(tris(&l), 8812, "the hanging lamp without its glass");
        // THE HANGING LAMP'S GLOW IS PLACED BY ITS TEXTURE: one material for
        // housing and bulb, its emissive texture black but for the bulb. The
        // map has a coordinate triple per triangle, and read through them it
        // lights some triangles -- the bulb's -- and leaves most dark.
        let map = l.iter().find_map(|p| p.glow_map.as_ref().map(|m| (m, p.triangles.len()))).expect("a glow map");
        let (map, triangles) = map;
        assert_eq!(map.uvs.len(), triangles, "a coordinate triple per triangle");
        assert_eq!((map.width, map.height), (1024, 1024));
        let lit = (0..triangles).filter(|&t| map.at(t, 1.0 / 3.0, 1.0 / 3.0).max_element() > 0.5).count();
        eprintln!("hanging lamp: {lit} of {triangles} triangles glow at their centres");
        assert!(lit > 50 && lit < triangles / 4, "the bulb's triangles only: {lit} of {triangles}");
    }

    /// A glow map reads between its texels and repeats past its edges, as the
    /// renderer's sampler does.
    #[test]
    fn a_glow_map_reads_bilinearly_and_repeats() {
        let map = GlowMap {
            width: 2,
            height: 1,
            texels: vec![glam::Vec3::ZERO, glam::Vec3::ONE],
            uvs: vec![[[0.5, 0.5], [0.5, 0.5], [0.5, 0.5]], [[0.25, 0.5], [0.25, 0.5], [0.25, 0.5]], [[1.25, 0.5]; 3]],
        };
        // u 0.5 is half way between the two texel centres (0.25 and 0.75).
        assert!((map.at(0, 0.2, 0.3).x - 0.5).abs() < 1e-6);
        assert!(map.at(1, 0.0, 0.0).x.abs() < 1e-6, "on the dark texel's centre");
        assert!((map.at(2, 0.0, 0.0).x - map.at(1, 0.0, 0.0).x).abs() < 1e-6, "one texture width on, the same");
        assert_eq!(map.at(9, 0.0, 0.0), glam::Vec3::ZERO, "no such triangle");
    }

    /// A CHARACTER has a colour too, for its reflection: the skinned avatar,
    /// which has no static geometry to weigh by area, averages its materials.
    #[test]
    fn a_skinned_character_has_a_mean_colour() {
        let boy = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game/models/boy/boy.glb");
        if !boy.exists() {
            eprintln!("skipping: boy.glb not present");
            return;
        }
        let a = model_albedo(&boy).expect("the avatar's materials");
        eprintln!("avatar albedo {a:?}");
        assert!(a.min_element() > 0.0 && a.max_element() < 1.0, "not a surface colour: {a:?}");
    }
}

/// DIAGNOSTIC, ignored: `MASK_OBJ_OUT=<dir>` writes the wall sconce with its
/// lightmap layout as OBJ texture coordinates, to look at a baked atlas on the
/// model it belongs to.
#[cfg(test)]
mod layout_on_model {
    use super::*;

    #[test]
    #[ignore]
    fn write_the_sconce_with_its_atlas_coordinates() {
        let Ok(dir) = std::env::var("MASK_OBJ_OUT") else { return };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../game/models/lights/industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let parts = load_mesh_parts(&path).expect("the sconce loads");
        let layout = mesh_lightmap_layout(&parts, DEFAULT_TEXEL).expect("a layout");
        let mut obj = String::new();
        let mut n = 1usize;
        for chart in &layout.charts {
            for k in 0..3 {
                let c = chart.corners[k];
                let p = chart.texel_local(c[0], c[1]);
                let uv = chart.uv2(k, layout.width, layout.height);
                obj.push_str(&format!("v {} {} {}\nvt {} {}\n", p[0], p[1], p[2], uv[0], 1.0 - uv[1]));
            }
            obj.push_str(&format!("f {0}/{0} {1}/{1} {2}/{2}\n", n, n + 1, n + 2));
            n += 3;
        }
        std::fs::write(format!("{dir}/sconce_atlas.obj"), obj).unwrap();
        let tiny = layout.charts.iter().filter(|c| c.w * c.h <= 4).count();
        // The bake's own test (`MESH_EDGE_SLACK` in tools/bake): a texel is
        // baked when its centre is within a fifth of a texel of the triangle.
        // A chart with none is a sliver the baker shades by moving its texels
        // onto the triangle (`chart_texels`); the sconce's six are its plate's
        // chamfers.
        let unbaked = layout
            .charts
            .iter()
            .filter(|c| !(0..c.h).any(|ty| (0..c.w).any(|tx| c.barycentric(tx, ty).iter().all(|w| *w >= -0.2))))
            .count();
        eprintln!("LAYOUT charts with no texel on their triangle: {unbaked}");
        for c in layout.charts.iter().filter(|c| !(0..c.h).any(|ty| (0..c.w).any(|tx| c.barycentric(tx, ty).iter().all(|w| *w >= -0.2)))) {
            let p = [0, 1, 2].map(|k| c.texel_local(c.corners[k][0], c.corners[k][1]));
            eprintln!("UNBAKED {}x{} at ({}, {}): {:?}", c.w, c.h, c.x, c.y, p);
        }
        eprintln!("LAYOUT {}x{} atlas, {} charts, {} of them 2x2 or less, density x{}", layout.width, layout.height, layout.charts.len(), tiny, layout.density_scale);
    }
}
