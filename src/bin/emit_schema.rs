
use space_soup_engine::schema::game_object_schema;

fn main() {
    let schema = game_object_schema();
    let json = schema.to_json();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.json");
    std::fs::write(&out, format!("{json}\n")).expect("write schema.json");
    println!(
        "wrote {} ({} components)",
        out.display(),
        schema.components.len()
    );
}
