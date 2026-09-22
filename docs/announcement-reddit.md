Submit link: https://www.reddit.com/r/wowforever/submit  (choose "Text" post; r/classicwow a day later if it lands well)

Check the sidebar rules first: some game subs require a "Tool" or "Addon" flair or a self-promotion disclosure (the first line already says it is your own project).

=====================================================================
TITLE (pick one)
=====================================================================

I made a small, open-source addon manager for WoW: Forever. No ads, no accounts, no telemetry.

AddonForge: a lightweight WoW Forever addon manager that only talks to GitHub / Wago / WoWInterface

=====================================================================
BODY (Reddit markdown, paste as-is)
=====================================================================

I've been playing the Forever beta and got fed up with the addon manager options, so I built my own. It's called **AddonForge** and it's free and open source.

**What it is**

- A tiny Windows app (5 MB, installer or portable exe) that reads your Forever AddOns folder, groups everything properly, and checks for updates in parallel.
- It only talks to the sites addons are actually published on: GitHub Releases, Wago, WoWInterface and TukUI. No account, no ads, no analytics, and no server of mine in the middle. Your PC talks to GitHub the same way your browser would.
- Comes with a catalogue of around 100 addons that already have Forever builds (BigWigs, DBM, Plater, RestedXP, BetterBags and lots of smaller ones). You can also paste any GitHub repo link and it installs from there.
- If an addon is CurseForge-only, it says so and gives you the link instead of pretending it can update it. Their API terms don't allow a key to ship inside an app, so I'd rather be upfront than hide those addons.
- Refuses any addon zip that contains an executable. Addons are Lua and XML, nothing else should be in there.
- Pin an addon to hold its version, ignore ones you don't care about, export your list to send a mate. Updates itself when I ship a new build.
- Memory: around 100 MB idle. It uses the WebView2 that Windows already has instead of bundling a browser.

**What it isn't**

- Forever only for now. Retail and Era aren't supported yet.
- Not code-signed, so Windows shows "Windows protected your PC" on first run. Click *More info*, then *Run anyway*. A signing certificate costs real money every year, which doesn't make sense for a free tool until people actually use it.
- Wago-only addons need your own free Wago API key pasted into Settings.
- ElvUI shows up but is flagged "no Forever build" because there isn't one yet. It'll work the moment TukUI publishes one.

**Download:** https://github.com/Topher121/AddonForge/releases/latest

**Source and catalogue:** https://github.com/Topher121/AddonForge

The catalogue is a JSON file in the repo. If your addon or a favourite is missing and the author publishes on GitHub, open a pull request or an issue and I'll add it. There's a "Report a problem" button in the app that pre-fills a GitHub issue with diagnostics if anything breaks.

Not affiliated with CurseForge, Overwolf or Blizzard. Made by one person, so be gentle with the bug reports, but do send them.

=====================================================================
REPLY TO HAVE READY: "why not just use WowUp?"
=====================================================================

WowUp is fine, I used it for years. Two things bugged me: it's an Electron app, so it sits at several hundred MB, and the CurseForge side goes through Overwolf. AddonForge is about a fifth of the memory and never phones anyone but the addon hosts. If WowUp works for you, keep using it. This is for people who wanted something smaller and quieter.

=====================================================================
REPLY TO HAVE READY: "is it safe / why should I trust a random exe?"
=====================================================================

Fair question. The whole thing is open source (MIT) so you can read every line, and the GitHub Actions on the repo build and test it. It never sends anything anywhere except HTTPS requests to GitHub, Wago, WoWInterface and TukUI to fetch addon files, and one request to GitHub on launch to see if there's a newer build of AddonForge itself. The settings and log live in %APPDATA%\AddonForge. If you'd rather not run it, that's completely reasonable.
