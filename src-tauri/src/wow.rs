//! WoW install discovery and addon folder / TOC scanning.
//!
//! Everything here is local disk only. No network.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Interface numbers that mean "WoW: Forever" (1.60.x -> 160xx).
pub const FOREVER_INTERFACE_RANGE: std::ops::RangeInclusive<u32> = 16000..=16999;

/// TOC suffixes the game (and Wago's data endpoint) recognise for Forever.
pub const FOREVER_TOC_SUFFIXES: &[&str] = &["_Forever", "-Forever", "_Camelot", "-Camelot"];

#[derive(Serialize, Clone, Debug)]
pub struct WowInstall {
    /// Absolute path of the product folder, e.g. `...\World of Warcraft\_classic_beta_`.
    pub path: String,
    /// Product code from `.flavor.info`, e.g. `wow_classic_beta`.
    pub product: String,
    /// Our flavour id: `forever` or `other`.
    pub flavor: String,
    /// Human label.
    pub label: String,
}

/// Map Blizzard product codes to the flavour we support.
pub fn product_flavor(product: &str) -> (&'static str, String) {
    match product {
        "wow_classic_beta" => ("forever", "WoW: Forever (beta)".into()),
        "wow" => ("other", "Retail".into()),
        "wow_classic_era" => ("other", "Classic Era".into()),
        "wow_anniversary" => ("other", "Anniversary".into()),
        "wow_classic" => ("other", "Classic (progression)".into()),
        other => ("other", other.to_string()),
    }
}

fn read_flavor_info(product_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(product_dir.join(".flavor.info")).ok()?;
    // Format: "Product Flavor!STRING:0\nwow_classic_beta\n"
    text.lines().nth(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Describe a product folder the user picked or we discovered.
pub fn describe_install(product_dir: &Path) -> Option<WowInstall> {
    if !product_dir.join("Interface").join("AddOns").is_dir()
        && !product_dir.join("Interface").is_dir()
        && !product_dir.join("WTF").is_dir()
    {
        // Not a WoW product folder at all.
        if read_flavor_info(product_dir).is_none() {
            return None;
        }
    }
    let product = read_flavor_info(product_dir).unwrap_or_else(|| {
        // Fall back to the folder name, e.g. "_classic_beta_" -> "classic_beta".
        let name = product_dir
            .file_name()
            .map(|s| s.to_string_lossy().trim_matches('_').to_string())
            .unwrap_or_default();
        format!("wow_{name}")
    });
    let (flavor, label) = product_flavor(&product);
    Some(WowInstall {
        path: product_dir.to_string_lossy().to_string(),
        product,
        flavor: flavor.to_string(),
        label,
    })
}

const ROOT_PATTERNS: &[&str] = &[
    "Program Files (x86)\\World of Warcraft",
    "Program Files\\World of Warcraft",
    "World of Warcraft",
    "Games\\World of Warcraft",
    "Blizzard\\World of Warcraft",
    "Battle.net\\World of Warcraft",
];

/// Scan the usual places for `World of Warcraft\_*_` product folders.
pub fn find_installs() -> Vec<WowInstall> {
    let mut found = Vec::new();
    for letter in b'C'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        if !Path::new(&drive).is_dir() {
            continue;
        }
        for pat in ROOT_PATTERNS {
            let root = PathBuf::from(&drive).join(pat);
            let Ok(rd) = fs::read_dir(&root) else { continue };
            for entry in rd.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if p.is_dir() && name.starts_with('_') && name.ends_with('_') {
                    if let Some(i) = describe_install(&p) {
                        found.push(i);
                    }
                }
            }
        }
    }
    // Forever first, then the rest, stable by path.
    found.sort_by(|a, b| {
        (a.flavor != "forever")
            .cmp(&(b.flavor != "forever"))
            .then(a.path.cmp(&b.path))
    });
    found.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path));
    found
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct AddonFolder {
    pub folder: String,
    pub title: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub notes: Option<String>,
    pub interfaces: Vec<u32>,
    pub wago: Option<String>,
    pub curse: Option<u64>,
    pub wowi: Option<u64>,
    pub website: Option<String>,
    /// True when a `_Forever` / `_Camelot` TOC exists.
    pub forever_toc: bool,
    /// True when any Interface number is in the Forever range.
    pub forever_interface: bool,
    pub dependencies: Vec<String>,
}

impl AddonFolder {
    pub fn supports_forever(&self) -> bool {
        self.forever_toc || self.forever_interface
    }
}

