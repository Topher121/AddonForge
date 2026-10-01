// Security gate. Fails (exit 1) if the shipped configuration weakens any protection. Run by `npm run check`,
// by scripts/selfcheck.js, and by desktop/build.ps1 before every build, so a weakened build cannot be produced.
//
// Studio rule (Chris 2026-10-01): nothing we build may turn off a security protection. See ~/.claude/CLAUDE.md.
import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const problems = [];

// ---- Tauri window / webview ----
const confPath = resolve(ROOT, 'src-tauri/tauri.conf.json');
const conf = JSON.parse(readFileSync(confPath, 'utf8'));
const FORBIDDEN_FLAGS = [
  /msSmartScreenProtection/i,
  /disable-client-side-phishing-detection/i,
  /disable-component-update/i,
  /disable-web-security/i,
  /allow-running-insecure-content/i,
  /ignore-certificate-errors/i,
  /no-sandbox/i,
  /disable-site-isolation/i,
  /allow-file-access-from-files/i,
];
for (const w of conf.app?.windows || []) {
  const args = w.additionalBrowserArgs || '';
  for (const re of FORBIDDEN_FLAGS) if (re.test(args)) problems.push(`tauri.conf.json window "${w.label}": additionalBrowserArgs contains ${re.source.replace(/\\/g, '')}`);
  if (/--disable-features=/.test(args)) problems.push(`tauri.conf.json window "${w.label}": additionalBrowserArgs disables browser features (${args.match(/--disable-features=\S+/)[0]})`);
  if (w.devtools === true) problems.push(`tauri.conf.json window "${w.label}": devtools forced on`);
}

// ---- Content Security Policy ----
const csp = conf.app?.security?.csp;
const cspText = csp == null ? '' : typeof csp === 'string' ? csp : Object.entries(csp).map(([k, v]) => `${k} ${v}`).join('; ');
if (!cspText) problems.push('tauri.conf.json: security.csp is missing or null (every shipped app needs a Content Security Policy)');
else {
  if (!/default-src\s+'self'/.test(cspText)) problems.push("csp: default-src must be 'self'");
  if (/script-src[^;]*('unsafe-inline'|'unsafe-eval'|\*)/.test(cspText)) problems.push("csp: script-src must not allow 'unsafe-inline', 'unsafe-eval' or *");
  if (/connect-src[^;]*(\shttps?:(\s|;|$)|\*)/.test(cspText)) problems.push('csp: connect-src must list the actual servers, not every http/https destination');
  if (!/object-src\s+'none'/.test(cspText)) problems.push("csp: object-src should be 'none'");
  if (!/frame-ancestors\s+'none'/.test(cspText)) problems.push("csp: frame-ancestors should be 'none'");
}
if (conf.app?.security?.dangerousDisableAssetCspModification) problems.push('tauri.conf.json: dangerousDisableAssetCspModification is set');
if (conf.app?.security?.freezePrototype === false) problems.push('tauri.conf.json: freezePrototype disabled');

// ---- Capabilities: no shell / fs / http wildcard permissions the app does not need ----
const capDir = resolve(ROOT, 'src-tauri/capabilities');
if (existsSync(capDir)) {
  const cap = JSON.parse(readFileSync(resolve(capDir, 'default.json'), 'utf8'));
  const risky = (cap.permissions || []).map(p => (typeof p === 'string' ? p : p.identifier)).filter(p => /^(shell:|fs:|http:|process:)/.test(p) && !/deny/.test(p));
  if (risky.length) problems.push(`capabilities/default.json grants ${risky.join(', ')} — the app does not need shell, filesystem, process or raw http access`);
}

// ---- The UI must not need inline scripts or styles (CSP would block them, and allowing them weakens it) ----
for (const f of ['index.html', 'app.js']) {
  const p = resolve(ROOT, 'ui', f);
  if (!existsSync(p)) continue;
  const s = readFileSync(p, 'utf8');
  if (/\sstyle="/.test(s)) problems.push(`ui/${f}: inline style="" attribute (move it to style.css)`);
  if (f.endsWith('.html') && /<script(?![^>]*\ssrc=)/i.test(s)) problems.push(`ui/${f}: inline <script> block`);
  if (/\son[a-z]+="/i.test(s)) problems.push(`ui/${f}: inline event handler attribute`);
  if (/\beval\(|new Function\(/.test(s)) problems.push(`ui/${f}: eval / new Function`);
}

// ---- Repo must never touch antivirus or Windows security settings ----
const SEC = /Set-MpPreference|Add-MpPreference|Remove-MpPreference|MpCmdRun|-ExclusionPath|DisableRealtimeMonitoring|PUAProtection|SmartScreenEnabled|EnableLUA|netsh\s+advfirewall\s+set|Set-NetFirewallProfile|Set-ExecutionPolicy\s+Unrestricted/i;
const scan = dir => {
  for (const name of readdirSafe(dir)) {
    const p = resolve(dir, name);
    if (/node_modules|target|\.git$|data$|logs$/.test(p)) continue;
    if (isDir(p)) scan(p);
    else if (/\.(ps1|js|mjs|cjs|cmd|bat|sh|json)$/.test(name) && !/security-check.m?js$/.test(name)) {
      const s = readFileSync(p, 'utf8');
      const m = s.match(SEC);
      if (m) problems.push(`${p.replace(ROOT, '.')}: touches a Windows security setting (${m[0]})`);
    }
  }
};
import { readdirSync, statSync } from 'node:fs';
function readdirSafe(d) {
  try {
    return readdirSync(d);
  } catch {
    return [];
  }
}
function isDir(p) {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
}
scan(ROOT);

if (problems.length) {
  console.error('SECURITY CHECK FAILED:\n - ' + problems.join('\n - '));
  process.exit(1);
}
console.log(`security check OK — webview protections on, CSP set (${cspText.length} chars), no inline scripts/styles, nothing touches Windows security settings`);
