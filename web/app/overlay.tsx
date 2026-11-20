import { useEffect, useState } from 'react';
import { Btn, Card, Check, Labeled, Sel, Text, useApp, type Json, type OutFile } from './core';

const PRESETS = ['classic', 'bar', 'compact', 'big', 'minimal'] as const;
/** Where each template sits when chosen (the watermark stays top right). */
const PRESET_ANCHOR: Record<string, string> = { classic: 'tl', bar: 'bc', compact: 'tl', big: 'tc', minimal: 'bl' };
const ANCHORS = ['tl', 'tc', 'tr', 'ml', 'mc', 'mr', 'bl', 'bc', 'br'];

/** A slider bound to a project path (0.5 … 2 for sizes, 0 … 1 for opacity), shown in per cent. */
function Range({ k, label, min, max, step }: { k: string; label: string; min: number; max: number; step: number }) {
  const { project, set } = useApp();
  const v = Number(k.split('.').reduce<unknown>((a, x) => (a as Json | undefined)?.[x], project) ?? 1);
  return <Labeled label={`${label} · ${Math.round(v * 100)} %`}><input type="range" min={min} max={max} step={step} value={v} onChange={(e) => set(k, parseFloat(e.target.value))} /></Labeled>;
}

/** A colour that may be left to the team colour ('' = automatic). */
function AutoColor({ k, label, fallback }: { k: string; label: string; fallback: string }) {
  const { project, set, t } = useApp();
  const v = (k.split('.').reduce<unknown>((a, x) => (a as Json | undefined)?.[x], project) as string) || '';
  return <Labeled label={label}><div className="mm-row mm-gap">
    <input type="color" value={v || fallback || '#888888'} onChange={(e) => set(k, e.target.value)} />
    {v ? <button type="button" className="mm-link small" data-hint={t.overlay.teamColor} onClick={() => set(k, '')}>{t.overlay.teamColor}</button> : <span className="mm-muted small">{t.overlay.teamColor}</span>}
  </div></Labeled>;
}

/** The project video with the graphics on it, drawn by the program itself (as in the video). */
function Preview({ overlay }: { overlay: Json }) {
  const { dir, project, t } = useApp();
  const [track, setTrack] = useState(true);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const key = JSON.stringify(overlay);
  useEffect(() => {
    if (!project.video) { setFailed(true); return undefined; }
    let stop = false, made: string | null = null;
    const id = window.setTimeout(() => {
      void fetch('/api/overlay/preview', { method: 'POST', body: JSON.stringify({ dir, overlay, track }) }).then(async (r) => {
        if (stop) return;
        if (!r.ok) { setFailed(true); return; }
        made = URL.createObjectURL(await r.blob());
        setUrl(made); setFailed(false);
      }).catch(() => !stop && setFailed(true));
    }, 350);
    return () => { stop = true; window.clearTimeout(id); if (made) URL.revokeObjectURL(made); };
  }, [key, track, dir, project.video]); // eslint-disable-line react-hooks/exhaustive-deps
  const to = t.overlay;
  return <>
    <div className="mm-row mm-gap mm-wrap"><h3 className="mm-grow">{to.preview}</h3>
      <div className="segmented">{([[false, to.previewPano], [true, to.previewTrack]] as [boolean, string][]).map(([v, l]) => <button key={l} type="button" className={track === v ? 'active' : ''} onClick={() => setTrack(v)}>{l}</button>)}</div></div>
    {failed ? <p className="mm-muted">{to.previewError}</p> : url ? <img className="mm-overlay-preview" src={url} alt={to.preview} /> : null}
    <p className="mm-hint">{to.previewNote}</p>
  </>;
}

