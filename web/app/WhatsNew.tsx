import { useEffect, useState } from 'react';
import { api, useApp } from './core';

type Notes = Record<string, Record<string, string[]>>;
const SEEN = 'reco-minimap-seen-version';

/** Ordered newest first ("0.15.0" > "0.14.0" > "0.10.1"). */
function byVersion(a: string, b: string) {
  const pa = a.split('.').map(Number), pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pb[i] ?? 0) - (pa[i] ?? 0);
  return 0;
}

/** "What's new": opens by itself once after each new version, from the version button, with
 *  `#whats-new` in the address, or from the Mac app ("About Reco Minimap"). */
export function WhatsNew({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { init, lang } = useApp();
  const [notes, setNotes] = useState<Notes | null>(null);
  const [older, setOlder] = useState(false);
  useEffect(() => {
    if (open && !notes) void api('/api/changelog').then((r) => r.ok && setNotes(r.notes as Notes));
  }, [open, notes]);
  if (!open || !init || !notes) return null;
  const current = init.version;
  const pick = (v: string) => notes[v]?.[lang] ?? notes[v]?.en ?? [];
  const versions = Object.keys(notes).sort(byVersion);
  const close = () => { try { localStorage.setItem(SEEN, current); } catch { /* private window */ } setOlder(false); onClose(); };
  const list = (v: string) => <ul className="mm-news">{pick(v).map((line, i) => <li key={i}>{line}</li>)}</ul>;
  return <div className="walkthrough-backdrop" role="presentation" onClick={(e) => e.target === e.currentTarget && close()}>
    <section className="progress-dialog mm-news-dialog" role="dialog" aria-modal="true" aria-labelledby="news-title">
      <h2 id="news-title">{T('new.title', { v: current })}</h2>
      {list(notes[current] ? current : versions[0])}
      <button type="button" className="mm-link small" onClick={() => setOlder((v) => !v)}>{older ? '▾' : '▸'} {T('new.older')}</button>
      {older && versions.filter((v) => v !== current && byVersion(v, current) > 0).map((v) => <div key={v}><h3 className="mm-news-version">{v}</h3>{list(v)}</div>)}
      <div className="onnx-actions"><button type="button" className="primary-button" onClick={close}>{T('new.close')}</button></div>
    </section>
  </div>;
}

/** Whether the notes of this version have not been shown yet. */
export function unseen(version: string) {
  try { return localStorage.getItem(SEEN) !== version; } catch { return false; }
}
