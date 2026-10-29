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

    pub field_order: Vec<&'static str>,
}

impl SchemaDescriptor {
    #[must_use]
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

#[must_use]
pub fn game_object_field_order() -> Vec<&'static str> {
    vec![
        "id",

        "uuid",
        "parent",
        "cuboid",
        "mesh",

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

        "grip_pose",
        "grip_pose_left",
        "grip_pose_right",
        "rigid_body",
        "grip_points",
        "sockets",
        "slider_joint",
        "door",
        "terrain_collider",

        "walk_area",
        "lights",
        "sound",
        "particle_emitter",
        "effect",
        "laser",
        "optic",
        "spawn_point",
        "teleportal",
        "reflection_probe",
        "breakable",
        "trigger_volume",
        "solid",
    ]
}

#[must_use]
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
            comp!(door, "DoorDef", Optional, door_fields()),
            comp!(
                terrain_collider,
                "TerrainColliderDef",
                Optional,
                terrain_collider_fields()
            ),
            comp!(walk_area, "WalkAreaDef", Optional, walk_area_fields()),
            comp!(lights, "LightDef", List),
            comp!(sound, "SoundSourceDef", Optional, sound_fields()),
            comp!(
                particle_emitter,
                "ParticleEmitterDef",
                Optional,
                particle_emitter_fields()
            ),
            comp!(effect, "EffectDef", Optional, effect_fields()),
            comp!(laser, "LaserDef", Optional, laser_fields()),
            comp!(optic, "OpticDef", Optional, optic_fields()),
            comp!(spawn_point, "SpawnPointDef", Optional),
            comp!(teleportal, "TeleportalDef", Optional, teleportal_fields()),
            comp!(reflection_probe, "ReflectionProbeDef", Optional, reflection_probe_fields()),
            comp!(breakable, "BreakableDef", Optional, breakable_fields()),
            comp!(
                trigger_volume,
                "TriggerVolumeDef",
                Optional,
                trigger_volume_fields()
            ),

            comp!(solid, "bool", Optional),
        ],
    }
}

fn trigger_volume_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(t: crate::trigger_volume::TriggerVolumeDef) {
        let crate::trigger_volume::TriggerVolumeDef {
            enabled,
            var,
            on_enter,
            on_exit,
            once,
        } = t;
    }

    vec![
        field!(enabled, Bool),

        field!(var, Text, opt),
        field!(once, Bool),
    ]
}

/// Leaf fields of `OpticDef`. Nested structures (magnification, paths, reticle,
/// zero) have no `FieldKind`, so only scalars and plain enums are described; the
/// exhaustive destructure still covers every field.
fn optic_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(o: crate::scene::OpticDef) {
        let crate::scene::OpticDef {
            class,
            magnification,
            magnification_control,
            objective_diameter_mm,
            true_fov_deg_at_1x,
            eye_relief_mm,
            paths,
            reticle,
            zero,
            quality,
        } = o;
    }
    vec![
        field!(
            class,
            Enum,
            [
                "ReflexRedDot",
                "Holographic",
                "Lpvo",
                "FixedPrism",
                "PrecisionScope",
                "Binocular",
            ]
        ),
        field!(objective_diameter_mm, Number),
        field!(true_fov_deg_at_1x, Number),
        field!(eye_relief_mm, Number),
        field!(quality, Enum, ["Low", "Balanced", "Ultra"]),
    ]
}

fn sound_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
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
            audio,
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

fn walk_area_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(w: crate::walk_area::WalkAreaDef) {
        let crate::walk_area::WalkAreaDef { fence_height, lift, flat: _, bumps: _ } = w;
    }
    vec![field!(fence_height, Number), field!(lift, Number)]
}

fn terrain_collider_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(t: crate::scene::TerrainColliderDef) {
        let crate::scene::TerrainColliderDef { node_filter } = t;
    }
    vec![field!(node_filter, Text, opt)]
}

fn laser_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
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
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
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