/** Scoreboard (five templates, each adjustable) and watermark of the videos and the live overlay. */
export function OverlayCard() {
  const { project, set, mutate, t } = useApp();
  const to = t.overlay;
  const ov = project.overlay ?? {}, sb = ov.scoreboard ?? {}, wm = ov.watermark ?? {};
  const anchors = ANCHORS.map((a) => [a, to.anchors[a]] as [string, string]);
  const pick = (p: string) => mutate((x) => { x.overlay.scoreboard.preset = p; x.overlay.scoreboard.anchor = PRESET_ANCHOR[p]; x.overlay.scoreboard.scale = 1; });
  return <Card title={to.title} hint={to.hint}>
    <h3>{to.scoreboard}</h3>
    <Check k="overlay.scoreboard.on" label={to.sbOn} />
    {sb.on && <>
      <div className="segmented mm-wrap" role="radiogroup">{PRESETS.map((p) => <button key={p} type="button" role="radio" aria-checked={sb.preset === p} className={sb.preset === p ? 'active' : ''} data-hint={to.presetHints[p]} onClick={() => pick(p)}>{to.presets[p]}</button>)}</div>
      <p className="mm-hint">{to.presetHints[sb.preset] ?? ''} {to.sbHint}</p>
      <div className="mm-grid3"><Sel k="overlay.scoreboard.anchor" label={to.anchor} options={anchors} /><Range k="overlay.scoreboard.scale" label={to.size} min={0.5} max={2} step={0.05} /><Range k="overlay.scoreboard.opacity" label={to.opacity} min={0} max={1} step={0.05} /></div>
      <Check k="overlay.scoreboard.clock" label={to.clock} /><Check k="overlay.scoreboard.period" label={to.period} />
      <div className="mm-grid3"><Text k="overlay.scoreboard.home" label={to.home} placeholder={to.auto} /><Text k="overlay.scoreboard.away" label={to.away} placeholder={to.auto} /><span /></div>
      <div className="mm-grid3">
        <AutoColor k="overlay.scoreboard.home_color" label={to.homeColor} fallback={project.colors?.draw_home} />
        <AutoColor k="overlay.scoreboard.away_color" label={to.awayColor} fallback={project.colors?.draw_away} />
        <Labeled label={`${to.bg} / ${to.fg}`}><div className="mm-row mm-gap"><input type="color" value={sb.bg || '#10141a'} onChange={(e) => set('overlay.scoreboard.bg', e.target.value)} /><input type="color" value={sb.fg || '#ffffff'} onChange={(e) => set('overlay.scoreboard.fg', e.target.value)} /></div></Labeled>
      </div>
      <div><Btn hint={to.reset} onClick={() => mutate((x) => { Object.assign(x.overlay.scoreboard, { anchor: PRESET_ANCHOR[sb.preset] ?? 'tl', scale: 1, opacity: 0.85, bg: '#10141a', fg: '#ffffff', home_color: '', away_color: '' }); })}>{to.reset}</Btn></div>
    </>}
    <h3>{to.watermark}</h3>
    <Check k="overlay.watermark.on" label={to.wmOn} />
    {wm.on && <div className="mm-grid3"><Text k="overlay.watermark.text" label={to.wmText} placeholder="RecoCam" /><Sel k="overlay.watermark.anchor" label={to.anchor} options={anchors} /><Range k="overlay.watermark.scale" label={to.size} min={0.5} max={3} step={0.05} /></div>}
    {wm.on && <div className="mm-grid3"><Range k="overlay.watermark.opacity" label={to.wmOpacity} min={0.1} max={1} step={0.05} /></div>}
    {(sb.on || wm.on) && <Preview overlay={ov} />}
  </Card>;
}

/** A new video from a finished analysis: autotrack and/or panorama, with sound and graphics. */
export function RenderCard({ outs }: { outs: OutFile[] }) {
  const { project, runJob, common, t } = useApp();
  const tv = t.videos;
  const results = outs.filter((o) => o.name.endsWith('.jsonl') && !o.name.endsWith('.dets.jsonl'));
  const [file, setFile] = useState(results[0]?.path ?? '');
  useEffect(() => { if (!results.some((o) => o.path === file)) setFile(results[0]?.path ?? ''); }, [outs]); // eslint-disable-line react-hooks/exhaustive-deps
  const r = project.render ?? {}, c = project.colors ?? {};
  const zooms: [string, string][] = [['1', tv.zooms.wide], ['0.8', tv.zooms.normal], ['0.65', tv.zooms.close]];
  return <Card title={tv.title} hint={tv.hint}>
    <Labeled label={tv.file}><select value={file} onChange={(e) => setFile(e.target.value)}>{results.map((x) => <option key={x.path} value={x.path}>{x.name}</option>)}</select></Labeled>
    <Check k="render.track" label={tv.track} />
    {r.track && <div className="mm-grid3"><Sel k="render.zoom" label={tv.zoom} options={zooms} numeric /></div>}
    <Check k="render.pip" label={tv.pip} /><Check k="render.minimap" label={tv.minimap} /><Check k="render.audio" label={tv.audio} />
    <p className="mm-hint">{tv.speed}</p>
    <div className="mm-row mm-gap"><Btn primary hint={tv.btn} disabled={!file || (!r.track && !r.pip)} onClick={() => void runJob('render', { ...common(), file, track: r.track === true, pip: r.pip === true, audio: r.audio === true, minimap: r.minimap !== false, zoom: Number(r.zoom) || 0.8, home_name: project.live?.home_name, away_name: project.live?.away_name, draw_home: c.draw_home, draw_away: c.draw_away })}>{tv.btn}</Btn></div>
  </Card>;
}
