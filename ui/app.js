// AddonForge UI. Plain JS, no framework: everything comes from Rust over IPC.
const T = window.__TAURI__;
const invoke = T.core.invoke;
const openUrl = (u) => T.opener.openUrl(u).catch((e) => toast(String(e), true));

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

let packages = [];
let catalog = [];
let busy = false;
let settings = null;
let selfUpdate = null;
let activeFilter = "all";
let bundles = [];
const icons = new Map(); // "p:<package key>" / "c:<catalogue id>" -> data URL, or null when there is none
const needsAttention = (p) => p.status === "error" || p.status === "no-key" || p.status === "no-source" || p.missing_deps.length > 0 || !p.supports_forever;

// ---------------------------------------------------------------- helpers
function iconTile(kind, id, name) {
  const src = icons.get(kind + ":" + id);
  const letter = esc(String(name || "?").replace(/^[^a-z0-9]+/i, "").charAt(0).toUpperCase() || "?");
  return `<span class="ico" data-ico="${esc(kind + ":" + id)}" aria-hidden="true">${src ? `<img src="${src}" alt="">` : letter}</span>`;
}
// Fetch icons for rows that don't have one yet and drop them into the DOM in place (no re-render).
async function loadIcons(kind, ids) {
  const want = ids.filter((id) => !icons.has(kind + ":" + id));
  if (!want.length) return;
  let got;
  try {
    got = await invoke(kind === "p" ? "package_icons" : "catalog_icons", kind === "p" ? { keys: want } : { ids: want });
  } catch (_) {
    return;
  }
  if (!got) return;
  for (const id of want) icons.set(kind + ":" + id, got[id] || null);
  for (const [id, src] of Object.entries(got)) {
    document.querySelectorAll(`[data-ico="${CSS.escape(kind + ":" + id)}"]`).forEach((el) => { el.innerHTML = `<img src="${src}" alt="">`; });
  }
}
function fmtCount(n) {
  if (n == null) return "";
  if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e7 ? 0 : 1).replace(/\.0$/, "") + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(n >= 1e4 ? 0 : 1).replace(/\.0$/, "") + "k";
  return String(n);
}
function ago(iso) {
  if (!iso) return "";
  const days = Math.floor((Date.now() - new Date(iso).getTime()) / 86400000);
  if (!Number.isFinite(days) || days < 0) return "";
  if (days === 0) return "updated today";
  if (days === 1) return "updated yesterday";
  if (days < 14) return `updated ${days} days ago`;
  if (days < 60) return `updated ${Math.floor(days / 7)} weeks ago`;
  if (days < 365) return `updated ${Math.floor(days / 30)} months ago`;
  return `updated ${Math.floor(days / 365)} year${days < 730 ? "" : "s"} ago`;
}

let toastTimer;
function toast(msg, bad = false) {
  const t = $("toast");
  t.textContent = msg;
  t.className = "toast show" + (bad ? " bad" : "");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = "toast"), bad ? 6000 : 3000);
}

function confirmModal(html, yes = "Continue") {
  return new Promise((resolve) => {
    const previousFocus = document.activeElement;
    $("modal-text").innerHTML = html;
    $("modal-yes").textContent = yes;
    $("modal").classList.remove("hidden");
    $("modal-no").focus();
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); done(false); }
      if (e.key === "Tab") {
        e.preventDefault();
        (document.activeElement === $("modal-no") ? $("modal-yes") : $("modal-no")).focus();
      }
    };
    const done = (v) => {
      $("modal").classList.add("hidden");
      $("modal-yes").onclick = $("modal-no").onclick = null;
      document.removeEventListener("keydown", onKey);
      previousFocus?.focus();
      resolve(v);
    };
    document.addEventListener("keydown", onKey);
    $("modal-yes").onclick = () => done(true);
    $("modal-no").onclick = () => done(false);
  });
}

function setBusy(b) {
  busy = b;
  document.querySelectorAll(".btn").forEach((el) => {
    if (el.id === "modal-yes" || el.id === "modal-no") return;
    el.disabled = b || (el.id === "btn-update-all" && !packages.some((p) => p.status === "update" && !p.pinned && !p.ignored));
  });
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (_) {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  }
}

document.addEventListener("click", (e) => {
  const navigation = e.target.closest("[data-navigate]");
  if (navigation) document.querySelector(`.tab[data-tab="${navigation.dataset.navigate}"]`)?.click();
  document.querySelectorAll(".more-actions[open]").forEach((menu) => {
    if (!menu.contains(e.target)) menu.open = false;
  });
  const a = e.target.closest("[data-url]");
  if (a && a.dataset.url) {
    e.preventDefault();
    openUrl(a.dataset.url);
  }
});

// ---------------------------------------------------------------- tabs
document.querySelectorAll(".tab").forEach((btn) =>
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => {
      b.classList.toggle("active", b === btn);
      if (b === btn) b.setAttribute("aria-current", "page");
      else b.removeAttribute("aria-current");
    });
    document.querySelectorAll(".panel").forEach((p) => p.classList.toggle("active", p.id === "tab-" + btn.dataset.tab));
    document.querySelector("main").scrollTop = 0;
    if ((btn.dataset.tab === "browse" || btn.dataset.tab === "start") && !catalog.length) loadCatalog(false);
    if (btn.dataset.tab === "settings") loadSettings();
  })
);

