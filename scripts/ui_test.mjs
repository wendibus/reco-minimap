// Smoke test of the React interface in headless Chrome (Chrome DevTools Protocol, no npm packages):
// every step in every language renders, no i18n key shows up untranslated, every button carries a
// hover hint, the hints switch works and nothing throws.
//
//   reco-minimap ui --no-open --no-update-check --port 18790 &      (a project with model + court.json + teams.json)
//   node scripts/ui_test.mjs [http://127.0.0.1:18790/] [project-dir]
//
// CHROME=<path> picks the browser (default: Google Chrome / Chromium on PATH).
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const url = process.argv[2] ?? 'http://127.0.0.1:18790/';
const project = process.argv[3];
const chrome = process.env.CHROME ?? ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser'].find(existsSync);
if (!chrome) { console.error('Chrome not found (set CHROME)'); process.exit(2); }
const port = 9300 + Math.floor(Math.random() * 600);
const profile = mkdtempSync(join(tmpdir(), 'reco-ui-test-'));
const browser = spawn(chrome, ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--window-size=1360,900', 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));
let failed = false;
const fail = (m) => { console.error('FAIL:', m); failed = true; };

async function connect() {
  for (let i = 0; i < 50; i++) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' });
      if (r.ok) return (await r.json()).webSocketDebuggerUrl;
    } catch { /* still starting */ }
    await sleep(200);
  }
  throw new Error('no DevTools endpoint');
}

const ws = new WebSocket(await connect());
await new Promise((ok) => ws.addEventListener('open', ok));
let seq = 0;
const pending = new Map();
const errors = [];
ws.addEventListener('message', (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
  if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') errors.push(m.params.args.map((a) => a.value ?? a.description).join(' '));
});
const send = (method, params = {}) => new Promise((ok) => { const id = ++seq; pending.set(id, ok); ws.send(JSON.stringify({ id, method, params })); });
const run = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? 'evaluate failed');
  return r.result?.result?.value;
};

await send('Runtime.enable');
await send('Page.enable');
await send('Page.navigate', { url });
await sleep(1500);
if (project) {
  await run(`(async () => { const i = document.querySelector('.mm-dir'); const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(i, ${JSON.stringify(project)}); i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); })()`);
  await sleep(800);
}

const KEY = /\b(sport|model|video|cal|run|live|eval|step|btn|status|zone1|lm|ticker|board|upd|nav|log|project)\.(?!json\b|html\b)[a-z0-9_]+\b/;
const report = {};
for (const lang of ['de', 'en', 'fr', 'es']) {
  await run(`[...document.querySelectorAll('.language-switch button')].find(b => b.textContent === '${lang.toUpperCase()}').click()`);
  await sleep(200);
  const steps = await run(`[...document.querySelectorAll('.mm-step')].length`);
  if (steps !== 7) fail(`${lang}: expected 7 steps, got ${steps}`);
  for (let i = 0; i < steps; i++) {
    await run(`document.querySelectorAll('.mm-step')[${i}].click()`);
    await sleep(350);
    const r = await run(`(() => {
      const text = document.querySelector('.preview-page').innerText;
      const noHint = [...document.querySelectorAll('button')].filter(b => !b.closest('[data-hint]') && !b.dataset.hint).map(b => b.textContent.trim() || b.outerHTML.slice(0, 60));
      return { len: document.querySelector('.mm-panel').innerText.length, title: document.querySelector('.mm-step-title').innerText, text, noHint };
    })()`);
    if (r.len < 60) fail(`${lang} step ${i + 1}: empty panel`);
    const key = r.text.match(KEY);
    if (key) fail(`${lang} step ${i + 1}: untranslated key ${key[0]}`);
    if (r.noHint.length) fail(`${lang} step ${i + 1}: buttons without hint: ${r.noHint.join(' | ')}`);
    (report[lang] ??= []).push(r.title);
  }
}
console.log(JSON.stringify(report, null, 1));

// hover hints: on by default, the switch turns them off and on again
await run(`document.querySelectorAll('.mm-step')[0].click()`);
const box = await run(`(() => { const r = document.querySelector('.mm-step').getBoundingClientRect(); return { x: r.x + 10, y: r.y + 8 }; })()`);
const hover = async () => {
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: box.x, y: box.y, pointerType: 'mouse' });
  await sleep(600);
  const shown = await run(`!!document.querySelector('.hint-bubble')`);
  await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 5, y: 890, pointerType: 'mouse' });
  await sleep(100);
  return shown;
};
if (!(await hover())) fail('hint bubble does not appear');
await run(`document.querySelector('.hints-toggle').click()`);
if (await hover()) fail('hint bubble appears although hints are off');
await run(`document.querySelector('.hints-toggle').click()`);

if (errors.length) fail('errors in the page:\n  ' + errors.join('\n  '));
ws.close();
browser.kill();
await sleep(500); // Chrome still writes its profile while it exits
try { rmSync(profile, { recursive: true, force: true, maxRetries: 5 }); } catch { /* a temporary folder */ }
console.log(failed ? 'UI test FAILED' : 'UI test ok');
process.exit(failed ? 1 : 0);
