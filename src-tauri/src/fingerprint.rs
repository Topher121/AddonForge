//! CurseForge folder fingerprints, computed locally from the addon's own files.
//!
//! This is how the CurseForge app recognises addons it installed: each file
//! the addon loads (its TOCs, Bindings.xml, and every .lua/.xml they pull in)
//! is hashed with MurmurHash2 (seed 1) after removing whitespace bytes, the
//! hashes are sorted and joined as decimal text, and that text is hashed again.
//! Nothing here touches the network; `sources::curseforge::match_fingerprints`
//! sends the resulting numbers in one request.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// TOC flavour suffixes counted as part of a folder (case-insensitive).
const TOC_SUFFIXES: &[&str] = &[
    "", "mainline", "bcc", "tbc", "classic", "vanilla", "wrath", "wotlkc", "cata", "mists",
    "forever", "camelot",
];
/// Files bigger than this are skipped rather than read (no real addon file is).
const MAX_FILE: u64 = 64 * 1024 * 1024;
/// A folder that pulls in more files than this is not hashed at all.
const MAX_FILES: usize = 20_000;

/// MurmurHash2, 32-bit.
pub fn murmur2(data: &[u8], seed: u32) -> u32 {
    const M: u32 = 0x5bd1_e995;
    let mut h = seed ^ data.len() as u32;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M) ^ k;
    }
    let tail = chunks.remainder();
    if tail.len() >= 3 {
        h ^= (tail[2] as u32) << 16;
    }
    if tail.len() >= 2 {
        h ^= (tail[1] as u32) << 8;
    }
    if !tail.is_empty() {
        h ^= tail[0] as u32;
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^ (h >> 15)
}

/// CurseForge's file hash: MurmurHash2 seed 1 over the bytes minus tab, LF, CR and space.
pub fn normalized_hash(bytes: &[u8]) -> u32 {
    let kept: Vec<u8> = bytes.iter().copied().filter(|b| !matches!(b, 9 | 10 | 13 | 32)).collect();
    murmur2(&kept, 1)
}

fn is_folder_toc(folder: &str, file: &str) -> bool {
    let (f, n) = (folder.to_ascii_lowercase(), file.to_ascii_lowercase());
    let Some(rest) = n.strip_prefix(&f).and_then(|r| r.strip_suffix(".toc")) else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(suffix) = rest.strip_prefix(['-', '_']) else { return false };
    TOC_SUFFIXES.contains(&suffix)
}

/// Paths a TOC loads: lines that are a .lua or .xml file name.
fn toc_includes(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && !l.contains(".."))
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            lower.ends_with(".lua") || lower.ends_with(".xml")
        })
        .map(str::to_string)
        .collect()
}

/// Paths an XML file loads: `<Include file="..."/>` and `<Script file="..."/>`,
/// ignoring anything inside comments.
fn xml_includes(text: &str) -> Vec<String> {
    let mut clean = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        clean.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    clean.push_str(rest);
    let lower = clean.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find('<') {
        let tag_start = at + i + 1;
        let Some(len) = lower[tag_start..].find('>') else { break };
        let tag = &clean[tag_start..tag_start + len];
        let tag_lower = &lower[tag_start..tag_start + len];
        at = tag_start + len;
        let is_ref = (tag_lower.starts_with("include") || tag_lower.starts_with("script"))
            && tag_lower.trim_end().ends_with('/');
        if !is_ref {
            continue;
        }
        let Some(fpos) = tag_lower.find("file") else { continue };
        let after = tag[fpos + 4..].trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(end) = after[1..].find(quote) else { continue };
        let path = &after[1..1 + end];
        if !path.is_empty() && !path.contains("..") {
            out.push(path.to_string());
        }
    }
    out
}

