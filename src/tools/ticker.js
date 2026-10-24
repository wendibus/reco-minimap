// reco-minimap live ticker: one piece of code for the ticker page, the iframe and the <reco-ticker> widget.
// (The texts, RT_STRINGS, are put in front of this file; the whole is wrapped so it adds nothing to the host page.)
const STYLE = `
:host{display:block;--rt-home:#e63946;--rt-away:#3b82f6;--rt-acc:#3fa7ff;font:16px/1.4 system-ui,-apple-system,"Segoe UI",sans-serif}
*{box-sizing:border-box}
[hidden]{display:none!important}
.w{--card:var(--rt-card,#1a2129);--line:var(--rt-line,#2a343f);--fg:var(--rt-fg,#e9edf1);--dim:var(--rt-dim,#8e9ba8);--acc:var(--rt-acc);--home:var(--rt-home);--away:var(--rt-away);color:var(--fg)}
.w.light{--card:var(--rt-card,#fff);--line:var(--rt-line,#d3dae1);--fg:var(--rt-fg,#18202a);--dim:var(--rt-dim,#5d6a77)}
@media (prefers-color-scheme:light){.w.auto{--card:var(--rt-card,#fff);--line:var(--rt-line,#d3dae1);--fg:var(--rt-fg,#18202a);--dim:var(--rt-dim,#5d6a77)}}
header{display:flex;flex-wrap:wrap;gap:8px 12px;align-items:center;margin-bottom:10px}
h1{font-size:20px;margin:0;flex:1 1 100%}
h1 .vs{color:var(--dim);font-weight:400;padding:0 6px}
.home{color:var(--home)}.away{color:var(--away)}
.sub{color:var(--dim);font-size:13px;flex:1 1 100%}
.dot{display:inline-block;width:9px;height:9px;border-radius:50%;background:#43c37b;margin-right:6px;animation:p 1.6s infinite}
.dot.off{background:#c9a227;animation:none}
@keyframes p{50%{opacity:.35}}
.bar{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin:8px 0 14px}
.seg{display:flex;border:1px solid var(--line);border-radius:10px;overflow:hidden}
.seg button{border:0;border-radius:0;border-right:1px solid var(--line)}
.seg button:last-child{border-right:0}
button,select{font:inherit;color:var(--fg);background:var(--card);border:1px solid var(--line);border-radius:10px;min-height:40px;padding:0 12px;cursor:pointer}
button.on{background:var(--acc);color:#fff;border-color:var(--acc)}
ol{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:8px}
li{display:flex;gap:12px;background:var(--card);border:1px solid var(--line);border-left:4px solid var(--line);border-radius:12px;padding:10px 12px;animation:in .35s ease-out}
li.home{border-left-color:var(--home)}li.away{border-left-color:var(--away)}
li.weak{opacity:.78}
@keyframes in{from{opacity:0;transform:translateY(-6px)}}
.t{font-variant-numeric:tabular-nums;color:var(--dim);min-width:3.2em;font-size:14px;padding-top:2px}
.x{flex:1}
.x b{font-weight:600}
.tag{display:inline-block;font-size:12px;color:var(--dim);border:1px solid var(--line);border-radius:6px;padding:0 6px;margin-left:6px}
.empty{color:var(--dim);text-align:center;padding:28px 10px}
footer{color:var(--dim);font-size:12px;margin-top:14px;text-align:center}
.admin{border:1px solid var(--acc);border-radius:12px;padding:10px 12px;margin-bottom:14px;background:var(--card)}
.admin h2{font-size:15px;margin:0 0 8px}
.admin .row{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-top:8px}
li .act{display:flex;gap:6px;align-items:flex-start}
li .act button{min-height:36px;padding:0 10px}
button.ok{background:#43c37b;border-color:#43c37b;color:#fff}button.no{background:transparent;color:#ff6b6b;border-color:#ff6b6b}
`;

const LANG_IDS = Object.keys(RT_STRINGS);
function pickLang(pref) {
  let saved = null; try { saved = localStorage.getItem('lang'); } catch (e) {}
  const nav = (navigator.language || 'en').slice(0, 2).toLowerCase();
  return [pref, saved, nav].map(x => x && x.slice(0, 2).toLowerCase()).find(x => x && RT_STRINGS[x]) || 'en';
}
const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const clock = t => { const s = Math.max(0, Math.round(t || 0)); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };

