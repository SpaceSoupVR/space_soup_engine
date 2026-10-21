pub const GUTTER: u32 = 1;

pub const MAX_ATLAS: u32 = 1024;

const MIN_CHART: u32 = 2;

pub const DEFAULT_TEXEL: f32 = 0.04;

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

#[derive(Debug, Clone, PartialEq)]
pub struct MeshChart {

    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,

    pub origin: [f32; 3],

    pub du: [f32; 3],
    pub dv: [f32; 3],

    pub normal: [f32; 3],

    pub corners: [[f32; 2]; 3],
}

impl MeshChart {

    pub fn texel_local(&self, tx: f32, ty: f32) -> [f32; 3] {
        add(self.origin, add(mul(self.du, tx), mul(self.dv, ty)))
    }

    pub fn uv2(&self, corner: usize, atlas_w: u32, atlas_h: u32) -> [f32; 2] {
        let c = self.corners[corner];

        [
            (self.x as f32 + c[0] + 0.5) / atlas_w as f32,
            (self.y as f32 + c[1] + 0.5) / atlas_h as f32,
        ]
    }

    pub fn barycentric(&self, tx: u32, ty: u32) -> [f32; 3] {
        let p = [tx as f32, ty as f32];
        let [a, b, c] = self.corners;
        let v0 = [b[0] - a[0], b[1] - a[1]];
        let v1 = [c[0] - a[0], c[1] - a[1]];
        let v2 = [p[0] - a[0], p[1] - a[1]];
        let den = v0[0] * v1[1] - v1[0] * v0[1];
        if den.abs() < 1e-12 {

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

    pub charts: Vec<MeshChart>,

    pub parts: Vec<PartRange>,

    pub density_scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartRange {

    pub node: usize,

    pub primitive: usize,

    pub first: usize,
    pub count: usize,
}

impl MeshLightmapLayout {

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

#[derive(Debug, Clone, Default)]
pub struct MeshPart {
    pub node: usize,
    pub primitive: usize,

    pub positions: Vec<[f32; 3]>,

    pub normals: Vec<[f32; 3]>,

    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// `TEXCOORD_1`; empty when the primitive has none.
    pub uv1: Vec<[f32; 2]>,
    /// The size of the atlas `uv1` was AUTHORED into, when the file says its
    /// second uv set is its lightmap's layout
    /// (`extras.space_soup.lightmap = { set: 1, size: [w, h] }` on the mesh:
    /// a baked cave). See [`authored_layout`].
    pub authored: Option<[u32; 2]>,
}

pub fn mesh_lightmap_layout(parts: &[MeshPart], texel: f32) -> Option<MeshLightmapLayout> {
    let mut order: Vec<&MeshPart> = parts.iter().filter(|p| p.indices.len() >= 3).collect();
    order.sort_by_key(|p| (p.node, p.primitive));
    if let Some(layout) = authored_layout(&order) {
        return Some(layout);
    }

    let total: usize = order.iter().map(|p| p.indices.len() / 3).sum();
    if total == 0 || total > MAX_TRIANGLES {
        return None;
    }

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges: Vec<PartRange> = Vec::new();
    for p in &order {
        let base = positions.len() as u32;
        let first = indices.len() / 3;
        positions.extend_from_slice(&p.positions);

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
/// THE LAYOUT A MESH BROUGHT WITH IT.
///
/// A baked cave's second uv set IS its lightmap's layout: box-projected charts
/// many triangles across, packed by the editor at a density of its choosing
/// (`scene_editor_web/frontend/src/lib/sdfBake.js`). Nothing is packed here and
/// the renderer splits nothing: a vertex has one uv2, as the file gives it.
///
/// The baker still walks CHARTS, one a triangle, so each triangle gets one:
/// its rectangle the texels its corners span, its corners in that rectangle's
/// texel coordinates, and the affine map from texel to mesh-local point that
/// its three corners fix. Unlike a packed chart's, these rectangles OVERLAP
/// their neighbours' -- the triangles of one authored chart share texels along
/// every edge -- so a consumer that writes a texel must let the triangle the
/// texel centre is INSIDE win over one that only reaches it by its slack.
///
/// A triangle no texel centre falls inside, or with no area in the atlas (the
/// lip a cave runs under the ground beside it carries its edge's uv), gets an
/// EMPTY rectangle: its neighbours' texels cover it, and the baker's fallback
/// for a chart with nothing on it -- shade every texel of the rectangle from
/// the nearest point of the triangle -- would overwrite theirs.
///
/// `None` unless every part is authored into one size of atlas. No triangle
/// budget: the budget bounds what a chart per triangle costs in atlas, and an
/// authored atlas costs what it says.
fn authored_layout(order: &[&MeshPart]) -> Option<MeshLightmapLayout> {
    let [width, height] = order.first()?.authored?;
    if width == 0 || height == 0 || order.iter().any(|p| p.authored != Some([width, height]) || p.uv1.len() != p.positions.len()) {
        return None;
    }
    let mut charts: Vec<MeshChart> = Vec::new();
    let mut ranges: Vec<PartRange> = Vec::new();
    for p in order {
        let first = charts.len();
        let tri_count = p.indices.len() / 3;
        for t in 0..tri_count {
            let idx = [0, 1, 2].map(|k| p.indices[t * 3 + k] as usize);
            let pos = idx.map(|i| p.positions.get(i).copied().unwrap_or([0.0; 3]));
            let at = idx.map(|i| {
                let uv = p.uv1.get(i).copied().unwrap_or([0.0; 2]);
                [uv[0] * width as f32 - 0.5, uv[1] * height as f32 - 0.5]
            });
            let lo = [0, 1].map(|k| at[0][k].min(at[1][k]).min(at[2][k]));
            let hi = [0, 1].map(|k| at[0][k].max(at[1][k]).max(at[2][k]));
            let x = (lo[0].floor().max(0.0) as u32).min(width - 1);
            let y = (lo[1].floor().max(0.0) as u32).min(height - 1);
            let x1 = (hi[0].ceil().max(0.0) as u32).min(width - 1).max(x);
            let y1 = (hi[1].ceil().max(0.0) as u32).min(height - 1).max(y);
            let corners = at.map(|c| [c[0] - x as f32, c[1] - y as f32]);
            let e1 = [corners[1][0] - corners[0][0], corners[1][1] - corners[0][1]];
            let e2 = [corners[2][0] - corners[0][0], corners[2][1] - corners[0][1]];
            let (big1, big2) = (sub(pos[1], pos[0]), sub(pos[2], pos[0]));
            let den = e1[0] * e2[1] - e2[0] * e1[1];
            let normal = norm(cross(big1, big2));
            let mut chart = MeshChart { x, y, w: 0, h: 0, origin: pos[0], du: [0.0; 3], dv: [0.0; 3], normal, corners };
            if den.abs() > 1e-9 {
                chart.du = mul(sub(mul(big1, e2[1]), mul(big2, e1[1])), 1.0 / den);
                chart.dv = mul(sub(mul(big2, e1[0]), mul(big1, e2[0])), 1.0 / den);
                chart.origin = sub(pos[0], add(mul(chart.du, corners[0][0]), mul(chart.dv, corners[0][1])));
                chart.w = x1 - x + 1;
                chart.h = y1 - y + 1;
                let inside = (0..chart.h).any(|ty| (0..chart.w).any(|tx| chart.barycentric(tx, ty).iter().all(|b| *b >= 0.0)));
                if !inside {
                    chart.w = 0;
                    chart.h = 0;
                }
            }
            charts.push(chart);
        }
        ranges.push(PartRange { node: p.node, primitive: p.primitive, first, count: tri_count });
    }
    if charts.is_empty() {
        return None;
    }
    Some(MeshLightmapLayout { width, height, charts, parts: ranges, density_scale: 1.0 })
}

struct Measured {
    u: [f32; 3],
    v: [f32; 3],
    normal: [f32; 3],
    min_u: f32,
    min_v: f32,

    plane_d: f32,

    w: f32,
    h: f32,

    corners_m: [[f32; 2]; 3],
}

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

    let mut density_scale = 1.0f32;
    for _ in 0..12 {
        if let Some(layout) = try_pack(&measured, texel, density_scale) {
            return Some(layout);
        }
        density_scale *= 0.5;
    }
    None
}

fn texels_across(metres: f32, per_metre: f32) -> u32 {
    let raw = metres * per_metre;
    ((raw - 1e-4).ceil().max(1.0)) as u32
}

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

        let step_u = span_per_texel(m.w, w);
        let step_v = span_per_texel(m.h, h);
        let du = mul(m.u, step_u);
        let dv = mul(m.v, step_v);

        let origin = add(
            add(mul(m.u, m.min_u), mul(m.v, m.min_v)),
            add(mul(du, 0.5), mul(dv, 0.5)),
        );

        let along = dot(origin, m.normal);
        let origin = add(origin, mul(m.normal, m.plane_d - along));

        let (cx, cy) = (cursor_x + GUTTER, shelf_y + GUTTER);

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

pub fn load_mesh_parts(path: &std::path::Path) -> Option<Vec<MeshPart>> {

    let gltf = gltf::Gltf::open(path).ok()?;
    let base = path.parent();
    let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob.clone()).ok()?;
    let mut parts = mesh_parts(&gltf.document, &buffers)?;
    let authored = authored_atlases(path);
    if !authored.is_empty() {
        let mesh_of_node: std::collections::HashMap<usize, usize> =
            gltf.document.nodes().filter_map(|n| Some((n.index(), n.mesh()?.index()))).collect();
        for part in &mut parts {
            part.authored = mesh_of_node.get(&part.node).and_then(|m| authored.get(m)).copied();
        }
    }
    Some(parts)
}

/// Mesh index to the atlas its `TEXCOORD_1` was authored into, for the meshes
/// of a file that say so. See [`MeshPart::authored`].
fn authored_atlases(path: &std::path::Path) -> std::collections::HashMap<usize, [u32; 2]> {
    let mut out = std::collections::HashMap::new();
    let Some(json) = gltf_json_bytes(path).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else {
        return out;
    };
    let Some(meshes) = json.get("meshes").and_then(|m| m.as_array()) else { return out };
    for (i, mesh) in meshes.iter().enumerate() {
        let Some(lm) = mesh.get("extras").and_then(|e| e.get("space_soup")).and_then(|s| s.get("lightmap")) else { continue };
        if lm.get("set").and_then(|s| s.as_u64()) != Some(1) {
            continue;
        }
        let size = lm.get("size").and_then(|s| s.as_array());
        let dim = |k: usize| size.and_then(|s| s.get(k)).and_then(|v| v.as_u64()).filter(|v| (1..=8192).contains(v));
        if let (Some(w), Some(h)) = (dim(0), dim(1)) {
            out.insert(i, [w as u32, h as u32]);
        }
    }
    out
}

pub fn light_blocking_triangles(path: &std::path::Path) -> Option<Vec<[glam::Vec3; 3]>> {
    fixture_light_geometry(path).map(|g| g.blocking)
}

/// A GROUND MODEL's light-blocking triangles -- a cave baked by the editor --
/// without its COPY OF THE GROUND: the triangles whose every corner the file
/// marks as untouched ground (`TEXCOORD_1.x` past one half; see the editor's
/// caveSky.groundCopy). That copy lies on the terrain's own surface under the
/// cells the cave opens, so as an occluder it only buried the terrain's bake
/// points beneath it: each opened cell baked black or shadowed, an angular
/// dark patch on the sand in front of every mouth. The terrain is its own
/// occluder there already. A model without the attribute keeps every
/// triangle, as [`light_blocking_triangles`] gives them.
///
/// And so does a mesh whose `TEXCOORD_1` is its LIGHTMAP ([`MeshPart::authored`],
/// the baked cave): it carries no copy of the ground -- the terrain draws the
/// ground round it -- and a uv past one half there is the right half of its
/// atlas, not a mark. Read as one, half the rock let the light through.
pub fn ground_model_blocking_triangles(path: &std::path::Path) -> Option<Vec<[glam::Vec3; 3]>> {
    let (doc, buffers, _) = gltf::import(path).ok()?;
    let authored = authored_atlases(path);
    let mut out = Vec::new();
    fn walk(
        node: gltf::Node,
        parent: [[f32; 4]; 4],
        buffers: &[gltf::buffer::Data],
        authored: &std::collections::HashMap<usize, [u32; 2]>,
        out: &mut Vec<[glam::Vec3; 3]>,
    ) {
        let world = mat_mul(parent, node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let marks_ground = !authored.contains_key(&mesh.index());
            for prim in mesh.primitives() {
                let reader = prim.reader(|b| Some(&buffers[b.index()]));
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<[f32; 3]> = positions.map(|p| transform_point(world, p)).collect();
                let ground: Vec<f32> = reader
                    .read_tex_coords(1)
                    .filter(|_| marks_ground)
                    .map(|t| t.into_f32().map(|uv| uv[0]).collect())
                    .filter(|g: &Vec<f32>| g.len() == positions.len())
                    .unwrap_or_else(|| vec![0.0; positions.len()]);
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                for t in indices.chunks_exact(3) {
                    let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
                    if a >= positions.len() || b >= positions.len() || c >= positions.len() {
                        continue;
                    }
                    if ground[a] > 0.5 && ground[b] > 0.5 && ground[c] > 0.5 {
                        continue;
                    }
                    out.push([positions[a].into(), positions[b].into(), positions[c].into()]);
                }
            }
        }
        for child in node.children() {
            walk(child, world, buffers, authored, out);
        }
    }
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    for scene in doc.scenes() {
        for node in scene.nodes() {
            walk(node, identity, &buffers, &authored, &mut out);
        }
    }
    Some(out)
}

pub struct FixtureLightGeometry {
    pub blocking: Vec<[glam::Vec3; 3]>,

    pub bulb_dropped: bool,
}

const PAINTED_BULB_SHARE: f32 = 0.1;

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

pub fn model_albedo(path: &std::path::Path) -> Option<glam::Vec3> {
    let gltf = gltf::Gltf::open(path).ok()?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone()).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&gltf_json_bytes(path)?).ok()?;
    let mut texture_means: std::collections::HashMap<usize, Option<glam::Vec3>> = std::collections::HashMap::new();

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

pub struct AlbedoPart {
    pub triangles: Vec<[glam::Vec3; 3]>,
    pub albedo: glam::Vec3,

    pub emissive: glam::Vec3,

    pub glow_map: Option<GlowMap>,
}

pub struct GlowMap {
    pub width: u32,
    pub height: u32,

    pub texels: Vec<glam::Vec3>,
    pub uvs: Vec<[[f32; 2]; 3]>,
}

impl GlowMap {

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

            let uv = |i: u32| part.uvs.get(i as usize).copied().unwrap_or([0.0, 0.0]);
            map.uvs.extend(part.indices.chunks_exact(3).map(|t| [uv(t[0]), uv(t[1]), uv(t[2])]));
        }
    }
    Some(by_material.into_iter().map(|(_, p)| p).filter(|p| !p.triangles.is_empty()).collect())
}

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

fn material_emissive_factor(json: &serde_json::Value, material: Option<usize>) -> glam::Vec3 {
    let Some(f) = material.and_then(|i| json["materials"][i]["emissiveFactor"].as_array()) else { return glam::Vec3::ZERO };
    let c = |k: usize| f.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    glam::Vec3::new(c(0), c(1), c(2))
}

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

fn gltf_json_bytes(path: &std::path::Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() >= 20 && &bytes[0..4] == b"glTF" {
        let len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        return bytes.get(20..20 + len).map(|b| b.to_vec());
    }
    Some(bytes)
}

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
                let uv1: Vec<[f32; 2]> = reader
                    .read_tex_coords(1)
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
                    uv1,
                    authored: None,
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