// ---------------------------------------------------------------- installed
function badge(p) {
  const map = {
    ok: "Up to date",
    update: "Update available",
    error: "Error",
    "curse-only": "CurseForge only",
    "no-source": "No source",
    "no-key": "Needs Wago key",
    unknown: "Not checked",
    checking: "Checking",
  };
  return `<span class="badge ${p.status}">${p.status === "checking" ? '<span class="spinner"></span> ' : ""}${map[p.status] || p.status}</span>`;
}

function visiblePackages() {
  const q = $("installed-filter").value.trim().toLowerCase();
  const showIgnored = $("show-ignored").checked;
  return packages.filter((p) => (showIgnored || !p.ignored) && (activeFilter === "all" || (activeFilter === "updates" ? p.status === "update" : needsAttention(p))) && (!q || `${p.name} ${p.folders.join(" ")} ${p.author || ""} ${p.source_label}`.toLowerCase().includes(q)));
}

function renderInstalled() {
  const list = $("installed-list");
  const rows = visiblePackages();
  $("stat-total").textContent = packages.length;
  $("stat-updates").textContent = packages.filter((p) => p.status === "update" && !p.ignored).length;
  $("stat-attention").textContent = packages.filter((p) => !p.ignored && needsAttention(p)).length;
  const eligible = packages.filter((p) => p.status === "update" && !p.pinned && !p.ignored).length;
  $("btn-update-all").textContent = eligible ? `Update all (${eligible})` : "Update all";
  if (!packages.length) {
    list.innerHTML = `<div class="empty"><strong>No addons found</strong>Nothing in this installation's AddOns folder yet.<br><button class="btn" data-navigate="start">Discover addons</button></div>`;
    $("summary").textContent = "";
    setBusy(busy);
    return;
  }
  list.innerHTML = rows.length
    ? rows
        .map((p) => {
          const ver = p.installed_version ? `<b>${esc(p.installed_version)}</b>` : `<span class="muted">unknown</span>`;
          const remote = p.remote_version && p.status === "update" ? `<span class="arrow">→</span><b>${esc(p.remote_version)}</b>${p.remote_prerelease ? ' <span class="badge pre">pre</span>' : ""}` : "";
          const flags = [
            p.supports_forever ? "" : `<span class="badge not-forever" title="No 16001 interface or _Forever toc found">not for Forever</span>`,
            p.pinned ? `<span class="badge pinned" title="Updates held back">pinned</span>` : "",
            p.ignored ? `<span class="badge managed">ignored</span>` : "",
          ].filter(Boolean).join(" ");
          const src = p.source_url
            ? `<a href="#" data-url="${esc(p.source_url)}">${esc(p.source_label || (p.curse_id ? "CurseForge" : "link"))}</a>`
            : `<span class="muted">${esc(p.source_label || "—")}</span>`;
          const canUpdate = p.source && p.status !== "no-key";
          const btnLabel = p.status === "update" ? "Update" : p.managed ? "Reinstall" : "Install";
          const noteCls = p.status === "error" ? "note err" : "note";
          const deps = p.missing_deps.length
            ? `<div class="note warn">Needs: ${p.missing_deps
                .map((d) => (d.catalog_id ? `<b>${esc(d.name || d.folder)}</b> <button class="mini" data-act="dep" data-id="${esc(d.catalog_id)}">install</button>` : `<b>${esc(d.folder)}</b> <i>(not in catalogue)</i>`))
                .join(", ")}</div>`
            : "";
          const sub = p.author ? esc(p.author) : "";
          return `<div class="item ${p.ignored ? "ignored" : ""} ${p.status === "no-source" || p.status === "curse-only" ? "dim" : ""}" data-key="${esc(p.key)}">
        <div class="c-name">${iconTile("p", p.key, p.name)}<div class="nm-wrap"><span class="nm">${esc(p.name)}</span>${flags ? `<span class="flags">${flags}</span>` : ""}${sub ? `<div class="sub-line">${sub}</div>` : ""}</div></div>
        <div class="c-ver">${ver}${remote}</div>
        <div class="c-src">${src}</div>
        <div class="c-status">${badge(p)}</div>
        <div class="c-act">
          ${canUpdate ? `<button class="btn small ${p.status === "update" && !p.pinned ? "primary" : ""}" data-act="update">${btnLabel}</button>` : ""}
          <details class="more-actions"><summary aria-label="Options for ${esc(p.name)}" title="Addon options">···</summary><div class="action-menu">
          <button class="btn small" data-act="pin" title="${p.pinned ? "Allow updates again" : "Hold this addon at its current version"}">${p.pinned ? "Allow updates" : "Pin version"}</button>
          <button class="btn small" data-act="ignore" title="${p.ignored ? "Show it again" : "Hide it from the list and stop checking it, e.g. your own addons or ones with no update source"}">${p.ignored ? "Unignore" : "Ignore"}</button>
          <button class="btn small" data-act="remove" title="Delete these folders from AddOns (saved settings in WTF are kept)">Remove</button>
          </div></details>
        </div>
        ${p.note ? `<div class="${noteCls}">${esc(p.note)}</div>` : ""}
        ${deps}
      </div>`;
        })
        .join("")
    : `<div class="empty"><strong>Nothing in this view</strong>Try another filter or clear the search.<br><button class="btn" data-reset-filters>Show all addons</button></div>`;
  const updates = packages.filter((p) => p.status === "update" && !p.ignored).length;
  const errors = packages.filter((p) => p.status === "error" && !p.ignored).length;
  const ignored = packages.filter((p) => p.ignored).length;
  $("summary").textContent =
    `Showing ${rows.length} of ${packages.length} addon${packages.length === 1 ? "" : "s"}` +
    (updates ? ` · ${updates} update${updates === 1 ? "" : "s"}` : "") +
    (errors ? ` · ${errors} error${errors === 1 ? "" : "s"}` : "") +
    (ignored ? ` · ${ignored} ignored` : "");
  setBusy(busy);
}

