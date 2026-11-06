#!/usr/bin/env node
// Validates catalog/forever.json. Run locally (`node scripts/validate-catalog.js`)
// and on every pull request (.github/workflows/catalog.yml).
// Exit code 1 with a readable list of problems if anything is wrong.

const fs = require("fs");
const path = require("path");

const file = path.join(__dirname, "..", "catalog", "forever.json");
const problems = [];
let cat;
try {
  cat = JSON.parse(fs.readFileSync(file, "utf8"));
} catch (e) {
  console.error(`catalog/forever.json is not valid JSON: ${e.message}`);
  process.exit(1);
}

if (cat.flavor !== "forever") problems.push(`top-level "flavor" must be "forever"`);
if (!Array.isArray(cat.addons)) problems.push(`top-level "addons" must be an array`);

const ids = new Set();
const folderOwner = new Map();
const idRe = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const repoRe = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const dateRe = /^\d{4}-\d{2}-\d{2}$/;
// Same hosts as allowed_image_host in src-tauri/src/icons.rs.
function allowedImageHost(value) {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || url.username || url.password) return false;
    const host = url.hostname;
    return host === "github.com" || host.endsWith(".githubusercontent.com") || host === "media.forgecdn.net"
      || host === "addons.wago.io" || host.endsWith(".wago.io") || host === "www.wowinterface.com" || host === "cdn-wow.mmoui.com";
  } catch {
    return false;
  }
}
const knownKeys = new Set(["id", "name", "desc", "category", "github", "wago", "wowi", "tukui", "curse", "asset_hint", "folders", "group_required_dependencies", "curse_locked", "url", "forever", "icon", "notice", "added", "github_moved_from"]);

