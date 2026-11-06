//! Reversible removal: the Recycle Bin on Windows, the Trash on macOS.
//!
//! Used by "Remove" for addons AddonForge did not install. It never put those
//! folders there, so it never knew what was in them, and the honest way to take
//! someone else's folder away is a removal they can undo.
//!
//! Safety: the shell's delete is not our code, so we never hand it a tree that
//! can lead out of AddOns. Every junction / symlink inside a folder is unlinked
//! first (ForeverSVFix puts a `ForeverSVFixData` junction inside addon folders,
//! pointing at the user's SavedVariables). `WTF\` is never a target here.

use std::fs;
use std::path::{Path, PathBuf};

/// Unlink every junction / symlink inside `dir` without following any of them,
/// and report how many were unlinked. A directory entry's own type is read from
/// the directory, so a junction is removed as a link and never walked into.
pub fn unlink_reparse_points(dir: &Path) -> std::io::Result<usize> {
    let mut unlinked = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        // A junction reports as a directory too, so links are checked first.
        if kind.is_symlink() {
            if fs::remove_dir(&path).is_err() {
                fs::remove_file(&path)?;
            }
            unlinked += 1;
        } else if kind.is_dir() {
            unlinked += unlink_reparse_points(&path)?;
        }
    }
    Ok(unlinked)
}

/// What the removal is called on this platform, for dialogs and toasts.
pub fn label() -> &'static str {
    if cfg!(windows) {
        "Recycle Bin"
    } else {
        "Trash"
    }
}

/// Move whole folders to the Recycle Bin / Trash. Paths must be real folders,
/// not links: the caller checks that.
#[cfg(windows)]
pub fn to_trash(paths: &[PathBuf]) -> Result<(), String> {
    windows_shell::delete_to_recycle_bin(paths)
}

/// macOS has no shell call for this without an Objective-C bridge, so the
/// folder is moved into `~/.Trash`, which is what the Trash is. "Put back" in
/// Finder will not know where it came from; the folder itself is intact.
#[cfg(target_os = "macos")]
pub fn to_trash(paths: &[PathBuf]) -> Result<(), String> {
    let trash = dirs::home_dir().ok_or("could not find your home folder")?.join(".Trash");
    fs::create_dir_all(&trash).map_err(|e| format!("could not open the Trash: {e}"))?;
    for path in paths {
        let name = path.file_name().ok_or("unnamed folder")?.to_string_lossy().to_string();
        let mut target = trash.join(&name);
        let mut n = 2;
        while target.exists() {
            target = trash.join(format!("{name} {n}"));
            n += 1;
        }
        fs::rename(path, &target).map_err(|e| {
            format!("could not move {name} to the Trash ({e}); move it there in Finder instead")
        })?;
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn to_trash(_paths: &[PathBuf]) -> Result<(), String> {
    Err("this build cannot move folders to the Trash; remove them in your file manager".into())
}

#[cfg(windows)]
mod windows_shell {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};

    #[repr(C)]
    struct ShFileOpStruct {
        hwnd: *mut c_void,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        any_aborted: i32,
        name_mappings: *mut c_void,
        progress_title: *const u16,
    }

    const FO_DELETE: u32 = 0x0003;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
    const FOF_NOERRORUI: u16 = 0x0400;
    /// Asks Windows to tell the user if it would have to delete outright
    /// instead (a drive with no Recycle Bin). We never want a silent nuke.
    const FOF_WANTNUKEWARNING: u16 = 0x4000;
    /// The shell's own file operation is the MAX_PATH-era API.
    const SHELL_PATH_LIMIT: usize = 258;

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStruct) -> i32;
    }

    /// One shell operation for the whole list: a double-null-terminated set of
    /// full paths, deleted with "allow undo" so they land in the Recycle Bin.
    pub fn delete_to_recycle_bin(paths: &[PathBuf]) -> Result<(), String> {
        let mut from: Vec<u16> = Vec::new();
        for path in paths {
            let full = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
            // canonicalize returns a \\?\ path; the shell API does not take one.
            let text = full.to_string_lossy().to_string();
            let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
            if text.chars().count() > SHELL_PATH_LIMIT {
                return Err(format!(
                    "{text} is too long for Windows to move to the Recycle Bin; delete it in Explorer instead"
                ));
            }
            from.extend(OsStr::new(&text).encode_wide());
            from.push(0);
        }
        if from.is_empty() {
            return Ok(());
        }
        from.push(0);
        let mut op = ShFileOpStruct {
            hwnd: null_mut(),
            func: FO_DELETE,
            from: from.as_ptr(),
            to: null(),
            flags: FOF_ALLOWUNDO
                | FOF_NOCONFIRMATION
                | FOF_NOCONFIRMMKDIR
                | FOF_SILENT
                | FOF_NOERRORUI
                | FOF_WANTNUKEWARNING,
            any_aborted: 0,
            name_mappings: null_mut(),
            progress_title: null(),
        };
        // Safe: `from` outlives the call, `op` is a live exclusive borrow, and
        // the struct matches SHFILEOPSTRUCTW field for field.
        let code = unsafe { SHFileOperationW(&mut op) };
        if code != 0 {
            return Err(format!("Windows would not move it to the Recycle Bin (error {code})"));
        }
        if op.any_aborted != 0 {
            return Err("the move to the Recycle Bin was cancelled".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique() -> String {
        format!(
            "{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )
    }

    #[test]
    #[cfg(windows)]
    fn a_junction_is_unlinked_and_its_target_survives() {
        let root = std::env::temp_dir().join(format!("af-recycle-{}", unique()));
        let addon = root.join("HealBot");
        let saved = root.join("SavedVariables");
        fs::create_dir_all(addon.join("sub")).unwrap();
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("HealBot.lua"), "the user's settings").unwrap();
        fs::write(addon.join("HealBot.toc"), "## Version: 1\n").unwrap();
        let link = addon.join("sub").join("ForeverSVFixData");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &link.to_string_lossy(), &saved.to_string_lossy()])
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink failed: {}", String::from_utf8_lossy(&made.stderr));
        assert!(link.join("HealBot.lua").exists(), "the junction should expose the file");

        assert_eq!(unlink_reparse_points(&addon).unwrap(), 1);
        assert!(!link.exists(), "the junction should be gone");
        assert!(saved.join("HealBot.lua").exists(), "saved variables must survive");
        assert!(addon.join("HealBot.toc").exists(), "the addon's own files are left for the bin");
        let _ = fs::remove_dir_all(&root);
    }

    /// Proves the shell call itself, so it puts a folder in the real Recycle
    /// Bin. Run by hand (`cargo test -- --ignored recycles`), never in CI.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn a_folder_really_goes_to_the_recycle_bin() {
        let root = std::env::temp_dir().join(format!("af-recycle-live-{}", unique()));
        let addon = root.join("HealBot");
        fs::create_dir_all(addon.join("sub")).unwrap();
        fs::write(addon.join("HealBot.toc"), "## Version: 1
").unwrap();
        fs::write(addon.join("sub").join("settings.lua"), "user config").unwrap();
        to_trash(&[addon.clone()]).unwrap();
        assert!(!addon.exists(), "the folder should have left the disk");
        assert!(root.exists(), "only the folder asked for moves");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn walking_a_plain_folder_finds_nothing_to_unlink() {
        let root = std::env::temp_dir().join(format!("af-recycle-plain-{}", unique()));
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::write(root.join("a").join("b").join("c.lua"), "x").unwrap();
        assert_eq!(unlink_reparse_points(&root).unwrap(), 0);
        assert!(root.join("a").join("b").join("c.lua").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
