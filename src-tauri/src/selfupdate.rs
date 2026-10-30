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
use std::path::Path;
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;

pub const LATEST_URL: &str =
    "https://github.com/Topher121/AddonForge/releases/latest/download/latest.json";

/// Where non-Windows users get sent to update by hand.
#[cfg(not(windows))]
pub const RELEASES_URL: &str = "https://github.com/Topher121/AddonForge/releases/latest";

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
    /// SHA-256 (hex) of the exe at `url`. Required: apply() refuses a download
    /// that does not match it.
    #[serde(default)]
    pub sha256: String,
    /// Recent builds' notes, newest first, so a user who skipped builds
    /// still reads what changed in each of them.
    #[serde(default)]
    pub history: Vec<HistoryNote>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HistoryNote {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub build: u32,
    #[serde(default)]
    pub notes: String,
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
    // Off Windows the app is built from source (build 0), so only the version counts.
    a > b || (cfg!(windows) && a == b && latest.build > build())
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
    // The manifest is trusted only with a valid signature from the release key.
    let res: anyhow::Result<Latest> = async {
        let manifest = crate::icons::read_capped(client.get(url).send().await?.error_for_status()?, 1024 * 1024).await?;
        let sig_resp = client.get(format!("{url}.sig")).send().await?.error_for_status()?;
        let sig = String::from_utf8(crate::icons::read_capped(sig_resp, 4096).await?)?;
        let key = crate::signing::verified_key()?;
        crate::signing::verify_manifest(&manifest, &sig, &key)?;
        Ok(serde_json::from_slice(&manifest)?)
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

/// Is this exe the portable download (`AddonForge-v0.2.0-b5.exe`) rather
/// than an installed copy (`AddonForge.exe` under the installer's folder)?
#[cfg(windows)]
fn is_portable(exe: &Path) -> bool {
    let name = exe.file_name().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    name.starts_with("addonforge-v") && name.contains("-b") && name.ends_with(".exe")
}

/// Download the new exe and hand over to it. Returns the path of the exe
/// that will be running next; the caller exits the process after.
///
/// Portable: the new build lands BESIDE the old file under its own
/// versioned name, and the new process deletes the old file.
/// Installed: the running exe is renamed to `.old` and the new build takes
/// its exact path, so Start-menu shortcuts and the uninstaller keep working;
/// the new process deletes the `.old` file.
#[cfg(windows)]
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
    let portable = is_portable(&current);
    let target = if portable { dir.join(&name) } else { current.clone() };
    if portable && target == current {
        anyhow::bail!("update has the same filename as the running exe");
    }
    let tmp = dir.join(format!("{name}.part"));
    let bytes = crate::icons::read_capped(client.get(&latest.url).send().await?.error_for_status()?, 200 * 1024 * 1024).await?;
    if bytes.len() < 1_000_000 {
        anyhow::bail!("downloaded file is too small to be AddonForge ({} bytes)", bytes.len());
    }
    crate::signing::check_sha256(&bytes, &latest.sha256)?;
    if latest.size > 0 && bytes.len() as u64 != latest.size {
        anyhow::bail!("downloaded size {} does not match the release's {}", bytes.len(), latest.size);
    }
    std::fs::write(&tmp, &bytes)?;
    let old: PathBuf;
    if portable {
        if target.exists() {
            std::fs::remove_file(&target)?;
        }
        std::fs::rename(&tmp, &target)?;
        old = current.clone();
    } else {
        // A running exe can be renamed on Windows, just not overwritten.
        old = current.with_extension("exe.old");
        if old.exists() {
            std::fs::remove_file(&old)?;
        }
        std::fs::rename(&current, &old)?;
        if let Err(e) = std::fs::rename(&tmp, &target) {
            // Put things back rather than leave the app with no exe.
            let _ = std::fs::rename(&old, &current);
            return Err(e.into());
        }
    }
    crate::logi!(
        "self-update: {} -> {} ({} bytes, {}), handing over",
        current.display(),
        target.display(),
        bytes.len(),
        if portable { "portable" } else { "in place" }
    );
    // The new copy writes this marker once its window is up. Only then is the old
    // copy removed; until then the old copy is the fallback.
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let marker = dir.join(format!("AddonForge-started-{nanos:x}.flag"));
    // A launch refused by Defender or Windows fails in spawn(), not in the child, so
    // both failure paths go through the same rollback below.
    let started = match std::process::Command::new(&target)
        .arg("--replaced")
        .arg(&old)
        .arg(&marker)
        .spawn()
    {
        Ok(child) => tokio::task::spawn_blocking(move || wait_for_start(child, &marker)).await?,
        Err(e) => {
            crate::loge!("self-update: could not launch the new version: {e}");
            false
        }
    };
    if !started {
        let (t, o, c) = (target.clone(), old.clone(), current.clone());
        let restored = tokio::task::spawn_blocking(move || restore_previous(&t, &o, &c, portable)).await?;
        return Err(match restored {
            Ok(()) => anyhow::anyhow!("the new version did not start, so the previous version was kept"),
            Err(e) => {
                crate::loge!("self-update: rollback failed: {e}");
                anyhow::anyhow!(
                    "the new version did not start and the previous version could not be put back ({e}).                      Reinstall AddonForge from the releases page; your addons are not affected."
                )
            }
        });
    }
    Ok(target)
}

/// Put the previous copy back after a failed launch. Portable: the old file was
/// never moved, so only the new file goes. Installed: the old exe was renamed
/// aside, so it is renamed back over the new one.
pub(crate) fn restore_previous(target: &Path, old: &Path, current: &Path, portable: bool) -> std::io::Result<()> {
    // Antivirus and Windows can hold a file open for a moment, so retry before giving up.
    retry_io(|| if target.exists() { std::fs::remove_file(target) } else { Ok(()) })?;
    if !portable {
        retry_io(|| std::fs::rename(old, current))?;
    }
    Ok(())
}

/// Run an operation, retrying for a few seconds while a file is briefly locked.
fn retry_io(mut op: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut last = None;
    for _ in 0..10 {
        match op() {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("operation failed")))
}

/// True once the new copy has written its start marker. False if it exits first,
/// or does not start within 20 seconds (it is then stopped).
#[cfg(windows)]
fn wait_for_start(mut child: std::process::Child, marker: &Path) -> bool {
    for _ in 0..40 {
        if marker.exists() {
            let _ = std::fs::remove_file(marker);
            return true;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let _ = child.kill();
    let _ = child.wait();
    false
}

/// Set by main when launched as `--replaced <old exe> [start marker]`.
static PENDING_REPLACE: std::sync::OnceLock<(PathBuf, Option<PathBuf>)> = std::sync::OnceLock::new();

pub fn note_replace(old: PathBuf, marker: Option<PathBuf>) {
    let _ = PENDING_REPLACE.set((old, marker));
}

/// Create the start marker, but only as a new file named like ours, in the app
/// folder. It never overwrites an existing file.
fn write_start_marker(marker: &Path) {
    let app_dir = std::env::current_exe().ok().and_then(|me| me.parent().map(Path::to_path_buf));
    let name = marker.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ours = name.starts_with("AddonForge-started-") && name.ends_with(".flag");
    if app_dir.as_deref() != marker.parent() || !ours {
        crate::loge!("self-update: refusing to write start marker {}", marker.display());
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(marker) {
        use std::io::Write;
        let _ = f.write_all(version().as_bytes());
    }
}

/// Called once the window is up: this copy has really started, so it tells the
/// old copy and removes the old exe.
pub fn confirm_started() {
    if let Some((old, marker)) = PENDING_REPLACE.get() {
        if let Some(marker) = marker {
            write_start_marker(marker);
        }
        let old = old.clone();
        std::thread::spawn(move || finish_replace(&old));
    }
}

/// Only the old AddonForge exe (`AddonForge*.exe` or `AddonForge*.exe.old`)
/// sitting in the app's own folder may be removed. The argument comes from the
/// command line, so it is never trusted as a path to delete.
fn is_update_leftover(old: &Path, app_dir: &Path) -> bool {
    if old.parent().map(|p| p.to_string_lossy().to_ascii_lowercase())
        != Some(app_dir.to_string_lossy().to_ascii_lowercase())
    {
        return false;
    }
    let Some(name) = old.file_name().and_then(|n| n.to_str()) else { return false };
    let lower = name.to_ascii_lowercase();
    lower.starts_with("addonforge") && (lower.ends_with(".exe") || lower.ends_with(".exe.old"))
}

/// Called on startup when launched as `--replaced <old exe>`: delete the
/// old file once the old process has let go of it.
pub fn finish_replace(old: &Path) {
    let dir = std::env::current_exe().ok().and_then(|me| me.parent().map(Path::to_path_buf));
    let Some(dir) = dir else {
        crate::loge!("self-update: refusing to clean up {}: no app folder", old.display());
        return;
    };
    if !is_update_leftover(old, &dir) {
        crate::loge!("self-update: refusing to delete {}: not an AddonForge file in the app folder", old.display());
        return;
    }
    for _ in 0..20 {
        if !old.exists() || std::fs::remove_file(old).is_ok() {
            crate::logi!("self-update: removed old exe {}", old.display());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    crate::loge!("self-update: could not remove old exe {}", old.display());
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn same_version_higher_build_is_not_newer_off_windows() {
        let same = Latest { version: version().into(), build: 99, ..Default::default() };
        assert!(!is_newer(&same));
        let next = Latest { version: "999.0.0".into(), ..Default::default() };
        assert!(is_newer(&next));
    }
}

#[cfg(test)]
mod leftover_tests {
    use super::is_update_leftover;
    use std::path::Path;

    #[test]
    fn only_our_old_exes_in_the_app_folder_are_removable() {
        let dir = Path::new("C:/Apps/AddonForge");
        assert!(is_update_leftover(Path::new("C:/Apps/AddonForge/AddonForge-v0.4.0-b23.exe"), dir));
        assert!(is_update_leftover(Path::new("C:/Apps/AddonForge/addonforge.exe.old"), dir));
        // Wrong folder, wrong name, or not an exe: refused.
        assert!(!is_update_leftover(Path::new("C:/Users/someone/Documents/AddonForge.exe"), dir));
        assert!(!is_update_leftover(Path::new("C:/Apps/AddonForge/state.json"), dir));
        assert!(!is_update_leftover(Path::new("C:/Apps/AddonForge/WTF/Account.lua"), dir));
        assert!(!is_update_leftover(Path::new("C:/Apps/AddonForge/notes.exe"), dir));
        assert!(!is_update_leftover(Path::new("C:/Apps/AddonForge/sub/AddonForge.exe"), dir));
    }
}

#[cfg(test)]
mod rollback_tests {
    use super::restore_previous;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("af-rollback-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn installed_copy_gets_the_old_exe_back_in_place() {
        let dir = scratch("installed");
        let current = dir.join("AddonForge.exe");
        let old = dir.join("AddonForge.exe.old");
        // The update renamed the old exe aside and wrote the new one at the same path.
        fs::write(&old, "old build").unwrap();
        fs::write(&current, "new build").unwrap();
        restore_previous(&current, &old, &current, false).unwrap();
        assert_eq!(fs::read_to_string(&current).unwrap(), "old build");
        assert!(!old.exists());
    }

    #[test]
    fn portable_copy_keeps_the_old_exe_and_drops_the_new_one() {
        let dir = scratch("portable");
        let old = dir.join("AddonForge-v0.4.0-b23.exe");
        let target = dir.join("AddonForge-v0.4.0-b27.exe");
        fs::write(&old, "old build").unwrap();
        fs::write(&target, "new build").unwrap();
        restore_previous(&target, &old, &old, true).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read_to_string(&old).unwrap(), "old build");
    }
}

