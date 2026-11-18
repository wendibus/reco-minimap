//! Reading small boards: every digit is compared with drawn seven-segment digits.
//!
//! The segment reader in [`super::detect`] samples fixed areas per segment. On a board that is
//! only a few pixels high in the video (a table scoreboard across the hall: digits of 4 × 9
//! pixels) those areas are smaller than a pixel, the board drifts by a pixel or two over a game
//! (stitching, a camera that settles) and the colour channels are only stored at half the
//! resolution. This reader therefore works on the brightness only and per digit:
//!
//! * over a short window (half a second, against flicker and multiplexed rows) the bright
//!   percentile of every pixel,
//! * every digit compared with the ten digits drawn at its position (correlation), each also
//!   shifted by half a pixel; the best one wins, the gap to the second best is the confidence,
//! * a leading digit that is much darker than the lit ones is blank,
//! * the drift of the board is followed by comparing the edges of the dark picture of the last
//!   half minute with the one at the start.
//!
//! Every window gives one raw [`Reading`] (unsure parts `None`); the course of the game is
//! fitted to them afterwards ([`super::timeline`]).

use super::quad::{self, Mat3};
use super::{Config, FieldCfg, HomeSide, Reading};

/// Segment centre lines of a digit, in units of its box (x 0..1, y 0..1): a b c d e f g.
const LINES: [[f64; 4]; 7] = [
    [0.12, 0.0, 0.88, 0.0],
    [1.0, 0.08, 1.0, 0.44],
    [1.0, 0.56, 1.0, 0.92],
    [0.12, 1.0, 0.88, 1.0],
    [0.0, 0.56, 0.0, 0.92],
    [0.0, 0.08, 0.0, 0.44],
    [0.12, 0.5, 0.88, 0.5],
];
const MASKS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
/// Supersampling when drawing.
const SS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Score,
    Clock,
    Period,
}

/// One digit position: its patch in the region and the drawn digits (0..9) for every small shift.
struct Cell {
    lines: [[f64; 4]; 7],
    stroke: f64,
    /// jitter step (half a pixel or a little more for large digits)
    jit: f64,
    /// x0, y0, x1, y1 (exclusive) in region pixels, without drift
    base: [i32; 4],
    /// the patch the templates are drawn for (moved with the drift)
    roi: [i32; 4],
    /// drift the templates are drawn for
    drawn_for: (f32, f32),
    /// per small shift: ten templates (mean-free, unit length) and the mask of the "8" (on/off pixels)
    shifted: Vec<([Vec<f32>; 10], Vec<u8>)>,
}

impl Cell {
    fn new(lines: [[f64; 4]; 7]) -> Self {
        let xs = lines.iter().flat_map(|l| [l[0], l[2]]);
        let ys = lines.iter().flat_map(|l| [l[1], l[3]]);
        let (x0, x1) = xs.fold((f64::MAX, f64::MIN), |a, v| (a.0.min(v), a.1.max(v)));
        let (y0, y1) = ys.fold((f64::MAX, f64::MIN), |a, v| (a.0.min(v), a.1.max(v)));
        let stroke = (0.28 * (x1 - x0)).max(1.0);
        let m = stroke / 2.0 + 1.5;
        let base = [(x0 - m).floor() as i32, (y0 - m).floor() as i32, (x1 + m).ceil() as i32, (y1 + m).ceil() as i32];
        let mut c = Cell { lines, stroke, jit: (0.06 * (x1 - x0)).max(0.5), base, roi: base, drawn_for: (f32::NAN, f32::NAN), shifted: Vec::new() };
        c.draw_for((0.0, 0.0));
        c
    }

    /// Draw the templates for the board moved by `shift` (sub-pixel), each also a small step around it.
    fn draw_for(&mut self, shift: (f32, f32)) {
        let (ix, iy) = (shift.0.round() as i32, shift.1.round() as i32);
        self.roi = [self.base[0] + ix, self.base[1] + iy, self.base[2] + ix, self.base[3] + iy];
        let j = self.jit;
        self.shifted.clear();
        for dy in [-j, 0.0, j] {
            for dx in [-j, 0.0, j] {
                let (ox, oy) = (shift.0 as f64 + dx, shift.1 as f64 + dy);
                let t: [Vec<f32>; 10] = std::array::from_fn(|d| normalized(draw(&self.lines, MASKS[d], self.roi, self.stroke, ox, oy)));
                let eight = draw(&self.lines, 0x7F, self.roi, self.stroke, ox, oy).iter().map(|v| if *v > 0.5 { 2 } else if *v < 0.05 { 0 } else { 1 }).collect();
                self.shifted.push((t, eight));
            }
        }
        self.drawn_for = shift;
    }
}

