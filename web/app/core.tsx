import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { TEXT, type Lang } from './text';

// ---------------------------------------------------------------- shared i18n.js (scripts/gen_i18n.py)
declare global {
  // eslint-disable-next-line no-var
  var I18N: Record<string, Record<string, string>>;
  // eslint-disable-next-line no-var
  var LANG: string;
  function T(key: string, vars?: Record<string, unknown>): string;
  function setLang(lang: string): void;
}

export const LANGS: Lang[] = ['de', 'en', 'fr', 'es'];

// ---------------------------------------------------------------- API
export type Json = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

export async function api(path: string, body?: Json): Promise<Json> {
  try {
    const r = await fetch(path, body === undefined ? {} : { method: 'POST', body: JSON.stringify({ ...body, lang: LANG }) });
    return await r.json();
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

export type Model = { id: string; dir: string; sport?: string; classes: string[]; input_size: number; size_mb: number; sha: 'ok' | 'bad' | 'none' | string; builtin?: boolean; contract?: string };
export type OutFile = { name: string; path: string; mb: number | null; t: number | null };
export type Files = { court: Json | null; scales: Json | null; teams: Json | null; out: OutFile[] };
export type Sport = { id: string; field: [number, number]; team_size: number; referees: number; keepers?: boolean };
export type Init = Json & { ok: boolean; version: string; default_dir: string; sports: Sport[]; ocr?: { exists: boolean }; camera?: { id: string; mb: number; exists: boolean }; lan_ip?: string; ort_error?: string };

export type JobStatus = 'running' | 'cancelling' | 'done' | 'failed' | 'cancelled';
export type JobView = {
  id: number; kind: string; status: JobStatus; progress: number | null; message: string; stage: { index: number; count: number; label: string } | null;
  elapsedSeconds: number; etaSeconds: number | null; keepable: boolean; kept: boolean | null; cancellable: boolean;
  lines: { t: string; p: boolean }[]; startedAt: number;
  /** Set by the page, e.g. the section of a run queue. */
  outer?: { index: number; count: number; label: string } | null;
};

/** Kinds that run as servers until stopped: they have their own controls, no progress window. */
export const BACKGROUND_KINDS = new Set(['live', 'board']);

// ---------------------------------------------------------------- project defaults (same as before the React rebuild)
function mergeDefaults(t: Json, d: Json) {
  for (const k in d) {
    if (t[k] === undefined) t[k] = d[k];
    else if (d[k] && typeof d[k] === 'object' && !Array.isArray(d[k])) mergeDefaults(t[k], d[k]);
  }
}
const defaults = (): Json => ({
  lang: LANG, sport: 'basketball', custom: { length: 40, width: 20, team_size: 5, referees: 2 }, model_id: '', video: '', calib_time: 60,
  colors: { draw_home: '#ffffff', draw_away: '#2f6bff', ref_color: '#000000' }, segments: [{ name: T('run.segname') + '1', start: 0, duration: 0 }],
  figures: { torso_top: 0.18, torso_bottom: 0.5, shirt_match: false, no_referees: false },
  run: { every: 5, outputs: { minimap: true, alpha: false, pip: false, json: true }, alpha_bg: 0.6, min_alpha: 1, ep: 'auto', units: 'all', scenes: false, workers: 1, numbers: { enabled: false, hires: '', roster_home: '', roster_away: '' } },
  live: { source: '', size: '1920x1080', fps: 25, port: 8765, realtime: false, adaptive: false, ticker_names: 'numbers', home_name: '', away_name: '', network: false, ticker_approve: false, ticker_export: false, ticker_push: '', ticker_key: '' },
});

export function getPath(o: Json, path: string): unknown {
  return path.split('.').reduce<unknown>((a, k) => (a as Json | undefined)?.[k], o);
}
function setPath(o: Json, path: string, v: unknown) {
  const p = path.split('.');
  let x = o;
  for (let i = 0; i < p.length - 1; i++) x = x[p[i]] ??= {};
  x[p[p.length - 1]] = v;
}

// ---------------------------------------------------------------- app state
type State = {
  init: Init | null; dir: string; project: Json; models: Model[]; files: Files;
};

type Ctx = State & {
  lang: Lang; setLanguage: (l: Lang) => void; t: (typeof TEXT)['de'];
  hints: boolean; toggleHints: () => void;
  set: (path: string, value: unknown) => void;
  mutate: (fn: (p: Json) => void) => void;
  openProject: (dir: string) => Promise<void>;
  refresh: () => Promise<void>;
  log: (line: string) => void; logLines: string[];
  jobs: JobView[]; runJob: (kind: string, params: Json, outer?: JobView['outer']) => Promise<JobStatus>;
  cancelJob: (id: number, keep: boolean) => Promise<void>;
  activeModel: () => Model | null; sport: () => Sport; common: () => Json; isKeeperSport: () => boolean;
};

const AppCtx = createContext<Ctx | null>(null);
export const useApp = () => useContext(AppCtx)!;

function storedHints() {
  try { return localStorage.getItem('reco-minimap-hints') !== 'off'; } catch { return true; }
}

export function AppProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<State>({ init: null, dir: '', project: {}, models: [], files: { court: null, scales: null, teams: null, out: [] } });
  const [lang, setLangState] = useState<Lang>(LANG as Lang);
  const [hints, setHints] = useState(storedHints);
  const [logLines, setLog] = useState<string[]>([]);
  const [jobs, setJobs] = useState<JobView[]>([]);
  const ref = useRef(state);
  ref.current = state;
  const saveTimer = useRef(0);

  const save = useCallback((dir: string, project: Json) => {
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => void api('/api/project/save', { dir, project }), 250);
  }, []);
  const mutate = useCallback((fn: (p: Json) => void) => {
    setState((s) => {
      const project = structuredClone(s.project);
      fn(project);
      save(s.dir, project);
      return { ...s, project };
    });
  }, [save]);
  const set = useCallback((path: string, value: unknown) => mutate((p) => setPath(p, path, value)), [mutate]);
  const log = useCallback((line: string) => setLog((l) => [...l.slice(-400), line]), []);

  const setLanguage = useCallback((l: Lang) => {
    setLang(l);
    setLangState(l);
    document.documentElement.lang = l;
    if (ref.current.dir) mutate((p) => { p.lang = l; });
  }, [mutate]);

  const openProject = useCallback(async (dir: string) => {
    const r = await api('/api/project/load', { dir });
    if (!r.ok) { log(T('status.error', { m: r.error })); return; }
    const project = r.project as Json;
    mergeDefaults(project, defaults());
    if (project.lang && I18N[project.lang]) { setLang(project.lang); setLangState(project.lang); }
    setState((s) => ({ ...s, dir: r.dir, project, models: r.models, files: r.files }));
  }, [log]);
  const refresh = useCallback(async () => {
    const r = await api('/api/project/load', { dir: ref.current.dir });
    if (r.ok) setState((s) => ({ ...s, models: r.models, files: r.files }));
  }, []);

  useEffect(() => {
    void (async () => {
      const init = (await api('/api/init')) as Init;
      if (!init.ok) return;
      setState((s) => ({ ...s, init }));
      await openProject(init.default_dir);
    })();
  }, [openProject]);

  const runJob = useCallback(async (kind: string, params: Json, outer?: JobView['outer']): Promise<JobStatus> => {
    const r = await api('/api/job/start', { dir: ref.current.dir, kind, params });
    if (!r.ok) { log(T('status.error', { m: r.error })); return 'failed'; }
    const id = r.id as number;
    const startedAt = Date.now();
    setJobs((js) => [...js.filter((j) => j.status === 'running' || j.status === 'cancelling').filter((j) => j.id !== id), { id, kind, status: 'running', progress: null, message: '', stage: null, elapsedSeconds: 0, etaSeconds: null, keepable: false, kept: null, cancellable: true, lines: [], startedAt, outer }]);
    for (;;) {
      await new Promise((ok) => setTimeout(ok, 600));
      const j = await api(`/api/job/${id}?from=0`);
      if (!j.ok) continue;
      setJobs((js) => js.map((x) => x.id === id ? { ...x, ...j, id, kind, startedAt, outer, message: j.message ?? '', lines: j.lines ?? [] } as JobView : x));
      if (j.status !== 'running' && j.status !== 'cancelling') {
        await refresh();
        return j.status as JobStatus;
      }
    }
  }, [log, refresh]);
  const cancelJob = useCallback(async (id: number, keep: boolean) => {
    await api(`/api/job/${id}/cancel`, { keep });
  }, []);

  const toggleHints = useCallback(() => setHints((h) => {
    try { localStorage.setItem('reco-minimap-hints', h ? 'off' : 'on'); } catch { /* private window */ }
    return !h;
  }), []);

  // without a choice the general model of Reco camera is used (once it is downloaded)
  const activeModel = () => state.models.find((m) => m.id === (state.project.model_id || state.init?.camera?.id)) ?? null;
  const sport = () => state.init!.sports.find((s) => s.id === state.project.sport) ?? state.init!.sports[0];
  const isKeeperSport = () => !!state.init?.sports.find((s) => s.id === state.project.sport)?.keepers;
  const common = () => {
    const p = state.project;
    return { ep: p.run?.ep || 'auto', units: p.run?.units, workers: Number(p.run?.workers) || 1, model_dir: activeModel()?.dir ?? '', video: p.video, numbers: p.run?.numbers, scenes: p.run?.scenes, figures: p.figures, direction: p.direction };
  };

  const value: Ctx = { ...state, lang, setLanguage, t: TEXT[lang], hints, toggleHints, set, mutate, openProject, refresh, log, logLines, jobs, runJob, cancelJob, activeModel, sport, common, isKeeperSport };
  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>;
}

