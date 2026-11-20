import { QrButton, QrDialog, ReachCheck } from './Qr';
import { OverlayCard, RenderCard } from './overlay';
import { useEffect, useState } from 'react';
import { api, Btn, Card, chooseNative, Check, clock, Color, Labeled, Num, PathField, secs, Sel, showFile, Text, useApp, type Json, type OutFile } from './core';

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
// Reco Trainer models for the chosen sport: imported ones and what the public model hub lists.
function TrainerModels() {
  const { project, models, mutate, activeModel, t } = useApp();
  const sportId = useSportId();
  const tm = t.models;
  const [hub, setHub] = useState<Json | null>(null);
  useEffect(() => {
    let stop = false;
    setHub(null);
    void api('/api/hub', { sport: project.sport }).then((r) => { if (!stop) setHub(r); });
    return () => { stop = true; };
  }, [project.sport]);
  const mine = models.filter((x) => !x.builtin && sportId(x.sport) === project.sport);
  const m = activeModel();
  const sportName = T('sport.' + project.sport);
  return <div className="mm-trainer">
    <h3>{tm.trainerTitle(sportName)}</h3>
    <p className="mm-hint">{mine.length ? tm.imported(mine.length) : tm.noneImported}{' '}
      {!hub ? <span className="mm-muted">{tm.hubChecking}</span> : !hub.ok ? <span className="mm-muted">{tm.hubOffline}</span> : hub.models.length ? <>{tm.hub(hub.models.length)} {hub.models.map((x: Json) => <span key={x.id} className="mm-chip">{x.name || x.id}{x.size ? ` · ${x.size}` : ''}</span>)}</> : tm.hubNone}
    </p>
    {m?.builtin && mine.length > 0 && <div className="mm-row mm-wrap"><span className="mm-hint">{tm.better}</span>
      <Btn primary hint={t.hints.useTrainer} onClick={() => mutate((p) => { p.model_id = mine[0].id; })}>{tm.useIt(mine[0].id)}</Btn></div>}
    {!mine.length && <p className="mm-hint">{hub?.ok && hub.models.length ? tm.hubHow : tm.trainNew}</p>}
  </div>;
}

