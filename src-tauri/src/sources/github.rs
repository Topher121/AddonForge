//! GitHub releases.
//!
//! Preferred path needs no API and has no rate limit: the BigWigs packager
//! uploads a `release.json` to every release describing each zip and the
//! flavours it supports. We fetch it through the stable
//! `releases/latest/download/<file>` redirect. If a repo has releases but no
//! release.json we fall back to the REST API (60 requests/hour unauthenticated).

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

pub async fn resolve(
    client: &reqwest::Client,
    repo: &str,
    asset_hint: Option<&str>,
) -> anyhow::Result<Remote> {
    let repo = repo.trim().trim_matches('/');
    let base = format!("https://github.com/{repo}/releases/latest/download/");

    let resp = client.get(format!("{base}release.json")).send().await?;
    if resp.status().is_success() {
        let rj: ReleaseJson = resp.json().await?;
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
            // Some packager configs leave version blank; the filename still carries it.
            pick.filename.trim_end_matches(".zip").to_string()
        } else {
            pick.version.clone()
        };
        return Ok(Remote {
            version,
            download_url: format!("{base}{}", pick.filename),
            filename: pick.filename.clone(),
            forever: Some(is_forever_build),
        });
    }

    // No release.json: ask the API for the latest release's assets.
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client
        .get(&api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if resp.status().as_u16() == 404 {
        anyhow::bail!("no GitHub release found for {repo}");
    }
    if resp.status().as_u16() == 403 {
        anyhow::bail!("GitHub API rate limit hit; try again in an hour");
    }
    let rel: ApiRelease = resp.error_for_status()?.json().await?;
    let zips: Vec<&ApiAsset> = rel
        .assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
        .collect();
    if zips.is_empty() {
        anyhow::bail!("latest release of {repo} has no zip asset");
    }
    let lower = |s: &str| s.to_ascii_lowercase();
    let pick = asset_hint
        .and_then(|h| zips.iter().find(|a| lower(&a.name).contains(&lower(h))))
        .or_else(|| {
            zips.iter()
                .find(|a| lower(&a.name).contains("forever") || lower(&a.name).contains("camelot"))
        })
        .or_else(|| {
            // Skip obvious other-flavour zips if there is a choice.
            zips.iter().find(|a| {
                let n = lower(&a.name);
                !["classic", "vanilla", "bcc", "tbc", "wrath", "wotlk", "cata", "mists", "mop", "nolib"]
                    .iter()
                    .any(|k| n.contains(k))
            })
        })
        .unwrap_or(&zips[0]);
    Ok(Remote {
        version: rel.tag_name.clone(),
        download_url: pick.browser_download_url.clone(),
        filename: pick.name.clone(),
        forever: None,
    })
}
