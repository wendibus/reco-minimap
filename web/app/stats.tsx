import { useEffect, useMemo, useState } from 'react';
import { api, Btn, Card, clock, Labeled, useApp, type Json } from './core';

type Team = 'home' | 'away' | 'none';
type Attempt = { t: number; team: Team; x: number; y: number; kind: string; zone: string; goal: number; distance: number; confidence: number };
type Stats = Json & {
  sport: string; field: { length: number; width: number }; lines: [number, number][][]; goals: [number, number][];
  t0: number; t1: number; has_attempts: boolean; attempts: Attempt[];
  totals: { home: number; away: number; none: number };
  zones: { zone: string; count: number; home: number; away: number; share: number }[];
  periods: { index: number; from: number; to: number; home: number; away: number; none: number }[];
  possession: { home: number; away: number; frames_held: number; ball_seen: number };
  thirds: number[]; heat: { nx: number; ny: number; home: number[]; away: number[]; ball: number[] };
  shape?: Shape;
  moments: { t: number; type: 'attempt' | 'scene' | 'sub'; kind?: string; team: Team; zone?: string; out?: number | null; in?: number | null }[];
};

/** Which charts a sport gets (attempts only where the tracking can see them). */
function sections(sport: string) {
  const goalSport = ['football', 'futsal', 'handball', 'hockey', 'lacrosse'].includes(sport);
  const shots = sport === 'basketball' || goalSport;
  return { shots, zones: shots, periods: shots, possession: sport !== 'volleyball' && sport !== 'custom', thirds: goalSport || sport === 'rugby' || sport === 'american_football', heat: true };
}

type Spread = { w: number; d: number } | null;
type Side = { w: number; d: number; cx: number; cy: number; frames: number; slots: { id: number; x: number; y: number; share: number }[] } | null;
type Phase = 'all' | 'att' | 'def';
type Shape = {
  series: { t: number; home: Spread; away: Spread }[];
  sections: { index: number; from: number; to: number; home: Record<Phase, Side>; away: Record<Phase, Side> }[];
  home: Record<Phase, Spread>; away: Record<Phase, Spread>;
};

// ---------------------------------------------------------------- tooltip shared by all charts
type Tip = { x: number; y: number; text: string } | null;
let setTipGlobal: (t: Tip) => void = () => {};
const tipHandlers = (text: string) => ({
  onPointerEnter: (e: React.PointerEvent) => setTipGlobal({ x: e.clientX, y: e.clientY, text }),
  onPointerMove: (e: React.PointerEvent) => setTipGlobal({ x: e.clientX, y: e.clientY, text }),
  onPointerLeave: () => setTipGlobal(null),
});
function TipLayer() {
  const [tip, setTip] = useState<Tip>(null);
  setTipGlobal = setTip;
  if (!tip) return null;
  return <div className="hint-bubble mm-tip" role="tooltip" style={{ left: Math.min(tip.x + 12, window.innerWidth - 240), top: tip.y + 14 }}>{tip.text}</div>;
}

// ---------------------------------------------------------------- field drawing (lines come from the sport profile)
function Field({ s, children, pad = 1.2 }: { s: Stats; children?: React.ReactNode; pad?: number }) {
  const { length: l, width: w } = s.field;
  const sw = Math.max(l, w) / 380;
  return <svg className="mm-field-svg" viewBox={`${-pad} ${-pad} ${l + 2 * pad} ${w + 2 * pad}`} role="img">
    <rect x={-pad} y={-pad} width={l + 2 * pad} height={w + 2 * pad} className="mm-field-bg" />
    {children}
    <g className="mm-field-lines" strokeWidth={sw} fill="none">
      {s.lines.map((pl, i) => <polyline key={i} points={pl.map((p) => p.join(',')).join(' ')} />)}
    </g>
  </svg>;
}

