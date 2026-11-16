//! Reading the scoreboard from video frames.
//!
//! Against the flicker (PWM-dimmed LEDs): per segment the mean brightness of its fixed sample
//! area is collected over a short time window and a high percentile of it is taken ("was bright
//! recently"). Then a threshold between "on" and "off" is set per digit block, the segment
//! pattern decoded, and a new value is only accepted once it was read stably for a minimum time.

use super::seven_seg::{decode, Digit};
use super::{Channel, Config, FieldCfg};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug)]
pub struct PxRect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize, // exclusive
    pub y1: usize, // exclusive
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Score,
    Clock,
    Period,
}

struct Seg {
    rect: PxRect,
    ring: Vec<f32>,
    pos: usize,
    filtered: f32,
    on: bool,
}

/// Majority over the last N valid readings: outliers and short transitions (segments do not
/// switch at the same time) are ignored.
struct Stabilizer {
    accepted: Option<u32>,
    recent: VecDeque<u32>,
    n: usize,
    need: usize,
}

impl Stabilizer {
    fn new(stable_ms: f64, fps: f64, share: f64) -> Self {
        let n = ((stable_ms / 1000.0 * fps).ceil() as usize).max(3);
        let need = ((n as f64 * share).ceil() as usize).clamp(1, n);
        Self { accepted: None, recent: VecDeque::with_capacity(n + 1), n, need }
    }

    /// true when the accepted value changed. Unreadable frames (`None`, e.g. in a flicker
    /// trough) do not count: the decision is made over the last N *valid* readings.
    fn update(&mut self, cand: Option<u32>) -> bool {
        let Some(cand) = cand else { return false };
        self.recent.push_back(cand);
        if self.recent.len() > self.n {
            self.recent.pop_front();
        }
        if self.recent.len() < self.n {
            return false;
        }
        let mut counts: Vec<(u32, usize)> = Vec::with_capacity(4);
        for v in self.recent.iter() {
            match counts.iter_mut().find(|(k, _)| k == v) {
                Some((_, c)) => *c += 1,
                None => counts.push((*v, 1)),
            }
        }
        let Some(&(best, cnt)) = counts.iter().max_by_key(|(_, c)| *c) else {
            return false;
        };
        if cnt >= self.need && self.accepted != Some(best) {
            self.accepted = Some(best);
            return true;
        }
        false
    }
}

struct Field {
    kind: Kind,
    digits: Vec<[Seg; 7]>,
    /// Peak of the brightest segments (forgets slowly)
    hi: f32,
    stab: Stabilizer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    pub left: Option<u32>,
    pub right: Option<u32>,
    /// Game clock in seconds (minutes*60 + seconds)
    pub clock: Option<u32>,
    pub period: Option<u32>,
}

pub struct Detector {
    fields: Vec<Field>, // 0 left, 1 right, 2 clock, 3 period (optional)
    window: usize,
    percentile: f64,
    channel: Channel,
    invert: bool,
    min_contrast: f32,
    threshold: f32,
    track_k: f32,
    width: usize,
    height: usize,
    frames: usize,
    pub state: State,
}

/// Frames whose brightest segments are below this share of their peak are skipped.
const GUARD: f32 = 0.35;

fn scale_rect(r: &[f64; 4], sx: f64, sy: f64, w: usize, h: usize) -> PxRect {
    let x0 = ((r[0] * sx).floor().max(0.0) as usize).min(w.saturating_sub(1));
    let y0 = ((r[1] * sy).floor().max(0.0) as usize).min(h.saturating_sub(1));
    let x1 = ((r[2] * sx).ceil().max(0.0) as usize).clamp(x0 + 1, w);
    let y1 = ((r[3] * sy).ceil().max(0.0) as usize).clamp(y0 + 1, h);
    PxRect { x0, y0, x1, y1 }
}

/// Size of the full video frame and the part of it the detector receives (`[x, y, w, h]`).
#[derive(Clone, Copy)]
struct Geometry {
    full: (usize, usize),
    region: [usize; 4],
}

