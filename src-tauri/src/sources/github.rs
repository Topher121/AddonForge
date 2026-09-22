//! GitHub releases.
//!
//! Preferred path needs no API and has no rate limit: the BigWigs packager
//! uploads a `release.json` to every release describing each zip and the
//! flavours it supports. We fetch it through the stable
//! `releases/latest/download/<file>` redirect. If a repo has releases but no
//! release.json we fall back to the REST API (60 requests/hour
//! unauthenticated). With pre-releases allowed we must use the API to list
//! releases (the `latest` redirect never points at a pre-release).

use super::Remote;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct ReleaseJson {
    #[serde(default)]
    releases: Vec<ReleaseEntry>,
}

#[derive(Deserialize, Debug)]
struct ReleaseEntry {
    #[serde(default)]
    version: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    nolib: bool,
    #[serde(default)]
    metadata: Vec<Meta>,
}

#[derive(Deserialize, Debug)]
struct Meta {
    #[serde(default)]
    flavor: String,
    #[serde(default)]
    interface: u32,
}

#[derive(Deserialize, Debug)]
struct ApiRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize, Debug)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

fn is_forever(m: &Meta) -> bool {
    m.flavor.eq_ignore_ascii_case("forever")
        || m.flavor.eq_ignore_ascii_case("camelot")
        || crate::wow::FOREVER_INTERFACE_RANGE.contains(&m.interface)
}

/// Pick from a release.json. `url_for` maps a filename to its download URL.
fn pick_from_release_json(
    rj: ReleaseJson,
    url_for: impl Fn(&str) -> String,
    prerelease: bool,
) -> anyhow::Result<Remote> {
    let candidates: Vec<&ReleaseEntry> = rj.releases.iter().filter(|r| !r.nolib).collect();
    let forever = candidates.iter().find(|r| r.metadata.iter().any(is_forever));
    let (pick, is_forever_build): (&ReleaseEntry, bool) = match forever {
        Some(r) => (r, true),
        None => match candidates.first().copied().or(rj.releases.first()) {
            Some(r) => (r, false),
            None => anyhow::bail!("release.json lists no files"),
        },
    };
    let version = if pick.version.trim().is_empty() {
        pick.filename.trim_end_matches(".zip").to_string()
    } else {
        pick.version.clone()
    };
    Ok(Remote {
        version,
        download_url: url_for(&pick.filename),
        filename: pick.filename.clone(),
        forever: Some(is_forever_build),
        prerelease,
    })
}

fn pick_asset<'a>(zips: &'a [&'a ApiAsset], asset_hint: Option<&str>) -> &'a ApiAsset {
    let lower = |s: &str| s.to_ascii_lowercase();
    asset_hint
        .and_then(|h| zips.iter().find(|a| lower(&a.name).contains(&lower(h))))
        .or_else(|| {
            zips.iter()
                .find(|a| lower(&a.name).contains("forever") || lower(&a.name).contains("camelot"))
        })
        .or_else(|| {
            zips.iter().find(|a| {
                let n = lower(&a.name);
                !["classic", "vanilla", "bcc", "tbc", "wrath", "wotlk", "cata", "mists", "mop", "nolib"]
                    .iter()
                    .any(|k| n.contains(k))
            })
        })
        .unwrap_or(&zips[0])
}

async fn from_api_release(
    client: &reqwest::Client,
    rel: ApiRelease,
    asset_hint: Option<&str>,
) -> anyhow::Result<Remote> {
    // Prefer the packager's release.json if this release has one.
    if let Some(rj_asset) = rel.assets.iter().find(|a| a.name == "release.json") {
        if let Ok(resp) = client.get(&rj_asset.browser_download_url).send().await {
            if resp.status().is_success() {
                if let Ok(rj) = resp.json::<ReleaseJson>().await {
                    let assets = &rel.assets;
                    let url_for = |name: &str| {
                        assets
                            .iter()
                            .find(|a| a.name == name)
                            .map(|a| a.browser_download_url.clone())
                            .unwrap_or_default()
                    };
                    let r = pick_from_release_json(rj, url_for, rel.prerelease)?;
                    if !r.download_url.is_empty() {
                        return Ok(r);
                    }
                }
            }
        }
    }
    let zips: Vec<&ApiAsset> = rel
        .assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
        .collect();
    if zips.is_empty() {
        anyhow::bail!("release {} has no zip asset", rel.tag_name);
    }
    let pick = pick_asset(&zips, asset_hint);
    Ok(Remote {
        version: rel.tag_name.clone(),
        download_url: pick.browser_download_url.clone(),
        filename: pick.name.clone(),
        forever: None,
        prerelease: rel.prerelease,
    })
}

fn api_error(status: u16, repo: &str) -> Option<anyhow::Error> {
    match status {
        404 => Some(anyhow::anyhow!("no GitHub release found for {repo}")),
        403 | 429 => Some(anyhow::anyhow!("GitHub API rate limit hit; try again in an hour")),
        _ => None,
    }
}

pub async fn resolve(
    client: &reqwest::Client,
    repo: &str,
    asset_hint: Option<&str>,
    prerelease: bool,
) -> anyhow::Result<Remote> {
    let repo = repo.trim().trim_matches('/');

    if prerelease {
        // Newest release including pre-releases (one API call).
        let api = format!("https://api.github.com/repos/{repo}/releases?per_page=5");
        let resp = client
            .get(&api)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if let Some(e) = api_error(resp.status().as_u16(), repo) {
            return Err(e);
        }
        let list: Vec<ApiRelease> = resp.error_for_status()?.json().await?;
        let rel = list
            .into_iter()
            .find(|r| !r.draft && !r.assets.is_empty())
            .ok_or_else(|| anyhow::anyhow!("no GitHub release with files for {repo}"))?;
        return from_api_release(client, rel, asset_hint).await;
    }

    let base = format!("https://github.com/{repo}/releases/latest/download/");
    let resp = client.get(format!("{base}release.json")).send().await?;
    if resp.status().is_success() {
        let rj: ReleaseJson = resp.json().await?;
        return pick_from_release_json(rj, |name| format!("{base}{name}"), false);
    }

    // No release.json: ask the API for the latest release's assets.
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if let Some(e) = api_error(resp.status().as_u16(), repo) {
        return Err(e);
    }
    let rel: ApiRelease = resp.error_for_status()?.json().await?;
    from_api_release(client, rel, asset_hint).await
}