// ---------------------------------------------------------------- widgets (all bound to a project path)
export function Card({ title, children, hint }: { title?: string; hint?: string; children: React.ReactNode }) {
  return <section className="mm-card">{title && <h2>{title}</h2>}{hint && <p className="mm-hint">{hint}</p>}{children}</section>;
}

export function Btn({ hint, primary, children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement> & { hint: string; primary?: boolean }) {
  return <button type="button" data-hint={hint} className={primary ? 'primary-button' : 'secondary-button'} {...rest}>{children}</button>;
}

export function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return <label className="mm-field"><span>{label}</span>{children}</label>;
}

export function Num({ k, label, step, min, max }: { k: string; label: string; step?: number; min?: number; max?: number }) {
  const { project, set } = useApp();
  const v = getPath(project, k) as number | undefined;
  return <Labeled label={label}><input type="number" step={step} min={min} max={max} value={v ?? ''} onChange={(e) => set(k, e.target.value === '' ? '' : parseFloat(e.target.value))} /></Labeled>;
}

export function Text({ k, label, placeholder, password }: { k: string; label: string; placeholder?: string; password?: boolean }) {
  const { project, set } = useApp();
  return <Labeled label={label}><input type={password ? 'password' : 'text'} autoComplete="off" placeholder={placeholder} value={(getPath(project, k) as string) ?? ''} onChange={(e) => set(k, e.target.value)} /></Labeled>;
}

