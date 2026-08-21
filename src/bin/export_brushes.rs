//! Export a scene's brush geometry as OBJ + MTL, for handing to an artist.
//!
//! ```text
//! export_brushes <scene.json> <out.obj> [--keep-internal] [--textures <path>]
//! ```
//!
//! Writes `<out.obj>` and a sibling `.mtl`, and prints a one-line JSON summary
//! on stdout so the editor's server can report what happened without parsing
//! the file back.
//!
//! A separate binary rather than an editor-only feature because a handoff is a
//! pipeline step: it wants to be runnable from a script, from CI, and from a
//! Makefile, not only from a browser with the level open.

use space_soup_engine::brush_obj::{export_scene, ObjExportOptions};
use space_soup_engine::scene::Scene;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "usage: export_brushes <scene.json> <out.obj> [--keep-internal] \
             [--textures <relative-path>]\n\n\
             Exports every object with brush geometry, preserving quads and\n\
             n-gons. Import into Blender with the DEFAULT axes (Forward -Z,\n\
             Up Y) -- the file is written Y-up and unrotated.\n\n\
             --keep-internal  keep the partitions between convex pieces, which\n\
             are dropped by default because no player can see them and an\n\
             artist would delete every one by hand."
        );
        std::process::exit(2);
    }

    let scene_path = PathBuf::from(&args[0]);
    let obj_path = PathBuf::from(&args[1]);

    let mut opts = ObjExportOptions {
        keep_internal: args.iter().any(|a| a == "--keep-internal"),
        ..Default::default()
    };
    if let Some(i) = args.iter().position(|a| a == "--textures") {
        if let Some(path) = args.get(i + 1) {
            opts.texture_root = path.clone();
        }
    }

    // The MTL must sit beside the OBJ and be named after it, or Blender resolves
    // `mtllib` against the OBJ's directory and silently imports with no
    // materials at all -- geometry arrives, textures do not, and nothing says
    // why.
    let stem = obj_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "brushes".to_string());
    opts.material_library = format!("{stem}.mtl");
    let mtl_path = obj_path.with_extension("mtl");

    let scene = match Scene::load(&scene_path) {
        Ok(scene) => scene,
        Err(e) => {
            eprintln!("could not load {}: {e:#}", scene_path.display());
            std::process::exit(1);
        }
    };

    let export = export_scene(&scene, &opts);
    if export.stats.objects == 0 {
        eprintln!(
            "{} has no brush geometry -- nothing to export. Build some in the \
             editor's Geometry panel first.",
            scene_path.display()
        );
        std::process::exit(1);
    }

    if let Err(e) = write(&obj_path, &export.obj).and_then(|_| write(&mtl_path, &export.mtl)) {
        eprintln!("could not write {}: {e}", obj_path.display());
        std::process::exit(1);
    }

    let s = &export.stats;
    println!(
        "{{\"obj\":{:?},\"mtl\":{:?},\"objects\":{},\"faces\":{},\"quads\":{},\
         \"ngons\":{},\"triangles\":{},\"vertices\":{},\"internalDropped\":{},\
         \"materials\":{:?}}}",
        obj_path.display().to_string(),
        mtl_path.display().to_string(),
        s.objects,
        s.faces,
        s.quads,
        s.ngons,
        s.triangles,
        s.vertices,
        s.internal_dropped,
        s.materials,
    );
}

fn write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}
