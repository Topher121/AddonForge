#!/usr/bin/env node
// Builds the AddonForge website (GitHub Pages) from the catalogue.
//
//   node scripts/build-site.js [outDir]        default: _site
//
// Everything comes from catalog/forever.json, catalog/stats.json and the
// latest GitHub release. The site is a pointer list like the catalogue: it
// links to wherever each author publishes and never hosts an addon file.
// No trackers, no third-party scripts or fonts; one same-origin script for
// the directory search.
"use strict";
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const OUT = path.resolve(process.argv[2] || path.join(ROOT, "_site"));
const REPO = "Topher121/AddonForge";
const REPO_URL = `https://github.com/${REPO}`;
const DISCORD = "https://discord.gg/2qyV5kxT9h";
const KOFI = "https://ko-fi.com/pocketforgestudios";
const EMAIL = "PocketForgeStudios@proton.me";
const SITE_URL = "https://topher121.github.io/AddonForge/";

const cat = JSON.parse(fs.readFileSync(path.join(ROOT, "catalog", "forever.json"), "utf8"));
const stats = (() => {
  try { return JSON.parse(fs.readFileSync(path.join(ROOT, "catalog", "stats.json"), "utf8")).entries || {}; }
  catch { return {}; }
})();

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const slugOk = (id) => /^[a-z0-9][a-z0-9-]*$/.test(id);
const https = (u) => (typeof u === "string" && /^https:\/\//i.test(u) ? u : null);

function compact(n) {
  if (!n) return "";
  if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e7 ? 0 : 1).replace(/\.0$/, "") + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(n >= 1e4 ? 0 : 1).replace(/\.0$/, "") + "k";
  return String(n);
}
function day(iso) {
  const d = new Date(iso);
  return isNaN(d) ? "" : d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
}
function mb(bytes) { return bytes ? (bytes / 1048576).toFixed(1) + " MB" : ""; }

/** Where an addon can be fetched, in the order AddonForge prefers them. */
function sources(e) {
  const out = [];
  if (e.github) out.push({ label: "GitHub", url: `https://github.com/${e.github}` });
  if (e.curse) out.push({ label: "CurseForge", url: e.url && /curseforge\.com/.test(e.url) ? e.url : `https://www.curseforge.com/projects/${e.curse}` });
  if (e.wowi) out.push({ label: "WoWInterface", url: `https://www.wowinterface.com/downloads/info${e.wowi}` });
  if (e.tukui) out.push({ label: "TukUI", url: `https://www.tukui.org/download.php?ui=${encodeURIComponent(e.tukui)}` });
  if (e.wago) out.push({ label: "Wago", url: `https://addons.wago.io/addons/${encodeURIComponent(e.wago)}` });
  return out;
}
/** Can AddonForge install it, or does it only link out? */
function installable(e) {
  return !!(e.github || e.wowi || e.tukui || (e.curse && !e.curse_locked));
}

async function latestRelease() {
  const headers = { "User-Agent": "AddonForge-site", Accept: "application/vnd.github+json" };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers });
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const j = await r.json();
    const asset = (re) => (j.assets || []).find((a) => re.test(a.name));
    return {
      tag: j.tag_name,
      date: j.published_at,
      url: j.html_url,
      setup: asset(/^AddonForge-Setup-.*\.exe$/i),
      portable: asset(/^AddonForge-v.*\.exe$/i),
    };
  } catch (err) {
    console.warn(`latest release unavailable (${err.message}); linking the releases page`);
    return null;
  }
}

// ---- page shell -----------------------------------------------------------

const CSP = "default-src 'none'; style-src 'self'; script-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'";