export function Check({ k, label, hint }: { k: string; label: string; hint?: string }) {
  const { project, set } = useApp();
  return <label className="mm-check" data-hint={hint}><input type="checkbox" checked={!!getPath(project, k)} onChange={(e) => set(k, e.target.checked)} />{label}</label>;
}

export function Sel({ k, label, options, numeric }: { k: string; label: string; options: [string | number, string][]; numeric?: boolean }) {
  const { project, set } = useApp();
  return <Labeled label={label}><select value={String(getPath(project, k) ?? '')} onChange={(e) => set(k, numeric ? Number(e.target.value) : e.target.value)}>{options.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select></Labeled>;
}

export function Color({ k, label }: { k: string; label: string }) {
  const { project, set } = useApp();
  return <Labeled label={label}><input type="color" value={(getPath(project, k) as string) ?? '#000000'} onChange={(e) => set(k, e.target.value)} /></Labeled>;
}

/** Text field with a "Browse …" button that opens the native file dialog. */
export function PathField({ value, onChange, kind = 'file', prompt }: { value: string; onChange: (v: string) => void; kind?: 'file' | 'folder'; prompt: string }) {
  const { t } = useApp();
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return <div className="mm-row">
    <input type="text" className="mm-grow" value={v} onChange={(e) => setV(e.target.value)} onBlur={() => v !== value && onChange(v)} onKeyDown={(e) => { if (e.key === 'Enter') onChange(v); }} />
    <Btn hint={t.hints.browse} onClick={async () => { const r = await api('/api/dialog', { kind, prompt }); if (r.path) onChange(r.path); }}>{T('btn.browse')}</Btn>
  </div>;
}

export async function showFile(path: string, open: boolean, log: (l: string) => void) {
  const r = await api('/api/reveal', { path, open });
  if (!r.ok) log(T('status.error', { m: r.error }));
}

export function clock(t: number) {
  t = Math.max(0, Math.round(+t || 0));
  const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = String(t % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}
export function secs(v: string) {
  const p = String(v).trim().split(':').map((x) => parseFloat(x.replace(',', '.')) || 0);
  return p.length > 3 ? 0 : p.reduce((a, x) => a * 60 + x, 0);
}