$("installed-filter").addEventListener("input", renderInstalled);
$("show-ignored").addEventListener("change", renderInstalled);
document.querySelectorAll("[data-filter]").forEach((btn) => btn.addEventListener("click", () => {
  activeFilter = btn.dataset.filter;
  document.querySelectorAll("[data-filter]").forEach((b) => {
    b.classList.toggle("active", b === btn);
    b.setAttribute("aria-pressed", String(b === btn));
  });
  renderInstalled();
}));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") document.querySelectorAll(".more-actions[open]").forEach((menu) => {
    menu.open = false;
    menu.querySelector("summary").focus();
  });
});
// Keep the options menu inside the window, including the last visible row.
$("installed-list").addEventListener("toggle", (e) => {
  const details = e.target;
  if (!details.matches(".more-actions") || !details.open) return;
  document.querySelectorAll(".more-actions[open]").forEach((other) => {
    if (other !== details) other.open = false;
  });
  const menu = details.querySelector(".action-menu");
  const rect = details.getBoundingClientRect();
  const height = menu.offsetHeight;
  menu.style.right = `${Math.max(12, window.innerWidth - rect.right)}px`;
  menu.style.top = `${Math.max(12, rect.bottom + height + 48 > window.innerHeight ? rect.top - height - 6 : rect.bottom + 6)}px`;
}, true);
document.querySelector("main").addEventListener("scroll", () => {
  document.querySelectorAll(".more-actions[open]").forEach((menu) => { menu.open = false; });
});

$("installed-list").addEventListener("click", async (e) => {
  if (e.target.closest("[data-reset-filters]")) {
    $("installed-filter").value = "";
    $("show-ignored").checked = true;
    document.querySelector('[data-filter="all"]').click();
    return;
  }
  const btn = e.target.closest("button[data-act]");
  if (!btn || busy) return;
  const key = btn.closest(".item").dataset.key;
  const p = packages.find((x) => x.key === key);
  if (!p) return;
  const act = btn.dataset.act;
  const menu = btn.closest("details");
  if (menu) menu.open = false;
  if (act === "update") await updateOne(p);
  if (act === "dep") {
    const entry = catalog.find((c) => c.id === btn.dataset.id) || { id: btn.dataset.id, name: btn.dataset.id };
    await installCatalog(entry, false);
  }
  if (act === "pin" || act === "ignore") {
    try {
      if (act === "pin") await invoke("set_pin", { key, pinned: !p.pinned });
      else await invoke("set_ignore", { key, ignored: !p.ignored });
      await rescan(true);
    } catch (err) {
      toast(String(err), true);
    }
  }
  if (act === "remove") {
    const ok = await confirmModal(`<p>Remove <b>${esc(p.name)}</b>?</p><p class="muted">Deletes: ${esc(p.folders.join(", "))}.<br>Your saved settings for it (in WTF) are left alone.</p>`, "Remove");
    if (!ok) return;
    setBusy(true);
    try {
      await invoke("remove_package", { key });
      toast(`Removed ${p.name}`);
      await rescan();
    } catch (err) {
      toast(String(err), true);
    } finally {
      setBusy(false);
    }
  }
});

function explainError(err) {
  const s = String(err);
  if (s.startsWith("SAFETY:")) return s.replace("SAFETY: refused, ", "Refused for safety: ");
  return s;
}

async function updateOne(p, allowNonForever = false) {
  setBusy(true);
  p.status = "checking";
  renderInstalled();
  try {
    const fresh = await invoke("update_package", { key: p.key, allowNonForever });
    const i = packages.findIndex((x) => x.key === p.key);
    if (i >= 0) packages[i] = fresh;
    toast(`${fresh.name} is now ${fresh.installed_version || "installed"}`);
  } catch (err) {
    if (String(err) === "NOT_FOREVER") {
      p.status = "update";
      renderInstalled();
      setBusy(false);
      const ok = await confirmModal(`<p><b>${esc(p.name)}</b>'s latest release does not list a Forever build.</p><p class="muted">It may still work, or it may error in game. Install it anyway?</p>`, "Install anyway");
      if (ok) return updateOne(p, true);
      return;
    }
    p.status = "error";
    p.note = explainError(err);
    toast(explainError(err), true);
  } finally {
    setBusy(false);
    renderInstalled();
  }
}