function page({ title, desc, body, depth = 0, active = "", script = false }) {
  const up = "../".repeat(depth);
  const nav = [
    ["", "AddonForge"],
    ["addons.html", "Addons"],
    ["packs.html", "Starter packs"],
    ["help.html", "Help"],
  ];
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="${CSP}">
<meta name="referrer" content="no-referrer">
<title>${esc(title)}</title>
<meta name="description" content="${esc(desc)}">
<link rel="icon" href="${up}icon.svg" type="image/svg+xml">
<link rel="stylesheet" href="${up}style.css">
</head>
<body>
<header class="top">
  <a class="brand" href="${up}index.html"><img class="mark" src="${up}icon.svg" alt="" width="26" height="26"><span class="name">AddonForge</span></a>
  <nav>
${nav.slice(1).map(([href, label]) => `    <a href="${up}${href}"${active === href ? ' aria-current="page"' : ""}>${label}</a>`).join("\n")}
    <a href="${REPO_URL}">GitHub</a>
  </nav>
</header>
<main>
${body}
</main>
<footer>
  <p>AddonForge is open source (MIT) and made by Pocket Forge Studios. It is not affiliated with Blizzard Entertainment, CurseForge, Overwolf, Wago, WoWInterface or TukUI. World of Warcraft is a trademark of Blizzard Entertainment.</p>
  <p><a href="${REPO_URL}">Source</a> · <a href="${DISCORD}">Discord</a> · <a href="mailto:${EMAIL}">${EMAIL}</a> · <a href="${KOFI}">Support the dev</a></p>
</footer>
${script ? `<script src="${up}site.js" defer></script>\n` : ""}</body>
</html>
`;
}

// ---- pages ----------------------------------------------------------------

function downloadBlock(rel) {
  if (!rel || !rel.setup) {
    return `<p><a class="btn primary" href="${REPO_URL}/releases/latest">Download for Windows</a></p>`;
  }
  const port = rel.portable
    ? ` or the <a href="${esc(rel.portable.browser_download_url)}">portable exe</a> (${mb(rel.portable.size)}, no install)`
    : "";
  return `<p><a class="btn primary" href="${esc(rel.setup.browser_download_url)}">Download for Windows</a></p>
<p class="meta">${esc(rel.tag)}, released ${esc(day(rel.date))}. Installer ${mb(rel.setup.size)}${port}. Windows 10 and 11. <a href="${esc(rel.url)}">Release notes</a></p>`;
}

function home(rel) {
  const n = cat.addons.length;
  const body = `
<section class="intro">
  <h1>An addon manager for WoW: Forever</h1>
  <p>AddonForge finds your Forever install, reads your AddOns folder, and keeps your addons up to date. It downloads straight from where each author publishes: GitHub, CurseForge, WoWInterface and TukUI. No account, no ads, no tracking.</p>
  ${downloadBlock(rel)}
</section>

<section>
  <h2>Coming from CurseForge</h2>
  <p>Point AddonForge at the same game folder. Addons the CurseForge app installed are recognised as they are, by the same folder fingerprints the CurseForge app uses, so there is nothing to reinstall. Updates then come from the author's own releases where they publish them, and from CurseForge where they allow other apps to download.</p>
  <p>Some authors only allow downloads through CurseForge's own app. AddonForge still tells you when those have a new version and links you to the page, rather than pretending.</p>
</section>

<section>
  <h2>What it does</h2>
  <ul class="plain">
    <li>Checks every addon for updates and updates them all at once, or one at a time.</li>
    <li>Searches ${n} addons in its own catalogue and every Forever addon on CurseForge.</li>
    <li>Installs from any GitHub repo, or from a zip you downloaded yourself.</li>
    <li>Starter packs for people new to addons, and a backup of your <code>WTF</code> settings folder.</li>
    <li>Refuses any addon download that contains a program file.</li>
    <li>Small: a 7 MB program using about 100 MB of memory.</li>
  </ul>
</section>

<section>
  <h2>Privacy</h2>
  <p>There is no server of ours. AddonForge talks only to the sites addons are published on, plus GitHub for its own updates and catalogue. It sends no analytics and has no account. The full list of what it contacts and why is in the <a href="${REPO_URL}#code-signing-policy">README</a>.</p>
</section>

<section>
  <h2>Help and feedback</h2>
  <p>Questions go to the <a href="${DISCORD}">Discord</a>. Bugs go to <a href="${REPO_URL}/issues">GitHub issues</a>; the app's Report a problem button fills one in for you. Common questions are on the <a href="help.html">help page</a>.</p>
</section>`;
  return page({
    title: "AddonForge: addon manager for WoW: Forever",
    desc: "Free, open-source addon manager for WoW: Forever. Updates your addons from GitHub, CurseForge, WoWInterface and TukUI. No account, no ads, no tracking.",
    body,
  });
}

function directory() {
  const cats = [...new Set(cat.addons.map((e) => e.category).filter(Boolean))].sort();
  const rows = cat.addons
    .filter((e) => slugOk(e.id))
    .map((e) => ({ e, s: stats[e.id] || {} }))
    .sort((a, b) => (b.s.downloads || 0) - (a.s.downloads || 0) || a.e.name.localeCompare(b.e.name))
    .map(({ e, s }) => {
      const src = sources(e)[0];
      const how = installable(e) ? "" : ' <span class="tag">link only</span>';
      return `<tr data-cat="${esc(e.category || "")}" data-text="${esc((e.name + " " + (e.desc || "") + " " + (e.category || "")).toLowerCase())}">
  <td><a href="addons/${e.id}.html">${esc(e.name)}</a>${how}<div class="desc">${esc(e.desc || "")}</div></td>
  <td class="c-cat">${esc(e.category || "")}</td>
  <td class="c-src">${src ? esc(src.label) : ""}</td>
  <td class="c-num">${compact(s.downloads)}</td>
</tr>`;
    }).join("\n");
  const body = `
<h1>Addons</h1>
<p>The ${cat.addons.length} addons in AddonForge's catalogue. In the app, every Forever addon on CurseForge is searchable too. Anyone can <a href="${REPO_URL}/blob/main/CONTRIBUTING.md">add an addon by pull request</a>.</p>
<div class="toolbar">
  <input id="q" type="search" placeholder="Search addons" aria-label="Search addons">
  <select id="cat" aria-label="Type"><option value="">All types</option>${cats.map((c) => `<option>${esc(c)}</option>`).join("")}</select>
  <span id="count" class="meta" aria-live="polite"></span>
</div>
<table class="list">
<thead><tr><th>Addon</th><th class="c-cat">Type</th><th class="c-src">Source</th><th class="c-num" title="Downloads counted on GitHub and WoWInterface">Downloads</th></tr></thead>
<tbody id="rows">
${rows}
</tbody>
</table>
<p class="meta">Download counts are from GitHub and WoWInterface only, updated daily.</p>`;
  return page({ title: "Addons for WoW: Forever · AddonForge", desc: `${cat.addons.length} WoW: Forever addons and where their authors publish them.`, body, active: "addons.html", script: true });
}

function addonPage(e) {
  const s = stats[e.id] || {};
  const src = sources(e);
  const packs = (cat.bundles || []).filter((b) => (b.addons || []).includes(e.id));
  const facts = [
    ["Type", esc(e.category || "")],
    ["Downloads", s.downloads ? `${s.downloads.toLocaleString("en-GB")} <span class="meta">(${esc(s.source || "")})</span>` : ""],
    ["Last release", s.updated_at ? esc(day(s.updated_at)) : ""],
    ["Folders", (e.folders || []).map((f) => `<code>${esc(f)}</code>`).join(" ")],
    ["Forever", e.forever === false ? "Not marked as working on Forever yet" : "Works on Forever"],
    ["In packs", packs.map((b) => `<a href="../packs.html#${esc(b.id)}">${esc(b.name)}</a>`).join(", ")],
  ].filter(([, v]) => v);
  const how = installable(e)
    ? `<p>In AddonForge, open <b>Find addons</b>, search for <b>${esc(e.name)}</b> and choose Install. AddonForge downloads it from ${esc(src[0]?.label || "its source")}.</p>`
    : `<p>The author only allows downloads through CurseForge's own app, so AddonForge links to it instead of installing it. It still tells you when a new version is out.</p>`;
  const body = `
<p class="crumb"><a href="../addons.html">Addons</a></p>
<h1>${esc(e.name)}</h1>
<p class="lede">${esc(e.desc || "")}</p>
${e.notice ? `<p class="notice">${esc(e.notice)}</p>` : ""}
<table class="facts">${facts.map(([k, v]) => `<tr><th>${k}</th><td>${v}</td></tr>`).join("")}</table>
<h2>Get it</h2>
${how}
${src.length ? `<p class="links">${src.map((x) => `<a class="btn" href="${esc(x.url)}">${esc(x.label)}</a>`).join(" ")}</p>` : ""}
<p class="meta">AddonForge only links here. The files come from the author's own page.</p>`;
  return page({ title: `${e.name} · WoW: Forever addon · AddonForge`, desc: `${e.name}: ${e.desc || ""}`, body, depth: 1, active: "addons.html" });
}

