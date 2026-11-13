// UI behavior checks without starting Tauri or changing the user's addons.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const nodes = new Map();
function element(id) {
  if (!nodes.has(id)) nodes.set(id, {
    value: '', checked: false, textContent: '', innerHTML: '', dataset: {},
    classList: { add() {}, remove() {}, toggle() {} },
    listeners: {}, attributes: {}, addEventListener(type, fn) { this.listeners[type] = fn; },
    setAttribute(name, value) { this.attributes[name] = String(value); },
    removeAttribute(name) { delete this.attributes[name]; },
    focus() { document.activeElement = this; },
  });
  return nodes.get(id);
}
const makeSortHeads = (prefix, fields) => fields.map((by) => {
  const head = element(`${prefix}-${by}`);
  head.dataset.sort = by;
  head.textContent = by === 'name' ? 'Addon' : by[0].toUpperCase() + by.slice(1);
  return head;
});
const sortHeads = makeSortHeads('sort', ['name', 'category', 'downloads', 'source']);
const installedSortHeads = makeSortHeads('installed-sort', ['name', 'source', 'status']);
element('browse-sort').value = 'name';
const document = {
  getElementById: element,
  querySelectorAll: (selector) => ({ '.sort-head': sortHeads, '.installed-sort-head': installedSortHeads })[selector] || [],
  querySelector: () => element('main'),
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
  {key:'c',name:'Gamma',folders:['Gamma'],source_label:'',status:'ok',supports_forever:true,missing_deps:[],ignored:true},
  {key:'d',name:'Delta',folders:['Delta'],source_label:'CurseForge',source:true,status:'update',supports_forever:true,missing_deps:[],pinned:true}
]; renderInstalled();`);
assert.equal(String(element('stat-total').textContent), '4');
assert.equal(String(element('stat-updates').textContent), '2');
assert.equal(String(element('stat-attention').textContent), '1');
assert.equal(element('btn-update-all').textContent, 'Update all (1)');
assert.match(element('installed-list').innerHTML, /Alpha &lt;addon&gt;/);
assert.doesNotMatch(element('installed-list').innerHTML, /c-ver|Installed version|Available version/);
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
const installedKeys = () => run('visiblePackages().map(p => p.key).join()');
assert.equal(element('installed-sort-status').attributes['data-direction'], 'asc');
element('installed-sort-name').onclick();
assert.equal(installedKeys(), 'a,b,d,c');
element('installed-sort-name').onclick();
assert.equal(installedKeys(), 'c,d,b,a');
assert.equal(element('installed-sort-name').attributes['data-direction'], 'desc');
element('installed-sort-source').onclick();
assert.equal(installedKeys(), 'd,a,b,c');
element('installed-sort-source').onclick();
assert.equal(installedKeys(), 'b,a,d,c', 'Unknown source stays last in either direction');
element('installed-filter').value = 'Ada';
assert.equal(installedKeys(), 'a');
element('installed-filter').value = '';
element('installed-sort-status').onclick();
assert.equal(installedKeys(), 'a,b,d,c');
element('installed-sort-status').onclick();
assert.equal(installedKeys(), 'd,b,a,c');
element('installed-sort-status').onclick();
assert.match(element('installed-sort-status').attributes['aria-label'], /highest priority first/);
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
// Repeated heading clicks reverse direction; unknown counts stay below real zeroes.
run(`catalog = [
  {id:'z',name:'Zulu',category:'Utility',downloads:100,github:'example/z',updated_at:'2026-10-01'},
  {id:'a',name:'Alpha',category:'Bags',downloads:10,curse:1,updated_at:'2026-11-01'},
  {id:'b',name:'Beta',category:'Bags',downloads:10,github:'example/b',updated_at:'2026-11-02'},
  {id:'zero',name:'Zero',category:'UI',downloads:0,wowi:4,updated_at:''},
  {id:'unknown',name:'Unknown',category:'',downloads:null,updated_at:null}
];`);
const sortedNames = () => run('sortedCatalog([...catalog]).map(e => e.name).join()');
element('sort-downloads').onclick();
assert.equal(element('browse-sort').value, 'downloads');
assert.equal(sortedNames(), 'Zulu,Alpha,Beta,Zero,Unknown');
assert.equal(element('sort-downloads').attributes['data-direction'], 'desc');
element('sort-downloads').onclick();
assert.equal(element('browse-sort').value, 'downloads-asc');
assert.equal(sortedNames(), 'Zero,Alpha,Beta,Zulu,Unknown');
assert.equal(element('sort-downloads').attributes['data-direction'], 'asc');
assert.match(element('sort-downloads').attributes['aria-label'], /lowest to highest/);
element('sort-downloads').onclick();
assert.equal(element('browse-sort').value, 'downloads');
element('sort-name').onclick();
assert.equal(sortedNames(), 'Alpha,Beta,Unknown,Zero,Zulu');
assert.equal(element('sort-downloads').attributes['data-direction'], undefined);
element('sort-name').onclick();
assert.equal(sortedNames(), 'Zulu,Zero,Unknown,Beta,Alpha');
element('sort-category').onclick();
assert.equal(sortedNames(), 'Unknown,Alpha,Beta,Zero,Zulu');
element('sort-category').onclick();
assert.equal(sortedNames(), 'Zulu,Zero,Alpha,Beta,Unknown');
element('sort-source').onclick();
assert.equal(sortedNames(), 'Alpha,Unknown,Beta,Zulu,Zero');
element('sort-source').onclick();
assert.equal(sortedNames(), 'Zero,Beta,Zulu,Alpha,Unknown');
element('browse-sort').value = 'updated-asc';
element('browse-sort').listeners.change();
assert.equal(sortedNames(), 'Zulu,Alpha,Beta,Unknown,Zero');
assert.ok(sortHeads.every(head => head.attributes['data-direction'] === undefined));
element('browse-sort').value = 'name-desc';
element('browse-sort').listeners.change();
assert.equal(element('sort-name').attributes['data-direction'], 'desc');
element('browse-sort').value = 'downloads-asc';
const ascendingKey = run('curseParams().key');
assert.equal(run('curseParams().sort'), 'downloads-asc');
element('browse-sort').value = 'downloads';
assert.notEqual(run('curseParams().key'), ascendingKey);
// Unsupported server sorts keep the same alphabetical page in either direction.
for (const order of ['category', 'category-desc', 'source', 'source-desc']) {
  element('browse-sort').value = order;
  assert.equal(run('curseParams().sort'), 'name');
}
run(`settings = {has_curse:true}; curse.rows = [
  {id:1,name:'Zulu',category:'Bags',downloads:5},
  {id:2,name:'Alpha',category:'UI',downloads:2}
]; curse.total = 100; curse.loaded = 50; curse.loading = false;`);
element('browse-sort').value = 'source-desc';
run('renderCurseRows()');
assert.ok(element('curse-list').innerHTML.indexOf('Alpha') < element('curse-list').innerHTML.indexOf('Zulu'));
element('browse-sort').value = 'category-desc';
run('renderCurseRows()');
assert.match(element('curse-status').textContent, /Category order applies to shown results/);
run('curse.loaded = curse.total; renderCurseRows()');
assert.equal(element('curse-status').textContent, 'Category order applies to shown results');
run(`settings = null; curse.rows = []; curse.total = 0; curse.loaded = 0;`);
element('browse-sort').value = 'name';
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
run(`renderImportPlan([{ name: 'List <b>', key: 'x', installed: false, installable: true, source: { kind: 'github', id: 'a/<img src=x>' }, why: '' }])`);
assert.match(element('import-plan').innerHTML, /a\/&lt;img src=x&gt;/, 'An imported source is shown as text');
assert.doesNotMatch(element('import-plan').innerHTML, /<img/, 'An imported list cannot add markup');
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
  await run(`openDetail('boss', {key:'boss-local',name:'Boss',folders:['Boss'],installed_version:'2',remote_version:'3',remote_prerelease:true})`);
  assert.equal(element('detail-desc').textContent, 'Boss timers');
  assert.match(element('detail-meta').innerHTML, /Installed 2.*Available 3 \(beta\)/);
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

  // Actionable updates move up without changing the order within each group.
  const addon = (key, name, status, flags = {}) => ({
    key, name, status, folders: [name], source: true, source_label: 'GitHub',
    supports_forever: true, missing_deps: [], installed_version: '1', ...flags,
  });
  const batch = [
    addon('normal', 'Alpha', 'ok'),
    addon('pinned', 'Bravo', 'update', { pinned: true, remote_version: '2' }),
    addon('updated', 'Charlie', 'update'),
    addon('ignored', 'Delta', 'update', { ignored: true, remote_version: '2' }),
    addon('failed', 'Zulu', 'update'),
  ];
  context.batch = batch.map(p => ({ ...p }));
  element('show-ignored').checked = false;
  run(`activeFilter = 'all'; packages = batch; renderInstalled();`);
  assert.equal(run(`visiblePackages().map(p => p.key).join()`), 'updated,failed,normal,pinned');
  const batchStart = invokeCalls.length;
  invokeReply = (command, args) => {
    if (command === 'update_package') {
      if (args.key === 'failed') return Promise.reject('cannot replace addon folder QuestieDB: it belongs to folder:QuestieDB');
      return Promise.resolve({ ...batch[2], status: 'ok', installed_version: '2', remote_version: '2' });
    }
    if (command === 'scan') return Promise.resolve(batch.map(p => ({ ...p, status: 'unknown', installed_version: p.key === 'updated' ? '2' : '1' })));
    return Promise.resolve({});
  };
  await run('updateAll()');
  assert.deepEqual(invokeCalls.slice(batchStart).filter(c => c.command === 'update_package').map(c => c.args.key), ['updated', 'failed']);
  assert.ok(invokeCalls.slice(batchStart).some(c => c.command === 'scan'), 'A successful batch item refreshes related rows');
  assert.equal(installedKeys(), 'failed,normal,updated,pinned');
  assert.match(element('toast').textContent, /1 updated, 1 failed \(Zulu\)/);
  assert.match(element('installed-list').innerHTML, /Update failed/);
  assert.match(element('installed-list').innerHTML, /The QuestieDB folder is treated as a separate addon/);
  assert.doesNotMatch(element('installed-list').innerHTML, /folder:QuestieDB/);
  element('show-ignored').checked = true;
  assert.equal(installedKeys(), 'failed,normal,updated,pinned,ignored');
  element('show-ignored').checked = false;
  assert.equal(run(`activeFilter = 'updates'; visiblePackages().map(p => p.key).join()`), 'failed,pinned', 'Updates filter keeps the failed row and the pinned update after refresh');
  // Update all runs several at once: every update starts before the first one finishes.
  const pending = [];
  const parallelBatch = ['a', 'b', 'c', 'd', 'e'].map(k => addon(k, k.toUpperCase(), 'update'));
  context.parallelBatch = parallelBatch;
  run(`packages = parallelBatch.map(p => ({ ...p }))`);
  const parallelStart = invokeCalls.length;
  invokeReply = (command, args) => {
    if (command === 'update_package') return new Promise(resolve => pending.push(() => resolve({ ...parallelBatch.find(p => p.key === args.key), status: 'ok', installed_version: '2' })));
    if (command === 'scan') return Promise.resolve(parallelBatch.map(p => ({ ...p, status: 'ok', installed_version: '2' })));
    return Promise.resolve({});
  };
  const parallelRun = run('updateAll()');
  await new Promise(r => setTimeout(r, 0));
  assert.equal(invokeCalls.slice(parallelStart).filter(c => c.command === 'update_package').length, 4, 'Four updates start together');
  while (pending.length) { pending.shift()(); await new Promise(r => setTimeout(r, 0)); }
  await parallelRun;
  assert.deepEqual(invokeCalls.slice(parallelStart).filter(c => c.command === 'update_package').map(c => c.args.key), ['a', 'b', 'c', 'd', 'e']);
  assert.match(element('toast').textContent, /^5 updated/);
  run(`packages = batch; renderInstalled();`);
  invokeReply = () => Promise.resolve({ ...batch[4], status: 'ok', installed_version: '2' });
  await run(`updateOne(packages.find(p => p.key === 'failed'))`);
  assert.equal(run(`visiblePackages().some(p => p.key === 'failed')`), false, 'Successful retry clears the failure');
  run(`activeFilter = 'all'; packages = []; renderInstalled();`);

  // Reversing remote results restarts pagination, keeps its direction on more,
  // and ignores responses that arrive from the previous direction.
  run(`settings = {has_curse:true}; curse.key = ''; curse.query = ''; curse.loaded = 50; curse.rows = [];`);
  element('browse-sort').value = 'downloads-asc';
  invokeReply = () => Promise.resolve({rows:[{id:10,name:'First',downloads:0}],total:100});
  await run('loadCurse(false)');
  assert.equal(invokeCalls.at(-1).args.sort, 'downloads-asc');
  assert.equal(invokeCalls.at(-1).args.index, 0);
  await run('loadCurse(true)');
  assert.equal(invokeCalls.at(-1).args.sort, 'downloads-asc');
  assert.equal(invokeCalls.at(-1).args.index, 1);
  let finishPreviousSort;
  element('browse-sort').value = 'downloads';
  invokeReply = () => new Promise(resolve => { finishPreviousSort = resolve; });
  const previousSort = run('loadCurse(false)');
  element('browse-sort').value = 'downloads-asc';
  invokeReply = () => Promise.resolve({rows:[{id:20,name:'Latest',downloads:1}],total:100});
  await run('loadCurse(false)');
  finishPreviousSort({rows:[{id:30,name:'Stale',downloads:100}],total:100});
  await previousSort;
  assert.equal(run('curse.rows.map(r => r.id).join()'), '20');
  run(`settings = null; curse.rows = [];`);
  element('browse-sort').value = 'name';

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
  // A CurseForge-only row: one line, and an update you have to fetch yourself.
  run(`packages = [
    {key:'bt',name:'Bartender4',folders:['Bartender4'],source_label:'CurseForge',source_url:'https://www.curseforge.com/projects/1',status:'curse-update',supports_forever:true,missing_deps:[],installed_version:'4.15.3',remote_version:'Bartender4-4.16.0',note:'Bartender4-4.16.0 is on CurseForge; you have 4.15.3. Open its page to download it.'},
    {key:'bg',name:'Bagnon',folders:['Bagnon'],source_label:'CurseForge',status:'curse-only',supports_forever:true,missing_deps:[],note:'The author only distributes this through CurseForge.'}
  ]; activeFilter = 'all'; $('installed-filter').value = ''; $('show-ignored').checked = true; renderInstalled();`);
  assert.match(element('installed-list').innerHTML, /Update on CurseForge/);
  assert.match(element('summary').textContent, /1 to get from CurseForge/);
  assert.equal(String(element('stat-curse').textContent), '1');
  assert.equal(String(element('stat-updates').textContent), '0', 'Update all never counts one it cannot install');
  assert.equal(element('btn-update-all').textContent, 'Update all');
  assert.equal(String(element('stat-attention').textContent), '1');
  assert.equal((element('installed-list').innerHTML.match(/class="note/g) || []).length, 2, 'One line per CurseForge row');
  assert.doesNotMatch(element('installed-list').innerHTML, /data-act="update"/, 'Never an Install or Update button for a row we cannot download');
  run(`activeFilter = 'updates'; renderInstalled();`);
  assert.match(element('installed-list').innerHTML, /Bartender4/, 'The Updates view lists it');
  assert.doesNotMatch(element('installed-list').innerHTML, /Bagnon/);
  run(`activeFilter = 'all';`);

  // Remove for an addon AddonForge did not install asks a different question.
  run(`packages = [{key:'h',name:'HealBot',folders:['HealBot'],source_label:'',status:'no-source',supports_forever:true,missing_deps:[],managed:false,note:'No update source found in its files'}];
    settings = { recycle_label: 'Recycle Bin' }; renderInstalled();`);
  const rowOf = (key) => ({ dataset: { key } });
  const clickAction = (key, act) => {
    const button = { dataset: { act }, closest: (sel) => (sel === '.item' ? rowOf(key) : null) };
    return element('installed-list').listeners.click({ target: { closest: (sel) => (sel === 'button[data-act]' ? button : null) } });
  };
  invokeReply = (command) => Promise.resolve(command === 'remove_package' ? ['HealBot'] : []);
  const removing = clickAction('h', 'remove');
  assert.match(element('modal-text').innerHTML, /didn&#39;t install|didn't install/);
  assert.match(element('modal-text').innerHTML, /Recycle Bin/);
  assert.equal(element('modal-yes').textContent, 'Move to Recycle Bin');
  element('modal-yes').onclick();
  await removing;
  assert.equal(JSON.stringify(invokeCalls.find((c) => c.command === 'remove_package').args), '{"key":"h","recycle":true}');
  assert.match(element('toast').textContent, /Moved HealBot to the Recycle Bin/);

  // An addon it did install keeps the plain delete wording.
  run(`packages = [{key:'m',name:'Mine',folders:['Mine'],source_label:'GitHub',status:'ok',supports_forever:true,missing_deps:[],managed:true}]; renderInstalled();`);
  const removingMine = clickAction('m', 'remove');
  assert.match(element('modal-text').innerHTML, /Remove <b>Mine<\/b>/);
  assert.equal(element('modal-yes').textContent, 'Remove');
  element('modal-yes').onclick();
  await removingMine;
  assert.match(element('toast').textContent, /Removed Mine/);

  // Local ZIP preview escapes names and cancellation never installs.
  invokeCalls.length = 0;
  invokeReply = (command) => Promise.resolve(command === 'preview_zip'
    ? { install: 'QA/game', plan: { digest: 'abc', folders: [['Example<unsafe>', true], ['Options', false]] } }
    : []);
  run('setBusy(false)');
  const cancelledZip = run(`importZip('QA/download.zip')`);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(element('modal-text').innerHTML, /Example&lt;unsafe&gt;/);
  assert.match(element('modal-text').innerHTML, /Replace installed folder/);
  assert.match(element('modal-text').innerHTML, /Install new folder/);
  element('modal-no').onclick();
  await cancelledZip;
  assert.equal(invokeCalls.filter(c => c.command === 'install_zip').length, 0);
  const installedZip = run(`importZip('QA/download.zip')`);
  await new Promise(resolve => setTimeout(resolve, 0));
  element('modal-yes').onclick();
  await installedZip;
  const zipCall = invokeCalls.find(c => c.command === 'install_zip');
  assert.equal(zipCall.args.path, 'QA/download.zip');
  assert.equal(zipCall.args.plan.digest, 'abc');
  assert.equal(run('busy'), false);
  invokeReply = () => Promise.reject(new Error('Unsafe ZIP'));
  await run(`importZip('QA/unsafe.zip')`);
  assert.match(element('toast').textContent, /Unsafe ZIP/);
  assert.equal(run('busy'), false);

  // Exercise the native drag callback with the same window API the app uses.
  run(`T.webviewWindow = { getCurrentWebviewWindow: () => ({ onDragDropEvent: (fn) => { window.zipDrop = fn; return Promise.resolve(() => {}); } }) };`);
  vm.runInContext(fs.readFileSync('ui/app.js', 'utf8').split('// Native Tauri drag events provide paths without exposing filesystem access to JS.')[1], context);
  invokeCalls.length = 0;
  run(`window.zipDrop({payload:{type:'drop',paths:['a.zip','b.zip']}})`);
  assert.match(element('toast').textContent, /one downloaded addon ZIP/);
  assert.equal(invokeCalls.length, 0);
  run(`setBusy(true); window.zipDrop({payload:{type:'drop',paths:['a.zip']}}); setBusy(false);`);
  assert.equal(invokeCalls.length, 0);
  invokeReply = () => Promise.reject(new Error('Drop preview reached'));
  run(`window.zipDrop({payload:{type:'drop',paths:['QA/Addon.ZIP']}})`);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(invokeCalls[0].command, 'preview_zip');
  assert.equal(invokeCalls[0].args.path, 'QA/Addon.ZIP');
  assert.equal(run('busy'), false);

  run('clearTimeout(toastTimer)');
  console.log('UI checks passed: counts, eligible updates, failure priority and retry, filters, search, escaping, empty states, modal focus and cancellation, sort direction and remote pagination, saved preferences, backups and folder actions, CurseForge-only rows, reversible removal, ZIP confirmation/cancellation/errors and native drop routing.');
})().catch((error) => { console.error(error); process.exitCode = 1; });
