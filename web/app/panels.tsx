import { useEffect, useState } from 'react';
import { api, Btn, Card, Check, clock, Color, Labeled, Num, PathField, secs, Sel, showFile, Text, useApp, type Json } from './core';

const ALIAS: Record<string, string> = { soccer: 'football', fussball: 'football', 'fußball': 'football', futbol: 'football', 'fútbol': 'football', football11: 'football', field_hockey: 'hockey', 'field-hockey': 'hockey', fieldhockey: 'hockey', feldhockey: 'hockey', basket: 'basketball', hand: 'handball', volley: 'volleyball' };
export function useSportId() {
  const { init } = useApp();
  return (x?: string) => {
    if (!x) return null;
    const k = String(x).toLowerCase().trim();
    const id = ALIAS[k] || k;
    return init?.sports.some((s) => s.id === id) ? id : null;
  };
}

function Warn({ children }: { children: React.ReactNode }) {
  return <Card><div className="bad">{children}</div></Card>;
}

// ---------------------------------------------------------------- 1 · sport & model
export function SetupPanel() {
  const { project, models, mutate, set, runJob, common, activeModel, refresh, log, t, dir } = useApp();
  const sportId = useSportId();
  const [path, setPath] = useState('');
  const sp = useApp().sport();
  const m = activeModel();
  const mismatch = m && m.sport && sportId(m.sport) !== project.sport;
  const doImport = async (p: string) => {
    p = (p || '').trim();
    if (!p) { log(T('model.need_path')); return; }
    const r = await api('/api/model/import', { dir, path: p });
    if (!r.ok) { log(T('status.error', { m: r.error })); return; }
    log(T('model.imported', { id: r.model.id }));
    mutate((pr) => { pr.model_id = r.model.id; const s = sportId(r.model.sport); if (s) pr.sport = s; });
    await refresh();
  };
  return <>
    <Card title={T('sport.label')}>
      {project.sport === 'custom'
        ? <><div className="mm-grid2"><Num k="custom.length" label={T('sport.length')} /><Num k="custom.width" label={T('sport.width')} /><Num k="custom.team_size" label={T('sport.team')} /><Num k="custom.referees" label={T('sport.refs')} /></div><p className="mm-hint">{T('sport.custom_hint')}</p></>
        : <p className="mm-hint">{sp.field[0]} × {sp.field[1]} m · {T('sport.team')}: {sp.team_size} · {T('sport.refs')}: {sp.referees}<br />{T('sport.note')}</p>}
    </Card>
    <Card title={T('model.title')}>
      {models.length ? models.map((x, mi) => <div key={`${x.id}-${mi}`} className={`mm-model${x.id === project.model_id ? ' sel' : ''}`}>
        <input type="radio" name="mdl" checked={x.id === project.model_id} onChange={() => set('model_id', x.id)} data-hint={t.hints.useModel} />
        <div><b>{x.id}</b> {x.sport && <span className="mm-chip">{sportId(x.sport) ? T('sport.' + sportId(x.sport)) : x.sport}</span>}<br />
          <span className="mm-muted small">{T('model.classes')}: {x.classes.join(', ')} · {T('model.input')}: {x.input_size}×{x.input_size} · {x.size_mb} MB · <span className={x.sha === 'ok' ? 'ok' : x.sha === 'bad' ? 'bad' : ''}>{T('model.sha_' + x.sha)}</span></span></div>
        <span>{x.id === project.model_id ? <span className="ok">{T('model.active')}</span> : <Btn hint={t.hints.useModel} onClick={() => mutate((p) => { p.model_id = x.id; const s = sportId(x.sport); if (s) p.sport = s; })}>{T('model.use')}</Btn>}</span>
      </div>) : <p className="mm-muted">{T('model.none')}</p>}
      <div className="mm-row mm-gap"><Btn hint={t.hints.testModel} disabled={!m || !project.video} onClick={() => void runJob('selfcheck', common())}>{T('model.test')}</Btn></div>
      <p className="mm-hint">{T('model.test_hint')}</p>
      {mismatch && <p className="bad small">{T('model.mismatch', { m: T('sport.' + sportId(m!.sport)), s: T('sport.' + project.sport) })}</p>}
      <h3>{T('model.import')}</h3><p className="mm-hint">{T('model.import_hint')}</p>
      <Labeled label={T('model.path')}><div className="mm-row"><input type="text" className="mm-grow" value={path} onChange={(e) => setPath(e.target.value)} /><Btn primary hint={t.hints.importPath} onClick={() => void doImport(path)}>{T('model.import_path')}</Btn></div></Labeled>
      <div className="mm-row mm-gap">
        <Btn hint={t.hints.importFolder} onClick={async () => { const r = await api('/api/dialog', { kind: 'folder', prompt: T('model.import') }); if (r.path) { setPath(r.path); void doImport(r.path); } }}>📁 {T('model.import_folder')}</Btn>
        <Btn hint={t.hints.importFile} onClick={async () => { const r = await api('/api/dialog', { kind: 'file', prompt: '.onnx' }); if (r.path) { setPath(r.path); void doImport(r.path); } }}>📄 {T('model.import_file')}</Btn>
      </div>
    </Card>
  </>;
}