function heatCells(s: Stats, values: number[], nx: number, ny: number, color: string, label: (v: number) => string) {
  const max = Math.max(1, ...values);
  const cw = s.field.length / nx, ch = s.field.width / ny;
  return values.map((v, i) => v > 0 && <rect key={i} x={(i % nx) * cw} y={Math.floor(i / nx) * ch} width={cw} height={ch} fill={color} fillOpacity={0.12 + 0.78 * (v / max)} {...tipHandlers(label(v))} />);
}

// ---------------------------------------------------------------- live ticker simulation of the chosen result
function TickerSimCard({ file }: { file: string }) {
  const { files, t, lang, project, runJob } = useApp();
  const tx = t.tickerSim;
  const path = file.replace(/\.jsonl$/, '.liveticker.json');
  const exists = files.out.some((o) => o.path === path);
  const [sim, setSim] = useState<Json | null>(null);
  const feed = `/api/ticker-feed?file=${encodeURIComponent(path)}`;
  useEffect(() => {
    setSim(null);
    if (!exists) return;
    void fetch(feed, { cache: 'no-store' }).then((r) => r.ok ? r.json() : null).then((j) => setSim(j?.simulation ?? null)).catch(() => setSim(null));
  }, [path, exists, files.out]); // eslint-disable-line react-hooks/exhaustive-deps
  const make = () => void runJob('ticker', { file, ticker_names: project.live?.ticker_names, home_name: project.live?.home_name, away_name: project.live?.away_name });
  const scenes = sim ? Object.values(sim.scenes ?? {}).reduce((a: number, b) => a + Number(b), 0) : 0;
  return <Card title={tx.title} hint={tx.hint}>
    {sim ? <>
      <div className="mm-tiles">
        <Tile label={tx.entries} value={sim.entries} sub={tx.per10(sim.per_10min)} />
        <Tile label={tx.subs} value={sim.substitutions?.confirmed ?? 0} sub={tx.possible(sim.substitutions?.possible ?? 0)} />
        <Tile label={tx.scenes} value={scenes} />
        <Tile label={tx.gap} value={clock(sim.longest_gap)} sub={tx.gapAt(clock(sim.longest_gap_at))} />
      </div>
      {sim.entries === 0 && <p className="mm-hint">{tx.empty}</p>}
      <div className="mm-row mm-wrap">
        <a className="primary-button" data-hint={t.hints.tickerView} href={`/ticker?feed=${encodeURIComponent(feed)}&lang=${lang}`} target="_blank" rel="noreferrer">▷ {tx.view}</a>
        <Btn hint={t.hints.tickerMake} onClick={make}>{tx.remake}</Btn>
        <span className="mm-muted small">{path.split('/').pop()}</span>
      </div>
    </> : <div className="mm-row mm-wrap"><span className="mm-hint mm-grow">{exists ? tx.unreadable : tx.none}</span><Btn primary hint={t.hints.tickerMake} onClick={make}>{tx.make}</Btn></div>}
  </Card>;
}

