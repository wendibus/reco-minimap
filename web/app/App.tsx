import { useEffect, useRef, useState } from 'react';
import { HintLayer } from './HintLayer';
import { JobDialog } from './JobDialog';
import { api, Btn, LANGS, useApp } from './core';
import { CalibPanel, EvalPanel, LivePanel, RunPanel, SetupPanel, VideoPanel } from './panels';
import { StatsPanel } from './stats';

const STEPS = ['setup', 'video', 'calib', 'run', 'live', 'eval', 'stats'] as const;
type Step = (typeof STEPS)[number];
const PANELS: Record<Step, () => React.ReactNode> = { setup: SetupPanel, video: VideoPanel, calib: CalibPanel, run: RunPanel, live: LivePanel, eval: EvalPanel, stats: StatsPanel };

function storedStep(): Step {
  try { const s = localStorage.getItem('reco-minimap-step') as Step; return STEPS.includes(s) ? s : 'setup'; } catch { return 'setup'; }
}

export function App() {
  const app = useApp();
  const { init, t, lang, setLanguage, hints, toggleHints, dir, openProject, project, mutate, log, logLines, jobs, runJob } = app;
  const h = t.hints;
  const [step, setStep] = useState<Step>(storedStep);
  const [dirInput, setDirInput] = useState('');
  const [logOpen, setLogOpen] = useState(false);
  const [update, setUpdate] = useState<{ version: string; url: string; installed?: boolean } | null>(null);
  const logRef = useRef<HTMLPreElement>(null);

  useEffect(() => setDirInput(dir), [dir]);
  const panelRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    try { localStorage.setItem('reco-minimap-step', step); } catch { /* private window */ }
    panelRef.current?.scrollTo(0, 0);
  }, [step]);
  useEffect(() => { document.title = 'Reco Minimap'; }, []);
  // newer version on GitHub: a banner, installed only when the user asks for it
  useEffect(() => {
    let tries = 4, stop = false;
    const poll = async () => {
      const r = await api('/api/update');
      if (stop) return;
      if (r.ok && r.update) { if (r.update.latest) setUpdate(r.update.latest); return; }
      if (tries-- > 0) setTimeout(() => void poll(), 3000);
    };
    void poll();
    return () => { stop = true; };
  }, []);

  const last = jobs[jobs.length - 1];
  const lines = [...logLines, ...(last ? last.lines.map((l) => l.t) : [])];
  useEffect(() => { if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight; }, [lines.length, logOpen]);

  if (!init) return <main className="preview-page" />;
  const Panel = PANELS[step];
  const stepLabel = (s: Step) => s === 'stats' ? t.ui.stepStats : T('step.' + s);

  return <main className="preview-page"><HintLayer enabled={hints} /><JobDialog />
    <header className="preview-header">
      <div className="brand-block"><div className="brand-mark" aria-hidden="true">M</div><div><h1>Reco Minimap <span className="version-button" data-hint={h.version}>{init.version}</span></h1><p>{t.ui.tagline}</p></div></div>
      <div className="switches">
        <button type="button" data-hint={hints ? h.toggleOn : h.toggleOff} className={hints ? 'hints-toggle active' : 'hints-toggle'} aria-pressed={hints} onClick={toggleHints}>💬 {h.toggle}</button>
        <div className="segmented language-switch" role="group" aria-label={T('lang.label')}>{LANGS.map((l) => <button data-hint={h.language} type="button" key={l} className={lang === l ? 'active' : ''} onClick={() => setLanguage(l)}>{l.toUpperCase()}</button>)}</div>
      </div>
      <div className="header-actions">
        <a className="walkthrough-launch" data-hint={h.guide} href={`/guide?lang=${lang}`} target="_blank" rel="noreferrer">▷ {T('nav.guide')}</a>
        <a className="walkthrough-launch" data-hint={h.review} href="/review" target="_blank" rel="noreferrer">◎ {T('nav.review')}</a>
      </div>
    </header>
    {update && <div className="update-banner"><span>⬇ {update.installed ? T('upd.restart', { v: update.version }) : T('upd.available', { v: update.version, cur: init.version })}</span>
      {!update.installed && <><a href={update.url} target="_blank" rel="noreferrer" className="update-banner-dismiss">{T('upd.notes')}</a><button data-hint={h.update} type="button" className="update-banner-download" onClick={() => void runJob('update', {}).then((s) => s === 'done' && setUpdate({ ...update, installed: true }))}>{T('upd.install')}</button></>}</div>}
    {init.ort_error && <div className="update-banner mm-error-banner"><span>{init.ort_error}</span></div>}
    <section className="preview-stage"><div className="app-window platform-mac"><div className="app-body">
      <aside className="app-sidebar">
        <section className="side-section"><label>{t.ui.project}</label>
          <button data-hint={h.chooseFolder} type="button" className="secondary-button folder-button" onClick={async () => { const r = await api('/api/dialog', { kind: 'folder', prompt: T('project.folder') }); if (r.path) void openProject(r.path); }}>▣&nbsp; {t.ui.choose}</button>
          <div className="mm-row"><input type="text" className="mm-grow mm-dir" value={dirInput} onChange={(e) => setDirInput(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') void openProject(dirInput); }} aria-label={T('project.folder')} />
            <button type="button" className="tool-button" data-hint={h.openFolder} onClick={() => void openProject(dirInput)}>{T('project.open')}</button></div>
          <button type="button" className="mm-link small" data-hint={h.showFolder} onClick={() => void api('/api/reveal', { path: dir, open: false }).then((r) => !r.ok && log(T('status.error', { m: r.error })))}>{T('btn.reveal')}</button>
        </section>
        <section className="side-section"><label>{T('sport.label')}</label>
          <select data-hint={h.sport} aria-label={T('sport.label')} value={project.sport ?? 'basketball'} onChange={(e) => mutate((p) => { p.sport = e.target.value; })}>{init.sports.map((s) => <option key={s.id} value={s.id}>{T('sport.' + s.id)}</option>)}</select>
        </section>
        <section className="side-section mm-steps"><label>{t.ui.steps}</label>
          {STEPS.map((s) => <button key={s} type="button" data-hint={h.step} className={step === s ? 'mm-step active' : 'mm-step'} onClick={() => setStep(s)}>{stepLabel(s)}</button>)}
        </section>
        <div className="privacy-lock"><span>●</span> {t.ui.privacy}</div>
      </aside>
      <div className="mm-main">
        <div className="mm-panel" ref={panelRef}>
          <h1 className="mm-step-title">{stepLabel(step)}</h1>
          <Panel />
        </div>
        <footer className={logOpen ? 'mm-log open' : 'mm-log'}>
          <div className="mm-log-head"><button type="button" className="mm-link" data-hint={h.showLog} onClick={() => setLogOpen((v) => !v)}>{logOpen ? '▾' : '▸'} {t.ui.log}</button>
            {last && <span className={last.status === 'done' ? 'ok' : last.status === 'failed' || last.status === 'cancelled' ? 'bad' : 'mm-muted'}>{t.job.titles[last.kind] ?? last.kind} · {T('status.' + (last.status === 'cancelled' || last.status === 'cancelling' ? 'failed' : last.status))}</span>}
            {last && (last.status === 'running') && (last.kind === 'live' || last.kind === 'board') && <Btn hint={h.cancelJob} onClick={() => void app.cancelJob(last.id, true)}>{T('btn.cancel')}</Btn>}
          </div>
          {logOpen && <pre ref={logRef} className="mm-log-body">{lines.join('\n')}</pre>}
        </footer>
      </div>
    </div></div></section>
  </main>;
}
