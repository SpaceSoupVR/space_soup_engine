use crate::brush::{
    solid_polygons, BrushDef, BrushFace, BrushSolid, Plane, Vec3,
};
use crate::scene::Scene;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const COINCIDENT_EPS: f64 = 1e-4;

#[derive(Debug, Clone)]
pub struct ObjExportOptions {

    pub keep_internal: bool,

    pub material_library: String,

    pub texture_root: String,
}

impl Default for ObjExportOptions {
    fn default() -> Self {
        ObjExportOptions {
            keep_internal: false,
            material_library: "brushes.mtl".to_string(),
            texture_root: "../materials".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjExportStats {
    pub objects: usize,
    pub vertices: usize,
    pub faces: usize,

    pub ngons: usize,
    pub quads: usize,
    pub triangles: usize,
    pub internal_dropped: usize,
    pub materials: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ObjExport {
    pub obj: String,
    pub mtl: String,
    pub stats: ObjExportStats,
}

struct Polygon<'a> {
    points: Vec<Vec3>,
    face: &'a BrushFace,
    plane: Plane,
    min: Vec3,
    max: Vec3,
}

fn bounds_of(points: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = points[0];
    let mut max = points[0];
    for p in points {
        for i in 0..3 {
            if p[i] < min[i] {
                min[i] = p[i];
            }
            if p[i] > max[i] {
                max[i] = p[i];
            }
        }
    }
    (min, max)
}

fn overlaps(a: &Polygon, b: &Polygon) -> bool {
    for i in 0..3 {
        if a.min[i] > b.max[i] + COINCIDENT_EPS || a.max[i] < b.min[i] - COINCIDENT_EPS {
            return false;
        }
    }
    true
}

fn back_to_back(a: &Polygon, b: &Polygon) -> bool {
    let n = a.plane.n;
    let m = b.plane.n;
    let opposed = (n[0] + m[0]).abs() < COINCIDENT_EPS
        && (n[1] + m[1]).abs() < COINCIDENT_EPS
        && (n[2] + m[2]).abs() < COINCIDENT_EPS;
    opposed && (a.plane.d + b.plane.d).abs() < COINCIDENT_EPS && overlaps(a, b)
}

fn polygons_of(solids: &[BrushSolid]) -> Vec<Polygon<'_>> {
    let mut out = Vec::new();
    for solid in solids {
        for (i, poly) in solid_polygons(solid).into_iter().enumerate() {
            let Some(points) = poly else { continue };
            let face = &solid.faces[i];
            let (min, max) = bounds_of(&points);
            out.push(Polygon {
                points,
                face,
                plane: face.plane(),
                min,
                max,
            });
        }
    }
    out
}

fn internal_flags(polys: &[Polygon]) -> Vec<bool> {
    let mut flags = vec![false; polys.len()];
    for i in 0..polys.len() {
        for j in (i + 1)..polys.len() {
            if back_to_back(&polys[i], &polys[j]) {
                flags[i] = true;
                flags[j] = true;
            }
        }
    }
    flags
}

fn mtl_name(material: &str) -> String {
    let cleaned: String = material
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

fn write_object(
    obj: &mut String,
    name: &str,
    brush: &BrushDef,
    opts: &ObjExportOptions,
    vertex_base: &mut usize,
    uv_base: &mut usize,
    stats: &mut ObjExportStats,
) {
    let pieces = brush.evaluate();
    let polys = polygons_of(&pieces);
    if polys.is_empty() {
        return;
    }
    let internal = if opts.keep_internal {
        vec![false; polys.len()]
    } else {
        internal_flags(&polys)
    };

    let kept: Vec<&Polygon> = polys
        .iter()
        .zip(internal.iter())
        .filter(|(_, hidden)| !**hidden)
        .map(|(p, _)| p)
        .collect();
    stats.internal_dropped += polys.len() - kept.len();
    if kept.is_empty() {
        return;
    }

    let _ = writeln!(obj, "\no {name}");
    stats.objects += 1;

    for poly in &kept {
        for p in &poly.points {
            let _ = writeln!(obj, "v {:.6} {:.6} {:.6}", p[0], p[1], p[2]);
        }
    }
    for poly in &kept {
        for p in &poly.points {
            let uv = poly.face.uv(*p);

            let _ = writeln!(obj, "vt {:.6} {:.6}", uv[0], -uv[1]);
        }
    }

    let mut by_material: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, poly) in kept.iter().enumerate() {
        by_material
            .entry(poly.face.material.clone())
            .or_default()
            .push(i);
    }

    let mut offsets = Vec::with_capacity(kept.len());
    let mut running = 0usize;
    for poly in &kept {
        offsets.push(running);
        running += poly.points.len();
    }

    for (material, indices) in &by_material {
        let name = mtl_name(material);
        if !stats.materials.contains(material) {
            stats.materials.push(material.clone());
        }
        let _ = writeln!(obj, "usemtl {name}");
        for &i in indices {
            let poly = kept[i];
            let n = poly.points.len();

            let mut line = String::from("f");
            for k in 0..n {
                let v = *vertex_base + offsets[i] + k + 1;
                let t = *uv_base + offsets[i] + k + 1;
                let _ = write!(line, " {v}/{t}");
            }
            let _ = writeln!(obj, "{line}");
            stats.faces += 1;
            match n {
                3 => stats.triangles += 1,
                4 => {
                    stats.quads += 1;
                    stats.ngons += 1;
                }
                _ => stats.ngons += 1,
            }
        }
    }

    *vertex_base += running;
    *uv_base += running;
    stats.vertices += running;
}

pub fn export_scene(scene: &Scene, opts: &ObjExportOptions) -> ObjExport {
    let mut obj = String::new();
    let mut stats = ObjExportStats::default();

    obj.push_str("# Space Soup brush geometry\n");
    obj.push_str("# Quads and n-gons are preserved -- see brush_obj.rs.\n");
    obj.push_str("# Import into Blender with the default axes (Forward -Z, Up Y).\n");
    let _ = writeln!(obj, "mtllib {}", opts.material_library);

    let mut vertex_base = 0usize;
    let mut uv_base = 0usize;
    for object in &scene.objects {
        let Some(brush) = object.brush.as_ref() else { continue };
        write_object(
            &mut obj,
            &object.id,
            brush,
            opts,
            &mut vertex_base,
            &mut uv_base,
            &mut stats,
        );
    }

    let mtl = write_mtl(&stats.materials, opts);
    ObjExport { obj, mtl, stats }
}

pub fn export_brush(name: &str, brush: &BrushDef, opts: &ObjExportOptions) -> ObjExport {
    let mut obj = String::new();
    let mut stats = ObjExportStats::default();
    let _ = writeln!(obj, "mtllib {}", opts.material_library);
    let mut vb = 0usize;
    let mut ub = 0usize;
    write_object(&mut obj, name, brush, opts, &mut vb, &mut ub, &mut stats);
    let mtl = write_mtl(&stats.materials, opts);
    ObjExport { obj, mtl, stats }
}

fn write_mtl(materials: &[String], opts: &ObjExportOptions) -> String {
    let mut mtl = String::from("# Space Soup brush materials\n");
    for material in materials {
        let name = mtl_name(material);
        let _ = writeln!(mtl, "\nnewmtl {name}");

        let _ = writeln!(mtl, "Kd 1.000 1.000 1.000");
        let _ = writeln!(mtl, "Ks 0.000 0.000 0.000");
        let _ = writeln!(mtl, "d 1.0");
        let _ = writeln!(mtl, "illum 2");
        if material != "default" {
            let root = opts.texture_root.trim_end_matches('/');
            let _ = writeln!(mtl, "map_Kd {root}/{material}/color.jpg");

            let _ = writeln!(mtl, "map_Bump -bm 1.0 {root}/{material}/normal.jpg");
        }
    }
    mtl
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::block_solid;

    trait SolidFaces {
        fn solids_faces(self) -> Vec<BrushFace>;
    }
    impl SolidFaces for BrushSolid {
        fn solids_faces(self) -> Vec<BrushFace> {
            self.faces
        }
    }

    fn wall() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([0.0, 0.0, 0.0], [4.0, 3.0, 0.3], "brick")],
            subtract: vec![],
        }
    }

    fn wall_with_door() -> BrushDef {
        BrushDef {
            solids: vec![block_solid([0.0, 0.0, 0.0], [4.0, 3.0, 0.3], "brick")],
            subtract: vec![block_solid([1.5, 0.0, -1.0], [2.5, 2.1, 1.0], "brick")],
        }
    }

    fn face_lines(obj: &str) -> Vec<&str> {
        obj.lines().filter(|l| l.starts_with("f ")).collect()
    }

    #[test]
    fn a_box_exports_as_six_quads_not_twelve_triangles() {
        let out = export_brush("wall", &wall(), &ObjExportOptions::default());
        let faces = face_lines(&out.obj);
        assert_eq!(faces.len(), 6, "{}", out.obj);
        for f in &faces {
            assert_eq!(f.split_whitespace().count() - 1, 4, "not a quad: {f}");
        }
        assert_eq!(out.stats.quads, 6);
        assert_eq!(out.stats.triangles, 0);
    }

    #[test]
    fn a_pillar_cap_exports_as_a_single_ngon() {

        let mut faces = block_solid([0.0, 0.0, 0.0], [2.0, 4.0, 2.0], "metal").solids_faces();
        for (nx, nz) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            let pl = Plane::new([nx, 0.0, nz], 0.0);
            let corner = [
                if nx > 0.0 { 2.0 } else { 0.0 },
                0.0,
                if nz > 0.0 { 2.0 } else { 0.0 },
            ];
            let d = pl.n[0] * corner[0] + pl.n[2] * corner[2] - 0.4;
            faces.push(BrushFace {
                plane: [pl.n[0], pl.n[1], pl.n[2], d],
                material: "metal".into(),
                u_axis: None,
                v_axis: None,
                scale: [2.0, 2.0],
                shift: [0.0, 0.0],
                rotation: 0.0,
                lightmap_scale: 0.25,
                smoothing: 0,
            });
        }
        let octagonal = BrushDef {
            solids: vec![BrushSolid { faces }],
            subtract: vec![],
        };
        let out = export_brush("pillar", &octagonal, &ObjExportOptions::default());
        let widest = face_lines(&out.obj)
            .iter()
            .map(|f| f.split_whitespace().count() - 1)
            .max()
            .unwrap();
        assert_eq!(widest, 8, "top cap should be one 8-gon:\n{}", out.obj);
        assert!(out.stats.ngons >= out.stats.quads, "n-gon count includes quads");
    }

    #[test]
    fn partitions_between_pieces_are_dropped() {
        let opts = ObjExportOptions::default();
        let out = export_brush("wall", &wall_with_door(), &opts);
        assert!(out.stats.internal_dropped > 0, "nothing was recognised as internal");

        let kept = ObjExportOptions { keep_internal: true, ..opts };
        let all = export_brush("wall", &wall_with_door(), &kept);
        assert!(
            face_lines(&all.obj).len() > face_lines(&out.obj).len(),
            "keep_internal should produce more faces"
        );
        assert_eq!(
            face_lines(&all.obj).len() - face_lines(&out.obj).len(),
            out.stats.internal_dropped
        );
    }

    #[test]
    fn a_solid_wall_has_no_internal_faces_to_drop() {
        let out = export_brush("wall", &wall(), &ObjExportOptions::default());
        assert_eq!(out.stats.internal_dropped, 0);
    }

    #[test]
    fn vertex_indices_continue_across_objects() {
        let mut scene = Scene::default();
        for id in ["wall_a", "wall_b"] {
            let mut obj = crate::scene::GameObject::default();
            obj.id = id.to_string();
            obj.brush = Some(wall());
            scene.objects.push(obj);
        }
        let out = export_scene(&scene, &ObjExportOptions::default());
        assert_eq!(out.stats.objects, 2);

        let max_index: usize = face_lines(&out.obj)
            .iter()
            .flat_map(|f| f.split_whitespace().skip(1))
            .map(|token| token.split('/').next().unwrap().parse::<usize>().unwrap())
            .max()
            .unwrap();
        let vertex_count = out.obj.lines().filter(|l| l.starts_with("v ")).count();
        assert_eq!(max_index, vertex_count, "index runs past the vertex list");
        assert_eq!(vertex_count, 48, "two boxes of six quads");
    }

    #[test]
    fn every_index_is_in_range_and_one_based() {
        let out = export_brush("wall", &wall_with_door(), &ObjExportOptions::default());
        let vs = out.obj.lines().filter(|l| l.starts_with("v ")).count();
        let vts = out.obj.lines().filter(|l| l.starts_with("vt ")).count();
        for f in face_lines(&out.obj) {
            for token in f.split_whitespace().skip(1) {
                let (v, t) = token.split_once('/').expect("v/vt pair");
                let v: usize = v.parse().unwrap();
                let t: usize = t.parse().unwrap();
                assert!(v >= 1 && v <= vs, "vertex {v} out of 1..={vs}");
                assert!(t >= 1 && t <= vts, "uv {t} out of 1..={vts}");
            }
        }
    }

    #[test]
    fn objects_without_a_brush_are_skipped() {
        let mut scene = Scene::default();
        let mut plain = crate::scene::GameObject::default();
        plain.id = "rifle".into();
        scene.objects.push(plain);
        let out = export_scene(&scene, &ObjExportOptions::default());
        assert_eq!(out.stats.objects, 0);
        assert!(face_lines(&out.obj).is_empty());
    }

    #[test]
    fn writes_a_material_library_pointing_at_the_texture_files() {
        let out = export_brush("wall", &wall(), &ObjExportOptions::default());
        assert!(out.obj.contains("mtllib brushes.mtl"));
        assert!(out.obj.contains("usemtl brick"));
        assert!(out.mtl.contains("newmtl brick"));
        assert!(out.mtl.contains("map_Kd ../materials/brick/color.jpg"));
        assert!(out.mtl.contains("map_Bump -bm 1.0 ../materials/brick/normal.jpg"));
    }

    #[test]
    fn the_untextured_default_material_gets_no_texture_lines() {
        let brush = BrushDef {
            solids: vec![block_solid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], "default")],
            subtract: vec![],
        };
        let out = export_brush("blockout", &brush, &ObjExportOptions::default());
        assert!(out.mtl.contains("newmtl default"));
        assert!(!out.mtl.contains("map_Kd"));
    }

    #[test]
    fn material_names_with_awkward_characters_are_made_safe() {
        assert_eq!(mtl_name("Bricks 097/A"), "Bricks_097_A");
        assert_eq!(mtl_name(""), "default");
    }

    #[test]
    fn geometry_is_written_y_up_unrotated() {

        let out = export_brush("wall", &wall(), &ObjExportOptions::default());
        let ys: Vec<f64> = out
            .obj
            .lines()
            .filter(|l| l.starts_with("v "))
            .map(|l| l.split_whitespace().nth(2).unwrap().parse().unwrap())
            .collect();

        assert!((ys.iter().cloned().fold(f64::MIN, f64::max) - 3.0).abs() < 1e-4);
    }

    #[test]
    fn uv_v_is_flipped_for_objs_bottom_up_convention() {
        let out = export_brush("wall", &wall(), &ObjExportOptions::default());
        let brush = wall();
        let solid = &brush.solids[0];
        let i = solid
            .faces
            .iter()
            .position(|f| f.plane().n[2] < -0.9)
            .unwrap();
        let poly = solid_polygons(solid)[i].clone().unwrap();
        let expected = -solid.faces[i].uv(poly[0])[1];
        let found = out
            .obj
            .lines()
            .filter(|l| l.starts_with("vt "))
            .any(|l| {
                let v: f64 = l.split_whitespace().nth(2).unwrap().parse().unwrap();
                (v - expected).abs() < 1e-5
            });
        assert!(found, "no vt row matches the negated projection V");
    }

    #[test]
    fn an_empty_brush_writes_no_object(){
        let out = export_brush("gone", &BrushDef::default(), &ObjExportOptions::default());
        assert_eq!(out.stats.objects, 0);
    }
}
