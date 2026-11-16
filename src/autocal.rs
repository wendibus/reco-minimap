//! Automatic field calibration: find the painted field lines in a frame and fit the field model to them.
//!
//! 1. **Line pixels:** painted lines are thin and brighter than the floor on both sides, so a pixel
//!    counts as line when it is clearly brighter than its neighbours a few pixels away on both sides,
//!    in at least one of four directions (a ridge test). Large bright areas (walls, lights) fail it.
//! 2. **Distance map:** for every pixel the distance to the nearest line pixel (chamfer transform).
//! 3. **Fit:** the field is placed in the image by its four corners (plus the fisheye strength). A
//!    placement is good when the field lines, drawn with it, run along line pixels: the cost is the
//!    mean (capped) distance of points sampled on the field lines. A coarse search over field shapes
//!    seen from a sideline camera gives starting points, Nelder–Mead refines the best of them.
//!
//! Which corners lie **outside the picture** is the one hint the user gives: it rules out the many
//! placements where a smaller or shifted field fits some of the lines.

use crate::geom::{Homography, Vec2};
use crate::lens::Lens;

/// Working image: luminance, scaled so that the width is at most [`WORK_W`].
const WORK_W: usize = 960;

pub struct LineMap {
    pub w: usize,
    pub h: usize,
    /// 1 = line pixel.
    pub mask: Vec<u8>,
    /// Distance to the nearest line pixel, in working pixels.
    dist: Vec<f32>,
}

impl LineMap {
    /// From an RGB24 frame.
    pub fn from_rgb(rgb: &[u8], w: usize, h: usize) -> LineMap {
        let f = (w as f32 / WORK_W as f32).max(1.0);
        let (ww, wh) = (((w as f32 / f).round() as usize).max(8), ((h as f32 / f).round() as usize).max(8));
        // box-filtered downscale of the luminance
        let mut lum = vec![0f32; ww * wh];
        for y in 0..wh {
            let (y0, y1) = (((y as f32 * f) as usize).min(h - 1), (((y + 1) as f32 * f) as usize).clamp(1, h));
            for x in 0..ww {
                let (x0, x1) = (((x as f32 * f) as usize).min(w - 1), (((x + 1) as f32 * f) as usize).clamp(1, w));
                let (mut s, mut n) = (0f32, 0f32);
                for yy in y0..y1.max(y0 + 1) {
                    for xx in x0..x1.max(x0 + 1) {
                        let i = (yy * w + xx) * 3;
                        s += 0.299 * rgb[i] as f32 + 0.587 * rgb[i + 1] as f32 + 0.114 * rgb[i + 2] as f32;
                        n += 1.0;
                    }
                }
                lum[y * ww + x] = s / n;
            }
        }
        let mask = ridges(&lum, ww, wh);
        let dist = chamfer(&mask, ww, wh);
        LineMap { w: ww, h: wh, mask, dist }
    }

    fn dist_at(&self, u: f32, v: f32) -> Option<f32> {
        let (x, y) = (u * self.w as f32, v * self.h as f32);
        if x < 0.0 || y < 0.0 || x >= self.w as f32 || y >= self.h as f32 {
            return None;
        }
        Some(self.dist[y as usize * self.w + x as usize])
    }

    /// The line pixels as an RGB image (magenta on black, drawn with "screen" over the frame).
    pub fn mask_rgb(&self) -> Vec<u8> {
        self.mask.iter().flat_map(|&m| if m > 0 { [255u8, 0, 255] } else { [0u8, 0, 0] }).collect()
    }
}

/// Ridge test: brighter than both neighbours `d` pixels away along one of four directions.
fn ridges(lum: &[f32], w: usize, h: usize) -> Vec<u8> {
    let d = ((w as f32 / 320.0).round() as isize).max(2);
    let dirs: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];
    let mut resp = vec![0f32; w * h];
    let at = |x: isize, y: isize| lum[y as usize * w + x as usize];
    for y in d..h as isize - d {
        for x in d..w as isize - d {
            let c = at(x, y);
            let mut best = 0f32;
            for (dx, dy) in dirs {
                let a = c - at(x + dx * d, y + dy * d);
                let b = c - at(x - dx * d, y - dy * d);
                best = best.max(a.min(b));
            }
            resp[y as usize * w + x as usize] = best;
        }
    }
    // threshold: clearly above the typical texture of the frame
    let mut sorted: Vec<f32> = resp.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p995 = sorted.get(sorted.len() * 995 / 1000).copied().unwrap_or(0.0);
    let thr = (0.25 * p995).max(14.0);
    resp.iter().map(|&r| (r > thr) as u8).collect()
}