// ---------------------------------------------------------------- 2 · video
export function VideoPanel() {
  const { project, set } = useApp();
  const [probe, setProbe] = useState<Json | null>(null);
  useEffect(() => {
    if (!project.video) { setProbe(null); return; }
    void api('/api/probe', { path: project.video }).then((r) => setProbe(r.ok ? r : { error: true }));
  }, [project.video]);
  let info: React.ReactNode = null;
  if (probe && !probe.error) {
    const tot = Math.round(probe.duration || 0);
    info = <div className="ok">{T('video.info', { w: probe.w, h: probe.h, fps: probe.fps, d: tot ? `${Math.floor(tot / 60)}:${String(tot % 60).padStart(2, '0')} min` : '' })}</div>;
  } else if (probe?.error) info = <div className="bad">{T('video.err')}</div>;
  return <Card title={T('video.title')}>
    <Labeled label={T('video.path')}><PathField value={project.video ?? ''} prompt={T('video.path')} onChange={(v) => set('video', v)} /></Labeled>
    <p className="mm-hint">{T('video.hint')}</p>
    <div className="mm-gap">{info}</div>
  </Card>;
}

// ---------------------------------------------------------------- 3 · calibration
export function CalibPanel() {
  const { project, files, runJob, common, dir, refresh, isKeeperSport, activeModel, t, lang } = useApp();
  const ready = activeModel() && project.video;
  const tm = files.teams ?? {};
  const [gkOn, setGkOn] = useState(!!(tm.home_gk || tm.away_gk));
  useEffect(() => setGkOn(!!(files.teams?.home_gk || files.teams?.away_gk)), [files.teams]);
  if (!ready) return <Warn>{T('cal.need_setup')}</Warn>;
  const c = files.court, sc = files.scales;
  const home = tm.home ?? '#cccccc', away = tm.away ?? '#2f354e', hgk = tm.home_gk || '#22aa44', agk = tm.away_gk || '#ffcc00';
  const saveTeams = async (h: string, a: string, hg: string, ag: string, on = gkOn) => {
    await api('/api/teams/save', { dir, home: h, away: a, home_gk: on ? hg : '', away_gk: on ? ag : '' });
    await refresh();
  };
  return <>
    <Card title={T('cal.field')} hint={T('cal.field_hint')}>
      <div className="mm-row mm-gap"><span className="mm-muted">{T('cal.time')}</span><div style={{ width: 90 }}><Num k="calib_time" label="" /></div>
        <Btn primary hint={t.hints.calibrate} onClick={() => void runJob('calibrate', { ...common(), sport: project.sport, time: project.calib_time, lang, length: project.custom.length, width: project.custom.width, team_size: project.sport === 'custom' ? project.custom.team_size : '', referees: project.sport === 'custom' ? project.custom.referees : '' })}>{T('cal.field_btn')}</Btn>
        <span className={c ? 'ok' : 'mm-muted'}>{c ? T('cal.field_state', { n: c.points, rms: (+c.rms).toFixed(2) }) : T('cal.field_none')}</span></div>
    </Card>
    <Card title={T('cal.scales')} hint={T('cal.scales_hint')}>
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.scales} disabled={!c} onClick={() => void runJob('scales', common())}>{T('cal.scales_btn')}</Btn><span className={sc ? 'ok' : 'mm-muted'}>{sc ? T('cal.scales_state', { n: sc.n }) : T('cal.scales_none')}</span></div>
    </Card>
    <Card title={T('cal.colors')} hint={T('cal.colors_hint')}>
      <div className="mm-row mm-gap mm-wrap">
        <Btn primary hint={t.hints.colors} disabled={!c} onClick={() => void runJob('colors', common())}>{T('cal.colors_btn')}</Btn>
        <span className="mm-muted">{T('cal.home')}</span><input type="color" value={home} onChange={(e) => void saveTeams(e.target.value, away, hgk, agk)} />
        <span className="mm-muted">{T('cal.away')}</span><input type="color" value={away} onChange={(e) => void saveTeams(home, e.target.value, hgk, agk)} />
        <Btn hint={t.hints.swap} onClick={() => void saveTeams(away, home, agk, hgk)}>{T('cal.swap')}</Btn>
      </div>
      {isKeeperSport() && <><div className="mm-row mm-gap mm-wrap">
        <label className="mm-check"><input type="checkbox" checked={gkOn} onChange={(e) => { setGkOn(e.target.checked); void saveTeams(home, away, hgk, agk, e.target.checked); }} />{T('cal.gk_on')}</label>
        <span className="mm-muted">{T('cal.gk_home')}</span><input type="color" value={hgk} onChange={(e) => void saveTeams(home, away, e.target.value, agk)} />
        <span className="mm-muted">{T('cal.gk_away')}</span><input type="color" value={agk} onChange={(e) => void saveTeams(home, away, hgk, e.target.value)} />
        <Btn hint={t.hints.swapKeepers} onClick={() => void saveTeams(home, away, agk, hgk)}>{T('cal.gk_swap')}</Btn>
      </div><p className="mm-hint">{T('cal.gk_hint')}</p></>}
      <div className="mm-grid3 mm-gap"><Color k="colors.draw_home" label={T('cal.draw_home')} /><Color k="colors.draw_away" label={T('cal.draw_away')} /><Color k="colors.ref_color" label={T('cal.referee')} /></div>
      <h3>{T('cal.figures')}</h3><p className="mm-hint">{T('cal.torso_hint')}</p>
      <div className="mm-grid3"><Num k="figures.torso_top" label={T('cal.torso_top')} step={0.05} min={0} max={0.95} /><Num k="figures.torso_bottom" label={T('cal.torso_bottom')} step={0.05} min={0.05} max={1} /></div>
      <Check k="figures.shirt_match" label={T('cal.shirt_match')} /><p className="mm-hint">{T('cal.shirt_match_hint')}</p>
      <Check k="figures.no_referees" label={T('cal.no_referees')} />
    </Card>
  </>;
}

