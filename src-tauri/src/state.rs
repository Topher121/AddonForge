//! Persistent local state: `%APPDATA%\AddonForge\state.json` (macOS: `~/Library/Application Support/AddonForge`).
//!
//! This is the only settings file the app writes outside the game's AddOns
//! folder (plus `addonforge.log` beside it). Written atomically (tmp +
//! rename) so a crash can never corrupt it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
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
    /// A Wago Addons API key from before build 25. Unused now (Wago's updater
    /// keys went Patreon-only and the source was dropped); kept so older
    /// state files still load.
    #[serde(default)]
    pub wago_key: Option<String>,
    /// Packages we installed ourselves, keyed by package key.
    #[serde(default)]
    pub installed: HashMap<String, Installed>,
    /// Consider GitHub pre-releases and Wago beta builds when checking.
    #[serde(default)]
    pub allow_prerelease: bool,
    /// Package keys whose updates are held back.
    #[serde(default)]
    pub pinned: BTreeSet<String>,
    /// Package keys hidden from the Installed list.
    #[serde(default)]
    pub ignored: BTreeSet<String>,
    /// A self-update version the user chose to skip ("v0.2.0-b7").
    #[serde(default)]
    pub skip_self_update: Option<String>,
    /// Build number of the last version that ran, for the one-time "what's new" note.
    #[serde(default)]
    pub last_seen_build: u32,
    /// Install every available update as soon as the launch check finishes
    /// (pinned and ignored addons are left alone).
    #[serde(default)]
    pub auto_update: bool,
    /// A live `_forever_` folder the user chose not to switch to from the beta
    /// folder, so the hint is not shown again for that path.
    #[serde(default)]
    pub dismissed_live_hint: Option<String>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl AppState {
    /// `%APPDATA%\AddonForge`. `ADDONFORGE_STATE_DIR` overrides it so tests and
    /// screenshots can use a throwaway profile without touching the real one.
    pub fn dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("ADDONFORGE_STATE_DIR").filter(|v| !v.is_empty()) {
            return PathBuf::from(dir);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AddonForge")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("state.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::dir())
    }

    /// Current settings, else the last good copy, else empty.
    pub fn load_from(dir: &Path) -> Self {
        let read = |p: PathBuf| -> Option<Self> { serde_json::from_slice(&fs::read(p).ok()?).ok() };
        read(dir.join("state.json")).or_else(|| read(dir.join("state.json.bak"))).unwrap_or_default()
    }

    /// Exclusive lock on the settings folder, shared by every copy of the app.
    /// Held across reload -> change -> save so no copy can overwrite another's change.
    pub fn lock_settings() -> anyhow::Result<fs::File> {
        let dir = Self::dir();
        fs::create_dir_all(&dir)?;
        let file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("state.lock"))?;
        file.lock()?;
        Ok(file)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::dir())
    }

    /// Write the settings atomically. The last good copy is kept as `state.json.bak`:
    /// it is only replaced by a current file that still reads back as valid, so a
    /// corrupt file can never overwrite the only good copy.
    pub fn save_to(&self, dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("state.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        let final_path = dir.join("state.json");
        let current_is_valid = fs::read(&final_path)
            .map(|b| serde_json::from_slice::<AppState>(&b).is_ok())
            .unwrap_or(false);
        if current_is_valid {
            let _ = fs::copy(&final_path, dir.join("state.json.bak"));
        }
        // Rust's rename replaces an existing file on Windows, so the state file is
        // never missing part-way through a save.
        fs::rename(&tmp, &final_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("af-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_corrupt_settings_file_does_not_overwrite_the_good_backup() {
        let dir = scratch("keep-backup");
        let mut good = AppState::default();
        good.allow_prerelease = true;
        good.save_to(&dir).unwrap();
        good.save_to(&dir).unwrap(); // the good copy is now also the backup
        fs::write(dir.join("state.json"), "{ not json").unwrap();
        // Saving over a corrupt file must leave the good backup in place.
        AppState::default().save_to(&dir).unwrap();
        let backup: AppState = serde_json::from_slice(&fs::read(dir.join("state.json.bak")).unwrap()).unwrap();
        assert!(backup.allow_prerelease);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_falls_back_to_the_backup_when_the_main_file_is_corrupt() {
        let dir = scratch("fallback");
        let mut good = AppState::default();
        good.auto_update = true;
        good.save_to(&dir).unwrap();
        good.save_to(&dir).unwrap();
        fs::write(dir.join("state.json"), "broken").unwrap();
        assert!(AppState::load_from(&dir).auto_update);
        let _ = fs::remove_dir_all(&dir);
    }
}

