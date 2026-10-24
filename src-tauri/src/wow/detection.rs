//! WoW installation discovery from local Battle.net metadata.
//!
//! Reads the installation database, config, and conventional paths. Validates
//! client executables and identifies Retail, Classic, and Forever installations,
//! deduplicated by canonical path.
//!
//! Everything here is local disk only. No network, settings writes, or client selection.

use super::{read_flavor_info, root_dirs, WowInstall};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Return existing WoW clients with `flavor` set to retail, classic, or forever.
/// Reads Battle.net's database, then config and conventional paths; never writes
/// settings or selects a client.
pub fn detect_installations() -> Vec<WowInstall> {
    let mut candidates = Vec::new();
    let database = if cfg!(target_os = "macos") {
        Some(PathBuf::from("/Users/Shared/Battle.net/Agent/product.db"))
    } else {
        std::env::var_os("PROGRAMDATA")
            .map(|path| PathBuf::from(path).join("Battle.net/Agent/product.db"))
    };
    if let Some(bytes) = database.as_deref().and_then(read_metadata) {
        candidates.extend(database_paths(&bytes));
    }
    if let Some(bytes) = dirs::config_dir()
        .map(|path| path.join("Battle.net/Battle.net.config"))
        .as_deref()
        .and_then(read_metadata)
    {
        if let Ok(config) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(path) = config
                .pointer("/Client/Install/DefaultInstallPath")
                .and_then(|p| p.as_str())
            {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    candidates.push((path.join("World of Warcraft"), None));
                    candidates.push((path, None));
                }
            }
        }
    }
    candidates.extend(root_dirs().into_iter().map(|path| (path, None)));
    clients_at(candidates)
}

fn read_metadata(path: &Path) -> Option<Vec<u8>> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= LIMIT).then_some(bytes)
}

// Only length-delimited protobuf fields are needed. Skip scalar fields, reject
// truncated/overflowing data, and descend only through the known message tags.
fn varint(bytes: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = bytes.split_first()?;
        *bytes = rest;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn fields(mut bytes: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    let mut result = Vec::new();
    while !bytes.is_empty() {
        let tag = varint(&mut bytes)?;
        if tag >> 3 == 0 || tag >> 3 > 0x1fff_ffff {
            return None;
        }
        let length = match tag & 7 {
            0 => {
                varint(&mut bytes)?;
                continue;
            }
            1 => 8,
            2 => usize::try_from(varint(&mut bytes)?).ok()?,
            5 => 4,
            _ => return None,
        };
        let payload = bytes.get(..length)?;
        bytes = bytes.get(length..)?;
        if tag & 7 == 2 {
            result.push((tag >> 3, payload));
        }
    }
    Some(result)
}

fn field<'a>(fields: &[(u64, &'a [u8])], tag: u64) -> Option<&'a [u8]> {
    fields
        .iter()
        .rev()
        .find(|(number, _)| *number == tag)
        .map(|(_, bytes)| *bytes)
}

fn text_field<'a>(fields: &[(u64, &'a [u8])], tag: u64) -> Option<&'a str> {
    std::str::from_utf8(field(fields, tag)?).ok()
}

fn database_paths(bytes: &[u8]) -> Vec<(PathBuf, Option<String>)> {
    let mut paths = Vec::new();
    for (_, product) in fields(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter(|(tag, _)| *tag == 1)
    {
        let Some(product) = fields(product) else {
            continue;
        };
        let code = text_field(&product, 2)
            .filter(|s| client_type(s).is_some())
            .or_else(|| text_field(&product, 1).filter(|s| client_type(s).is_some()));
        let family = text_field(&product, 6).unwrap_or_default();
        if family != "wow" && !(family.is_empty() && code.is_some()) {
            continue;
        }
        let Some(settings) = field(&product, 3).and_then(fields) else {
            continue;
        };
        let Some(path) = text_field(&settings, 1) else {
            continue;
        };
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            continue;
        }
        let folder = text_field(&settings, 13).unwrap_or_default();
        let code = code.or_else(|| folder_product(folder)).map(str::to_owned);
        paths.push((path.clone(), code.clone()));
        if folder_product(folder).is_some() {
            paths.push((path.join(folder), code));
        }
    }
    paths
}

fn folder_product(folder: &str) -> Option<&'static str> {
    match folder {
        "_retail_" => Some("wow"),
        "_ptr_" => Some("wow_ptr"),
        "_xptr_" => Some("wow_xptr"),
        "_beta_" => Some("wow_beta"),
        "_classic_" => Some("wow_classic"),
        "_classic_ptr_" => Some("wow_classic_ptr"),
        "_classic_era_" => Some("wow_classic_era"),
        "_classic_era_ptr_" => Some("wow_classic_era_ptr"),
        "_anniversary_" => Some("wow_anniversary"),
        "_classic_beta_" => Some("wow_classic_beta"),
        "_forever_" => Some("wow_forever"),
        _ => None,
    }
}

