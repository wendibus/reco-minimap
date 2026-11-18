//! Automatic search for the scoreboard and proposal of its digit blocks.
//!
//! The input is a short "burst" (a few seconds of video, ~8 frames/s). An LED board shows up in
//! it as bright, thin structures on a dark background that flicker or change (the clock). Per
//! pixel a high and a low percentile over time are taken; "bright against the surroundings"
//! (top-hat) plus "changes" gives a weight map. In it, row bands and horizontal runs within them
//! are searched – those are the board candidates. Everything else (confirming, correcting the
//! corners) is done by a person on the calibration page.

use super::quad::Quad;
use super::FieldCfg;

/// Full frames kept for picking the brightest moment: at most 16 and at most ~256 MB.
fn max_keep(w: usize, h: usize) -> usize {
    (256_000_000 / (w * h * 3).max(1)).clamp(3, 16)
}

pub struct Burst {
    /// Full frame size
    pub w: usize,
    pub h: usize,
    /// Downscale factor of the analysis images
    pub f: usize,
    pub ws: usize,
    pub hs: usize,
    /// Per frame the brightness (max of R,G,B, max-pooled) at analysis size
    pub small: Vec<Vec<u8>>,
    /// Pixel-wise maximum of all frames (RGB, full size): all segments lit
    pub max_rgb: Vec<u8>,
    /// Some full frames to pick the brightest moment from
    pub keep: Vec<Vec<u8>>,
    /// First frame
    pub first: Vec<u8>,
}

pub struct BurstBuilder {
    b: Burst,
    keep_every: usize,
    n: usize,
}

