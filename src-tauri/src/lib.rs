//! AddonForge core: Tauri commands the UI calls over IPC.

mod backup;
mod catalog;
mod icons;
mod install;
pub mod log;
mod selfupdate;
mod signing;
mod sources;
mod state;
mod wow;

use catalog::{Catalog, CatalogEntry, Stats};
use serde::{Deserialize, Serialize};
use sources::{normalize_version, Remote, ResolveOpts, Source};
use state::{AppState, Installed};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tokio::sync::{Mutex, Semaphore};
use wow::{AddonFolder, WowInstall};

pub const USER_AGENT: &str = concat!(
    "AddonForge/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Topher121/AddonForge)"
);
pub const REPO_URL: &str = "https://github.com/Topher121/AddonForge";
pub const SUPPORT_EMAIL: &str = "PocketForgeStudios@proton.me";

struct App {
    client: reqwest::Client,
    state: Mutex<AppState>,
    catalog: Mutex<Option<(Catalog, &'static str)>>,
    stats: Mutex<Option<Stats>>,
    /// icon key -> data URL, for this run (disk cache underneath).
    icon_cache: Mutex<HashMap<String, Option<String>>>,
}

type Shared = Arc<App>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- packages

#[derive(Serialize, Clone, Debug)]
pub struct MissingDep {
    pub folder: String,
    pub catalog_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Package {
    pub key: String,
    pub name: String,
    pub folders: Vec<String>,
    pub installed_version: Option<String>,
    pub author: Option<String>,
    pub notes: Option<String>,
    pub source: Option<Source>,
    pub source_label: String,
    pub source_url: Option<String>,
    pub curse_id: Option<u64>,
    pub catalog_id: Option<String>,
    /// Catalogue `notice` for this addon, shown on its row.
    pub notice: Option<String>,
    pub supports_forever: bool,
    /// True when AddonForge installed this itself (exact version known).
    pub managed: bool,
    pub pinned: bool,
    pub ignored: bool,
    pub missing_deps: Vec<MissingDep>,
    /// `## IconTexture` of the first folder that has one (resolved lazily by `package_icons`).
    pub icon_texture: Option<String>,
    pub remote_version: Option<String>,
    pub remote_forever: Option<bool>,
    pub remote_prerelease: bool,
    pub update_available: bool,
    /// ok | update | unknown | curse-only | no-source | no-key | error | checking
    pub status: String,
    pub note: String,
}

fn addons_dir(install: &str) -> PathBuf {
    Path::new(install).join("Interface").join("AddOns")
}

fn pick_source(
    entry: Option<&CatalogEntry>,
    f: &AddonFolder,
    managed: Option<&Installed>,
) -> Option<Source> {
    // What we installed it from wins: it's what the user chose. Wago is the
    // exception since build 25: its updater keys went Patreon-only, so a
    // package once installed from Wago falls through to any other source.
    if let Some(m) = managed {
        if let Some(s) = Source::from_parts(&m.source, &m.source_id) {
            if !matches!(s, Source::Wago(_)) {
                return Some(s);
            }
        }
    }
    if let Some(e) = entry {
        if let Some(r) = &e.github {
            return Some(Source::Github(r.clone()));
        }
        if sources::curseforge::available() && !e.curse_locked {
            if let Some(c) = e.curse {
                return Some(Source::Curse(c));
            }
        }
        if let Some(i) = e.wowi {
            return Some(Source::Wowi(i));
        }
        if let Some(t) = &e.tukui {
            return Some(Source::Tukui(t.clone()));
        }
    }
    if sources::curseforge::available() {
        if let Some(c) = f.curse {
            return Some(Source::Curse(c));
        }
    }
    if let Some(i) = f.wowi {
        return Some(Source::Wowi(i));
    }
    if let Some(site) = &f.website {
        if let Some(repo) = sources::parse_github_repo(site) {
            return Some(Source::Github(repo));
        }
    }
    None
}

/// Group scanned folders into packages and attach sources.
fn build_packages(st: &AppState, cat: &Catalog, folders: &[AddonFolder]) -> Vec<Package> {
    let mut by_folder: HashMap<String, &CatalogEntry> = HashMap::new();
    let mut by_wago: HashMap<&str, &CatalogEntry> = HashMap::new();
    let mut by_curse: HashMap<u64, &CatalogEntry> = HashMap::new();
    let mut by_wowi: HashMap<u64, &CatalogEntry> = HashMap::new();
    for e in &cat.addons {
        for f in &e.folders {
            by_folder.insert(f.to_ascii_lowercase(), e);
        }
        if let Some(w) = &e.wago {
            by_wago.insert(w.as_str(), e);
        }
        if let Some(c) = e.curse {
            by_curse.insert(c, e);
        }
        if let Some(i) = e.wowi {
            by_wowi.insert(i, e);
        }
    }

    // Folders we installed ourselves are authoritative: they stay with their package.
    let mut managed_folder: HashMap<String, &str> = HashMap::new();
    for (key, rec) in &st.installed {
        for f in &rec.folders {
            managed_folder.insert(f.to_ascii_lowercase(), key.as_str());
        }
    }
    let on_disk: HashMap<String, &AddonFolder> =
        folders.iter().map(|f| (f.folder.to_ascii_lowercase(), f)).collect();

    // First pass: direct keys (managed / catalogue / ids / standalone).
    let direct_key = |f: &AddonFolder| -> (String, Option<&CatalogEntry>) {
        let cat_entry = by_folder
            .get(&f.folder.to_ascii_lowercase())
            .copied()
            .or_else(|| f.wago.as_deref().and_then(|w| by_wago.get(w).copied()))
            .or_else(|| f.curse.and_then(|c| by_curse.get(&c).copied()))
            .or_else(|| f.wowi.and_then(|i| by_wowi.get(&i).copied()));
        if let Some(k) = managed_folder.get(&f.folder.to_ascii_lowercase()) {
            let entry = k
                .strip_prefix("cat:")
                .and_then(|id| cat.addons.iter().find(|e| e.id == id))
                .or(cat_entry);
            return ((*k).to_string(), entry);
        }
        let key = if let Some(e) = cat_entry {
            format!("cat:{}", e.id)
        } else if let Some(w) = &f.wago {
            format!("wago:{w}")
        } else if let Some(i) = f.wowi {
            format!("wowi:{i}")
        } else if let Some(c) = f.curse {
            format!("curse:{c}")
        } else {
            format!("folder:{}", f.folder)
        };
        (key, cat_entry)
    };

    // Prefer an explicit parent, then a required dependency backed by Group
    // or a shared name stem (e.g. DBM-GUI -> DBM-Core).
    fn module_of<'a>(f: &AddonFolder, on_disk: &HashMap<String, &'a AddonFolder>) -> Option<&'a AddonFolder> {
        if let Some(p) = &f.part_of {
            if let Some(parent) = on_disk.get(&p.to_ascii_lowercase()) {
                return Some(parent);
            }
        }
        if let Some(group) = &f.group {
            if f.dependencies.iter().any(|d| d.eq_ignore_ascii_case(group)) {
                if let Some(parent) = on_disk.get(&group.to_ascii_lowercase()) {
                    if !parent.folder.eq_ignore_ascii_case(&f.folder) {
                        return Some(parent);
                    }
                }
            }
        }
        let stem = |s: &str| s.split(['_', '-']).next().unwrap_or(s).to_ascii_lowercase();
        let my_stem = stem(&f.folder);
        for d in &f.dependencies {
            if let Some(parent) = on_disk.get(&d.to_ascii_lowercase()) {
                if parent.folder != f.folder && stem(&parent.folder) == my_stem {
                    return Some(parent);
                }
            }
        }
        None
    }

    let mut keyed: Vec<(String, Option<&CatalogEntry>, &AddonFolder)> = Vec::with_capacity(folders.len());
    for f in folders {
        let (k, e) = direct_key(f);
        keyed.push((k, e, f));
    }
    let has_package_owner = |key: &str, folder: &AddonFolder| {
        !key.starts_with("folder:")
            || managed_folder.contains_key(&folder.folder.to_ascii_lowercase())
    };
    // Resolve existing module ownership before adopting required dependencies.
    let key_of: HashMap<String, (String, Option<&CatalogEntry>)> = keyed
        .iter()
        .map(|(k, e, f)| (f.folder.to_ascii_lowercase(), (k.clone(), *e)))
        .collect();
    for item in keyed.iter_mut() {
        if has_package_owner(&item.0, item.2) {
            continue;
        }
        let mut cur = item.2;
        for _ in 0..4 {
            let Some(parent) = module_of(cur, &on_disk) else { break };
            let Some((k, e)) = key_of.get(&parent.folder.to_ascii_lowercase()) else { break };
            item.0 = k.clone();
            item.1 = *e;
            // An identified package owns this module even if it
            // depends on another package in the same group.
            if has_package_owner(k, parent) {
                break;
            }
            cur = parent;
        }
    }

    // Adopt unowned dependency groups only after their module membership is known.
    let required_roots: Vec<_> = keyed.iter().filter_map(|(key, entry, f)| {
        let e = entry.filter(|e| e.group_required_dependencies)?;
        let primary = e.folders.first()?;
        primary.eq_ignore_ascii_case(&f.folder).then(|| (key.clone(), e, *f))
    }).collect();
    let mut adopted = HashMap::new();
    for (key, _, f) in &keyed {
        if has_package_owner(key, f) || key != &format!("folder:{}", f.folder)
            || f.folder.to_ascii_lowercase().starts_with("blizzard_")
        {
            continue;
        }
        let mut roots = required_roots.iter().filter(|(_, _, root)| {
            root.dependencies.iter().any(|name| name.eq_ignore_ascii_case(&f.folder))
        });
        let Some((owner, entry, _)) = roots.next() else { continue };
        if roots.next().is_none() {
            adopted.insert(key.clone(), (owner.clone(), *entry));
        }
    }
    for item in &mut keyed {
        if let Some((key, entry)) = adopted.get(&item.0) {
            item.0 = key.clone();
            item.1 = Some(*entry);
        }
    }

