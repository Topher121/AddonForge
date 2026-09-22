//! AddonForge core: Tauri commands the UI calls over IPC.

mod catalog;
mod install;
mod sources;
mod state;
mod wow;

use catalog::{Catalog, CatalogEntry};
use serde::Serialize;
use sources::{normalize_version, ResolveOpts, Source};
use state::{AppState, Installed};
use std::collections::HashMap;
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

struct App {
    client: reqwest::Client,
    state: Mutex<AppState>,
    catalog: Mutex<Option<(Catalog, &'static str)>>,
}

type Shared = Arc<App>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- packages

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
    pub supports_forever: bool,
    /// True when AddonForge installed this itself (exact version known).
    pub managed: bool,
    pub remote_version: Option<String>,
    pub remote_forever: Option<bool>,
    pub update_available: bool,
    /// ok | update | unknown | curse-only | no-source | error | checking
    pub status: String,
    pub note: String,
}

fn addons_dir(install: &str) -> PathBuf {
    Path::new(install).join("Interface").join("AddOns")
}

fn pick_source(entry: Option<&CatalogEntry>, f: &AddonFolder, has_wago: bool) -> Option<Source> {
    if let Some(e) = entry {
        if let Some(r) = &e.github {
            return Some(Source::Github(r.clone()));
        }
        if has_wago {
            if let Some(w) = &e.wago {
                return Some(Source::Wago(w.clone()));
            }
        }
        if let Some(i) = e.wowi {
            return Some(Source::Wowi(i));
        }
        if let Some(t) = &e.tukui {
            return Some(Source::Tukui(t.clone()));
        }
        if let Some(w) = &e.wago {
            // Listed but user has no key: surface it so the UI can explain.
            return Some(Source::Wago(w.clone()));
        }
    }
    if let Some(w) = &f.wago {
        return Some(Source::Wago(w.clone()));
    }
    if let Some(i) = f.wowi {
        return Some(Source::Wowi(i));
    }
    if let Some(site) = &f.website {
        // "https://github.com/owner/repo" in X-Website is common.
        if let Some(rest) = site
            .trim()
            .trim_end_matches('/')
            .strip_prefix("https://github.com/")
            .or_else(|| site.trim().trim_end_matches('/').strip_prefix("http://github.com/"))
        {
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                return Some(Source::Github(format!("{}/{}", parts[0], parts[1])));
            }
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
    let has_wago = st.wago_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);

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
        if let Some(k) = managed_folder.get(&f.folder.to_ascii_lowercase()) {
            let entry = k.strip_prefix("cat:").and_then(|id| cat.addons.iter().find(|e| e.id == id));
            return ((*k).to_string(), entry);
        }
        let entry = by_folder
            .get(&f.folder.to_ascii_lowercase())
            .copied()
            .or_else(|| f.wago.as_deref().and_then(|w| by_wago.get(w).copied()))
            .or_else(|| f.curse.and_then(|c| by_curse.get(&c).copied()))
            .or_else(|| f.wowi.and_then(|i| by_wowi.get(&i).copied()));
        let key = if let Some(e) = entry {
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
        (key, entry)
    };

    // Does `f` look like a module of `parent` (e.g. BigWigs_Sporefall -> BigWigs,
    // DBM-GUI -> DBM-Core)? Needs a declared dependency and a shared name prefix.
    fn module_of<'a>(f: &AddonFolder, on_disk: &HashMap<String, &'a AddonFolder>) -> Option<&'a AddonFolder> {
        if let Some(p) = &f.part_of {
            if let Some(parent) = on_disk.get(&p.to_ascii_lowercase()) {
                return Some(parent);
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
    // Second pass: standalone folders that are modules of another addon inherit its key.
    let key_of: HashMap<String, (String, Option<&CatalogEntry>)> = keyed
        .iter()
        .map(|(k, e, f)| (f.folder.to_ascii_lowercase(), (k.clone(), *e)))
        .collect();
    for item in keyed.iter_mut() {
        if !item.0.starts_with("folder:") {
            continue;
        }
        let mut cur = item.2;
        // Walk up at most a few levels (module -> core -> catalogue entry).
        for _ in 0..4 {
            match module_of(cur, &on_disk) {
                Some(parent) => {
                    if let Some((k, e)) = key_of.get(&parent.folder.to_ascii_lowercase()) {
                        if !k.starts_with("folder:") || parent.folder != cur.folder {
                            item.0 = k.clone();
                            item.1 = *e;
                        }
                    }
                    cur = parent;
                }
                None => break,
            }
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
        // Primary folder: one nothing else in the group depends on being a
        // module of (the root: DBM-Core over DBM-GUI), then the shortest name
        // (BigWigs over BigWigs_Core).
        let in_group: Vec<String> = fs.iter().map(|f| f.folder.to_ascii_lowercase()).collect();
        let is_root = |f: &AddonFolder| {
            !f.dependencies.iter().any(|d| in_group.contains(&d.to_ascii_lowercase()))
                && f.part_of.as_ref().map(|p| !in_group.contains(&p.to_ascii_lowercase())).unwrap_or(true)
        };
        let primary = fs
            .iter()
            .min_by_key(|f| (!is_root(f), f.folder.len(), f.folder.clone()))
            .copied()
            .unwrap();
        let managed = st.installed.get(&key);
        let name = entry
            .map(|e| e.name.clone())
            .unwrap_or_else(|| primary.title.clone());
        let installed_version = managed
            .map(|m| m.version.clone())
            .or_else(|| fs.iter().find_map(|f| f.version.clone()))
            .filter(|v| !v.is_empty() && !v.starts_with('@'));
        let source = pick_source(entry, primary, has_wago).or_else(|| {
            fs.iter().find_map(|f| pick_source(None, f, has_wago))
        });
        let curse_id = entry.and_then(|e| e.curse).or_else(|| fs.iter().find_map(|f| f.curse));
        let supports_forever = fs.iter().any(|f| f.supports_forever());
        let (status, note) = match &source {
            Some(Source::Wago(_)) if !has_wago => (
                "no-key".to_string(),
                "On Wago: add your free Wago API key in Settings to update".to_string(),
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
                .or_else(|| curse_id.map(|c| format!("https://www.curseforge.com/projects/{c}"))),
            source,
            curse_id,
            catalog_id: entry.map(|e| e.id.clone()),
            supports_forever,
            managed: managed.is_some(),
            remote_version: None,
            remote_forever: None,
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
        *guard = Some(catalog::load(&app.client).await);
    }
    guard.as_ref().unwrap().0.clone()
}

async fn resolve_into(app: &App, pkg: &mut Package, cat: &Catalog) {
    let Some(src) = pkg.source.clone() else { return };
    let st = app.state.lock().await.clone();
    if matches!(src, Source::Wago(_)) && st.wago_key.as_deref().map(|k| k.trim().is_empty()).unwrap_or(true) {
        return;
    }
    let entry = pkg
        .catalog_id
        .as_ref()
        .and_then(|id| cat.addons.iter().find(|e| &e.id == id));
    let hint = entry.and_then(|e| e.asset_hint.clone());
    let opts = ResolveOpts { asset_hint: hint.as_deref(), wago_key: st.wago_key.as_deref() };
    match sources::resolve(&app.client, &src, opts).await {
        Ok(mut r) => {
            // Some authors list 16001 in the TOC but never added "forever" to
            // release.json. If the catalogue vouches for it, don't scare people.
            if r.forever == Some(false) && entry.and_then(|e| e.forever) == Some(true) {
                r.forever = None;
            }
            let same = pkg
                .installed_version
                .as_deref()
                .map(|v| normalize_version(v) == normalize_version(&r.version))
                .unwrap_or(false);
            pkg.update_available = !same;
            pkg.status = if same { "ok".into() } else { "update".into() };
            pkg.note = match r.forever {
                Some(false) => "Latest release does not list a Forever build".into(),
                _ => String::new(),
            };
            pkg.remote_forever = r.forever;
            pkg.remote_version = Some(r.version);
        }
        Err(e) => {
            pkg.status = "error".into();
            pkg.note = e.to_string();
        }
    }
}

async fn check_all(app: Shared, pkgs: &mut [Package], cat: &Catalog) {
    let sem = Arc::new(Semaphore::new(6));
    let mut set = tokio::task::JoinSet::new();
    for (i, p) in pkgs.iter().enumerate() {
        if p.source.is_none() {
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
    install_path: Option<String>,
    install: Option<WowInstall>,
    has_wago_key: bool,
    state_file: String,
    catalog_source: Option<String>,
    catalog_updated: Option<String>,
    catalog_count: usize,
    wago_key_url: String,
}

#[tauri::command]
async fn get_settings(app: State<'_, Shared>) -> Result<Settings, String> {
    let st = app.state.lock().await.clone();
    let install = st.install_path.as_deref().and_then(|p| wow::describe_install(Path::new(p)));
    let cat = app.catalog.lock().await.clone();
    Ok(Settings {
        version: env!("CARGO_PKG_VERSION").into(),
        install_path: st.install_path.clone(),
        install,
        has_wago_key: st.wago_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false),
        state_file: AppState::path().to_string_lossy().to_string(),
        catalog_source: cat.as_ref().map(|c| c.1.to_string()),
        catalog_updated: cat.as_ref().map(|c| c.0.updated.clone()),
        catalog_count: cat.as_ref().map(|c| c.0.addons.len()).unwrap_or(0),
        wago_key_url: sources::wago::KEY_URL.into(),
    })
}

#[tauri::command]
async fn find_installs(app: State<'_, Shared>) -> Result<Vec<WowInstall>, String> {
    let found = tokio::task::spawn_blocking(wow::find_installs).await.map_err(err)?;
    // Auto-pick the first Forever install if nothing is set yet.
    let mut st = app.state.lock().await;
    if st.install_path.is_none() {
        if let Some(f) = found.iter().find(|i| i.flavor == "forever") {
            st.install_path = Some(f.path.clone());
            st.save().map_err(err)?;
        }
    }
    Ok(found)
}

#[tauri::command]
async fn set_install_path(app: State<'_, Shared>, path: String) -> Result<WowInstall, String> {
    let p = PathBuf::from(path.trim());
    let info = wow::describe_install(&p)
        .ok_or_else(|| "That folder doesn't look like a WoW product folder (expected e.g. ...\\_classic_beta_ with Interface\\AddOns inside)".to_string())?;
    let mut st = app.state.lock().await;
    st.install_path = Some(info.path.clone());
    st.save().map_err(err)?;
    Ok(info)
}

#[tauri::command]
async fn set_wago_key(app: State<'_, Shared>, key: String) -> Result<bool, String> {
    let mut st = app.state.lock().await;
    let k = key.trim().to_string();
    st.wago_key = if k.is_empty() { None } else { Some(k) };
    st.save().map_err(err)?;
    Ok(st.wago_key.is_some())
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
    let opts = ResolveOpts { asset_hint, wago_key: st.wago_key.as_deref() };
    let remote = sources::resolve(&app.client, src, opts).await.map_err(err)?;
    let vouched = match key.strip_prefix("cat:") {
        Some(id) => catalog_cached(app)
            .await
            .addons
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.forever),
        None => None,
    };
    if remote.forever == Some(false) && !allow_non_forever && vouched != Some(true) {
        return Err("NOT_FOREVER".into());
    }
    let tmp = addons.join(install::TMP_DIR).join("dl");
    let zip = install::download(&app.client, &remote.download_url, &tmp, &remote.filename)
        .await
        .map_err(err)?;
    let addons2 = addons.clone();
    let folders = tokio::task::spawn_blocking(move || install::place(&zip, &addons2))
        .await
        .map_err(err)?
        .map_err(err)?;
    let rec = Installed {
        source: src.kind().into(),
        source_id: src.id(),
        version: remote.version.clone(),
        folders,
        installed_at: state::now_secs(),
    };
    let mut st = app.state.lock().await;
    st.installed.insert(key.to_string(), rec.clone());
    st.save().map_err(err)?;
    Ok(rec)
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
    let hint = pkg
        .catalog_id
        .as_ref()
        .and_then(|id| cat.addons.iter().find(|e| &e.id == id))
        .and_then(|e| e.asset_hint.clone());
    install_from(&app, &key, &src, hint.as_deref(), allow_non_forever.unwrap_or(false)).await?;
    // Return the refreshed row.
    let mut pkgs = scan_packages(&app).await?;
    let mut p = pkgs
        .drain(..)
        .find(|p| p.key == key)
        .ok_or_else(|| "Installed, but the folder vanished on rescan".to_string())?;
    resolve_into(&app, &mut p, &cat).await;
    Ok(p)
}

#[derive(Serialize, Clone)]
struct CatalogRow {
    #[serde(flatten)]
    entry: CatalogEntry,
    installed: bool,
    key: String,
}

#[tauri::command]
async fn catalog_list(app: State<'_, Shared>, refresh: Option<bool>) -> Result<Vec<CatalogRow>, String> {
    if refresh.unwrap_or(false) {
        *app.catalog.lock().await = None;
    }
    let cat = catalog_cached(&app).await;
    let installed_keys: Vec<String> = match scan_packages(&app).await {
        Ok(p) => p.into_iter().map(|p| p.key).collect(),
        Err(_) => Vec::new(),
    };
    Ok(cat
        .addons
        .into_iter()
        .map(|e| {
            let key = format!("cat:{}", e.id);
            CatalogRow { installed: installed_keys.contains(&key), key, entry: e }
        })
        .collect())
}

#[tauri::command]
async fn install_catalog(
    app: State<'_, Shared>,
    id: String,
    allow_non_forever: Option<bool>,
) -> Result<Package, String> {
    let cat = catalog_cached(&app).await;
    let e = cat
        .addons
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or_else(|| "Not in the catalogue".to_string())?;
    let st = app.state.lock().await.clone();
    let has_wago = st.wago_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);
    let dummy = AddonFolder::default();
    let src = pick_source(Some(&e), &dummy, has_wago)
        .ok_or_else(|| "This catalogue entry has no downloadable source".to_string())?;
    if matches!(src, Source::Wago(_)) && !has_wago {
        return Err("This addon is on Wago: add your free Wago API key in Settings first".into());
    }
    let key = format!("cat:{}", e.id);
    install_from(&app, &key, &src, e.asset_hint.as_deref(), allow_non_forever.unwrap_or(false)).await?;
    let mut pkgs = scan_packages(&app).await?;
    let mut p = pkgs
        .drain(..)
        .find(|p| p.key == key)
        .ok_or_else(|| "Installed, but its folders didn't match the catalogue entry; rescan".to_string())?;
    resolve_into(&app, &mut p, &cat).await;
    Ok(p)
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
    st.save().map_err(err)?;
    Ok(gone)
}

#[tauri::command]
async fn open_addons_folder(app: State<'_, Shared>) -> Result<String, String> {
    let install = current_install(&app).await?;
    let dir = addons_dir(&install);
    std::process::Command::new("explorer").arg(&dir).spawn().map_err(err)?;
    Ok(dir.to_string_lossy().to_string())
}

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
                // Dry run: what would we download for these catalogue ids (or "all")?
                let cat = catalog_cached(&app).await;
                let st = app.state.lock().await.clone();
                let has_wago = st.wago_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);
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
                    let key = st.wago_key.clone();
                    set.spawn(async move {
                        let _p = sem.acquire().await;
                        let src = pick_source(Some(&e), &AddonFolder::default(), has_wago);
                        let r = match &src {
                            Some(s) => sources::resolve(
                                &app.client,
                                s,
                                ResolveOpts { asset_hint: e.asset_hint.as_deref(), wago_key: key.as_deref() },
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
                let hint = pkg
                    .catalog_id
                    .as_ref()
                    .and_then(|id| cat.addons.iter().find(|e| &e.id == id))
                    .and_then(|e| e.asset_hint.clone());
                let force = args.iter().any(|a| a == "--force");
                let rec = install_from(&app, key, &src, hint.as_deref(), force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            "install" => {
                let id = args.get(1).ok_or("usage: install <catalog id>")?;
                let cat = catalog_cached(&app).await;
                let e = cat.addons.iter().find(|e| &e.id == id).cloned().ok_or("not in catalogue")?;
                let st = app.state.lock().await.clone();
                let has_wago = st.wago_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false);
                let src = pick_source(Some(&e), &AddonFolder::default(), has_wago).ok_or("no source")?;
                let force = args.iter().any(|a| a == "--force");
                let rec = install_from(&app, &format!("cat:{}", e.id), &src, e.asset_hint.as_deref(), force).await?;
                Ok(serde_json::to_value(rec).unwrap())
            }
            _ => Err("usage: addonforge --cli installs | use <dir> | scan | check | catalog | resolve <id..>|all | update <key> [--force] | install <catalog-id> [--force]".into()),
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
    let app = make_app();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app)
        .invoke_handler(tauri::generate_handler![
            get_settings,
            find_installs,
            set_install_path,
            set_wago_key,
            scan,
            check_updates,
            update_package,
            catalog_list,
            install_catalog,
            remove_package,
            open_addons_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AddonForge");
}

#[cfg(test)]
mod tests {
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
}
