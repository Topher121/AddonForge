//! Download a release zip and swap it into `Interface\AddOns` safely.
//!
//! Layout rules:
//!  - work happens inside `<AddOns>\.addonforge-tmp\` so the final step is a
//!    same-volume rename (atomic per folder), never a slow cross-drive copy;
//!  - existing folders are moved aside first and restored if anything fails;
//!  - the zip crate refuses paths that escape the extraction directory;
//!  - SavedVariables live in `WTF\`, which we never touch.

use anyhow::{anyhow, Context};
use futures_util::StreamExt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TMP_DIR: &str = ".addonforge-tmp";

fn unique() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{n:x}")
}

/// Remove stale temp work from a previous crash.
pub fn clean_tmp(addons_dir: &Path) {
    let _ = fs::remove_dir_all(addons_dir.join(TMP_DIR));
}

pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest_dir: &Path,
    filename: &str,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(dest_dir)?;
    let safe: String = filename
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let safe = if safe.to_ascii_lowercase().ends_with(".zip") { safe } else { format!("{safe}.zip") };
    let path = dest_dir.join(format!("{}-{}", unique(), safe));
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("download failed: {url}"))?
        .error_for_status()
        .with_context(|| format!("download refused: {url}"))?;
    let mut file = fs::File::create(&path)?;
    let mut stream = resp.bytes_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total += chunk.len() as u64;
        if total > 512 * 1024 * 1024 {
            anyhow::bail!("download exceeded 512 MB; refusing");
        }
        file.write_all(&chunk)?;
    }
    file.flush()?;
    Ok(path)
}

fn has_toc(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".toc"))
        })
        .unwrap_or(false)
}

/// Work out which extracted directories are addon folders.
fn addon_dirs(extracted: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(extracted)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        anyhow::bail!("zip contains no addon folder");
    }
    // A wrapper directory (e.g. "MyAddon-1.2/") holding the real folders.
    if dirs.len() == 1 && !has_toc(&dirs[0]) {
        let inner: Vec<PathBuf> = fs::read_dir(&dirs[0])?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && has_toc(p))
            .collect();
        if !inner.is_empty() {
            return Ok(inner);
        }
    }
    let with_toc: Vec<PathBuf> = dirs.iter().filter(|d| has_toc(d)).cloned().collect();
    if with_toc.is_empty() {
        anyhow::bail!("zip has folders but none contains a .toc file");
    }
    Ok(with_toc)
}

/// Extract `zip_path` and move its addon folders into `addons_dir`,
/// replacing same-named folders. Returns the folder names installed.
pub fn place(zip_path: &Path, addons_dir: &Path) -> anyhow::Result<Vec<String>> {
    fs::create_dir_all(addons_dir)?;
    let work = addons_dir.join(TMP_DIR).join(unique());
    let extract_dir = work.join("x");
    let backup_dir = work.join("old");
    fs::create_dir_all(&extract_dir)?;
    fs::create_dir_all(&backup_dir)?;

    let result = (|| -> anyhow::Result<Vec<String>> {
        let file = fs::File::open(zip_path)?;
        let mut archive = zip::ZipArchive::new(file).context("not a valid zip")?;
        archive.extract(&extract_dir).context("extract failed")?;

        let sources = addon_dirs(&extract_dir)?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new(); // (target, backup)
        let mut installed = Vec::new();

        let restore = |moved: &Vec<(PathBuf, PathBuf)>| {
            for (target, backup) in moved.iter().rev() {
                let _ = fs::remove_dir_all(target);
                let _ = fs::rename(backup, target);
            }
        };

        for src in sources {
            let name = src
                .file_name()
                .ok_or_else(|| anyhow!("bad folder name"))?
                .to_string_lossy()
                .to_string();
            if name.starts_with('.') || name.contains("..") {
                anyhow::bail!("refusing suspicious folder name {name}");
            }
            let target = addons_dir.join(&name);
            let backup = backup_dir.join(&name);
            if target.exists() {
                if let Err(e) = fs::rename(&target, &backup) {
                    restore(&moved);
                    return Err(anyhow!("could not move aside {name}: {e} (is WoW running?)"));
                }
                moved.push((target.clone(), backup));
            } else {
                moved.push((target.clone(), backup));
            }
            if let Err(e) = fs::rename(&src, &target) {
                restore(&moved);
                return Err(anyhow!("could not install {name}: {e}"));
            }
            installed.push(name);
        }
        Ok(installed)
    })();

    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_file(zip_path);
    // Leave the tmp root tidy if empty.
    let _ = fs::remove_dir(addons_dir.join(TMP_DIR));
    result
}

/// Delete the given addon folders (used by "Remove").
pub fn remove_folders(addons_dir: &Path, folders: &[String]) -> anyhow::Result<Vec<String>> {
    let mut gone = Vec::new();
    for f in folders {
        if f.is_empty() || f.starts_with('.') || f.contains("..") || f.contains(['/', '\\']) {
            anyhow::bail!("refusing to remove suspicious folder name {f}");
        }
        let p = addons_dir.join(f);
        if p.is_dir() {
            fs::remove_dir_all(&p).with_context(|| format!("remove {f}"))?;
            gone.push(f.clone());
        }
    }
    Ok(gone)
}
