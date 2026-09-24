//! Addon icons, three sources in order:
//!  1. the addon's own `## IconTexture` file on disk (TGA / BLP / PNG),
//!  2. the GitHub author's avatar for GitHub-sourced addons (or a catalogue
//!     entry's explicit `icon` URL, https only),
//!  3. nothing: the UI draws a lettered tile.
//!
//! Icons the game keeps inside its own data files (`Interface\Icons\...`)
//! are not readable from disk and are deliberately NOT fetched from any
//! third-party site: the app only talks to the addon hosts.
//!
//! Everything is cached in `%APPDATA%\AddonForge\icons\` as 32px PNGs and
//! handed to the UI as data URLs.

use base64::Engine;
use image::imageops::FilterType;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const SIZE: u32 = 32;
const AVATAR_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

fn cache_dir() -> PathBuf {
    crate::state::AppState::dir().join("icons")
}

fn key(s: &str) -> String {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn to_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

fn encode_png(img: image::DynamicImage) -> anyhow::Result<Vec<u8>> {
    let small = img.resize_to_fill(SIZE, SIZE, FilterType::Lanczos3).to_rgba8();
    let mut out = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

fn decode_file(path: &Path) -> anyhow::Result<image::DynamicImage> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "blp" {
        let blp = image_blp::parser::load_blp(path).map_err(|e| anyhow::anyhow!("blp: {e:?}"))?;
        let old = image_blp::convert::blp_to_image(&blp, 0).map_err(|e| anyhow::anyhow!("blp convert: {e:?}"))?;
        // image-blp uses an older `image` crate; hand the pixels over by value.
        let rgba = old.to_rgba8();
        let (w, h) = rgba.dimensions();
        let ours = image::RgbaImage::from_raw(w, h, rgba.into_raw()).ok_or_else(|| anyhow::anyhow!("blp: bad buffer"))?;
        return Ok(image::DynamicImage::ImageRgba8(ours));
    }
    Ok(image::open(path)?)
}

/// Resolve `## IconTexture` to a file inside the install, if it points at one.
pub fn resolve_texture(install: &Path, texture: &str) -> Option<PathBuf> {
    let t = texture.trim().replace('/', "\\");
    let lower = t.to_ascii_lowercase();
    let rel = lower.strip_prefix("interface\\addons\\")?;
    let rel = &t[t.len() - rel.len()..]; // original case, same offset
    let base = install.join("Interface").join("AddOns").join(rel.replace('\\', "/"));
    if base.is_file() {
        return Some(base);
    }
    // No extension in the TOC: the game tries these.
    for ext in ["tga", "blp", "png"] {
        let p = base.with_extension(ext);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Icon from the addon's own files. Cached by path + mtime.
pub fn local_icon(install: &Path, texture: &str) -> Option<String> {
    let path = resolve_texture(install, texture)?;
    let mtime = fs::metadata(&path).ok()?.modified().ok()?;
    let stamp = mtime.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let cache = cache_dir().join(format!("{}.png", key(&format!("{}|{}", path.display(), stamp))));
    if let Ok(png) = fs::read(&cache) {
        return Some(to_data_url(&png));
    }
    let img = decode_file(&path).ok()?;
    let png = encode_png(img).ok()?;
    let _ = fs::create_dir_all(cache_dir());
    let _ = fs::write(&cache, &png);
    Some(to_data_url(&png))
}

/// Fetch an https image (GitHub avatar or a catalogue `icon` URL), cached for 30 days.
pub async fn remote_icon(client: &reqwest::Client, url: &str) -> Option<String> {
    if !url.starts_with("https://") {
        return None;
    }
    let cache = cache_dir().join(format!("{}.png", key(url)));
    if let Ok(meta) = fs::metadata(&cache) {
        let fresh = meta.modified().ok().and_then(|m| m.elapsed().ok()).map(|age| age < AVATAR_TTL).unwrap_or(false);
        if fresh {
            if let Ok(png) = fs::read(&cache) {
                return Some(to_data_url(&png));
            }
        }
    }
    let bytes = client
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .bytes()
        .await
        .ok()?;
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(&bytes).ok()?;
    let png = encode_png(img).ok()?;
    let _ = fs::create_dir_all(cache_dir());
    let _ = fs::write(&cache, &png);
    Some(to_data_url(&png))
}

pub fn github_avatar_url(repo: &str) -> Option<String> {
    let owner = repo.split('/').next()?.trim();
    if owner.is_empty() {
        return None;
    }
    Some(format!("https://github.com/{owner}.png?size=64"))
}

// ---------------------------------------------------------------- addon details (README excerpt + first image)

const PREVIEW_W: u32 = 720;
const README_TTL: Duration = Duration::from_secs(24 * 3600);

/// Hosts an image may come from for the preview. The app promises to talk
/// only to the addon sites, so anything else in a README is ignored.
fn allowed_image_host(url: &str) -> bool {
    let rest = match url.strip_prefix("https://") {
        Some(r) => r,
        None => return false,
    };
    let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
    host == "github.com"
        || host.ends_with(".githubusercontent.com")
        || host == "media.forgecdn.net"
        || host == "addons.wago.io"
        || host.ends_with(".wago.io")
        || host == "www.wowinterface.com"
        || host == "cdn-wow.mmoui.com"
}

/// First image reference in a README: `![alt](url)` or `<img src="url">`.
/// Relative paths are resolved against the repo's raw HEAD.
fn first_readme_image(md: &str, repo: &str) -> Option<String> {
    let mut candidates: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while let Some(p) = md[i..].find("![") {
        let start = i + p;
        if let Some(close) = md[start..].find("](") {
            let after = start + close + 2;
            let end = md[after..].find(|c: char| c == ')' || c == ' ' || c == '\n').map(|e| after + e).unwrap_or(md.len());
            candidates.push((start, md[after..end].trim().to_string()));
        }
        i = start + 2;
    }
    let lower = md.to_ascii_lowercase();
    let mut i = 0;
    while let Some(p) = lower[i..].find("<img") {
        let start = i + p;
        if let Some(s) = lower[start..].find("src=") {
            let q = start + s + 4;
            let quote = md[q..].chars().next().unwrap_or('"');
            if quote == '"' || quote == '\'' {
                if let Some(e) = md[q + 1..].find(quote) {
                    candidates.push((start, md[q + 1..q + 1 + e].trim().to_string()));
                }
            }
        }
        i = start + 4;
    }
    candidates.sort_by_key(|c| c.0);
    for (_, mut url) in candidates {
        let l = url.to_ascii_lowercase();
        // badges are the usual first image; skip the obvious ones
        if l.contains("shields.io") || l.contains("badge") || l.ends_with(".svg") {
            continue;
        }
        if !l.starts_with("http") {
            let rel = url.trim_start_matches("./").trim_start_matches('/');
            url = format!("https://raw.githubusercontent.com/{repo}/HEAD/{rel}");
        } else if let Some(rest) = url.strip_prefix("https://github.com/") {
            // github.com/owner/repo/blob/branch/path -> raw
            if rest.contains("/blob/") {
                url = format!("https://raw.githubusercontent.com/{}", rest.replacen("/blob/", "/", 1));
            }
        }
        if allowed_image_host(&url) {
            return Some(url);
        }
    }
    None
}

/// Attribute value from an html tag body (`src="..."` or `src='...'`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find(name) {
        let at = from + p;
        let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric() && lower.as_bytes()[at - 1] != b'-';
        let rest = &tag[at + name.len()..];
        let rest_t = rest.trim_start();
        if before_ok && rest_t.starts_with('=') {
            let v = rest_t[1..].trim_start();
            let quote = v.chars().next()?;
            if quote == '"' || quote == '\'' {
                let end = v[1..].find(quote)?;
                return Some(v[1..1 + end].to_string());
            }
        }
        from = at + name.len();
    }
    None
}

/// First real picture in GitHub's rendered README (images come back
/// through camo.githubusercontent.com, so nothing off-GitHub is fetched).
/// Badges, logos and widgets are skipped.
fn first_html_image(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while let Some(p) = lower[i..].find("<img") {
        let start = i + p;
        let end = html[start..].find('>').map(|e| start + e).unwrap_or(html.len());
        let tag = &html[start..end];
        i = end.max(start + 4);
        let Some(src) = attr(tag, "src") else { continue };
        let canonical = attr(tag, "data-canonical-src").unwrap_or_default().to_ascii_lowercase();
        let alt = attr(tag, "alt").unwrap_or_default().to_ascii_lowercase();
        let probe = format!("{} {}", src.to_ascii_lowercase(), canonical);
        if probe.contains("shields.io") || probe.contains("badge") || probe.contains(".svg") || probe.contains("discordapp.com/api") || probe.contains("widget") || alt.contains("logo") || alt.contains("badge") || alt.contains("discord") {
            continue;
        }
        if allowed_image_host(&src) {
            return Some(src);
        }
    }
    None
}

/// Plain-text excerpt of a README: badges, headings and html dropped,
/// links reduced to their text, first ~700 chars of prose.
fn readme_excerpt(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for raw in md.lines() {
        let line = raw.trim();
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.is_empty() || line.starts_with('#') || line.starts_with("[![") || line.starts_with("![") || line.starts_with('<') || line.starts_with('|') || line.starts_with("---") {
            if line.is_empty() && !out.is_empty() && !out.ends_with("\n\n") {
                out.push_str("\n\n");
            }
            continue;
        }
        let mut s = line.to_string();
        // [text](url) -> text ; **x** -> x ; `x` -> x
        while let (Some(a), Some(b)) = (s.find("]("), s.find('[')) {
            if b < a {
                if let Some(c) = s[a..].find(')') {
                    let text = s[b + 1..a].to_string();
                    s.replace_range(b..a + c + 1, &text);
                    continue;
                }
            }
            break;
        }
        s = s.replace("**", "").replace('`', "");
        if s.starts_with("- ") || s.starts_with("* ") {
            s = format!("\u{2022} {}", &s[2..]);
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(&s);
        if out.len() > 700 {
            break;
        }
    }
    let mut t = out.trim().to_string();
    if t.len() > 700 {
        let cut = t.char_indices().take_while(|(i, _)| *i < 700).last().map(|(i, c)| i + c.len_utf8()).unwrap_or(t.len());
        t.truncate(cut);
        t.push('\u{2026}');
    }
    t
}

#[derive(serde::Serialize, Clone, Default)]
pub struct Details {
    pub summary: Option<String>,
    pub image: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedDetails {
    summary: Option<String>,
    image_url: Option<String>,
}

/// README summary and one preview image for a GitHub repo. Cached a day.
pub async fn github_details(client: &reqwest::Client, repo: &str) -> Details {
    let cache = cache_dir().join(format!("{}.json", key(&format!("readme2|{repo}"))));
    if let Ok(meta) = fs::metadata(&cache) {
        let fresh = meta.modified().ok().and_then(|m| m.elapsed().ok()).map(|age| age < README_TTL).unwrap_or(false);
        if fresh {
            if let Some(d) = fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<CachedDetails>(&b).ok()) {
                return Details { summary: d.summary, image: d.image_url.as_deref().and_then(preview_cached) };
            }
        }
    }
    let mut md = None;
    for name in ["README.md", "readme.md", "Readme.md", "README.MD", "README"] {
        let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/{name}");
        if let Ok(r) = client.get(&url).timeout(Duration::from_secs(10)).send().await {
            if r.status().is_success() {
                if let Ok(t) = r.text().await {
                    md = Some(t);
                    break;
                }
            }
        }
    }
    let Some(md) = md else { return Details::default() };
    let summary = Some(readme_excerpt(&md)).filter(|s| !s.is_empty());
    // Picture: GitHub's rendered README first (its proxy serves the
    // author's off-site images from a GitHub host), raw markdown as fallback.
    let mut image_url = None;
    if let Ok(r) = client
        .get(format!("https://api.github.com/repos/{repo}/readme"))
        .header("Accept", "application/vnd.github.html")
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        if r.status().is_success() {
            if let Ok(html) = r.text().await {
                image_url = first_html_image(&html);
            }
        }
    }
    if image_url.is_none() {
        image_url = first_readme_image(&md, repo);
    }
    let image = match &image_url {
        Some(u) => preview_image(client, u).await,
        None => None,
    };
    let _ = fs::create_dir_all(cache_dir());
    let cached = CachedDetails { summary: summary.clone(), image_url: image_url.filter(|_| image.is_some()) };
    let _ = fs::write(&cache, serde_json::to_vec(&cached).unwrap_or_default());
    Details { summary, image }
}

fn preview_path(url: &str) -> PathBuf {
    cache_dir().join(format!("{}.jpg", key(&format!("preview|{url}"))))
}

fn preview_cached(url: &str) -> Option<String> {
    let bytes = fs::read(preview_path(url)).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Fetch and shrink a preview image (jpeg, <= 720px wide). 8 MB cap.
pub async fn preview_image(client: &reqwest::Client, url: &str) -> Option<String> {
    if !allowed_image_host(url) {
        return None;
    }
    if let Some(d) = preview_cached(url) {
        return Some(d);
    }
    let bytes = client.get(url).timeout(Duration::from_secs(20)).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
    if bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(&bytes).ok()?;
    let img = if img.width() > PREVIEW_W { img.resize(PREVIEW_W, PREVIEW_W * 4, FilterType::Triangle) } else { img };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82);
    enc.encode_image(&rgb).ok()?;
    let _ = fs::create_dir_all(cache_dir());
    let _ = fs::write(preview_path(url), &out);
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_image_skips_badges_and_resolves_relative() {
        let md = "# Foo\n[![build](https://img.shields.io/x.svg)](x)\n\nText\n\n![shot](docs/shot.png)\n";
        assert_eq!(first_readme_image(md, "o/r").as_deref(), Some("https://raw.githubusercontent.com/o/r/HEAD/docs/shot.png"));
        let md2 = "<img src=\"https://user-images.githubusercontent.com/1/a.png\" width=400>";
        assert_eq!(first_readme_image(md2, "o/r").as_deref(), Some("https://user-images.githubusercontent.com/1/a.png"));
        assert_eq!(first_readme_image("![x](https://evil.example/a.png)", "o/r"), None);
    }

    #[test]
    fn html_image_prefers_camo_and_skips_badges() {
        let html = r#"<p><a href="x"><img src="https://camo.githubusercontent.com/aaa" data-canonical-src="https://img.shields.io/badge/a.svg" alt="CurseForge"></a></p>
<img src="https://camo.githubusercontent.com/bbb" data-canonical-src="https://i.imgur.com/logo.png" alt="BetterBags Logo">
<td><a href="y"><img src="https://camo.githubusercontent.com/ccc" data-canonical-src="https://i.imgur.com/shot.png" alt="The bag view"></a></td>"#;
        assert_eq!(first_html_image(html).as_deref(), Some("https://camo.githubusercontent.com/ccc"));
        assert_eq!(first_html_image(r#"<img src="https://i.imgur.com/direct.png">"#), None);
    }

    #[test]
    fn excerpt_strips_markup() {
        let md = "# Title\n\n[![b](https://img.shields.io/a.svg)](l)\n\nThis is **bold** and a [link](https://x) here.\n- one\n\n```lua\ncode\n```\nMore.";
        let e = readme_excerpt(md);
        assert!(e.starts_with("This is bold and a link here."), "{e}");
        assert!(e.contains("\u{2022} one"));
        assert!(!e.contains("code"));
    }
}