export function SetupPanel() {
  const { project, models, mutate, set, runJob, common, activeModel, refresh, log, t, dir, init } = useApp();
  const tm = t.models;
  const camera = init?.camera;
  const hasCamera = models.some((x) => x.builtin);
  const chosen = project.model_id || camera?.id;
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
    <Card title={tm.cardTitle}>
      {!hasCamera && camera && <div className={`mm-model${chosen === camera.id ? ' sel' : ''}`}>
        <span />
        <div><b>{tm.camera}</b> <span className="mm-chip">{tm.standard}</span><span className="mm-chip">{tm.allSports}</span><br />
          <span className="mm-muted small">{tm.cameraText}</span><br /><span className="small">{tm.cameraMissing(camera.mb)}</span></div>
        <span><Btn primary hint={t.hints.fetchCamera} onClick={() => void runJob('fetch_model', {})}>{tm.fetch}</Btn></span>
      </div>}
      {models.map((x, mi) => <div key={`${x.id}-${mi}`} className={`mm-model${x.id === chosen ? ' sel' : ''}`}>
        <input type="radio" name="mdl" checked={x.id === chosen} onChange={() => set('model_id', x.id)} data-hint={t.hints.useModel} />
        <div><b>{x.builtin ? tm.camera : x.id}</b> {x.builtin ? <><span className="mm-chip">{tm.standard}</span><span className="mm-chip">{tm.allSports}</span></> : x.sport && <span className="mm-chip">{sportId(x.sport) ? T('sport.' + sportId(x.sport)) : x.sport}</span>}<br />
          {x.builtin && <><span className="mm-muted small">{tm.cameraText}</span><br /></>}
          <span className="mm-muted small">{x.builtin ? 'person, sports ball' : x.classes.join(', ')} · {T('model.input')}: {x.input_size}×{x.input_size} · {x.size_mb} MB · <span className={x.sha === 'ok' ? 'ok' : x.sha === 'bad' ? 'bad' : ''}>{T('model.sha_' + x.sha)}</span></span></div>
        <span>{x.id === chosen ? <span className="ok">{T('model.active')}</span> : <Btn hint={t.hints.useModel} onClick={() => mutate((p) => { p.model_id = x.id; const s = sportId(x.sport); if (s) p.sport = s; })}>{T('model.use')}</Btn>}</span>
      </div>)}
      {!models.length && !camera && <p className="mm-muted">{T('model.none')}</p>}
      <TrainerModels />
      <div className="mm-row mm-gap"><Btn hint={t.hints.testModel} disabled={!m || !project.video} onClick={() => void runJob('selfcheck', common())}>{T('model.test')}</Btn></div>
      <p className="mm-hint">{T('model.test_hint')}</p>
      {mismatch && <p className="bad small">{T('model.mismatch', { m: T('sport.' + sportId(m!.sport)), s: T('sport.' + project.sport) })}</p>}
      <h3>{T('model.import')}</h3><p className="mm-hint">{T('model.import_hint')}</p>
      <Labeled label={T('model.path')}><div className="mm-row"><input type="text" className="mm-grow" value={path} onChange={(e) => setPath(e.target.value)} /><Btn primary hint={t.hints.importPath} onClick={() => void doImport(path)}>{T('model.import_path')}</Btn></div></Labeled>
      <div className="mm-row mm-gap">
        <Btn hint={t.hints.importFolder} onClick={async () => { const p = await chooseNative('folder', T('model.import')); if (p) { setPath(p); void doImport(p); } }}>📁 {T('model.import_folder')}</Btn>
        <Btn hint={t.hints.importFile} onClick={async () => { const p = await chooseNative('file', '.onnx'); if (p) { setPath(p); void doImport(p); } }}>📄 {T('model.import_file')}</Btn>
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
  return <>
    <Card title={T('video.title')}>
      <Labeled label={T('video.path')}><PathField value={project.video ?? ''} prompt={T('video.path')} onChange={(v) => set('video', v)} /></Labeled>
      <p className="mm-hint">{T('video.hint')}</p>
      <div className="mm-gap">{info}</div>
    </Card>
    <TeamsCard />
  </>;
}

