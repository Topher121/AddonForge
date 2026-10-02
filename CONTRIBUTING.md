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
| `github` | one source | `owner/repo` that publishes GitHub Releases. Preferred: no key, no rate limit when the release has a `release.json` (the BigWigs packager makes one). |
| `wago` | one source | Wago Addons project id. Users need their own free Wago key. |
| `wowi` | one source | WoWInterface file id. |
| `tukui` | one source | TukUI slug (`elvui`, `tukui`). |
| `curse` | no | CurseForge project id. Link only, never downloaded. An entry with only `curse` is a link-only entry so people know where the addon lives. |
| `asset_hint` | no | substring that picks the right zip when a release has several and no `release.json`, e.g. `"-forever"`. |
| `forever` | no | `true` if the author ships a Forever build (interface 16001), `false` if not yet, leave out if unsure. Be honest: `true` suppresses the "not for Forever" warning. |
| `desc`, `category`, `url` | no | shown in Browse |
| `notice` | no | one short line (160 chars) shown in amber on the installed row. For "remove this once X is fixed" situations. |
| `added` | no | `YYYY-MM-DD` the entry was added. Feeds "New in the catalogue" when nothing is featured. |

Rules:

- Only the author's own publishing location. Unofficial mirrors (for example `curseforge-mirror/*` or `hippuli/*`) are refused; several authors have DMCA'd them.
- No zips, no binaries, no scripts in this repo. The catalogue is JSON only.
- If an author asks for their addon to be removed, it goes.

## Getting your addon featured

The top of Find addons shows up to four featured addons for the week, with one sentence from the author. It is free and always will be; there is no paid placement. To apply, open a [Feature my addon](https://github.com/Topher121/AddonForge/issues/new?template=feature.yml) request. You must be the addon's author and it must already be in the catalogue. Accepted requests are added to the `featured` list by pull request:

```json
{ "id": "bigwigs", "from": "2026-11-09", "to": "2026-11-15", "blurb": "One sentence from the author." }
```

Weeks with no featured addons show what was recently added instead, so nobody has to apply to keep the strip alive.

## Code changes

Rust core in `src-tauri/`, plain HTML/JS in `ui/`. `cargo test` must pass. Keep the promises in the README: no accounts, no ads, no analytics, no server of ours in the loop, and no CurseForge API key in the exe.

Headless mode is the quickest way to test without the window:

```
addonforge.exe --cli scan
addonforge.exe --cli check
addonforge.exe --cli resolve all
```