function packsPage() {
  const byId = new Map(cat.addons.map((e) => [e.id, e]));
  const blocks = (cat.bundles || []).map((b) => `
<section id="${esc(b.id)}">
  <h2>${esc(b.name)}</h2>
  <p>${esc(b.desc || "")}</p>
  <ul class="plain">
${(b.addons || []).map((id) => byId.get(id)).filter(Boolean).map((e) => `    <li><a href="addons/${e.id}.html">${esc(e.name)}</a> <span class="meta">${esc(e.desc || "")}</span></li>`).join("\n")}
  </ul>
  ${b.note ? `<p class="meta">${esc(b.note)}</p>` : ""}
</section>`).join("\n");
  const body = `
<h1>Starter packs</h1>
<p>New to addons? These are groups that work well together. In AddonForge, open <b>Starter packs</b> and choose Install all, or pick them one by one.</p>
${blocks}`;
  return page({ title: "Starter packs · AddonForge", desc: "Groups of WoW: Forever addons that work well together.", body, active: "packs.html" });
}

function helpPage() {
  const qa = [
    ["Which game does it support?", "WoW: Forever, Blizzard's Classic+ client. It finds the <code>_forever_</code> folder by itself; you can also choose the folder in Settings."],
    ["Will it touch my settings?", "No. AddonForge only changes folders inside <code>Interface\\AddOns</code> and never writes to <code>WTF</code>, where your addon settings live. Settings has a button to back <code>WTF</code> up to a zip."],
    ["I used the CurseForge app before. Do I reinstall everything?", "No. Point AddonForge at the same game folder and run Check for updates. It recognises CurseForge installs by their fingerprints."],
    ["Why does an addon say “link only”?", "Its author only allows downloads through CurseForge's own app. AddonForge respects that and links to the page."],
    ["How does AddonForge update itself?", "It checks GitHub for a newer release when it starts and asks before installing it. Updates are signed, and an update that fails the signature check is refused."],
    ["Is it signed?", "The update files are signed with the project's key. The program itself isn't code-signed by a certificate authority yet, so Windows may treat it as an unknown app. The source is public, and every release is built from it."],
    ["How do I uninstall it?", "Windows Settings, Apps, AddonForge, Uninstall. Your addons stay where they are. The portable exe can simply be deleted."],
    ["Does it work on a Mac?", "There are no Mac downloads, but it builds from source on macOS. See the README."],
    ["Something's wrong", `Use Report a problem in the app, or open an <a href="${REPO_URL}/issues">issue</a>. The <a href="${DISCORD}">Discord</a> is quickest for questions.`],
  ];
  const body = `
<h1>Help</h1>
${qa.map(([q, a]) => `<section class="qa"><h2>${q}</h2><p>${a}</p></section>`).join("\n")}`;
  return page({ title: "Help · AddonForge", desc: "Common questions about AddonForge.", body, active: "help.html" });
}

