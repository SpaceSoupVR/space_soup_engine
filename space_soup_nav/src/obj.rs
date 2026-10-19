//! Triangle meshes as Wavefront OBJ: the exchange format between this crate's tests, the golden
//! reference tool (which runs C++ Recast on the same files) and Recast's own demo meshes.
//!
//! Only what navmesh input needs: `v x y z` and `f a b c ...` (polygons fanned into triangles,
//! `a/b/c` indices and negative indices accepted). Metres, +Y up, as Recast expects.

use glam::Vec3;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl TriMesh {
    pub fn parse(text: &str) -> Result<TriMesh, String> {
        let mut mesh = TriMesh::default();
        for (line_no, line) in text.lines().enumerate() {
            let mut it = line.split_whitespace();
            match it.next() {
                Some("v") => {
                    let mut c = [0.0f32; 3];
                    for slot in &mut c {
                        *slot = it
                            .next()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| format!("line {}: bad vertex", line_no + 1))?;
                    }
                    mesh.vertices.push(Vec3::from(c));
                }
                Some("f") => {
                    let n = mesh.vertices.len() as i64;
                    let idx: Vec<u32> = it
                        .map(|tok| {
                            let first = tok.split('/').next().unwrap_or("");
                            let i: i64 = first.parse().map_err(|_| format!("line {}: bad face index {tok}", line_no + 1))?;
                            let i = if i < 0 { n + i } else { i - 1 };
                            if i < 0 || i >= n {
                                return Err(format!("line {}: face index {tok} out of range", line_no + 1));
                            }
                            Ok(i as u32)
                        })
                        .collect::<Result<_, String>>()?;
                    if idx.len() < 3 {
                        return Err(format!("line {}: face with {} vertices", line_no + 1, idx.len()));
                    }
                    for k in 1..idx.len() - 1 {
                        mesh.triangles.push([idx[0], idx[k], idx[k + 1]]);
                    }
                }
                _ => {}
            }
        }
        Ok(mesh)
    }

    pub fn load(path: &std::path::Path) -> Result<TriMesh, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        TriMesh::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// OBJ text. Coordinates printed with enough digits to round-trip an f32 exactly, so the C++
    /// tool and this crate read the same numbers.
    pub fn to_obj(&self, comment: &str) -> String {
        let mut s = String::new();
        for line in comment.lines() {
            s.push_str("# ");
            s.push_str(line);
            s.push('\n');
        }
        for v in &self.vertices {
            s.push_str(&format!("v {:?} {:?} {:?}\n", v.x, v.y, v.z));
        }
        for t in &self.triangles {
            s.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        s
    }

    /// Adds `other`'s triangles.
    pub fn append(&mut self, other: &TriMesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.triangles.extend(other.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }

    /// An axis-aligned box, every face outward. Floors and walls of test geometry are built from these.
    pub fn cuboid(min: Vec3, max: Vec3) -> TriMesh {
        let c = |i: u32| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        };
        let vertices = (0..8).map(c).collect();
        // Each face as two triangles wound so the normal points out (counter-clockwise from outside).
        let quads: [[u32; 4]; 6] = [
            [0, 4, 6, 2], // -X
            [1, 3, 7, 5], // +X
            [0, 1, 5, 4], // -Y
            [2, 6, 7, 3], // +Y
            [0, 2, 3, 1], // -Z
            [4, 5, 7, 6], // +Z
        ];
        let triangles = quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect();
        TriMesh { vertices, triangles }
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quads_slashes_and_negative_indices() {
        let m = TriMesh::parse("v 0 0 0\nv 1 0 0\nv 1 0 1\nv 0 0 1\nf 1/1/1 2//2 3 -1\n").unwrap();
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.triangles, vec![[0, 1, 2], [0, 2, 3]]);
        assert!(TriMesh::parse("v 0 0\n").is_err());
        assert!(TriMesh::parse("v 0 0 0\nf 1 2 3\n").is_err());
    }

    #[test]
    fn writing_then_reading_is_exact() {
        let mut m = TriMesh::cuboid(Vec3::new(-1.1, 0.0, 0.3333333), Vec3::new(2.0, 0.15, 7.1));
        m.append(&TriMesh::cuboid(Vec3::splat(10.0), Vec3::splat(11.0)));
        let back = TriMesh::parse(&m.to_obj("test\nmesh")).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn a_cuboids_top_faces_up() {
        let m = TriMesh::cuboid(Vec3::ZERO, Vec3::ONE);
        let up = m
            .triangles
            .iter()
            .filter(|t| {
                let (a, b, c) = (m.vertices[t[0] as usize], m.vertices[t[1] as usize], m.vertices[t[2] as usize]);
                (b - a).cross(c - a).normalize().y > 0.99
            })
            .count();
        assert_eq!(up, 2, "exactly the two top triangles face up, so Recast sees one walkable top");
    }
}