fn build_field(kind: Kind, f: &FieldCfg, cfg: &Config, g: Geometry, window: usize, stable_ms: f64, fps: f64, share: f64) -> Field {
    let sx = g.full.0 as f64 / cfg.frame[0] as f64;
    let sy = g.full.1 as f64 / cfg.frame[1] as f64;
    let [ox, oy, w, h] = g.region;
    let (ox, oy) = (ox as f64, oy as f64);
    let digits = cfg
        .field_segments(f)
        .iter()
        .map(|d| {
            std::array::from_fn(|i| Seg {
                rect: scale_rect(&[d[i][0] * sx - ox, d[i][1] * sy - oy, d[i][2] * sx - ox, d[i][3] * sy - oy], 1.0, 1.0, w, h),
                ring: vec![0.0; window],
                pos: 0,
                filtered: 0.0,
                on: false,
            })
        })
        .collect();
    Field { kind, digits, hi: 0.0, stab: Stabilizer::new(stable_ms, fps, share) }
}

fn sample(frame: &[u8], stride: usize, r: PxRect, ch: Channel, invert: bool) -> f32 {
    let mut sum: u64 = 0;
    let mut n: u64 = 0;
    for y in r.y0..r.y1 {
        let row = &frame[y * stride + r.x0 * 3..y * stride + r.x1 * 3];
        for px in row.chunks_exact(3) {
            let (r8, g8, b8) = (px[0] as u32, px[1] as u32, px[2] as u32);
            let v = match ch {
                Channel::Luma => (54 * r8 + 183 * g8 + 19 * b8) >> 8,
                Channel::Max => r8.max(g8).max(b8),
                Channel::Red => r8,
                Channel::Green => g8,
                Channel::Blue => b8,
            };
            sum += v as u64;
            n += 1;
        }
    }
    let m = sum as f32 / n.max(1) as f32;
    if invert { 255.0 - m } else { m }
}

fn percentile(ring: &[f32], p: f64) -> f32 {
    let mut buf = [0f32; 64];
    let n = ring.len().min(64);
    buf[..n].copy_from_slice(&ring[..n]);
    let s = &mut buf[..n];
    s.sort_unstable_by(|a, b| a.total_cmp(b));
    let idx = (((n - 1) as f64) * p).round() as usize;
    s[idx.min(n - 1)]
}

fn field_value(kind: Kind, digits: &[Digit]) -> Option<u32> {
    match kind {
        Kind::Score | Kind::Period => {
            let mut started = false;
            let mut v: u32 = 0;
            for d in digits {
                match d {
                    Digit::Blank => {
                        if started {
                            return None;
                        }
                    }
                    Digit::Num(n) => {
                        started = true;
                        v = v * 10 + *n as u32;
                    }
                    Digit::Unknown => return None,
                }
            }
            if !started {
                return None;
            }
            if kind == Kind::Period && !(1..=9).contains(&v) {
                return None;
            }
            Some(v)
        }
        Kind::Clock => {
            if digits.len() < 3 {
                return None;
            }
            let (min_d, sec_d) = digits.split_at(digits.len() - 2);
            let mut minutes = 0u32;
            for d in min_d {
                match d {
                    Digit::Blank => minutes *= 10,
                    Digit::Num(n) => minutes = minutes * 10 + *n as u32,
                    Digit::Unknown => return None,
                }
            }
            let (Digit::Num(s1), Digit::Num(s2)) = (sec_d[0], sec_d[1]) else {
                return None;
            };
            if s1 > 5 {
                return None;
            }
            Some(minutes * 60 + s1 as u32 * 10 + s2 as u32)
        }
    }
}

impl Detector {
    /// Detector for whole `width` × `height` frames.
    pub fn new(cfg: &Config, width: usize, height: usize, fps: f64) -> Self {
        Self::new_region(cfg, width, height, [0, 0, width, height], fps)
    }

