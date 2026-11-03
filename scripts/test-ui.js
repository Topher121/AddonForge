// UI behavior checks without starting Tauri or changing the user's addons.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const nodes = new Map();
function element(id) {
  if (!nodes.has(id)) nodes.set(id, {
    value: '', checked: false, textContent: '', innerHTML: '', dataset: {},
    classList: { add() {}, remove() {}, toggle() {} },
    listeners: {}, addEventListener(type, fn) { this.listeners[type] = fn; }, setAttribute() {}, removeAttribute() {},
    focus() { document.activeElement = this; },
  });
  return nodes.get(id);
}
const document = {
  getElementById: element, querySelectorAll: () => [], querySelector: () => element('main'),
  addEventListener() {}, removeEventListener() {}, activeElement: element('previous'),
};
let detailReply;
let invokeReply = () => Promise.resolve();
const invokeCalls = [];
const context = vm.createContext({
  document, window: { __TAURI__: { core: { invoke(command, args) {
    invokeCalls.push({ command, args });
    if (command === "addon_details") return detailReply;
    return invokeReply(command, args);
  } }, opener: {} } },
  setTimeout, clearTimeout, console,
});
const source = fs.readFileSync('ui/app.js', 'utf8').split('// ---------------------------------------------------------------- boot')[0];
vm.runInContext(source, context);
const run = (code) => vm.runInContext(code, context);
run(`packages = [
  {key:'a',name:'Alpha <addon>',folders:['Alpha'],author:'Ada',source_label:'GitHub',source:true,status:'update',supports_forever:true,missing_deps:[],installed_version:'1',remote_version:'2'},
  {key:'b',name:'Beta',folders:['Beta'],source_label:'Wago',status:'no-key',supports_forever:true,missing_deps:[]},
  {key:'c',name:'Gamma',folders:['Gamma'],source_label:'GitHub',status:'ok',supports_forever:true,missing_deps:[],ignored:true},
  {key:'d',name:'Delta',folders:['Delta'],source_label:'GitHub',source:true,status:'update',supports_forever:true,missing_deps:[],pinned:true}
]; renderInstalled();`);
assert.equal(String(element('stat-total').textContent), '4');
assert.equal(String(element('stat-updates').textContent), '2');
assert.equal(String(element('stat-attention').textContent), '1');
assert.equal(element('btn-update-all').textContent, 'Update all (1)');
assert.match(element('installed-list').innerHTML, /Alpha &lt;addon&gt;/);
run(`packages[0].notice = 'Beta workaround <b>'; renderInstalled();`);
assert.match(element('installed-list').innerHTML, /note warn">Beta workaround &lt;b&gt;/);
run(`delete packages[0].notice; renderInstalled();`);
assert.doesNotMatch(element('installed-list').innerHTML, /Gamma/);
assert.equal(run(`activeFilter = 'updates'; visiblePackages().length`), 2);
assert.equal(run(`activeFilter = 'attention'; visiblePackages()[0].key`), 'b');
element('installed-filter').value = 'Ada';
assert.equal(run(`activeFilter = 'all'; visiblePackages().length`), 1);
element('installed-filter').value = 'unmatched';
run('renderInstalled()');
assert.match(element('installed-list').innerHTML, /Show all addons/);
element('installed-filter').value = '';
element('show-ignored').checked = true;
assert.equal(run('visiblePackages().length'), 4);
// CurseForge rows: installable, opted-out (link only), already installed; status line and paging button.
run(`settings = { has_curse: true };
curse.rows = [
  {id:1,name:'TomTom <x>',summary:'Nav',authors:'A',category:'Map & Minimap',downloads:1500,url:'https://www.curseforge.com/projects/1',installable:true,installed:false,catalog_id:null},
  {id:2,name:'Leatrix',summary:'QoL',authors:'B',category:'',downloads:70000000,url:'u',installable:false,installed:false,catalog_id:null},
  {id:3,name:'Questie',summary:'Q',authors:'C',category:'Quests',downloads:5,url:'u',installable:true,installed:true,catalog_id:null}
]; curse.query = 'tom'; curse.total = 120; curse.loaded = 50; renderCurseRows();`);
assert.match(element('curse-list').innerHTML, /TomTom &lt;x&gt;/);
assert.match(element('curse-list').innerHTML, /data-act="curse-install"/);
assert.match(element('curse-list').innerHTML, /link only/);
assert.match(element('curse-list').innerHTML, /installed/);
assert.match(element('curse-list').innerHTML, /Map &amp; Minimap/);
assert.equal(String(element('curse-status').textContent), '3 of 120 from CurseForge');
run(`curse.rows = []; curse.total = 0; curse.loaded = 0; curse.query = 'zzz'; renderCurseRows();`);
assert.equal(String(element('curse-status').textContent), 'Nothing on CurseForge for "zzz".');
assert.equal(element('curse-list').innerHTML, '');
// Type filter maps catalogue categories; unknown types match nothing special.
assert.equal(run("catalogMatchesType({category:'Bags'}, typeFor('bags'))"), true);
assert.equal(run("catalogMatchesType({category:'UI'}, typeFor('bags'))"), false);
assert.equal(run("catalogMatchesType({category:'UI'}, typeFor(''))"), true);
assert.equal(run("typeFor('pvp').curse"), 1004);
run(`settings = null; curse.rows = []; curse.query = '';`);
// Source picker: one source is a link, several become a select; Wago ids and locked CurseForge ids never count.
run(`settings = { has_curse: true };
catalog = [
  { id:'one', name:'One', github:'x/one' },
  { id:'two', name:'Two', github:'x/two', curse: 7, wago: 'w' },
  { id:'locked', name:'Locked', curse: 8, curse_locked: true },
  { id:'wagoonly', name:'WagoOnly', wago: 'w2' },
];
document.getElementById('browse-filter').value = ''; document.getElementById('browse-cat').value = ''; renderCatalog();`);
assert.equal(run("installSources(catalog[0]).length"), 1);
assert.equal(run("installSources(catalog[1]).map(s => s.kind).join()"), 'github,curse', 'a Wago id never makes a source');
assert.equal(run("installSources(catalog[2]).length"), 0);
assert.equal(run("installSources(catalog[3]).length"), 0, 'Wago-only is link only');
assert.match(element('browse-list').innerHTML, /<select class="src-pick"[^>]*><option value="github">GitHub/);
assert.match(element('browse-list').innerHTML, /link only/);
assert.doesNotMatch(element('browse-list').innerHTML, /Wago key/);
run(`settings = null; catalog = [];`);
run(`packages = []; renderInstalled()`);
assert.equal(String(element('stat-updates').textContent), '0');
assert.match(element('installed-list').innerHTML, /Discover addons/);
const menu = { style: {}, offsetHeight: 110 };
const details = {
  open: true, matches: () => true, querySelector: () => menu,
  getBoundingClientRect: () => ({ top: 580, bottom: 612, right: 920 }),
};
context.window.innerWidth = 980;
context.window.innerHeight = 660;
element('installed-list').listeners.toggle({ target: details });
assert.ok(parseFloat(menu.style.top) >= 0);
assert.ok(parseFloat(menu.style.top) + menu.offsetHeight < 580, 'Bottom-row options open above the trigger');
assert.ok(parseFloat(menu.style.right) >= 12, 'Options stay inside the right edge');
(async () => {
  const originalFocus = document.activeElement;
  const promise = run(`confirmModal('Confirm?', 'Remove')`);
  assert.equal(document.activeElement, element('modal-no'));
  element('modal-no').onclick();
  assert.equal(await promise, false);
  assert.equal(document.activeElement, originalFocus);
  run(`catalog = [{ id:'boss', name:'Boss', github:'example/boss', desc:'Boss timers', installed:true }];`);
  detailReply = Promise.resolve({summary:'Release description', image:'data:image/png;base64,test'});
  await run(`openDetail('boss', {key:'boss-local',name:'Boss',folders:['Boss'],installed_version:'2'})`);
  assert.equal(element('detail-desc').textContent, 'Boss timers');
  assert.match(element('detail-meta').innerHTML, /Installed: 2/);
  assert.match(element('detail-body').innerHTML, /Release description/);
  assert.match(element('detail-body').innerHTML, /class="shot"/);
  run('closeDetail()');
  assert.equal(document.activeElement, originalFocus);
  await run(`openDetail(null, {key:'local',name:'Local addon',folders:['Local<folder>'],notes:'Local description',source_url:'https://example.test/addon'})`);
  assert.equal(element('detail-desc').textContent, 'Local description');
  assert.match(element('detail-body').innerHTML, /Local&lt;folder&gt;/);
  assert.equal(run('entryUrl(detailEntry)'), 'https://example.test/addon');
  run('closeDetail()');
  let finish;
  detailReply = new Promise(resolve => { finish = resolve; });
  const loading = run("openDetail('boss')");
  run('closeDetail()');
  const before = element('detail-body').innerHTML;
  finish({summary:'Late response'});
  await loading;
  assert.equal(element('detail-body').innerHTML, before, 'Closed dialogs ignore late responses');

  // Exercise the actual settings handlers, including pending and failed calls.
  run('settings = { auto_update: false, allow_prerelease: true };');
  element('pref-autoupdate').checked = true;
  let finishPreference;
  invokeReply = () => new Promise(resolve => { finishPreference = resolve; });
  const savingPreference = element('pref-autoupdate').onchange();
  assert.equal(element('pref-autoupdate').disabled, true, 'Preference waits for persistence');
  assert.equal(invokeCalls.at(-1).command, 'set_prefs');
  assert.equal(invokeCalls.at(-1).args.autoUpdate, true);
  assert.deepEqual(Object.keys(invokeCalls.at(-1).args), ['autoUpdate'], 'Only the changed preference is sent');
  finishPreference();
  await savingPreference;
  assert.equal(run('settings.auto_update'), true);
  assert.equal(element('pref-autoupdate').disabled, false);

  element('pref-autoupdate').checked = false;
  invokeReply = () => Promise.reject(new Error('Preference write failed'));
  await element('pref-autoupdate').onchange();
  assert.equal(element('pref-autoupdate').checked, true, 'Failed save restores the saved checkbox value');
  assert.equal(run('settings.auto_update'), true, 'Failed save does not change the saved preference');
  assert.equal(element('pref-autoupdate').disabled, false);
  assert.match(element('toast').textContent, /Preference write failed/);
  assert.match(element('toast').className, /bad/);

  element('pref-prerelease').checked = false;
  invokeReply = () => Promise.resolve();
  await element('pref-prerelease').onchange();
  assert.equal(invokeCalls.at(-1).command, 'set_prefs');
  assert.equal(invokeCalls.at(-1).args.allowPrerelease, false);
  assert.deepEqual(Object.keys(invokeCalls.at(-1).args), ['allowPrerelease']);
  assert.equal(run('settings.allow_prerelease'), false);
  invokeReply = () => Promise.resolve({ version: '0.4.0', build: 28, auto_update: true, allow_prerelease: false });
  await run('loadSettings()');
  assert.equal(element('pref-autoupdate').checked, true, 'Saved auto-update preference survives settings reload');
  assert.equal(element('pref-prerelease').checked, false);

  element('btn-backup-wtf').textContent = 'Back up the WTF folder';
  let finishBackup;
  invokeReply = () => new Promise(resolve => { finishBackup = resolve; });
  const startingBackup = element('btn-backup-wtf').onclick();
  assert.equal(invokeCalls.at(-1).command, 'backup_wtf');
  assert.equal(element('btn-backup-wtf').disabled, true);
  assert.equal(element('btn-backup-wtf').textContent, 'Backing up…');
  const callsWhileBackingUp = invokeCalls.length;
  await element('btn-backup-wtf').onclick();
  assert.equal(invokeCalls.length, callsWhileBackingUp, 'A second click cannot start a second backup');
  finishBackup({ files: 2, bytes: 100, path: 'QA/WTF-synthetic.zip' });
  await startingBackup;
  assert.match(element('toast').textContent, /Backed up 2 files to QA\/WTF-synthetic\.zip/);
  assert.equal(element('btn-backup-wtf').disabled, false);
  assert.equal(element('btn-backup-wtf').textContent, 'Back up the WTF folder');

  invokeReply = () => Promise.reject(new Error('No WTF folder in the game folder yet'));
  await element('btn-backup-wtf').onclick();
  assert.match(element('toast').textContent, /No WTF folder/);
  assert.match(element('toast').className, /bad/);
  assert.equal(element('btn-backup-wtf').disabled, false);
  assert.equal(run('busy'), false, 'A failed backup releases the interface');
  assert.equal(element('btn-backup-wtf').textContent, 'Back up the WTF folder');

  invokeReply = () => Promise.resolve('QA/backups');
  await element('btn-open-backups').onclick();
  assert.equal(invokeCalls.at(-1).command, 'open_backups_folder');
  invokeReply = () => Promise.reject(new Error('Could not open backups folder'));
  await element('btn-open-backups').onclick();
  assert.match(element('toast').textContent, /Could not open backups folder/);
  assert.match(element('toast').className, /bad/);
  run('clearTimeout(toastTimer)');
  console.log('UI checks passed: counts, eligible updates, filters, search, escaping, empty states, modal focus and cancellation, saved preferences, backups and folder actions.');
})().catch((error) => { console.error(error); process.exitCode = 1; });