// ---------------------------------------------------------------- the panel
export function StatsPanel() {
  const { files, dir, t, project, runJob, common, log } = useApp();
  const ts = t.stats;
  const outs = files.out.filter((x) => x.name.endsWith('.jsonl'));
  const [file, setFile] = useState(outs[0]?.path ?? '');
  const [stats, setStats] = useState<Stats | null>(null);
  const [error, setError] = useState('');
  const [team, setTeam] = useState<'both' | 'home' | 'away'>('both');
  const [layer, setLayer] = useState<'home' | 'away' | 'ball'>('home');
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [tableView, setTableView] = useState(false);

  useEffect(() => { if (!outs.some((o) => o.path === file)) setFile(outs[0]?.path ?? ''); }, [files.out]); // eslint-disable-line react-hooks/exhaustive-deps
  const load = async () => {
    if (!file) return;
    const r = await api('/api/stats', { dir, file });
    if (r.ok) { setStats(r.stats); setError(''); setPicked(new Set()); } else { setStats(null); setError(r.error); }
  };
  useEffect(() => { void load(); }, [file, dir]); // eslint-disable-line react-hooks/exhaustive-deps

  const colors: Record<Team, string> = { home: project.colors?.draw_home ?? '#ffffff', away: project.colors?.draw_away ?? '#2f6bff', none: 'var(--label-3)' };
  const teamName: Record<Team, string> = { home: ts.home, away: ts.away, none: ts.unknown };
  const shown = useMemo(() => (stats?.attempts ?? []).filter((a) => team === 'both' || a.team === team), [stats, team]);

  if (!files.court) return <Card><div className="bad">{ts.needCourt}</div></Card>;
  const picker = <Card title={ts.title} hint={ts.attemptsOnly}>
    {outs.length ? <div className="mm-row mm-gap mm-wrap">
      <div className="mm-grow"><Labeled label={ts.file}><select data-hint={t.hints.statsFile} value={file} onChange={(e) => setFile(e.target.value)}>{outs.map((x) => <option key={x.path} value={x.path}>{x.name}</option>)}</select></Labeled></div>
      <Btn hint={t.hints.statsLoad} onClick={() => void load()}>{ts.load}</Btn>
    </div> : <p className="mm-muted">{ts.none}</p>}
    {error && <p className="bad small">{error}</p>}
  </Card>;
  if (!stats) return <>{picker}{file && <TickerSimCard file={file} />}</>;

  const sec = sections(stats.sport);
  const kindLabel = (k?: string) => (k && ts.kinds[k]) || k || '';
  const zoneLabel = (z?: string) => (z && ts.zones[z]) || z || '';
  const segmented = <div className="segmented" role="group" aria-label={ts.both}>{(['both', 'home', 'away'] as const).map((k) => <button key={k} type="button" data-hint={t.hints.teamFilter} className={team === k ? 'active' : ''} onClick={() => setTeam(k)}>{k === 'both' ? ts.both : teamName[k]}</button>)}</div>;
  const dur = stats.t1 - stats.t0;
  const basket = stats.sport === 'basketball';

  return <>
    <TipLayer />
    {picker}
    <TickerSimCard file={file} />
    <div className="mm-tiles">
      {sec.shots && <><Tile label={`${ts.attempts} · ${ts.home}`} value={stats.totals.home} swatch={colors.home} /><Tile label={`${ts.attempts} · ${ts.away}`} value={stats.totals.away} swatch={colors.away} /></>}
      {sec.possession && <Tile label={`${ts.possession} · ${ts.home}`} value={`${Math.round(stats.possession.home * 100)} %`} sub={`${ts.away} ${Math.round(stats.possession.away * 100)} %`} />}
      <Tile label={ts.ballSeen} value={`${Math.round(stats.possession.ball_seen * 100)} %`} />
      <Tile label={ts.duration} value={clock(dur)} sub={`${clock(stats.t0)} – ${clock(stats.t1)}`} />
    </div>

    {sec.shots && <Card title={basket ? ts.shotChart : ts.shotMap}>
      <div className="mm-row mm-gap mm-wrap">{segmented}<span className="mm-grow" /><button type="button" className="secondary-button" data-hint={ts.table} onClick={() => setTableView((v) => !v)}>{tableView ? ts.chart : ts.table}</button></div>
      {shown.length === 0 ? <p className="mm-muted">{basket ? ts.noAttemptsBasket : ts.noAttempts}</p> : tableView
        ? <table className="mm-table"><thead><tr><th>⏱</th><th>{ts.home}/{ts.away}</th><th /><th /><th>m</th></tr></thead><tbody>{shown.map((a, i) => <tr key={i}><td>{clock(a.t)}</td><td>{teamName[a.team]}</td><td>{kindLabel(a.kind)}</td><td>{zoneLabel(a.zone)}</td><td>{a.distance.toFixed(1)}</td></tr>)}</tbody></table>
        : <>
          <Field s={stats}>
            {stats.goals.map((g, i) => <circle key={i} cx={g[0]} cy={g[1]} r={stats.field.length / 160} className="mm-goal" />)}
            {shown.map((a, i) => <Marker key={i} a={a} size={stats.field.length / 75} color={colors[a.team]} tip={`${clock(a.t)} · ${teamName[a.team]} · ${kindLabel(a.kind)} · ${zoneLabel(a.zone)} · ${a.distance.toFixed(1)} m`} />)}
          </Field>
          <div className="mm-legend">
            {(['home', 'away'] as Team[]).map((k) => <span key={k}><i className="mm-sw" style={{ background: colors[k] }} />{teamName[k]}</span>)}
            {basket ? <><span><Shape kind="shot2" />{ts.legend2}</span><span><Shape kind="shot3" />{ts.legend3}</span><span><Shape kind="free_throw" />{ts.legendFt}</span></>
              : <><span><Shape kind="shot" />{ts.legendShot}</span>{stats.sport === 'handball' && <span><Shape kind="seven_meter" />{kindLabel('seven_meter')}</span>}</>}
          </div>
        </>}
    </Card>}

    {sec.zones && shown.length > 0 && <Card title={ts.zoneHeat} hint={ts.zoneHeatHint}>
      <div className="mm-split">
        <Field s={stats}>{(() => {
          const nx = Math.max(6, Math.round(stats.field.length / 2)), ny = Math.max(4, Math.round(stats.field.width / 2));
          const v = new Array(nx * ny).fill(0);
          for (const a of shown) v[Math.min(ny - 1, Math.max(0, Math.floor(a.y / stats.field.width * ny))) * nx + Math.min(nx - 1, Math.max(0, Math.floor(a.x / stats.field.length * nx)))]++;
          return heatCells(stats, v, nx, ny, 'var(--accent)', (n) => `${n} ${ts.attempts}`);
        })()}</Field>
        <div className="mm-bars">{stats.zones.map((z) => {
          const n = team === 'both' ? z.count : z[team];
          const share = shown.length ? n / shown.length : 0;
          return <div key={z.zone} className="mm-bar-row" {...tipHandlers(`${zoneLabel(z.zone)}: ${n} (${ts.home} ${z.home} · ${ts.away} ${z.away})`)}>
            <span className="mm-bar-label">{zoneLabel(z.zone)}</span>
            <span className="mm-bar-track"><span className="mm-bar-fill" style={{ width: `${share * 100}%` }} /></span>
            <span className="mm-bar-value">{Math.round(share * 100)} % · {n}</span>
          </div>;
        })}</div>
      </div>
    </Card>}

    {sec.periods && stats.attempts.length > 0 && <Card title={ts.periods} hint={ts.periodsHint}>
      <Periods s={stats} colors={colors} names={teamName} label={ts.period} />
      <div className="mm-diff"><span className="mm-muted">{ts.diff}</span>{stats.periods.map((p) => { const d = p.home - p.away; return <span key={p.index} className="mm-chip">{ts.period(p.index)}: {d > 0 ? `+${d}` : d}</span>; })}</div>
    </Card>}

    {(sec.possession || sec.thirds) && <Card title={ts.possession}>
      {sec.possession && stats.possession.frames_held > 0 && <div className="mm-stack" role="img" aria-label={ts.possession}>
        <span style={{ flex: stats.possession.home, background: colors.home }} {...tipHandlers(`${ts.home} ${Math.round(stats.possession.home * 100)} %`)} />
        <span style={{ flex: stats.possession.away, background: colors.away }} {...tipHandlers(`${ts.away} ${Math.round(stats.possession.away * 100)} %`)} />
      </div>}
      {sec.possession && stats.possession.frames_held > 0 && <div className="mm-legend"><span><i className="mm-sw" style={{ background: colors.home }} />{ts.home} {Math.round(stats.possession.home * 100)} %</span><span><i className="mm-sw" style={{ background: colors.away }} />{ts.away} {Math.round(stats.possession.away * 100)} %</span></div>}
      {sec.thirds && <><h3>{ts.thirds}</h3><div className="mm-thirds">{stats.thirds.map((v, i) => <div key={i} style={{ background: `color-mix(in srgb, var(--accent) ${Math.round(10 + v * 80)}%, var(--content))` }}><strong>{Math.round(v * 100)} %</strong><span>{ts.thirdsLabels[i]}</span></div>)}</div></>}
    </Card>}

    {sec.heat && <Card title={ts.heat} hint={ts.heatHint}>
      <div className="segmented" role="group">{(['home', 'away', 'ball'] as const).map((k) => <button key={k} type="button" data-hint={t.hints.heatLayer} className={layer === k ? 'active' : ''} onClick={() => setLayer(k)}>{k === 'ball' ? ts.ball : teamName[k]}</button>)}</div>
      <Field s={stats}>{heatCells(stats, stats.heat[layer], stats.heat.nx, stats.heat.ny, layer === 'ball' ? 'var(--ball)' : 'var(--accent)', (n) => `${Math.round(n / Math.max(1, stats.heat[layer].reduce((a, b) => a + b, 0)) * 1000) / 10} %`)}</Field>
    </Card>}

    {stats.shape && <ShapeCard s={stats} shape={stats.shape} colors={colors} names={teamName} />}

    <Card title={ts.moments} hint={ts.momentsHint}>
      {stats.moments.length === 0 ? <p className="mm-muted">{ts.noMoments}</p> : <>
        <div className="mm-row mm-gap mm-wrap">
          <Btn hint={t.hints.clipsAll} onClick={() => setPicked(picked.size === stats.moments.length ? new Set() : new Set(stats.moments.map((_, i) => i)))}>{picked.size === stats.moments.length ? ts.noneSel : ts.all}</Btn>
          <Btn primary hint={t.hints.clips} disabled={!picked.size || !project.video} onClick={() => void runJob('clip', { ...common(), at: [...picked].map((i) => stats.moments[i].t) })}>{ts.clips(picked.size)}</Btn>
          <span className="mm-grow" /><button type="button" className="mm-link" data-hint={t.hints.showFolder} onClick={() => void api('/api/reveal', { path: `${dir}/out/clips`, open: false }).then((r) => !r.ok && log(T('status.error', { m: r.error })))}>{ts.showClips}</button>
        </div>
        <ul className="mm-moments">{stats.moments.map((m, i) => <li key={i}>
          <label data-hint={t.hints.moment}><input type="checkbox" checked={picked.has(i)} onChange={() => setPicked((s) => { const n = new Set(s); if (n.has(i)) n.delete(i); else n.add(i); return n; })} />
            <span className="mm-time">{clock(m.t)}</span>
            <i className="mm-sw" style={{ background: colors[m.team] ?? colors.none }} />
            <span>{m.type === 'sub' ? `${kindLabel('sub')} ${ts.sub(m.out != null ? '#' + m.out : '?', m.in != null ? '#' + m.in : '?')}` : kindLabel(m.kind)}</span>
            <span className="mm-muted">{teamName[m.team] ?? ''}{m.zone ? ` · ${zoneLabel(m.zone)}` : ''}</span>
          </label>
        </li>)}</ul>
      </>}
    </Card>
  </>;
}