    /// Detector for frames that only contain `region` (`[x, y, w, h]`) of a `full_w` × `full_h`
    /// video, e.g. an ffmpeg crop around the board.
    pub fn new_region(cfg: &Config, full_w: usize, full_h: usize, region: [usize; 4], fps: f64) -> Self {
        let d = &cfg.detect;
        let g = Geometry { full: (full_w, full_h), region };
        let (width, height) = (region[2], region[3]);
        let window = ((fps * d.window_ms / 1000.0).round() as usize).clamp(3, 64);
        let mut fields = vec![
            build_field(Kind::Score, &cfg.left_score, cfg, g, window, d.score_stable_ms, fps, d.score_share),
            build_field(Kind::Score, &cfg.right_score, cfg, g, window, d.score_stable_ms, fps, d.score_share),
            build_field(Kind::Clock, &cfg.clock, cfg, g, window, d.clock_stable_ms, fps, d.clock_share),
        ];
        if let Some(p) = &cfg.period {
            fields.push(build_field(Kind::Period, p, cfg, g, window, d.period_stable_ms, fps, d.score_share));
        }
        Self {
            fields,
            window,
            percentile: d.percentile,
            channel: d.channel,
            invert: d.invert,
            min_contrast: d.min_contrast as f32,
            threshold: d.threshold as f32,
            // the peak tracker forgets with a time constant of ~20 s
            track_k: (1.0 / (fps * 20.0)) as f32,
            width,
            height,
            frames: 0,
            state: State::default(),
        }
    }

    pub fn window_frames(&self) -> usize {
        self.window
    }

    /// Process one RGB24 frame (the size given to the constructor).
    /// true when the recognised state changed.
    pub fn push_frame(&mut self, frame: &[u8]) -> bool {
        debug_assert_eq!(frame.len(), self.width * self.height * 3);
        let stride = self.width * 3;
        let (ch, inv) = (self.channel, self.invert);

        for f in &mut self.fields {
            for d in &mut f.digits {
                for s in d.iter_mut() {
                    let v = sample(frame, stride, s.rect, ch, inv);
                    s.ring[s.pos] = v;
                    s.pos = (s.pos + 1) % s.ring.len();
                }
            }
        }
        self.frames += 1;
        if self.frames < self.window {
            return false;
        }

        let before = self.state;
        let (p, min_contrast, k, thr_frac) = (self.percentile, self.min_contrast, self.track_k, self.threshold);
        for f in &mut self.fields {
            let mut cur_hi = f32::MIN;
            let mut cur_lo = f32::MAX;
            for d in &mut f.digits {
                for s in d.iter_mut() {
                    s.filtered = percentile(&s.ring, p);
                    cur_hi = cur_hi.max(s.filtered);
                    cur_lo = cur_lo.min(s.filtered);
                }
            }
            f.hi = if cur_hi > f.hi { cur_hi } else { f.hi + (cur_hi - f.hi) * k };
            // threshold from the current frame -> dimming / flicker of the whole board moves
            // "on" and "off" together.
            let thr = cur_lo + thr_frac * (cur_hi - cur_lo);
            // only read when "on" and "off" are clearly apart and the board is not much darker
            // than usual (flicker trough).
            let reliable = cur_hi - cur_lo >= min_contrast && cur_hi >= GUARD * f.hi;

            let mut decoded = Vec::with_capacity(f.digits.len());
            for d in &mut f.digits {
                let mut mask = 0u8;
                for (i, s) in d.iter_mut().enumerate() {
                    s.on = s.filtered > thr;
                    if s.on {
                        mask |= 1 << i;
                    }
                }
                decoded.push(decode(mask));
            }
            let cand = if reliable { field_value(f.kind, &decoded) } else { None };
            f.stab.update(cand);
        }

        self.state = State {
            left: self.fields[0].stab.accepted,
            right: self.fields[1].stab.accepted,
            clock: self.fields[2].stab.accepted,
            period: self.fields.get(3).and_then(|f| f.stab.accepted),
        };
        self.state != before
    }

