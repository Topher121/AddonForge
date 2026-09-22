//! Persistent local state: `%APPDATA%\AddonForge\state.json`.
//!
//! This is the only file the app writes outside the game's AddOns folder.
//! Written atomically (tmp + rename) so a crash can never corrupt it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Installed {
    /// Source kind: github | wago | wowi | tukui
    pub source: String,
    pub source_id: String,
    pub version: String,
    pub folders: Vec<String>,
    pub installed_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AppState {
    /// Product folder we manage, e.g. `C:\...\World of Warcraft\_classic_beta_`.
    #[serde(default)]
    pub install_path: Option<String>,
    /// Optional user-supplied Wago Addons API key (stored locally only).
    #[serde(default)]
    pub wago_key: Option<String>,
    /// Packages we installed ourselves, keyed by package key.
    #[serde(default)]
    pub installed: HashMap<String, Installed>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl AppState {
    pub fn dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AddonForge")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("state.json")
    }

    pub fn load() -> Self {
        match fs::read(Self::path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::dir();
        fs::create_dir_all(&dir)?;
        let tmp = dir.join("state.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        let final_path = Self::path();
        if final_path.exists() {
            fs::remove_file(&final_path)?;
        }
        fs::rename(&tmp, &final_path)?;
        Ok(())
    }
}
