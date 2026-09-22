//! The addon catalogue: a JSON file in this repo (`catalog/forever.json`).
//!
//! It is a *pointer list*, not a host: every entry says where the author
//! publishes releases (GitHub, Wago, WoWInterface, TukUI). Downloads always
//! come straight from that source. A copy is compiled into the exe as a
//! fallback; at runtime we try the latest copy on GitHub first.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const BUNDLED: &str = include_str!("../../catalog/forever.json");
pub const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/Topher121/AddonForge/main/catalog/forever.json";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub category: String,
    /// `owner/repo` on GitHub. Releases are read from
    /// `https://github.com/<owner>/<repo>/releases/latest/download/release.json`.
    #[serde(default)]
    pub github: Option<String>,
    /// Wago Addons project id (needs the user's own free API key).
    #[serde(default)]
    pub wago: Option<String>,
    /// WoWInterface file id.
    #[serde(default)]
    pub wowi: Option<u64>,
    /// TukUI slug (elvui / tukui).
    #[serde(default)]
    pub tukui: Option<String>,
    /// CurseForge project id: link-out only, we never download from CurseForge.
    #[serde(default)]
    pub curse: Option<u64>,
    /// Substring to pick the right zip when a GitHub release has several
    /// and no release.json.
    #[serde(default)]
    pub asset_hint: Option<String>,
    /// Top-level AddOns folders the release installs (used to match what's
    /// already on disk to this entry).
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Does the author publish a Forever build?  true / false / null=unknown.
    #[serde(default)]
    pub forever: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Catalog {
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub addons: Vec<CatalogEntry>,
}

pub fn bundled() -> Catalog {
    serde_json::from_str(BUNDLED).unwrap_or_default()
}

/// Latest catalogue: remote copy if reachable within a few seconds, else bundled.
pub async fn load(client: &reqwest::Client) -> (Catalog, &'static str) {
    let remote = async {
        let r = client
            .get(REMOTE_URL)
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        r.json::<Catalog>().await.ok()
    }
    .await;
    match remote {
        Some(c) if !c.addons.is_empty() => (c, "remote"),
        _ => (bundled(), "bundled"),
    }
}