    /// Sample areas with their current on/off state (for debugging).
    pub fn segments(&self) -> impl Iterator<Item = (PxRect, bool)> + '_ {
        self.fields
            .iter()
            .flat_map(|f| f.digits.iter().flat_map(|d| d.iter().map(|s| (s.rect, s.on))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{DetectCfg, HomeSide};
    use super::super::seven_seg::mask_of;

    const W: usize = 640;
    const H: usize = 360;
    const FPS: f64 = 25.0;

    fn cfg() -> Config {
        Config {
            frame: [W as u32, H as u32],
            board: None,
            detect: DetectCfg::default(),
            left_score: FieldCfg { rect: [40.0, 40.0, 80.0, 50.0], digits: 2, gap_ratio: 0.25, slant: 0.0 },
            right_score: FieldCfg { rect: [500.0, 40.0, 80.0, 50.0], digits: 2, gap_ratio: 0.25, slant: 0.0 },
            clock: FieldCfg { rect: [240.0, 40.0, 160.0, 50.0], digits: 4, gap_ratio: 0.3, slant: 0.0 },
            period: Some(FieldCfg { rect: [300.0, 120.0, 20.0, 30.0], digits: 1, gap_ratio: 0.25, slant: 0.0 }),
            home: HomeSide::Left,
        }
    }

    fn mask_for(d: u8) -> u8 {
        let s = ["abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg"];
        mask_of(s[d as usize])
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32) / (1u64 << 31) as f32
        }
    }

    /// `common`: brightness factor of this frame for the whole board (PWM beat),
    /// `duty`: probability that a single segment is fully bright in this frame.
    fn draw(frame: &mut [u8], f: &FieldCfg, digits: &[Option<u8>], rng: &mut Rng, common: f32, duty: f32) {
        for (rects, d) in f.segment_rects().iter().zip(digits) {
            let mask = d.map(mask_for).unwrap_or(0);
            for (i, r) in rects.iter().enumerate() {
                if mask & (1 << i) == 0 {
                    continue;
                }
                let lvl = if rng.next() < duty { 235.0 } else { 40.0 + 40.0 * rng.next() } * common;
                for y in r[1] as usize..r[3].ceil() as usize {
                    for x in r[0] as usize..r[2].ceil() as usize {
                        let o = (y * W + x) * 3;
                        frame[o] = lvl as u8;
                        frame[o + 1] = (lvl * 0.3) as u8;
                        frame[o + 2] = (lvl * 0.1) as u8;
                    }
                }
            }
        }
    }

    fn digits2(n: u32) -> [Option<u8>; 2] {
        [if n >= 10 { Some((n / 10) as u8) } else { None }, Some((n % 10) as u8)]
    }

    /// Plays 12 s: the clock counts down from 08:12, left 0 -> 2 at t=4, right 0 -> 11 at t=7.
    fn run_scene(duty: f32, beat: bool) -> (State, usize) {
        let c = cfg();
        let mut det = Detector::new(&c, W, H, FPS);
        let mut rng = Rng(7);
        let mut changes = 0;
        for i in 0..(12.0 * FPS) as usize {
            let t = i as f64 / FPS;
            let mut frame = vec![0u8; W * H * 3];
            for p in frame.chunks_exact_mut(3) {
                p.copy_from_slice(&[22, 20, 20]);
            }
            // slow beat of the whole board between 15 % and 100 %
            let common = if beat { 0.575 + 0.425 * (t * 2.0 * std::f64::consts::PI * 0.9).cos() as f32 } else { 1.0 };
            let left = if t >= 4.0 { 2 } else { 0 };
            let right = if t >= 7.0 { 11 } else { 0 };
            let secs = 8 * 60 + 12 - t as u32;
            let (m, s) = (secs / 60, secs % 60);
            draw(&mut frame, &c.left_score, &digits2(left), &mut rng, common, duty);
            draw(&mut frame, &c.right_score, &digits2(right), &mut rng, common, duty);
            draw(
                &mut frame,
                &c.clock,
                &[Some((m / 10) as u8), Some((m % 10) as u8), Some((s / 10) as u8), Some((s % 10) as u8)],
                &mut rng,
                common,
                duty,
            );
            draw(&mut frame, c.period.as_ref().unwrap(), &[Some(2)], &mut rng, common, duty);
            if det.push_frame(&frame) {
                changes += 1;
            }
        }
        (det.state, changes)
    }

