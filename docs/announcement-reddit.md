> Human here - Yes, I asked Claude how to write a Reddit post because this was my first one ever. The replies below were prep for the questions I guessed would come up. The post itself I tweaked and reworded before posting, but the bones are here.

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

Before anyone says it: I know a lot of people want Forever to be a clean, no-addons experience, and honestly that was my plan too. But a few days into the beta there are a handful of small quality-of-life things I've really enjoyed having. The reason I don't just use one of the existing managers is the ads and the tracking that come with them. If you're going addon-free, fair play, this one isn't for you.

**What it is**

- A tiny Windows app (5 MB, installer or portable exe) that reads your Forever AddOns folder, groups everything properly, and checks for updates in parallel.
- It only talks to the sites addons are actually published on: GitHub Releases, Wago, WoWInterface and TukUI. No account, no ads, no analytics, and no server of mine in the middle. Your PC talks to GitHub the same way your browser would.
- Comes with a catalogue of around 100 addons that already have Forever builds (BigWigs, DBM, Plater, RestedXP, BetterBags and lots of smaller ones). You can also paste any GitHub repo link and it installs from there.
- If an addon is CurseForge-only, it says so and gives you the link instead of pretending it can update it. I'm going to apply for CurseForge API access so those can update through the app too, but that needs Overwolf's approval and even then some authors opt out of third-party apps, so no promises on timing.
- Refuses any addon zip that contains an executable. Addons are Lua and XML, nothing else should be in there.
- Pin an addon to hold its version, ignore ones you don't care about, export your list to send a mate. Updates itself when I ship a new build.
- Memory: around 100 MB idle. It uses the WebView2 that Windows already has instead of bundling a browser.

**What it isn't**

- Finished. This is v0.2, built during the beta, so expect rough edges. I'll patch it for the 4 November launch when the game folder changes.
- Forever only for now. Retail and Era aren't supported yet.
- Not code-signed, so Windows shows "Windows protected your PC" on first run. Click *More info*, then *Run anyway*. A signing certificate costs real money every year, which doesn't make sense for a free tool until people actually use it.
- Wago-only addons need your own free Wago API key pasted into Settings.
- ElvUI shows up but is flagged "no Forever build" because there isn't one yet. It'll work the moment TukUI publishes one.

**Download:** https://github.com/Topher121/AddonForge/releases/latest

**Source and catalogue:** https://github.com/Topher121/AddonForge

The catalogue is a JSON file in the repo. If your addon or a favourite is missing and the author publishes on GitHub, open a pull request or an issue and I'll add it. Addon authors: if you'd rather not be listed, say so and it comes out, no argument. There's a "Report a problem" button in the app that pre-fills a GitHub issue with diagnostics if anything breaks.

Which addons are you all running on Forever? That's the quickest way for me to find what's missing from the catalogue.

Not affiliated with CurseForge, Overwolf or Blizzard. Made by one person, so be gentle with the bug reports, but do send them.

=====================================================================
REPLY TO HAVE READY: "why not just use WowUp?"
=====================================================================

WowUp is fine, I used it for years. Two things bugged me: it's an Electron app, so it sits at several hundred MB, and the CurseForge side goes through Overwolf. AddonForge is about a fifth of the memory and never phones anyone but the addon hosts. If WowUp works for you, keep using it. This is for people who wanted something smaller and quieter.

=====================================================================
REPLY TO HAVE READY: "is it safe / why should I trust a random exe?"
=====================================================================

Fair question. The whole thing is open source (MIT) so you can read every line, and the GitHub Actions on the repo build and test it. It never sends anything anywhere except HTTPS requests to GitHub, Wago, WoWInterface and TukUI to fetch addon files, and one request to GitHub on launch to see if there's a newer build of AddonForge itself. The settings and log live in %APPDATA%\AddonForge. If you'd rather not run it, that's completely reasonable.
