use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoneAssignment {
    pub role: String,
    pub bone_name: String,
}

fn one() -> f32 {
    1.0
}
fn identity_quat_arr() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceOffsetDef {
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default = "identity_quat_arr")]
    pub rotation: [f32; 4],
    #[serde(default = "one")]
    pub scale: f32,
}

impl Default for DeviceOffsetDef {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: identity_quat_arr(), scale: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointConstraintDef {
    pub bone_name: String,
    #[serde(default)]
    pub min_deg: [f32; 3],
    #[serde(default)]
    pub max_deg: [f32; 3],
    #[serde(default)]
    pub twist_limit_deg: f32,
    #[serde(default = "one")]
    pub stretch_limit: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeightCalibrationDef {
    pub calibrated_height_m: f32,
    pub eye_height_m: f32,
    pub neck_length_m: f32,
    pub shoulder_width_m: f32,
    pub forward_offset_m: f32,
}

impl Default for HeightCalibrationDef {
    fn default() -> Self {
        Self {
            calibrated_height_m: 1.7,
            eye_height_m: 1.6,
            neck_length_m: 0.1,
            shoulder_width_m: 0.4,
            forward_offset_m: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RigProfileDef {
    pub name: String,
    pub source_model: String,
    pub bone_assignments: Vec<BoneAssignment>,
    pub device_offsets: HashMap<String, DeviceOffsetDef>,
    pub constraints: Vec<JointConstraintDef>,
    pub height: HeightCalibrationDef,
    pub ik: avatar_ik::RigConfig,
    pub poses: Vec<String>,
    pub input_mapping: HashMap<String, String>,
}

impl Default for RigProfileDef {
    fn default() -> Self {
        Self {
            name: "Untitled Rig".into(),
            source_model: String::new(),
            bone_assignments: Vec::new(),
            device_offsets: HashMap::new(),
            constraints: Vec::new(),
            height: HeightCalibrationDef::default(),
            ik: avatar_ik::RigConfig::default(),
            poses: Vec::new(),
            input_mapping: HashMap::new(),
        }
    }
}

impl RigProfileDef {
    pub fn dir(game_dir: &Path) -> PathBuf {
        game_dir.join("rig_profiles")
    }

    pub fn path(game_dir: &Path, name: &str) -> PathBuf {
        Self::dir(game_dir).join(format!("{name}.json"))
    }

    pub fn load(game_dir: &Path, name: &str) -> Result<Self> {
        let path = Self::path(game_dir, name);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let profile: RigProfileDef = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(profile)
    }

    pub fn save(&self, game_dir: &Path) -> Result<()> {
        let dir = Self::dir(game_dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        let path = dir.join(format!("{}.json", self.name));
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn list(game_dir: &Path) -> Result<Vec<String>> {
        let dir = Self::dir(game_dir);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read {}", dir.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn legacy_from_avatar_rig_json(game_dir: &Path) -> Self {
        let ik = avatar_ik::load_rig_config(&game_dir.join("avatar_rig.json"));
        Self { name: "default".into(), ik, ..Self::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips_every_field() {
        let dir = std::env::temp_dir().join(format!("rig_profile_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut profile = RigProfileDef::default();
        profile.name = "test_rig".into();
        profile.source_model = "models/boy/boy.glb".into();
        profile.bone_assignments.push(BoneAssignment { role: "head".into(), bone_name: "Head".into() });
        profile.device_offsets.insert("left_controller".into(), DeviceOffsetDef::default());
        profile.constraints.push(JointConstraintDef {
            bone_name: "Left elbow".into(),
            min_deg: [0.0, 0.0, 0.0],
            max_deg: [150.0, 0.0, 0.0],
            twist_limit_deg: 45.0,
            stretch_limit: 1.0,
        });

        profile.save(&dir).unwrap();
        let loaded = RigProfileDef::load(&dir, "test_rig").unwrap();

        assert_eq!(loaded.source_model, "models/boy/boy.glb");
        assert_eq!(loaded.bone_assignments.len(), 1);
        assert_eq!(loaded.bone_assignments[0].bone_name, "Head");
        assert!(loaded.device_offsets.contains_key("left_controller"));
        assert_eq!(loaded.constraints[0].max_deg[0], 150.0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_returns_empty_not_an_error_when_no_profiles_exist_yet() {
        let dir = std::env::temp_dir().join(format!("rig_profile_empty_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(RigProfileDef::list(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_from_avatar_rig_json_falls_back_to_default_ik_when_file_absent() {
        let dir = std::env::temp_dir().join(format!("rig_profile_legacy_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let profile = RigProfileDef::legacy_from_avatar_rig_json(&dir);
        assert_eq!(profile.ik.finger_curl_max_deg, avatar_ik::RigConfig::default().finger_curl_max_deg);
        std::fs::remove_dir_all(&dir).ok();
    }
}

