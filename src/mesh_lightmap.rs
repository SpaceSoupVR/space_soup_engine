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
                out.push(MeshPart {
                    node: node.index(),
                    primitive: pi,
                    positions,
                    normals,
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