// the one place for team names and squads (numbers and player names); used by processing, live,
// ticker, statistics and videos
function TeamsCard() {
  const { isKeeperSport } = useApp();
  return <Card title={T('teams.title')} hint={T('teams.hint')}>
    <div className="mm-teams">
      <div><h3>{T('teams.home')}</h3><Text k="live.home_name" label={T('ticker.ui_home')} /><Text k="run.numbers.roster_home" label={T('run.roster_home')} /></div>
      <div><h3>{T('teams.away')}</h3><Text k="live.away_name" label={T('ticker.ui_away')} /><Text k="run.numbers.roster_away" label={T('run.roster_away')} /></div>
    </div>
    <p className="mm-hint">{T('run.roster_hint')}{isKeeperSport() ? ' ' + T('run.roster_hint_gk') : ''}</p>
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
    const gk = on && isKeeperSport(); // goalkeeper colours only exist in sports with goalkeepers
    await api('/api/teams/save', { dir, home: h, away: a, home_gk: gk ? hg : '', away_gk: gk ? ag : '' });
    await refresh();
  };
  return <>
    <Card title={T('cal.field')} hint={T('cal.field_hint')}>
      <div className="mm-row mm-gap"><span className="mm-muted">{T('cal.time')}</span><div style={{ width: 90 }}><Num k="calib_time" label="" /></div>
        <Btn primary hint={t.hints.calibrate} onClick={() => void runJob('calibrate', { ...common(), sport: project.sport, time: project.calib_time, lang, lens: project.lens || (project.fisheye ? 'fisheye' : ''), length: project.custom.length, width: project.custom.width, team_size: project.sport === 'custom' ? project.custom.team_size : '', referees: project.sport === 'custom' ? project.custom.referees : '' })}>{T('cal.field_btn')}</Btn>
        <span className={c ? 'ok' : 'mm-muted'}>{c ? T('cal.field_state', { n: c.points, rms: (+c.rms).toFixed(2) }) + (c.lens ? ' · ' + T(c.lens === 'panorama' ? 'calp.lens_panorama' : 'calp.lens_fisheye') : '') : T('cal.field_none')}</span></div>
      <div style={{ maxWidth: 360 }}><Sel k="lens" label={T('calp.lens_title')} options={[['', T('cal.lens_auto')], ['none', T('calp.lens_none')], ['fisheye', T('calp.lens_fisheye')], ['panorama', T('calp.lens_panorama')]]} /></div><p className="mm-hint">{T('cal.lens_hint')}</p>
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
      <Check k="figures.hide_referees" label={T('cal.hide_referees')} /><p className="mm-hint">{T('cal.hide_referees_hint')}</p>
      <Check k="figures.no_referees" label={T('cal.no_referees')} /><p className="mm-hint">{T('cal.no_referees_hint')}</p>
    </Card>
    {project.sport === 'basketball' && <Card title={T('sb.title')} hint={T('sb.hint')}>
      <div className="mm-row mm-gap"><span className="mm-muted">{T('sb.time')}</span><div style={{ width: 90 }}><Num k="scoreboard_time" label="" /></div>
        <Btn primary hint={t.hints.calibrateScoreboard} onClick={() => void runJob('scoreboard_calibrate', { ...common(), time: project.scoreboard_time ?? 60, lang })}>{T('sb.btn')}</Btn>
        <span className={files.scoreboard ? 'ok' : 'mm-muted'}>{files.scoreboard ? T('sb.state', { n: files.scoreboard.fields, side: T(files.scoreboard.home === 'right' ? 'sb.right' : 'sb.left') }) : T('sb.none')}</span></div>
      {files.scoreboard && (files.scoreboard.digits as number[] | undefined)?.some((d) => d < 2) && <p className="bad small">{t.stats.boardDigits((files.scoreboard.digits as number[])[0], (files.scoreboard.digits as number[])[1])}</p>}
      {files.scoreboard && files.scoreboard.period === false && <p className="mm-hint">{t.stats.boardNoPeriod}</p>}
    </Card>}
    {['basketball', 'football', 'futsal', 'handball', 'hockey', 'lacrosse'].includes(project.sport) && <Card title={t.direction.title} hint={t.direction.hint}>
      <div className="mm-grid2"><Sel k="direction.home" label={t.direction.home} options={[['', t.direction.auto], ['l', t.direction.left], ['r', t.direction.right]]} /><SwitchSel /></div>
      <p className="mm-hint">{t.direction.switchHint}</p>
    </Card>}
  </>;
}

// ---------------------------------------------------------------- shared: workers and jersey numbers
function Workers() {
  return <Sel k="run.workers" label={T('run.workers')} numeric options={[1, 2, 3, 4].map((n) => [n, String(n)])} />;
}

