
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

    /// Every key a serialized `GameObject` can carry, in the order serde emits
    /// them.
    ///
    /// `components` deliberately omits `id`, `hidden_parts` and the deprecated
    /// `grip_pose`, so it is not a complete key list and cannot be used to
    /// order a file. Anything that WRITES scene JSON needs the complete order —
    /// notably the editor's Python backend, which is the only writer of
    /// `game/scenes/*.json` in practice and has no way to see serde's field
    /// order otherwise. Emitting it here keeps one source of truth instead of a
    /// hand-copied list that silently drifts the first time a field is added.
    pub field_order: Vec<&'static str>,
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

/// The complete serialized key order of `GameObject`, matching serde's
/// declaration-order output.
///
/// Kept honest by `field_order_matches_serde` below rather than by review: a
/// field added to `GameObject` and left out of here fails that test, the same
/// way the exhaustive destructure in `exhaustive` forces it to be classified.
pub fn game_object_field_order() -> Vec<&'static str> {
    vec![
        "id",
        // Identity and structure, not authorable components -- see the
        // exhaustive destructure below. Both are skip_serializing_if, so a
        // default object carries neither.
        "uuid",
        "parent",
        "cuboid",
        "mesh",
        // Declaration order, which is serde's output order. `brush` is
        // skip_serializing_if, so it is in `skipped` below too -- a field that
        // is absent from BOTH lists passes `field_order_matches_serde` without
        // being ordered anywhere, which is how this one nearly slipped through.
        "brush",
        "is_trigger",
        "hidden",
        "tags",
        "script",
        "animations",
        "animation_bindings",
        "part_animations",
        "hidden_parts",
        "rig_attachment",
        // `grip_pose_legacy` on the struct; renamed on the wire, and the only
        // field with skip_serializing_if, so it is absent from a default object.
        "grip_pose",
        "grip_pose_left",
        "grip_pose_right",
        "rigid_body",
        "grip_points",
        "sockets",
        "slider_joint",
        "terrain_collider",
        "lights",
        "sound",
        "particle_emitter",
        "laser",
        "spawn_point",
        "teleportal",
        "breakable",
        "trigger_volume",
    ]
}

pub fn game_object_schema() -> SchemaDescriptor {
    SchemaDescriptor {
        field_order: game_object_field_order(),
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
            comp!(breakable, "BreakableDef", Optional, breakable_fields()),
            comp!(
                trigger_volume,
                "TriggerVolumeDef",
                Optional,
                trigger_volume_fields()
            ),
        ],
    }
}

fn trigger_volume_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(t: crate::trigger_volume::TriggerVolumeDef) {
        let crate::trigger_volume::TriggerVolumeDef {
            enabled,
            var,
            on_enter,
            on_exit,
            once,
        } = t;
    }
    // Only the fields a generic inspector can render. The action lists have
    // their own editor, the same way part triggers do -- a text box holding
    // JSON would be worse than no field at all.
    vec![
        field!(enabled, Bool),
        // Optional: a zone that only runs enter/exit actions needs no state,
        // and an empty string is not the same answer as "no var".
        field!(var, Text, opt),
        field!(once, Bool),
    ]
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

fn breakable_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables)]
    fn exhaustive(b: crate::scene::BreakableDef) {
        let crate::scene::BreakableDef { health, stages } = b;
    }
    #[allow(dead_code, unused_variables)]
    fn exhaustive_stage(s: crate::scene::DamageStage) {
        let crate::scene::DamageStage { at, hidden_parts, solid } = s;
    }
    // `stages` is deliberately absent: it is a list of structs, which the
    // generic Layer 0 field renderer cannot express. The editor gives breakable
    // a bespoke card, and listing a field here that no generic surface can draw
    // would make the coverage gate pass while the authoring did not exist.
    vec![field!(health, Number)]
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
        breakable,
        trigger_volume,
        // Level geometry made of planes -- a third shape kind beside `cuboid`
        // and `mesh`, not a component you add to an object. Which shape an
        // object has is authored in the Geometry panel by creating it, the same
        // way `mesh` is set by choosing a model, so it does not belong in the
        // inspector's add-component list.
        brush,
        // Identity and structure rather than components. `id` is the human
        // name and the scripting handle; `uuid` is the stable identity that
        // `parent` points at, so a rename cannot restructure the scene. None of
        // the three is something you "add to an object" in the inspector, which
        // is what `components` describes. Listed here to satisfy the
        // exhaustiveness check.
        uuid,
        parent,
        // Organisation, not behaviour: nothing in the runtime reads a tag, and
        // there is no "add a tags component" in the inspector -- it is a label
        // set on an object that already exists. Same category as uuid/parent.
        tags,
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
    fn schema_has_every_authorable_component() {
        assert_eq!(game_object_schema().components.len(), 24);
    }

    /// `field_order` is consumed by a writer in another language, so a drift
    /// here reorders every scene file on the next save and produces a diff that
    /// looks like content changed. Assert it against what serde actually emits
    /// rather than against a second hand-written list.
    #[test]
    fn field_order_matches_serde() {
        let json = serde_json::to_string_pretty(&crate::scene::GameObject::default())
            .expect("GameObject serializes");

        // Top-level keys are exactly the lines at one indent level in
        // to_string_pretty's 2-space output.
        let emitted: Vec<&str> = json
            .lines()
            .filter_map(|line| line.strip_prefix("  \""))
            .filter_map(|rest| rest.split_once("\":"))
            .map(|(key, _)| key)
            .collect();

        // The skip_serializing_if fields, which a default object never
        // carries -- but a real file does, and the writer needs to know where
        // they sort. Keep this in step when a field gains that attribute, or
        // the test fails describing a reorder that did not happen.
        let skipped = [
            "grip_pose", "uuid", "parent", "tags", "breakable", "brush", "trigger_volume",
        ];
        let expected: Vec<&str> = game_object_field_order()
            .into_iter()
            .filter(|k| !skipped.contains(k))
            .collect();

        assert_eq!(
            emitted, expected,
            "game_object_field_order() no longer matches serde's output -- a \
             GameObject field was added, removed or reordered"
        );
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
        assert_eq!(v["components"].as_array().unwrap().len(), 24);
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
