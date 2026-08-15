
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    Required,
    Optional,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Number,
    Bool,
    Text,
    Vec3,
    Enum,
    Color,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldDescriptor {
    pub name: &'static str,
    pub kind: FieldKind,
    pub optional: bool,
    #[serde(skip_serializing_if = "<[&str]>::is_empty")]
    pub variants: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComponentDescriptor {
    pub name: &'static str,
    pub rust_type: &'static str,
    pub cardinality: Cardinality,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDescriptor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaDescriptor {
    pub components: Vec<ComponentDescriptor>,
}

impl SchemaDescriptor {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("schema descriptor serializes")
    }
}

macro_rules! comp {
    ($name:ident, $ty:literal, $card:ident) => {
        comp!($name, $ty, $card, vec![])
    };
    ($name:ident, $ty:literal, $card:ident, $fields:expr) => {
        ComponentDescriptor {
            name: stringify!($name),
            rust_type: $ty,
            cardinality: Cardinality::$card,
            fields: $fields,
        }
    };
}

macro_rules! field {
    ($name:ident, $kind:ident) => {
        field!($name, $kind, false, &[])
    };
    ($name:ident, $kind:ident, opt) => {
        field!($name, $kind, true, &[])
    };
    ($name:ident, Enum, [$($v:literal),* $(,)?]) => {
        field!($name, Enum, false, &[$($v),*])
    };
    ($name:ident, $kind:ident, $opt:literal, $variants:expr) => {
        FieldDescriptor {
            name: stringify!($name),
            kind: FieldKind::$kind,
            optional: $opt,
            variants: $variants,
        }
    };
}

pub fn game_object_schema() -> SchemaDescriptor {
    SchemaDescriptor {
        components: vec![
            comp!(cuboid, "CuboidDef", Required),
            comp!(mesh, "MeshRef", Optional),
            comp!(is_trigger, "bool", Required),
            comp!(hidden, "bool", Required),
            comp!(script, "String", Optional),
            comp!(animations, "Animation", List),
            comp!(animation_bindings, "AnimationBinding", List),
            comp!(part_animations, "PartAnimationDef", List),
            comp!(rig_attachment, "RigAttachmentDef", Optional),
            comp!(grip_pose_left, "GripPoseDef", Optional),
            comp!(grip_pose_right, "GripPoseDef", Optional),
            comp!(rigid_body, "RigidBodyDef", Optional),
            comp!(grip_points, "GripPointDef", List),
            comp!(sockets, "SocketDef", List),
            comp!(slider_joint, "SliderJointDef", Optional),
            comp!(
                terrain_collider,
                "TerrainColliderDef",
                Optional,
                terrain_collider_fields()
            ),
            comp!(lights, "LightDef", List),
            comp!(sound, "SoundSourceDef", Optional, sound_fields()),
            comp!(
                particle_emitter,
                "ParticleEmitterDef",
                Optional,
                particle_emitter_fields()
            ),
            comp!(laser, "LaserDef", Optional, laser_fields()),
            comp!(spawn_point, "SpawnPointDef", Optional),
            comp!(teleportal, "TeleportalDef", Optional, teleportal_fields()),
        ],
    }
}

fn sound_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(s: crate::scene::SoundSourceDef) {
        let crate::scene::SoundSourceDef {
            clip,
            volume,
            pitch,
            min_distance,
            max_distance,
            looping,
            autoplay,
            directional,
            cone_angle_deg,
        } = s;
    }
    vec![
        field!(clip, Text),
        field!(volume, Number),
        field!(pitch, Number),
        field!(min_distance, Number),
        field!(max_distance, Number),
        field!(looping, Bool),
        field!(autoplay, Bool),
        field!(directional, Bool),
        field!(cone_angle_deg, Number),
    ]
}

fn terrain_collider_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(t: crate::scene::TerrainColliderDef) {
        let crate::scene::TerrainColliderDef { node_filter } = t;
    }
    vec![field!(node_filter, Text, opt)]
}

fn laser_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(l: crate::scene::LaserDef) {
        let crate::scene::LaserDef { color, max_distance, beam_width } = l;
    }
    vec![
        field!(color, Color),
        field!(max_distance, Number),
        field!(beam_width, Number),
    ]
}

fn particle_emitter_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(p: crate::scene::ParticleEmitterDef) {
        let crate::scene::ParticleEmitterDef {
            particle_size,
            spawn_rate,
            color,
            lifetime,
            speed,
            spread_deg,
            size_growth,
        } = p;
    }
    vec![
        field!(particle_size, Number),
        field!(spawn_rate, Number),
        field!(color, Color),
        field!(lifetime, Number),
        field!(speed, Number),
        field!(spread_deg, Number),
        field!(size_growth, Number),
    ]
}

