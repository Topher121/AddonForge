//! TukUI (ElvUI / Tukui) public API. No key needed.

use super::Remote;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct TukAddon {
    #[serde(default)]
    version: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    patch: Vec<String>,
}

pub async fn resolve(client: &reqwest::Client, slug: &str) -> anyhow::Result<Remote> {
    let url = format!("https://api.tukui.org/v1/addon/{slug}");
    let resp = client.get(&url).send().await?;
    if resp.status().as_u16() == 404 {
        anyhow::bail!("TukUI has no addon '{slug}'");
    }
    let a: TukAddon = resp.error_for_status()?.json().await?;
    if a.url.is_empty() {
        anyhow::bail!("TukUI entry has no download url");
    }
    let forever = Some(a.patch.iter().any(|p| p.starts_with("1.60")));
    Ok(Remote {
        version: a.version.clone(),
        download_url: a.url,
        filename: format!("{slug}-{}.zip", a.version),
        forever,
    })
}
