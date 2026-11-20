import { useEffect, useMemo, useState } from 'react';
import qrcode from 'qrcode-generator';
import { useApp } from './core';

/** A QR code as SVG: dark modules on a white field with the quiet zone the standard asks for. */
export function QrCode({ text, size = 240 }: { text: string; size?: number }) {
  const path = useMemo(() => {
    const qr = qrcode(0, 'M');
    qr.addData(text);
    qr.make();
    const n = qr.getModuleCount();
    let d = '';
    for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (qr.isDark(r, c)) d += `M${c + 4} ${r + 4}h1v1h-1z`;
    return { d, n: n + 8 };
  }, [text]);
  return <svg className="mm-qr" width={size} height={size} viewBox={`0 0 ${path.n} ${path.n}`} role="img" aria-label={text} shapeRendering="crispEdges">
    <rect width={path.n} height={path.n} fill="#fff" /><path d={path.d} fill="#000" />
  </svg>;
}

/** True for a link a phone in the same network cannot open (this computer only). */
const localOnly = (url: string) => /^https?:\/\/(127\.0\.0\.1|localhost|\[::1\])[:/]/.test(url);

/** A popup with the QR code of a link: the phone's camera opens it in the same Wi-Fi. */
export function QrDialog({ url, title, note, onClose }: { url: string; title: string; note?: string; onClose: () => void }) {
  const { t } = useApp();
  const tq = t.qr;
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [onClose]);
  const local = localOnly(url);
  return <div className="walkthrough-backdrop" role="presentation" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
    <section className="progress-dialog mm-qr-dialog" role="dialog" aria-modal="true" aria-labelledby="qr-title">
      <h2 id="qr-title">{title}</h2>
      {local ? <p className="bad">{tq.local}</p> : <>
        <div className="mm-qr-box"><QrCode text={url} /></div>
        <p>{tq.scan}</p>
      </>}
      {note && <p className="mm-hint">{note}</p>}
      <p className="mm-break small mm-muted">{url}</p>
      <div className="onnx-actions">
        <button type="button" className="secondary-button" data-hint={tq.copy} onClick={() => { void navigator.clipboard?.writeText(url).then(() => setCopied(true)); }}>{copied ? tq.copied : tq.copy}</button>
        <button type="button" className="primary-button" data-hint={tq.close} onClick={onClose}>{tq.close}</button>
      </div>
    </section>
  </div>;
}

/** A button that shows a link as a QR code. */
export function QrButton({ url, title, note }: { url: string; title: string; note?: string }) {
  const { t } = useApp();
  const [open, setOpen] = useState(false);
  return <>
    <button type="button" className="secondary-button" data-hint={t.qr.hint} onClick={() => setOpen(true)}>▦ {t.qr.button}</button>
    {open && <QrDialog url={url} title={title} note={note} onClose={() => setOpen(false)} />}
  </>;
}