// o: { feed, incremental, admin, lang, show, max, embed, theme, onDraw }
function mount(root, o) {
  o = Object.assign({ feed: '/ticker.json', incremental: false, admin: false, lang: '', show: '', max: 0, embed: false, theme: 'auto', onDraw: null, every: 3000 }, o);
  let LANG = pickLang(o.lang);
  const T = (k, v) => { let s = (RT_STRINGS[LANG] || {})[k]; if (s === undefined) s = RT_STRINGS.en[k]; if (s === undefined) return k; if (v) for (const n in v) s = s.split('{' + n + '}').join(v[n]); return s; };
  const hasKey = k => RT_STRINGS.en[k] !== undefined;
  let cfg = { names: 'numbers', teams: { home: '', away: '' }, players: { home: {}, away: {} }, sport: '' };
  let events = [], seq = 0, mode = 'numbers', lastOk = 0, pending = [], manual = false, retracted = [], beat = null, beatAt = 0, timer = 0, stopped = false;

  root.innerHTML = `<style>${STYLE}</style><div class="w ${['dark', 'light'].includes(o.theme) ? o.theme : 'auto'}">
<header ${o.embed ? 'hidden' : ''}><h1><span class="home" data-r="th"></span><span class="vs">–</span><span class="away" data-r="ta"></span></h1>
<div class="sub"><span class="dot off" data-r="dot"></span><span data-r="status"></span></div></header>
<div class="bar" ${o.embed ? 'hidden' : ''}><div class="seg" data-r="mode" hidden><button data-m="numbers"></button><button data-m="names"></button><button data-m="both"></button></div>
<select data-r="lang" aria-label="language"></select><button data-r="share"></button></div>
<div class="admin" data-r="admin" hidden><h2><span data-r="aq"></span> <span data-r="pcount"></span></h2><ol data-r="pend"></ol>
<div class="row"><button class="ok" data-r="relall"></button><label style="display:flex;gap:8px;align-items:center;color:var(--dim)"><input type="checkbox" data-r="manual" style="width:20px;height:20px"> <span data-r="amanual"></span></label></div>
<div class="sub" style="margin-top:8px" data-r="ahint"></div></div>
<div class="empty" data-r="empty"></div><ol data-r="feed"></ol><footer data-r="note" ${o.embed ? 'hidden' : ''}></footer></div>`;
  const R = {}; root.querySelectorAll('[data-r]').forEach(e => { R[e.dataset.r] = e; });
  const W = root.querySelector('.w');

  const teamName = t => (cfg.teams && cfg.teams[t]) || (t === 'home' || t === 'away' ? T('team.' + t) : '');
  // A player as the viewer wants to see it: "#7", "Anna" or "#7 Anna"; without a name the number stands in.
  function who(team, n) {
    if (n == null) return T('ticker.someone');
    const name = cfg.players && cfg.players[team] && cfg.players[team][String(n)];
    if (mode === 'names') return name ? esc(name) : '#' + n;
    if (mode === 'both') return name ? '#' + n + ' ' + esc(name) : '#' + n;
    return '#' + n;
  }
  function describe(ev) {
    const team = teamName(ev.team), tb = team ? '<b>' + esc(team) + '</b>' : '';
    if (ev.type === 'sub') {
      if (ev.kind === 'confirmed') return { text: T('ticker.sub', { team: tb, out: who(ev.team, ev.out), in: who(ev.team, ev.in) }), weak: false };
      return { text: T('ticker.sub_possible', { team: tb, out: who(ev.team, ev.out) }), weak: true, tag: T('ticker.unconfirmed') };
    }
    if (ev.type === 'scene') {
      let k = ev.kind;
      if (k === 'kickoff' && cfg.sport === 'handball') k = 'throw_off';
      else if (k === 'throw_in' && cfg.sport === 'basketball') k = 'inbound';
      else if (k === 'free_throw' && ev.phase === 'attempt') k = 'free_throw_attempt';
      const label = T('scene.' + k) + (ev.side === 'l' ? ' ←' : ev.side === 'r' ? ' →' : '');
      return { text: tb ? label + ' · ' + tb : label, weak: false, tag: ev.borderline ? T('scene.borderline') : '' };
    }
    return null;
  }
  const row = (ev, d, extra) => `<li class="${ev.team === 'home' || ev.team === 'away' ? ev.team : ''}${d.weak ? ' weak' : ''}"><span class="t">${clock(ev.t)}</span><span class="x">${d.text}${d.tag ? '<span class="tag">' + esc(d.tag) + '</span>' : ''}</span>${extra || ''}</li>`;

  function draw() {
    R.th.textContent = teamName('home'); R.ta.textContent = teamName('away');
    const rows = []; let shown = 0;
    for (let i = events.length - 1; i >= 0; i--) {
      const ev = events[i]; if (retracted.includes(ev.seq)) continue;
      const d = describe(ev); if (!d) continue;
      if (o.max && shown >= o.max) break;
      shown++;
      rows.push(row(ev, d, o.admin ? `<span class="act"><button class="no" data-retract="${ev.seq}">${esc(T('ticker.a_retract'))}</button></span>` : ''));
    }
    const html = rows.join('');
    if (R.feed.dataset.h !== html) { R.feed.innerHTML = html; R.feed.dataset.h = html; } // untouched rows keep their place
    R.empty.hidden = rows.length > 0; R.empty.textContent = T('ticker.empty');
    const sport = cfg.sport && hasKey('sport.' + cfg.sport) ? T('sport.' + cfg.sport) : '';
    const live = beat !== null ? Date.now() - beatAt < 45000 : Date.now() - lastOk < 12000;
    R.dot.className = 'dot' + (live ? '' : ' off');
    R.status.textContent = (sport ? sport + ' · ' : '') + (live ? T('ticker.live') : T('ticker.offline'));
    W.querySelectorAll('[data-m]').forEach(b => b.classList.toggle('on', b.dataset.m === mode));
    if (o.admin) drawPending();
    if (o.onDraw) o.onDraw();
  }
  function drawPending() {
    R.admin.hidden = false; R.pcount.textContent = pending.length ? '(' + pending.length + ')' : '';
    R.relall.disabled = !pending.length; R.manual.checked = manual;
    R.pend.innerHTML = pending.length ? pending.map(ev => { const d = describe(ev); return d ? row(ev, d, `<span class="act"><button class="ok" data-release="${ev.id}">${esc(T('ticker.a_release'))}</button><button class="no" data-discard="${ev.id}">${esc(T('ticker.a_discard'))}</button></span>`) : ''; }).join('') : `<li><span class="x" style="color:var(--dim)">${esc(T('ticker.a_none'))}</span></li>`;
  }
  // The viewer may only choose among what the operator allows: with "numbers" no names are ever sent.
  function setupMode() {
    const allow = cfg.names !== 'numbers' || o.admin;
    R.mode.hidden = !allow || o.embed;
    let saved = null; try { saved = localStorage.getItem('ticker.mode'); } catch (e) {}
    const wanted = o.show || saved || cfg.names || 'numbers';
    mode = !allow ? 'numbers' : (['numbers', 'names', 'both'].includes(wanted) ? wanted : cfg.names);
  }
  async function act(path) {
    try { await fetch(o.admin + '/ticker/admin/' + path, { method: 'POST', headers: { 'x-ticker-admin': '1' } }); } catch (e) {}
    poll();
  }
  async function poll() {
    if (stopped) return;
    try {
      if (o.admin) {
        const r = await fetch(o.admin + '/ticker/admin.json', { cache: 'no-store' }); if (!r.ok) throw new Error(r.status);
        const j = await r.json();
        cfg = j.config || cfg; events = j.published || []; pending = j.pending || []; manual = !!j.approve; retracted = j.retracted || [];
        lastOk = Date.now(); setupMode();
      } else if (o.incremental) {
        const r = await fetch(o.feed + '?since=' + seq, { cache: 'no-store' }); if (!r.ok) throw new Error(r.status);
        const j = await r.json(); cfg = j.config || cfg;
        if (j.seq < seq) { events = []; seq = 0; return poll(); } // the program restarted
        events = events.concat(j.events || []); seq = j.seq; retracted = j.retracted || []; lastOk = Date.now(); setupMode();
      } else {
        const r = await fetch(o.feed + (o.feed.includes('?') ? '&' : '?') + 't=' + Date.now(), { cache: 'no-store' }); if (!r.ok) throw new Error(r.status);
        const j = await r.json(); cfg = j.config || cfg; events = j.events || []; retracted = j.retracted || []; lastOk = Date.now();
        if (j.beat !== undefined && j.beat !== beat) { beat = j.beat; beatAt = Date.now(); } // the sender writes a new beat every few seconds
        setupMode();
      }
    } catch (e) { /* offline: the status line says so */ }
    draw();
  }
  function texts() {
    R.aq.textContent = T('ticker.a_queue'); R.relall.textContent = T('ticker.a_release_all'); R.amanual.textContent = T('ticker.a_manual'); R.ahint.textContent = T('ticker.a_hint');
    const m = { numbers: 'ticker.m_numbers', names: 'ticker.m_names', both: 'ticker.m_both' };
    W.querySelectorAll('[data-m]').forEach(b => { b.textContent = T(m[b.dataset.m]); });
    R.share.textContent = T('ticker.share'); R.note.textContent = T('ticker.note');
    R.lang.innerHTML = LANG_IDS.map(k => `<option value="${k}"${k === LANG ? ' selected' : ''}>${esc(RT_STRINGS[k]['lang.name'])}</option>`).join('');
    draw();
  }
  W.addEventListener('click', e => {
    const b = e.target.closest && e.target.closest('button'); if (!b) return;
    if (b.dataset.m) { mode = b.dataset.m; try { localStorage.setItem('ticker.mode', mode); } catch (x) {} draw(); }
    else if (!o.admin) return;
    else if (b.dataset.release) act('release?id=' + b.dataset.release);
    else if (b.dataset.discard) act('discard?id=' + b.dataset.discard);
    else if (b.dataset.retract) act('retract?seq=' + b.dataset.retract);
  });
  R.relall.onclick = () => act('release_all');
  R.manual.onchange = e => act('mode?approve=' + (e.target.checked ? 1 : 0));
  R.lang.onchange = e => { LANG = e.target.value; try { localStorage.setItem('lang', LANG); } catch (x) {} texts(); };
  R.share.onclick = async () => {
    const u = new URL(location.href); u.searchParams.set('lang', LANG); if (mode !== 'numbers') u.searchParams.set('show', mode); const url = u.toString();
    try { if (navigator.share) { await navigator.share({ title: T('ticker.title'), url }); return; } } catch (x) { return; }
    try { await navigator.clipboard.writeText(url); R.share.textContent = T('ticker.copied'); setTimeout(() => { R.share.textContent = T('ticker.share'); }, 1800); } catch (x) { prompt(T('ticker.share'), url); }
  };
  texts(); poll(); timer = setInterval(poll, o.admin ? 1500 : o.every);
  return { stop() { stopped = true; clearInterval(timer); }, title: () => T('ticker.title') + (o.admin ? ' – ' + T('ticker.a_title') : ''), lang: () => LANG };
}