/// keepStatus: re-scan but keep remote versions we already know.
async function rescan(keepStatus = false) {
  setBusy(true);
  try {
    const fresh = await invoke("scan");
    if (keepStatus) {
      for (const f of fresh) {
        const old = packages.find((x) => x.key === f.key);
        if (old && old.remote_version) {
          f.remote_version = old.remote_version;
          f.remote_forever = old.remote_forever;
          f.remote_prerelease = old.remote_prerelease;
          f.update_available = old.update_available;
          f.status = old.status;
          f.note = old.note;
        }
      }
    }
    packages = fresh;
    renderInstalled();
    loadIcons("p", packages.map((p) => p.key));
    if (!packages.length) $("guide").open = true; // nothing installed yet: probably new to this
  } catch (err) {
    packages = [];
    renderInstalled();
    $("installed-list").innerHTML = `<div class="empty"><strong>No install selected</strong>${esc(String(err))}<br><button class="btn primary" data-navigate="settings">Choose WoW folder</button></div>`;
  } finally {
    setBusy(false);
  }
}

async function checkUpdates() {
  setBusy(true);
  $("btn-check").innerHTML = '<span class="spinner"></span> Checking…';
  packages.forEach((p) => {
    if (p.source && p.status !== "no-key" && !p.ignored) p.status = "checking";
  });
  renderInstalled();
  try {
    packages = await invoke("check_updates");
    renderInstalled();
    const n = packages.filter((p) => p.status === "update" && !p.ignored && !p.pinned).length;
    const unchecked = packages.filter((p) => !p.ignored && p.status !== "ok" && p.status !== "update").length;
    toast(n ? `${n} update${n === 1 ? "" : "s"} available` : unchecked ? `Check complete. ${unchecked} addon${unchecked === 1 ? " could" : "s could"} not be verified.` : "All checked addons are up to date");
  } catch (err) {
    toast(String(err), true);
  } finally {
    $("btn-check").textContent = "Check for updates";
    setBusy(false);
  }
}

async function updateAll() {
  const todo = packages.filter((p) => p.status === "update" && !p.pinned && !p.ignored);
  for (const p of todo) await updateOne(p);
  toast(`Updated ${todo.length} addon${todo.length === 1 ? "" : "s"}`);
}

$("btn-check").onclick = checkUpdates;
$("btn-rescan").onclick = () => rescan();
$("btn-update-all").onclick = updateAll;

// ---------------------------------------------------------------- browse
function sortedCatalog(rows) {
  const by = $("browse-sort").value || "name";
  const name = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  if (by === "downloads") return rows.sort((a, b) => (b.downloads || 0) - (a.downloads || 0) || name(a, b));
  if (by === "updated") return rows.sort((a, b) => String(b.updated_at || "").localeCompare(String(a.updated_at || "")) || name(a, b));
  return rows.sort(name);
}

function renderCatalog() {
  const q = $("browse-filter").value.trim().toLowerCase();
  const rows = sortedCatalog(catalog.filter((e) => !q || `${e.name} ${e.desc} ${e.category} ${e.github || ""}`.toLowerCase().includes(q)));
  $("browse-list").innerHTML = rows.length
    ? rows
        .map((e) => {
          const src = e.github ? `GitHub · ${e.github}` : e.wago ? "Wago" : e.wowi ? "WoWInterface" : e.tukui ? "TukUI" : "CurseForge only";
          const fv = e.forever === true ? `<span class="badge forever">Forever</span>` : e.forever === false ? `<span class="badge not-forever">No Forever build</span>` : "";
          const canInstall = e.github || e.wowi || e.tukui || (e.wago && settings?.has_wago_key);
          const needsKey = e.wago && !e.github && !e.wowi && !e.tukui && !settings?.has_wago_key;
          return `<div class="item" data-id="${esc(e.id)}">
            <div class="c-name clickable" data-detail="${esc(e.id)}" title="Details">${iconTile("c", e.id, e.name)}<div class="nm-wrap"><span class="nm">${esc(e.name)}</span><span class="flags">${fv}${e.installed ? ' <span class="badge ok">installed</span>' : ""}</span><div class="sub-line" title="${esc(e.desc || "")}">${esc(e.desc || "")}</div></div></div>
            <div class="c-cat muted">${esc(e.category || "")}</div>
            <div class="c-dl" title="${e.downloads != null ? esc(e.downloads.toLocaleString() + " downloads, all time, counted on " + (e.stat_source === "wowi" ? "WoWInterface" : "GitHub")) : "No download count for this source"}">${e.downloads != null ? `<b>${fmtCount(e.downloads)}</b>` : `<span class="muted">\u2014</span>`}${e.updated_at ? `<div class="sub-line">${ago(e.updated_at)}</div>` : ""}</div>
            <div class="c-src"><a href="#" data-url="${esc(e.url || (e.github ? "https://github.com/" + e.github : "#"))}">${esc(src)}</a></div>
            <div class="c-act">
              ${canInstall ? `<button class="btn small" data-act="install">${e.installed ? "Reinstall" : "Install"}</button>` : needsKey ? `<span class="badge no-key">needs Wago key</span>` : `<span class="badge curse-only">link only</span>`}
            </div>
          </div>`;
        })
        .join("")
    : `<div class="empty">Nothing matches.</div>`;
  $("browse-summary").textContent = `${rows.length} of ${catalog.length}`;
  setBusy(busy);
}

