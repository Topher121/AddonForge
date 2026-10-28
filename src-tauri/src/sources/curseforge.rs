//! CurseForge (api.curseforge.com), 3rd Party API.
//!
//! The API key is baked into release builds (see `build.rs`); builds without
//! one have CurseForge switched off and every function here says so. Rules we
//! keep because their terms ask for them: nothing from the API is written to
//! disk (the only cache is in memory for a few minutes), there is no relay
//! server, authors who disallow third-party distribution are never downloaded
//! around (they stay link-only), and calls are kept to a minimum (one batch
//! request per update check, one request per search).

use super::Remote;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/cf_key.rs"));

pub const BASE: &str = "https://api.curseforge.com/v1";
/// World of Warcraft's game id on CurseForge.
pub const GAME_ID: u64 = 1;
/// The "WoW Forever" game version type (slug `wow-forever`, version 1.60.1).
pub const FOREVER_TYPE: u64 = 88568;
pub const OPT_OUT: &str = "CURSE_OPTOUT";
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// The key, if this build has one.
pub fn key() -> Option<String> {
    if CF_KEY_MASKED.is_empty() {
        return None;
    }
    String::from_utf8(CF_KEY_MASKED.iter().map(|b| b ^ CF_KEY_MASK).collect()).ok()
}

pub fn available() -> bool {
    key().is_some()
}

pub fn project_url(id: u64) -> String {
    format!("https://www.curseforge.com/projects/{id}")
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub allow_mod_distribution: Option<bool>,
    #[serde(default)]
    pub links: Links,
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub latest_files: Vec<FileInfo>,
    #[serde(default)]
    pub latest_files_indexes: Vec<FileIndex>,
    #[serde(default)]
    pub categories: Vec<Category>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Category {
    #[serde(default)]
    pub name: String,
}

/// How a list page is ordered. Maps to CurseForge's `sortField`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sort {
    Popularity,
    Downloads,
    Updated,
    Name,
}

impl Sort {
    pub fn parse(s: &str) -> Sort {
        match s {
            "downloads" => Sort::Downloads,
            "updated" => Sort::Updated,
            "name" => Sort::Name,
            _ => Sort::Popularity,
        }
    }
    fn query(self) -> &'static str {
        match self {
            Sort::Popularity => "sortField=2&sortOrder=desc",
            Sort::Downloads => "sortField=6&sortOrder=desc",
            Sort::Updated => "sortField=3&sortOrder=desc",
            Sort::Name => "sortField=4&sortOrder=asc",
        }
    }
}