function Tile({ label, value, sub, swatch }: { label: string; value: React.ReactNode; sub?: string; swatch?: string }) {
  return <div className="mm-tile"><span className="mm-tile-label">{swatch && <i className="mm-sw" style={{ background: swatch }} />}{label}</span><strong>{value}</strong>{sub && <small>{sub}</small>}</div>;
}

function Shape({ kind }: { kind: string }) {
  return <svg className="mm-shape" viewBox="-6 -6 12 12" aria-hidden="true"><MarkerPath kind={kind} r={4.5} /></svg>;
}
function MarkerPath({ kind, r, ...rest }: { kind: string; r: number } & React.SVGProps<SVGPathElement>) {
  if (kind === 'shot3') return <path d={`M0 ${-r * 1.2}L${r * 1.2} 0L0 ${r * 1.2}L${-r * 1.2} 0Z`} {...rest} />;
  if (kind === 'free_throw' || kind === 'seven_meter' || kind === 'penalty') return <path d={`M0 ${-r * 1.15}L${r * 1.1} ${r * 0.8}L${-r * 1.1} ${r * 0.8}Z`} {...rest} />;
  return <path d={`M${-r} 0a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`} {...rest} />;
}
function Marker({ a, size, color, tip }: { a: Attempt; size: number; color: string; tip: string }) {
  return <g transform={`translate(${a.x} ${a.y})`} className="mm-marker" {...tipHandlers(tip)}>
    <circle r={size * 2.2} fill="transparent" />
    <MarkerPath kind={a.kind} r={size} fill={color} stroke="var(--content)" strokeWidth={size * 0.35} paintOrder="stroke" />
    <MarkerPath kind={a.kind} r={size} fill="none" stroke="var(--label)" strokeOpacity={0.55} strokeWidth={size * 0.12} />
  </g>;
}

