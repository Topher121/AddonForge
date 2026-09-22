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
- `ui/app.js` — three tabs: **My addons** (stats row, search, All /
  Updates / Need attention filters, per-row actions in a "..." menu),
  **Discover** (GitHub-link install box + catalogue), **Settings** (install
  folder, pre-releases, Wago key, export/import, report a problem, about).
  Redesigned 2026-09-22 by a second session (green accent theme, footer
  "No accounts. No ads. Just addons."); `node scripts/test-ui.js` runs its
  DOM-free behaviour checks (counts, filters, search, escaping, modal).
  Keep both `cargo test` and that script green.

## Footprint (measured 2026-09-22, 20s idle after launch, private bytes)
Exe ~5 MB on disk. v0.1.0: ~170 MB private total (app ~6 MB + six WebView2
helper processes; working set ~350 MB but that double-counts shared Edge
pages). v0.2.0 added `additionalBrowserArgs` in tauri.conf.json
(`--disable-gpu --in-process-gpu --renderer-process-limit=1`, background
networking/component-update/crash reporter off, plus
`--enable-features=NetworkServiceInProcess2`): **~97 MB private**, five
processes, GPU and network-service processes gone. WebView2 is the floor; going lower means a native egui window
instead of a webview (big rewrite, not planned). Measure: Get-CimInstance
Win32_Process filtered on the exe pid + msedgewebview2 whose CommandLine
contains "addonforge", sum PrivateMemorySize64 (snippet in the 2026-09-22
session). NOTE: `additionalBrowserArgs` REPLACES Tauri's defaults, so the
`msWebOOUI,msPdfOOUI,msSmartScreenProtection` disable-features must stay.

## Installer (added 2026-09-22, build 6)
`npx tauri build` (no `--no-bundle`) also produces an NSIS installer
`AddonForge_<ver>_x64-setup.exe`; build.ps1 copies it to
`build\AddonForge-Setup-v<ver>-b<n>.exe` and attaches it to the release
beside the portable exe. Per-user install (no admin): exe + uninstaller in
`%LOCALAPPDATA%\AddonForge\addonforge.exe`, Start-menu shortcut,
Settings → Apps uninstall entry. Silent install: `-Setup.exe /S`.
Self-update tells the two apart by filename: `AddonForge-v*-b*.exe` =
portable (new file beside the old one), anything else = installed
(rename running exe to `.exe.old`, put the new build at the SAME path so
shortcuts keep working, new process deletes `.old`). Both paths tested
against a local server 2026-09-22. latest.json always points at the raw
exe, never the installer: an installed copy updates by swapping its exe.
the owner installed build 6 on this PC that day.

## v0.2.0 features (built 2026-09-22, all the owner-approved that day)
- **Self-update**: build.ps1 writes `build\latest.json` ({version, build,
  filename, url, size, notes}); `-Release` publishes a GitHub release
  `v<ver>-b<n>` with the exe + latest.json. The app fetches
  `releases/latest/download/latest.json` on launch (`ADDONFORGE_UPDATE_URL`
  env overrides for testing), shows a banner, downloads the new exe BESIDE
  the running one, starts it with `--replaced <old>`, exits; the new exe
  deletes the old file. Build number is baked in via `ADDONFORGE_BUILD`
  env at compile time (`option_env!`), 0 = dev build. Tested end-to-end
  against a local server 2026-09-22, and live against the real release
  URL the same day once the repo went public.
- **Install from GitHub link** (Browse tab box; `--cli github owner/repo`):
  key `github:<owner/repo>` unless the catalogue knows the repo. Managed
  source wins over catalogue/TOC when picking where to update from.
- **Pre-release toggle** (Settings → Updates; state `allow_prerelease`):
  GitHub uses the API list (1 call per addon, 60/hr unauth) and prefers a
  release's own release.json asset; Wago prefers beta over stable.
- **Missing dependencies**: required deps (`## Dependencies` /
  `RequiredDeps`, minus `Blizzard_*`) not on disk are listed per package
  with an install button when the catalogue has them.
- **Pin / Ignore** (state `pinned` / `ignored` sets): pinned = excluded
  from Update All, still checked; ignored = hidden + not checked.
- **Export / Import** (Settings → Your addon list): JSON
  `{addonforge:1, flavor, addons:[{name,key,source,version,folders}]}`;
  import shows a plan and installs the missing installable ones.
- **Report a problem**: builds diagnostics (version, install, addon list,
  last 40 log lines) → prefilled GitHub issue or mailto to the studio
  address, plus copy-to-clipboard. Deliberately NOT the studio Firestore
  reporter: this app promises no telemetry.
- **Local log**: `%APPDATA%\AddonForge\addonforge.log`, 1 MB rotation,
  `logi!`/`loge!` macros (src/log.rs). Never uploaded.
