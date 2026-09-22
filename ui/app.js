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

// ---------------------------------------------------------------- helpers
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
    $("modal-text").innerHTML = html;
    $("modal-yes").textContent = yes;
    $("modal").classList.remove("hidden");
    const done = (v) => {
      $("modal").classList.add("hidden");
      $("modal-yes").onclick = $("modal-no").onclick = null;
      resolve(v);
    };
    $("modal-yes").onclick = () => done(true);
    $("modal-no").onclick = () => done(false);
  });
}

function setBusy(b) {
  busy = b;
  document.querySelectorAll(".pill").forEach((el) => {
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
  const a = e.target.closest("[data-url]");
  if (a && a.dataset.url) {
    e.preventDefault();
    openUrl(a.dataset.url);
  }
});

// ---------------------------------------------------------------- tabs
document.querySelectorAll(".tab").forEach((btn) =>
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b === btn));
    document.querySelectorAll(".panel").forEach((p) => p.classList.toggle("active", p.id === "tab-" + btn.dataset.tab));
    if (btn.dataset.tab === "browse" && !catalog.length) loadCatalog(false);
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
  return packages.filter((p) => (showIgnored || !p.ignored) && (!q || `${p.name} ${p.folders.join(" ")} ${p.author || ""} ${p.source_label}`.toLowerCase().includes(q)));
}

function renderInstalled() {
  const list = $("installed-list");
  const rows = visiblePackages();
  if (!packages.length) {
    list.innerHTML = `<div class="empty">No addons found in this install's Interface\\AddOns folder yet.<br>Head to <b>Browse</b> to install some.</div>`;
    $("summary").textContent = "";
    setBusy(busy);
    return;
  }
  list.innerHTML = rows.length
    ? rows
        .map((p) => {
          const ver = p.installed_version ? `<b>${esc(p.installed_version)}</b>` : "<i>version unknown</i>";
          const remote = p.remote_version && p.status === "update" ? ` → <b>${esc(p.remote_version)}</b>${p.remote_prerelease ? ' <span class="badge pre">pre-release</span>' : ""}` : "";
          const fv = p.supports_forever
            ? `<span class="badge forever">Forever</span>`
            : `<span class="badge not-forever" title="No 16001 interface or _Forever toc found">Not for Forever</span>`;
          const src = p.source_url
            ? `<a href="#" data-url="${esc(p.source_url)}">${esc(p.source_label || (p.curse_id ? "CurseForge" : "link"))}</a>`
            : `<span>${esc(p.source_label)}</span>`;
          const canUpdate = p.source && p.status !== "no-key";
          const btnLabel = p.status === "update" ? "Update" : p.managed ? "Reinstall" : "Install via source";
          const noteCls = p.status === "error" ? "note err" : "note";
          const deps = p.missing_deps.length
            ? `<div class="note warn">Needs: ${p.missing_deps
                .map((d) => (d.catalog_id ? `<b>${esc(d.name || d.folder)}</b> <button class="mini" data-act="dep" data-id="${esc(d.catalog_id)}">install</button>` : `<b>${esc(d.folder)}</b> <i>(not in catalogue)</i>`))
                .join(", ")}</div>`
            : "";
          return `<div class="item ${p.ignored ? "ignored" : ""} ${p.status === "no-source" || p.status === "curse-only" ? "dim" : ""}" data-key="${esc(p.key)}">
        <div>
          <div class="title">${esc(p.name)} ${badge(p)} ${fv} ${p.pinned ? '<span class="badge pinned" title="Updates held back">pinned</span>' : ""} ${p.ignored ? '<span class="badge managed">ignored</span>' : ""} ${p.managed ? '<span class="badge managed" title="Installed by AddonForge">managed</span>' : ""}</div>
          <div class="meta">${ver}${remote} · ${src}${p.author ? " · " + esc(p.author) : ""} · <span title="${esc(p.folders.join(", "))}">${p.folders.length} folder${p.folders.length === 1 ? "" : "s"}</span></div>
        </div>
        <div class="actions">
          ${canUpdate ? `<button class="pill small ${p.status === "update" && !p.pinned ? "" : "ghost"}" data-act="update">${btnLabel}</button>` : ""}
          <button class="pill small ghost" data-act="pin" title="${p.pinned ? "Allow updates again" : "Hold this addon at its current version"}">${p.pinned ? "Unpin" : "Pin"}</button>
          <button class="pill small ghost" data-act="ignore" title="${p.ignored ? "Show it again" : "Hide from this list and skip checks"}">${p.ignored ? "Unignore" : "Ignore"}</button>
          <button class="pill small ghost" data-act="remove" title="Delete these folders from AddOns (saved settings in WTF are kept)">Remove</button>
        </div>
        ${p.note ? `<div class="${noteCls}">${esc(p.note)}</div>` : ""}
        ${deps}
      </div>`;
        })
        .join("")
    : `<div class="empty">Nothing matches.</div>`;
  const updates = packages.filter((p) => p.status === "update" && !p.ignored).length;
  const errors = packages.filter((p) => p.status === "error" && !p.ignored).length;
  const ignored = packages.filter((p) => p.ignored).length;
  $("summary").textContent =
    `${packages.length} addon${packages.length === 1 ? "" : "s"}` +
    (updates ? ` · ${updates} update${updates === 1 ? "" : "s"}` : "") +
    (errors ? ` · ${errors} error${errors === 1 ? "" : "s"}` : "") +
    (ignored ? ` · ${ignored} ignored` : "");
  setBusy(busy);
}