    fn check(st: State, changes: usize) {
        assert_eq!(st.left, Some(2));
        assert_eq!(st.right, Some(11));
        assert_eq!(st.period, Some(2));
        // t=11.96 -> 08:01; at most ~1 s behind with the latency
        let expect = 8 * 60 + 12 - 11;
        let got = st.clock.unwrap();
        assert!((expect..=expect + 1).contains(&got), "clock {got}");
        assert!(changes >= 10, "too few changes: {changes}");
    }

    #[test]
    fn reads_independent_flicker() {
        let (st, ch) = run_scene(0.8, false);
        check(st, ch);
    }

    #[test]
    fn reads_a_cropped_region_like_the_full_frame() {
        // the same scene, but the detector only gets the board region (as from an ffmpeg crop)
        let c = cfg();
        let region = c.region(W, H, 6);
        let mut full = Detector::new(&c, W, H, FPS);
        let mut crop = Detector::new_region(&c, W, H, region, FPS);
        let mut rng = Rng(3);
        let [rx, ry, rw, rh] = region;
        for i in 0..(6.0 * FPS) as usize {
            let t = i as f64 / FPS;
            let mut frame = vec![20u8; W * H * 3];
            let left = if t >= 2.0 { 3 } else { 1 };
            draw(&mut frame, &c.left_score, &digits2(left), &mut rng, 1.0, 0.8);
            draw(&mut frame, &c.right_score, &digits2(7), &mut rng, 1.0, 0.8);
            draw(&mut frame, &c.clock, &[Some(0), Some(5), Some(1), Some((9 - t as u32 % 10) as u8)], &mut rng, 1.0, 0.8);
            draw(&mut frame, c.period.as_ref().unwrap(), &[Some(1)], &mut rng, 1.0, 0.8);
            let mut part = Vec::with_capacity(rw * rh * 3);
            for y in ry..ry + rh {
                part.extend_from_slice(&frame[(y * W + rx) * 3..(y * W + rx + rw) * 3]);
            }
            full.push_frame(&frame);
            crop.push_frame(&part);
        }
        assert_eq!(full.state, crop.state);
        assert_eq!(crop.state.left, Some(3));
        assert_eq!(crop.state.right, Some(7));
    }

    #[test]
    fn reads_common_mode_beat() {
        let (st, ch) = run_scene(0.9, true);
        check(st, ch);
    }

    #[test]
    fn stabilizer_majority() {
        // 500 ms @ 10 fps = 5 frames, 70 % -> 4 needed
        let mut s = Stabilizer::new(500.0, 10.0, 0.7);
        for _ in 0..4 {
            assert!(!s.update(Some(5)));
        }
        assert!(s.update(Some(5))); // 5 von 5
        assert_eq!(s.accepted, Some(5));
        // a single outlier changes nothing
        assert!(!s.update(Some(9)));
        assert_eq!(s.accepted, Some(5));
        // unreadable frames are ignored
        for _ in 0..3 {
            assert!(!s.update(None));
        }
        assert_eq!(s.accepted, Some(5));
        // a new value needs 4 of 5
        for _ in 0..3 {
            assert!(!s.update(Some(7)));
        }
        assert!(s.update(Some(7)));
        assert_eq!(s.accepted, Some(7));
    }

    #[test]
    fn field_values() {
        use Digit::*;
        assert_eq!(field_value(Kind::Score, &[Blank, Num(7)]), Some(7));
        assert_eq!(field_value(Kind::Score, &[Num(1), Num(0)]), Some(10));
        assert_eq!(field_value(Kind::Score, &[Num(1), Blank]), None);
        assert_eq!(field_value(Kind::Score, &[Blank, Blank]), None);
        assert_eq!(field_value(Kind::Clock, &[Blank, Num(8), Num(3), Num(2)]), Some(8 * 60 + 32));
        assert_eq!(field_value(Kind::Clock, &[Num(0), Num(0), Num(6), Num(0)]), None);
        assert_eq!(field_value(Kind::Period, &[Num(0)]), None);
    }
}
