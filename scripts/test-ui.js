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
const context = vm.createContext({
  document, window: { __TAURI__: { core: { invoke(command) { if (command === "addon_details") return detailReply; } }, opener: {} } },
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
  console.log('UI checks passed: counts, eligible updates, filters, search, escaping, empty states, modal focus and cancellation.');
})().catch((error) => { console.error(error); process.exitCode = 1; });