fn client_type(product: &str) -> Option<&'static str> {
    if product == "wow_classic_beta"
        || (product.starts_with("wow_") && product.split('_').any(|s| s == "forever"))
    {
        Some("forever")
    } else if matches!(
        product,
        "wow_classic"
            | "wow_classic_ptr"
            | "wow_classic_era"
            | "wow_classic_era_ptr"
            | "wow_anniversary"
            | "wow_anniversary_ptr"
    ) {
        Some("classic")
    } else if matches!(
        product,
        "wow" | "wowt" | "wow_beta" | "wow_ptr" | "wow_xptr"
    ) {
        Some("retail")
    } else {
        None
    }
}

fn executable_present(path: &Path) -> bool {
    if [
        "Wow.exe",
        "WowB.exe",
        "WowT.exe",
        "WowClassic.exe",
        "WowClassicB.exe",
        "WowClassicT.exe",
        "Wow-arm64.exe",
        "WowB-arm64.exe",
        "WowT-arm64.exe",
        "WowClassic-arm64.exe",
        "WowClassicB-arm64.exe",
        "WowClassicT-arm64.exe",
    ]
    .iter()
    .any(|name| path.join(name).is_file())
    {
        return true;
    }
    [
        "World of Warcraft.app",
        "World of Warcraft Beta.app",
        "World of Warcraft Test.app",
        "World of Warcraft Classic.app",
        "World of Warcraft Classic Beta.app",
        "World of Warcraft Classic Test.app",
    ]
    .iter()
    .any(|name| {
        fs::read_dir(path.join(name).join("Contents/MacOS"))
            .is_ok_and(|files| files.flatten().any(|file| file.path().is_file()))
    })
}

fn client_at(path: &Path, hint: Option<&str>) -> Option<WowInstall> {
    let folder = path.file_name()?.to_str()?.to_ascii_lowercase();
    let marker = read_flavor_info(path);
    let product = if folder == "_forever_" {
        Some("wow_forever")
    } else {
        marker
            .as_deref()
            .filter(|code| client_type(code).is_some())
            .or_else(|| folder_product(&folder))
            .or(hint)
    }?;
    let flavor = client_type(product)?.to_owned();
    if !executable_present(path) {
        return None;
    }
    let label = super::product_flavor(product).1;
    let product = marker.as_deref().unwrap_or(product).to_owned();
    Some(WowInstall {
        path: readable_path(&fs::canonicalize(path).ok()?),
        product,
        flavor,
        label,
    })
}

/// Return ordinary drive/UNC paths while retaining canonical keys for deduplication.
fn readable_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = path
        .strip_prefix(r"\\?\")
        .filter(|drive| matches!(drive.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic()))
    {
        drive.to_owned()
    } else {
        path.into_owned()
    }
}