async function loadCatalog(refresh) {
  setBusy(true);
  try {
    catalog = await invoke("catalog_list", { refresh });
    renderCatalog();
    loadIcons("c", catalog.map((e) => e.id));
    loadBundles();
  } catch (err) {
    toast(String(err), true);
  } finally {
    setBusy(false);
  }
}

$("browse-filter").addEventListener("input", renderCatalog);
$("browse-sort").addEventListener("change", renderCatalog);

// ---------------------------------------------------------------- details window
let detailId = null;
let detailSeq = 0;
function sourceLabel(e) {
  return e.github ? `GitHub \u00b7 ${e.github}` : e.wago ? "Wago" : e.wowi ? "WoWInterface" : e.tukui ? "TukUI" : "CurseForge";
}
function entryUrl(e) {
  return e.url || (e.github ? "https://github.com/" + e.github : e.wago ? "https://addons.wago.io/addons/" + e.wago : e.wowi ? "https://www.wowinterface.com/downloads/info" + e.wowi : e.curse ? "https://www.curseforge.com/wow/addons/search?search=" + encodeURIComponent(e.name) : "");
}
async function openDetail(id) {
  const e = catalog.find((x) => x.id === id);
  if (!e) return;
  detailId = id;
  const seq = ++detailSeq;
  const src = icons.get("c:" + id);
  $("detail-icon").innerHTML = src ? `<img src="${src}" alt="">` : esc(e.name.replace(/^[^a-z0-9]+/i, "").charAt(0).toUpperCase() || "?");
  $("detail-icon").dataset.ico = "c:" + id; // filled in by loadIcons if it arrives later
  $("detail-name").textContent = e.name;
  const bits = [e.category, sourceLabel(e)];
  if (e.downloads != null) bits.push(`${fmtCount(e.downloads)} downloads`);
  if (e.updated_at) bits.push(ago(e.updated_at));
  $("detail-meta").innerHTML = bits.filter(Boolean).map(esc).join(" \u00b7 ") +
    (e.forever === true ? ' \u00b7 <span class="badge forever">Forever</span>' : e.forever === false ? ' \u00b7 <span class="badge not-forever">No Forever build</span>' : "") +
    (e.installed ? ' \u00b7 <span class="badge ok">installed</span>' : "");
  $("detail-desc").textContent = e.desc || "";
  $("detail-body").innerHTML = e.github ? '<div class="muted"><span class="spinner"></span> Reading its README\u2026</div>' : "";
  const canInstall = e.github || e.wowi || e.tukui || (e.wago && settings?.has_wago_key);
  const needsKey = e.wago && !e.github && !e.wowi && !e.tukui && !settings?.has_wago_key;
  $("detail-install").textContent = e.installed ? "Reinstall" : "Install";
  $("detail-install").classList.toggle("hidden", !canInstall);
  $("detail-note").textContent = needsKey ? "Needs a Wago key (Settings)." : canInstall ? "" : "Not installable from here: get it from its page.";
  $("detail-open").classList.toggle("hidden", !entryUrl(e));
  $("detail").classList.remove("hidden");
  $("detail-close").focus();
  if (!e.github) return;
  let d = null;
  try {
    d = await invoke("addon_details", { id });
  } catch (_) {}
  if (seq !== detailSeq) return;
  const parts = [];
  if (d?.image) parts.push(`<img class="shot" src="${d.image}" alt="Screenshot from the addon's README">`);
  if (d?.summary) parts.push(`<div class="readme">${esc(d.summary)}</div>`);
  if (!parts.length) parts.push('<div class="muted">No README to show. Its page has the details.</div>');
  $("detail-body").innerHTML = parts.join("");
}
function closeDetail() {
  $("detail").classList.add("hidden");
  detailId = null;
}
$("detail-close").onclick = closeDetail;
$("detail").addEventListener("click", (e) => { if (e.target === $("detail")) closeDetail(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && detailId) closeDetail(); });
$("detail-open").onclick = () => { const e = catalog.find((x) => x.id === detailId); if (e) openUrl(entryUrl(e)); };
$("detail-install").onclick = async () => {
  const e = catalog.find((x) => x.id === detailId);
  if (!e || busy) return;
  closeDetail();
  await installCatalog(e, false);
  renderBundles();
};

// ---------------------------------------------------------------- starter packs
async function loadBundles() {
  try {
    bundles = (await invoke("bundles")) || [];
  } catch (_) {
    bundles = [];
  }
  renderBundles();
}

function renderBundles() {
  const box = $("bundle-list");
  if (!bundles.length) {
    box.innerHTML = "";
    return;
  }
  box.innerHTML = bundles
    .map((b) => {
      const todo = b.addons.filter((a) => a.installable && !a.installed);
      const chips = b.addons
        .map((a) => {
          const state = a.installed ? "have" : a.installable ? "todo" : a.needs_key ? "key" : "link";
          const tip = a.installed ? "Installed" : a.installable ? a.desc : a.needs_key ? "Needs a Wago key (Settings)" : "Link only, not installable from here";
          return `<button class="chip ${state}" data-detail="${esc(a.id)}" title="${esc(tip)}">${iconTile("c", a.id, a.name)}${esc(a.name)}${a.installed ? " \u2713" : ""}</button>`;
        })
        .join("");
      const label = todo.length ? `Install all (${todo.length})` : b.addons.some((a) => a.installed) ? "Installed" : "Nothing to install";
      return `<div class="bundle" data-bundle="${esc(b.id)}">
        <div class="bundle-head"><span class="bundle-name">${esc(b.name)}</span><span class="grow"></span>
          <button class="btn small ${todo.length ? "primary" : ""}" data-act="bundle" ${todo.length ? "" : "disabled"}>${label}</button></div>
        <div class="bundle-desc">${esc(b.desc)}</div>
        <div class="chips">${chips}</div>
        ${b.note ? `<div class="bundle-note">${esc(b.note)}</div>` : ""}
      </div>`;
    })
    .join("");
  setBusy(busy);
}

