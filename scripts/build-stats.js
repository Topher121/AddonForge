#!/usr/bin/env node
// Builds catalog/stats.json: download totals + last release date per catalogue entry.
// Runs daily in GitHub Actions with GITHUB_TOKEN (5000 req/h), so the app itself
// never needs the GitHub API. GitHub numbers = lifetime release-asset downloads
// across all releases (all flavours, not Forever-only). WoWInterface numbers come
// from its public API. CurseForge is not available to us.
//
//   GITHUB_TOKEN=... node scripts/build-stats.js

const fs = require("fs");
const path = require("path");

const catalog = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "catalog", "forever.json"), "utf8"));
const out = path.join(__dirname, "..", "catalog", "stats.json");
const token = process.env.GITHUB_TOKEN;
const headers = { "User-Agent": "AddonForge-stats", Accept: "application/vnd.github+json" };
if (token) headers.Authorization = `Bearer ${token}`;

async function github(repo) {
  let downloads = 0;
  let latest = null;
  for (let page = 1; page <= 10; page++) {
    const res = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=100&page=${page}`, { headers });
    if (!res.ok) throw new Error(`${repo}: HTTP ${res.status}`);
    const releases = await res.json();
    if (!releases.length) break;
    for (const r of releases) {
      if (r.draft) continue;
      for (const a of r.assets || []) downloads += a.download_count || 0;
      if (!r.prerelease && (!latest || r.published_at > latest)) latest = r.published_at;
    }
    if (releases.length < 100) break;
  }
  return { downloads, updated_at: latest, source: "github" };
}

async function wowi(id) {
  const res = await fetch(`https://api.mmoui.com/v3/game/WOW/filedetails/${id}.json`, { headers: { "User-Agent": "AddonForge-stats" } });
  if (!res.ok) throw new Error(`wowi ${id}: HTTP ${res.status}`);
  const [d] = await res.json();
  return { downloads: Number(d.UIDownloadTotal) || 0, updated_at: d.UIDate ? new Date(Number(d.UIDate)).toISOString() : null, source: "wowi" };
}

(async () => {
  const previous = fs.existsSync(out) ? JSON.parse(fs.readFileSync(out, "utf8")).entries || {} : {};
  const entries = {};
  let failures = 0;
  for (const a of catalog.addons) {
    try {
      if (a.github) entries[a.id] = await github(a.github);
      else if (a.wowi) entries[a.id] = await wowi(a.wowi);
    } catch (e) {
      failures++;
      console.error(`  ! ${a.id}: ${e.message}`);
      if (previous[a.id]) entries[a.id] = previous[a.id]; // keep last known
    }
  }
  const stats = { updated: new Date().toISOString().slice(0, 10), entries };
  fs.writeFileSync(out, JSON.stringify(stats, null, 1) + "\n");
  console.log(`stats.json: ${Object.keys(entries).length} entries, ${failures} failures`);
  if (failures > catalog.addons.length / 2) process.exit(1);
})();