/// One request's worth of the list: text filter (may be empty), order,
/// one CurseForge category id, and the offset into the result set.
#[derive(Clone, Debug)]
pub struct Page {
    pub query: String,
    pub sort: Sort,
    pub category: Option<u64>,
    pub index: u32,
    pub limit: usize,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Links {
    #[serde(default)]
    pub website_url: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Author {
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub id: u64,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub file_name: String,
    /// 1 = release, 2 = beta, 3 = alpha.
    #[serde(default)]
    pub release_type: u8,
    #[serde(default)]
    pub file_date: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub sortable_game_versions: Vec<SortableVersion>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SortableVersion {
    #[serde(default)]
    pub game_version_type_id: Option<u64>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileIndex {
    #[serde(default)]
    pub file_id: u64,
    #[serde(default)]
    pub release_type: u8,
    #[serde(default)]
    pub game_version_type_id: Option<u64>,
}

#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Deserialize)]
struct Paged<T> {
    data: T,
    #[serde(default)]
    pagination: Pagination,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Pagination {
    #[serde(default)]
    total_count: u64,
}

fn cache() -> &'static Mutex<HashMap<u64, (Instant, ModInfo)>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, (Instant, ModInfo)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(id: u64) -> Option<ModInfo> {
    let map = cache().lock().ok()?;
    map.get(&id).filter(|(t, _)| t.elapsed() < CACHE_TTL).map(|(_, m)| m.clone())
}

fn remember(mods: &[ModInfo]) {
    if let Ok(mut map) = cache().lock() {
        let now = Instant::now();
        for m in mods {
            map.insert(m.id, (now, m.clone()));
        }
    }
}

fn request(client: &reqwest::Client, method: reqwest::Method, url: String) -> anyhow::Result<reqwest::RequestBuilder> {
    let key = key().ok_or_else(|| anyhow::anyhow!("CurseForge isn't available in this build"))?;
    Ok(client
        .request(method, url)
        .header("x-api-key", key)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(20)))
}

fn api_error(status: u16) -> Option<anyhow::Error> {
    match status {
        401 | 403 => Some(anyhow::anyhow!("CurseForge rejected the API key")),
        404 => Some(anyhow::anyhow!("CurseForge has no such addon")),
        429 => Some(anyhow::anyhow!("CurseForge rate limit hit; try again in a minute")),
        _ => None,
    }
}

pub async fn fetch(client: &reqwest::Client, id: u64) -> anyhow::Result<ModInfo> {
    if let Some(m) = cached(id) {
        return Ok(m);
    }
    let resp = request(client, reqwest::Method::GET, format!("{BASE}/mods/{id}"))?.send().await?;
    if let Some(e) = api_error(resp.status().as_u16()) {
        return Err(e);
    }
    let m: Data<ModInfo> = resp.error_for_status()?.json().await?;
    remember(std::slice::from_ref(&m.data));
    Ok(m.data)
}

/// One request for many ids; what comes back is kept in memory for a while.
pub async fn prefetch(client: &reqwest::Client, ids: &[u64]) -> anyhow::Result<usize> {
    let want: Vec<u64> = ids.iter().copied().filter(|id| cached(*id).is_none()).collect();
    if want.is_empty() {
        return Ok(0);
    }
    let mut total = 0;
    for chunk in want.chunks(50) {
        let resp = request(client, reqwest::Method::POST, format!("{BASE}/mods"))?
            .json(&serde_json::json!({ "modIds": chunk, "filterPcOnly": true }))
            .send()
            .await?;
        if let Some(e) = api_error(resp.status().as_u16()) {
            return Err(e);
        }
        let m: Data<Vec<ModInfo>> = resp.error_for_status()?.json().await?;
        remember(&m.data);
        total += m.data.len();
    }
    Ok(total)
}

/// Forever addons matching `query`, most popular first. Not cached.
pub async fn search(client: &reqwest::Client, query: &str, limit: usize) -> anyhow::Result<Vec<ModInfo>> {
    let page = Page { query: query.into(), sort: Sort::Popularity, category: None, index: 0, limit };
    Ok(search_page(client, &page).await?.0)
}

/// One page of Forever addons plus CurseForge's total count for the
/// filter. An empty query is the whole list. One request per call,
/// nothing kept beyond the 15-minute in-memory mod cache.
pub async fn search_page(client: &reqwest::Client, page: &Page) -> anyhow::Result<(Vec<ModInfo>, u64)> {
    let q = page.query.trim();
    if !q.is_empty() && q.len() < 2 {
        return Ok((Vec::new(), 0));
    }
    let mut url = format!(
        "{BASE}/mods/search?gameId={GAME_ID}&gameVersionTypeId={FOREVER_TYPE}&{}&pageSize={}&index={}",
        page.sort.query(),
        page.limit.clamp(1, 50),
        page.index.min(9950)
    );
    if let Some(c) = page.category {
        url.push_str(&format!("&categoryId={c}"));
    }
    if !q.is_empty() {
        url.push_str("&searchFilter=");
        url.push_str(&urlencoding(q));
    }
    let resp = request(client, reqwest::Method::GET, url)?.send().await?;
    if let Some(e) = api_error(resp.status().as_u16()) {
        return Err(e);
    }
    let m: Paged<Vec<ModInfo>> = resp.error_for_status()?.json().await?;
    remember(&m.data);
    Ok((m.data, m.pagination.total_count))
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// What `pick_file` decided about a mod.
#[derive(Debug, PartialEq)]
pub enum Pick {
    /// A Forever file with a download link was in the mod's `latestFiles`.
    Ready(Remote),
    /// The Forever file exists (by index) but its details need one more request.
    NeedsFile(u64),
    /// The author has switched off third-party downloads.
    OptOut,
    /// Nothing published for Forever.
    NoForeverFile,
}

fn release_order(prerelease: bool) -> [u8; 3] {
    if prerelease { [2, 1, 3] } else { [1, 2, 3] }
}

fn is_forever(f: &FileInfo) -> bool {
    f.sortable_game_versions.iter().any(|v| v.game_version_type_id == Some(FOREVER_TYPE))
}

fn remote_from(f: &FileInfo) -> Option<Remote> {
    let url = f.download_url.clone().filter(|u| u.starts_with("https://"))?;
    Some(Remote {
        version: f.display_name.clone(),
        download_url: url,
        filename: if f.file_name.is_empty() { format!("curse-{}.zip", f.id) } else { f.file_name.clone() },
        forever: Some(true),
        prerelease: f.release_type != 1,
    })
}

/// Choose the Forever file for a mod. Stable first unless the user allows
/// pre-releases; the newest file of the chosen kind wins.
pub fn pick_file(m: &ModInfo, prerelease: bool) -> Pick {
    if m.allow_mod_distribution == Some(false) {
        return Pick::OptOut;
    }
    for kind in release_order(prerelease) {
        let best = m
            .latest_files
            .iter()
            .filter(|f| f.release_type == kind && is_forever(f))
            .max_by(|a, b| a.file_date.cmp(&b.file_date).then(a.id.cmp(&b.id)));
        if let Some(f) = best {
            return match remote_from(f) {
                Some(r) => Pick::Ready(r),
                None => Pick::OptOut,
            };
        }
        if let Some(ix) = m
            .latest_files_indexes
            .iter()
            .filter(|i| i.release_type == kind && i.game_version_type_id == Some(FOREVER_TYPE))
            .max_by_key(|i| i.file_id)
        {
            return Pick::NeedsFile(ix.file_id);
        }
    }
    Pick::NoForeverFile
}

pub async fn file(client: &reqwest::Client, mod_id: u64, file_id: u64) -> anyhow::Result<FileInfo> {
    let resp = request(client, reqwest::Method::GET, format!("{BASE}/mods/{mod_id}/files/{file_id}"))?
        .send()
        .await?;
    if let Some(e) = api_error(resp.status().as_u16()) {
        return Err(e);
    }
    let f: Data<FileInfo> = resp.error_for_status()?.json().await?;
    Ok(f.data)
}

pub async fn resolve(client: &reqwest::Client, id: u64, prerelease: bool) -> anyhow::Result<Remote> {
    let m = fetch(client, id).await?;
    match pick_file(&m, prerelease) {
        Pick::Ready(r) => Ok(r),
        Pick::NeedsFile(fid) => {
            let f = file(client, id, fid).await?;
            remote_from(&f).ok_or_else(|| anyhow::anyhow!("{OPT_OUT}: the author only distributes this addon through CurseForge's own app"))
        }
        Pick::OptOut => anyhow::bail!("{OPT_OUT}: the author only distributes this addon through CurseForge's own app"),
        Pick::NoForeverFile => anyhow::bail!("CurseForge has no Forever release for this addon"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, name: &str, kind: u8, types: &[u64], url: Option<&str>, date: &str) -> FileInfo {
        FileInfo {
            id,
            display_name: name.into(),
            file_name: format!("{name}.zip"),
            release_type: kind,
            file_date: date.into(),
            download_url: url.map(String::from),
            sortable_game_versions: types.iter().map(|t| SortableVersion { game_version_type_id: Some(*t) }).collect(),
        }
    }

    #[test]
    fn picks_the_newest_stable_forever_file_with_a_link() {
        let m = ModInfo {
            allow_mod_distribution: Some(true),
            latest_files: vec![
                file(1, "v11", 1, &[517], Some("https://edge.forgecdn.net/a.zip"), "2026-01-01"),
                file(2, "v12.3.0", 1, &[517, FOREVER_TYPE], Some("https://edge.forgecdn.net/b.zip"), "2026-10-01"),
                file(3, "v12.3.1", 1, &[FOREVER_TYPE], Some("https://edge.forgecdn.net/c.zip"), "2026-10-20"),
                file(4, "v12.4-beta", 2, &[FOREVER_TYPE], Some("https://edge.forgecdn.net/d.zip"), "2026-10-25"),
            ],
            ..Default::default()
        };
        match pick_file(&m, false) {
            Pick::Ready(r) => {
                assert_eq!(r.version, "v12.3.1");
                assert_eq!(r.forever, Some(true));
                assert!(!r.prerelease);
            }
            other => panic!("{other:?}"),
        }
        match pick_file(&m, true) {
            Pick::Ready(r) => assert_eq!(r.version, "v12.4-beta"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn opted_out_authors_are_never_downloaded() {
        let m = ModInfo {
            allow_mod_distribution: Some(false),
            latest_files: vec![file(9, "x", 1, &[FOREVER_TYPE], Some("https://edge.forgecdn.net/x.zip"), "2026-10-01")],
            ..Default::default()
        };
        assert_eq!(pick_file(&m, false), Pick::OptOut);
        // Allowed on paper but no link on the file: treat the same way.
        let m2 = ModInfo {
            allow_mod_distribution: Some(true),
            latest_files: vec![file(9, "x", 1, &[FOREVER_TYPE], None, "2026-10-01")],
            ..Default::default()
        };
        assert_eq!(pick_file(&m2, false), Pick::OptOut);
    }

    #[test]
    fn falls_back_to_the_index_when_latest_files_lacks_forever() {
        let m = ModInfo {
            allow_mod_distribution: Some(true),
            latest_files: vec![file(1, "retail", 1, &[517], Some("https://edge.forgecdn.net/a.zip"), "2026-10-01")],
            latest_files_indexes: vec![
                FileIndex { file_id: 70, release_type: 3, game_version_type_id: Some(FOREVER_TYPE) },
                FileIndex { file_id: 71, release_type: 1, game_version_type_id: Some(FOREVER_TYPE) },
                FileIndex { file_id: 72, release_type: 1, game_version_type_id: Some(517) },
            ],
            ..Default::default()
        };
        assert_eq!(pick_file(&m, false), Pick::NeedsFile(71));
        let none = ModInfo { allow_mod_distribution: Some(true), ..Default::default() };
        assert_eq!(pick_file(&none, false), Pick::NoForeverFile);
    }

    #[test]
    fn key_is_absent_in_test_builds_or_round_trips() {
        // Either no key (CI, contributors) or a masked key that unmasks to text.
        match key() {
            None => assert!(!available()),
            Some(k) => assert!(available() && !k.is_empty() && !k.contains('\n')),
        }
        assert_eq!(urlencoding("Leatrix Plus & Maps"), "Leatrix+Plus+%26+Maps");
    }
}
