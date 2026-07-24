use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry_scene: String,
    pub scenes: Vec<String>,
}

impl Manifest {
    pub fn load(game_dir: &Path) -> Result<Self> {
        let path = game_dir.join("manifest.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    pub fn save(&self, game_dir: &Path) -> Result<()> {
        let path = game_dir.join("manifest.json");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn scene_path(game_dir: &Path, scene_name: &str) -> PathBuf {
        game_dir.join("scenes").join(format!("{scene_name}.json"))
    }

    pub fn entry_scene_path(&self, game_dir: &Path) -> PathBuf {
        Self::scene_path(game_dir, &self.entry_scene)
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            name: "Untitled Game".into(),
            version: "0.1.0".into(),
            entry_scene: "lobby".into(),
            scenes: vec!["lobby".into()],
        }
    }
}