impl BurstBuilder {
    pub fn new(w: usize, h: usize, keep_every: usize) -> Self {
        let f = w.div_ceil(1000).clamp(1, 4);
        let (ws, hs) = ((w / f).max(1), (h / f).max(1));
        Self {
            b: Burst {
                w,
                h,
                f,
                ws,
                hs,
                small: Vec::new(),
                max_rgb: vec![0; w * h * 3],
                keep: Vec::new(),
                first: Vec::new(),
            },
            keep_every: keep_every.max(1),
            n: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.n
    }

    pub fn push(&mut self, frame: &[u8]) {
        let b = &mut self.b;
        debug_assert_eq!(frame.len(), b.w * b.h * 3);
        if self.n == 0 {
            b.first = frame.to_vec();
            b.max_rgb.copy_from_slice(frame);
        } else {
            for (m, &v) in b.max_rgb.iter_mut().zip(frame) {
                if v > *m {
                    *m = v;
                }
            }
        }
        let (w, h, f, ws, hs) = (b.w, b.h, b.f, b.ws, b.hs);
        let mut s = vec![0u8; ws * hs];
        for y in 0..hs {
            for x in 0..ws {
                let mut m = 0u8;
                for dy in 0..f {
                    let row = (y * f + dy).min(h - 1) * w;
                    for dx in 0..f {
                        let o = (row + (x * f + dx).min(w - 1)) * 3;
                        m = m.max(frame[o]).max(frame[o + 1]).max(frame[o + 2]);
                    }
                }
                s[y * ws + x] = m;
            }
        }
        b.small.push(s);
        if self.n % self.keep_every == 0 && b.keep.len() < max_keep(w, h) {
            b.keep.push(frame.to_vec());
        }
        self.n += 1;
    }

    pub fn finish(self) -> Burst {
        self.b
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    /// Top left, top right, bottom right, bottom left (full-frame coordinates)
    pub corners: Quad,
    /// 1.0 = best candidate
    pub score: f64,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    x0: usize,
    y0: usize,
    x1: usize, // inclusive
    y1: usize,
    sum: f64,
    count: usize,
}

impl Item {
    fn w(&self) -> usize {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> usize {
        self.y1 - self.y0 + 1
    }
}

fn box_mean(src: &[u8], w: usize, h: usize, r: usize) -> Vec<f32> {
    // integral image
    let mut ii = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u64;
        for x in 0..w {
            row += src[y * w + x] as u64;
            ii[(y + 1) * (w + 1) + x + 1] = ii[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = ii[y1 * (w + 1) + x1] + ii[y0 * (w + 1) + x0] - ii[y0 * (w + 1) + x1] - ii[y1 * (w + 1) + x0];
            out[y * w + x] = s as f32 / ((x1 - x0) * (y1 - y0)) as f32;
        }
    }
    out
}

/// Weight map: bright against the surroundings, stronger when the pixel changes over time.
fn weight_map(b: &Burst) -> Vec<f32> {
    let (ws, hs, n) = (b.ws, b.hs, b.small.len());
    let mut hi = vec![0u8; ws * hs];
    let mut lo = vec![0u8; ws * hs];
    let ih = ((n - 1) as f64 * 0.9).round() as usize;
    let il = ((n - 1) as f64 * 0.1).round() as usize;
    let mut buf = vec![0u8; n];
    for i in 0..ws * hs {
        for (k, s) in b.small.iter().enumerate() {
            buf[k] = s[i];
        }
        buf.sort_unstable();
        hi[i] = buf[ih];
        lo[i] = buf[il];
    }
    let bg = box_mean(&hi, ws, hs, 10);
    let mut w = vec![0f32; ws * hs];
    for i in 0..ws * hs {
        let th = hi[i] as f32 - bg[i];
        if th >= 55.0 && hi[i] >= 100 {
            let act = ((hi[i] - lo[i]) as f32 / 90.0).clamp(0.0, 1.0);
            w[i] = 0.35 + 0.65 * act;
        }
    }
    w
}

fn merge_items(items: &mut Vec<Item>) {
    loop {
        let mut merged = false;
        'outer: for i in 0..items.len() {
            for j in i + 1..items.len() {
                let (a, b) = (items[i], items[j]);
                let ox = (a.x1.min(b.x1) as i64 - a.x0.max(b.x0) as i64 + 1).max(0) as f64;
                let minw = a.w().min(b.w()) as f64;
                let gap = (a.y0.max(b.y0) as i64 - a.y1.min(b.y1) as i64 - 1).max(0) as f64;
                let hmax = a.h().max(b.h()) as f64;
                if ox >= 0.5 * minw && gap <= hmax {
                    items[i] = Item {
                        x0: a.x0.min(b.x0),
                        y0: a.y0.min(b.y0),
                        x1: a.x1.max(b.x1),
                        y1: a.y1.max(b.y1),
                        sum: a.sum + b.sum,
                        count: a.count + b.count,
                    };
                    items.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
}

fn iou(a: &Item, b: &Item) -> f64 {
    let ix = (a.x1.min(b.x1) as i64 - a.x0.max(b.x0) as i64 + 1).max(0) as f64;
    let iy = (a.y1.min(b.y1) as i64 - a.y0.max(b.y0) as i64 + 1).max(0) as f64;
    let inter = ix * iy;
    let min_area = ((a.w() * a.h()).min(b.w() * b.h())) as f64;
    inter / min_area.max(1.0)
}

pub fn find_boards(b: &Burst) -> Vec<Candidate> {
    let (ws, hs) = (b.ws, b.hs);
    if b.small.len() < 4 {
        return vec![];
    }
    let w = weight_map(b);

    // row profile -> bands
    let mut r: Vec<f32> = (0..hs).map(|y| w[y * ws..(y + 1) * ws].iter().sum()).collect();
    r = (0..hs)
        .map(|y| {
            let (a, c) = (y.saturating_sub(1), (y + 1).min(hs - 1));
            (r[a] + r[y] + r[c]) / 3.0
        })
        .collect();
    let peak0 = r.iter().cloned().fold(0.0, f32::max);
    if peak0 < 4.0 {
        return vec![];
    }
    let mut bands: Vec<(usize, usize)> = vec![];
    for _ in 0..5 {
        let (py, pv) = r.iter().cloned().enumerate().fold((0, 0.0), |a, (i, v)| if v > a.1 { (i, v) } else { a });
        if pv < 0.15 * peak0 || pv < 4.0 {
            break;
        }
        let thr = 0.12 * pv;
        let mut y0 = py;
        while y0 > 0 && r[y0 - 1] >= thr {
            y0 -= 1;
        }
        let mut y1 = py;
        while y1 + 1 < hs && r[y1 + 1] >= thr {
            y1 += 1;
        }
        bands.push((y0, y1));
        for v in &mut r[y0..=y1] {
            *v = 0.0;
        }
    }

    // per band: horizontal runs
    let mut items: Vec<Item> = vec![];
    for (y0, y1) in bands {
        let hb = y1 - y0 + 1;
        if hb < 5 || hb as f64 > hs as f64 * 0.45 {
            continue;
        }
        let gx = (5.0 * hb as f64) as usize;
        let mut col = vec![0f32; ws];
        for y in y0..=y1 {
            for x in 0..ws {
                col[x] += w[y * ws + x];
            }
        }
        let mut start: Option<usize> = None;
        let mut last = 0usize;
        let close = |s: usize, e: usize, items: &mut Vec<Item>| {
            let mut sum = 0f64;
            let mut count = 0usize;
            let (mut ymin, mut ymax) = (usize::MAX, 0usize);
            for y in y0..=y1 {
                for x in s..=e {
                    let v = w[y * ws + x];
                    if v > 0.0 {
                        sum += v as f64;
                        count += 1;
                        ymin = ymin.min(y);
                        ymax = ymax.max(y);
                    }
                }
            }
            if count > 0 {
                items.push(Item { x0: s, y0: ymin, x1: e, y1: ymax, sum, count });
            }
        };
        for x in 0..ws {
            if col[x] > 0.0 {
                match start {
                    Some(_) if x - last <= gx => {}
                    Some(s) => {
                        close(s, last, &mut items);
                        start = Some(x);
                    }
                    None => start = Some(x),
                }
                last = x;
            }
        }
        if let Some(s) = start {
            close(s, last, &mut items);
        }
    }
    merge_items(&mut items);

    // filter and score
    let mut scored: Vec<(f64, Item)> = items
        .into_iter()
        .filter_map(|it| {
            let (iw, ih) = (it.w() as f64, it.h() as f64);
            if ih < 5.0 || iw < 1.2 * ih || iw / ih > 30.0 || it.count < 12 {
                return None;
            }
            let fill = it.count as f64 / (iw * ih);
            if fill > 0.75 || fill < 0.015 {
                return None;
            }
            let bonus = if (0.06..=0.55).contains(&fill) { 1.0 } else { 0.5 };
            Some((it.sum * bonus, it))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut kept: Vec<(f64, Item)> = vec![];
    for (s, it) in scored {
        if kept.iter().all(|(_, k)| iou(&it, k) < 0.3) {
            kept.push((s, it));
        }
        if kept.len() == 3 {
            break;
        }
    }
    let top = kept.first().map(|k| k.0).unwrap_or(1.0).max(1e-9);
    kept.into_iter()
        .map(|(s, it)| Candidate { corners: oriented_quad(b, &w, &it), score: s / top })
        .collect()
}

/// Oriented quad around the active pixels (main axis by PCA, at most ±25°).
fn oriented_quad(b: &Burst, w: &[f32], it: &Item) -> Quad {
    let ws = b.ws;
    let mut pts: Vec<(f64, f64)> = vec![];
    for y in it.y0..=it.y1 {
        for x in it.x0..=it.x1 {
            if w[y * ws + x] > 0.0 {
                pts.push((x as f64 + 0.5, y as f64 + 0.5));
            }
        }
    }
    let n = pts.len().max(1) as f64;
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let (mut cxx, mut cyy, mut cxy) = (0.0, 0.0, 0.0);
    for p in &pts {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        cxx += dx * dx;
        cyy += dy * dy;
        cxy += dx * dy;
    }
    let mut theta = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
    if theta.abs() > 25f64.to_radians() || it.w() < 2 * it.h() {
        theta = 0.0;
    }
    let (ex, ey) = ((theta.cos(), theta.sin()), (-theta.sin(), theta.cos()));
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &pts {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        let u = dx * ex.0 + dy * ex.1;
        let v = dx * ey.0 + dy * ey.1;
        u0 = u0.min(u);
        u1 = u1.max(u);
        v0 = v0.min(v);
        v1 = v1.max(v);
    }
    let m = (0.25 * it.h() as f64).max(3.0);
    // more room on the left: when the tens digit of the left score is blank right now, it
    // would otherwise lie outside the frame.
    let ml = m.max(0.9 * it.h() as f64);
    let (u0, u1, v0, v1) = (u0 - ml, u1 + m, v0 - m, v1 + m);
    let f = b.f as f64;
    let corner = |u: f64, v: f64| -> [f64; 2] {
        let x = (mx + u * ex.0 + v * ey.0) * f;
        let y = (my + u * ex.1 + v * ey.1) * f;
        [x.clamp(0.0, b.w as f64 - 1.0), y.clamp(0.0, b.h as f64 - 1.0)]
    };
    [corner(u0, v0), corner(u1, v0), corner(u1, v1), corner(u0, v1)]
}

#[derive(Clone, Copy, Debug)]
struct Run {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

struct Block {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    n: usize,
    gap: f64,
    dw: f64,
}

/// Propose digit blocks on the rectified view (brightness `v`, w × h).
/// Returns left_score, right_score, clock and period as far as recognised.
pub fn propose_fields(v: &[u8], w: usize, h: usize) -> Vec<(&'static str, FieldCfg)> {
    let mut hist = [0usize; 256];
    for &p in v {
        hist[p as usize] += 1;
    }
    let mut acc = 0usize;
    let mut p995 = 255usize;
    for i in (0..256).rev() {
        acc += hist[i];
        if acc as f64 >= v.len() as f64 * 0.005 {
            p995 = i;
            break;
        }
    }
    if p995 < 80 {
        return vec![];
    }
    let thr = (p995 / 2) as u8;

    // row range of the digits and all mask pixels
    let mut pts: Vec<(usize, usize)> = vec![];
    let mut rowc = vec![0u32; h];
    for y in 0..h {
        for x in 0..w {
            if v[y * w + x] > thr {
                pts.push((x, y));
                rowc[y] += 1;
            }
        }
    }
    let rows: Vec<usize> = (0..h).filter(|&y| rowc[y] >= 3).collect();
    if rows.is_empty() {
        return vec![];
    }
    let (ya, yb) = (rows[0], *rows.last().unwrap());
    let cy = (ya + yb) as f64 / 2.0;
    let hh = (yb - ya + 1) as f64;
    let pad = (0.5 * hh).ceil() as usize;
    let wp = w + 2 * pad;

    // estimate the slant (italic digits): with the right shear the vertical strokes become
    // narrow, tall columns -> the column profile is sharpest.
    let sharp = |sl: f64| -> f64 {
        let mut col = vec![0u32; wp];
        for &(x, y) in &pts {
            let i = (x as f64 + sl * (y as f64 - cy)).round() as i64 + pad as i64;
            if i >= 0 && (i as usize) < wp {
                col[i as usize] += 1;
            }
        }
        col.iter().map(|&c| (c as f64).powi(2)).sum()
    };
    let base = sharp(0.0);
    let mut best = (0.0f64, base);
    for k in -14..=14 {
        let sl = k as f64 * 0.025;
        let sc = sharp(sl);
        if sc > best.1 {
            best = (sl, sc);
        }
    }
    let slant = if best.1 > 1.04 * base { best.0 } else { 0.0 };

    let mut col = vec![0u32; wp];
    let mut ymin = vec![usize::MAX; wp];
    let mut ymax = vec![0usize; wp];
    for &(x, y) in &pts {
        let i = (x as f64 + slant * (y as f64 - cy)).round() as i64 + pad as i64;
        if i >= 0 && (i as usize) < wp {
            let i = i as usize;
            col[i] += 1;
            ymin[i] = ymin[i].min(y);
            ymax[i] = ymax[i].max(y);
        }
    }
    let build_runs = |tol: usize| -> Vec<Run> {
        let mut runs: Vec<Run> = vec![];
        let mut cur: Option<Run> = None;
        let mut last = 0usize;
        for x in 0..wp {
            if col[x] < 2 {
                continue;
            }
            if cur.is_some() && x - last <= tol {
                let r = cur.as_mut().unwrap();
                r.x1 = x;
                r.y0 = r.y0.min(ymin[x]);
                r.y1 = r.y1.max(ymax[x]);
            } else {
                if let Some(r) = cur.take() {
                    runs.push(r);
                }
                cur = Some(Run { x0: x, x1: x, y0: ymin[x], y1: ymax[x] });
            }
            last = x;
        }
        if let Some(r) = cur.take() {
            runs.push(r);
        }
        runs
    };
    // fine runs first, the digit height from them. Colons (narrow and short) are dropped before
    // merging, otherwise they bridge the gap between two digits. Then gaps within a digit
    // (between bars and side strokes) are bridged relative to the digit height.
    let fine = build_runs(3);
    let href0 = fine.iter().map(|r| r.y1 - r.y0 + 1).max().unwrap_or(0) as f64;
    let tol = ((0.09 * href0) as usize).max(3);
    let mut runs: Vec<Run> = vec![];
    for r in fine {
        let (rw, rh) = ((r.x1 - r.x0 + 1) as f64, (r.y1 - r.y0 + 1) as f64);
        if rw < 0.25 * href0 && rh < 0.7 * href0 {
            continue;
        }
        match runs.last_mut() {
            Some(l) if r.x0 - l.x1 <= tol => {
                l.x1 = r.x1;
                l.y0 = l.y0.min(r.y0);
                l.y1 = l.y1.max(r.y1);
            }
            _ => runs.push(r),
        }
    }
    let href = runs.iter().map(|r| r.y1 - r.y0 + 1).max().unwrap_or(0) as f64;
    // colon and noise are dropped: digits are tall
    runs.retain(|r| r.x1 - r.x0 + 1 >= 3 && (r.y1 - r.y0 + 1) as f64 >= 0.45 * href);
    if runs.is_empty() {
        return vec![];
    }

    let mut groups: Vec<Vec<Run>> = vec![];
    for r in runs {
        match groups.last_mut() {
            Some(g) if (r.x0 as f64 - g.last().unwrap().x1 as f64) <= 0.4 * href => g.push(r),
            _ => groups.push(vec![r]),
        }
    }
    let blocks: Vec<Block> = groups
        .iter()
        .map(|g| {
            let n = g.len();
            let dw = g.iter().map(|r| (r.x1 - r.x0 + 1) as f64).fold(0.0, f64::max);
            let (pitch, gap) = if n >= 2 {
                let p = (g[n - 1].x1 as f64 - g[0].x1 as f64) / (n - 1) as f64;
                (p, (p / dw - 1.0).clamp(0.05, 0.8))
            } else {
                (dw * 1.25, 0.25)
            };
            let x1 = g[n - 1].x1 as f64 + 1.0;
            let x0 = x1 - ((n - 1) as f64 * pitch + dw);
            Block {
                x0,
                y0: g.iter().map(|r| r.y0).min().unwrap() as f64,
                x1,
                y1: g.iter().map(|r| r.y1).max().unwrap() as f64 + 1.0,
                n,
                gap,
                dw,
            }
        })
        .collect();

    let width = |b: &Block| b.x1 - b.x0;
    let clock_i = (0..blocks.len())
        .filter(|&i| blocks[i].n >= 3)
        .max_by(|&a, &b| (blocks[a].n, width(&blocks[a]) as i64).cmp(&(blocks[b].n, width(&blocks[b]) as i64)));
    let rest: Vec<usize> = (0..blocks.len()).filter(|&i| Some(i) != clock_i).collect();
    let (mut left, mut right, mut period) = (None, None, None);
    match (rest.len(), clock_i) {
        (0, _) => {}
        (1, Some(c)) => {
            if blocks[rest[0]].x0 < blocks[c].x0 {
                left = Some(rest[0]);
            } else {
                right = Some(rest[0]);
            }
        }
        (1, None) => left = Some(rest[0]),
        (2, _) => {
            left = Some(rest[0]);
            right = Some(rest[1]);
        }
        _ => {
            left = Some(rest[0]);
            right = rest.last().copied();
            let mid = &rest[1..rest.len() - 1];
            period = match clock_i {
                Some(c) => mid
                    .iter()
                    .copied()
                    .min_by(|&a, &b| {
                        let da = (blocks[a].x0 - blocks[c].x1).abs().min((blocks[a].x1 - blocks[c].x0).abs());
                        let db = (blocks[b].x0 - blocks[c].x1).abs().min((blocks[b].x1 - blocks[c].x0).abs());
                        da.total_cmp(&db)
                    }),
                None => mid.first().copied(),
            };
        }
    }

    // frame = outline of the slanted digits (the width includes the offset |slant|·height)
    let field = |b: &Block, digits: usize, x0: f64| {
        let bh = b.y1 - b.y0;
        let sh = slant.abs() * bh;
        FieldCfg {
            rect: [x0 - pad as f64 - sh / 2.0, b.y0, (b.x1 - x0) + sh, bh],
            digits,
            gap_ratio: b.gap,
            slant,
            colon: 0.0,
        }
    };
    let mut out: Vec<(&'static str, FieldCfg)> = vec![];
    // score: a single visible character usually means "tens digit blank"
    let score = |i: usize| -> FieldCfg {
        let b = &blocks[i];
        if b.n == 1 && clock_i.is_some() {
            let x0 = b.x0 - b.dw * 1.25;
            FieldCfg { gap_ratio: 0.25, ..field(b, 2, x0) }
        } else {
            field(b, b.n.min(3), b.x0)
        }
    };
    if let Some(i) = left {
        out.push(("left_score", score(i)));
    }
    if let Some(i) = right {
        out.push(("right_score", score(i)));
    }
    if let Some(i) = clock_i {
        out.push(("clock", field(&blocks[i], blocks[i].n.min(4), blocks[i].x0)));
    }
    if let Some(i) = period {
        out.push(("period", field(&blocks[i], 1, blocks[i].x0)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::seven_seg::mask_of;

    const W: usize = 480;
    const H: usize = 270;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32) / (1u64 << 31) as f32
        }
    }

    const SEGS: [&str; 10] = ["abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg"];

    fn fill(frame: &mut [u8], w: usize, r: [f64; 4], rgb: [f32; 3]) {
        for y in r[1] as usize..(r[3].ceil() as usize) {
            for x in r[0] as usize..(r[2].ceil() as usize) {
                let o = (y * w + x) * 3;
                for k in 0..3 {
                    frame[o + k] = frame[o + k].max(rgb[k] as u8);
                }
            }
        }
    }

    fn draw_field(frame: &mut [u8], w: usize, f: &FieldCfg, digits: &[Option<u8>], rng: &mut Rng, common: f32, duty: f32) {
        for (rects, d) in f.segment_rects().iter().zip(digits) {
            let mask = d.map(|d| mask_of(SEGS[d as usize])).unwrap_or(0);
            for (i, r) in rects.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    let lvl = if rng.next() < duty { 235.0 } else { 50.0 + 40.0 * rng.next() } * common;
                    fill(frame, w, *r, [lvl, lvl * 0.3, lvl * 0.1]);
                }
            }
        }
    }

    fn board_fields() -> [FieldCfg; 4] {
        let mk = |x: f64, w: f64, n: usize| FieldCfg { rect: [x, 90.0, w, 40.0], digits: n, gap_ratio: 0.3, slant: 0.0, colon: 0.0 };
        [mk(60.0, 60.0, 2), mk(330.0, 60.0, 2), mk(170.0, 130.0, 4), mk(222.0, 18.0, 1)]
    }

    #[test]
    fn finds_flickering_board_among_distractors() {
        let [l, r, c, _] = board_fields();
        let mut rng = Rng(11);
        let mut b = BurstBuilder::new(W, H, 4);
        for i in 0..48 {
            let t = i as f64 / 8.0;
            let mut fr = vec![0u8; W * H * 3];
            for p in fr.chunks_exact_mut(3) {
                p.copy_from_slice(&[40, 36, 30]);
            }
            // dark housing
            for y in 70..150 {
                for x in 30..450 {
                    let o = (y * W + x) * 3;
                    fr[o..o + 3].copy_from_slice(&[12, 10, 10]);
                }
            }
            // distractors: a court line (static, bright) and a moving bright "player"
            fill(&mut fr, W, [0.0, 220.0, 480.0, 223.0], [210.0, 210.0, 210.0]);
            let px = 40.0 + t * 50.0;
            fill(&mut fr, W, [px, 180.0, px + 14.0, 214.0], [230.0, 120.0, 40.0]);
            let common = 0.75 + 0.25 * (t * 5.0).cos() as f32;
            let secs = 8 * 60 + 12 - t as u32;
            let (m, s) = (secs / 60, secs % 60);
            let two = |n: u32| [if n >= 10 { Some((n / 10) as u8) } else { None }, Some((n % 10) as u8)];
            draw_field(&mut fr, W, &l, &two(2), &mut rng, common, 0.6);
            draw_field(&mut fr, W, &r, &two(11), &mut rng, common, 0.6);
            draw_field(
                &mut fr,
                W,
                &c,
                &[Some((m / 10) as u8), Some((m % 10) as u8), Some((s / 10) as u8), Some((s % 10) as u8)],
                &mut rng,
                common,
                0.6,
            );
            b.push(&fr);
        }
        let burst = b.finish();
        let cands = find_boards(&burst);
        assert!(!cands.is_empty(), "no candidates");
        let q = cands[0].corners;
        let (x0, x1) = (q.iter().map(|p| p[0]).fold(f64::MAX, f64::min), q.iter().map(|p| p[0]).fold(f64::MIN, f64::max));
        let (y0, y1) = (q.iter().map(|p| p[1]).fold(f64::MAX, f64::min), q.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
        // expected: visible digits from x=94 (left tens digit blank, room before it) to 390, y=90..130
        assert!(x0 <= 75.0 && x0 >= 40.0, "x0 {x0}");
        assert!(x1 >= 388.0 && x1 <= 440.0, "x1 {x1}");
        assert!(y0 <= 92.0 && y0 >= 60.0, "y0 {y0}");
        assert!(y1 >= 128.0 && y1 <= 160.0, "y1 {y1}");
    }

    #[test]
    fn proposes_slanted_blocks() {
        let (w, h) = (1600usize, 300usize);
        let sl = 0.15;
        let sh = sl * 180.0;
        let mk = |x: f64, ww: f64, n: usize| FieldCfg { rect: [x, 60.0, ww + sh, 180.0], digits: n, gap_ratio: 0.3, slant: sl, colon: 0.0 };
        let fields = [mk(100.0, 248.0, 2), mk(520.0, 529.0, 4), mk(1150.0, 108.0, 1), mk(1340.0, 248.0, 2)];
        let mut rng = Rng(9);
        let mut rgb = vec![10u8; w * h * 3];
        for f in &fields {
            let digs: Vec<Option<u8>> = vec![Some(8); f.digits];
            draw_field(&mut rgb, w, f, &digs, &mut rng, 1.0, 1.0);
        }
        let v: Vec<u8> = (0..w * h).map(|i| rgb[i * 3].max(rgb[i * 3 + 1]).max(rgb[i * 3 + 2])).collect();
        let out = propose_fields(&v, w, h);
        let get = |k: &str| out.iter().find(|(n, _)| *n == k).map(|(_, f)| f.clone()).unwrap_or_else(|| panic!("{k} missing"));
        let (l, r, c, p) = (get("left_score"), get("right_score"), get("clock"), get("period"));
        assert_eq!((l.digits, r.digits, c.digits, p.digits), (2, 2, 4, 1));
        assert!((c.slant - sl).abs() < 0.05, "slant {}", c.slant);
        assert!((c.rect[0] - 520.0).abs() < 14.0 && (c.rect[2] - (529.0 + sh)).abs() < 20.0, "{:?}", c.rect);
        assert!((c.gap_ratio - 0.3).abs() < 0.12, "{}", c.gap_ratio);
    }

    #[test]
    fn proposes_blocks() {
        let (w, h) = (1600usize, 300usize);
        let mut v = vec![10u8; w * h * 3 / 3];
        let mut rng = Rng(5);
        let mk = |x: f64, ww: f64, n: usize| FieldCfg { rect: [x, 60.0, ww, 180.0], digits: n, gap_ratio: 0.3, slant: 0.0, colon: 0.0 };
        let fields = [("left", mk(100.0, 248.0, 2)), ("clock", mk(520.0, 529.0, 4)), ("period", mk(1150.0, 108.0, 1)), ("right", mk(1340.0, 248.0, 2))];
        let mut rgb = vec![10u8; w * h * 3];
        for (_, f) in &fields {
            // all digits as "8" (overlay of all states)
            let digs: Vec<Option<u8>> = vec![Some(8); f.digits];
            draw_field(&mut rgb, w, f, &digs, &mut rng, 1.0, 1.0);
        }
        // colon in the clock, close to the digits (gap ~9 px)
        fill(&mut rgb, w, [777.0, 110.0, 791.0, 124.0], [235.0, 70.0, 25.0]);
        fill(&mut rgb, w, [777.0, 170.0, 791.0, 184.0], [235.0, 70.0, 25.0]);
        for i in 0..w * h {
            v[i] = rgb[i * 3].max(rgb[i * 3 + 1]).max(rgb[i * 3 + 2]);
        }
        let out = propose_fields(&v, w, h);
        let get = |k: &str| out.iter().find(|(n, _)| *n == k).map(|(_, f)| f.clone());
        let (l, r, c, p) = (get("left_score"), get("right_score"), get("clock"), get("period"));
        let (l, r, c, p) = (l.expect("left"), r.expect("right"), c.expect("clock"), p.expect("period"));
        assert_eq!((l.digits, r.digits, c.digits, p.digits), (2, 2, 4, 1));
        assert!((l.rect[0] - 102.0).abs() < 6.0 && (l.rect[2] - 244.0).abs() < 10.0, "{:?}", l.rect);
        assert!((c.rect[0] - 522.0).abs() < 6.0 && (c.rect[2] - 525.0).abs() < 14.0, "{:?}", c.rect);
        assert!((r.rect[0] - 1342.0).abs() < 8.0, "{:?}", r.rect);
        assert!((c.gap_ratio - 0.3).abs() < 0.12, "{}", c.gap_ratio);
        assert!((p.rect[0] - 1152.0).abs() < 8.0, "{:?}", p.rect);
        assert!((l.rect[1] - 63.6).abs() < 4.0 && (l.rect[3] - 173.0).abs() < 6.0, "{:?}", l.rect);
    }
}