/// 3-4 chamfer distance transform (result in pixels).
fn chamfer(mask: &[u8], w: usize, h: usize) -> Vec<f32> {
    const BIG: f32 = 1e6;
    let mut d: Vec<f32> = mask.iter().map(|&m| if m > 0 { 0.0 } else { BIG }).collect();
    let idx = |x: usize, y: usize| y * w + x;
    for y in 0..h {
        for x in 0..w {
            let mut v = d[idx(x, y)];
            if x > 0 {
                v = v.min(d[idx(x - 1, y)] + 3.0);
            }
            if y > 0 {
                v = v.min(d[idx(x, y - 1)] + 3.0);
                if x > 0 {
                    v = v.min(d[idx(x - 1, y - 1)] + 4.0);
                }
                if x + 1 < w {
                    v = v.min(d[idx(x + 1, y - 1)] + 4.0);
                }
            }
            d[idx(x, y)] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let mut v = d[idx(x, y)];
            if x + 1 < w {
                v = v.min(d[idx(x + 1, y)] + 3.0);
            }
            if y + 1 < h {
                v = v.min(d[idx(x, y + 1)] + 3.0);
                if x + 1 < w {
                    v = v.min(d[idx(x + 1, y + 1)] + 4.0);
                }
                if x > 0 {
                    v = v.min(d[idx(x - 1, y + 1)] + 4.0);
                }
            }
            d[idx(x, y)] = v;
        }
    }
    d.iter().map(|v| v / 3.0).collect()
}

/// A placement of the field: the four corners in the straightened image (normalized coordinates, may
/// lie outside `[0, 1]`) in the order far-left, far-right, near-right, near-left, and the lens.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub corners: [Vec2; 4],
    pub k1: f64,
}

/// Maps court metres into the image for one placement.
pub struct Mapper {
    h: Homography,
    lens: Lens,
    r_max: f64,
}

impl Mapper {
    pub fn new(p: &Placement, field: (f32, f32), aspect: f64) -> Option<Mapper> {
        let (l, w) = field;
        let court = [Vec2::new(0.0, 0.0), Vec2::new(l, 0.0), Vec2::new(l, w), Vec2::new(0.0, w)];
        let h = Homography::from_points(&court, &p.corners)?;
        let lens = Lens { k1: p.k1, ..Lens::none(aspect) };
        let r_max = if lens.is_identity() { 0.0 } else { lens.max_radius() };
        Some(Mapper { h, lens, r_max })
    }

    pub fn to_image(&self, c: Vec2) -> Option<Vec2> {
        let p = self.h.apply(c)?;
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        if self.lens.is_identity() {
            return Some(p);
        }
        self.lens.distort(p.x as f64, p.y as f64, self.r_max).map(|(u, v)| Vec2::new(u as f32, v as f32))
    }
}

/// What the user says about the picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hints {
    /// Corners outside the picture, order far-left, far-right, near-right, near-left.
    pub outside: [bool; 4],
    pub fisheye: bool,
}

struct Problem<'a> {
    map: &'a LineMap,
    pts: Vec<Vec2>,
    field: (f32, f32),
    aspect: f64,
    hints: Hints,
}

impl Problem<'_> {
    /// Mean capped squared distance of the visible line samples (in units of `cap`), plus penalties for
    /// corners on the wrong side of the picture edge and for a field that is mostly out of view.
    fn cost(&self, p: &Placement, cap: f32) -> f64 {
        let Some(m) = Mapper::new(p, self.field, self.aspect) else { return 1e9 };
        let (l, w) = self.field;
        let corners = [Vec2::new(0.0, 0.0), Vec2::new(l, 0.0), Vec2::new(l, w), Vec2::new(0.0, w)];
        let mut pen = shape_penalty(&p.corners);
        for (i, c) in corners.iter().enumerate() {
            let inside = m.to_image(*c).map(|q| q.x > 0.005 && q.y > 0.005 && q.x < 0.995 && q.y < 0.995).unwrap_or(false);
            if inside == self.hints.outside[i] {
                pen += 1.0;
            }
        }
        let (mut sum, mut n) = (0f64, 0usize);
        for c in &self.pts {
            if let Some(q) = m.to_image(*c) {
                if let Some(d) = self.map.dist_at(q.x, q.y) {
                    let e = (d.min(cap) / cap) as f64;
                    sum += e * e;
                    n += 1;
                }
            }
        }
        if n < self.pts.len() / 5 {
            return 2.0 + pen;
        }
        sum / n as f64 + pen + 0.3 * (1.0 - n as f64 / self.pts.len() as f64)
    }

    /// Share of visible line samples within `px` of a line pixel.
    fn agreement(&self, p: &Placement, px: f32) -> f64 {
        let Some(m) = Mapper::new(p, self.field, self.aspect) else { return 0.0 };
        let (mut hit, mut n) = (0usize, 0usize);
        for c in &self.pts {
            if let Some(d) = m.to_image(*c).and_then(|q| self.map.dist_at(q.x, q.y)) {
                n += 1;
                hit += (d <= px) as usize;
            }
        }
        if n == 0 { 0.0 } else { hit as f64 / n as f64 }
    }
}