fn teleportal_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(t: crate::scene::TeleportalDef) {
        let crate::scene::TeleportalDef { target_id, target_scene } = t;
    }
    vec![field!(target_id, Text, opt), field!(target_scene, Text, opt)]
}

#[allow(dead_code, unused_variables)]
fn schema_exhaustiveness(o: crate::scene::GameObject) {
    let crate::scene::GameObject {
        id,
        grip_pose_legacy,
        cuboid,
        mesh,
        is_trigger,
        hidden,
        script,
        animations,
        animation_bindings,
        part_animations,
        rig_attachment,
        grip_pose_left,
        grip_pose_right,
        rigid_body,
        grip_points,
        sockets,
        slider_joint,
        terrain_collider,
        lights,
        sound,
        particle_emitter,
        laser,
        spawn_point,
        teleportal,
        // Not an authorable component in its own right: it is per-part state on
        // the model, edited from the Model Editor's part list rather than added
        // to an object as a component. Listed here only to satisfy the
        // exhaustiveness check, which is what caught it being added at all.
        hidden_parts,
    } = o;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_the_twenty_two_authorable_components() {
        assert_eq!(game_object_schema().components.len(), 22);
    }

    #[test]
    fn schema_excludes_identity_and_deprecated_fields() {
        let names: Vec<&str> = game_object_schema()
            .components
            .iter()
            .map(|c| c.name)
            .collect();
        assert!(!names.contains(&"id"), "id is identity, not a component");
        assert!(
            !names.contains(&"grip_pose_legacy"),
            "grip_pose_legacy is deprecated"
        );
        for must in [
            "rigid_body",
            "grip_points",
            "slider_joint",
            "terrain_collider",
            "sound",
        ] {
            assert!(names.contains(&must), "schema missing '{must}'");
        }
    }

    #[test]
    fn layer0_leaf_components_expose_their_fields() {
        let schema = game_object_schema();
        let by = |n: &str| schema.components.iter().find(|c| c.name == n).unwrap();

        let sound = by("sound");
        assert_eq!(sound.fields.len(), 9, "SoundSourceDef has 9 fields");
        assert_eq!(sound.fields[0].name, "clip");
        assert_eq!(sound.fields[0].kind, FieldKind::Text);
        assert!(sound.fields.iter().any(|f| f.name == "looping" && f.kind == FieldKind::Bool));

        let terrain = by("terrain_collider");
        assert_eq!(terrain.fields.len(), 1);
        assert_eq!(terrain.fields[0].name, "node_filter");
        assert!(terrain.fields[0].optional, "node_filter is Option<String>");

        let laser = by("laser");
        assert_eq!(laser.fields.len(), 3, "LaserDef has 3 fields");
        assert!(laser.fields.iter().any(|f| f.name == "color" && f.kind == FieldKind::Color));
        assert!(laser.fields.iter().any(|f| f.name == "max_distance" && f.kind == FieldKind::Number));

        let particle_emitter = by("particle_emitter");
        assert_eq!(particle_emitter.fields.len(), 7, "ParticleEmitterDef has 7 fields");
        assert!(particle_emitter.fields.iter().any(|f| f.name == "color" && f.kind == FieldKind::Color));

        let teleportal = by("teleportal");
        assert_eq!(teleportal.fields.len(), 2, "TeleportalDef has 2 fields");
        assert!(teleportal.fields.iter().all(|f| f.optional), "both teleportal fields are Option<String>");

        assert!(by("grip_points").fields.is_empty());
        assert!(by("sockets").fields.is_empty());
        assert!(by("spawn_point").fields.is_empty(), "SpawnPointDef is a zero-field marker component");
        assert!(by("animations").fields.is_empty());
    }

    #[test]
    fn schema_serializes_and_round_trips_as_json() {
        let json = game_object_schema().to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["components"].as_array().unwrap().len(), 22);
        assert_eq!(v["components"][0]["name"], "cuboid");
        assert_eq!(v["components"][0]["cardinality"], "required");
    }

    #[test]
    fn checked_in_schema_json_is_up_to_date() {
        let generated = game_object_schema().to_json();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.json");
        let checked_in = std::fs::read_to_string(&path).expect(
            "schema.json missing — run `cargo run --bin emit_schema` in space_soup_engine",
        );
        // Compare line-ending-insensitively. to_json() emits LF, but a Windows
        // checkout can easily hold schema.json with CRLF, and .trim() only strips
        // the outside -- so an identical file failed here as "stale", sending the
        // developer to regenerate something that was already correct. Same family
        // as the CRLF that made run.sh unrunnable on macOS.
        let normalise = |s: &str| s.replace("\r\n", "\n").trim().to_string();
        assert_eq!(
            normalise(&checked_in),
            normalise(&generated),
            "schema.json is stale — regenerate with `cargo run --bin emit_schema`"
        );
    }
}