function Numbers({ withHires }: { withHires: boolean }) {
  const { project, init, set, runJob, t } = useApp();
  const n = project.run.numbers;
  const ok = init?.ocr?.exists;
  return <Card title={T('run.numbers')}>
    <Check k="run.numbers.enabled" label={T('run.numbers')} /><p className="mm-hint">{T('run.numbers_hint')}</p>
    <div className="mm-row mm-gap"><span className={ok ? 'ok' : 'bad'}>{ok ? T('run.ocr_ok') : T('run.ocr_missing')}</span>{!ok && <Btn hint={t.hints.fetchOcr} onClick={() => void runJob('fetch_ocr', {}).then(() => location.reload())}>{T('run.ocr_fetch')}</Btn>}</div>
    <p className="mm-hint">{T('run.roster_where')}</p>
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
      // the autotrack video is made from the result afterwards: it needs the .jsonl
      const outputs = r.outputs.track ? { ...r.outputs, json: true } : r.outputs;
      const st = await runJob('run', { ...common(), name: s.name, start: s.start, duration: s.duration, every: r.every, scoreboard: r.scoreboard !== false, adaptive: r.adaptive === true, distant: r.distant === true, outputs, audio: r.audio === true, ticker_names: project.live?.ticker_names, home_name: project.live?.home_name, away_name: project.live?.away_name, alpha_bg: r.alpha_bg, min_alpha: r.min_alpha, draw_home: c.draw_home, draw_away: c.draw_away, ref_color: c.ref_color }, { index: i + 1, count: list.length, label: s.name });
      if (st !== 'done') break;
      if (r.outputs.track) {
        const rt = await runJob('render', { ...common(), file: s.name, by_name: true, track: true, audio: r.audio === true, minimap: true, zoom: Number(project.render?.zoom) || 0.8, home_name: project.live?.home_name, away_name: project.live?.away_name, draw_home: c.draw_home, draw_away: c.draw_away }, { index: i + 1, count: list.length, label: s.name });
        if (rt !== 'done') break;
      }
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
      <Check k="run.outputs.minimap" label={T('run.out_minimap')} /><Check k="run.outputs.alpha" label={T('run.out_alpha')} /><Check k="run.outputs.pip" label={T('run.out_pip')} />{r.outputs.pip && <Check k="run.audio" label={t.videos.runAudio} />}<Check k="run.outputs.track" label={t.videos.runTrack} hint={t.videos.runTrackHint} /><Check k="run.outputs.json" label={T('run.out_json')} /><Check k="run.outputs.evidence" label={T('run.out_evidence')} />
      <Check k="run.outputs.ticker" label={t.tickerSim.runOption} /><p className="mm-hint">{t.tickerSim.runHint}</p>
      <p className="mm-hint">{T('run.out_evidence_hint')}</p>
      <Check k="run.scenes" label={T('run.scenes')} /><p className="mm-hint">{T('run.scenes_hint')}</p>
      {files.scoreboard && <><Check k="run.scoreboard" label={T('sb.use')} /><p className="mm-hint">{T('sb.use_hint')}</p></>}
      <div className="mm-grid3"><Num k="run.every" label={T('run.every')} min={1} /><Num k="run.alpha_bg" label={T('run.alpha_bg')} step={0.1} /><Num k="run.min_alpha" label={T('run.min_alpha')} step={0.1} /></div>
      <p className="mm-hint">{T('run.every_hint')}</p>
      <Check k="run.adaptive" label={T('run.adaptive')} /><Check k="run.distant" label={T('run.distant')} /><p className="mm-hint">{T('run.speed_hint')}</p>
      <div className="mm-grid3"><Sel k="run.ep" label={T('run.ep')} options={ep} /><Sel k="run.units" label="CoreML" options={['all', 'gpu', 'ane', 'cpu'].map((u) => [u, u])} /><Workers /></div>
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.run} disabled={busy} onClick={() => void start()}>{T('run.start_btn')}</Btn>
        <span className="mm-muted small">{T('run.output_dir')}: <button type="button" className="mm-link" data-hint={t.hints.showFolder} onClick={() => void showFile(`${dir}/out`, false, log)}>{dir}/out</button></span></div>
    </Card>
    <OverlayCard />
    <Numbers withHires />
    {outs.length > 0 && <Card><table className="mm-table"><tbody>{outs.map((x) => <tr key={x.path}>
      <td><button type="button" className="mm-link" data-hint={t.hints.openFile} onClick={() => void showFile(x.path, true, log)}>{x.name}</button></td><td className="mm-muted">{x.mb} MB</td>
      <td><Btn hint={t.hints.reveal} onClick={() => void showFile(x.path, false, log)}>{T('btn.reveal')}</Btn></td></tr>)}</tbody></table></Card>}
  </>;
}

// ---------------------------------------------------------------- 5 · live
/** A random key for the phone page (24 letters and digits, from the browser's secure random source). */
function newPickKey(): string {
  const abc = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const r = new Uint8Array(24);
  crypto.getRandomValues(r);
  return Array.from(r, (b) => abc[b % abc.length]).join('');
}

