//! Self-update.
//!
//! Each release on GitHub carries a tiny `latest.json` asset (written by
//! build.ps1) describing the newest build. We fetch it through the stable
//! `releases/latest/download/latest.json` redirect: no API, no rate limit.
//!
//! Applying an update on Windows: the running exe can't be overwritten but
//! a new file can sit beside it. We download the new exe next to the old
//! one, start it with `--replaced <old path>`, and exit. The new process
//! deletes the old file once it is free. Nothing else on the machine is
//! touched (no installer, no registry, no service).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LATEST_URL: &str =
    "https://github.com/Topher121/AddonForge/releases/latest/download/latest.json";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Build number baked in by build.ps1 (`ADDONFORGE_BUILD`), 0 for dev builds.
pub fn build() -> u32 {
    option_env!("ADDONFORGE_BUILD").and_then(|s| s.parse().ok()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Latest {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub build: u32,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct SelfUpdate {
    pub current_version: String,
    pub current_build: u32,
    pub latest: Option<Latest>,
    pub available: bool,
    pub tag: String,
    pub error: Option<String>,
}

fn semver(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(latest: &Latest) -> bool {
    let (a, b) = (semver(&latest.version), semver(version()));
    a > b || (a == b && latest.build > build())
}

pub async fn check(client: &reqwest::Client, url_override: Option<&str>) -> SelfUpdate {
    let url = url_override.unwrap_or(LATEST_URL);
    let mut out = SelfUpdate {
        current_version: version().into(),
        current_build: build(),
        latest: None,
        available: false,
        tag: String::new(),
        error: None,
    };
    let res = async {
        let r = client.get(url).send().await?.error_for_status()?;
        r.json::<Latest>().await
    }
    .await;
    match res {
        Ok(l) if !l.version.is_empty() && !l.url.is_empty() => {
            out.available = is_newer(&l);
            out.tag = format!("v{}-b{}", l.version, l.build);
            out.latest = Some(l);
        }
        Ok(_) => out.error = Some("latest.json was empty".into()),
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}

/// Download the new exe beside the current one and hand over to it.
/// Returns the path of the new exe; the caller exits the process after.
pub async fn apply(client: &reqwest::Client, latest: &Latest) -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe()?;
    let dir = current.parent().ok_or_else(|| anyhow::anyhow!("no exe dir"))?;
    let name = if latest.filename.is_empty() {
        format!("AddonForge-v{}-b{}.exe", latest.version, latest.build)
    } else {
        latest.filename.clone()
    };
    if name.contains(['/', '\\']) || !name.to_ascii_lowercase().ends_with(".exe") {
        anyhow::bail!("refusing odd update filename {name}");
    }
    let target = dir.join(&name);
    if target == current {
        anyhow::bail!("update has the same filename as the running exe");
    }
    let tmp = dir.join(format!("{name}.part"));
    let bytes = client.get(&latest.url).send().await?.error_for_status()?.bytes().await?;
    if bytes.len() < 1_000_000 {
        anyhow::bail!("downloaded file is too small to be AddonForge ({} bytes)", bytes.len());
    }
    if latest.size > 0 && bytes.len() as u64 != latest.size {
        anyhow::bail!("downloaded size {} does not match the release's {}", bytes.len(), latest.size);
    }
    std::fs::write(&tmp, &bytes)?;
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(&tmp, &target)?;
    crate::logi!("self-update: downloaded {} ({} bytes), handing over", target.display(), bytes.len());
    std::process::Command::new(&target)
        .arg("--replaced")
        .arg(&current)
        .spawn()?;
    Ok(target)
}

/// Called on startup when launched as `--replaced <old exe>`: delete the
/// old file once the old process has let go of it.
pub fn finish_replace(old: &Path) {
    for _ in 0..20 {
        if !old.exists() || std::fs::remove_file(old).is_ok() {
            crate::logi!("self-update: removed old exe {}", old.display());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    crate::loge!("self-update: could not remove old exe {}", old.display());
}
