# AddonForge

A small, fast addon manager for **World of Warcraft: Forever**.

- **Light.** A single exe on top of the WebView2 that Windows already has. No bundled browser.
- **Private.** No account, no ads, no analytics, no server of ours in the loop. The app talks only to the sites addons are published on (GitHub, Wago, WoWInterface, TukUI), the same way your browser would.
- **Honest.** If an addon only publishes on CurseForge, AddonForge says so and gives you the link instead of pretending.

## How it finds addons

The app reads your `Interface\AddOns` folder and each addon's `.toc` file. Authors put their Wago / WoWInterface / CurseForge ids in there, and the BigWigs packager most authors use publishes a `release.json` with every GitHub release, so matching what you have to where it came from is mostly reading text files.

On top of that there is a **catalogue**: [`catalog/forever.json`](catalog/forever.json). It is a pointer list, not a host. Each entry says where the author publishes releases; downloads always come straight from there. Anyone can add an addon with a pull request.

## Sources

| Source | Key needed? | Notes |
|---|---|---|
| GitHub releases | No | Preferred. Uses the packager's `release.json` when present. |
| Wago Addons | Your own free key | Paste it in Settings. Stored locally, only sent to addons.wago.io. |
| WoWInterface | No | Public JSON API. |
| TukUI | No | ElvUI / Tukui. |
| CurseForge | Link only | Their API terms don't allow a key to ship inside an app. |

## Build

Needs Rust (rustup), the MSVC build tools and Node.

```
npm install
npm run dev      # run with hot reload of the UI folder
npm run build    # portable exe in src-tauri/target/release/
```

## Support

Bugs and ideas: open an issue, or email PocketForgeStudios@proton.me.
If AddonForge saves you some hassle, the support link in the app's About tab keeps the lights on. No pressure.

Made by Pocket Forge Studios.