function sitemap() {
  const urls = ["", "addons.html", "packs.html", "help.html", ...cat.addons.filter((e) => slugOk(e.id)).map((e) => `addons/${e.id}.html`)];
  return `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls.map((u) => `  <url><loc>${SITE_URL}${u}</loc></url>`).join("\n")}
</urlset>
`;
}

// ---- write ----------------------------------------------------------------

(async () => {
  const rel = await latestRelease();
  fs.rmSync(OUT, { recursive: true, force: true });
  fs.mkdirSync(path.join(OUT, "addons"), { recursive: true });
  const write = (name, text) => fs.writeFileSync(path.join(OUT, name), text);
  write("index.html", home(rel));
  write("addons.html", directory());
  write("packs.html", packsPage());
  write("help.html", helpPage());
  write("404.html", page({ title: "Not found · AddonForge", desc: "Page not found.", body: `<h1>Not found</h1><p>That page doesn't exist. Try the <a href="${SITE_URL}addons.html">addon list</a>.</p>` }).replace(/href="(?!https?:|mailto:|#)([^"]*)"/g, `href="${SITE_URL}$1"`).replace(/src="(?!https?:)([^"]*)"/g, `src="${SITE_URL}$1"`));
  for (const e of cat.addons) {
    if (slugOk(e.id)) write(path.join("addons", `${e.id}.html`), addonPage(e));
  }
  write("sitemap.xml", sitemap());
  write("robots.txt", `User-agent: *\nAllow: /\nSitemap: ${SITE_URL}sitemap.xml\n`);
  write(".nojekyll", "");
  for (const f of ["style.css", "site.js"]) fs.copyFileSync(path.join(ROOT, "site", f), path.join(OUT, f));
  fs.copyFileSync(path.join(ROOT, "ui", "icon.svg"), path.join(OUT, "icon.svg"));
  console.log(`site: ${cat.addons.length} addon pages, release ${rel ? rel.tag : "(none)"} -> ${OUT}`);
})().catch((err) => { console.error(err); process.exit(1); });