        break;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

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

pub fn lightmap_uv_for_asset(
    path: &std::path::Path,
) -> Option<Vec<((usize, usize), Vec<[f32; 2]>)>> {
    let parts = load_mesh_parts(path)?;
    if parts.iter().any(|p| p.authored.is_some()) {
        return None;
    }
    let layout = mesh_lightmap_layout(&parts, DEFAULT_TEXEL)?;
    Some(corner_uv2(&layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mesh() -> (Vec<[f32; 3]>, Vec<u32>) {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],

            [5.0, 2.0, -3.0],
            [5.7, 2.4, -3.0],
            [5.0, 2.9, -3.6],

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

    #[test]
    fn a_corner_uv2_lands_on_its_own_chart() {
        let (positions, indices) = sample_mesh();
        let layout = pack_triangles(&positions, &indices, DEFAULT_TEXEL).unwrap();
        for (t, chart) in layout.charts.iter().enumerate() {
            for corner in 0..3 {
                let uv = chart.uv2(corner, layout.width, layout.height);
                let px = uv[0] * layout.width as f32;
                let py = uv[1] * layout.height as f32;

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

    #[test]
    fn texels_off_the_triangle_read_as_negative() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let layout = pack_triangles(&positions, &[0, 1, 2], 0.1).unwrap();
        let c = &layout.charts[0];
        let inside = |tx, ty| c.barycentric(tx, ty).iter().all(|w| *w >= 0.0);

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

    #[test]
    fn a_dense_mesh_scales_its_density_down_to_fit() {

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

    #[test]
    fn a_nine_thousand_triangle_prop_gets_a_sane_atlas() {
        let mut positions = Vec::new();
        let mut indices = Vec::new();

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
mod authored_tests {
    use super::*;

    /// A 2 m square in the plane y = 3, two triangles, authored into the
    /// middle of a 16 x 16 atlas at four texels a metre; and a third triangle
    /// hanging under one edge with that edge's uv (a cave's lip).
    fn square() -> MeshPart {
        let positions = vec![[0.0, 3.0, 0.0], [2.0, 3.0, 0.0], [2.0, 3.0, 2.0], [0.0, 3.0, 2.0], [0.0, 2.9, 0.0], [2.0, 2.9, 0.0]];
        let uv = |x: f32, z: f32| [(4.0 + x * 4.0) / 16.0, (4.0 + z * 4.0) / 16.0];
        MeshPart {
            node: 0,
            primitive: 0,
            normals: vec![[0.0, 1.0, 0.0]; 6],
            uv1: vec![uv(0.0, 0.0), uv(2.0, 0.0), uv(2.0, 2.0), uv(0.0, 2.0), uv(0.0, 0.0), uv(2.0, 0.0)],
            positions,
            indices: vec![0, 2, 1, 0, 3, 2, 0, 1, 5],
            authored: Some([16, 16]),
            ..Default::default()
        }
    }

    #[test]
    fn an_authored_mesh_is_read_not_packed() {
        let part = square();
        let layout = mesh_lightmap_layout(&[part.clone()], DEFAULT_TEXEL).expect("a layout");
        assert_eq!((layout.width, layout.height), (16, 16), "the atlas is the file's");
        assert_eq!(layout.charts.len(), 3);
        for t in 0..3 {
            let chart = layout.chart(0, 0, t).unwrap();
            for k in 0..3 {
                let want = part.uv1[part.indices[t * 3 + k] as usize];
                let got = chart.uv2(k, 16, 16);
                assert!((got[0] - want[0]).abs() < 1e-5 && (got[1] - want[1]).abs() < 1e-5, "{got:?} {want:?}");
            }
        }
    }

    #[test]
    fn every_texel_inside_a_triangle_stands_for_its_own_point_of_the_surface() {
        let part = square();
        let layout = mesh_lightmap_layout(&[part], DEFAULT_TEXEL).unwrap();
        let mut covered = std::collections::HashSet::new();
        for chart in &layout.charts[..2] {
            assert!(chart.w > 0 && chart.h > 0);
            assert!((chart.normal[1].abs() - 1.0).abs() < 1e-5);
            for ty in 0..chart.h {
                for tx in 0..chart.w {
                    if chart.barycentric(tx, ty).iter().any(|b| *b < 0.0) {
                        continue;
                    }
                    let (px, py) = (chart.x + tx, chart.y + ty);
                    covered.insert((px, py));
                    let p = chart.texel_local(tx as f32, ty as f32);
                    let want = [(px as f32 + 0.5 - 4.0) / 4.0, 3.0, (py as f32 + 0.5 - 4.0) / 4.0];
                    for k in 0..3 {
                        assert!((p[k] - want[k]).abs() < 1e-4, "texel ({px}, {py}): {p:?} is not {want:?}");
                    }
                }
            }
        }
        for y in 4..12 {
            for x in 4..12 {
                assert!(covered.contains(&(x, y)), "texel ({x}, {y}) of the square is in no triangle");
            }
        }
        assert_eq!(covered.len(), 64, "and none outside it");
    }

    #[test]
    fn a_triangle_with_no_area_in_the_atlas_owns_no_texels() {
        let layout = mesh_lightmap_layout(&[square()], DEFAULT_TEXEL).unwrap();
        let lip = layout.chart(0, 0, 2).unwrap();
        assert_eq!((lip.w, lip.h), (0, 0), "the lip carries its edge's uv: nothing of the atlas is its own");
        let mut part = square();
        part.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.01]];
        part.uv1 = vec![[4.1 / 16.0, 4.6 / 16.0], [8.1 / 16.0, 4.6 / 16.0], [8.1 / 16.0, 4.64 / 16.0]];
        part.normals = vec![[0.0, 1.0, 0.0]; 3];
        part.indices = vec![0, 1, 2];
        let sliver = mesh_lightmap_layout(&[part], DEFAULT_TEXEL).unwrap();
        assert_eq!((sliver.charts[0].w, sliver.charts[0].h), (0, 0));
    }

    #[test]
    fn an_authored_mesh_has_no_triangle_budget_and_a_half_authored_one_is_packed() {
        let n = 160usize;
        let mut part = MeshPart { authored: Some([256, 256]), ..Default::default() };
        for j in 0..=n {
            for i in 0..=n {
                part.positions.push([i as f32 * 0.1, 0.0, j as f32 * 0.1]);
                part.normals.push([0.0, 1.0, 0.0]);
                part.uv1.push([(8.0 + i as f32 * 1.5) / 256.0, (8.0 + j as f32 * 1.5) / 256.0]);
            }
        }
        let at = |i: usize, j: usize| (j * (n + 1) + i) as u32;
        for j in 0..n {
            for i in 0..n {
                part.indices.extend([at(i, j), at(i + 1, j + 1), at(i + 1, j), at(i, j), at(i, j + 1), at(i + 1, j + 1)]);
            }
        }
        assert!(part.indices.len() / 3 > MAX_TRIANGLES);
        let layout = mesh_lightmap_layout(&[part.clone()], DEFAULT_TEXEL).expect("authored: no budget");
        assert_eq!(layout.charts.len(), part.indices.len() / 3);
        assert_eq!((layout.width, layout.height), (256, 256));
        let plain = MeshPart {
            node: 1,
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        assert!(mesh_lightmap_layout(&[part, plain], DEFAULT_TEXEL).is_none(), "packed by the ordinary rule, which the budget refuses");
    }

    /// A `.glb` as the editor's cave bake writes one, by hand: the file says
    /// its second uv set is its lightmap, and the loader hears it.
    #[test]
    fn a_file_that_says_its_uv1_is_its_lightmap_is_loaded_as_authored() {
        let part = square();
        let write = |extras: &str, name: &str, shift: f32| {
            let mut bin: Vec<u8> = Vec::new();
            for p in &part.positions {
                for c in p {
                    bin.extend(c.to_le_bytes());
                }
            }
            let uv_at = bin.len();
            for uv in &part.uv1 {
                for c in [uv[0] + shift, uv[1]] {
                    bin.extend(c.to_le_bytes());
                }
            }
            let idx_at = bin.len();
            for i in &part.indices {
                bin.extend(i.to_le_bytes());
            }
            let json = format!(
                r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_1":1}},"indices":2}}]{extras}}}],
"buffers":[{{"byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{uv_at}}},{{"buffer":0,"byteOffset":{uv_at},"byteLength":{uvl}}},{{"buffer":0,"byteOffset":{idx_at},"byteLength":{il}}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3","min":[0,2.9,0],"max":[2,3,2]}},
{{"bufferView":1,"componentType":5126,"count":6,"type":"VEC2"}},{{"bufferView":2,"componentType":5125,"count":9,"type":"SCALAR"}}]}}"#,
                len = bin.len(),
                uvl = idx_at - uv_at,
                il = bin.len() - idx_at,
            );
            let mut json = json.into_bytes();
            while json.len() % 4 != 0 {
                json.push(b' ');
            }
            let mut glb: Vec<u8> = Vec::new();
            glb.extend(b"glTF");
            glb.extend(2u32.to_le_bytes());
            glb.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
            glb.extend((json.len() as u32).to_le_bytes());
            glb.extend(b"JSON");
            glb.extend(&json);
            glb.extend((bin.len() as u32).to_le_bytes());
            glb.extend(b"BIN\0");
            glb.extend(&bin);
            let path = std::env::temp_dir().join(format!("ss_authored_{}_{name}.glb", std::process::id()));
            std::fs::write(&path, glb).unwrap();
            path
        };
        let said = r#","extras":{"space_soup":{"shading":"baked_cave","lightmap":{"set":1,"size":[16,16]}}}"#;
        let authored = write(said, "yes", 0.0);
        let parts = load_mesh_parts(&authored).expect("it loads");
        assert_eq!(parts[0].authored, Some([16, 16]));
        assert_eq!(parts[0].uv1.len(), 6);
        assert_eq!(mesh_lightmap_layout(&parts, DEFAULT_TEXEL).unwrap().width, 16);
        assert!(lightmap_uv_for_asset(&authored).is_none(), "the renderer reads the file's own TEXCOORD_1: nothing to hand it");
        let plain = write("", "no", 0.0);
        let parts = load_mesh_parts(&plain).expect("it loads");
        assert_eq!(parts[0].authored, None);
        assert!(lightmap_uv_for_asset(&plain).is_some());
        let right = write(said, "right", 0.26);
        assert_eq!(ground_model_blocking_triangles(&right).unwrap().len(), 3, "a lightmap's right half is rock");
        let marked = write("", "marked", 0.26);
        assert_eq!(ground_model_blocking_triangles(&marked).unwrap().len(), 0, "an older cave's ground copy casts nothing");
        let _ = std::fs::remove_file(right);
        let _ = std::fs::remove_file(marked);
        let _ = std::fs::remove_file(authored);
        let _ = std::fs::remove_file(plain);
    }
}

#[cfg(test)]
mod asset_tests {

    use super::*;
    use std::path::PathBuf;

    fn lamp() -> Option<PathBuf> {
        let p = crate::test_game_dir().join("models/lights/hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
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

    #[test]
    fn the_lamps_glass_would_seal_its_own_bulb_in_shadow() {
        let Some(path) = lamp() else { return };
        let parts = load_mesh_parts(&path).unwrap();

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

        dot(e2, q) * inv > 0.01
    }

}

#[cfg(test)]
mod light_blocking_tests {
    use super::*;

    fn model(rel: &str) -> std::path::PathBuf {
        crate::test_game_dir().join("models/lights").join(rel)
    }

    #[test]
    fn a_fixture_blocks_with_its_housing_not_its_glass_or_bulb() {
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let sconce = model("industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
        let (Some(lamp_tris), Some(sconce_tris)) = (light_blocking_triangles(&lamp), light_blocking_triangles(&sconce)) else {
            eprintln!("skipping: fixture models not present");
            return;
        };

        assert_eq!(lamp_tris.len(), 8494, "the lamp's glass or bulb was kept, or its housing dropped");
        assert_eq!(sconce_tris.len(), 7130, "the sconce's bulb was kept, or its housing dropped");
    }

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

    #[test]
    fn a_painted_bulb_leaves_nothing_round_its_lamp() {
        let lamp = model("hanging_industrial_lamp/hanging_industrial_lamp_1k.gltf");
        let Some(lamp) = fixture_light_geometry(&lamp) else {
            eprintln!("skipping: fixture models not present");
            return;
        };

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

        assert!(s.max_element() < 0.9 && l.max_element() < 0.9, "the textures were not read: {s:?} {l:?}");
    }

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

        let map = l.iter().find_map(|p| p.glow_map.as_ref().map(|m| (m, p.triangles.len()))).expect("a glow map");
        let (map, triangles) = map;
        assert_eq!(map.uvs.len(), triangles, "a coordinate triple per triangle");
        assert_eq!((map.width, map.height), (1024, 1024));
        let lit = (0..triangles).filter(|&t| map.at(t, 1.0 / 3.0, 1.0 / 3.0).max_element() > 0.5).count();
        eprintln!("hanging lamp: {lit} of {triangles} triangles glow at their centres");
        assert!(lit > 50 && lit < triangles / 4, "the bulb's triangles only: {lit} of {triangles}");
    }

    #[test]
    fn a_glow_map_reads_bilinearly_and_repeats() {
        let map = GlowMap {
            width: 2,
            height: 1,
            texels: vec![glam::Vec3::ZERO, glam::Vec3::ONE],
            uvs: vec![[[0.5, 0.5], [0.5, 0.5], [0.5, 0.5]], [[0.25, 0.5], [0.25, 0.5], [0.25, 0.5]], [[1.25, 0.5]; 3]],
        };

        assert!((map.at(0, 0.2, 0.3).x - 0.5).abs() < 1e-6);
        assert!(map.at(1, 0.0, 0.0).x.abs() < 1e-6, "on the dark texel's centre");
        assert!((map.at(2, 0.0, 0.0).x - map.at(1, 0.0, 0.0).x).abs() < 1e-6, "one texture width on, the same");
        assert_eq!(map.at(9, 0.0, 0.0), glam::Vec3::ZERO, "no such triangle");
    }

    #[test]
    fn a_skinned_character_has_a_mean_colour() {
        let boy = crate::test_game_dir().join("models/boy/boy.glb");
        if !boy.exists() {
            eprintln!("skipping: boy.glb not present");
            return;
        }
        let a = model_albedo(&boy).expect("the avatar's materials");
        eprintln!("avatar albedo {a:?}");
        assert!(a.min_element() > 0.0 && a.max_element() < 1.0, "not a surface colour: {a:?}");
    }
}

#[cfg(test)]
mod layout_on_model {
    use super::*;

    #[test]
    #[ignore]
    fn write_the_sconce_with_its_atlas_coordinates() {
        let Ok(dir) = std::env::var("MASK_OBJ_OUT") else { return };
        let path = crate::test_game_dir().join("models/lights/industrial_wall_sconce/industrial_wall_sconce_1k.gltf");
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