function Periods({ s, colors, names, label }: { s: Stats; colors: Record<Team, string>; names: Record<Team, string>; label: (i: number) => string }) {
  const max = Math.max(1, ...s.periods.flatMap((p) => [p.home, p.away]));
  const H = 140, bw = 22, gap = 2, group = bw * 2 + gap + 34;
  const W = s.periods.length * group + 30;
  return <svg className="mm-periods" viewBox={`0 0 ${W} ${H + 34}`} role="img">
    <line x1={20} x2={W} y1={H} y2={H} className="mm-axis" />
    {[0.5, 1].map((f) => <g key={f}><line x1={20} x2={W} y1={H - f * (H - 16)} y2={H - f * (H - 16)} className="mm-grid" /><text x={16} y={H - f * (H - 16) + 4} className="mm-tick" textAnchor="end">{Math.round(max * f)}</text></g>)}
    {s.periods.map((p, i) => {
      const x0 = 34 + i * group;
      return <g key={p.index}>
        {(['home', 'away'] as Team[]).map((k, j) => {
          const v = p[k as 'home' | 'away'], h = (v / max) * (H - 16);
          const x = x0 + j * (bw + gap);
          return <g key={k} {...tipHandlers(`${label(p.index)} (${clock(p.from)}–${clock(p.to)}) · ${names[k]}: ${v}`)}>
            <rect x={x - 4} y={0} width={bw + 8} height={H} fill="transparent" />
            {v > 0 && <path d={`M${x} ${H}V${H - h + 4}q0 -4 4 -4h${bw - 8}q4 0 4 4V${H}Z`} fill={colors[k]} stroke="var(--label)" strokeOpacity={0.35} strokeWidth={0.75} />}
            <text x={x + bw / 2} y={H - h - 4} className="mm-val" textAnchor="middle">{v}</text>
          </g>;
        })}
        <text x={x0 + bw + gap / 2} y={H + 18} className="mm-tick" textAnchor="middle">{label(p.index)}</text>
      </g>;
    })}
  </svg>;
}

