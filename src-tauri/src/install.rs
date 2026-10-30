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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TMP_DIR: &str = ".addonforge-tmp";

/// File types that have no business inside a WoW addon.
const FORBIDDEN_EXT: &[&str] = &[
    "exe", "dll", "sys", "scr", "com", "pif", "cpl", "msi", "msp", "bat", "cmd", "ps1", "psm1",
    "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "jar", "py", "sh", "lnk", "reg", "inf",
    "ocx", "drv", "app", "dmg", "apk", "command", "dylib", "so", "pkg", "scpt", "tool", "terminal",
];

/// macOS bundles are directories; refuse folders with these extensions.
const FORBIDDEN_DIR_EXT: &[&str] = &[
    "app", "bundle", "framework", "kext", "plugin", "workflow", "appex", "prefpane", "saver",
];

fn unique() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{n:x}")
}

/// Exclusive lock on the addons folder's temp area, shared by every copy of the
/// app. The OS drops it if the process dies, so it cannot go stale. `wait` false
/// returns None when another copy holds it.
fn lock_tmp(addons_dir: &Path, wait: bool) -> Option<fs::File> {
    let dir = addons_dir.join(TMP_DIR);
    fs::create_dir_all(&dir).ok()?;
    let file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(dir.join("lock")).ok()?;
    if wait {
        file.lock().ok()?;
    } else if file.try_lock().is_err() {
        return None;
    }
    Some(file)
}

/// Run at startup. An install interrupted by a crash leaves the folders it
/// replaced in `<tmp>/<id>/old`. Put back each one whose place is now empty;
/// any other backup is moved to `<tmp>/recovered` so nothing is deleted. A work
/// folder is only removed once all its backups have been moved out.
pub fn clean_tmp(addons_dir: &Path) {
    // Another copy of the app may be mid-install; its folders are not abandoned.
    let Some(_guard) = lock_tmp(addons_dir, false) else {
        crate::logi!("another AddonForge is installing; skipping recovery");
        return;
    };
    let tmp = addons_dir.join(TMP_DIR);
    let Ok(entries) = fs::read_dir(&tmp) else { return };
    let recovered = tmp.join("recovered");
    for entry in entries.flatten() {
        let work = entry.path();
        if !work.is_dir() || work == recovered {
            continue;
        }
        let mut all_moved = true;
        if let Ok(backups) = fs::read_dir(work.join("old")) {
            for backup in backups.flatten() {
                let name = backup.file_name();
                let target = addons_dir.join(&name);
                let moved = if target.exists() {
                    fs::create_dir_all(&recovered).is_ok()
                        && fs::rename(backup.path(), recovered.join(format!("{}-{}", entry.file_name().to_string_lossy(), name.to_string_lossy()))).is_ok()
                } else {
                    fs::rename(backup.path(), &target).is_ok()
                };
                crate::logi!("recovered interrupted install: {} ({})", name.to_string_lossy(), if target.exists() { "kept as a backup" } else { "restored" });
                all_moved &= moved;
            }
        }
        if all_moved {
            let _ = fs::remove_dir_all(&work);
        }
    }
}

/// Extract with limits, so a small archive cannot fill the disk. Entries that
/// would leave the destination are refused.
fn extract_limited(archive: &mut zip::ZipArchive<fs::File>, dest: &Path) -> anyhow::Result<()> {
    const MAX_ENTRIES: usize = 20_000;
    const MAX_BYTES: u64 = 1 << 30; // 1 GiB expanded
    use std::io::Read as _;
    if archive.len() > MAX_ENTRIES {
        anyhow::bail!("archive has {} files, more than the {MAX_ENTRIES} allowed", archive.len());
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let rel = entry.enclosed_name().ok_or_else(|| anyhow!("unsafe path in archive"))?;
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let remaining = MAX_BYTES - total;
        let mut file = fs::File::create(&out)?;
        let written = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut file)?;
        if written > remaining {
            anyhow::bail!("archive expands past the {} MB limit", MAX_BYTES >> 20);
        }
        total += written;
    }
    Ok(())
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