$("bundle-list").addEventListener("click", async (e) => {
  const d = e.target.closest("[data-detail]");
  if (d) return openDetail(d.dataset.detail);
  const btn = e.target.closest("button[data-act=bundle]");
  if (!btn || busy) return;
  const b = bundles.find((x) => x.id === btn.closest(".bundle").dataset.bundle);
  if (!b) return;
  const todo = b.addons.filter((a) => a.installable && !a.installed);
  setBusy(true);
  let done = 0;
  const skipped = [];
  for (const a of todo) {
    btn.innerHTML = `<span class="spinner"></span> ${done + skipped.length + 1} / ${todo.length}: ${esc(a.name)}`;
    try {
      await invoke("install_catalog", { id: a.id, allowNonForever: false });
      a.installed = true;
      const row = catalog.find((c) => c.id === a.id);
      if (row) row.installed = true;
      done++;
    } catch (err) {
      skipped.push(String(err) === "NOT_FOREVER" ? `${a.name} (no Forever build)` : `${a.name}: ${explainError(err)}`);
    }
  }
  setBusy(false);
  toast(skipped.length ? `Installed ${done} of ${todo.length}. Skipped ${skipped.join("; ")}` : `Installed ${done} addon${done === 1 ? "" : "s"} from ${b.name}`, skipped.length > 0);
  renderBundles();
  renderCatalog();
  await rescan(true);
});
$("btn-catalog-refresh").onclick = () => loadCatalog(true);
$("browse-list").addEventListener("click", async (e) => {
  const d = e.target.closest("[data-detail]");
  if (d && !e.target.closest("a")) return openDetail(d.dataset.detail);
  const btn = e.target.closest("button[data-act=install]");
  if (!btn || busy) return;
  const id = btn.closest(".item").dataset.id;
  const entry = catalog.find((x) => x.id === id);
  await installCatalog(entry, false);
});

async function installCatalog(entry, allowNonForever) {
  setBusy(true);
  btnState(entry.id, true);
  try {
    const p = await invoke("install_catalog", { id: entry.id, allowNonForever });
    toast(`Installed ${p.name} ${p.installed_version || ""}`);
    entry.installed = true;
    for (const b of bundles) for (const a of b.addons) if (a.id === entry.id) a.installed = true;
    renderBundles();
    renderCatalog();
    await rescan(true);
  } catch (err) {
    if (String(err) === "NOT_FOREVER") {
      setBusy(false);
      btnState(entry.id, false);
      const ok = await confirmModal(`<p><b>${esc(entry.name)}</b>'s latest release does not list a Forever build.</p><p class="muted">Install it anyway?</p>`, "Install anyway");
      if (ok) return installCatalog(entry, true);
      return;
    }
    toast(explainError(err), true);
  } finally {
    setBusy(false);
    btnState(entry.id, false);
  }
}
function btnState(id, working) {
  const b = document.querySelector(`.item[data-id="${CSS.escape(id)}"] button[data-act=install]`);
  if (b) b.innerHTML = working ? '<span class="spinner"></span> Installing' : b.textContent.trim().replace(/^Installing$/, "Install");
}

async function installGithub(allowNonForever = false) {
  const repo = $("gh-repo").value.trim();
  if (!repo) return toast("Paste a GitHub repo first", true);
  setBusy(true);
  const btn = $("btn-gh-install");
  btn.innerHTML = '<span class="spinner"></span> Installing';
  try {
    const p = await invoke("install_github", { repo, allowNonForever });
    toast(`Installed ${p.name} ${p.installed_version || ""}`);
    $("gh-repo").value = "";
    await rescan(true);
    loadCatalog(false);
  } catch (err) {
    if (String(err) === "NOT_FOREVER") {
      setBusy(false);
      btn.textContent = "Install";
      const ok = await confirmModal(`<p>The latest release of <b>${esc(repo)}</b> does not list a Forever build.</p><p class="muted">Install it anyway?</p>`, "Install anyway");
      if (ok) return installGithub(true);
      return;
    }
    toast(explainError(err), true);
  } finally {
    setBusy(false);
    btn.textContent = "Install";
  }
}
$("btn-gh-install").onclick = () => installGithub(false);
$("gh-repo").addEventListener("keydown", (e) => {
  if (e.key === "Enter") installGithub(false);
});