export function LivePanel() {
  const { project, files, set, runJob, common, activeModel, jobs, init, cancelJob, t, lang } = useApp();
  const [qrNew, setQrNew] = useState(false);
  if (!activeModel() || !files.court || !files.teams) return <Warn>{T('run.need_cal')}</Warn>;
  const l = project.live;
  const job = [...jobs].reverse().find((j) => j.kind === 'live' && (j.status === 'running' || j.status === 'cancelling'));
  const url = job ? `http://127.0.0.1:${l.port}/` : '';
  const turl = url ? `http://${l.network && init?.lan_ip ? init.lan_ip : '127.0.0.1'}:${l.port}/ticker` : '';
  const pickUrl = l.pick && l.pick_key ? `http://${l.network && init?.lan_ip ? init.lan_ip : '127.0.0.1'}:${l.port}/pick?key=${l.pick_key}&lang=${lang}` : '';
  const start = () => {
    const c = project.colors;
    // the phone page's secret: made once per project, renewed on request
    const pickKey = l.pick ? (l.pick_key || newPickKey()) : '';
    if (l.pick && !l.pick_key) set('live.pick_key', pickKey);
    void runJob('live', { ...common(), pick: l.pick === true, pick_key: pickKey, ticker_names: l.ticker_names, home_name: l.home_name, away_name: l.away_name, network: l.network === true, ticker_approve: l.ticker_approve === true, ticker_export: l.ticker_export === true, ticker_push: l.ticker_push, ticker_key: l.ticker_key, source: l.source, size: l.size, fps: l.fps, port: l.port, realtime: l.realtime, adaptive: l.adaptive === true, draw_home: c.draw_home, draw_away: c.draw_away, ref_color: c.ref_color });
  };
  return <>
    <Card title={T('live.title')} hint={T('live.hint')}>
      <Labeled label={T('live.source')}><PathField value={l.source ?? ''} prompt={T('live.source')} onChange={(v) => set('live.source', v)} /></Labeled>
      <div className="mm-grid3"><Text k="live.size" label={T('live.size')} /><Num k="live.fps" label={T('live.fps')} /><Num k="live.port" label={T('live.port')} /></div>
      <Workers />
      <Check k="live.realtime" label={T('live.realtime')} /><Check k="live.adaptive" label={T('live.adaptive')} />
      <h3>{T('ticker.title')}</h3><p className="mm-hint">{T('ticker.ui_hint')}</p>
      <div className="mm-grid3"><Sel k="live.ticker_names" label={T('ticker.ui_show')} options={['numbers', 'names', 'both'].map((m) => [m, T('ticker.m_' + m)])} /></div><p className="mm-hint">{T('run.roster_where')}</p>
      <Check k="live.ticker_approve" label={T('ticker.ui_approve')} /><Check k="live.ticker_export" label={T('ticker.ui_export')} />
      <div className="mm-grid2"><Text k="live.ticker_push" label={T('ticker.ui_push')} placeholder="https://…/ticker.json" /><Text k="live.ticker_key" label={T('ticker.ui_key')} password /></div>
      <p className="mm-hint">{T('ticker.ui_web_hint')}</p>
      <Check k="live.network" label={T('ticker.ui_network')} />
      {project.sport === 'basketball' && files.scoreboard && <>
        <h3>{T('pick.ui_on')}</h3><p className="mm-hint">{T('pick.ui_hint')}</p>
        <Check k="live.pick" label={T('pick.ui_on')} />
        {l.pick && !l.network && <p className="bad small">{T('pick.ui_need_net')}</p>}
        {l.pick && l.pick_key && <div className="mm-row mm-gap mm-wrap"><span className="mm-muted small mm-grow">{T('pick.ui_link')}</span>
          <Btn hint={T('pick.ui_new_hint')} onClick={() => { set('live.pick_key', newPickKey()); setQrNew(true); }}>{T('pick.ui_new')}</Btn></div>}
        {qrNew && l.pick_key && <QrDialog url={`http://${l.network && init?.lan_ip ? init.lan_ip : '127.0.0.1'}:${l.port}/pick?key=${l.pick_key}&lang=${lang}`} title={T('pick.ui_on')} note={url ? t.qr.nextStart : undefined} onClose={() => setQrNew(false)} />}
        {pickUrl && url && <div className="mm-row mm-gap mm-wrap"><a href={pickUrl} target="_blank" rel="noreferrer" className="mm-grow mm-break">{pickUrl}</a>
          <Btn hint={T('pick.ui_copy')} onClick={() => void navigator.clipboard?.writeText(pickUrl)}>{T('pick.ui_copy')}</Btn><QrButton url={pickUrl} title={T('pick.ui_on')} /></div>}
      </>}
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.liveStart} disabled={!!url} onClick={start}>{T('live.start')}</Btn><Btn hint={t.hints.liveStop} disabled={!url} onClick={() => job && void cancelJob(job.id, true)}>{T('live.stop')}</Btn></div>
      {url && <><h3>{T('live.url')}</h3><a href={url} target="_blank" rel="noreferrer">{url}</a> · <a href={`${url}?floor=1&debug=1`} target="_blank" rel="noreferrer">{T('live.test')}</a><p className="mm-hint">{T('live.obs')}</p>
        {(project.overlay?.scoreboard?.on || project.overlay?.watermark?.on) && <><h3>{t.overlay.obs}</h3><a href={`${url}scoreboard`} target="_blank" rel="noreferrer">{url}scoreboard</a> · <a href={`${url}scoreboard?floor=1`} target="_blank" rel="noreferrer">{T('live.test')}</a></>}
        <h3>{T('ticker.link')}</h3><div className="mm-row mm-gap mm-wrap"><a href={turl} target="_blank" rel="noreferrer" className="mm-grow mm-break">{turl}</a><QrButton url={turl} title={T('ticker.link')} /></div>{l.network && !init?.lan_ip && <p className="bad small">{T('ticker.ui_noip')}</p>}
        <ReachCheck port={Number(l.port) || 8765} network={l.network === true} />
        {l.ticker_approve && <><h3>{T('ticker.ui_admin')}</h3><a href={`${url}ticker/admin`} target="_blank" rel="noreferrer">{url}ticker/admin</a></>}</>}
    </Card>
    <OverlayCard />
    <Numbers withHires={false} />
  </>;
}