/// A field seen from a sideline is a convex quadrilateral with the far side above the near one and
/// some width and depth. Squeezed placements (all lines on top of one line) fit too well otherwise.
fn shape_penalty(c: &[Vec2; 4]) -> f64 {
    let [fl, fr, nr, nl] = *c;
    let mut pen = 0.0;
    let wf = fr.x - fl.x;
    let wn = nr.x - nl.x;
    if wf < 0.08 || wn < 0.08 {
        pen += 1.0;
    }
    let depth = ((nl.y - fl.y) + (nr.y - fr.y)) / 2.0;
    if depth < 0.12 * wf.max(wn).max(0.0) || nl.y - fl.y < 0.03 || nr.y - fr.y < 0.03 {
        pen += 1.0;
    }
    // convex, same turning direction at every corner
    let cross = |a: Vec2, b: Vec2, c: Vec2| (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
    let t = [cross(fl, fr, nr), cross(fr, nr, nl), cross(nr, nl, fl), cross(nl, fl, fr)];
    if !(t.iter().all(|v| *v > 0.0) || t.iter().all(|v| *v < 0.0)) {
        pen += 1.0;
    }
    pen
}

/// Result of an automatic calibration.
#[derive(Clone, Copy, Debug)]
pub struct AutoResult {
    pub placement: Placement,
    /// Share of the visible field-line samples that lie on detected line pixels (within 2 px).
    pub agreement: f64,
}

/// Fit the field (`length`, `width` in metres, its `lines` as polylines) to the line map.
/// `start`: an optional placement to refine (e.g. from points the user already clicked).
pub fn fit(map: &LineMap, lines: &[Vec<Vec2>], field: (f32, f32), aspect: f64, hints: Hints, start: Option<Placement>) -> Option<AutoResult> {
    let sample = |step: f32| -> Vec<Vec2> {
        let mut v = Vec::new();
        for pl in lines {
            for (i, p) in pl.iter().enumerate() {
                if i > 0 {
                    let q = pl[i - 1];
                    let n = ((p.dist(q) / step).ceil() as usize).max(1);
                    v.extend((0..n).map(|k| q.lerp(*p, k as f32 / n as f32)));
                }
            }
            if let Some(last) = pl.last() {
                v.push(*last);
            }
        }
        v
    };
    let coarse = Problem { map, pts: sample(1.0), field, aspect, hints };
    let fine = Problem { map, pts: sample(0.3), field, aspect, hints };

    // coarse search over field shapes seen from a sideline camera
    let k1s: &[f64] = if hints.fisheye { &[0.0, -0.15, -0.3] } else { &[0.0] };
    let lin = |a: f32, b: f32, n: usize| (0..n).map(move |i| a + (b - a) * i as f32 / (n - 1) as f32);
    let grid = |k1: f64| -> Vec<(f64, Placement)> {
        let mut out = Vec::new();
        for yf in lin(0.05, 0.75, 15) {
            for dy in lin(0.08, 1.3, 15) {
                let yn = yf + dy;
                for hf in lin(0.12, 0.75, 10) {
                    for g in lin(1.0, 4.0, 10) {
                        let hn = hf * g;
                        for cx in [0.42f32, 0.5, 0.58] {
                            let p = Placement { corners: [Vec2::new(cx - hf, yf), Vec2::new(cx + hf, yf), Vec2::new(cx + hn, yn), Vec2::new(cx - hn, yn)], k1 };
                            let c = coarse.cost(&p, 12.0);
                            if c < 1.0 {
                                out.push((c, p));
                            }
                        }
                    }
                }
            }
        }
        out
    };
    // one thread per lens strength
    let mut cands: Vec<(f64, Placement)> = std::thread::scope(|sc| {
        let hs: Vec<_> = k1s.iter().map(|&k1| sc.spawn(move || grid(k1))).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    // keep distinct starting points (different far/near lines)
    let mut starts: Vec<Placement> = start.into_iter().collect();
    for (_, p) in &cands {
        if starts.len() >= 8 {
            break;
        }
        let distinct = starts.iter().all(|s| (0..4).map(|i| s.corners[i].dist(p.corners[i])).fold(0.0f32, f32::max) > 0.06);
        if distinct {
            starts.push(*p);
        }
    }
    // refine every start in its own thread: coarse-to-fine distance caps
    let fine_ref = &fine;
    let refined: Vec<(f64, Placement)> = std::thread::scope(|sc| {
        let hs: Vec<_> = starts
            .into_iter()
            .map(|s| {
                sc.spawn(move || {
                    let mut p = s;
                    for (cap, step) in [(16.0f32, 0.03f64), (6.0, 0.01), (3.0, 0.004)] {
                        p = refine(fine_ref, p, cap, step, hints.fisheye);
                    }
                    (fine_ref.cost(&p, 3.0), p)
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let (c, p) = refined.into_iter().min_by(|a, b| a.0.total_cmp(&b.0))?;
    if c >= 1.0 {
        return None; // a corner hint is violated or almost nothing fits
    }
    Some(AutoResult { placement: p, agreement: fine.agreement(&p, 2.0) })
}

fn refine(pr: &Problem, p: Placement, cap: f32, step: f64, fisheye: bool) -> Placement {
    let mut x: Vec<f64> = p.corners.iter().flat_map(|c| [c.x as f64, c.y as f64]).collect();
    if fisheye {
        x.push(p.k1);
    }
    let unpack = |x: &[f64]| Placement {
        corners: [0, 1, 2, 3].map(|i| Vec2::new(x[2 * i] as f32, x[2 * i + 1] as f32)),
        k1: if fisheye { x[8].clamp(-0.6, 0.1) } else { 0.0 },
    };
    let f = |x: &[f64]| pr.cost(&unpack(x), cap);
    let (best, _) = nelder_mead(&f, &x, step, 600);
    unpack(&best)
}

fn nelder_mead(f: &dyn Fn(&[f64]) -> f64, start: &[f64], step: f64, iters: usize) -> (Vec<f64>, f64) {
    let n = start.len();
    let mut s: Vec<(Vec<f64>, f64)> = vec![(start.to_vec(), f(start))];
    for i in 0..n {
        let mut p = start.to_vec();
        p[i] += step;
        let c = f(&p);
        s.push((p, c));
    }
    let cmp = |a: &(Vec<f64>, f64), b: &(Vec<f64>, f64)| a.1.total_cmp(&b.1);
    for _ in 0..iters {
        s.sort_by(cmp);
        if (s[n].1 - s[0].1).abs() < 1e-9 {
            break;
        }
        let cen: Vec<f64> = (0..n).map(|j| s[..n].iter().map(|v| v.0[j]).sum::<f64>() / n as f64).collect();
        let at = |t: f64, w: &[f64]| -> Vec<f64> { (0..n).map(|j| cen[j] + t * (w[j] - cen[j])).collect() };
        let worst = s[n].0.clone();
        let r = at(-1.0, &worst);
        let fr = f(&r);
        if fr < s[0].1 {
            let e = at(-2.0, &worst);
            let fe = f(&e);
            s[n] = if fe < fr { (e, fe) } else { (r, fr) };
        } else if fr < s[n - 1].1 {
            s[n] = (r, fr);
        } else {
            let c = at(0.5, &worst);
            let fc = f(&c);
            if fc < s[n].1 {
                s[n] = (c, fc);
            } else {
                let b = s[0].0.clone();
                for v in s.iter_mut().skip(1) {
                    v.0 = (0..n).map(|j| b[j] + 0.5 * (v.0[j] - b[j])).collect();
                    v.1 = f(&v.0);
                }
            }
        }
    }
    s.sort_by(cmp);
    s.swap_remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sport::SportProfile;

    /// Draws the field lines of `prof` with a placement into a grey frame with some floor texture.
    fn render(prof: &SportProfile, p: &Placement, w: usize, h: usize) -> Vec<u8> {
        let m = Mapper::new(p, (prof.field.length, prof.field.width), w as f64 / h as f64).unwrap();
        let mut rgb = vec![0u8; w * h * 3];
        for (i, px) in rgb.chunks_mut(3).enumerate() {
            let (x, y) = (i % w, i / w);
            let t = ((x * 7 + y * 13) % 17) as u8; // mild texture
            px.copy_from_slice(&[140 + t, 105 + t, 70 + t]);
        }
        for pl in prof.densified_lines(0.02) {
            for c in pl {
                if let Some(q) = m.to_image(c) {
                    let (cx, cy) = ((q.x * w as f32) as isize, (q.y * h as f32) as isize);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (x, y) = (cx + dx, cy + dy);
                            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                                let i = (y as usize * w + x as usize) * 3;
                                rgb[i..i + 3].copy_from_slice(&[240, 240, 240]);
                            }
                        }
                    }
                }
            }
        }
        rgb
    }

    fn worst_corner_error(prof: &SportProfile, a: &Placement, b: &Placement, aspect: f64) -> f32 {
        // compare where landmarks land in the picture, in pixels of a 1280 wide frame
        let (ma, mb) = (Mapper::new(a, (prof.field.length, prof.field.width), aspect).unwrap(), Mapper::new(b, (prof.field.length, prof.field.width), aspect).unwrap());
        prof.landmarks
            .iter()
            .filter_map(|l| {
                let c = Vec2::new(l.x, l.y);
                let (p, q) = (ma.to_image(c)?, mb.to_image(c)?);
                let inside = |v: Vec2| v.x > 0.0 && v.y > 0.0 && v.x < 1.0 && v.y < 1.0;
                (inside(p) || inside(q)).then(|| ((p.x - q.x) * 1280.0).hypot((p.y - q.y) * 720.0))
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn finds_a_basketball_court_with_near_corners_out_of_view() {
        let prof = SportProfile::basketball();
        let (w, h) = (1280, 720);
        // near corners below and beside the picture, as on a panorama from the sideline
        let truth = Placement { corners: [Vec2::new(0.27, 0.36), Vec2::new(0.74, 0.37), Vec2::new(1.12, 1.06), Vec2::new(-0.1, 1.04)], k1: 0.0 };
        let rgb = render(&prof, &truth, w, h);
        let map = LineMap::from_rgb(&rgb, w, h);
        let hints = Hints { outside: [false, false, true, true], fisheye: false };
        let r = fit(&map, &prof.lines, (prof.field.length, prof.field.width), w as f64 / h as f64, hints, None).expect("found");
        let err = worst_corner_error(&prof, &r.placement, &truth, w as f64 / h as f64);
        assert!(err < 6.0, "landmarks off by {err} px, agreement {}, got {:?}", r.agreement, r.placement);
        assert!(r.agreement > 0.8, "agreement {}", r.agreement);
    }

    #[test]
    fn finds_a_fisheye_full_court() {
        let prof = SportProfile::basketball();
        let (w, h) = (1280, 720);
        let truth = Placement { corners: [Vec2::new(0.2, 0.4), Vec2::new(0.8, 0.4), Vec2::new(1.0, 0.86), Vec2::new(0.0, 0.86)], k1: -0.3 };
        let rgb = render(&prof, &truth, w, h);
        let map = LineMap::from_rgb(&rgb, w, h);
        let hints = Hints { outside: [false; 4], fisheye: true };
        let r = fit(&map, &prof.lines, (prof.field.length, prof.field.width), w as f64 / h as f64, hints, None).expect("found");
        let err = worst_corner_error(&prof, &r.placement, &truth, w as f64 / h as f64);
        assert!(err < 6.0, "landmarks off by {err} px, k1 {}, agreement {}", r.placement.k1, r.agreement);
    }

    #[test]
    fn line_map_marks_thin_bright_lines_only() {
        let (w, h) = (200, 100);
        let mut rgb = vec![100u8; w * h * 3];
        for y in 0..h {
            for x in 98..101 {
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[250, 250, 250]);
            }
            // a wide bright area is not a line
            for x in 150..200 {
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[250, 250, 250]);
            }
        }
        let m = LineMap::from_rgb(&rgb, w, h);
        assert!(m.mask[50 * m.w + 99] == 1);
        assert!(m.mask[50 * m.w + 180] == 0);
        assert!(m.dist_at(0.497, 0.5).unwrap() < 1.0);
    }
}