fn resolve(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Hash `path` and everything it includes, depth first, each file once.
fn walk(path: &Path, root: &Path, seen: &mut HashSet<String>, hashes: &mut Vec<u32>) -> Option<()> {
    let key = path.to_string_lossy().to_ascii_lowercase();
    if !seen.insert(key) || seen.len() > MAX_FILES {
        return Some(());
    }
    // Never follow a link or a path outside the addon folder.
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE || !path.starts_with(root) {
        return Some(());
    }
    let bytes = fs::read(path).ok()?;
    hashes.push(normalized_hash(&bytes));
    let lower = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    let includes = match lower.as_deref() {
        Some("toc") => toc_includes(&String::from_utf8_lossy(&bytes)),
        Some("xml") => xml_includes(&String::from_utf8_lossy(&bytes)),
        _ => Vec::new(),
    };
    let dir = path.parent()?.to_path_buf();
    for rel in includes {
        walk(&resolve(&dir, &rel), root, seen, hashes);
    }
    Some(())
}

/// The fingerprint CurseForge knows this addon folder by, or None when the
/// folder has no TOC of its own.
pub fn folder_fingerprint(folder: &Path) -> Option<u32> {
    let name = folder.file_name()?.to_string_lossy().to_string();
    let mut tocs = Vec::new();
    let mut bindings = Vec::new();
    for entry in fs::read_dir(folder).ok()?.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if is_folder_toc(&name, &file) {
            tocs.push(entry.path());
        } else if file.eq_ignore_ascii_case("bindings.xml") {
            bindings.push(entry.path());
        }
    }
    if tocs.is_empty() {
        return None;
    }
    tocs.sort();
    let mut seen = HashSet::new();
    let mut hashes = Vec::new();
    // Bindings.xml is counted but not followed, as CurseForge does.
    for b in bindings {
        if seen.insert(b.to_string_lossy().to_ascii_lowercase()) {
            if let Ok(bytes) = fs::read(&b) {
                hashes.push(normalized_hash(&bytes));
            }
        }
    }
    for toc in &tocs {
        walk(toc, folder, &mut seen, &mut hashes);
    }
    if seen.len() > MAX_FILES || hashes.is_empty() {
        return None;
    }
    hashes.sort_unstable();
    let joined: String = hashes.iter().map(|h| h.to_string()).collect();
    Some(normalized_hash(joined.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur2_matches_known_values() {
        // Reference values from the canonical C implementation (seed 1).
        assert_eq!(murmur2(b"", 1), 0x5bd1_5e36);
        assert_eq!(normalized_hash(b"a b\tc\r\n"), murmur2(b"abc", 1));
    }

    #[test]
    fn toc_names_and_includes() {
        assert!(is_folder_toc("Bagnon", "Bagnon.toc"));
        assert!(is_folder_toc("Bagnon", "bagnon_Vanilla.toc"));
        assert!(is_folder_toc("Bagnon", "Bagnon-Forever.toc"));
        assert!(!is_folder_toc("Bagnon", "Bagnon_Config.toc"));
        assert!(!is_folder_toc("Bagnon", "BagnonX.toc"));
        let toc = "## Title: x\n#core.lua\nmain.lua\n  libs\\embeds.xml  \n../escape.lua\nreadme.txt\n";
        assert_eq!(toc_includes(toc), vec!["main.lua", "libs\\embeds.xml"]);
        let xml = r#"<Ui><!-- <Script file="dead.lua"/> --><Include file="a\b.xml"/>
            <Script  file = 'c.lua' /><Script file="../up.lua"/><Frame file="no.lua"/></Ui>"#;
        assert_eq!(xml_includes(xml), vec!["a\\b.xml", "c.lua"]);
    }

    #[test]
    fn folder_fingerprint_follows_includes_and_ignores_the_rest() {
        let root = std::env::temp_dir().join(format!("af-fp-{}", std::process::id()));
        let dir = root.join("Demo");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(dir.join("libs")).unwrap();
        fs::write(dir.join("Demo.toc"), "## Title: Demo\ncore.lua\nlibs\\load.xml\n").unwrap();
        fs::write(dir.join("core.lua"), "print(1)").unwrap();
        fs::write(dir.join("libs/load.xml"), "<Ui><Script file=\"lib.lua\"/></Ui>").unwrap();
        fs::write(dir.join("libs/lib.lua"), "x=1").unwrap();
        let first = folder_fingerprint(&dir).unwrap();
        // A file nothing loads does not change it; whitespace does not either.
        fs::write(dir.join("notes.txt"), "hello").unwrap();
        fs::write(dir.join("core.lua"), "print(1)\r\n  ").unwrap();
        assert_eq!(folder_fingerprint(&dir), Some(first));
        // A loaded file's content does.
        fs::write(dir.join("libs/lib.lua"), "x=2").unwrap();
        assert_ne!(folder_fingerprint(&dir), Some(first));
        // Expected value computed by hand from the same rules.
        let mut hs = vec![
            normalized_hash(b"## Title: Demo\ncore.lua\nlibs\\load.xml\n"),
            normalized_hash(b"print(1)"),
            normalized_hash(b"<Ui><Script file=\"lib.lua\"/></Ui>"),
            normalized_hash(b"x=2"),
        ];
        hs.sort_unstable();
        let s: String = hs.iter().map(|h| h.to_string()).collect();
        assert_eq!(folder_fingerprint(&dir), Some(normalized_hash(s.as_bytes())));
        assert_eq!(folder_fingerprint(&root.join("missing")), None);
        let _ = fs::remove_dir_all(&root);
    }
}