for (const [i, a] of (cat.addons || []).entries()) {
  const where = `addons[${i}]${a && a.id ? ` (${a.id})` : ""}`;
  if (!a || typeof a !== "object") {
    problems.push(`${where}: not an object`);
    continue;
  }
  for (const k of Object.keys(a)) if (!knownKeys.has(k)) problems.push(`${where}: unknown field "${k}"`);
  if (typeof a.id !== "string" || !idRe.test(a.id)) problems.push(`${where}: "id" must be kebab-case (a-z, 0-9, dashes)`);
  else if (ids.has(a.id)) problems.push(`${where}: duplicate id`);
  else ids.add(a.id);
  if (typeof a.name !== "string" || !a.name.trim()) problems.push(`${where}: "name" is required`);
  if (a.desc !== undefined && typeof a.desc !== "string") problems.push(`${where}: "desc" must be a string`);
  if (a.github !== undefined && (typeof a.github !== "string" || !repoRe.test(a.github))) problems.push(`${where}: "github" must be owner/repo`);
  if (a.github && /^(curseforge-mirror|hippuli)\//i.test(a.github)) problems.push(`${where}: unauthorised mirror repos are not accepted`);
  if (a.github_moved_from !== undefined) {
    if (!Array.isArray(a.github_moved_from) || !a.github_moved_from.every((r) => typeof r === "string" && repoRe.test(r))) problems.push(`${where}: "github_moved_from" must be a list of owner/repo`);
    else if (!a.github) problems.push(`${where}: "github_moved_from" needs a current "github"`);
    else if (a.github_moved_from.some((r) => r.toLowerCase() === a.github.toLowerCase())) problems.push(`${where}: "github_moved_from" must not include the current repo`);
  }
  if (a.wago !== undefined && (typeof a.wago !== "string" || !/^[A-Za-z0-9]{6,12}$/.test(a.wago))) problems.push(`${where}: "wago" must be the Wago project id`);
  if (a.wowi !== undefined && !Number.isInteger(a.wowi)) problems.push(`${where}: "wowi" must be an integer id`);
  if (a.curse !== undefined && !Number.isInteger(a.curse)) problems.push(`${where}: "curse" must be an integer id`);
  if (a.tukui !== undefined && typeof a.tukui !== "string") problems.push(`${where}: "tukui" must be a slug`);
  if (a.asset_hint !== undefined && typeof a.asset_hint !== "string") problems.push(`${where}: "asset_hint" must be a string`);
  if (a.url !== undefined && !/^https?:\/\//.test(a.url)) problems.push(`${where}: "url" must start with http(s)://`);
  if (a.icon !== undefined && (typeof a.icon !== "string" || !allowedImageHost(a.icon))) problems.push(`${where}: "icon" must be an https URL on GitHub, CurseForge's media host, Wago or WoWInterface`);
  if (a.notice !== undefined && (typeof a.notice !== "string" || a.notice.length > 160)) problems.push(`${where}: "notice" must be a short string (160 chars max)`);
  if (a.added !== undefined && !dateRe.test(a.added)) problems.push(`${where}: "added" must be YYYY-MM-DD`);
  if (a.forever !== undefined && a.forever !== null && typeof a.forever !== "boolean") problems.push(`${where}: "forever" must be true, false or null`);
  if (a.curse_locked !== undefined && (typeof a.curse_locked !== "boolean" || a.curse === undefined)) problems.push(`${where}: "curse_locked" must be a boolean on an entry with a curse id`);
  if (a.group_required_dependencies !== undefined && typeof a.group_required_dependencies !== "boolean") problems.push(`${where}: "group_required_dependencies" must be true or false`);
  if (!Array.isArray(a.folders) || a.folders.length === 0 || !a.folders.every((f) => typeof f === "string" && f.trim() && !/[\\/]/.test(f))) {
    problems.push(`${where}: "folders" must list at least one AddOns folder name (no slashes)`);
  } else {
    for (const f of a.folders) {
      const key = f.toLowerCase();
      if (folderOwner.has(key) && folderOwner.get(key) !== a.id) problems.push(`${where}: folder "${f}" is also claimed by "${folderOwner.get(key)}"`);
      folderOwner.set(key, a.id);
    }
  }
  const hasSource = a.github || a.wago || a.wowi || a.tukui;
  if (!hasSource && !a.curse && !a.url) problems.push(`${where}: needs a source (github/wago/wowi/tukui) or at least a curse id / url for a link-only entry`);
}

// bundles: every id must exist
const known = new Set(cat.addons.map((a) => a.id));
for (const [i, b] of (cat.bundles || []).entries()) {
  const where = `bundles[${i}]${b && b.id ? ` (${b.id})` : ""}`;
  if (!b || typeof b.id !== "string" || !idRe.test(b.id)) problems.push(`${where}: "id" must be kebab-case`);
  if (typeof b.name !== "string" || !b.name.trim()) problems.push(`${where}: "name" is required`);
  if (!Array.isArray(b.addons) || !b.addons.length) problems.push(`${where}: "addons" must list catalogue ids`);
  else for (const id of b.addons) if (!known.has(id)) problems.push(`${where}: unknown addon id "${id}"`);
}

if ("featured" in cat) problems.push('"featured" was removed in build 20: starter packs are the only spotlight now');

if (problems.length) {
  console.error(`catalog/forever.json: ${problems.length} problem(s)\n` + problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
const withSource = cat.addons.filter((a) => a.github || a.wago || a.wowi || a.tukui).length;
console.log(`catalog ok: ${cat.addons.length} entries, ${withSource} installable, ${cat.addons.length - withSource} link-only`);

// Apps from build 32 only use GitHub's copy when forever.json.sig matches it.
// Contributors can't sign, so this only warns: the maintainer re-signs after merging
// (node scripts/sign-release.js --text catalog/forever.json).
try {
  const crypto = require("crypto");
  const signing = fs.readFileSync(path.join(__dirname, "..", "src-tauri", "src", "signing.rs"), "utf8");
  const block = signing.slice(signing.indexOf("RELEASE_PUBLIC_KEY: Option"));
  const bytes = [...block.slice(0, block.indexOf("]);")).matchAll(/0x([0-9a-f]{2})/gi)].map((m) => parseInt(m[1], 16));
  const key = crypto.createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(bytes)]), format: "der", type: "spki" });
  const data = Buffer.from(fs.readFileSync(file).filter((b) => b !== 13));
  const sig = Buffer.from(fs.readFileSync(file + ".sig", "utf8").trim(), "base64");
  if (crypto.verify(null, data, key, sig)) console.log("catalogue signature ok");
  else console.warn("warning: catalog/forever.json.sig does not match; apps will keep their built-in copy until the maintainer re-signs it");
} catch (e) {
  console.warn("warning: catalogue signature not checked: " + e.message);
}