/// Strip WoW colour / texture escape codes from a TOC title.
pub fn clean_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '|' {
            match chars.peek() {
                Some('c') | Some('C') => {
                    chars.next();
                    for _ in 0..8 {
                        chars.next();
                    }
                }
                Some('r') | Some('R') => {
                    chars.next();
                }
                Some('T') | Some('t') => {
                    // |Tpath:size|t  -> skip to the closing |t
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '|' {
                            if let Some('t') | Some('T') = chars.peek() {
                                chars.next();
                                break;
                            }
                        }
                    }
                }
                Some('|') => {
                    chars.next();
                    out.push('|');
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_toc(path: &Path, into: &mut AddonFolder) -> bool {
    let Ok(bytes) = fs::read(path) else { return false };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("##") else { continue };
        let Some((k, v)) = rest.split_once(':') else { continue };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        if val.is_empty() {
            continue;
        }
        match key.as_str() {
            "title" => {
                if into.title.is_empty() {
                    into.title = clean_title(val);
                }
            }
            "version" => {
                if into.version.is_none() {
                    into.version = Some(val.to_string());
                }
            }
            "author" => {
                if into.author.is_none() {
                    into.author = Some(clean_title(val));
                }
            }
            "notes" => {
                if into.notes.is_none() {
                    into.notes = Some(clean_title(val));
                }
            }
            "interface" => {
                for n in val.split(',').filter_map(|s| s.trim().parse::<u32>().ok()) {
                    if !into.interfaces.contains(&n) {
                        into.interfaces.push(n);
                    }
                }
            }
            "x-wago-id" => {
                if into.wago.is_none() {
                    into.wago = Some(val.to_string());
                }
            }
            "x-curse-project-id" => {
                if into.curse.is_none() {
                    into.curse = val.parse().ok();
                }
            }
            "x-wowi-id" => {
                if into.wowi.is_none() {
                    into.wowi = val.parse().ok();
                }
            }
            "x-website" | "x-project" | "x-github" => {
                if into.website.is_none() {
                    into.website = Some(val.to_string());
                }
            }
            "dependencies" | "requireddeps" => {
                for d in val.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    into.dependencies.push(d.to_string());
                }
            }
            _ => {}
        }
    }
    true
}

/// Scan `<install>\Interface\AddOns` and parse each folder's TOC.
pub fn scan_addons(install: &Path) -> Vec<AddonFolder> {
    let addons = install.join("Interface").join("AddOns");
    let Ok(rd) = fs::read_dir(&addons) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.starts_with("Blizzard_") {
            continue;
        }
        let mut af = AddonFolder { folder: name.clone(), ..Default::default() };
        // Forever-specific TOC first so its Interface/Version win.
        for suf in FOREVER_TOC_SUFFIXES {
            let candidate = p.join(format!("{name}{suf}.toc"));
            if candidate.is_file() {
                af.forever_toc = true;
                parse_toc(&candidate, &mut af);
            }
        }
        let generic = p.join(format!("{name}.toc"));
        let mut any = af.forever_toc;
        if generic.is_file() {
            any |= parse_toc(&generic, &mut af);
        }
        if !any {
            // Case-insensitive fallback: any *.toc in the folder.
            if let Ok(inner) = fs::read_dir(&p) {
                for f in inner.flatten() {
                    let fname = f.file_name().to_string_lossy().to_string();
                    if fname.to_ascii_lowercase().ends_with(".toc") {
                        any |= parse_toc(&f.path(), &mut af);
                        let stem = fname.trim_end_matches(".toc").trim_end_matches(".TOC");
                        if FOREVER_TOC_SUFFIXES.iter().any(|s| stem.ends_with(s)) {
                            af.forever_toc = true;
                        }
                    }
                }
            }
        }
        if !any {
            continue; // not an addon folder
        }
        if af.title.is_empty() {
            af.title = name.clone();
        }
        af.forever_interface = af.interfaces.iter().any(|n| FOREVER_INTERFACE_RANGE.contains(n));
        out.push(af);
    }
    out.sort_by(|a, b| a.folder.to_ascii_lowercase().cmp(&b.folder.to_ascii_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(clean_title("|cff33ff99BigWigs|r Boss Mods"), "BigWigs Boss Mods");
        assert_eq!(clean_title("Plain"), "Plain");
        assert_eq!(clean_title("|TInterface\\Icons\\x:16|t Name"), "Name");
    }
}