// ---------------------------------------------------------------- settings
async function loadSettings() {
  try {
    settings = await invoke("get_settings");
  } catch (err) {
    toast(String(err), true);
    return;
  }
  $("version").textContent = "v" + settings.version + (settings.build ? " build " + settings.build : " (dev)");
  $("state-file").textContent = settings.state_file;
  $("install-path").value = settings.install_path || "";
  $("wago-link").dataset.url = settings.wago_key_url;
  $("wago-status").textContent = settings.has_wago_key ? "A key is saved." : "No key saved. Wago-only addons will show as \"Needs Wago key\".";
  $("pref-prerelease").checked = !!settings.allow_prerelease;
  $("catalog-info").textContent = settings.catalog_count
    ? `${settings.catalog_count} addons · ${settings.catalog_source === "remote" ? "latest from GitHub" : "built-in copy (GitHub copy unreachable)"}${settings.catalog_updated ? " · " + settings.catalog_updated : ""}`
    : "not loaded yet";
  $("install-line").textContent = settings.install
    ? `${settings.install.label} · ${settings.install.path}`
    : "No WoW install selected. Open Settings to pick one.";
}

async function detect() {
  $("detected").innerHTML = `<span class="muted"><span class="spinner"></span> Scanning drives…</span>`;
  try {
    const found = await invoke("find_installs");
    if (!found.length) {
      $("detected").innerHTML = `<span class="muted">Nothing found in the usual places. Use Browse… to pick the <code>_classic_beta_</code> folder yourself.</span>`;
      return;
    }
    $("detected").innerHTML = found
      .map((i) => `<button class="btn ${i.flavor}" data-path="${esc(i.path)}" ${i.flavor !== "forever" ? 'title="Not supported yet: only WoW: Forever for now"' : ""}>${esc(i.label)} <code>${esc(i.path)}</code></button>`)
      .join("");
    await loadSettings();
    if (settings.install_path) rescan();
  } catch (err) {
    $("detected").innerHTML = `<span class="muted">${esc(String(err))}</span>`;
  }
}
$("detected").addEventListener("click", async (e) => {
  const b = e.target.closest("button[data-path]");
  if (!b) return;
  $("install-path").value = b.dataset.path;
  await savePath();
});

async function savePath() {
  const path = $("install-path").value.trim();
  if (!path) return toast("Enter a folder first", true);
  try {
    const info = await invoke("set_install_path", { path });
    toast(`Managing ${info.label}`);
    await loadSettings();
    await rescan();
  } catch (err) {
    toast(String(err), true);
  }
}

$("btn-detect").onclick = detect;
$("btn-save-path").onclick = savePath;
$("btn-pick").onclick = async () => {
  try {
    const dir = await T.dialog.open({ directory: true, multiple: false, title: "Pick the WoW product folder (e.g. _classic_beta_)" });
    if (dir) {
      $("install-path").value = dir;
      await savePath();
    }
  } catch (err) {
    toast(String(err), true);
  }
};
$("btn-open-folder").onclick = () => invoke("open_addons_folder").catch((e) => toast(String(e), true));
$("btn-open-log").onclick = () => invoke("open_log").catch((e) => toast(String(e), true));
$("btn-save-key").onclick = async () => {
  try {
    await invoke("set_wago_key", { key: $("wago-key").value });
    $("wago-key").value = "";
    toast("Wago key saved");
    await loadSettings();
    await rescan();
  } catch (err) {
    toast(String(err), true);
  }
};
$("btn-clear-key").onclick = async () => {
  await invoke("set_wago_key", { key: "" });
  toast("Wago key cleared");
  await loadSettings();
  await rescan();
};
$("pref-prerelease").addEventListener("change", async (e) => {
  try {
    await invoke("set_prefs", { allowPrerelease: e.target.checked });
    toast(e.target.checked ? "Pre-releases included. Check for updates to see them." : "Pre-releases excluded");
  } catch (err) {
    toast(String(err), true);
  }
});

// ---------------------------------------------------------------- export / import
$("btn-export").onclick = async () => {
  try {
    const path = await T.dialog.save({ title: "Export addon list", defaultPath: "addonforge-list.json", filters: [{ name: "AddonForge list", extensions: ["json"] }] });
    if (!path) return;
    const n = await invoke("export_list", { path });
    toast(`Exported ${n} addon${n === 1 ? "" : "s"}`);
  } catch (err) {
    toast(String(err), true);
  }
};

$("btn-import").onclick = async () => {
  try {
    const path = await T.dialog.open({ title: "Import addon list", multiple: false, filters: [{ name: "AddonForge list", extensions: ["json"] }] });
    if (!path) return;
    const plan = await invoke("import_list", { path });
    renderImportPlan(plan);
  } catch (err) {
    toast(String(err), true);
  }
};

