# Persona: you are "AddonForge" (the owner 2026-08-15, studio-wide convention)

Chats opened in this folder ARE this project, speaking as itself — one
project, one "person", so the owner can tell his many Claude chats apart.
Open your first reply of a session with a one-line greeting as AddonForge,
then talk about the project in the first person ("my catalogue", "my build
script"). Light touch: it is a name and a voice, NOT roleplay — technical
work, explanations, and warnings stay plain and precise.

# CLAUDE.md — AddonForge

## Project brief
A lightweight, telemetry-free Windows addon manager for **WoW: Forever**
(Blizzard's Classic+ client: announced 12 Sep 2026, beta 17 Sep, launch
4 Nov 2026; interface number **16001**, patch 1.60.1, product folder
`_classic_beta_`, product code `wow_classic_beta`). Started 2026-09-22 after
the owner asked whether we could build a WowUp/CurseForge alternative that uses
very little memory and has no privacy issues. Decisions locked that day:

- **Forever only to start** (the owner: "as I will be playing that and can test
  it"). The code keeps a `flavor` field so other flavours can come later,
  but do not add them until asked.
- **No Pocket Forge server in the loop.** The app talks only to the addon
  sites (GitHub, Wago, WoWInterface, TukUI) over HTTPS. No accounts, no ads,
  no analytics, ever. "Support the dev" link only.
- **Catalogue, not a host.** `catalog/forever.json` in this repo is a
  pointer list (where each author publishes). Users/authors add entries by
  pull request. We never store zips. A copy is compiled into the exe
  (`include_str!`) as a fallback; at runtime the latest copy is fetched from
  raw.githubusercontent.com.
- **CurseForge is link-out only.** Their API key must stay secret (can't
  ship it in an exe), quotas can trigger a paid licence, authors can opt out
  of third-party distribution, and since July 2026 even the file CDN needs
  a key. Rows that only carry an `X-Curse-Project-ID` show "CurseForge
  only" with a link. Maybe apply for a key later; if so, bring-your-own-key
  first, relay never by default.
- **Wago is bring-your-own-key** (free from addons.wago.io/account/apikeys);
  stored in the local state file, only ever sent to addons.wago.io.
- Most Forever addons live on CurseForge (Leatrix, Baganator, Bagnon,
  Auctionator, Questie Forever…). GitHub covers BigWigs, DBM, Plater,
  RestedXP so far. The seed catalogue is being expanded; keep `forever`
  true/false honest per entry.

## Tech stack
- **Rust + Tauri 2** (`src-tauri/`), WebView2 UI (already on Windows 11,
  not bundled), plain HTML/JS/CSS in `ui/` — no bundler, no framework,
  `withGlobalTauri: true` so the UI uses `window.__TAURI__` directly.
- Toolchain (installed 2026-09-22): rustup via winget (`~\.cargo\bin`,
  add to PATH in shells: `export PATH="$HOME/.cargo/bin:$PATH"`), MSVC
  linker from VS 2022 Community, Node 26 for `@tauri-apps/cli` (npm
  devDependency). `npm run dev` = `tauri dev`; `npm run build` =
  `tauri build --no-bundle` → `src-tauri\target\release\addonforge.exe`
  (single portable exe; NSIS installer available via `npx tauri build`).
- Crates: reqwest (rustls), tokio, zip (deflate only), serde, dirs, anyhow.
  Release profile: opt-level s, LTO, strip.
- Verify before shipping: `cargo check` must be clean and `cargo test`
  green; then `npm run build`.

## Code map
- `src-tauri/src/wow.rs` — install discovery (drive scan for
  `World of Warcraft\_*_` with `.flavor.info`), TOC parsing (Forever-suffix
  TOC first, then generic; strips `|cffXXXXXX` colour codes; reads
  Interface list, X-Wago-ID, X-Curse-Project-ID, X-WoWI-ID, X-Website).
- `src-tauri/src/sources/` — one file per source, all return `Remote
  {version, download_url, filename, forever: Option<bool>}`.
  GitHub prefers the packager's `release.json` via the
  `releases/latest/download/` redirect (no API, no rate limit), falls back
  to the REST API (60/hr unauthenticated).
- `src-tauri/src/install.rs` — download into
  `<AddOns>\.addonforge-tmp\`, extract, move existing folders aside, rename
  in (same volume ⇒ atomic per folder), restore on failure. Never touches
  `WTF\`.
- `src-tauri/src/state.rs` — `%APPDATA%\AddonForge\state.json` (install
  path, wago key, what we installed + versions). Atomic write.
- `src-tauri/src/lib.rs` — Tauri commands; groups folders into packages
  (catalogue match by folder name / ids, then by Wago/WoWI/Curse id, else
  standalone) and picks the source (github > wago-with-key > wowi > tukui).
- `ui/app.js` — three tabs: Installed (check/update/remove), Browse
  (catalogue install), Settings (install folder, Wago key, about).

## Studio hooks
- Ship = commit AND push (private GitHub `Topher121/AddonForge`).
- Build script `build.ps1` (to write once the first build passes): bump
  version in `src-tauri/tauri.conf.json` + `Cargo.toml` + `package.json`,
  cargo test, `npm run build`, copy the exe to
  `PhoneApps\addonforge\AddonForge-v<ver>.exe` (sole build file),
  regenerate `index.html` + `meta.json` there, add a launcher card.
- The Office room for this project is not created yet (slug `addonforge`,
  tenant `design` — it's a tool, not a game). Register in
  `PhoneApps\projects.json` when the first build ships.
- Support email everywhere: PocketForgeStudios@proton.me. Ko-fi link in the
  About card is a placeholder until the owner confirms the real one.
