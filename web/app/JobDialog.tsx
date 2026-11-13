import { useEffect, useRef, useState } from 'react';
import { BACKGROUND_KINDS, clock, showFile, useApp, type JobView } from './core';

const SHOW_AFTER_SECONDS = 0.8;

/** Progress window for every long job, as in Reco Trainer: bar, stage, elapsed / remaining time,
 *  minimizable to a chip, cancellable with the choice to keep the shortened result or delete it. */
export function JobDialog() {
  const { jobs, t, cancelJob, files, log, dir } = useApp();
  const tj = t.job, h = t.hints;
  const [minimized, setMinimized] = useState<number | null>(null);
  const [asking, setAsking] = useState<number | null>(null);
  const [dismissed, setDismissed] = useState<Set<number>>(new Set());
  const [showLog, setShowLog] = useState(false);
  const [, tick] = useState(0);
  const seen = useRef<Set<number>>(new Set());

  const fg = jobs.filter((j) => !BACKGROUND_KINDS.has(j.kind));
  const current = [...fg].reverse().find((j) => j.status === 'running' || j.status === 'cancelling') ?? null;
  // a finished job stays open when it failed, was cancelled, or the user was watching it
  const finished = !current ? [...fg].reverse().find((j) => !dismissed.has(j.id) && seen.current.has(j.id) && (j.status === 'failed' || j.status === 'cancelled' || ((j.kind === 'run' || j.kind === 'clip') && !(j.outer && j.outer.index < j.outer.count)))) ?? null : null;

  useEffect(() => {
    if (!current) return undefined;
    const timer = window.setInterval(() => tick((v) => v + 1), 500);
    return () => window.clearInterval(timer);
  }, [current?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const title = (j: JobView) => tj.titles[j.kind] ?? tj.fallback;

  if (finished) {
    const statusText = finished.status === 'failed' ? tj.failed : finished.status === 'cancelled' ? tj.cancelled : tj.done;
    const tail = finished.lines.slice(-14).map((l) => l.t).join('\n');
    const newest = files.out.slice(0, 4);
    const close = () => setDismissed((s) => new Set(s).add(finished.id));
    return <div className="walkthrough-backdrop" role="presentation"><section className="progress-dialog" role="dialog" aria-modal="true" aria-labelledby="job-title">
      <h2 id="job-title">{title(finished)} · {statusText}</h2>
      {finished.status === 'cancelled' && <p className="progress-dialog-message">{finished.kept ? tj.kept : tj.discarded}</p>}
      {finished.status === 'failed' && <pre className="progress-dialog-error mm-pre">{tail}</pre>}
      {finished.status !== 'failed' && finished.kind === 'run' && finished.kept !== false && newest.length > 0 && <div className="mm-results"><strong>{tj.results}</strong>
        {newest.map((f) => <div key={f.path} className="mm-row"><button type="button" className="mm-link" data-hint={h.openFile} onClick={() => void showFile(f.path, true, log)}>{f.name}</button><span className="mm-muted">{f.mb} MB</span><button type="button" className="tool-button" data-hint={h.reveal} onClick={() => void showFile(f.path, false, log)}>{T('btn.reveal')}</button></div>)}
      </div>}
      {finished.kind === 'clip' && finished.status === 'done' && <div className="mm-row"><span>{t.stats.clipsDone}</span><button type="button" className="secondary-button" data-hint={h.showFolder} onClick={() => void showFile(`${dir}/out/clips`, false, log)}>{t.stats.showClips}</button></div>}
      {finished.status !== 'failed' && <details className="mm-details"><summary data-hint={h.jobLog}>{tj.log}</summary><pre className="mm-pre">{tail}</pre></details>}
      <div className="onnx-actions"><button data-hint={h.closeJob} type="button" className="primary-button" onClick={close}>{tj.close}</button></div>
    </section></div>;
  }

  if (!current) return null;
  const elapsed = Math.max((Date.now() - current.startedAt) / 1000, current.elapsedSeconds);
  if (elapsed < SHOW_AFTER_SECONDS && !seen.current.has(current.id)) return null;
  seen.current.add(current.id);

  const progress = typeof current.progress === 'number' ? Math.min(Math.max(current.progress, 0), 1) : null;
  const percent = progress === null ? null : Math.round(progress * 100);
  const cancelling = current.status === 'cancelling';
  const stage = current.outer ?? current.stage;
  const lastLine = [...current.lines].reverse().find((l) => !l.t.startsWith('$'))?.t;

  if (minimized === current.id) {
    return <button data-hint={h.restore} type="button" className="job-chip" onClick={() => setMinimized(null)}>
      <span className={`job-chip-bar${progress === null ? ' indeterminate' : ''}`}><span style={progress === null ? undefined : { width: `${Math.max(progress, 0.02) * 100}%` }} /></span>
      {title(current)}{percent !== null ? ` · ${percent} %` : ''}
    </button>;
  }

  return <div className="walkthrough-backdrop" role="presentation"><section className="progress-dialog" role="dialog" aria-modal="true" aria-labelledby="job-title">
    <div className="progress-dialog-head"><h2 id="job-title">{title(current)}</h2><button data-hint={h.minimize} type="button" className="tool-button" onClick={() => setMinimized(current.id)} aria-label={tj.minimize}>–</button></div>
    {stage && <p className="progress-dialog-stage">{current.outer ? tj.segment(stage.index, stage.count, stage.label) : `${stage.index}/${stage.count} · ${stage.label}`}</p>}
    <p className="progress-dialog-message">{cancelling ? tj.cancelling : current.message || lastLine || tj.working}</p>
    <div className={`progress-dialog-bar${progress === null ? ' indeterminate' : ''}`} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}>
      <span style={progress === null ? undefined : { width: `${Math.max(progress, 0.02) * 100}%` }} />
    </div>
    <div className="progress-dialog-facts">
      {percent !== null && <span>{percent} %</span>}
      <span>{tj.elapsed(clock(elapsed))}{current.etaSeconds != null ? ` · ${tj.remaining(clock(current.etaSeconds))}` : ''}</span>
    </div>
    {showLog && <pre className="mm-pre">{current.lines.slice(-12).map((l) => l.t).join('\n')}</pre>}
    {asking === current.id && !cancelling
      ? <>
        <p className="progress-dialog-message">{current.keepable ? tj.ask : tj.askPlain}</p>
        <div className="onnx-actions">
          <button data-hint={h.cancelContinue} type="button" className="secondary-button" onClick={() => setAsking(null)}>{tj.goOn}</button>
          <button data-hint={h.cancelDiscard} type="button" className="secondary-button" onClick={() => { setAsking(null); void cancelJob(current.id, false); }}>{tj.discard}</button>
          {current.keepable && <button data-hint={h.cancelKeep} type="button" className="primary-button" onClick={() => { setAsking(null); void cancelJob(current.id, true); }}>{tj.keep}</button>}
        </div>
      </>
      : <div className="onnx-actions">
        <button data-hint={h.jobLog} type="button" className="secondary-button" onClick={() => setShowLog((v) => !v)}>{tj.log}</button>
        <button data-hint={h.cancelJob} type="button" className="secondary-button" disabled={cancelling} onClick={() => setAsking(current.id)}>{cancelling ? tj.cancelling : tj.cancel}</button>
      </div>}
  </section></div>;
}