struct Field {
    kind: Kind,
    cells: Vec<Cell>,
    /// usual brightness of the lit segments (forgets slowly)
    level: f32,
}

/// Values of one window.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Raw {
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub clock: Option<u32>,
    pub period: Option<u32>,
}

pub struct CellReader {
    fields: Vec<Field>,
    w: usize,
    h: usize,
    /// luma of the frames of the current window
    frames: Vec<Vec<f32>>,
    window: usize,
    /// bright pictures of the recent windows (for the drift), newest last
    recent: Vec<Vec<f32>>,
    recent_n: usize,
    reference: Option<Vec<f32>>,
    estimates: Vec<(f32, f32)>,
    /// current drift of the board (region pixels)
    pub shift: (f32, f32),
    windows: usize,
    home: HomeSide,
    /// minimum confidence (correlation gap) for a score digit / a clock digit
    pub min_conf_score: f32,
    pub min_conf_clock: f32,
}

fn luma(rgb: &[u8]) -> Vec<f32> {
    rgb.chunks_exact(3).map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).collect()
}

/// Draw a digit: `lines` are the segment centre lines in region pixels, `mask` the lit segments.
fn draw(lines: &[[f64; 4]; 7], mask: u8, roi: [i32; 4], stroke: f64, dx: f64, dy: f64) -> Vec<f32> {
    let (w, h) = ((roi[2] - roi[0]) as usize, (roi[3] - roi[1]) as usize);
    let mut out = vec![0f32; w * h];
    let half = stroke / 2.0;
    for py in 0..h * SS {
        let y = roi[1] as f64 + (py as f64 + 0.5) / SS as f64 - dy;
        for px in 0..w * SS {
            let x = roi[0] as f64 + (px as f64 + 0.5) / SS as f64 - dx;
            let mut on = false;
            for (i, l) in lines.iter().enumerate() {
                if mask & (1 << i) == 0 {
                    continue;
                }
                let (vx, vy) = (l[2] - l[0], l[3] - l[1]);
                let len2 = (vx * vx + vy * vy).max(1e-9);
                let t = (((x - l[0]) * vx + (y - l[1]) * vy) / len2).clamp(0.0, 1.0);
                let (cx, cy) = (l[0] + t * vx - x, l[1] + t * vy - y);
                if cx * cx + cy * cy <= half * half {
                    on = true;
                    break;
                }
            }
            if on {
                out[(py / SS) * w + px / SS] += 1.0 / (SS * SS) as f32;
            }
        }
    }
    out
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let m = v.iter().sum::<f32>() / v.len().max(1) as f32;
    v.iter_mut().for_each(|x| *x -= m);
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

/// Segment centre lines of every digit of a field, in region pixels.
fn field_lines(cfg: &Config, f: &FieldCfg, sx: f64, sy: f64, ox: f64, oy: f64) -> Vec<[[f64; 4]; 7]> {
    let h: Option<Mat3> = cfg.board.as_ref().and_then(|b| b.to_frame());
    let map = |x: f64, y: f64| {
        let (fx, fy) = match &h {
            Some(m) => quad::apply(m, x, y),
            None => (x, y),
        };
        (fx * sx - ox, fy * sy - oy)
    };
    f.segment_rects()
        .iter()
        .map(|d| {
            // the digit box from its segment rects: a (top), d (bottom), f/e (left), b/c (right)
            let x0 = (d[5][0] + d[5][2]) / 2.0;
            let x1 = (d[1][0] + d[1][2]) / 2.0;
            let y0 = (d[0][1] + d[0][3]) / 2.0;
            let y1 = (d[3][1] + d[3][3]) / 2.0;
            let (bw, bh) = (x1 - x0, y1 - y0);
            let ym = (y0 + y1) / 2.0;
            std::array::from_fn(|i| {
                let l = LINES[i];
                let p = |u: f64, v: f64| {
                    let y = y0 + v * bh;
                    // slant: leaning right at the top
                    map(x0 + u * bw + f.slant * (ym - y), y)
                };
                let (a, b) = (p(l[0], l[1]), p(l[2], l[3]));
                [a.0, a.1, b.0, b.1]
            })
        })
        .collect()
}

impl CellReader {
    /// Reader for frames that contain `region` (`[x, y, w, h]`) of a `full_w` × `full_h` video.
    pub fn new(cfg: &Config, full_w: usize, full_h: usize, region: [usize; 4], fps: f64) -> Self {
        let sx = full_w as f64 / cfg.frame[0] as f64;
        let sy = full_h as f64 / cfg.frame[1] as f64;
        let (ox, oy) = (region[0] as f64, region[1] as f64);
        let mut fields = vec![(Kind::Score, &cfg.left_score), (Kind::Score, &cfg.right_score), (Kind::Clock, &cfg.clock)];
        if let Some(p) = &cfg.period {
            fields.push((Kind::Period, p));
        }
        let fields = fields
            .into_iter()
            .map(|(kind, f)| {
                let cells = field_lines(cfg, f, sx, sy, ox, oy).into_iter().map(Cell::new).collect();
                Field { kind, cells, level: 0.0 }
            })
            .collect();
        let window = ((fps * 0.5).round() as usize).max(3);
        Self {
            fields,
            w: region[2],
            h: region[3],
            frames: Vec::with_capacity(window),
            window,
            recent: Vec::new(),
            // half a minute of windows
            recent_n: 60,
            reference: None,
            estimates: Vec::new(),
            shift: (0.0, 0.0),
            windows: 0,
            home: cfg.home,
            min_conf_score: 0.02,
            min_conf_clock: 0.015,
        }
    }

    /// The edges of the board as read so far (after at least ten seconds), as the drift
    /// reference for another reader of the same board.
    pub fn reference_now(&self) -> Option<Vec<f32>> {
        (self.recent.len() >= 20).then(|| self.edges())
    }

    pub fn set_reference(&mut self, r: Vec<f32>) {
        if r.len() == self.w * self.h {
            self.reference = Some(r);
        }
    }

    /// Length of a window in frames.
    pub fn window_frames(&self) -> usize {
        self.window
    }

    /// Add one RGB24 frame of the region. At the end of every window the values read in it.
    pub fn push_frame(&mut self, rgb: &[u8]) -> Option<Raw> {
        debug_assert_eq!(rgb.len(), self.w * self.h * 3);
        self.frames.push(luma(rgb));
        if self.frames.len() < self.window {
            return None;
        }
        let img = self.bright();
        self.frames.clear();
        self.track_drift(&img);
        self.windows += 1;
        Some(self.read(&img))
    }

    /// Per pixel the bright percentile of the window, minus the median of the region.
    fn bright(&self) -> Vec<f32> {
        let n = self.frames.len();
        let k = ((n - 1) as f32 * 0.9).round() as usize;
        let mut col = vec![0f32; n];
        let mut out = vec![0f32; self.w * self.h];
        for (i, o) in out.iter_mut().enumerate() {
            for (j, f) in self.frames.iter().enumerate() {
                col[j] = f[i];
            }
            col.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
            *o = col[k];
        }
        let mut s = out.clone();
        let mid = s.len() / 2;
        s.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
        let bg = s[mid];
        out.iter_mut().for_each(|v| *v -= bg);
        out
    }

    /// Edges of the dark picture (low percentile over the recent windows: LEDs and passers-by mostly gone).
    fn edges(&self) -> Vec<f32> {
        let n = self.recent.len();
        let k = ((n - 1) as f32 * 0.2).round() as usize;
        let mut col = vec![0f32; n];
        let mut low = vec![0f32; self.w * self.h];
        for (i, o) in low.iter_mut().enumerate() {
            for (j, f) in self.recent.iter().enumerate() {
                col[j] = f[i];
            }
            col.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
            *o = col[k];
        }
        let (w, h) = (self.w, self.h);
        let mut g = vec![0f32; w * h];
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let gx = low[y * w + x + 1] - low[y * w + x - 1];
                let gy = low[(y + 1) * w + x] - low[(y - 1) * w + x];
                g[y * w + x] = (gx * gx + gy * gy).sqrt();
            }
        }
        g
    }

    fn track_drift(&mut self, img: &[f32]) {
        self.recent.push(img.to_vec());
        if self.recent.len() > self.recent_n {
            self.recent.remove(0);
        }
        // every ten seconds over up to the last half minute; without a reference the first ten
        // seconds become it
        if self.recent.len() < 20 || self.windows % 20 != 0 {
            return;
        }
        let e = self.edges();
        let Some(r) = self.reference.as_ref() else {
            self.reference = Some(e);
            return;
        };
        let (w, h) = (self.w as i32, self.h as i32);
        let sample = |x: f32, y: f32| -> f32 {
            if x < 0.0 || y < 0.0 || x >= (w - 1) as f32 || y >= (h - 1) as f32 {
                return 0.0;
            }
            let (x0, y0) = (x as usize, y as usize);
            let (fx, fy) = (x - x0 as f32, y - y0 as f32);
            let i = y0 * w as usize + x0;
            e[i] * (1.0 - fx) * (1.0 - fy) + e[i + 1] * fx * (1.0 - fy) + e[i + w as usize] * (1.0 - fx) * fy + e[i + w as usize + 1] * fx * fy
        };
        let m = 4;
        let mut best = (f32::MIN, self.shift);
        let mut sy = -3.0f32;
        while sy <= 3.0 {
            let mut sx = -4.0f32;
            while sx <= 4.0 {
                let (mut ab, mut aa, mut bb) = (0f32, 0f32, 0f32);
                for y in m..h - m {
                    for x in m..w - m {
                        let a = r[(y * w + x) as usize];
                        let b = sample(x as f32 + sx, y as f32 + sy);
                        ab += a * b;
                        aa += a * a;
                        bb += b * b;
                    }
                }
                let c = ab / (aa * bb).sqrt().max(1e-6);
                if c > best.0 {
                    best = (c, (sx, sy));
                }
                sx += 0.25;
            }
            sy += 0.25;
        }
        if best.0 > 0.5 {
            // median of the last estimates: single estimates jitter by a quarter pixel or so
            self.estimates.push(best.1);
            if self.estimates.len() > 5 {
                self.estimates.remove(0);
            }
            let med = |mut v: Vec<f32>| {
                v.sort_by(|a, b| a.total_cmp(b));
                v[v.len() / 2]
            };
            self.shift = (med(self.estimates.iter().map(|e| e.0).collect()), med(self.estimates.iter().map(|e| e.1).collect()));
            let shift = self.shift;
            for f in &mut self.fields {
                for c in &mut f.cells {
                    if (c.drawn_for.0 - shift.0).abs() > 0.1 || (c.drawn_for.1 - shift.1).abs() > 0.1 {
                        c.draw_for(shift);
                    }
                }
            }
        }
    }

    /// Best digit of a cell: (digit, confidence, brightness of its segments).
    fn classify(&self, img: &[f32], c: &Cell) -> Option<(u8, f32, f32)> {
        let [x0, y0, x1, y1] = c.roi;
        if x0 < 0 || y0 < 0 || x1 > self.w as i32 || y1 > self.h as i32 {
            return None;
        }
        let cw = (x1 - x0) as usize;
        let mut patch = Vec::with_capacity(cw * (y1 - y0) as usize);
        for y in y0..y1 {
            let row = y as usize * self.w;
            patch.extend_from_slice(&img[row + x0 as usize..row + x1 as usize]);
        }
        let p = normalized(patch.clone());
        let mut score = [f32::MIN; 10];
        let mut lit = f32::MIN;
        for (t, eight) in &c.shifted {
            for d in 0..10 {
                let s: f32 = t[d].iter().zip(&p).map(|(a, b)| a * b).sum();
                score[d] = score[d].max(s);
            }
            let (mut on, mut n_on) = (0f32, 0usize);
            let mut off: Vec<f32> = Vec::new();
            for (v, m) in patch.iter().zip(eight) {
                match m {
                    2 => {
                        on += v;
                        n_on += 1;
                    }
                    0 => off.push(*v),
                    _ => {}
                }
            }
            if n_on > 0 && !off.is_empty() {
                let mid = off.len() / 2;
                off.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
                lit = lit.max(on / n_on as f32 - off[mid]);
            }
        }
        let mut order: Vec<usize> = (0..10).collect();
        order.sort_by(|a, b| score[*b].total_cmp(&score[*a]));
        Some((order[0] as u8, score[order[0]] - score[order[1]], lit))
    }

    fn read(&mut self, img: &[f32]) -> Raw {
        let (mcs, mcc) = (self.min_conf_score, self.min_conf_clock);
        let mut vals: Vec<Option<u32>> = Vec::with_capacity(self.fields.len());
        for fi in 0..self.fields.len() {
            let res: Vec<Option<(u8, f32, f32)>> = self.fields[fi].cells.iter().map(|c| self.classify(img, c)).collect();
            let f = &mut self.fields[fi];
            let top = res.iter().flatten().map(|r| r.2).fold(f32::MIN, f32::max);
            if top > f.level {
                f.level = top;
            } else if top > 0.0 {
                f.level += (top - f.level) * 0.0005;
            }
            let n = res.len();
            let min_conf = if f.kind == Kind::Clock { mcc } else { mcs };
            // digits: None = blank, Some(d); a failed cell or a too unsure digit makes the field unreadable
            let mut digits: Vec<Option<u8>> = Vec::with_capacity(n);
            let mut ok = top > 0.0;
            for (i, r) in res.iter().enumerate() {
                match r {
                    None => ok = false,
                    // a leading cell much darker than the lit ones is blank
                    Some((_, _, lit)) if i + 1 < n && *lit < 0.3 * top.max(0.5 * f.level) => digits.push(None),
                    Some((d, conf, _)) => {
                        if *conf < min_conf {
                            ok = false;
                        }
                        digits.push(Some(*d));
                    }
                }
            }
            vals.push(if ok { value(f.kind, &digits) } else { None });
        }
        let (left, right, clock) = (vals[0], vals[1], vals[2]);
        Raw { left, right, clock, period: vals.get(3).copied().flatten() }
    }

    /// Raw values as a reading with home / away.
    pub fn reading(&self, t: f64, r: &Raw) -> Reading {
        let (home, away) = match self.home {
            HomeSide::Left => (r.left, r.right),
            HomeSide::Right => (r.right, r.left),
        };
        Reading { t, home, away, clock: r.clock, period: r.period }
    }
}