/// The folder name WoW expects: the one its `.toc` is named after. A GitHub
/// source archive unpacks as `GatherLite-7.6.3/GatherLite.toc`, so the folder
/// has to become `GatherLite`. Multi-TOC addons (`X.toc`, `X_Forever.toc`)
/// resolve to the shortest stem.
fn canonical_name(dir: &Path) -> Option<String> {
    let current = dir.file_name()?.to_string_lossy().to_string();
    let mut stems: Vec<String> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.to_ascii_lowercase().ends_with(".toc").then(|| n[..n.len() - 4].to_string())
        })
        .collect();
    if stems.is_empty() || stems.iter().any(|s| s.eq_ignore_ascii_case(&current)) {
        return Some(current);
    }
    // Per-flavour TOCs share a base name: `GatherLite_Tbc`, `GatherLite_Vanilla`
    // -> `GatherLite`. Strip a known suffix, then take the most common base.
    const SUFFIXES: &[&str] = &[
        "forever", "camelot", "vanilla", "classic", "era", "anniversary", "tbc", "bcc", "wrath", "wotlk", "wotlkc",
        "cata", "mists", "mop", "standard", "mainline", "retail",
    ];
    let base = |stem: &str| -> String {
        let lower = stem.to_ascii_lowercase();
        for suf in SUFFIXES {
            for sep in ['_', '-'] {
                let tail = format!("{sep}{suf}");
                if lower.ends_with(&tail) && lower.len() > tail.len() {
                    return stem[..stem.len() - tail.len()].to_string();
                }
            }
        }
        stem.to_string()
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for stem in &stems {
        let b = base(stem);
        match counts.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(&b)) {
            Some(entry) => entry.1 += 1,
            None => counts.push((b, 1)),
        }
    }
    counts.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.len().cmp(&y.0.len())).then(x.0.cmp(&y.0)));
    stems.clear();
    Some(counts.remove(0).0)
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
                let lower = e.file_name().to_string_lossy().to_ascii_lowercase();
                if lower.contains('.') && FORBIDDEN_DIR_EXT.contains(&lower.rsplit('.').next().unwrap_or("")) {
                    anyhow::bail!(
                        "SAFETY: refused, the zip contains an application bundle ({}) which no WoW addon needs",
                        p.strip_prefix(root).unwrap_or(&p).display()
                    );
                }
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
            // Windows and macOS also run by "magic": check the first bytes of anything unusual.
            if !matches!(ext, "lua" | "xml" | "toc" | "tga" | "blp" | "png" | "jpg" | "jpeg" | "gif" | "ttf" | "otf" | "mp3" | "ogg" | "wav" | "txt" | "md" | "json" | "html" | "css" | "csv") {
                // Fail closed: a file we can't read (e.g. mode 0o111) can't be vetted.
                let unreadable = || anyhow!("SAFETY: refused, cannot inspect {}", p.strip_prefix(root).unwrap_or(&p).display());
                let mut f = fs::File::open(&p).map_err(|_| unreadable())?;
                let mut head = [0u8; 4];
                use std::io::Read;
                let n = f.read(&mut head).map_err(|_| unreadable())?;
                if n >= 2 && &head[..2] == b"MZ" {
                    anyhow::bail!(
                        "SAFETY: refused, {} is a Windows executable in disguise",
                        p.strip_prefix(root).unwrap_or(&p).display()
                    );
                }
                if n >= 2 && &head[..2] == b"#!" {
                    anyhow::bail!(
                        "SAFETY: refused, {} is a script in disguise",
                        p.strip_prefix(root).unwrap_or(&p).display()
                    );
                }
                // Mach-O 32/64-bit (both byte orders) and universal (32/64-bit) binaries.
                if n == 4
                    && matches!(
                        head,
                        [0xFE, 0xED, 0xFA, 0xCE | 0xCF] | [0xCE | 0xCF, 0xFA, 0xED, 0xFE] | [0xCA, 0xFE, 0xBA, 0xBE | 0xBF]
                    )
                {
                    anyhow::bail!(
                        "SAFETY: refused, {} is a macOS executable in disguise",
                        p.strip_prefix(root).unwrap_or(&p).display()
                    );
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
/// `protected_folders` maps lowercase folder names to their owning package;
/// ownership only prevents replacing folders that are still present on disk.
pub fn place(
    zip_path: &Path,
    addons_dir: &Path,
    protected_folders: &HashMap<String, String>,
) -> anyhow::Result<Vec<String>> {
    fs::create_dir_all(addons_dir)?;
    // Held until this install has finished or rolled back, so no other copy of
    // the app can recover or remove its folders in the meantime.
    let _guard = lock_tmp(addons_dir, true).ok_or_else(|| anyhow!("could not lock the addons folder"))?;
    let work = addons_dir.join(TMP_DIR).join(unique());
    let extract_dir = work.join("x");
    let backup_dir = work.join("old");
    fs::create_dir_all(&extract_dir)?;
    fs::create_dir_all(&backup_dir)?;

    // Set when a rollback could not put every folder back. The backups are then
    // kept in the work folder for clean_tmp to recover on the next start.
    let keep_backups = std::cell::Cell::new(false);

    let result = (|| -> anyhow::Result<Vec<String>> {
        let file = fs::File::open(zip_path)?;
        let mut archive = zip::ZipArchive::new(file).context("not a valid zip")?;
        extract_limited(&mut archive, &extract_dir).context("extract failed")?;
        let files = safety_scan(&extract_dir)?;
        crate::logi!("zip ok: {} files, no executables", files);

        // Compare actual names case-insensitively, including on case-sensitive volumes.
        let on_disk: HashSet<String> = fs::read_dir(addons_dir)?
            .map(|entry| entry.map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()))
            .collect::<std::io::Result<_>>()?;
        // Resolve every final target before moving any installed folder aside.
        let sources: Vec<(PathBuf, String)> = addon_dirs(&extract_dir)?
            .into_iter()
            .map(|src| {
                let original = src.file_name().ok_or_else(|| anyhow!("bad folder name"))?
                    .to_string_lossy().to_string();
                let name = canonical_name(&src).unwrap_or(original);
                if name.starts_with('.') || name.contains("..") {
                    anyhow::bail!("refusing suspicious folder name {name}");
                }
                let lower = name.to_ascii_lowercase();
                if on_disk.contains(&lower) {
                    if let Some(owner) = protected_folders.get(&lower) {
                        anyhow::bail!("cannot replace addon folder {name}: it belongs to {owner}");
                    }
                }
                Ok((src, name))
            })
            .collect::<anyhow::Result<_>>()?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new(); // (target, backup)
        let mut installed = Vec::new();

        let restore = |moved: &Vec<(PathBuf, PathBuf)>| {
            for (target, backup) in moved.iter().rev() {
                let _ = fs::remove_dir_all(target);
                if fs::rename(backup, target).is_err() && backup.exists() {
                    keep_backups.set(true);
                }
            }
        };

        for (src, name) in sources {
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

    if !keep_backups.get() {
        let _ = fs::remove_dir_all(&work);
    } else {
        crate::loge!("install rollback incomplete; kept backups in {} for recovery", work.display());
    }
    let _ = fs::remove_file(zip_path);
    // Leave the tmp root tidy if empty.
    let _ = fs::remove_dir(addons_dir.join(TMP_DIR));
    result
}

/// Delete the given addon folders (used by "Remove").
pub fn remove_folders(addons_dir: &Path, folders: &[String]) -> anyhow::Result<Vec<String>> {
    // Same lock as installs, so a removal cannot race an install in another copy.
    let _guard = lock_tmp(addons_dir, true).ok_or_else(|| anyhow!("could not lock the addons folder"))?;
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

    fn addon_zip(root: &Path, files: &[(&str, &str)]) -> PathBuf {
        fs::create_dir_all(root).unwrap();
        let path = root.join("release.zip");
        let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, contents) in files {
            archive.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(contents.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        path
    }

    #[test]
    fn protected_folder_prevents_changes_to_every_installed_folder() {
        let root = std::env::temp_dir().join(format!("af-owned-{}", unique()));
        let addons = root.join("AddOns");
        for name in ["AAddon", "ZDatabase"] {
            fs::create_dir_all(addons.join(name)).unwrap();
            fs::write(addons.join(name).join("keep.lua"), format!("original {name}")).unwrap();
        }
        let zip = addon_zip(&root, &[
            ("AAddon/AAddon.toc", "## Version: 2\n"),
            ("AAddon/new.lua", "new primary"),
            ("ZDATABASE/ZDatabase.toc", "## Version: 2\n"),
            ("ZDATABASE/new.lua", "new database"),
        ]);
        let protected = HashMap::from([("zdatabase".into(), "github:author/Database".into())]);
        let error = place(&zip, &addons, &protected).unwrap_err().to_string();
        assert!(error.contains("ZDATABASE"), "{error}");
        assert!(error.contains("github:author/Database"), "{error}");
        for name in ["AAddon", "ZDatabase"] {
            assert_eq!(fs::read_to_string(addons.join(name).join("keep.lua")).unwrap(), format!("original {name}"));
            assert!(!addons.join(name).join("new.lua").exists());
            assert!(!addons.join(name).join(format!("{name}.toc")).exists());
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ownership_guard_checks_the_canonical_source_archive_name() {
        let root = std::env::temp_dir().join(format!("af-owned-canon-{}", unique()));
        let addons = root.join("AddOns");
        fs::create_dir_all(addons.join("SharedData")).unwrap();
        fs::write(addons.join("SharedData").join("keep.lua"), "keep database").unwrap();
        let zip = addon_zip(&root, &[
            ("Project-1.2/SharedData_Forever.toc", "## Version: 2\n"),
        ]);
        let protected = HashMap::from([("shareddata".into(), "folder:SharedData".into())]);
        let error = place(&zip, &addons, &protected).unwrap_err().to_string();
        assert!(error.contains("SharedData"), "{error}");
        assert!(error.contains("folder:SharedData"), "{error}");
        assert_eq!(fs::read_to_string(addons.join("SharedData").join("keep.lua")).unwrap(), "keep database");
        assert!(!addons.join("SharedData").join("SharedData_Forever.toc").exists());
        assert!(!addons.join("Project-1.2").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_protected_folder_can_be_installed_but_is_protected_once_present() {
        let root = std::env::temp_dir().join(format!("af-owned-missing-{}", unique()));
        let addons = root.join("AddOns");
        let protected = HashMap::from([("shareddata".into(), "github:author/Database".into())]);
        let zip = addon_zip(&root, &[
            ("Project-1.2/SharedData_Forever.toc", "## Version: 1\n"),
            ("Project-1.2/data.lua", "original database"),
        ]);
        let installed = place(&zip, &addons, &protected).unwrap();
        assert_eq!(installed, vec!["SharedData"]);
        assert_eq!(fs::read_to_string(addons.join("SharedData").join("SharedData_Forever.toc")).unwrap(), "## Version: 1\n");
        assert_eq!(fs::read_to_string(addons.join("SharedData").join("data.lua")).unwrap(), "original database");
        assert!(!addons.join("Project-1.2").exists());

        fs::create_dir_all(addons.join("AAddon")).unwrap();
        fs::write(addons.join("AAddon").join("keep.lua"), "original primary").unwrap();
        let zip = addon_zip(&root, &[
            ("AAddon/AAddon.toc", "## Version: 2\n"),
            ("AAddon/new.lua", "new primary"),
            ("Project-2.0/SharedData_Forever.toc", "## Version: 2\n"),
            ("Project-2.0/data.lua", "new database"),
        ]);
        let error = place(&zip, &addons, &protected).unwrap_err().to_string();
        assert!(error.contains("SharedData"), "{error}");
        assert!(error.contains("github:author/Database"), "{error}");
        assert_eq!(fs::read_to_string(addons.join("AAddon").join("keep.lua")).unwrap(), "original primary");
        assert!(!addons.join("AAddon").join("new.lua").exists());
        assert!(!addons.join("AAddon").join("AAddon.toc").exists());
        assert_eq!(fs::read_to_string(addons.join("SharedData").join("SharedData_Forever.toc")).unwrap(), "## Version: 1\n");
        assert_eq!(fs::read_to_string(addons.join("SharedData").join("data.lua")).unwrap(), "original database");
        assert!(!addons.join("Project-2.0").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn addon_folders_are_installed_under_canonical_names() {
        let root = std::env::temp_dir().join(format!("af-folders-{}", unique()));
        let addons = root.join("AddOns");
        fs::create_dir_all(addons.join("Core")).unwrap();
        fs::write(addons.join("Core").join("old.lua"), "old primary").unwrap();
        let zip = addon_zip(&root, &[
            ("Bundle-1.2/Core-2.0/Core_Forever.toc", "## Version: 2\n"),
            ("Bundle-1.2/Core-2.0/new.lua", "new primary"),
            ("Bundle-1.2/Data/Data.toc", "## Version: 3\n"),
            ("Bundle-1.2/Data/new.lua", "new database"),
        ]);
        let mut installed = place(&zip, &addons, &HashMap::new()).unwrap();
        installed.sort();
        assert_eq!(installed, vec!["Core", "Data"]);
        assert_eq!(fs::read_to_string(addons.join("Core").join("Core_Forever.toc")).unwrap(), "## Version: 2\n");
        assert_eq!(fs::read_to_string(addons.join("Data").join("Data.toc")).unwrap(), "## Version: 3\n");
        assert_eq!(fs::read_to_string(addons.join("Core").join("new.lua")).unwrap(), "new primary");
        assert_eq!(fs::read_to_string(addons.join("Data").join("new.lua")).unwrap(), "new database");
        assert!(!addons.join("Core").join("old.lua").exists());
        assert!(!addons.join("Core-2.0").exists());
        assert!(!addons.join("Bundle-1.2").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// ForeverSVFix puts a directory link inside addon folders that points at
    /// the user's SavedVariables. Replacing or cleaning up an addon folder must
    /// remove the link, never what it points to.
    #[test]
    #[cfg(windows)]
    fn removing_a_folder_never_follows_a_junction() {
        let root = std::env::temp_dir().join(format!("af-junction-{}", unique()));
        let saved = root.join("WTF").join("SavedVariables");
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("Addon.lua"), "keep me").unwrap();
        let addon = root.join("AddOns").join("Addon");
        fs::create_dir_all(&addon).unwrap();
        let link = addon.join("ForeverSVFixData");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link.to_string_lossy(), &saved.to_string_lossy()])
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink failed: {}", String::from_utf8_lossy(&made.stderr));
        assert!(link.join("Addon.lua").exists(), "junction should expose the file");
        fs::remove_dir_all(&addon).unwrap();
        assert!(!addon.exists());
        assert!(saved.join("Addon.lua").exists(), "the linked SavedVariables file must survive");
        let _ = fs::remove_dir_all(&root);
    }

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

    #[test]
    fn source_archive_folder_takes_its_toc_name() {
        let root = std::env::temp_dir().join(format!("af-canon-{}", unique()));
        let wrapped = root.join("GatherLite-7.6.3");
        fs::create_dir_all(&wrapped).unwrap();
        fs::write(wrapped.join("GatherLite.toc"), "## Interface: 16001\n").unwrap();
        fs::write(wrapped.join("GatherLite_Forever.toc"), "## Interface: 16001\n").unwrap();
        assert_eq!(canonical_name(&wrapped).as_deref(), Some("GatherLite"));
        // Only per-flavour TOCs, no plain one (GatherLite 7.6.3 on GitHub).
        let flavours = root.join("GatherLite-7.6.4");
        fs::create_dir_all(&flavours).unwrap();
        for f in ["GatherLite_Cata.toc", "GatherLite_Tbc.toc", "GatherLite_Vanilla.toc", "GatherLite_Wrath.toc"] {
            fs::write(flavours.join(f), "").unwrap();
        }
        assert_eq!(canonical_name(&flavours).as_deref(), Some("GatherLite"));
        let proper = root.join("BigWigs");
        fs::create_dir_all(&proper).unwrap();
        fs::write(proper.join("BigWigs.toc"), "").unwrap();
        fs::write(proper.join("BigWigs_Forever.toc"), "").unwrap();
        assert_eq!(canonical_name(&proper).as_deref(), Some("BigWigs"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn safety_scan_refuses_mach_o() {
        let root = std::env::temp_dir().join(format!("af-macho-{}", unique()));
        let addon = root.join("Evil");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("Evil.toc"), "## Interface: 16001\n").unwrap();
        // 64-bit Mach-O header behind a harmless-looking extension.
        fs::write(addon.join("data.bin"), [0xCF, 0xFA, 0xED, 0xFE, 0x07, 0x00]).unwrap();
        let err = safety_scan(&root).unwrap_err().to_string();
        assert!(err.contains("macOS executable"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn safety_scan_refuses_shebang_and_app_bundle() {
        let root = std::env::temp_dir().join(format!("af-mac-{}", unique()));
        let addon = root.join("Evil");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("Evil.toc"), "## Interface: 16001\n").unwrap();
        fs::write(addon.join("run"), "#!/bin/sh\necho hi\n").unwrap();
        let err = safety_scan(&root).unwrap_err().to_string();
        assert!(err.contains("script in disguise"), "{err}");
        fs::remove_file(addon.join("run")).unwrap();
        fs::create_dir_all(addon.join("Helper.app").join("Contents")).unwrap();
        let err = safety_scan(&root).unwrap_err().to_string();
        assert!(err.contains("application bundle"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("af-recovery-{name}-{}", unique()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn interrupted_install_puts_back_the_old_folder() {
        let addons = scratch("restore");
        // A crash after the old folder was moved aside and before the new one landed.
        let old = addons.join(TMP_DIR).join("work1").join("old").join("Questie");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("Questie.lua"), "keep me").unwrap();
        clean_tmp(&addons);
        assert_eq!(fs::read_to_string(addons.join("Questie").join("Questie.lua")).unwrap(), "keep me");
        assert!(!addons.join(TMP_DIR).join("work1").exists());
    }

    #[test]
    fn interrupted_install_keeps_old_folder_when_a_new_one_exists() {
        let addons = scratch("keep");
        fs::create_dir_all(addons.join("Questie")).unwrap();
        fs::write(addons.join("Questie").join("new.lua"), "new").unwrap();
        let old = addons.join(TMP_DIR).join("work2").join("old").join("Questie");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("old.lua"), "old").unwrap();
        clean_tmp(&addons);
        // The new install stays in place, and the old folder is kept for recovery.
        assert!(addons.join("Questie").join("new.lua").exists());
        assert!(addons.join(TMP_DIR).join("recovered").join("work2-Questie").join("old.lua").exists());
    }

    #[test]
    fn extraction_refuses_paths_that_leave_the_folder() {
        let dir = scratch("zip");
        let zip_path = dir.join("evil.zip");
        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("../escape.txt", opts).unwrap();
            std::io::Write::write_all(&mut w, b"nope").unwrap();
            w.finish().unwrap();
        }
        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        assert!(extract_limited(&mut archive, &out).is_err());
        assert!(!dir.join("escape.txt").exists());
    }

    #[test]
    fn recovery_waits_while_another_copy_holds_the_lock() {
        let addons = scratch("lock");
        let old = addons.join(TMP_DIR).join("work3").join("old").join("Atlas");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("Atlas.lua"), "mid install").unwrap();
        let _held = lock_tmp(&addons, true).unwrap();
        clean_tmp(&addons);
        // Nothing was moved: the other copy is still installing.
        assert!(old.join("Atlas.lua").exists());
        assert!(!addons.join("Atlas").exists());
    }

    #[test]
    fn removal_waits_for_the_install_lock() {
        let addons = scratch("remove-lock");
        let addon = addons.join("Atlas");
        fs::create_dir_all(&addon).unwrap();
        fs::write(addon.join("Atlas.lua"), "x").unwrap();
        // Another copy is installing: it holds the lock.
        let held = lock_tmp(&addons, true).unwrap();
        let (dir, folders) = (addons.clone(), vec!["Atlas".to_string()]);
        let worker = std::thread::spawn(move || remove_folders(&dir, &folders));
        std::thread::sleep(std::time::Duration::from_millis(300));
        // The removal must still be waiting, not deleting under the install.
        assert!(addon.exists());
        drop(held);
        let gone = worker.join().unwrap().unwrap();
        assert_eq!(gone, vec!["Atlas".to_string()]);
        assert!(!addon.exists());
    }
}