// ---------------------------------------------------------------- shared: workers and jersey numbers
function Workers() {
  return <Sel k="run.workers" label={T('run.workers')} numeric options={[1, 2, 3, 4].map((n) => [n, String(n)])} />;
}

function Numbers({ withHires }: { withHires: boolean }) {
  const { project, init, set, runJob, isKeeperSport, t } = useApp();
  const n = project.run.numbers;
  const ok = init?.ocr?.exists;
  return <Card title={T('run.numbers')}>
    <Check k="run.numbers.enabled" label={T('run.numbers')} /><p className="mm-hint">{T('run.numbers_hint')}</p>
    <div className="mm-row mm-gap"><span className={ok ? 'ok' : 'bad'}>{ok ? T('run.ocr_ok') : T('run.ocr_missing')}</span>{!ok && <Btn hint={t.hints.fetchOcr} onClick={() => void runJob('fetch_ocr', {}).then(() => location.reload())}>{T('run.ocr_fetch')}</Btn>}</div>
    <div className="mm-grid2"><Text k="run.numbers.roster_home" label={T('run.roster_home')} /><Text k="run.numbers.roster_away" label={T('run.roster_away')} /></div>
    <p className="mm-hint">{T('run.roster_hint')}{isKeeperSport() ? ' ' + T('run.roster_hint_gk') : ''}</p>
    {withHires && <><Labeled label={T('run.hires')}><PathField value={n.hires ?? ''} prompt={T('run.hires')} onChange={(v) => set('run.numbers.hires', v)} /></Labeled><p className="mm-hint">{T('run.hires_hint')}</p></>}
  </Card>;
}