/** A finished analysis played back as if live: confirm made shots and shooters on the phone. */
function SimulateCard({ outs }: { outs: OutFile[] }) {
  const { project, set, runJob, cancelJob, common, jobs, init, t, lang } = useApp();
  const ts = t.sim;
  const results = outs.filter((o) => o.name.endsWith('.jsonl') && !o.name.endsWith('.dets.jsonl'));
  const [file, setFile] = useState(results[0]?.path ?? '');
  useEffect(() => { if (!results.some((o) => o.path === file)) setFile(results[0]?.path ?? ''); }, [outs]); // eslint-disable-line react-hooks/exhaustive-deps
  const s = project.simulate ?? {}, l = project.live ?? {}, c = project.colors ?? {};
  const port = Number(s.port) || 8766;
  const job = [...jobs].reverse().find((j) => j.kind === 'simulate' && (j.status === 'running' || j.status === 'cancelling'));
  const host = s.network && init?.lan_ip ? init.lan_ip : '127.0.0.1';
  const base = `http://127.0.0.1:${port}/`;
  const phone = l.pick_key ? `http://${host}:${port}/pick?key=${l.pick_key}&lang=${lang}` : '';
  const start = () => {
    const key = l.pick_key || newPickKey();
    if (!l.pick_key) set('live.pick_key', key);
    void runJob('simulate', { ...common(), file, port, speed: Number(s.speed) || 1, from: s.from ?? '', mode: s.mode || 'auto', pick_key: key, network: s.network === true, ticker_names: l.ticker_names, home_name: l.home_name, away_name: l.away_name, draw_home: c.draw_home, draw_away: c.draw_away });
  };
  return <Card title={ts.title} hint={ts.hint}>
    <Labeled label={ts.file}><select value={file} disabled={!!job} onChange={(e) => setFile(e.target.value)}>{results.map((x) => <option key={x.path} value={x.path}>{x.name}</option>)}</select></Labeled>
    <div className="mm-grid3">
      <Sel k="simulate.speed" label={ts.speed} options={Object.entries(ts.speeds)} />
      <Text k="simulate.from" label={ts.from} placeholder="0:00" />
      <Num k="simulate.port" label={T('live.port')} />
    </div>
    <div className="mm-grid2"><Sel k="simulate.mode" label={ts.mode} options={Object.entries(ts.modes)} /></div>
    <p className="mm-hint">{ts.modeHint}</p>
    <Check k="simulate.network" label={ts.network} />
    {!s.network && <p className="bad small">{ts.needNet}</p>}
    <div className="mm-row mm-gap"><Btn primary hint={ts.start} disabled={!file || !!job} onClick={start}>{ts.start}</Btn><Btn hint={ts.stop} disabled={!job} onClick={() => job && void cancelJob(job.id, true)}>{ts.stop}</Btn></div>
    {job && <>
      {phone && <><h3>{ts.phone}</h3><div className="mm-row mm-gap mm-wrap"><a href={phone} target="_blank" rel="noreferrer" className="mm-grow mm-break">{phone}</a><QrButton url={phone} title={ts.phone} /></div></>}
      {s.network && !init?.lan_ip && <p className="bad small">{T('ticker.ui_noip')}</p>}
      <ReachCheck port={port} network={s.network === true} />
      <p><a href={base} target="_blank" rel="noreferrer">{T('live.url')}</a>{(project.overlay?.scoreboard?.on || project.overlay?.watermark?.on) && <> · <a href={`${base}scoreboard?floor=1`} target="_blank" rel="noreferrer">{ts.overlay}</a></>} · <a href={`${base}ticker`} target="_blank" rel="noreferrer">{ts.ticker}</a></p>
    </>}
    <p className="mm-hint">{ts.saved}</p>
  </Card>;
}