// ---------------------------------------------------------------- team shape: width / depth over time, average line-up per section
function ShapeCard({ s, shape, colors, names }: { s: Stats; shape: Shape; colors: Record<Team, string>; names: Record<Team, string> }) {
  const { t } = useApp();
  const ts = t.stats;
  const [metric, setMetric] = useState<'w' | 'd'>('w');
  const [sec, setSec] = useState(0);
  const hasPhases = !!(shape.home.att || shape.away.att);
  const [phase, setPhase] = useState<Phase>('att');
  const ph: Phase = hasPhases ? phase : 'all';
  const sides = ['home', 'away'] as const;
  const m = (v: number) => `${v.toFixed(1)} m`;
  if (!shape.home.all && !shape.away.all) return <Card title={ts.shape} hint={ts.shapeHint}><p className="mm-muted">{ts.noShape}</p></Card>;
  const section = shape.sections[Math.min(sec, shape.sections.length - 1)];
  const phaseName: Record<Phase, string> = { all: ts.phaseAll, att: ts.phaseAtt, def: ts.phaseDef };
  const r = s.field.length / 90;
  const row = (v: Spread, label: string) => v && <div className="mm-shape-row"><span>{label}</span><strong>{m(v.w)}</strong><span className="mm-muted">{ts.width}</span><strong>{m(v.d)}</strong><span className="mm-muted">{ts.depth}</span></div>;
  return <Card title={ts.shape} hint={ts.shapeHint}>
    <div className="mm-tiles mm-tiles-inline">
      {sides.map((k) => shape[k].all && <div key={k} className="mm-tile">
        <span className="mm-tile-label"><i className="mm-sw" style={{ background: colors[k] }} />{names[k]}</span>
        {hasPhases ? <>{row(shape[k].att, ts.phaseAtt)}{row(shape[k].def, ts.phaseDef)}</> : row(shape[k].all, ts.phaseAll)}
      </div>)}
    </div>
    <h3>{ts.overTime}</h3>
    <div className="segmented" role="group">{(['w', 'd'] as const).map((k) => <button key={k} type="button" data-hint={t.hints.shapeMetric} className={metric === k ? 'active' : ''} onClick={() => setMetric(k)}>{k === 'w' ? ts.width : ts.depth}</button>)}</div>
    <p className="mm-muted small">{metric === 'w' ? ts.widthHint : ts.depthHint}. {ts.smoothed}</p>
    <ShapeLines shape={shape} metric={metric} colors={colors} names={names} />
    <div className="mm-legend">{sides.map((k) => <span key={k}><i className="mm-sw" style={{ background: colors[k] }} />{names[k]}</span>)}</div>
    <h3>{ts.formation}</h3>
    <div className="mm-row mm-gap mm-wrap">
      {hasPhases && <div className="segmented" role="group">{(['att', 'def', 'all'] as const).map((k) => <button key={k} type="button" data-hint={t.hints.shapePhase} className={phase === k ? 'active' : ''} onClick={() => setPhase(k)}>{phaseName[k]}</button>)}</div>}
      <div className="segmented" role="group">{shape.sections.map((p, i) => <button key={p.index} type="button" data-hint={t.hints.shapeSection} className={sec === i ? 'active' : ''} onClick={() => setSec(i)}>{ts.period(p.index)}</button>)}</div>
    </div>
    <p className="mm-muted small">{clock(section.from)} – {clock(section.to)}{sides.map((k) => section[k][ph] ? ` · ${names[k]}: ${ts.width} ${m(section[k][ph]!.w)}, ${ts.depth} ${m(section[k][ph]!.d)}` : '').join('')}</p>
    <Field s={s}>
      {sides.map((k) => {
        const v = section[k][ph];
        if (!v) return null;
        return <g key={k}>
          <rect x={v.cx - v.d / 2} y={v.cy - v.w / 2} width={v.d} height={v.w} fill={colors[k]} fillOpacity={0.12} stroke={colors[k]} strokeWidth={r * 0.25} strokeDasharray={`${r} ${r * 0.6}`} {...tipHandlers(`${names[k]} · ${phaseName[ph]} · ${ts.width} ${m(v.w)} · ${ts.depth} ${m(v.d)}`)} />
          <path d={`M${v.cx - r} ${v.cy}h${2 * r}M${v.cx} ${v.cy - r}v${2 * r}`} stroke="var(--label)" strokeWidth={r * 0.3} {...tipHandlers(`${names[k]} · ${ts.centre}`)} />
          {v.slots.filter((p) => p.share >= 0.2).map((p) => <g key={p.id} className="mm-marker" {...tipHandlers(`${names[k]} · ${ts.slot(p.id)} · ${ts.seen} ${Math.round(p.share * 100)} %`)}>
            <circle cx={p.x} cy={p.y} r={r * 2.2} fill="transparent" />
            <circle cx={p.x} cy={p.y} r={r} fill={colors[k]} stroke="var(--content)" strokeWidth={r * 0.35} paintOrder="stroke" />
            <circle cx={p.x} cy={p.y} r={r} fill="none" stroke="var(--label)" strokeOpacity={0.55} strokeWidth={r * 0.12} />
          </g>)}
        </g>;
      })}
    </Field>
  </Card>;
}