// ---------------------------------------------------------------- 4 · processing
export function RunPanel() {
  const { project, files, mutate, runJob, common, activeModel, dir, log, t } = useApp();
  const [busy, setBusy] = useState(false);
  if (!activeModel() || !project.video || !files.court || !files.teams) return <Warn>{!activeModel() || !project.video ? T('cal.need_setup') : T('run.need_cal')}</Warn>;
  const r = project.run, seg: Json[] = project.segments;
  const setSeg = (i: number, k: string, v: string) => mutate((p) => {
    const g = p.segments[i];
    if (k === 'name') { g.name = v; return; }
    const end = g.duration > 0 ? g.start + g.duration : 0, x = secs(v);
    if (k === 'start') { g.start = x; g.duration = end > x ? end - x : 0; } else g.duration = x > g.start ? x - g.start : 0;
  });
  const start = async () => {
    setBusy(true);
    const list = seg.slice(), c = project.colors;
    for (let i = 0; i < list.length; i++) {
      const s = list[i];
      log(T('run.queue', { i: i + 1, n: list.length, name: s.name }));
      const st = await runJob('run', { ...common(), name: s.name, start: s.start, duration: s.duration, every: r.every, outputs: r.outputs, alpha_bg: r.alpha_bg, min_alpha: r.min_alpha, draw_home: c.draw_home, draw_away: c.draw_away, ref_color: c.ref_color }, { index: i + 1, count: list.length, label: s.name });
      if (st !== 'done') break;
    }
    setBusy(false);
  };
  const outs = files.out.slice(0, 8);
  const ep: [string, string][] = [['auto', T('run.ep_auto')], ['coreml', 'CoreML (macOS)'], ['cuda', 'CUDA (NVIDIA)'], ['tensorrt', 'TensorRT (NVIDIA)'], ['directml', 'DirectML (Windows)'], ['cpu', 'CPU']];
  return <>
    <Card title={T('run.segments')}>
      <table className="mm-table"><thead><tr><th>{T('run.name')}</th><th>{T('run.start')}</th><th>{T('run.end')}</th><th /></tr></thead><tbody>
        {seg.map((s, i) => <tr key={i}>
          <td><input type="text" defaultValue={s.name} onBlur={(e) => setSeg(i, 'name', e.target.value)} /></td>
          <td><input type="text" key={`s${s.start}`} defaultValue={clock(s.start)} placeholder="0:00" onBlur={(e) => setSeg(i, 'start', e.target.value)} /></td>
          <td><input type="text" key={`e${s.start + s.duration}`} defaultValue={s.duration > 0 ? clock(s.start + s.duration) : ''} placeholder={T('run.whole')} onBlur={(e) => setSeg(i, 'end', e.target.value)} /></td>
          <td><button type="button" className="tool-button" data-hint={t.hints.removeSegment} onClick={() => mutate((p) => { p.segments.splice(i, 1); if (!p.segments.length) p.segments.push({ name: T('run.segname') + '1', start: 0, duration: 0 }); })}>✕</button></td>
        </tr>)}
      </tbody></table>
      <div className="mm-gap"><Btn hint={t.hints.addSegment} onClick={() => mutate((p) => { p.segments.push({ name: T('run.segname') + (p.segments.length + 1), start: 0, duration: 600 }); })}>+ {T('run.add')}</Btn></div>
    </Card>
    <Card title={T('run.outputs')}>
      <Check k="run.outputs.minimap" label={T('run.out_minimap')} /><Check k="run.outputs.alpha" label={T('run.out_alpha')} /><Check k="run.outputs.pip" label={T('run.out_pip')} /><Check k="run.outputs.json" label={T('run.out_json')} />
      <Check k="run.scenes" label={T('run.scenes')} /><p className="mm-hint">{T('run.scenes_hint')}</p>
      <div className="mm-grid3"><Num k="run.every" label={T('run.every')} min={1} /><Num k="run.alpha_bg" label={T('run.alpha_bg')} step={0.1} /><Num k="run.min_alpha" label={T('run.min_alpha')} step={0.1} /></div>
      <p className="mm-hint">{T('run.every_hint')}</p>
      <div className="mm-grid3"><Sel k="run.ep" label={T('run.ep')} options={ep} /><Sel k="run.units" label="CoreML" options={['all', 'gpu', 'ane', 'cpu'].map((u) => [u, u])} /><Workers /></div>
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.run} disabled={busy} onClick={() => void start()}>{T('run.start_btn')}</Btn>
        <span className="mm-muted small">{T('run.output_dir')}: <button type="button" className="mm-link" data-hint={t.hints.showFolder} onClick={() => void showFile(`${dir}/out`, false, log)}>{dir}/out</button></span></div>
    </Card>
    <Numbers withHires />
    {outs.length > 0 && <Card><table className="mm-table"><tbody>{outs.map((x) => <tr key={x.path}>
      <td><button type="button" className="mm-link" data-hint={t.hints.openFile} onClick={() => void showFile(x.path, true, log)}>{x.name}</button></td><td className="mm-muted">{x.mb} MB</td>
      <td><Btn hint={t.hints.reveal} onClick={() => void showFile(x.path, false, log)}>{T('btn.reveal')}</Btn></td></tr>)}</tbody></table></Card>}
  </>;
}