fn clients_at(candidates: Vec<(PathBuf, Option<String>)>) -> Vec<WowInstall> {
    let mut seen = HashSet::new();
    let mut scanned = HashSet::new();
    let mut found = Vec::new();
    let mut check = |path: &Path, hint: Option<&str>| {
        let Ok(key) = fs::canonicalize(path) else {
            return false;
        };
        if seen.contains(&key) {
            return true;
        }
        let Some(install) = client_at(path, hint) else {
            return false;
        };
        seen.insert(key);
        found.push(install);
        true
    };
    for (root, hint) in candidates {
        let is_client = check(&root, hint.as_deref())
            || root
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(folder_product)
                .is_some();
        let parent = if is_client { root.parent() } else { None };
        for directory in std::iter::once(root.as_path()).chain(parent) {
            let Ok(key) = fs::canonicalize(directory) else {
                continue;
            };
            if !scanned.insert(key) {
                continue;
            }
            if directory != root.as_path() {
                check(directory, None);
            }
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    check(&path, None);
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_paths_use_ordinary_drive_and_unc_forms() {
        for (canonical, expected) in [
            (
                r"\\?\D:\Games\World of Warcraft\_forever_",
                r"D:\Games\World of Warcraft\_forever_",
            ),
            (
                r"\\?\UNC\server\games\_forever_",
                r"\\server\games\_forever_",
            ),
            (r"D:\Games\_forever_", r"D:\Games\_forever_"),
            (r"\\server\games\_forever_", r"\\server\games\_forever_"),
            (
                "/Applications/World of Warcraft/_forever_",
                "/Applications/World of Warcraft/_forever_",
            ),
            (
                r"\\?\Volume{example}\_forever_",
                r"\\?\Volume{example}\_forever_",
            ),
            (r"\\?\D:relative", r"\\?\D:relative"),
        ] {
            assert_eq!(readable_path(Path::new(canonical)), expected);
        }
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("af-detect-{}-{unique}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn client(&self, folder: &str, executable: &str) -> PathBuf {
            let path = self.0.join(folder);
            fs::create_dir_all(path.join(executable).parent().unwrap()).unwrap();
            fs::write(path.join(executable), []).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn encoded(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![tag << 3 | 2];
        let mut length = payload.len();
        while length > 127 {
            bytes.push((length as u8 & 127) | 128);
            length >>= 7;
        }
        bytes.push(length as u8);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn database(path: &Path, family: &str) -> Vec<u8> {
        let mut settings = encoded(1, path.to_str().unwrap().as_bytes());
        settings.extend(encoded(13, b"_forever_"));
        let mut product = encoded(2, b"wow_forever");
        product.extend(encoded(3, &settings));
        product.extend(encoded(6, family.as_bytes()));
        encoded(1, &product)
    }

    #[test]
    fn custom_database_location_returns_live_beta_retail_and_classic_once() {
        let fixture = Fixture::new();
        fixture.client("Custom location/_forever_", "Wow.exe");
        fixture.client("Custom location/_classic_beta_", "WowB.exe");
        fixture.client("Custom location/_retail_", "Wow.exe");
        fixture.client("Custom location/_classic_era_", "WowClassic.exe");
        // An incomplete client with addon folders must not be returned.
        fs::create_dir_all(fixture.0.join("Custom location/_classic_/Interface/AddOns")).unwrap();
        let root = fixture.0.join("Custom location");
        let mut candidates = database_paths(&database(&root, "wow"));
        candidates.push((root.join("."), None));
        let found = clients_at(candidates);
        assert_eq!(found.len(), 4);
        assert_eq!(found.iter().filter(|i| i.flavor == "forever").count(), 2);
        assert!(found.iter().any(|i| i.flavor == "retail"));
        assert!(found.iter().any(|i| i.flavor == "classic"));
        assert!(found.iter().all(|i| Path::new(&i.path).is_dir()));
        assert!(database_paths(&database(&root, "agent")).is_empty());
    }

    #[test]
    fn corrupt_database_is_ignored_and_fallback_candidates_still_work() {
        for bytes in [&[0x0a, 0xff][..], &[0][..], &[0x0b][..], &[0x09, 0][..]] {
            assert!(fields(bytes).is_none());
            assert!(database_paths(bytes).is_empty());
        }
        assert!(fields(&[0x80; 10]).is_none());
        let fixture = Fixture::new();
        let fallback = fixture.client("_forever_", "Wow.exe");
        let mut candidates = database_paths(&[0x0a, 0xff]);
        candidates.push((fallback, None));
        assert_eq!(clients_at(candidates).len(), 1);
        // Unknown varint, fixed64, fixed32 and length fields are skipped safely.
        let mut bytes = vec![0x78, 1, 0x79];
        bytes.extend([0; 8]);
        bytes.push(0x7d);
        bytes.extend([0; 4]);
        bytes.extend(encoded(15, b"unused"));
        bytes.extend(database(&fixture.0, "wow"));
        assert!(!database_paths(&bytes).is_empty());
    }

    #[test]
    fn direct_custom_named_client_includes_siblings_and_accepts_arm_executables() {
        let fixture = Fixture::new();
        let direct = fixture.client("Renamed client", "WowB-arm64.exe");
        fs::write(
            direct.join(".flavor.info"),
            "Product Flavor!STRING:0\nwow_classic_beta\n",
        )
        .unwrap();
        fixture.client("_retail_", "Wow-arm64.exe");
        let found = clients_at(database_paths(&database(&direct, "wow")));
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|i| i.flavor == "forever"));
        assert!(found.iter().any(|i| i.flavor == "retail"));
        assert_eq!(client_type("wow_classicsoup"), None);
    }

    #[test]
    fn product_hint_does_not_leak_to_parent_or_siblings() {
        let fixture = Fixture::new();
        let parent = fixture.client("Shared", "Wow.exe");
        let direct = fixture.client("Shared/Renamed client", "WowB.exe");
        fixture.client("Shared/Unmarked sibling", "Wow.exe");
        fixture.client("Shared/_retail_", "Wow.exe");
        let found = clients_at(vec![(direct, Some("wow_classic_beta".into()))]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|i| i.flavor == "forever"));
        assert!(found.iter().any(|i| i.flavor == "retail"));
        let parent = readable_path(&fs::canonicalize(parent).unwrap());
        assert!(found.iter().all(|i| i.path != parent));
    }

    #[test]
    fn later_explicit_hint_can_identify_an_already_scanned_client() {
        let fixture = Fixture::new();
        let client = fixture.client("Renamed client", "WowClassic.exe");
        let found = clients_at(vec![
            (fixture.0.clone(), None),
            (client.clone(), None),
            (client.clone(), Some("wow_classic".into())),
            (client.join("."), Some("wow_classic".into())),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].flavor, "classic");
    }

    #[test]
    fn mac_requires_a_file_inside_the_bundle_and_forever_folder_overrides_marker() {
        let fixture = Fixture::new();
        let path = fixture.0.join("_forever_");
        fs::create_dir_all(path.join("World of Warcraft.app/Contents/MacOS")).unwrap();
        assert!(client_at(&path, None).is_none());
        fixture.client(
            "_forever_",
            "World of Warcraft.app/Contents/MacOS/World of Warcraft",
        );
        fs::write(
            path.join(".flavor.info"),
            "Product Flavor!STRING:0\nwow_unknown\n",
        )
        .unwrap();
        let found = client_at(&path, None).unwrap();
        assert_eq!(found.flavor, "forever");
        assert_eq!(found.product, "wow_unknown");
        assert_eq!(found.label, "WoW: Forever");
    }
}