function ShapeLines({ shape, metric, colors, names }: { shape: Shape; metric: 'w' | 'd'; colors: Record<Team, string>; names: Record<Team, string> }) {
  // centred moving average over five bins (about a tenth of the video), gaps stay gaps
  const pts = shape.series.map((p, i, all) => {
    const win = all.slice(Math.max(0, i - 2), i + 3);
    const avg = (k: 'home' | 'away'): Spread => {
      if (!p[k]) return null;
      const v = win.map((q) => q[k]).filter((q): q is NonNullable<Spread> => !!q);
      return { w: v.reduce((a, q) => a + q.w, 0) / v.length, d: v.reduce((a, q) => a + q.d, 0) / v.length };
    };
    return { t: p.t, home: avg('home'), away: avg('away') };
  });
  const max = Math.max(1, ...pts.flatMap((p) => [p.home?.[metric] ?? 0, p.away?.[metric] ?? 0]));
  const top = Math.ceil(max / 5) * 5;
  const W = 640, H = 150, x0 = 34, x1 = W - 8, y0 = 10;
  const t0 = pts[0]?.t ?? 0, t1 = pts[pts.length - 1]?.t ?? 1;
  const X = (t: number) => x0 + ((t - t0) / Math.max(1e-6, t1 - t0)) * (x1 - x0);
  const Y = (v: number) => H - (v / top) * (H - y0);
  const path = (k: 'home' | 'away') => {
    let d = '', pen = false;
    for (const p of pts) { const v = p[k]?.[metric]; if (v == null) { pen = false; continue; } d += `${pen ? 'L' : 'M'}${X(p.t).toFixed(1)} ${Y(v).toFixed(1)}`; pen = true; }
    return d;
  };
  const [hover, setHover] = useState<number | null>(null);
  const step = (x1 - x0) / Math.max(1, pts.length - 1);
  return <svg className="mm-periods mm-lines" viewBox={`0 0 ${W} ${H + 22}`} role="img" onPointerLeave={() => { setHover(null); setTipGlobal(null); }}>
    <line x1={x0} x2={x1} y1={H} y2={H} className="mm-axis" />
    {[0.5, 1].map((f) => <g key={f}><line x1={x0} x2={x1} y1={Y(top * f)} y2={Y(top * f)} className="mm-grid" /><text x={x0 - 4} y={Y(top * f) + 4} className="mm-tick" textAnchor="end">{Math.round(top * f)} m</text></g>)}
    <text x={x0} y={H + 16} className="mm-tick">{clock(t0)}</text>
    <text x={x1} y={H + 16} className="mm-tick" textAnchor="end">{clock(t1)}</text>
    {hover != null && <line x1={X(pts[hover].t)} x2={X(pts[hover].t)} y1={y0} y2={H} className="mm-axis" />}
    {(['home', 'away'] as const).map((k) => <g key={k} fill="none" strokeLinejoin="round" strokeLinecap="round">
      <path d={path(k)} stroke="var(--label)" strokeOpacity={0.3} strokeWidth={3.5} />
      <path d={path(k)} stroke={colors[k]} strokeWidth={2} />
      {hover != null && pts[hover][k] && <circle cx={X(pts[hover].t)} cy={Y(pts[hover][k]![metric])} r={4} fill={colors[k]} stroke="var(--label)" strokeOpacity={0.5} />}
    </g>)}
    {pts.map((p, i) => <rect key={i} x={X(p.t) - step / 2} y={0} width={step} height={H} fill="transparent"
      onPointerEnter={(e) => { setHover(i); setTipGlobal({ x: e.clientX, y: e.clientY, text: `${clock(p.t)} · ${(['home', 'away'] as const).map((k) => `${names[k]} ${p[k] ? p[k]![metric].toFixed(1) + ' m' : '–'}`).join(' · ')}` }); }}
      onPointerMove={(e) => setTipGlobal({ x: e.clientX, y: e.clientY, text: `${clock(p.t)} · ${(['home', 'away'] as const).map((k) => `${names[k]} ${p[k] ? p[k]![metric].toFixed(1) + ' m' : '–'}`).join(' · ')}` })} />)}
  </svg>;
}
