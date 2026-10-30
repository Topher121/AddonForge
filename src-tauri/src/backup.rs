//! Zip a folder the user cares about (the WTF settings folder) somewhere
//! safe. Read-only on the source; symlinks and junctions are skipped so a
//! ForeverSVFix link can never pull foreign data into the archive.

use std::fs;
use std::io::Write;
use std::path::Path;

pub struct Summary {
    pub files: u64,
    pub bytes: u64,
}

/// Write the archive to a temporary file and move it into place only when it is
/// complete. A failed backup never replaces an existing good archive.
pub fn zip_dir(src: &Path, dest: &Path) -> anyhow::Result<Summary> {
    anyhow::ensure!(src.is_dir(), "{} is not a folder", src.display());
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension("zip.part");
    match write_zip(src, &part) {
        Ok(summary) => {
            fs::rename(&part, dest)?;
            Ok(summary)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn write_zip(src: &Path, dest: &Path) -> anyhow::Result<Summary> {
    let file = fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut summary = Summary { files: 0, bytes: 0 };
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&dir)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = path.strip_prefix(src)?.to_string_lossy().replace('\\', "/");
            if meta.is_dir() {
                zip.add_directory(format!("{rel}/"), opts)?;
                stack.push(path);
            } else if meta.is_file() {
                zip.start_file(rel, opts)?;
                let bytes = fs::read(&path)?;
                zip.write_all(&bytes)?;
                summary.files += 1;
                summary.bytes += bytes.len() as u64;
            }
        }
    }
    zip.finish()?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_backup_keeps_the_previous_archive() {
        let tmp = std::env::temp_dir().join(format!("af-backup-keep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let src = tmp.join("WTF");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("Config.wtf"), "SET x 1").unwrap();
        let dest = tmp.join("WTF-good.zip");
        zip_dir(&src, &dest).unwrap();
        let good = fs::read(&dest).unwrap();
        // The source is not a folder any more, so the new backup must fail and leave the old one alone.
        fs::remove_dir_all(&src).unwrap();
        assert!(zip_dir(&src, &dest).is_err());
        assert_eq!(fs::read(&dest).unwrap(), good);
        assert!(!dest.with_extension("zip.part").exists());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn zips_nested_files_and_skips_links() {
        let tmp = std::env::temp_dir().join(format!("af-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let src = tmp.join("WTF");
        fs::create_dir_all(src.join("Account/ME/SavedVariables")).unwrap();
        fs::write(src.join("Config.wtf"), "SET x 1").unwrap();
        fs::write(src.join("Account/ME/SavedVariables/Questie.lua"), "QuestieDB = {}").unwrap();
        #[cfg(windows)]
        {
            let outside = tmp.join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("secret.txt"), "no").unwrap();
            // A junction inside WTF (what ForeverSVFix-style tools create). Needs no privilege.
            let _ = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J", &src.join("Link").to_string_lossy(), &outside.to_string_lossy()])
                .output();
        }
        let dest = tmp.join("out.zip");
        let s = zip_dir(&src, &dest).unwrap();
        assert_eq!(s.files, 2);
        let mut archive = zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        assert!(names.contains(&"Config.wtf".to_string()));
        assert!(names.contains(&"Account/ME/SavedVariables/Questie.lua".to_string()));
        assert!(!names.iter().any(|n| n.contains("secret")), "junction contents must not be archived: {names:?}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
