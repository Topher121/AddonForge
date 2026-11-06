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
    /// Group otherwise unowned direct required dependencies of the first folder.
    #[serde(default)]
    pub group_required_dependencies: bool,
    #[serde(default)]
    pub url: Option<String>,
    /// Does the author publish a Forever build?  true / false / null=unknown.
    #[serde(default)]
    pub forever: Option<bool>,
    /// Optional https icon for the Browse list (else the GitHub avatar is used).
    /// Only fetched from the hosts in `icons::allowed_image_host`.
    #[serde(default)]
    pub icon: Option<String>,
    /// A short line shown on the installed row, e.g. "beta workaround, remove
    /// once Blizzard fixes X". Changing it is a catalogue edit, no release needed.
    #[serde(default)]
    pub notice: Option<String>,
    /// The author has switched off third-party downloads on CurseForge, so
    /// the `curse` id is a link only even in builds that carry the API key.
    #[serde(default)]
    pub curse_locked: bool,
    /// Date the entry was added (YYYY-MM-DD). Feeds "New in the catalogue".
    #[serde(default)]
    pub added: Option<String>,
    /// Earlier `owner/repo` names for `github`, for when the author renamed or
    /// moved the repo. A saved install still pointing at one of these follows
    /// `github` instead, so nobody who later registers the old name can ship
    /// updates to people who installed before the move.
    #[serde(default)]
    pub github_moved_from: Vec<String>,
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

/// The bytes the catalogue signature covers: the file with any `\r` removed, so a
/// Windows checkout (CRLF) and GitHub's copy (LF) sign and verify the same.
pub fn signed_bytes(raw: &[u8]) -> Vec<u8> {
    raw.iter().copied().filter(|&b| b != b'\r').collect()
}

/// The remote catalogue decides where every user downloads from, so it is only
/// used when `forever.json.sig` (Ed25519, the release key) matches it.
pub fn verify_remote(raw: &[u8], sig: &str) -> anyhow::Result<Catalog> {
    verify_with(raw, sig, &crate::signing::verified_key()?)
}

fn verify_with(raw: &[u8], sig: &str, key: &[u8; 32]) -> anyhow::Result<Catalog> {
    crate::signing::verify_manifest(&signed_bytes(raw), sig, key)?;
    Ok(serde_json::from_slice(raw)?)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let r = client.get(url).timeout(Duration::from_secs(6)).send().await.ok()?.error_for_status().ok()?;
    r.bytes().await.ok().map(|b| b.to_vec())
}

/// Latest catalogue: the signed remote copy if reachable within a few seconds, else bundled.
pub async fn load(client: &reqwest::Client) -> (Catalog, &'static str) {
    let sig_url = format!("{REMOTE_URL}.sig");
    let (raw, sig) = tokio::join!(fetch_text(client, REMOTE_URL), fetch_text(client, &sig_url));
    let remote = match (raw, sig) {
        (Some(raw), Some(sig)) => match verify_remote(&raw, &String::from_utf8_lossy(&sig)) {
            Ok(c) => Some(c),
            Err(e) => {
                crate::loge!("catalogue on GitHub refused (signature): {e}");
                return (bundled(), "bundled (GitHub copy not signed)");
            }
        },
        (Some(_), None) => {
            crate::loge!("catalogue on GitHub has no signature; using the built-in copy");
            return (bundled(), "bundled (GitHub copy not signed)");
        }
        _ => None,
    };
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


#[cfg(test)]
mod signature_tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn remote_catalogue_needs_a_matching_signature() {
        let sk = SigningKey::from_bytes(&[5u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let lf: &[u8] = b"{\n  \"updated\": \"2026-11-06\",\n  \"addons\": []\n}\n";
        let sig = base64::engine::general_purpose::STANDARD.encode(sk.sign(lf).to_bytes());
        assert!(verify_with(lf, &sig, &pk).is_ok());
        // A CRLF checkout of the same file still verifies.
        let crlf = String::from_utf8_lossy(lf).replace('\n', "\r\n");
        assert!(verify_with(crlf.as_bytes(), &sig, &pk).is_ok());
        // Any change to the content, or another key, is refused.
        let changed = String::from_utf8_lossy(lf).replace("2026-11-06", "2026-11-07");
        assert!(verify_with(changed.as_bytes(), &sig, &pk).is_err());
        let other = SigningKey::from_bytes(&[6u8; 32]).verifying_key().to_bytes();
        assert!(verify_with(lf, &sig, &other).is_err());
        assert!(verify_with(lf, "not a signature", &pk).is_err());
    }
}