// ---------------------------------------------------------------- 5 · live
export function LivePanel() {
  const { project, files, set, runJob, common, activeModel, jobs, init, cancelJob, t } = useApp();
  if (!activeModel() || !files.court || !files.teams) return <Warn>{T('run.need_cal')}</Warn>;
  const l = project.live;
  const job = [...jobs].reverse().find((j) => j.kind === 'live' && (j.status === 'running' || j.status === 'cancelling'));
  const url = job ? `http://127.0.0.1:${l.port}/` : '';
  const turl = url ? `http://${l.network && init?.lan_ip ? init.lan_ip : '127.0.0.1'}:${l.port}/ticker` : '';
  const start = () => {
    const c = project.colors;
    void runJob('live', { ...common(), ticker_names: l.ticker_names, home_name: l.home_name, away_name: l.away_name, network: l.network === true, ticker_approve: l.ticker_approve === true, ticker_export: l.ticker_export === true, ticker_push: l.ticker_push, ticker_key: l.ticker_key, source: l.source, size: l.size, fps: l.fps, port: l.port, realtime: l.realtime, adaptive: l.adaptive === true, draw_home: c.draw_home, draw_away: c.draw_away, ref_color: c.ref_color });
  };
  return <>
    <Card title={T('live.title')} hint={T('live.hint')}>
      <Labeled label={T('live.source')}><PathField value={l.source ?? ''} prompt={T('live.source')} onChange={(v) => set('live.source', v)} /></Labeled>
      <div className="mm-grid3"><Text k="live.size" label={T('live.size')} /><Num k="live.fps" label={T('live.fps')} /><Num k="live.port" label={T('live.port')} /></div>
      <Workers />
      <Check k="live.realtime" label={T('live.realtime')} /><Check k="live.adaptive" label={T('live.adaptive')} />
      <h3>{T('ticker.title')}</h3><p className="mm-hint">{T('ticker.ui_hint')}</p>
      <div className="mm-grid3"><Sel k="live.ticker_names" label={T('ticker.ui_show')} options={['numbers', 'names', 'both'].map((m) => [m, T('ticker.m_' + m)])} /><Text k="live.home_name" label={T('ticker.ui_home')} /><Text k="live.away_name" label={T('ticker.ui_away')} /></div>
      <Check k="live.ticker_approve" label={T('ticker.ui_approve')} /><Check k="live.ticker_export" label={T('ticker.ui_export')} />
      <div className="mm-grid2"><Text k="live.ticker_push" label={T('ticker.ui_push')} placeholder="https://…/ticker.json" /><Text k="live.ticker_key" label={T('ticker.ui_key')} password /></div>
      <p className="mm-hint">{T('ticker.ui_web_hint')}</p>
      <Check k="live.network" label={T('ticker.ui_network')} />
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.liveStart} disabled={!!url} onClick={start}>{T('live.start')}</Btn><Btn hint={t.hints.liveStop} disabled={!url} onClick={() => job && void cancelJob(job.id, true)}>{T('live.stop')}</Btn></div>
      {url && <><h3>{T('live.url')}</h3><a href={url} target="_blank" rel="noreferrer">{url}</a> · <a href={`${url}?floor=1&debug=1`} target="_blank" rel="noreferrer">test</a><p className="mm-hint">{T('live.obs')}</p>
        <h3>{T('ticker.link')}</h3><a href={turl} target="_blank" rel="noreferrer">{turl}</a>{l.network && !init?.lan_ip && <p className="bad small">{T('ticker.ui_noip')}</p>}
        {l.ticker_approve && <><h3>{T('ticker.ui_admin')}</h3><a href={`${url}ticker/admin`} target="_blank" rel="noreferrer">{url}ticker/admin</a></>}</>}
    </Card>
    <Numbers withHires={false} />
  </>;
}