    // key -> (entry, folders)
    let mut groups: Vec<(String, Option<&CatalogEntry>, Vec<&AddonFolder>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (key, entry, f) in keyed {
        match index.get(&key) {
            Some(&i) => {
                groups[i].2.push(f);
                if groups[i].1.is_none() {
                    groups[i].1 = entry;
                }
            }
            None => {
                index.insert(key.clone(), groups.len());
                groups.push((key, entry, vec![f]));
            }
        }
    }

    let mut out = Vec::new();
    for (key, entry, fs) in groups {
        let in_group: Vec<String> = fs.iter().map(|f| f.folder.to_ascii_lowercase()).collect();
        let is_root = |f: &AddonFolder| {
            !f.dependencies.iter().any(|d| in_group.contains(&d.to_ascii_lowercase()))
                && f.part_of.as_ref().map(|p| !in_group.contains(&p.to_ascii_lowercase())).unwrap_or(true)
        };
        let primary = entry
            .filter(|e| e.group_required_dependencies)
            .and_then(|e| e.folders.first())
            .and_then(|name| fs.iter().copied().find(|f| f.folder.eq_ignore_ascii_case(name)))
            .or_else(|| fs.iter()
                .min_by_key(|f| (!is_root(f), f.folder.len(), f.folder.clone()))
                .copied())
            .unwrap();
        let managed = st.installed.get(&key);
        let name = entry
            .map(|e| e.name.clone())
            .unwrap_or_else(|| primary.title.clone());
        let installed_version = managed.map(|m| m.version.clone())
            .or_else(|| entry.filter(|e| e.group_required_dependencies).and_then(|_| primary.version.clone()))
            .or_else(|| fs.iter().find_map(|f| f.version.clone()))
            .filter(|v| !v.is_empty() && !v.starts_with('@'));
        let source = pick_source(entry, primary, managed)
            .or_else(|| fs.iter().find_map(|f| pick_source(None, f, None)));
        let curse_id = entry.and_then(|e| e.curse).or_else(|| fs.iter().find_map(|f| f.curse));
        let supports_forever = fs.iter().any(|f| f.supports_forever());

        // Required dependencies that are not on disk at all.
        let mut missing: Vec<MissingDep> = Vec::new();
        for f in &fs {
            for d in &f.dependencies {
                let dl = d.to_ascii_lowercase();
                if dl.starts_with("blizzard_") || on_disk.contains_key(&dl) || in_group.contains(&dl) {
                    continue;
                }
                if missing.iter().any(|m| m.folder.eq_ignore_ascii_case(d)) {
                    continue;
                }
                let ce = by_folder.get(&dl).copied();
                missing.push(MissingDep {
                    folder: d.clone(),
                    catalog_id: ce.map(|e| e.id.clone()),
                    name: ce.map(|e| e.name.clone()),
                });
            }
        }

        let (status, note) = match &source {
            Some(Source::Wago(_)) => (
                "no-source".to_string(),
                "Wago isn't a source any more (its updater keys are Patreon-only): reinstall from GitHub or CurseForge".to_string(),
            ),
            Some(_) => ("unknown".to_string(), String::new()),
            None if curse_id.is_some() => (
                "curse-only".to_string(),
                "Only on CurseForge: AddonForge can't update it, but the link works".to_string(),
            ),
            None => ("no-source".to_string(), "No update source found in its files".to_string()),
        };
        let mut folder_names: Vec<String> = fs.iter().map(|f| f.folder.clone()).collect();
        folder_names.sort();
        out.push(Package {
            pinned: st.pinned.contains(&key),
            ignored: st.ignored.contains(&key),
            key,
            name,
            folders: folder_names,
            installed_version,
            author: primary.author.clone(),
            notes: entry.map(|e| e.desc.clone()).filter(|d| !d.is_empty()).or_else(|| primary.notes.clone()),
            source_label: source.as_ref().map(|s| s.label()).unwrap_or_default(),
            source_url: source
                .as_ref()
                .map(|s| s.url())
                .or_else(|| curse_id.map(|c| format!("https://www.curseforge.com/projects/{c}")))
                .or_else(|| entry.and_then(|e| e.url.clone())),
            source,
            curse_id,
            catalog_id: entry.map(|e| e.id.clone()),
            notice: entry.and_then(|e| e.notice.clone()).filter(|n| !n.trim().is_empty()),
            supports_forever,
            managed: managed.is_some(),
            missing_deps: missing,
            icon_texture: std::iter::once(primary)
                .chain(fs.iter().copied())
                .find_map(|f| f.icon_texture.clone()),
            remote_version: None,
            remote_forever: None,
            remote_prerelease: false,
            update_available: false,
            status,
            note,
        });
    }
    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    out
}

async fn catalog_cached(app: &App) -> Catalog {
    let mut guard = app.catalog.lock().await;
    if guard.is_none() {
        let loaded = catalog::load(&app.client).await;
        logi!("catalogue loaded: {} entries ({})", loaded.0.addons.len(), loaded.1);
        *guard = Some(loaded);
    }
    guard.as_ref().unwrap().0.clone()
}

async fn stats_cached(app: &App) -> Stats {
    let mut guard = app.stats.lock().await;
    if guard.is_none() {
        let s = catalog::load_stats(&app.client).await;
        logi!("stats loaded: {} entries ({})", s.entries.len(), if s.updated.is_empty() { "none" } else { s.updated.as_str() });
        *guard = Some(s);
    }
    guard.as_ref().unwrap().clone()
}

/// Icon for one thing, via the run cache: `local:<install>|<texture>` or `url:<https>`.
async fn icon_cached(app: &App, cache_key: String, produce: impl std::future::Future<Output = Option<String>>) -> Option<String> {
    if let Some(v) = app.icon_cache.lock().await.get(&cache_key) {
        return v.clone();
    }
    let v = produce.await;
    app.icon_cache.lock().await.insert(cache_key, v.clone());
    v
}

fn entry_hint(cat: &Catalog, pkg: &Package) -> (Option<String>, Option<bool>) {
    let entry = pkg
        .catalog_id
        .as_ref()
        .and_then(|id| cat.addons.iter().find(|e| &e.id == id));
    (entry.and_then(|e| e.asset_hint.clone()), entry.and_then(|e| e.forever))
}

fn entry_for_source<'a>(cat: &'a Catalog, key: &str, src: &Source) -> Option<&'a CatalogEntry> {
    key.strip_prefix("cat:").and_then(|id| cat.addons.iter().find(|e| e.id == id))
        .or_else(|| cat.addons.iter().find(|e| match src {
            Source::Github(repo) => e.github.as_deref().map(|g| g.eq_ignore_ascii_case(repo)).unwrap_or(false),
            Source::Wago(id) => e.wago.as_ref() == Some(id),
            Source::Wowi(id) => e.wowi == Some(*id),
            Source::Tukui(id) => e.tukui.as_ref() == Some(id),
            Source::Curse(id) => e.curse == Some(*id),
        }))
}

fn key_for_source(cat: &Catalog, st: &AppState, src: &Source, requested: Option<&str>) -> String {
    let entry = entry_for_source(cat, "", src);
    if let Some(key) = requested {
        if st.installed.contains_key(key) || entry.is_none() {
            return key.to_string();
        }
    }
    let canonical = entry.map(|e| format!("cat:{}", e.id))
        .unwrap_or_else(|| format!("{}:{}", src.kind(), src.id()));
    if st.installed.contains_key(&canonical) {
        return canonical;
    }
    // Reuse an existing installation of this source.
    st.installed.iter().filter(|(_, rec)| {
        match (Source::from_parts(&rec.source, &rec.source_id), src) {
            (Some(Source::Github(a)), Source::Github(b)) => a.eq_ignore_ascii_case(b),
            (Some(a), b) => &a == b,
            _ => false,
        }
    }).map(|(key, _)| key).min().cloned().unwrap_or(canonical)
}

fn apply_remote(pkg: &mut Package, mut remote: Remote, vouched: Option<bool>) {
    if remote.forever == Some(false) && vouched == Some(true) {
        remote.forever = None;
    }
    let same = pkg.installed_version.as_deref()
        .map(|v| normalize_version(v) == normalize_version(&remote.version)).unwrap_or(false);
    pkg.update_available = !same;
    pkg.status = if same { "ok".into() } else { "update".into() };
    pkg.note = match remote.forever {
        Some(false) => "Latest release does not list a Forever build".into(),
        _ => String::new(),
    };
    pkg.remote_forever = remote.forever;
    pkg.remote_prerelease = remote.prerelease;
    pkg.remote_version = Some(remote.version);
}

fn protected_package_folders(st: &AppState, packages: &[Package], key: &str) -> HashMap<String, String> {
    let mut protected = HashMap::new();
    // Include recorded claims; the installer only enforces them for existing folders.
    for (owner, rec) in &st.installed {
        if owner != key {
            for folder in &rec.folders {
                protected.insert(folder.to_ascii_lowercase(), owner.clone());
            }
        }
    }
    for package in packages {
        if package.key != key {
            for folder in &package.folders {
                protected.insert(folder.to_ascii_lowercase(), package.key.clone());
            }
        }
    }
    protected
}

fn record_install(st: &mut AppState, key: &str, rec: &Installed) {
    let folders: HashSet<String> = rec.folders.iter().map(|f| f.to_ascii_lowercase()).collect();
    // Successful placement transfers any stale claims to the new owner.
    st.installed.retain(|owner, previous| {
        if owner == key || previous.folders.is_empty() {
            return true;
        }
        previous.folders.retain(|f| !folders.contains(&f.to_ascii_lowercase()));
        !previous.folders.is_empty()
    });
    st.installed.insert(key.to_string(), rec.clone());
}

async fn resolve_into(app: &App, pkg: &mut Package, cat: &Catalog) {
    let Some(src) = pkg.source.clone() else { return };
    let st = app.state.lock().await.clone();
    if matches!(src, Source::Wago(_)) {
        return;
    }
    let (hint, vouched) = entry_hint(cat, pkg);
    let opts = ResolveOpts {
        asset_hint: hint.as_deref(),
        wago_key: None,
        prerelease: st.allow_prerelease,
    };
    match sources::resolve(&app.client, &src, opts).await {
        Ok(r) => apply_remote(pkg, r, vouched),
        Err(e) => {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix(sources::curseforge::OPT_OUT) {
                // Not an error: the author only publishes through CurseForge's own app.
                pkg.status = "curse-only".into();
                pkg.note = format!("CurseForge{rest}; the link still works");
                return;
            }
            loge!("check {}: {e}", pkg.key);
            pkg.status = "error".into();
            pkg.note = msg;
        }
    }
}

async fn check_all(app: Shared, pkgs: &mut [Package], cat: &Catalog) {
    let curse_ids: Vec<u64> = pkgs
        .iter()
        .filter(|p| !p.ignored)
        .filter_map(|p| match &p.source {
            Some(Source::Curse(id)) => Some(*id),
            _ => None,
        })
        .collect();
    if !curse_ids.is_empty() {
        match sources::curseforge::prefetch(&app.client, &curse_ids).await {
            Ok(n) => logi!("curseforge: prefetched {n} of {} addons in one request", curse_ids.len()),
            Err(e) => loge!("curseforge prefetch: {e}"),
        }
    }
    let sem = Arc::new(Semaphore::new(6));
    let mut set = tokio::task::JoinSet::new();
    for (i, p) in pkgs.iter().enumerate() {
        if p.source.is_none() || p.ignored {
            continue;
        }
        let mut p = p.clone();
        let cat = cat.clone();
        let sem = sem.clone();
        let app_ref = app.clone();
        set.spawn(async move {
            let _permit = sem.acquire().await;
            resolve_into(&app_ref, &mut p, &cat).await;
            (i, p)
        });
    }
    while let Some(Ok((i, p))) = set.join_next().await {
        pkgs[i] = p;
    }
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
struct Settings {
    version: String,
    build: u32,
    install_path: Option<String>,
    install: Option<WowInstall>,
    allow_prerelease: bool,
    ignored_count: usize,
    state_file: String,
    log_file: String,
    catalog_source: Option<String>,
    catalog_updated: Option<String>,
    catalog_count: usize,
    repo_url: String,
    support_email: String,
    /// `ADDONFORGE_TAB=browse|settings` opens on that tab (screenshots, testing).
    start_tab: Option<String>,
    /// ForeverSVFix is set up on this install (see `wow::svfix_present`).
    svfix: bool,
    /// This build carries the CurseForge API key (search + install from CurseForge).
    has_curse: bool,
    /// Install everything that has an update right after the launch check.
    auto_update: bool,
}

#[tauri::command]
async fn get_settings(app: State<'_, Shared>) -> Result<Settings, String> {
    let st = app.state.lock().await.clone();
    let install = st.install_path.as_deref().and_then(|p| wow::describe_install(Path::new(p)));
    let cat = app.catalog.lock().await.clone();
    Ok(Settings {
        version: selfupdate::version().into(),
        build: selfupdate::build(),
        install_path: st.install_path.clone(),
        install,
        allow_prerelease: st.allow_prerelease,
        ignored_count: st.ignored.len(),
        state_file: AppState::path().to_string_lossy().to_string(),
        log_file: log::path().to_string_lossy().to_string(),
        catalog_source: cat.as_ref().map(|c| c.1.to_string()),
        catalog_updated: cat.as_ref().map(|c| c.0.updated.clone()),
        catalog_count: cat.as_ref().map(|c| c.0.addons.len()).unwrap_or(0),
        repo_url: REPO_URL.into(),
        support_email: SUPPORT_EMAIL.into(),
        start_tab: std::env::var("ADDONFORGE_TAB").ok().filter(|t| !t.is_empty()),
        svfix: st.install_path.as_deref().map(|p| wow::svfix_present(Path::new(p))).unwrap_or(false),
        has_curse: sources::curseforge::available(),
        auto_update: st.auto_update,
    })
}

#[tauri::command]
async fn find_installs(app: State<'_, Shared>) -> Result<Vec<WowInstall>, String> {
    let found = tokio::task::spawn_blocking(wow::find_installs).await.map_err(err)?;
    let mut st = app.state.lock().await;
    if st.install_path.is_none() {
        if let Some(f) = preferred_forever(&found) {
            st.install_path = Some(f.path.clone());
            st.save().map_err(err)?;
            logi!("auto-selected install {}", f.path);
        }
    }
    Ok(found)
}

fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The live `_forever_` client beats the beta folder when both are present
/// (from 4 Nov 2026 most people will have both for a while).
fn preferred_forever(found: &[WowInstall]) -> Option<&WowInstall> {
    let forever = found.iter().filter(|i| i.flavor == "forever");
    forever.clone().find(|i| folder_name(&i.path) == "_forever_").or_else(|| forever.clone().next())
}

/// The managed folder is still the beta one and a live `_forever_` client
/// exists beside it: its path, unless the user already said "not now" for it.
#[tauri::command]
async fn live_folder_hint(app: State<'_, Shared>) -> Result<Option<String>, String> {
    let (current, dismissed) = {
        let st = app.state.lock().await;
        (st.install_path.clone(), st.dismissed_live_hint.clone())
    };
    let Some(current) = current else { return Ok(None) };
    if folder_name(&current) != "_classic_beta_" {
        return Ok(None);
    }
    let found = tokio::task::spawn_blocking(wow::find_installs).await.map_err(err)?;
    Ok(found
        .iter()
        .filter(|i| i.flavor == "forever" && folder_name(&i.path) == "_forever_")
        .map(|i| i.path.clone())
        .find(|p| dismissed.as_deref() != Some(p.as_str())))
}

#[tauri::command]
async fn dismiss_live_hint(app: State<'_, Shared>, path: String) -> Result<(), String> {
    let mut st = app.state.lock().await;
    st.dismissed_live_hint = Some(path);
    st.save().map_err(err)
}

#[derive(Serialize)]
struct BackupResult {
    path: String,
    files: u64,
    bytes: u64,
}

/// Zip `<install>\WTF` into Documents\AddonForge backups. Read-only on WTF.
#[tauri::command]
async fn backup_wtf(app: State<'_, Shared>) -> Result<BackupResult, String> {
    do_backup(&app).await
}

async fn do_backup(app: &App) -> Result<BackupResult, String> {
    let install = current_install(app).await?;
    let src = Path::new(&install).join("WTF");
    if !src.is_dir() {
        return Err("No WTF folder in the game folder yet (start WoW once first)".into());
    }
    let flavor = folder_name(&install).trim_matches('_').to_string();
    let stamp = chrono_stamp();
    let dir = dirs::document_dir().unwrap_or_else(|| AppState::dir()).join("AddonForge backups");
    let dest = dir.join(format!("WTF-{flavor}-{stamp}.zip"));
    let d2 = dest.clone();
    let s = tokio::task::spawn_blocking(move || backup::zip_dir(&src, &d2)).await.map_err(err)?.map_err(err)?;
    logi!("backed up WTF to {} ({} files)", dest.display(), s.files);
    Ok(BackupResult { path: dest.to_string_lossy().to_string(), files: s.files, bytes: s.bytes })
}

/// `YYYYMMDD-HHMM` local-ish (UTC) stamp without pulling in a date crate.
fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let (h, m) = ((secs % 86400) / 3600, (secs % 3600) / 60);
    // Civil-from-days (Howard Hinnant), good for any date we will see.
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}")
}