// ---------------------------------------------------------------- 6 · evaluation (run report, tactics board)
export function EvalPanel() {
  const { project, dir, files, mutate, runJob, log, t } = useApp();
  const outs = files.out.filter((x) => x.name.endsWith('.jsonl') && !x.name.endsWith('.dets.jsonl'));
  const [file, setFile] = useState(outs[0]?.path ?? '');
  useEffect(() => { if (!outs.some((o) => o.path === file)) setFile(outs[0]?.path ?? ''); }, [files.out]); // eslint-disable-line react-hooks/exhaustive-deps
  const [at, setAt] = useState<string>(project.board_at ?? '');
  const [trail, setTrail] = useState<number>(project.board_trail ?? 5);
  // moments of the ticker (baskets, attempts, substitutions …): the board opens a few seconds before one
  const [moments, setMoments] = useState<Json[]>([]);
  const [moment, setMoment] = useState<string>('');
  const [before, setBefore] = useState<number>(project.board_before ?? 3);
  useEffect(() => {
    let stop = false;
    setMoments([]); setMoment('');
    if (file) void api('/api/board/moments', { dir, file }).then((r) => { if (!stop && r.ok) setMoments(r.moments as Json[]); });
    return () => { stop = true; };
  }, [file, dir]);
  const ts = t.stats;
  const names = { home: project.live?.home_name?.trim() || ts.home, away: project.live?.away_name?.trim() || ts.away } as Record<string, string>;
  const label = (m: Json) => {
    const who = [names[String(m.team)] ?? '', m.number != null ? `#${m.number}` : ''].filter(Boolean).join(' ');
    const what = m.type === 'score' ? (project.sport === 'basketball' ? ts.basketKinds[Number(m.points)] ?? '' : (ts.kinds as Record<string, string>)[String(m.kind)] ?? String(m.kind))
      : m.type === 'sub' ? ts.kinds.sub : (ts.kinds as Record<string, string>)[String(m.kind)] ?? String(m.kind);
    return `${clock(Number(m.t))} · ${what}${who ? ' · ' + who : ''}`;
  };
  const chosen = moment === '' ? undefined : moments[Number(moment)];
  const atOf = (m: Json) => clock(Math.max(0, Math.round(Number(m.t) - before)));
  const boardAt = chosen ? atOf(chosen) : at.trim();
  return <>
    <Card title={T('eval.title')}>
      <Labeled label={T('eval.file')}><select value={file} onChange={(e) => setFile(e.target.value)}>{outs.map((x) => <option key={x.path} value={x.path}>{x.name}</option>)}</select></Labeled>
      <div className="mm-row mm-gap"><Btn primary hint={t.hints.evalRun} disabled={!outs.length} onClick={() => void runJob('stats', { file })}>{T('eval.btn')}</Btn><Btn hint={t.hints.reveal} disabled={!outs.length} onClick={() => void showFile(file, false, log)}>{T('btn.reveal')}</Btn></div>
    </Card>
    <Card title={T('board.title')} hint={T('board.ui_hint')}>
      {moments.length > 0 && <div className="mm-row mm-gap"><span className="mm-muted">{T('board.ui_moment')}</span>
        <select value={moment} onChange={(e) => setMoment(e.target.value)}><option value="">{T('board.ui_own_time')}</option>{moments.map((m, i) => <option key={i} value={i}>{label(m)}</option>)}</select>
        {chosen && <><span className="mm-muted">{T('board.ui_before')}</span><input type="number" min={0} max={60} value={before} style={{ width: 60 }} onChange={(e) => setBefore(Math.max(0, +e.target.value || 0))} /><span className="mm-muted">→ {atOf(chosen)}</span></>}</div>}
      <div className="mm-row mm-gap">{!chosen && <><span className="mm-muted">{T('board.ui_at')}</span><input type="text" value={at} placeholder="12:30" style={{ width: 90 }} onChange={(e) => setAt(e.target.value)} /></>}
        <span className="mm-muted">{T('board.ui_trail')}</span><input type="number" min={0} value={trail} style={{ width: 70 }} onChange={(e) => setTrail(+e.target.value || 0)} />
        <Btn primary hint={t.hints.board} disabled={!outs.length || !files.court} onClick={() => { const c = project.colors; mutate((p) => { p.board_at = at.trim(); p.board_trail = trail || 5; p.board_before = before; }); void runJob('board', { file, at: boardAt, trail: trail || 5, draw_home: c.draw_home, draw_away: c.draw_away }); }}>{T('board.ui_open')}</Btn></div>
    </Card>
    {project.sport === 'basketball' && <SimulateCard outs={files.out} />}
    <RenderCard outs={files.out} />
  </>;
}

