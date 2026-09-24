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
    /// Optional https icon for the Browse list (else the GitHub avatar is used).
    #[serde(default)]
    pub icon: Option<String>,
}

/// A hand-picked set of addons shown as "Starter picks" in Browse.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Bundle {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    /// Catalogue ids, in display order.
    #[serde(default)]
    pub addons: Vec<String>,
    /// Short plain-English note shown under the bundle (e.g. "pick one boss mod").
    #[serde(default)]
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Catalog {
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub addons: Vec<CatalogEntry>,
    #[serde(default)]
    pub bundles: Vec<Bundle>,
}

/// Popularity feed written daily by a GitHub Action (`catalog/stats.json`):
/// release download totals and last-release dates, so the app never has to
/// hit the GitHub API itself.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct StatEntry {
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<String>,
    /// "github" | "wowi": what the number counts.
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stats {
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub entries: std::collections::HashMap<String, StatEntry>,
}

/// Copy shipped with the build, used when GitHub is unreachable (or before the first daily refresh).
pub const BUNDLED_STATS: &str = include_str!("../../catalog/stats.json");
pub const STATS_URL: &str =
    "https://raw.githubusercontent.com/Topher121/AddonForge/main/catalog/stats.json";

pub async fn load_stats(client: &reqwest::Client) -> Stats {
    let r = async {
        client
            .get(STATS_URL)
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json::<Stats>()
            .await
            .ok()
    }
    .await;
    r.unwrap_or_else(|| serde_json::from_str(BUNDLED_STATS).unwrap_or_default())
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
    // A build can ship with a catalogue newer than what is on GitHub for a
    // little while (or the remote copy could be rolled back); take the newer
    // of the two by their `updated` date, remote on a tie.
    let local = bundled();
    match remote {
        Some(c) if !c.addons.is_empty() && c.updated >= local.updated => (c, "remote"),
        Some(_) => (local, "bundled (newer)"),
        None => (local, "bundled"),
    }
}