function renderImportPlan(plan) {
  const box = $("import-plan");
  const todo = plan.filter((r) => r.installable && !r.installed);
  box.classList.remove("hidden");
  box.innerHTML =
    `<div class="muted">${plan.length} in the list · ${plan.filter((r) => r.installed).length} already installed · ${todo.length} to install · ${plan.filter((r) => !r.installable && !r.installed).length} can't be installed automatically</div>` +
    `<ul class="plan">` +
    plan
      .map((r) => `<li class="${r.installed ? "have" : r.installable ? "todo" : "skip"}">${esc(r.name)} <span class="muted">${r.installed ? "installed" : r.installable ? r.source.kind + " · " + r.source.id : r.why}</span></li>`)
      .join("") +
    `</ul>` +
    (todo.length ? `<button id="btn-import-go" class="btn primary">Install ${todo.length} missing</button>` : "");
  const go = $("btn-import-go");
  if (go)
    go.onclick = async () => {
      setBusy(true);
      let done = 0;
      for (const r of todo) {
        go.innerHTML = `<span class="spinner"></span> ${done + 1} / ${todo.length}: ${esc(r.name)}`;
        try {
          await invoke("install_source", { source: r.source, key: r.key, allowNonForever: true });
          done++;
        } catch (err) {
          toast(`${r.name}: ${explainError(err)}`, true);
        }
      }
      setBusy(false);
      toast(`Imported ${done} of ${todo.length}`);
      box.classList.add("hidden");
      await rescan();
      document.querySelector('.tab[data-tab="installed"]').click();
    };
}

// ---------------------------------------------------------------- report a problem
let diagText = "";
async function diag() {
  diagText = await invoke("diagnostics");
  $("diag").textContent = diagText;
  $("diag").classList.remove("hidden");
  return diagText;
}
$("btn-copy-diag").onclick = async () => {
  try {
    await diag();
    toast((await copyText(diagText)) ? "Diagnostics copied" : "Couldn't copy, select the text below instead");
  } catch (err) {
    toast(String(err), true);
  }
};
$("btn-report-gh").onclick = async () => {
  try {
    const d = await diag();
    const body = "**What happened?**\n\n(describe it here)\n\n**Diagnostics**\n```\n" + d.slice(0, 5000) + "\n```";
    const url = `${settings.repo_url}/issues/new?title=${encodeURIComponent("Problem: ")}&body=${encodeURIComponent(body)}`;
    if (url.length > 7500) {
      await copyText(d);
      toast("Diagnostics copied, paste them into the issue");
      openUrl(`${settings.repo_url}/issues/new?title=${encodeURIComponent("Problem: ")}`);
    } else openUrl(url);
  } catch (err) {
    toast(String(err), true);
  }
};
$("btn-report-mail").onclick = async () => {
  try {
    const d = await diag();
    await copyText(d);
    const body = "What happened?\n\n(describe it here)\n\nDiagnostics (also copied to your clipboard, paste if missing):\n\n" + d.slice(0, 1500);
    openUrl(`mailto:${settings.support_email}?subject=${encodeURIComponent("AddonForge problem")}&body=${encodeURIComponent(body)}`);
  } catch (err) {
    toast(String(err), true);
  }
};

// ---------------------------------------------------------------- self-update
async function checkSelf(manual = false) {
  try {
    selfUpdate = await invoke("check_self_update");
  } catch (err) {
    if (manual) toast(String(err), true);
    return;
  }
  if (selfUpdate.available && selfUpdate.latest) {
    const l = selfUpdate.latest;
    $("selfupdate-text").innerHTML = `A new version of AddonForge is available (<b>v${esc(l.version)}</b> build ${esc(l.build)}). Please update to the latest.`;
    $("selfupdate").classList.remove("hidden");
  } else {
    $("selfupdate").classList.add("hidden");
    if (manual) toast(selfUpdate.error ? `Couldn't check: ${selfUpdate.error}` : "AddonForge is up to date");
  }
}
$("btn-selfcheck").onclick = () => checkSelf(true);
$("selfupdate-skip").onclick = async () => {
  await invoke("skip_self_update", { tag: selfUpdate.tag }).catch(() => {});
  $("selfupdate").classList.add("hidden");
};
$("selfupdate-go").onclick = async () => {
  const go = $("selfupdate-go");
  go.innerHTML = '<span class="spinner"></span> Downloading';
  go.disabled = true;
  try {
    await invoke("apply_self_update");
    $("selfupdate-text").textContent = "Downloaded. Restarting into the new version…";
  } catch (err) {
    toast(String(err), true);
    go.textContent = "Update now";
    go.disabled = false;
  }
};

// ---------------------------------------------------------------- boot
(async () => {
  await loadSettings();
  if (!settings || !settings.install_path) {
    await detect();
    await loadSettings();
  }
  if (settings && settings.install_path) {
    await rescan();
    checkUpdates();
  } else {
    document.querySelector('.tab[data-tab="settings"]').click();
  }
  loadCatalog(false);
  checkSelf(false);
  if (settings?.start_tab?.startsWith("detail:")) { await loadCatalog(false); document.querySelector(`.tab[data-tab="browse"]`)?.click(); openDetail(settings.start_tab.slice(7)); }
  else if (settings?.start_tab === "guide") { document.querySelector(`.tab[data-tab="start"]`)?.click(); $("guide").open = true; }
  else if (settings?.start_tab) document.querySelector(`.tab[data-tab="${settings.start_tab}"]`)?.click();
  // One-time note on the first launch after an update.
  invoke("whats_new")
    .then((w) => {
      if (!w) return;
      $("modal-no").classList.add("hidden");
      confirmModal(`<p><b>AddonForge v${esc(w.version)} build ${esc(w.build)}</b> is installed. What changed:</p><div class="whatsnew">${esc(w.notes)}</div>`, "OK")
        .finally(() => $("modal-no").classList.remove("hidden"));
    })
    .catch(() => {});
})();