- **Zip safety** (install.rs `safety_scan`): after extraction, refuse the
  whole install if any file has an executable extension (exe/dll/bat/ps1/
  js/…) or starts with an `MZ` header, or is a symlink. Unit-tested.

## Headless mode (use it to test; no window needed)
`addonforge.exe --cli <cmd>` prints JSON: `installs` (drive scan, auto-picks
the Forever install), `use <dir>`, `scan`, `check` (scan + remote versions),
`catalog`, `resolve <id..>|all` (dry-run every catalogue entry — run this
after editing the catalogue: 2026-09-22 result 91/92 GitHub entries OK),
`update <key> [--force]`, `install <catalog-id> [--force]`. Same state file
as the GUI. Debug exe at `src-tauri\target\debug\addonforge.exe`.

## Grouping rules (lib.rs `build_packages`)
Folders → packages, in priority: (1) folders recorded in
`state.installed[key].folders` stay with that key; (2) catalogue match by
folder name, then Wago/Curse/WoWI id; (3) TOC ids (`wago:` / `wowi:` /
`curse:` keys); (4) standalone `folder:<name>`, BUT a standalone folder
that declares `## X-Part-Of: Parent` or depends on a sibling sharing its
name stem (`BigWigs_Sporefall`→`BigWigs_Core`→`BigWigs`, `DBM-GUI`→
`DBM-Core`) inherits that parent's key. Display name = the group's root
folder (nothing in the group depends on it), then shortest name. Unit tests
cover this — keep them green.

## Catalogue notes
- 98 entries seeded 2026-09-22 from a research pass (GitHub code search for
  `16001` in .toc files + wow4ever.quest / Warcraft Tavern lists). 91 have
  GitHub releases; 6 are link-only (Leatrix, Baganator, Auctionator,
  Platynator, Bagnon, Details) because the authors have no GitHub releases
  and DMCA the mirrors — never seed `curseforge-mirror/*` or `hippuli/*`.
- Some authors list 16001 in the TOC but not `forever` in release.json
  (TellMeWhen, idTip, Molinari, IceHUD, Breakables, CharacterNotes…). Their
  catalogue entry says `forever: true` and the app treats that as vouched
  (no "not for Forever" prompt). If one turns out broken, flip the flag.
- Per-flavour asset repos (d4kir92/*, Crasling, Jonathas-Conceicao) need
  `asset_hint: "-forever"`; substring match, case-insensitive.
- GitHub `releases/latest` ignores pre-releases, so prerelease-only Forever
  builds (What's Training beta, Wayfinder) can't be installed yet.

## Studio hooks
- Ship = commit AND push. GitHub `Topher121/AddonForge` is **PUBLIC** (the owner
  made the call 2026-09-22): the live catalogue fetch, pull-request
  submissions and self-update all depend on that. Releases: `build.ps1
  -Release` (or `-DeliverOnly -Release` to publish an existing build)
  creates tag `v<ver>-b<n>` with the exe + latest.json. First public
  release: v0.2.0-b3.
- `build.ps1` (working since 2026-09-22): bumps `build.buildnum`, optional
  `-SetVersion x.y.z` (syncs tauri.conf.json, Cargo.toml, package.json),
  runs `cargo test`, `npx tauri build --no-bundle` with `ADDONFORGE_BUILD`
  set, archives to `buildAddonForge-v<ver>-b<n>.exe` (keeps 5), writes
  `buildlatest.json` + `build.note` (`-Note "..."`, omitted = previous),
  and with `-Release` publishes the GitHub release. `-DeliverOnly -Release`
  publishes the newest existing exe. Keep it ASCII-only (PS 5.1 chokes on
  UTF-8 punctuation without a BOM).
- **NOT on the PhoneApps page** (the owner 2026-09-22: it is a Windows tool,
  GitHub Releases is the download). The PhoneApps folder + launcher card
  were removed; `PhoneAppsprojects.json` keeps the desk entry (slug
  `addonforge`, tenant `design`) with a note. Do not re-add a card.
- Launcher card + `PhoneApps\projects.json` entry added 2026-09-22 (slug
  `addonforge`, tenant `design`, phase proto 15%). Office write-backs go to
  `tenant=design&project=addonforge`. Studio FAQ row added the same day.
- Support email everywhere: PocketForgeStudios@proton.me. "Support the dev"
  link (About tab + README) is the STUDIO Ko-fi: https://ko-fi.com/pocketforgestudios
  (the owner set it up 2026-09-22 with a PayPal Business account
  paypal.me/PocketForgeStudios underneath; both logged in the studio ledger +
  FAQ). Never link a personal PayPal again; build 4 briefly did.
- Related: the owner writes his own Forever addons in `Desktop\wow-addons`
  (FishEasy, RepHelper — they show as "No source" here, which is correct).