// ---------------------------------------------------------------- 6 · evaluation (run report, tactics board)
export function EvalPanel() {
  const { project, files, mutate, runJob, log, t } = useApp();
  const outs = files.out.filter((x) => x.name.endsWith('.jsonl'));
  const [file, setFile] = useState(outs[0]?.path ?? '');
  useEffect(() => { if (!outs.some((o) => o.path === file)) setFile(outs[0]?.path ?? ''); }, [files.out]); // eslint-disable-line react-hooks/exhaustive-deps
  const [at, setAt] = useState<string>(project.board_at ?? '');
  const [trail, setTrail] = useState<number>(project.board_trail ?? 5);
  return <>
    <Card title={T('eval.title')}>
      <Labeled label={T('eval.file')}><select value={file} onChange={(e) => setFile(e.target.value)}>{outs.map((x) => <option key={x.path} value={x.path}>{x.name}</option>)}</select></Labeled>
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.evalRun} disabled={!outs.length} onClick={() => void runJob('stats', { file })}>{T('eval.btn')}</Btn><Btn hint={t.hints.reveal} disabled={!outs.length} onClick={() => void showFile(file, false, log)}>{T('btn.reveal')}</Btn></div>
    </Card>
    <Card title={T('board.title')} hint={T('board.ui_hint')}>
      <div className="mm-row mm-gap"><span className="mm-muted">{T('board.ui_at')}</span><input type="text" value={at} placeholder="12:30" style={{ width: 90 }} onChange={(e) => setAt(e.target.value)} />
        <span className="mm-muted">{T('board.ui_trail')}</span><input type="number" min={0} value={trail} style={{ width: 70 }} onChange={(e) => setTrail(+e.target.value || 0)} />
        <Btn primary hint={t.hints.board} disabled={!outs.length || !files.court} onClick={() => { const c = project.colors; mutate((p) => { p.board_at = at.trim(); p.board_trail = trail || 5; }); void runJob('board', { file, at: at.trim(), trail: trail || 5, draw_home: c.draw_home, draw_away: c.draw_away }); }}>{T('board.ui_open')}</Btn></div>
    </Card>
  </>;
}
