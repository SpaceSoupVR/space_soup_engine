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
    let entity_path = PathBuf::from(&args[1]);

    let mut opts = ObjExportOptions {
        keep_internal: args.iter().any(|a| a == "--keep-internal"),
        ..Default::default()
    };
    if let Some(i) = args.iter().position(|a| a == "--textures") {
        if let Some(path) = args.get(i + 1) {
            opts.texture_root = path.clone();
        }
    }

    let stem = entity_path
        .file_stem().map_or_else(|| "brushes".to_string(), |s| s.to_string_lossy().to_string());
    opts.material_library = format!("{stem}.mtl");
    let mtl_path = entity_path.with_extension("mtl");

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

    if let Err(e) = write(&entity_path, &export.obj).and_then(|()| write(&mtl_path, &export.mtl)) {
        eprintln!("could not write {}: {e}", entity_path.display());
        std::process::exit(1);
    }

    let s = &export.stats;
    println!(
        "{{\"obj\":{:?},\"mtl\":{:?},\"objects\":{},\"faces\":{},\"quads\":{},\
         \"ngons\":{},\"triangles\":{},\"vertices\":{},\"internalDropped\":{},\
         \"materials\":{:?}}}",
        entity_path.display().to_string(),
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
