# Contributing

## Adding an addon to the catalogue

The catalogue is [`catalog/forever.json`](catalog/forever.json). It is a pointer list, not a host: each entry says where the author publishes releases, and AddonForge downloads straight from there. We never copy zips anywhere.

1. Fork the repo and add one object to the `addons` array.
2. Run `node scripts/validate-catalog.js` (it also runs on your pull request).
3. Open the pull request. Say in a sentence where you checked that the Forever build exists.

An entry looks like this:

```json
{
  "id": "bigwigs",
  "name": "BigWigs",
  "desc": "Boss encounter alerts and timers.",
  "category": "Bosses",
  "github": "BigWigsMods/BigWigs",
  "wago": "5NRegwG3",
  "wowi": 5086,
  "curse": 2382,
  "folders": ["BigWigs", "BigWigs_Core", "BigWigs_Options", "BigWigs_Plugins"],
  "url": "https://github.com/BigWigsMods/BigWigs",
  "forever": true
}
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | kebab-case, unique |
| `name` | yes | display name |
| `folders` | yes | top-level folders the zip installs into `Interface\AddOns`. Used to recognise what's already on disk. |
| `group_required_dependencies` | no | group the first folder's direct required dependencies into this addon; defaults to `false`. See below. |
| `github` | one source | `owner/repo` that publishes GitHub Releases. Preferred: no key, no rate limit when the release has a `release.json` (the BigWigs packager makes one). |
| `wago` | one source | Wago Addons project id. Users need their own Wago key, which Wago only gives to Patreon supporters, so prefer a GitHub source where one exists. |
| `wowi` | one source | WoWInterface file id. |
| `tukui` | one source | TukUI slug (`elvui`, `tukui`). |
| `curse` | no | CurseForge project id. Link only, never downloaded. An entry with only `curse` is a link-only entry so people know where the addon lives. |
| `asset_hint` | no | substring that picks the right zip when a release has several and no `release.json`, e.g. `"-forever"`. |
| `forever` | no | `true` if the author ships a Forever build (interface 16001), `false` if not yet, leave out if unsure. Be honest: `true` suppresses the "not for Forever" warning. |
| `desc`, `category`, `url` | no | shown in Browse |
| `notice` | no | one short line (160 chars) shown in amber on the installed row. For "remove this once X is fixed" situations. |
| `added` | no | `YYYY-MM-DD` the entry was added. Kept for the record and for sorting later. |

Set `group_required_dependencies: true` only when the download bundles the
first folder's required dependencies, as Questie does with QuestieDB. Keep those
dependencies out of `folders` so they do not identify the addon on their own.
When the first folder is present, its direct `Dependencies` / `RequiredDeps`
join the package unless another managed or identified package owns them.
This opt-in skips optional dependencies, does not follow dependency chains, and
leaves dependencies claimed by multiple opted-in packages unassigned. Existing
module grouping still applies.
This controls grouping only; update checks still use the source's package version.

Rules:

- Only the author's own publishing location. Unofficial mirrors (for example `curseforge-mirror/*` or `hippuli/*`) are refused; several authors have DMCA'd them.
- No zips, no binaries, no scripts in this repo. The catalogue is JSON only.
- If an author asks for their addon to be removed, it goes.

## Code changes

Rust core in `src-tauri/`, plain HTML/JS in `ui/`. `cargo test` must pass. Keep the promises in the README: no accounts, no ads, no analytics, no server of ours in the loop, and no CurseForge API key in the exe.

Headless mode is the quickest way to test without the window:

```
addonforge.exe --cli scan
addonforge.exe --cli check
addonforge.exe --cli resolve all
```
