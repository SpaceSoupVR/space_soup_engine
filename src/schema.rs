
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
            comp!(slider_joint, "SliderJointDef", Optional),
            comp!(
                terrain_collider,
                "TerrainColliderDef",
                Optional,
                terrain_collider_fields()
            ),
            comp!(light, "LightDef", Optional),
            comp!(sound, "SoundSourceDef", Optional, sound_fields()),
            comp!(particle_emitter, "ParticleEmitterDef", Optional),
            comp!(laser, "LaserDef", Optional),
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
        slider_joint,
        terrain_collider,
        light,
        sound,
        particle_emitter,
        laser,
    } = o;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_the_nineteen_authorable_components() {
        assert_eq!(game_object_schema().components.len(), 19);
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

        assert!(by("grip_points").fields.is_empty());
        assert!(by("animations").fields.is_empty());
    }

    #[test]
    fn schema_serializes_and_round_trips_as_json() {
        let json = game_object_schema().to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["components"].as_array().unwrap().len(), 19);
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
        assert_eq!(
            checked_in.trim(),
            generated.trim(),
            "schema.json is stale — regenerate with `cargo run --bin emit_schema`"
        );
    }
}
