//! Download a release zip and swap it into `Interface\AddOns` safely.
//!
//! Layout rules:
//!  - work happens inside `<AddOns>\.addonforge-tmp\` so the final step is a
//!    same-volume rename (atomic per folder), never a slow cross-drive copy;
//!  - existing folders are moved aside first and restored if anything fails;
//!  - the zip crate refuses paths that escape the extraction directory;
//!  - the extracted tree is scanned and refused if it contains anything
//!    executable (addons are Lua, XML, TOC and media, nothing else);
//!  - SavedVariables live in `WTF\`, which we never touch.

use anyhow::{anyhow, Context};
use futures_util::StreamExt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TMP_DIR: &str = ".addonforge-tmp";

/// File types that have no business inside a WoW addon.
const FORBIDDEN_EXT: &[&str] = &[
    "exe", "dll", "sys", "scr", "com", "pif", "cpl", "msi", "msp", "bat", "cmd", "ps1", "psm1",
    "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "jar", "py", "sh", "lnk", "reg", "inf",
    "ocx", "drv", "app", "dmg", "apk",
];

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

/// Walk the extracted tree; refuse anything executable. Returns file count.
pub fn safety_scan(root: &Path) -> anyhow::Result<usize> {
    fn walk(root: &Path, dir: &Path, depth: usize, count: &mut usize) -> anyhow::Result<()> {
        if depth > 32 {
            anyhow::bail!("zip nests folders absurdly deep");
        }
        for e in fs::read_dir(dir)?.flatten() {
            let p = e.path();
            let ft = e.file_type()?;
            if ft.is_symlink() {
                anyhow::bail!("zip contains a symbolic link: {}", p.display());
            }
            if ft.is_dir() {
                walk(root, &p, depth + 1, count)?;
                continue;
            }
            *count += 1;
            let name = e.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            // "foo.lua.exe" style double extensions are caught by taking the last one.
            let ext = lower.rsplit('.').next().unwrap_or("");
            if lower.contains('.') && FORBIDDEN_EXT.contains(&ext) {
                anyhow::bail!(
                    "SAFETY: refused, the zip contains an executable file ({}) which no WoW addon needs",
                    p.strip_prefix(root).unwrap_or(&p).display()
                );
            }
            // Windows also runs by "magic": check the first bytes of anything unusual.
            if !matches!(ext, "lua" | "xml" | "toc" | "tga" | "blp" | "png" | "jpg" | "jpeg" | "gif" | "ttf" | "otf" | "mp3" | "ogg" | "wav" | "txt" | "md" | "json" | "html" | "css" | "csv") {
                if let Ok(mut f) = fs::File::open(&p) {
                    let mut head = [0u8; 4];
                    use std::io::Read;
                    if f.read(&mut head).unwrap_or(0) >= 2 && &head[..2] == b"MZ" {
                        anyhow::bail!(
                            "SAFETY: refused, {} is a Windows executable in disguise",
                            p.strip_prefix(root).unwrap_or(&p).display()
                        );
                    }
                }
            }
        }
        Ok(())
    }
    let mut count = 0;
    walk(root, root, 0, &mut count)?;
    Ok(count)
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
        let files = safety_scan(&extract_dir)?;
        crate::logi!("zip ok: {} files, no executables", files);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_scan_refuses_executables() {
        let root = std::env::temp_dir().join(format!("af-safety-{}", unique()));
        let addon = root.join("Evil");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("Evil.toc"), "## Interface: 16001\n").unwrap();
        fs::write(addon.join("Evil.lua"), "print(1)\n").unwrap();
        assert!(safety_scan(&root).is_ok());
        fs::write(addon.join("helper.exe"), b"MZ\0\0").unwrap();
        let err = safety_scan(&root).unwrap_err().to_string();
        assert!(err.contains("SAFETY"), "{err}");
        fs::remove_file(addon.join("helper.exe")).unwrap();
        // Disguised: no extension, MZ header.
        fs::write(addon.join("README"), b"MZ\x90\x00").unwrap();
        assert!(safety_scan(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