fn effect_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(e: crate::scene::EffectDef) {
        let crate::scene::EffectDef {
            kind,
            scale,
            rate,
            tint,
            intensity,
            size_variation,
            life_variation,
            speed_variation,
            temperature_variation,
            flicker,
            flicker_rate,
            turbulence,
            seed,
            asset,
        } = e;
    }
    vec![
        field!(kind, Enum, ["fire", "smoke", "embers", "dust", "coals"]),
        field!(scale, Number),
        field!(rate, Number),
        field!(tint, Color),
        field!(intensity, Number),
        field!(size_variation, Number),
        field!(life_variation, Number),
        field!(speed_variation, Number),
        field!(temperature_variation, Number),
        field!(flicker, Number),
        field!(flicker_rate, Number),
        field!(turbulence, Number),
        field!(seed, Number),
        field!(asset, Text, opt),
    ]
}

fn reflection_probe_fields() -> Vec<FieldDescriptor> {
    vec![field!(resolution, Number)]
}

fn door_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(d: crate::door::DoorDef) {
        let crate::door::DoorDef {
            hinge,
            open_min_deg,
            open_max_deg,
            mass,
            friction,
            closer,
            surface_friction,
            start_deg,
            latch,
            handle,
            lever,
            pin_inset,
            pin_offset,
            slide_m,
            auto,
            locked,
        } = d;
    }
    vec![
        field!(hinge, Enum, ["Left", "Right"]),
        field!(open_min_deg, Number),
        field!(open_max_deg, Number),
        field!(mass, Number),
        field!(friction, Number),
        field!(closer, Number),
        field!(surface_friction, Number),
        field!(start_deg, Number),
        field!(latch, Bool),
        field!(handle, Vec3, opt),
        field!(lever, Text, opt),
        field!(pin_inset, Number),
        field!(pin_offset, Number),
        field!(slide_m, Number),
    ]
}

fn teleportal_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(t: crate::scene::TeleportalDef) {
        let crate::scene::TeleportalDef { target_id, target_scene } = t;
    }
    vec![field!(target_id, Text, opt), field!(target_scene, Text, opt)]
}

fn breakable_fields() -> Vec<FieldDescriptor> {
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive(b: crate::scene::BreakableDef) {
        let crate::scene::BreakableDef { health, stages } = b;
    }
    #[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
    fn exhaustive_stage(s: crate::scene::DamageStage) {
        let crate::scene::DamageStage { at, hidden_parts, solid, removed } = s;
    }

    vec![field!(health, Number)]
}

#[allow(dead_code, unused_variables, reason = "schema mirror kept in step with the scripting API, not every field is read yet")]
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
        door,
        terrain_collider,
        walk_area,
        prop,
        selector,
        menu_panel,
        lights,
        sound,
        particle_emitter,
        effect,
        laser,
        optic,
        spawn_point,
        teleportal,
        reflection_probe,
        breakable,
        trigger_volume,
        solid,

        brush,

        uuid,
        parent,

        tags,

        hidden_parts,
    } = o;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_every_authorable_component() {
        assert_eq!(game_object_schema().components.len(), 30);
    }

    #[test]
    fn field_order_matches_serde() {
        let json = serde_json::to_string_pretty(&crate::scene::GameObject::default())
            .expect("GameObject serializes");

        let emitted: Vec<&str> = json
            .lines()
            .filter_map(|line| line.strip_prefix("  \""))
            .filter_map(|rest| rest.split_once("\":"))
            .map(|(key, _)| key)
            .collect();

        let skipped = [
            "grip_pose", "uuid", "parent", "tags", "breakable", "brush", "trigger_volume",

            "reflection_probe",

            "solid",

            "effect",

            "door",

            "walk_area",
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
            "optic",
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

        let effect = by("effect");
        assert_eq!(effect.fields.len(), 14, "EffectDef has 14 fields");
        assert!(effect.fields.iter().any(|f| f.name == "asset" && f.kind == FieldKind::Text && f.optional));
        assert_eq!(effect.fields[0].variants, &["fire", "smoke", "embers", "dust", "coals"]);

        let door = by("door");
        assert_eq!(door.fields.len(), 13, "DoorDef has 13 fields");
        assert_eq!(door.fields[0].variants, &["Left", "Right"]);

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
        assert_eq!(v["components"].as_array().unwrap().len(), 30);
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

        let normalise = |s: &str| s.replace("\r\n", "\n").trim().to_string();
        assert_eq!(
            normalise(&checked_in),
            normalise(&generated),
            "schema.json is stale — regenerate with `cargo run --bin emit_schema`"
        );
    }
}