#[tauri::command]
async fn open_backups_folder() -> Result<String, String> {
    let dir = dirs::document_dir().unwrap_or_else(|| AppState::dir()).join("AddonForge backups");
    std::fs::create_dir_all(&dir).map_err(err)?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(err)?;
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
async fn set_install_path(app: State<'_, Shared>, path: String) -> Result<WowInstall, String> {
    let p = PathBuf::from(path.trim());
    let info = wow::describe_install(&p)
        .ok_or_else(|| "That folder doesn't look like a WoW product folder (expected e.g. _forever_ or _classic_beta_ with Interface/AddOns inside)".to_string())?;
    let mut st = app.state.lock().await;
    st.install_path = Some(info.path.clone());
    st.save().map_err(err)?;
    logi!("install set to {}", info.path);
    Ok(info)
}

#[tauri::command]
async fn set_prefs(app: State<'_, Shared>, allow_prerelease: Option<bool>, auto_update: Option<bool>) -> Result<(), String> {
    let mut st = app.state.lock().await;
    if let Some(v) = allow_prerelease {
        st.allow_prerelease = v;
        logi!("allow_prerelease = {v}");
    }
    if let Some(v) = auto_update {
        st.auto_update = v;
        logi!("auto_update = {v}");
    }
    st.save().map_err(err)
}

#[tauri::command]
async fn set_pin(app: State<'_, Shared>, key: String, pinned: bool) -> Result<(), String> {
    let mut st = app.state.lock().await;
    if pinned {
        st.pinned.insert(key);
    } else {
        st.pinned.remove(&key);
    }
    st.save().map_err(err)
}

#[tauri::command]
async fn set_ignore(app: State<'_, Shared>, key: String, ignored: bool) -> Result<(), String> {
    let mut st = app.state.lock().await;
    if ignored {
        st.ignored.insert(key);
    } else {
        st.ignored.remove(&key);
    }
    st.save().map_err(err)
}

async fn current_install(app: &App) -> Result<String, String> {
    let st = app.state.lock().await;
    st.install_path
        .clone()
        .ok_or_else(|| "No WoW install selected yet".to_string())
}

async fn scan_packages(app: &App) -> Result<Vec<Package>, String> {
    let install = current_install(app).await?;
    let cat = catalog_cached(app).await;
    let folders = tokio::task::spawn_blocking(move || wow::scan_addons(Path::new(&install)))
        .await
        .map_err(err)?;
    let st = app.state.lock().await.clone();
    Ok(build_packages(&st, &cat, &folders))
}

#[tauri::command]
async fn scan(app: State<'_, Shared>) -> Result<Vec<Package>, String> {
    scan_packages(&app).await
}

#[tauri::command]
async fn check_updates(app: State<'_, Shared>) -> Result<Vec<Package>, String> {
    let mut pkgs = scan_packages(&app).await?;
    let cat = catalog_cached(&app).await;
    check_all(app.inner().clone(), &mut pkgs, &cat).await;
    let n = pkgs.iter().filter(|p| p.status == "update").count();
    logi!("check: {} packages, {} updates", pkgs.len(), n);
    Ok(pkgs)
}

async fn install_from(
    app: &App,
    key: &str,
    src: &Source,
    asset_hint: Option<&str>,
    allow_non_forever: bool,
) -> Result<Installed, String> {
    let install = current_install(app).await?;
    let addons = addons_dir(&install);
    let st = app.state.lock().await.clone();
    let cat = catalog_cached(app).await;
    let entry = entry_for_source(&cat, key, src);
    let opts = ResolveOpts {
        asset_hint,
        wago_key: None,
        prerelease: st.allow_prerelease,
    };
    let remote = sources::resolve(&app.client, src, opts).await.map_err(err)?;
    let vouched = entry.and_then(|e| e.forever);
    // A source archive says nothing about flavours (forever = None); then the
    // catalogue's own verdict stands in for it.
    let not_forever = remote.forever == Some(false) || (remote.forever.is_none() && vouched == Some(false));
    if not_forever && !allow_non_forever && vouched != Some(true) {
        return Err("NOT_FOREVER".into());
    }
    let protected = if entry.map(|e| e.group_required_dependencies).unwrap_or(false) {
        let scan_path = install.clone();
        let folders = tokio::task::spawn_blocking(move || wow::scan_addons(Path::new(&scan_path)))
            .await.map_err(err)?;
        let packages = build_packages(&st, &cat, &folders);
        protected_package_folders(&st, &packages, key)
    } else {
        protected_package_folders(&st, &[], key)
    };
    logi!("install {key} from {} {} -> {}", src.label(), remote.version, remote.download_url);
    let tmp = addons.join(install::TMP_DIR).join("dl");
    let zip = install::download(&app.client, &remote.download_url, &tmp, &remote.filename)
        .await
        .map_err(|e| {
            loge!("download {key}: {e}");
            e.to_string()
        })?;
    let addons2 = addons.clone();
    let folders = tokio::task::spawn_blocking(move || install::place(&zip, &addons2, &protected))
        .await
        .map_err(err)?
        .map_err(|e| {
            loge!("place {key}: {e}");
            e.to_string()
        })?;
    let rec = Installed {
        source: src.kind().into(),
        source_id: src.id(),
        version: remote.version.clone(),
        folders,
        installed_at: state::now_secs(),
    };
    let mut st = app.state.lock().await;
    record_install(&mut st, key, &rec);
    st.save().map_err(err)?;
    logi!("installed {key} {} ({} folders)", rec.version, rec.folders.len());
    Ok(rec)
}

async fn refreshed(app: &App, key: &str) -> Result<Package, String> {
    let cat = catalog_cached(app).await;
    let mut pkgs = scan_packages(app).await?;
    let mut p = pkgs
        .drain(..)
        .find(|p| p.key == key)
        .ok_or_else(|| "Installed, but its folders didn't show up on rescan".to_string())?;
    resolve_into(app, &mut p, &cat).await;
    Ok(p)
}

#[tauri::command]
async fn update_package(
    app: State<'_, Shared>,
    key: String,
    allow_non_forever: Option<bool>,
) -> Result<Package, String> {
    let pkgs = scan_packages(&app).await?;
    let pkg = pkgs
        .into_iter()
        .find(|p| p.key == key)
        .ok_or_else(|| "Addon not found on disk any more".to_string())?;
    let src = pkg.source.clone().ok_or_else(|| "This addon has no update source".to_string())?;
    let cat = catalog_cached(&app).await;
    let (hint, _) = entry_hint(&cat, &pkg);
    install_from(&app, &key, &src, hint.as_deref(), allow_non_forever.unwrap_or(false)).await?;
    refreshed(&app, &key).await
}

#[derive(Serialize, Clone)]
struct CatalogRow {
    #[serde(flatten)]
    entry: CatalogEntry,
    installed: bool,
    key: String,
    downloads: Option<u64>,
    updated_at: Option<String>,
    stat_source: Option<String>,
}

fn installed_catalog_ids(packages: Vec<Package>) -> Vec<String> {
    packages.into_iter().filter_map(|p| p.catalog_id).collect()
}

#[tauri::command]
async fn catalog_list(app: State<'_, Shared>, refresh: Option<bool>) -> Result<Vec<CatalogRow>, String> {
    if refresh.unwrap_or(false) {
        *app.catalog.lock().await = None;
        *app.stats.lock().await = None;
    }
    let cat = catalog_cached(&app).await;
    let stats = stats_cached(&app).await;
    let installed_ids = scan_packages(&app).await.map(installed_catalog_ids).unwrap_or_default();
    Ok(cat
        .addons
        .into_iter()
        .map(|e| {
            let key = format!("cat:{}", e.id);
            let st = stats.entries.get(&e.id);
            CatalogRow {
                installed: installed_ids.contains(&e.id),
                key,
                downloads: st.and_then(|s| s.downloads),
                updated_at: st.and_then(|s| s.updated_at.clone()),
                stat_source: st.map(|s| s.source.clone()).filter(|s| !s.is_empty()),
                entry: e,
            }
        })
        .collect())
}

#[derive(Serialize, Clone)]
struct BundleAddon {
    id: String,
    name: String,
    desc: String,
    installable: bool,
    needs_key: bool,
    installed: bool,
    forever: Option<bool>,
}

#[derive(Serialize, Clone)]
struct BundleView {
    id: String,
    name: String,
    desc: String,
    note: String,
    addons: Vec<BundleAddon>,
}

#[tauri::command]
async fn bundles(app: State<'_, Shared>) -> Result<Vec<BundleView>, String> {
    let cat = catalog_cached(&app).await;
    let installed_ids = scan_packages(&app).await.map(installed_catalog_ids).unwrap_or_default();
    Ok(cat
        .bundles
        .iter()
        .map(|b| BundleView {
            id: b.id.clone(),
            name: b.name.clone(),
            desc: b.desc.clone(),
            note: b.note.clone(),
            addons: b
                .addons
                .iter()
                .filter_map(|id| cat.addons.iter().find(|e| &e.id == id))
                .map(|e| {
                    let direct = e.github.is_some()
                        || e.wowi.is_some()
                        || e.tukui.is_some()
                        || (e.curse.is_some() && !e.curse_locked && sources::curseforge::available());
                    BundleAddon {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        desc: e.desc.clone(),
                        installable: direct,
                        needs_key: false,
                        installed: installed_ids.contains(&e.id),
                        forever: e.forever,
                    }
                })
                .collect(),
        })
        .collect())
}

/// Icons for installed packages: own icon file, else GitHub avatar, else none.
#[tauri::command]
async fn package_icons(app: State<'_, Shared>, keys: Vec<String>) -> Result<HashMap<String, String>, String> {
    let install = current_install(&app).await?;
    let pkgs = scan_packages(&app).await?;
    let mut out = HashMap::new();
    for p in pkgs.into_iter().filter(|p| keys.contains(&p.key)) {
        let mut icon = None;
        if let Some(tex) = &p.icon_texture {
            let (inst, tex2) = (install.clone(), tex.clone());
            icon = icon_cached(&app, format!("local:{inst}|{tex2}"), async move {
                tokio::task::spawn_blocking(move || icons::local_icon(Path::new(&inst), &tex2)).await.ok().flatten()
            })
            .await;
        }
        if icon.is_none() {
            if let Some(Source::Github(repo)) = &p.source {
                if let Some(url) = icons::github_avatar_url(repo) {
                    let (c, u) = (app.client.clone(), url.clone());
                    icon = icon_cached(&app, format!("url:{url}"), async move { icons::remote_icon(&c, &u).await }).await;
                }
            }
        }
        if let Some(i) = icon {
            out.insert(p.key, i);
        }
    }
    Ok(out)
}

/// Icons for catalogue entries: explicit `icon` URL, else GitHub avatar.
#[tauri::command]
async fn catalog_icons(app: State<'_, Shared>, ids: Vec<String>) -> Result<HashMap<String, String>, String> {
    let cat = catalog_cached(&app).await;
    let mut out = HashMap::new();
    for e in cat.addons.iter().filter(|e| ids.contains(&e.id)) {
        let url = e
            .icon
            .clone()
            .or_else(|| e.github.as_deref().and_then(icons::github_avatar_url));
        let Some(url) = url else { continue };
        let (c, u) = (app.client.clone(), url.clone());
        if let Some(i) = icon_cached(&app, format!("url:{url}"), async move { icons::remote_icon(&c, &u).await }).await {
            out.insert(e.id.clone(), i);
        }
    }
    Ok(out)
}

/// Longer description + preview image for the details window. GitHub only
/// for now (README excerpt and its first screenshot); other sources return
/// nothing and the UI shows the catalogue description and a link.
#[tauri::command]
async fn addon_details(app: State<'_, Shared>, id: String) -> Result<icons::Details, String> {
    let cat = catalog_cached(&app).await;
    let Some(e) = cat.addons.iter().find(|e| e.id == id) else { return Ok(icons::Details::default()) };
    match &e.github {
        Some(repo) => Ok(icons::github_details(&app.client, repo).await),
        None => Ok(icons::Details::default()),
    }
}

/// One CurseForge search result, trimmed to what the Find addons list shows.
#[derive(Serialize, Clone)]
struct CurseRow {
    id: u64,
    name: String,
    summary: String,
    authors: String,
    /// CurseForge's first category for the project, for the list's Category column.
    category: String,
    downloads: u64,
    url: String,
    /// False when the author only distributes through CurseForge's own app.
    installable: bool,
    /// Catalogue id when this project is already listed (so its row is preferred).
    catalog_id: Option<String>,
    installed: bool,
}

#[derive(Serialize, Clone)]
struct CurseSearch {
    rows: Vec<CurseRow>,
    /// How many Forever addons CurseForge has for this query (all of them when empty).
    total: u64,
}

/// One page (50) of CurseForge's Forever addons: text filter (empty = all),
/// sort, category id, offset. Results live in memory only.
#[tauri::command]
async fn curse_search(
    app: State<'_, Shared>,
    query: String,
    sort: Option<String>,
    category: Option<u64>,
    index: Option<u32>,
) -> Result<CurseSearch, String> {
    if !sources::curseforge::available() {
        return Err("CurseForge isn't available in this build".into());
    }
    let page = sources::curseforge::Page {
        query,
        sort: sources::curseforge::Sort::parse(sort.as_deref().unwrap_or("")),
        category,
        index: index.unwrap_or(0),
        limit: 50,
    };
    let (mods, total) = sources::curseforge::search_page(&app.client, &page).await.map_err(err)?;
    let cat = catalog_cached(&app).await;
    let st = app.state.lock().await.clone();
    let have: Vec<Package> = scan_packages(&app).await.unwrap_or_default();
    let rows: Vec<CurseRow> = mods
        .into_iter()
        .map(|m| {
            let entry = cat.addons.iter().find(|e| e.curse == Some(m.id));
            let installable = matches!(
                sources::curseforge::pick_file(&m, st.allow_prerelease),
                sources::curseforge::Pick::Ready(_) | sources::curseforge::Pick::NeedsFile(_)
            );
            let installed = have.iter().any(|p| {
                p.curse_id == Some(m.id)
                    || matches!(&p.source, Some(Source::Curse(c)) if *c == m.id)
                    || entry.map(|e| p.catalog_id.as_deref() == Some(e.id.as_str())).unwrap_or(false)
            });
            CurseRow {
                id: m.id,
                name: m.name.clone(),
                summary: m.summary.clone(),
                authors: m.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "),
                category: m.categories.first().map(|c| c.name.clone()).unwrap_or_default(),
                downloads: m.download_count,
                url: if m.links.website_url.is_empty() { sources::curseforge::project_url(m.id) } else { m.links.website_url.clone() },
                installable,
                catalog_id: entry.map(|e| e.id.clone()),
                installed,
            }
        })
        .collect();
    Ok(CurseSearch { rows, total })
}

/// Install a CurseForge project by id (from search). Catalogue entries that
/// carry the same id keep their catalogue identity.
#[tauri::command]
async fn install_curse(
    app: State<'_, Shared>,
    id: u64,
    allow_non_forever: Option<bool>,
) -> Result<Package, String> {
    if !sources::curseforge::available() {
        return Err("CurseForge isn't available in this build".into());
    }
    let cat = catalog_cached(&app).await;
    let st = app.state.lock().await.clone();
    let src = Source::Curse(id);
    let key = key_for_source(&cat, &st, &src, None);
    install_from(&app, &key, &src, None, allow_non_forever.unwrap_or(false)).await?;
    refreshed(&app, &key).await
}

/// The source the user picked in the Find addons row ("github", "curse",
/// "wago", "wowi", "tukui"), checked against what the entry actually offers.
fn chosen_source(e: &CatalogEntry, kind: &str) -> Result<Source, String> {
    let missing = || format!("{kind} isn't listed as a source for {}", e.name);
    match kind {
        "github" => e.github.clone().map(Source::Github).ok_or_else(missing),
        "curse" => {
            if !sources::curseforge::available() {
                return Err("CurseForge isn't available in this build".into());
            }
            if e.curse_locked {
                return Err(format!("{}'s author only distributes it through CurseForge's own app", e.name));
            }
            e.curse.map(Source::Curse).ok_or_else(missing)
        }
        "wago" => Err("Wago isn't a source any more: its updater keys are Patreon-only".into()),
        "wowi" => e.wowi.map(Source::Wowi).ok_or_else(missing),
        "tukui" => e.tukui.clone().map(Source::Tukui).ok_or_else(missing),
        other => Err(format!("Unknown source {other}")),
    }
}

#[tauri::command]
async fn install_catalog(
    app: State<'_, Shared>,
    id: String,
    allow_non_forever: Option<bool>,
    source: Option<String>,
) -> Result<Package, String> {
    let cat = catalog_cached(&app).await;
    let e = cat
        .addons
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or_else(|| "Not in the catalogue".to_string())?;
    let st = app.state.lock().await.clone();
    let src = match source.as_deref().filter(|s| !s.is_empty()) {
        Some(kind) => chosen_source(&e, kind)?,
        None => pick_source(Some(&e), &AddonFolder::default(), None)
            .ok_or_else(|| "This catalogue entry has no downloadable source".to_string())?,
    };
    let key = key_for_source(&cat, &st, &src, None);
    install_from(&app, &key, &src, e.asset_hint.as_deref(), allow_non_forever.unwrap_or(false)).await?;
    refreshed(&app, &key).await
}

/// Install straight from a source the user typed or imported.
#[tauri::command]
async fn install_source(
    app: State<'_, Shared>,
    source: Source,
    key: Option<String>,
    allow_non_forever: Option<bool>,
) -> Result<Package, String> {
    let cat = catalog_cached(&app).await;
    let st = app.state.lock().await.clone();
    let key = key_for_source(&cat, &st, &source, key.as_deref());
    install_from(&app, &key, &source, None, allow_non_forever.unwrap_or(false)).await?;
    refreshed(&app, &key).await
}

#[tauri::command]
async fn install_github(
    app: State<'_, Shared>,
    repo: String,
    allow_non_forever: Option<bool>,
) -> Result<Package, String> {
    let repo = sources::parse_github_repo(&repo)
        .ok_or_else(|| "Enter a GitHub repo as owner/name or paste its URL".to_string())?;
    // If the catalogue knows this source, retain its package identity.
    let cat = catalog_cached(&app).await;
    let src = Source::Github(repo);
    let st = app.state.lock().await.clone();
    let key = key_for_source(&cat, &st, &src, None);
    install_from(&app, &key, &src, None, allow_non_forever.unwrap_or(false)).await?;
    refreshed(&app, &key).await
}

#[tauri::command]
async fn remove_package(app: State<'_, Shared>, key: String) -> Result<Vec<String>, String> {
    let install = current_install(&app).await?;
    let pkgs = scan_packages(&app).await?;
    let pkg = pkgs
        .into_iter()
        .find(|p| p.key == key)
        .ok_or_else(|| "Addon not found".to_string())?;
    let addons = addons_dir(&install);
    let gone = tokio::task::spawn_blocking(move || install::remove_folders(&addons, &pkg.folders))
        .await
        .map_err(err)?
        .map_err(err)?;
    let mut st = app.state.lock().await;
    st.installed.remove(&key);
    st.pinned.remove(&key);
    st.ignored.remove(&key);
    st.save().map_err(err)?;
    logi!("removed {key}: {:?}", gone);
    Ok(gone)
}

#[tauri::command]
async fn open_addons_folder(app: State<'_, Shared>) -> Result<String, String> {
    let install = current_install(&app).await?;
    let dir = addons_dir(&install);
    std::fs::create_dir_all(&dir).map_err(err)?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(err)?;
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
async fn open_log() -> Result<String, String> {
    let p = log::path();
    if !p.exists() {
        std::fs::write(&p, "").map_err(err)?;
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(err)?;
    Ok(p.to_string_lossy().to_string())
}

// ---------------------------------------------------------------- import / export

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExportItem {
    name: String,
    key: String,
    #[serde(default)]
    source: Option<Source>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    folders: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExportFile {
    addonforge: u32,
    flavor: String,
    exported: u64,
    addons: Vec<ExportItem>,
}

async fn export_data(app: &App) -> Result<ExportFile, String> {
    let pkgs = scan_packages(app).await?;
    Ok(ExportFile {
        addonforge: 1,
        flavor: "forever".into(),
        exported: state::now_secs(),
        addons: pkgs
            .into_iter()
            .filter(|p| !p.ignored)
            .map(|p| ExportItem {
                name: p.name,
                key: p.key,
                source: p.source,
                version: p.installed_version,
                folders: p.folders,
            })
            .collect(),
    })
}

#[tauri::command]
async fn export_list(app: State<'_, Shared>, path: String) -> Result<usize, String> {
    let data = export_data(&app).await?;
    let n = data.addons.len();
    std::fs::write(&path, serde_json::to_vec_pretty(&data).map_err(err)?).map_err(err)?;
    logi!("exported {n} addons to {path}");
    Ok(n)
}

#[derive(Serialize, Clone)]
struct ImportRow {
    name: String,
    key: String,
    source: Option<Source>,
    installed: bool,
    installable: bool,
    why: String,
}

#[tauri::command]
async fn import_list(app: State<'_, Shared>, path: String) -> Result<Vec<ImportRow>, String> {
    let bytes = std::fs::read(&path).map_err(err)?;
    let file: ExportFile = serde_json::from_slice(&bytes).map_err(|e| format!("not an AddonForge list: {e}"))?;
    let have: Vec<String> = scan_packages(&app).await?.into_iter().map(|p| p.key).collect();
    let cat = catalog_cached(&app).await;
    let st = app.state.lock().await.clone();
    Ok(file
        .addons
        .into_iter()
        .map(|it| {
            let key = it.source.as_ref().map(|src| key_for_source(&cat, &st, src, Some(&it.key)))
                .unwrap_or(it.key);
            let installed = have.contains(&key);
            let (installable, why) = match &it.source {
                None => (false, "no source (probably CurseForge-only or a personal addon)".into()),
                Some(Source::Wago(_)) => (false, "Wago isn't a source any more".into()),
                Some(_) => (true, String::new()),
            };
            ImportRow { name: it.name, key, source: it.source, installed, installable, why }
        })
        .collect())
}

// ---------------------------------------------------------------- self-update

#[tauri::command]
async fn check_self_update(app: State<'_, Shared>) -> Result<selfupdate::SelfUpdate, String> {
    let url = std::env::var("ADDONFORGE_UPDATE_URL").ok();
    let mut r = selfupdate::check(&app.client, url.as_deref()).await;
    let st = app.state.lock().await;
    if st.skip_self_update.as_deref() == Some(r.tag.as_str()) {
        r.available = false;
    }
    if let Some(e) = &r.error {
        logi!("self-update check: {e}");
    } else {
        logi!("self-update check: latest {} (available: {})", r.tag, r.available);
    }
    Ok(r)
}

#[tauri::command]
async fn skip_self_update(app: State<'_, Shared>, tag: String) -> Result<(), String> {
    let mut st = app.state.lock().await;
    st.skip_self_update = Some(tag);
    st.save().map_err(err)
}

#[cfg(windows)]
#[tauri::command]
async fn apply_self_update(app: State<'_, Shared>, handle: tauri::AppHandle) -> Result<String, String> {
    let url = std::env::var("ADDONFORGE_UPDATE_URL").ok();
    let r = selfupdate::check(&app.client, url.as_deref()).await;
    let latest = r.latest.ok_or_else(|| r.error.unwrap_or_else(|| "no update info".into()))?;
    if !r.available {
        return Err("Already up to date".into());
    }
    let new_exe = selfupdate::apply(&app.client, &latest).await.map_err(|e| {
        loge!("self-update: {e}");
        e.to_string()
    })?;
    let h = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        h.exit(0);
    });
    Ok(new_exe.to_string_lossy().to_string())
}

/// Not Windows: releases only carry a Windows exe, so open the releases
/// page instead and keep running. "browser" tells the UI which happened.
#[cfg(not(windows))]
#[tauri::command]
async fn apply_self_update() -> Result<String, String> {
    tauri_plugin_opener::open_url(selfupdate::RELEASES_URL, None::<&str>).map_err(err)?;
    Ok("browser".into())
}

/// First launch of a newer build: return that build's release notes once.
/// Never on a fresh install (nothing to compare with), never for dev builds.
#[derive(Serialize)]
struct WhatsNew {
    version: String,
    build: u32,
    notes: String,
}

/// Every build's note between the one last seen and the one running, oldest
/// first, each headed by its build number when there is more than one.
/// Falls back to the single current note (older manifests have no history).
fn whats_new_text(l: &selfupdate::Latest, last_seen: u32, current: u32) -> String {
    let mut entries: Vec<&selfupdate::HistoryNote> = l
        .history
        .iter()
        .filter(|h| h.build <= current && (last_seen == 0 || h.build > last_seen) && !h.notes.trim().is_empty())
        .collect();
    entries.sort_by_key(|h| h.build);
    entries.dedup_by_key(|h| h.build);
    if last_seen == 0 {
        entries.retain(|h| h.build == current);
    }
    if entries.is_empty() {
        return l.notes.clone();
    }
    if entries.len() == 1 {
        return entries[0].notes.clone();
    }
    entries
        .iter()
        .map(|h| format!("Build {}{}\n{}", h.build, if h.version.is_empty() { String::new() } else { format!(" (v{})", h.version) }, h.notes.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[tauri::command]
async fn whats_new(app: State<'_, Shared>) -> Result<Option<WhatsNew>, String> {
    let current = selfupdate::build();
    if current == 0 {
        return Ok(None);
    }
    let (last, has_history) = {
        let st = app.state.lock().await;
        (st.last_seen_build, !st.installed.is_empty())
    };
    if last == current {
        return Ok(None);
    }
    // Existing user if: we recorded a previous build, or the self-updater
    // launched us, or they already have addons managed by AddonForge
    // (people on builds before this field existed). Otherwise fresh install.
    let just_updated = std::env::var("ADDONFORGE_JUST_UPDATED").is_ok();
    let existing = last != 0 || just_updated || has_history;
    let mut result = None;
    if existing {
        let url = std::env::var("ADDONFORGE_UPDATE_URL").ok();
        let r = selfupdate::check(&app.client, url.as_deref()).await;
        if let Some(l) = r.latest {
            if l.build == current {
                let notes = whats_new_text(&l, last, current);
                if !notes.trim().is_empty() {
                    result = Some(WhatsNew { version: l.version, build: l.build, notes });
                }
            }
        }
    }
    let mut st = app.state.lock().await;
    st.last_seen_build = current;
    st.save().map_err(err)?;
    Ok(result)
}

// ---------------------------------------------------------------- diagnostics

#[tauri::command]
async fn diagnostics(app: State<'_, Shared>) -> Result<String, String> {
    let st = app.state.lock().await.clone();
    let install = st.install_path.as_deref().and_then(|p| wow::describe_install(Path::new(p)));
    let pkgs = scan_packages(&app).await.unwrap_or_default();
    let mut s = String::new();
    s.push_str(&format!("AddonForge v{} build {}\n", selfupdate::version(), selfupdate::build()));
    s.push_str(&format!("OS: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
    s.push_str(&format!("Exe: {}\n", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()));
    match &install {
        Some(i) => s.push_str(&format!("Install: {} ({}, {})\n", i.path, i.product, i.label)),
        None => s.push_str("Install: none selected\n"),
    }
    s.push_str(&format!(
        "CurseForge: {} | pre-releases: {} | pinned: {} | ignored: {}\n",
        if sources::curseforge::available() { "yes" } else { "no" },
        st.allow_prerelease,
        st.pinned.len(),
        st.ignored.len()
    ));
    s.push_str(&format!("Addons on disk: {}\n", pkgs.len()));
    for p in &pkgs {
        s.push_str(&format!(
            "  - {} [{}] {} {}{}\n",
            p.name,
            p.status,
            p.installed_version.clone().unwrap_or_default(),
            p.source_label,
            if p.managed { " (managed)" } else { "" }
        ));
    }
    s.push_str("\n--- log tail ---\n");
    s.push_str(&log::tail(40));
    Ok(s)
}

// ---------------------------------------------------------------- boot

fn make_app() -> Shared {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("http client");
    let st = AppState::load();
    if let Some(p) = &st.install_path {
        install::clean_tmp(&addons_dir(p));
    }
    Arc::new(App {
        client,
        state: Mutex::new(st),
        catalog: Mutex::new(None),
        stats: Mutex::new(None),
        icon_cache: Mutex::new(HashMap::new()),
    })
}

/// Headless mode: prints JSON, returns a process exit code.
pub fn cli(args: &[String]) -> i32 {
    let rt = tokio::runtime::Runtime::new().expect("tokio");
    let app = make_app();
    let out = |v: serde_json::Value| println!("{}", serde_json::to_string_pretty(&v).unwrap());
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let res: Result<serde_json::Value, String> = rt.block_on(async {
        match cmd {
            "installs" => {
                let found = wow::find_installs();
                let mut st = app.state.lock().await;
                if st.install_path.is_none() {
                    if let Some(f) = found.iter().find(|i| i.flavor == "forever") {
                        st.install_path = Some(f.path.clone());
                        st.save().map_err(err)?;
                    }
                }
                Ok(serde_json::json!({ "installs": found, "selected": st.install_path }))
            }
            "use" => {
                let p = args.get(1).ok_or("usage: use <product folder>")?;
                let info = wow::describe_install(Path::new(p)).ok_or("not a WoW product folder")?;
                let mut st = app.state.lock().await;
                st.install_path = Some(info.path.clone());
                st.save().map_err(err)?;
                Ok(serde_json::to_value(info).unwrap())
            }
            "scan" => Ok(serde_json::to_value(scan_packages(&app).await?).unwrap()),
            "check" => {
                let mut pkgs = scan_packages(&app).await?;
                let cat = catalog_cached(&app).await;
                check_all(app.clone(), &mut pkgs, &cat).await;
                Ok(serde_json::to_value(pkgs).unwrap())
            }
            "catalog" => {
                let cat = catalog_cached(&app).await;
                Ok(serde_json::to_value(cat).unwrap())
            }
            "resolve" => {
                let cat = catalog_cached(&app).await;
                let st = app.state.lock().await.clone();
                let want: Vec<&CatalogEntry> = if args.get(1).map(String::as_str) == Some("all") {
                    cat.addons.iter().collect()
                } else {
                    cat.addons.iter().filter(|e| args[1..].iter().any(|a| a == &e.id)).collect()
                };
                let sem = Arc::new(Semaphore::new(8));
                let mut set = tokio::task::JoinSet::new();
                for e in want {
                    let e = e.clone();
                    let app = app.clone();
                    let sem = sem.clone();
                    let st = st.clone();
                    set.spawn(async move {
                        let _p = sem.acquire().await;
                        let src = pick_source(Some(&e), &AddonFolder::default(), None);
                        let r = match &src {
                            Some(s) => sources::resolve(
                                &app.client,
                                s,
                                ResolveOpts {
                                    asset_hint: e.asset_hint.as_deref(),
                                    wago_key: None,
                                    prerelease: st.allow_prerelease,
                                },
                            )
                            .await
                            .map_err(|x| x.to_string()),
                            None => Err("link-only".into()),
                        };
                        serde_json::json!({
                            "id": e.id,
                            "source": src.map(|s| s.label()),
                            "ok": r.is_ok(),
                            "version": r.as_ref().ok().map(|r| r.version.clone()),
                            "forever": r.as_ref().ok().and_then(|r| r.forever),
                            "prerelease": r.as_ref().ok().map(|r| r.prerelease),
                            "file": r.as_ref().ok().map(|r| r.filename.clone()),
                            "error": r.err(),
                        })
                    });
                }
                let mut rows = Vec::new();
                while let Some(Ok(v)) = set.join_next().await {
                    rows.push(v);
                }
                rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
                Ok(serde_json::Value::Array(rows))
            }
            "update" => {
                let key = args.get(1).ok_or("usage: update <package key>")?;
                let pkgs = scan_packages(&app).await?;
                let pkg = pkgs.into_iter().find(|p| &p.key == key).ok_or("no such package")?;
                let src = pkg.source.clone().ok_or("package has no source")?;
                let cat = catalog_cached(&app).await;
                let (hint, _) = entry_hint(&cat, &pkg);
                let force = args.iter().any(|a| a == "--force");
                let rec = install_from(&app, key, &src, hint.as_deref(), force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            "install" => {
                let id = args.get(1).ok_or("usage: install <catalog id>")?;
                let cat = catalog_cached(&app).await;
                let e = cat.addons.iter().find(|e| &e.id == id).cloned().ok_or("not in catalogue")?;
                let st = app.state.lock().await.clone();
                let src = pick_source(Some(&e), &AddonFolder::default(), None).ok_or("no source")?;
                let force = args.iter().any(|a| a == "--force");
                let key = key_for_source(&cat, &st, &src, None);
                let rec = install_from(&app, &key, &src, e.asset_hint.as_deref(), force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            "backup" => Ok(serde_json::to_value(do_backup(&app).await?).unwrap()),
            "search" => {
                let q = args[1..].join(" ");
                let mods = sources::curseforge::search(&app.client, &q, 15).await.map_err(|e| e.to_string())?;
                let st = app.state.lock().await.clone();
                Ok(serde_json::Value::Array(mods.iter().map(|m| serde_json::json!({
                    "id": m.id, "name": m.name, "downloads": m.download_count,
                    "distribution": m.allow_mod_distribution,
                    "pick": format!("{:?}", sources::curseforge::pick_file(m, st.allow_prerelease)),
                })).collect()))
            }
            "curse" => {
                let id: u64 = args.get(1).and_then(|s| s.parse().ok()).ok_or("usage: curse <project id> [--force]")?;
                let force = args.iter().any(|a| a == "--force");
                let src = Source::Curse(id);
                let cat = catalog_cached(&app).await;
                let st = app.state.lock().await.clone();
                let key = key_for_source(&cat, &st, &src, None);
                let rec = install_from(&app, &key, &src, None, force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            "github" => {
                let repo = args.get(1).and_then(|r| sources::parse_github_repo(r)).ok_or("usage: github <owner/repo>")?;
                let force = args.iter().any(|a| a == "--force");
                let src = Source::Github(repo);
                let cat = catalog_cached(&app).await;
                let st = app.state.lock().await.clone();
                let key = key_for_source(&cat, &st, &src, None);
                let rec = install_from(&app, &key, &src, None, force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            "remove" => {
                let key = args.get(1).ok_or("usage: remove <package key>")?;
                let install = current_install(&app).await?;
                let pkg = scan_packages(&app).await?.into_iter().find(|p| &p.key == key).ok_or("no such package")?;
                let gone = install::remove_folders(&addons_dir(&install), &pkg.folders).map_err(err)?;
                let mut st = app.state.lock().await;
                st.installed.remove(key);
                st.pinned.remove(key);
                st.ignored.remove(key);
                st.save().map_err(err)?;
                Ok(serde_json::json!({ "removed": gone }))
            }
            "prerelease" => {
                let v = args.get(1).map(|s| s == "on").unwrap_or(false);
                let mut st = app.state.lock().await;
                st.allow_prerelease = v;
                st.save().map_err(err)?;
                Ok(serde_json::json!({ "allow_prerelease": v }))
            }
            "pin" | "ignore" => {
                let key = args.get(1).ok_or("usage: pin|ignore <key> on|off")?.clone();
                let on = args.get(2).map(|s| s == "on").unwrap_or(true);
                let mut st = app.state.lock().await;
                let set = if cmd == "pin" { &mut st.pinned } else { &mut st.ignored };
                if on { set.insert(key.clone()); } else { set.remove(&key); }
                st.save().map_err(err)?;
                Ok(serde_json::json!({ cmd: key, "on": on }))
            }
            "export" => {
                let path = args.get(1).ok_or("usage: export <file.json>")?;
                let data = export_data(&app).await?;
                std::fs::write(path, serde_json::to_vec_pretty(&data).map_err(err)?).map_err(err)?;
                Ok(serde_json::json!({ "exported": data.addons.len(), "path": path }))
            }
            "selfcheck" => {
                let url = args.get(1).cloned().or_else(|| std::env::var("ADDONFORGE_UPDATE_URL").ok());
                let r = selfupdate::check(&app.client, url.as_deref()).await;
                Ok(serde_json::to_value(r).unwrap())
            }
            #[cfg(not(windows))]
            "selfupdate" => Err(format!("self-update is Windows-only; get the new version from {}", selfupdate::RELEASES_URL)),
            #[cfg(windows)]
            "selfupdate" => {
                let url = args.get(1).cloned().or_else(|| std::env::var("ADDONFORGE_UPDATE_URL").ok());
                let r = selfupdate::check(&app.client, url.as_deref()).await;
                let latest = r.latest.ok_or("no update info")?;
                let force = args.iter().any(|a| a == "--force");
                if !r.available && !force {
                    return Err("already up to date (use --force to test)".into());
                }
                let p = selfupdate::apply(&app.client, &latest).await.map_err(err)?;
                Ok(serde_json::json!({ "started": p }))
            }
            "diag" => {
                let st = app.state.lock().await.clone();
                Ok(serde_json::json!({ "state": st, "log_tail": log::tail(20) }))
            }
            _ => Err("usage: addonforge --cli installs | use <dir> | scan | check | catalog | resolve <id..>|all | update <key> [--force] | install <catalog-id> [--force] | github <owner/repo> [--force] | prerelease on|off | pin|ignore <key> on|off | export <file> | selfcheck [url] | selfupdate [url] [--force] | diag".into()),
        }
    });
    match res {
        Ok(v) => {
            out(v);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn run() {
    logi!("AddonForge v{} build {} starting", selfupdate::version(), selfupdate::build());
    let app = make_app();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app)
        .setup(|_app| {
            selfupdate::confirm_started();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            find_installs,
            live_folder_hint,
            dismiss_live_hint,
            backup_wtf,
            open_backups_folder,
            set_install_path,
            set_prefs,
            set_pin,
            set_ignore,
            scan,
            check_updates,
            update_package,
            catalog_list,
            bundles,
            curse_search,
            install_curse,
            package_icons,
            catalog_icons,
            addon_details,
            install_catalog,
            install_source,
            install_github,
            remove_package,
            open_addons_folder,
            open_log,
            export_list,
            import_list,
            check_self_update,
            skip_self_update,
            apply_self_update,
            whats_new,
            diagnostics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AddonForge");
}

pub fn note_replace(old: std::path::PathBuf, marker: Option<std::path::PathBuf>) {
    selfupdate::note_replace(old, marker);
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_folder_beats_beta_folder() {
        // Forward slashes: a separator on every platform CI runs on.
        let mk = |p: &str| WowInstall { path: p.into(), product: "x".into(), flavor: "forever".into(), label: "WoW: Forever".into() };
        let both = vec![mk("wow/_classic_beta_"), mk("wow/_forever_")];
        assert_eq!(preferred_forever(&both).unwrap().path, "wow/_forever_");
        let beta_only = vec![mk("wow/_classic_beta_")];
        assert_eq!(preferred_forever(&beta_only).unwrap().path, "wow/_classic_beta_");
        assert!(preferred_forever(&[]).is_none());
        assert_eq!(folder_name("wow/_Forever_"), "_forever_");
    }

    #[test]
    fn whats_new_joins_skipped_builds() {
        use selfupdate::{HistoryNote, Latest};
        let h = |b: u32, n: &str| HistoryNote { version: "0.4.0".into(), build: b, notes: n.into() };
        let l = Latest { version: "0.4.0".into(), build: 22, notes: "twenty-two".into(), history: vec![h(22, "twenty-two"), h(21, "twenty-one"), h(20, "twenty"), h(19, "nineteen")], ..Default::default() };
        let t = whats_new_text(&l, 19, 22);
        assert!(t.starts_with("Build 20 (v0.4.0)\ntwenty"), "{t}");
        assert!(t.contains("Build 21") && t.ends_with("twenty-two"));
        assert!(!t.contains("nineteen"));
        assert_eq!(whats_new_text(&l, 21, 22), "twenty-two");
        assert_eq!(whats_new_text(&l, 0, 22), "twenty-two", "no record of a previous build: just this one");
        let bare = Latest { build: 22, notes: "only".into(), ..Default::default() };
        assert_eq!(whats_new_text(&bare, 19, 22), "only");
    }

    #[test]
    fn stamp_is_sortable_and_sane() {
        let s = chrono_stamp();
        assert_eq!(s.len(), 13, "{s}");
        assert!(s.starts_with("20"), "{s}");
        assert_eq!(&s[8..9], "-");
    }

    #[test]
    fn chosen_source_respects_what_the_entry_offers() {
        let mut e = CatalogEntry::default();
        e.name = "Thing".into();
        e.github = Some("a/b".into());
        e.curse = Some(5);
        e.wago = Some("w".into());
        assert!(matches!(chosen_source(&e, "github"), Ok(Source::Github(r)) if r == "a/b"));
        assert!(chosen_source(&e, "wago").is_err(), "Wago is not a source any more");
        assert!(chosen_source(&e, "wowi").is_err(), "not listed");
        assert!(chosen_source(&e, "bogus").is_err());
        e.curse_locked = true;
        assert!(chosen_source(&e, "curse").is_err(), "opted-out authors stay link only");
    }

    use super::*;

    fn folder(name: &str, deps: &[&str], part_of: Option<&str>) -> AddonFolder {
        AddonFolder {
            folder: name.into(),
            title: name.into(),
            version: Some("1.0".into()),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            part_of: part_of.map(|s| s.to_string()),
            interfaces: vec![16001],
            forever_interface: true,
            ..Default::default()
        }
    }

    fn grouped_folder(name: &str, deps: &[&str], group: &str) -> AddonFolder {
        AddonFolder { group: Some(group.into()), ..folder(name, deps, None) }
    }

    fn dependency_catalog() -> Catalog {
        serde_json::from_str(r#"{"addons":[{
            "id":"atlas","name":"Atlas suite","github":"author/Atlas","folders":["Atlas"],
            "group_required_dependencies":true
        }]}"#).unwrap()
    }

    fn dependency_folders() -> Vec<AddonFolder> {
        let mut client = folder("Atlas", &["AtlasData"], None);
        client.version = Some("2.4".into());
        client.author = Some("Client author".into());
        let mut data = folder("AtlasData", &[], None);
        data.version = Some("7.1".into());
        data.author = Some("Data author".into());
        vec![client, data]
    }

    fn remote(version: &str) -> Remote {
        Remote {
            version: version.into(),
            download_url: "https://example.test/Atlas.zip".into(), filename: "Atlas.zip".into(),
            forever: Some(true), prerelease: false,
        }
    }


    #[test]
    fn source_installs_use_catalogue_identity_when_known() {
        let catalog = dependency_catalog();
        let mut state = AppState::default();
        let source = Source::Github("AUTHOR/aTlAs".into());
        assert_eq!(key_for_source(&catalog, &state, &source, None), "cat:atlas");
        assert_eq!(key_for_source(&catalog, &state, &source, Some("github:author/Atlas")), "cat:atlas");
        assert_eq!(key_for_source(&catalog, &state, &Source::Github("other/Atlas".into()), None), "github:other/Atlas");
        assert_eq!(key_for_source(&catalog, &state, &Source::Github("other/Atlas".into()), Some("custom:Atlas")), "custom:Atlas");
        assert_eq!(key_for_source(&catalog, &state, &Source::Wowi(123), None), "wowi:123");

        let recorded = Installed {
            source: "github".into(), source_id: "Author/ATLAS".into(),
            folders: vec!["Atlas".into()], ..Default::default()
        };
        state.installed.insert("github:author/Atlas".into(), recorded.clone());
        assert_eq!(key_for_source(&catalog, &state, &source, None), "github:author/Atlas");
        state.installed.insert("cat:atlas".into(), recorded.clone());
        assert_eq!(key_for_source(&catalog, &state, &source, None), "cat:atlas");
        assert_eq!(key_for_source(&catalog, &state, &source, Some("github:author/Atlas")), "github:author/Atlas");
        state.installed.insert("folder:Atlas".into(), recorded);
        assert_eq!(key_for_source(&catalog, &state, &source, Some("folder:Atlas")), "folder:Atlas");
    }

    #[test]
    fn catalogue_opt_in_groups_required_dependencies_under_the_primary() {
        let mut folders = dependency_folders();
        folders.reverse();
        folders[1].dependencies = vec!["ATLASDATA".into()];
        let packages = build_packages(&AppState::default(), &dependency_catalog(), &folders);
        assert_eq!(packages.len(), 1);
        let package = &packages[0];
        assert_eq!(package.key, "cat:atlas");
        assert_eq!(package.name, "Atlas suite");
        assert_eq!(package.author.as_deref(), Some("Client author"));
        assert_eq!(package.installed_version.as_deref(), Some("2.4"));
        assert_eq!(package.source, Some(Source::Github("author/Atlas".into())));
        assert_eq!(package.folders, vec!["Atlas", "AtlasData"]);
        assert_eq!(installed_catalog_ids(packages), vec!["atlas"]);
    }

    #[test]
    fn required_dependency_grouping_is_disabled_by_default() {
        let catalog: Catalog = serde_json::from_str(r#"{"addons":[{
            "id":"atlas","name":"Atlas","folders":["Atlas"]
        }]}"#).unwrap();
        assert!(!catalog.addons[0].group_required_dependencies);
        let packages = build_packages(&AppState::default(), &catalog, &dependency_folders());
        assert_eq!(packages.len(), 2);
        assert_eq!(packages.iter().find(|p| p.key == "cat:atlas").unwrap().folders, vec!["Atlas"]);
    }

    #[test]
    fn opt_in_groups_only_direct_non_blizzard_dependencies() {
        let mut folders = dependency_folders();
        folders[0].dependencies.extend(["Blizzard_Shared".into(), "MissingData".into()]);
        folders[1].dependencies.push("SharedStorage".into());
        for name in ["Blizzard_Shared", "OptionalAddon", "SharedStorage"] {
            folders.push(folder(name, &[], None));
        }
        folders.push(folder("IndependentTool", &["AtlasData"], None));
        let packages = build_packages(&AppState::default(), &dependency_catalog(), &folders);
        assert_eq!(packages.len(), 5);
        let bundle = packages.iter().find(|p| p.key == "cat:atlas").unwrap();
        assert_eq!(bundle.folders, vec!["Atlas", "AtlasData"]);
        assert_eq!(bundle.missing_deps.len(), 1);
        assert_eq!(bundle.missing_deps[0].folder, "MissingData");
        assert_eq!(packages.iter().find(|p| p.key == "folder:IndependentTool").unwrap().folders, vec!["IndependentTool"]);
    }

    #[test]
    fn managed_package_keys_preserve_catalogue_membership() {
        for owner in ["cat:atlas", "github:author/Atlas", "folder:Atlas"] {
            let mut state = AppState::default();
            state.installed.insert(owner.into(), Installed {
                source: "github".into(), source_id: "author/Atlas".into(),
                folders: vec!["Atlas".into()], version: "recorded-release".into(), ..Default::default()
            });
            let packages = build_packages(&state, &dependency_catalog(), &dependency_folders());
            assert_eq!(packages.len(), 1);
            assert_eq!(packages[0].key, owner);
            assert_eq!(packages[0].folders, vec!["Atlas", "AtlasData"]);
            assert_eq!(packages[0].installed_version.as_deref(), Some("recorded-release"));
            let installed_ids = installed_catalog_ids(packages);
            assert_eq!(installed_ids, vec!["atlas"]);
        }
    }

    #[test]
    fn dependency_modules_use_existing_grouping() {
        let mut folders = dependency_folders();
        folders.push(grouped_folder("AtlasOptions", &["Atlas"], "Atlas"));
        folders.push(grouped_folder("AtlasDataOptions", &["AtlasData"], "AtlasData"));
        folders.push(grouped_folder("AtlasDataLayout", &["AtlasDataOptions"], "AtlasDataOptions"));
        folders.reverse();
        let packages = build_packages(&AppState::default(), &dependency_catalog(), &folders);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].folders, vec!["Atlas", "AtlasData", "AtlasDataLayout", "AtlasDataOptions", "AtlasOptions"]);
        assert_eq!(packages[0].installed_version.as_deref(), Some("2.4"));
    }

    #[test]
    fn required_dependency_grouping_preserves_existing_module_ownership() {
        for owner in ["catalogue", "managed", "standalone"] {
            for hint in ["part_of", "group", "stem"] {
                let mut catalog = dependency_catalog();
                let mut state = AppState::default();
                let expected_key = match owner {
                    "catalogue" => {
                        catalog.addons.push(serde_json::from_str(r#"{
                            "id":"other","name":"Other addon","folders":["OtherRoot"]
                        }"#).unwrap());
                        "cat:other"
                    }
                    "managed" => {
                        state.installed.insert("github:provider/Other".into(), Installed {
                            source: "github".into(), source_id: "provider/Other".into(),
                            folders: vec!["OtherRoot".into()], ..Default::default()
                        });
                        "github:provider/Other"
                    }
                    _ => "folder:OtherRoot",
                };
                let module_name = if hint == "stem" { "OtherRoot_Data" } else { "SharedData" };
                let options_name = format!("{module_name}Options");
                let mut module = folder(module_name, &["OtherRoot"], None);
                match hint {
                    "part_of" => module.part_of = Some("OtherRoot".into()),
                    "group" => module.group = Some("OtherRoot".into()),
                    _ => {}
                }
                let folders = vec![
                    folder("Atlas", &[module_name], None),
                    module,
                    grouped_folder(&options_name, &[module_name], module_name),
                    folder("OtherRoot", &[], None),
                ];
                let packages = build_packages(&state, &catalog, &folders);
                assert_eq!(packages.len(), 2, "{owner}: {hint}");
                let bundle = packages.iter().find(|p| p.key == "cat:atlas").unwrap();
                assert_eq!(bundle.folders, vec!["Atlas"], "{owner}: {hint}");
                let other = packages.iter().find(|p| p.key == expected_key).unwrap();
                assert_eq!(other.folders, vec!["OtherRoot".to_string(), module_name.into(), options_name]);
                let protected = protected_package_folders(&state, &packages, &bundle.key);
                assert_eq!(protected.get(&module_name.to_ascii_lowercase()).map(String::as_str), Some(expected_key));
            }
        }
    }

    #[test]
    fn dependency_alone_does_not_identify_a_bundle() {
        let mut catalog = dependency_catalog();
        let mut folders = dependency_folders()[1..].to_vec();
        let packages = build_packages(&AppState::default(), &catalog, &folders);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].key, "folder:AtlasData");
        assert_eq!(packages[0].name, "AtlasData");
        assert_eq!(packages[0].installed_version.as_deref(), Some("7.1"));
        assert!(packages[0].catalog_id.is_none());
        assert!(packages[0].source.is_none());
        assert!(installed_catalog_ids(packages).is_empty());

        // Another identity folder cannot activate grouping without the primary.
        catalog.addons[0].folders.push("AtlasOptions".into());
        folders.push(folder("AtlasOptions", &["AtlasData"], None));
        let packages = build_packages(&AppState::default(), &catalog, &folders);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages.iter().find(|p| p.key == "folder:AtlasData").unwrap().folders, vec!["AtlasData"]);
    }

    #[test]
    fn identified_dependencies_remain_independent() {
        for catalogued in [false, true] {
            let mut catalog = dependency_catalog();
            let mut folders = dependency_folders();
            let expected = if catalogued {
                catalog.addons.push(serde_json::from_str(r#"{
                    "id":"atlas-data","name":"Atlas Data","folders":["AtlasData"]
                }"#).unwrap());
                "cat:atlas-data"
            } else {
                folders[1].wago = Some("data-project".into());
                "wago:data-project"
            };
            let packages = build_packages(&AppState::default(), &catalog, &folders);
            assert_eq!(packages.len(), 2);
            assert_eq!(packages.iter().find(|p| p.key == "cat:atlas").unwrap().folders, vec!["Atlas"]);
            assert_eq!(packages.iter().find(|p| p.key == expected).unwrap().folders, vec!["AtlasData"]);
        }
    }

    #[test]
    fn ambiguous_required_dependencies_stay_independent() {
        let mut catalog = dependency_catalog();
        catalog.addons.push(serde_json::from_str(r#"{
            "id":"survey","name":"Survey","folders":["Survey"],"group_required_dependencies":true
        }"#).unwrap());
        let mut folders = dependency_folders();
        folders.push(folder("Survey", &["AtlasData"], None));
        for _ in 0..2 {
            let packages = build_packages(&AppState::default(), &catalog, &folders);
            assert_eq!(packages.len(), 3);
            assert_eq!(packages.iter().find(|p| p.key == "folder:AtlasData").unwrap().folders, vec!["AtlasData"]);
            catalog.addons.reverse();
        }
    }

    #[test]
    fn grouped_packages_use_recorded_provider_versions_for_updates() {
        // A package recorded as installed from Wago keeps its recorded version
        // but is checked against the catalogue's GitHub source now (build 25).
        for (kind, id, installed, available, expect_kind, expect_id) in [
            ("github", "author/Atlas", "Atlas-v2.4+v7.1", "Atlas-v2.4+v7.2", "github", "author/Atlas"),
            ("wago", "atlas-project", "v2.4+v7.1", "v2.4+v7.2", "github", "author/Atlas"),
        ] {
            let mut state = AppState::default();
            state.installed.insert("cat:atlas".into(), Installed {
                source: kind.into(), source_id: id.into(), version: installed.into(),
                folders: vec!["Atlas".into(), "AtlasData".into()], ..Default::default()
            });
            let package = build_packages(&state, &dependency_catalog(), &dependency_folders()).pop().unwrap();
            assert_eq!(package.installed_version.as_deref(), Some(installed));
            assert_eq!(package.source, Source::from_parts(expect_kind, expect_id));
            for (version, update) in [(installed, false), (available, true)] {
                let mut checked = package.clone();
                apply_remote(&mut checked, remote(version), None);
                assert_eq!(checked.update_available, update, "{kind}: {version}");
                assert_eq!(checked.status, if update { "update" } else { "ok" });
                assert_eq!(checked.remote_version.as_deref(), Some(version));
            }
        }
    }

    #[test]
    fn independently_managed_dependencies_keep_their_source_version_and_folders() {
        for owner in ["github:provider/AtlasData", "folder:AtlasData"] {
            let mut state = AppState::default();
            state.installed.insert(owner.into(), Installed {
                source: "github".into(), source_id: "provider/AtlasData".into(),
                version: "provider-8".into(), folders: vec!["AtlasData".into()],
                ..Default::default()
            });
            let mut folders = dependency_folders();
            folders[1].group = Some("Atlas".into());
            folders[1].dependencies = vec!["Atlas".into()];
            folders.push(grouped_folder("AtlasDataOptions", &["AtlasData"], "AtlasData"));
            let packages = build_packages(&state, &dependency_catalog(), &folders);
            assert_eq!(packages.len(), 2, "{owner}");
            let bundle = packages.iter().find(|p| p.key == "cat:atlas").unwrap();
            assert_eq!(bundle.folders, vec!["Atlas"]);
            let provider = packages.iter().find(|p| p.key == owner).unwrap();
            assert_eq!(provider.folders, vec!["AtlasData", "AtlasDataOptions"]);
            assert_eq!(provider.installed_version.as_deref(), Some("provider-8"));
            assert_eq!(provider.source, Some(Source::Github("provider/AtlasData".into())));
            assert!(provider.managed);
            let protected = protected_package_folders(&state, &packages, &bundle.key);
            assert_eq!(protected.get("atlasdata").map(String::as_str), Some(owner));
            assert!(!protected.contains_key("atlas"));
        }
    }

    #[test]
    fn installing_missing_dependencies_transfers_ownership_and_allows_later_updates() {
        use std::fs;
        use std::io::Write;

        let root = std::env::temp_dir().join(format!("af-transfer-{}-{}", std::process::id(), state::now_secs()));
        let addons = root.join("Interface").join("AddOns");
        fs::create_dir_all(addons.join("OtherData")).unwrap();
        fs::write(addons.join("OtherData").join("OtherData.toc"), "## Version: 7.1\n").unwrap();
        fs::write(addons.join("OtherData").join("keep.lua"), "keep other data").unwrap();
        let partial_owner = "github:provider/AtlasData";
        let stale_owner = "folder:AtlasLayout";
        let mut state = AppState::default();
        state.installed.insert(partial_owner.into(), Installed {
            source: "github".into(), source_id: "provider/AtlasData".into(),
            version: "provider-7.1".into(), folders: vec!["AtlasData".into(), "OtherData".into()],
            installed_at: 11,
        });
        state.installed.insert(stale_owner.into(), Installed {
            folders: vec!["aTlAsLaYoUt".into()], ..Default::default()
        });
        state.installed.insert("folder:Absent".into(), Installed {
            folders: vec!["Absent".into()], ..Default::default()
        });
        state.pinned.insert(stale_owner.into());
        state.pinned.insert(partial_owner.into());
        state.ignored.insert(stale_owner.into());
        state.ignored.insert("folder:Absent".into());
        let pinned = state.pinned.clone();
        let ignored = state.ignored.clone();
        let catalog = dependency_catalog();

        for (release, data_version) in [("Atlas-v2.4+v7.1", "7.1"), ("Atlas-v2.4+v7.2", "7.2")] {
            let packages = build_packages(&state, &catalog, &wow::scan_addons(&root));
            let protected = protected_package_folders(&state, &packages, "cat:atlas");
            let zip_path = root.join("release.zip");
            let mut archive = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            for (name, contents) in [
                ("Atlas/Atlas.toc", "## Version: 2.4\n## RequiredDeps: AtlasData, AtlasLayout\n".into()),
                ("AtlasData/AtlasData.toc", format!("## Version: {data_version}\n")),
                ("AtlasData/data.lua", format!("database {data_version}")),
                ("AtlasLayout/AtlasLayout.toc", "## Version: 1\n".into()),
            ] {
                archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                archive.write_all(contents.as_bytes()).unwrap();
            }
            archive.finish().unwrap();
            let mut folders = install::place(&zip_path, &addons, &protected).unwrap();
            folders.sort();
            assert_eq!(folders, vec!["Atlas", "AtlasData", "AtlasLayout"]);
            assert_eq!(fs::read_to_string(addons.join("AtlasData").join("data.lua")).unwrap(), format!("database {data_version}"));
            let installed = Installed {
                source: "github".into(), source_id: "author/Atlas".into(),
                version: release.into(), folders, installed_at: 22,
            };
            record_install(&mut state, "cat:atlas", &installed);

            let remaining = &state.installed[partial_owner];
            assert_eq!(remaining.folders, vec!["OtherData"]);
            assert_eq!(remaining.source, "github");
            assert_eq!(remaining.source_id, "provider/AtlasData");
            assert_eq!(remaining.version, "provider-7.1");
            assert_eq!(remaining.installed_at, 11);
            assert!(!state.installed.contains_key(stale_owner));
            assert_eq!(state.installed["folder:Absent"].folders, vec!["Absent"]);
            assert_eq!(state.installed.len(), 3);
            assert_eq!(state.pinned, pinned);
            assert_eq!(state.ignored, ignored);
            assert_eq!(fs::read_to_string(addons.join("OtherData").join("keep.lua")).unwrap(), "keep other data");

            let packages = build_packages(&state, &catalog, &wow::scan_addons(&root));
            assert_eq!(packages.len(), 2);
            let mut bundle = packages.into_iter().find(|p| p.key == "cat:atlas").unwrap();
            assert_eq!(bundle.folders, vec!["Atlas", "AtlasData", "AtlasLayout"]);
            assert_eq!(bundle.source, Some(Source::Github("author/Atlas".into())));
            assert_eq!(bundle.installed_version.as_deref(), Some(release));
            assert!(bundle.missing_deps.is_empty());
            apply_remote(&mut bundle, remote(release), None);
            assert!(!bundle.update_available);
            assert_eq!(bundle.status, "ok");
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn single_addon_updates_keep_legacy_version_matching() {
        let mut package = build_packages(&AppState::default(), &Catalog::default(), &[folder("Single", &[], None)]).pop().unwrap();
        apply_remote(&mut package, remote("v1.0"), None);
        assert_eq!(package.status, "ok");
        assert!(!package.update_available);
        apply_remote(&mut package, remote("1.1"), None);
        assert_eq!(package.status, "update");
        assert!(package.update_available);
    }

    #[test]
    fn group_and_required_dependency_group_ellesmere_modules() {
        let mut core = folder("EllesmereUI", &[], None);
        core.wago = Some("ZKbxbRN1".into());
        core.version = Some("9.6.7".into());
        let folders = vec![
            core,
            grouped_folder("EllesmereUIActionBars", &["EllesmereUI"], "EllesmereUI"),
            grouped_folder("EllesmereUILocales", &["ELLESMEREUI"], "ellesmereui"),
            grouped_folder("EllesmereUIOptions", &["EllesmereUI"], "EllesmereUI"),
        ];
        let pkgs = build_packages(&AppState::default(), &Catalog::default(), &folders);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].key, "wago:ZKbxbRN1", "the Wago id still names the package");
        assert_eq!(pkgs[0].name, "EllesmereUI");
        assert_eq!(pkgs[0].source, None, "but Wago is not a source any more");
        assert_eq!(pkgs[0].installed_version.as_deref(), Some("9.6.7"));
        assert_eq!(pkgs[0].folders, vec![
            "EllesmereUI", "EllesmereUIActionBars", "EllesmereUILocales", "EllesmereUIOptions",
        ]);
    }

    #[test]
    fn group_requires_an_installed_required_parent() {
        let folders = vec![
            folder("EllesmereUI", &[], None),
            folder("LibStub", &[], None),
            // A group alone does not establish folder ownership.
            grouped_folder("EllesmereUIExtra", &[], "EllesmereUI"),
            // A required dependency alone can be an independent addon/library.
            folder("EllesmereUIPlugin", &["EllesmereUI"], None),
            grouped_folder("OtherAddon", &["LibStub"], "EllesmereUI"),
            // Shared category labels and missing parents are not packages.
            grouped_folder("FirstTool", &["LibStub"], "Tools"),
            grouped_folder("SecondTool", &["LibStub"], "Tools"),
            grouped_folder("OrphanModule", &["MissingCore"], "MissingCore"),
        ];
        let pkgs = build_packages(&AppState::default(), &Catalog::default(), &folders);
        assert_eq!(pkgs.len(), folders.len());
        for f in &folders {
            let pkg = pkgs.iter().find(|p| p.key == format!("folder:{}", f.folder)).unwrap();
            assert_eq!(pkg.folders, vec![f.folder.clone()]);
        }
        let orphan = pkgs.iter().find(|p| p.name == "OrphanModule").unwrap();
        assert_eq!(orphan.missing_deps.len(), 1);
        assert_eq!(orphan.missing_deps[0].folder, "MissingCore");
    }

    #[test]
    fn group_preserves_explicit_package_ownership() {
        let mut st = AppState::default();
        st.installed.insert(
            "github:author/Plugin".into(),
            Installed {
                source: "github".into(), source_id: "author/Plugin".into(), version: "2.0".into(),
                folders: vec!["ManagedPlugin".into()], installed_at: 0,
            },
        );
        let cat: Catalog = serde_json::from_str(r#"{"addons":[
            {"id":"dbm","name":"Deadly Boss Mods","folders":["DBM-Core"]},
            {"id":"dbm-dungeons","name":"DBM Dungeons","folders":["DBM-Party-Forever"]}
        ]}"#).unwrap();
        let mut wago = grouped_folder("WagoPlugin", &["DBM-Core"], "DBM-Core");
        wago.wago = Some("another-project".into());
        let mut curse = grouped_folder("CursePlugin", &["DBM-Core"], "DBM-Core");
        curse.curse = Some(12345);
        let mut wowi = grouped_folder("WowiPlugin", &["DBM-Core"], "DBM-Core");
        wowi.wowi = Some(67890);
        let folders = vec![
            folder("DBM-Core", &[], None),
            grouped_folder("DBM-Party-Forever", &["DBM-Core"], "DBM-Core"),
            grouped_folder("ManagedPlugin", &["DBM-Core"], "DBM-Core"),
            wago, curse, wowi,
        ];
        let pkgs = build_packages(&st, &cat, &folders);
        assert_eq!(pkgs.len(), folders.len());
        for key in ["cat:dbm", "cat:dbm-dungeons", "github:author/Plugin",
            "wago:another-project", "curse:12345", "wowi:67890"] {
            let pkg = pkgs.iter().find(|p| p.key == key).unwrap();
            assert_eq!(pkg.folders.len(), 1, "{key}");
        }
        let managed = pkgs.iter().find(|p| p.key == "github:author/Plugin").unwrap();
        assert!(managed.managed);
        assert_eq!(managed.source, Some(Source::Github("author/Plugin".into())));
    }

    #[test]
    fn group_stops_at_an_identified_parent() {
        let mut st = AppState::default();
        st.installed.insert(
            // Updating an originally standalone addon retains its folder key.
            "folder:ManagedAddon".into(),
            Installed {
                source: "github".into(), source_id: "author/Managed".into(), version: "1.0".into(),
                folders: vec!["ManagedAddon".into()], installed_at: 0,
            },
        );
        let cat: Catalog = serde_json::from_str(r#"{"addons":[
            {"id":"catalogue-addon","name":"Catalogue addon","folders":["CatalogueAddon"]}
        ]}"#).unwrap();
        let mut project = grouped_folder("ProjectAddon", &["SharedFramework"], "SharedFramework");
        project.wago = Some("project-id".into());
        let folders = vec![
            folder("SharedFramework", &[], None),
            grouped_folder("CatalogueAddon", &["SharedFramework"], "SharedFramework"),
            grouped_folder("CatalogueOptions", &["CatalogueAddon"], "CatalogueAddon"),
            grouped_folder("ManagedAddon", &["SharedFramework"], "SharedFramework"),
            grouped_folder("ManagedOptions", &["ManagedAddon"], "ManagedAddon"),
            project,
            grouped_folder("ProjectOptions", &["ProjectAddon"], "ProjectAddon"),
        ];
        let pkgs = build_packages(&st, &cat, &folders);
        assert_eq!(pkgs.len(), 4);
        let framework = pkgs.iter().find(|p| p.key == "folder:SharedFramework").unwrap();
        assert_eq!(framework.folders, vec!["SharedFramework"]);
        for (key, parent, child) in [
            ("cat:catalogue-addon", "CatalogueAddon", "CatalogueOptions"),
            ("folder:ManagedAddon", "ManagedAddon", "ManagedOptions"),
            ("wago:project-id", "ProjectAddon", "ProjectOptions"),
        ] {
            let pkg = pkgs.iter().find(|p| p.key == key).unwrap();
            assert_eq!(pkg.folders, vec![parent, child], "{key}");
        }
        let managed = pkgs.iter().find(|p| p.key == "folder:ManagedAddon").unwrap();
        assert!(managed.managed);
        assert_eq!(managed.source, Some(Source::Github("author/Managed".into())));
        assert_eq!(managed.installed_version.as_deref(), Some("1.0"));
    }

    #[test]
    fn modules_group_under_their_core_without_catalogue() {
        let st = AppState::default();
        let cat = Catalog::default();
        let folders = vec![
            folder("DBM-Core", &[], None),
            folder("DBM-GUI", &["DBM-Core"], None),
            folder("DBM-StatusBarTimers", &["DBM-Core"], None),
            folder("BigWigs", &[], None),
            folder("BigWigs_Core", &["BigWigs"], None),
            folder("BigWigs_Sporefall", &["BigWigs_Core"], None),
            folder("Plater", &[], None),
            folder("LibStub", &[], None),
            folder("SomeAddon", &["LibStub"], None),
            folder("ModuleX", &[], Some("Plater")),
        ];
        let pkgs = build_packages(&st, &cat, &folders);
        let names: Vec<(String, usize)> = pkgs.iter().map(|p| (p.name.clone(), p.folders.len())).collect();
        assert!(names.contains(&("DBM-Core".into(), 3)), "{names:?}");
        assert!(names.contains(&("BigWigs".into(), 3)), "{names:?}");
        assert!(names.contains(&("Plater".into(), 2)), "{names:?}");
        assert!(names.contains(&("LibStub".into(), 1)), "{names:?}");
        assert!(names.contains(&("SomeAddon".into(), 1)), "{names:?}");
        assert_eq!(pkgs.len(), 5);
    }

    #[test]
    fn managed_folders_stay_with_their_package() {
        let mut st = AppState::default();
        st.installed.insert(
            "cat:bigwigs".into(),
            Installed { source: "github".into(), source_id: "x".into(), version: "v1".into(), folders: vec!["BigWigs".into(), "BigWigs_Zone".into()], installed_at: 0 },
        );
        let cat: Catalog = serde_json::from_str(r#"{"addons":[{"id":"bigwigs","name":"BigWigs","github":"a/b","folders":["BigWigs"]}]}"#).unwrap();
        let folders = vec![folder("BigWigs", &[], None), folder("BigWigs_Zone", &[], None)];
        let pkgs = build_packages(&st, &cat, &folders);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].folders.len(), 2);
        assert_eq!(pkgs[0].installed_version.as_deref(), Some("v1"));
        assert!(pkgs[0].managed);
    }

    #[test]
    fn missing_dependencies_are_reported_and_matched_to_catalogue() {
        let st = AppState::default();
        let cat: Catalog = serde_json::from_str(r#"{"addons":[{"id":"titan","name":"Titan Panel","github":"a/b","folders":["Titan"]}]}"#).unwrap();
        let folders = vec![
            folder("TitanReputation", &["Titan", "Blizzard_Foo", "!BugGrabber"], None),
            folder("BugSack", &["!BugGrabber"], None),
        ];
        let pkgs = build_packages(&st, &cat, &folders);
        let tr = pkgs.iter().find(|p| p.name == "TitanReputation").unwrap();
        let deps: Vec<&str> = tr.missing_deps.iter().map(|d| d.folder.as_str()).collect();
        assert_eq!(deps, vec!["Titan", "!BugGrabber"]);
        assert_eq!(tr.missing_deps[0].catalog_id.as_deref(), Some("titan"));
        assert!(tr.missing_deps[1].catalog_id.is_none());
    }

    #[test]
    fn managed_source_wins_and_github_key_is_used() {
        let mut st = AppState::default();
        st.installed.insert(
            "github:someone/Thing".into(),
            Installed { source: "github".into(), source_id: "someone/Thing".into(), version: "2.0".into(), folders: vec!["Thing".into()], installed_at: 0 },
        );
        let pkgs = build_packages(&st, &Catalog::default(), &[folder("Thing", &[], None)]);
        assert_eq!(pkgs[0].key, "github:someone/Thing");
        assert_eq!(pkgs[0].source, Some(Source::Github("someone/Thing".into())));
    }
}