// change of ends: when a period starts (from the scoreboard) instead of a fixed video time
const SWITCH_PERIOD: Record<string, [number, 'switchQ3' | 'switchH2']> = { basketball: [3, 'switchQ3'], hockey: [3, 'switchQ3'], lacrosse: [3, 'switchQ3'], football: [2, 'switchH2'], futsal: [2, 'switchH2'], handball: [2, 'switchH2'] };
function SwitchSel() {
  const { project, set, t } = useApp();
  const v = String(project.direction?.switch ?? '');
  const [p, label] = SWITCH_PERIOD[project.sport] ?? [2, 'switchH2'];
  const period = `period:${p}`;
  const [timeMode, setTimeMode] = useState(() => v !== '' && !v.startsWith('period:'));
  const mode = v.startsWith('period:') ? 'period' : timeMode || v ? 'time' : '';
  return <>
    <Labeled label={t.direction.switch}><select value={mode} onChange={(e) => { const m = e.target.value; setTimeMode(m === 'time'); set('direction.switch', m === 'period' ? period : ''); }}>
      <option value="period">{t.direction[label]}</option>
      <option value="">{t.direction.switchNone}</option>
      <option value="time">{t.direction.switchTime}</option>
    </select></Labeled>
    {mode === 'time' && <Text k="direction.switch" label={t.direction.switchTimeLabel} placeholder="24:00" />}
  </>;
}