// <reco-ticker feed="ticker.json" lang="de" show="numbers" max="20" theme="auto">  (embedded by default: no header)
class RecoTickerEl extends HTMLElement {
  connectedCallback() {
    if (this._m) return;
    const a = n => this.getAttribute(n) || '';
    this._m = mount(this.shadowRoot || this.attachShadow({ mode: 'open' }), { feed: a('feed') || 'ticker.json', lang: a('lang'), show: a('show'), max: parseInt(a('max')) || 0, theme: a('theme') || 'auto', embed: a('embed') !== '0' && a('embed') !== 'false' });
  }
  disconnectedCallback() { if (this._m) { this._m.stop(); this._m = null; } }
}
if (!customElements.get('reco-ticker')) customElements.define('reco-ticker', RecoTickerEl);
window.RecoTicker = { mount };

// <script src="ticker.js" data-feed="ticker.json" data-lang="de" ...></script>: the widget appears where the script stands
const me = document.currentScript;
if (me && me.dataset && me.dataset.feed) {
  const el = document.createElement('reco-ticker');
  for (const k of ['feed', 'lang', 'show', 'max', 'theme', 'embed']) if (me.dataset[k] !== undefined) el.setAttribute(k, me.dataset[k]);
  me.parentNode.insertBefore(el, me.nextSibling);
}