$("installed-filter").addEventListener("input", renderInstalled);
$("show-ignored").addEventListener("change", renderInstalled);

$("installed-list").addEventListener("click", async (e) => {
  const btn = e.target.closest("button[data-act]");
  if (!btn || busy) return;
  const key = btn.closest(".item").dataset.key;
  const p = packages.find((x) => x.key === key);
  if (!p) return;
  const act = btn.dataset.act;
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
  } catch (err) {
    packages = [];
    renderInstalled();
    $("installed-list").innerHTML = `<div class="empty">${esc(String(err))}<br><br>Pick your WoW: Forever folder in <b>Settings</b>.</div>`;
  } finally {
    setBusy(false);
  }
}

async function checkUpdates() {
  setBusy(true);
  packages.forEach((p) => {
    if (p.source && p.status !== "no-key" && !p.ignored) p.status = "checking";
  });
  renderInstalled();
  try {
    packages = await invoke("check_updates");
    renderInstalled();
    const n = packages.filter((p) => p.status === "update" && !p.ignored && !p.pinned).length;
    toast(n ? `${n} update${n === 1 ? "" : "s"} available` : "Everything is up to date");
  } catch (err) {
    toast(String(err), true);
  } finally {
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
function renderCatalog() {
  const q = $("browse-filter").value.trim().toLowerCase();
  const rows = catalog.filter((e) => !q || `${e.name} ${e.desc} ${e.category} ${e.github || ""}`.toLowerCase().includes(q));
  $("browse-list").innerHTML = rows.length
    ? rows
        .map((e) => {
          const src = e.github ? `GitHub · ${e.github}` : e.wago ? "Wago" : e.wowi ? "WoWInterface" : e.tukui ? "TukUI" : "CurseForge only";
          const fv = e.forever === true ? `<span class="badge forever">Forever</span>` : e.forever === false ? `<span class="badge not-forever">No Forever build</span>` : "";
          const canInstall = e.github || e.wowi || e.tukui || (e.wago && settings?.has_wago_key);
          const needsKey = e.wago && !e.github && !e.wowi && !e.tukui && !settings?.has_wago_key;
          return `<div class="item" data-id="${esc(e.id)}">
            <div>
              <div class="title">${esc(e.name)} ${fv} ${e.installed ? '<span class="badge ok">Installed</span>' : ""}</div>
              <div class="meta">${esc(e.desc || "")}${e.category ? ` · ${esc(e.category)}` : ""} · <a href="#" data-url="${esc(e.url || (e.github ? "https://github.com/" + e.github : "#"))}">${esc(src)}</a></div>
            </div>
            <div class="actions">
              ${canInstall ? `<button class="pill small ${e.installed ? "ghost" : ""}" data-act="install">${e.installed ? "Reinstall" : "Install"}</button>` : needsKey ? `<span class="badge no-key">Needs Wago key</span>` : `<span class="badge curse-only">Link only</span>`}
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
  } catch (err) {
    toast(String(err), true);
  } finally {
    setBusy(false);
  }
}

$("browse-filter").addEventListener("input", renderCatalog);
$("btn-catalog-refresh").onclick = () => loadCatalog(true);
$("browse-list").addEventListener("click", async (e) => {
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
      .map((i) => `<button class="pill ghost ${i.flavor}" data-path="${esc(i.path)}" ${i.flavor !== "forever" ? 'title="Not supported yet: only WoW: Forever for now"' : ""}>${esc(i.label)} <code>${esc(i.path)}</code></button>`)
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
    (todo.length ? `<button id="btn-import-go" class="pill">Install ${todo.length} missing</button>` : "");
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
    $("selfupdate-text").innerHTML = `AddonForge <b>v${esc(l.version)}</b> build ${esc(l.build)} is out${l.notes ? ": " + esc(l.notes) : ""}`;
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
})();