fn value(kind: Kind, digits: &[Option<u8>]) -> Option<u32> {
    match kind {
        Kind::Clock => {
            let n = digits.len();
            if n < 3 {
                return None;
            }
            let (s1, s2) = (digits[n - 2]?, digits[n - 1]?);
            if s1 > 5 {
                return None;
            }
            let mut minutes = 0u32;
            let mut started = false;
            for d in &digits[..n - 2] {
                match d {
                    Some(v) => {
                        minutes = minutes * 10 + *v as u32;
                        started = true;
                    }
                    None if started => return None,
                    None => {}
                }
            }
            // the minute digit right before the seconds must be lit ("0:45", not ":45")
            digits[n - 3]?;
            Some(minutes * 60 + s1 as u32 * 10 + s2 as u32)
        }
        Kind::Score | Kind::Period => {
            let mut v = 0u32;
            let mut started = false;
            for d in digits {
                match d {
                    Some(x) => {
                        v = v * 10 + *x as u32;
                        started = true;
                    }
                    None if started => return None,
                    None => {}
                }
            }
            if !started || (kind == Kind::Period && !(1..=9).contains(&v)) {
                return None;
            }
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DetectCfg, FieldCfg};
    use super::*;

    const W: usize = 64;
    const H: usize = 36;

    /// A tiny board like a table scoreboard across a hall: digits of about 4 × 9 pixels.
    fn cfg() -> Config {
        Config {
            frame: [W as u32, H as u32],
            board: None,
            detect: DetectCfg::default(),
            left_score: FieldCfg { rect: [8.0, 20.0, 10.0, 10.0], digits: 2, gap_ratio: 0.2, slant: 0.0, colon: 0.0 },
            right_score: FieldCfg { rect: [40.0, 20.0, 10.0, 10.0], digits: 2, gap_ratio: 0.2, slant: 0.0, colon: 0.0 },
            clock: FieldCfg { rect: [14.0, 4.0, 30.0, 10.0], digits: 4, gap_ratio: 0.2, slant: 0.1, colon: 0.8 },
            period: None,
            home: HomeSide::Right,
            time: None,
        }
    }

    /// Draw the board into an RGB frame: LEDs at `lvl`, every second frame only the top or the
    /// bottom row lit (multiplexed), plus noise.
    fn frame(c: &Config, left: Option<u32>, right: Option<u32>, clock: u32, i: usize, seed: &mut u64) -> Vec<u8> {
        let mut lum = vec![30f32; W * H];
        let draw_field = |lum: &mut Vec<f32>, f: &FieldCfg, digits: &[Option<u8>]| {
            for (lines, d) in field_lines(c, f, 1.0, 1.0, 0.0, 0.0).iter().zip(digits) {
                let Some(d) = d else { continue };
                let img = draw(lines, MASKS[*d as usize], [0, 0, W as i32, H as i32], 1.1, 0.0, 0.0);
                for (o, v) in lum.iter_mut().zip(img) {
                    *o += 160.0 * v;
                }
            }
        };
        let two = |v: Option<u32>| -> [Option<u8>; 2] {
            match v {
                None => [None, None],
                Some(v) if v >= 10 => [Some((v / 10 % 10) as u8), Some((v % 10) as u8)],
                Some(v) => [None, Some(v as u8)],
            }
        };
        let (m, s) = (clock / 60, clock % 60);
        if i % 2 == 0 {
            draw_field(&mut lum, &c.clock, &[if m >= 10 { Some((m / 10) as u8) } else { None }, Some((m % 10) as u8), Some((s / 10) as u8), Some((s % 10) as u8)]);
        } else {
            draw_field(&mut lum, &c.left_score, &two(left));
            draw_field(&mut lum, &c.right_score, &two(right));
        }
        let mut out = Vec::with_capacity(W * H * 3);
        for v in lum {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let n = ((*seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 16.0;
            let g = (v + n).clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&[g, g, g]);
        }
        out
    }

    #[test]
    fn reads_a_tiny_multiplexed_board() {
        let c = cfg();
        let mut r = CellReader::new(&c, W, H, [0, 0, W, H], 60.0);
        let mut seed = 5u64;
        let mut got = Vec::new();
        for i in 0..240 {
            let clock = 9 * 60 + 41 - (i / 60) as u32;
            let f = frame(&c, Some(7), Some(23), clock, i, &mut seed);
            if let Some(raw) = r.push_frame(&f) {
                got.push((raw, clock));
            }
        }
        assert_eq!(got.len(), 8);
        let ok = got.iter().filter(|(raw, clock)| raw.left == Some(7) && raw.right == Some(23) && raw.clock.map(|c| c.abs_diff(*clock) <= 1).unwrap_or(false)).count();
        assert!(ok >= 7, "{got:?}");
        let rd = r.reading(1.0, &got[0].0);
        assert_eq!((rd.home, rd.away), (Some(23), Some(7)));
    }

    #[test]
    fn values() {
        assert_eq!(value(Kind::Clock, &[None, Some(9), Some(0), Some(1)]), Some(541));
        assert_eq!(value(Kind::Clock, &[Some(1), Some(0), Some(0), Some(0)]), Some(600));
        assert_eq!(value(Kind::Clock, &[None, Some(9), Some(7), Some(1)]), None);
        assert_eq!(value(Kind::Clock, &[None, None, Some(4), Some(1)]), None);
        assert_eq!(value(Kind::Score, &[None, Some(3)]), Some(3));
        assert_eq!(value(Kind::Score, &[Some(4), Some(5)]), Some(45));
        assert_eq!(value(Kind::Score, &[None, None]), None);
    }
}
