//! Lens distortion for fisheye footage (e.g. one camera that sees the full court).
//!
//! A fisheye image bends straight court lines, so a plain homography cannot map it onto the court.
//! [`Lens`] removes the bend first (radial *division model*, the usual choice for fisheye lenses
//! because one or two parameters already cover strong distortion), and the homography maps the
//! straightened image onto the court. With `k1 = k2 = 0` the lens is the identity and everything
//! behaves exactly as without it.
//!
//! Coordinates are the normalized image coordinates used everywhere else (`u`, `v` in `[0, 1]`).
//! Radii are measured in half image widths, so the parameters do not depend on the resolution.

use crate::geom::{Homography, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Distortion centre (normalized image coordinates).
    pub cx: f64,
    pub cy: f64,
    pub k1: f64,
    pub k2: f64,
    /// Image width / height.
    pub aspect: f64,
}

impl Lens {
    pub fn none(aspect: f64) -> Self {
        Self { cx: 0.5, cy: 0.5, k1: 0.0, k2: 0.0, aspect: if aspect > 0.0 { aspect } else { 1.0 } }
    }

    pub fn is_identity(&self) -> bool {
        self.k1 == 0.0 && self.k2 == 0.0
    }

    fn factor(&self, r2: f64) -> f64 {
        1.0 + self.k1 * r2 + self.k2 * r2 * r2
    }

    fn to_radial(&self, u: f64, v: f64) -> (f64, f64) {
        ((u - self.cx) * 2.0, (v - self.cy) * 2.0 / self.aspect)
    }

    fn from_radial(&self, x: f64, y: f64) -> (f64, f64) {
        (self.cx + x / 2.0, self.cy + y * self.aspect / 2.0)
    }

    /// Largest distorted radius up to which the model is one-to-one (the straightened radius
    /// `r / factor(r²)` still grows). Beyond it the model folds back and has no meaning.
    pub fn max_radius(&self) -> f64 {
        let g = |r: f64| {
            let f = self.factor(r * r);
            if f <= 1e-6 {
                f64::NAN
            } else {
                r / f
            }
        };
        let (mut r, step) = (0.0, 0.01);
        let mut last = 0.0;
        while r < 4.0 {
            let n = g(r + step);
            if !n.is_finite() || n <= last {
                return r;
            }
            last = n;
            r += step;
        }
        4.0
    }

    /// Image point -> straightened point (same coordinate frame).
    pub fn undistort(&self, u: f64, v: f64) -> Option<(f64, f64)> {
        if self.is_identity() {
            return Some((u, v));
        }
        let (x, y) = self.to_radial(u, v);
        let f = self.factor(x * x + y * y);
        if f <= 1e-6 {
            return None;
        }
        Some(self.from_radial(x / f, y / f))
    }

    /// Straightened point -> image point; `None` if no image point maps there.
    pub fn distort(&self, u: f64, v: f64, r_max: f64) -> Option<(f64, f64)> {
        if self.is_identity() {
            return Some((u, v));
        }
        let (x, y) = self.to_radial(u, v);
        let ru = (x * x + y * y).sqrt();
        if ru < 1e-12 {
            return Some((u, v));
        }
        let g = |r: f64| r / self.factor(r * r);
        if ru > g(r_max) {
            return None;
        }
        // Newton from the straightened radius converges in a few steps for usual lenses
        let mut r = ru.min(r_max);
        for _ in 0..8 {
            let f = self.factor(r * r);
            let df = 2.0 * self.k1 * r + 4.0 * self.k2 * r * r * r;
            let gr = r / f;
            let dg = (f - r * df) / (f * f);
            if dg <= 1e-9 {
                break;
            }
            let next = r - (gr - ru) / dg;
            if !(0.0..=r_max).contains(&next) {
                break;
            }
            r = next;
        }
        if (g(r) - ru).abs() < 1e-10 {
            return Some(self.from_radial(x * r / ru, y * r / ru));
        }
        // otherwise bisection: g grows on [0, r_max]
        let (mut lo, mut hi) = (0.0, r_max);
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            if g(mid) < ru {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let rd = 0.5 * (lo + hi);
        Some(self.from_radial(x * rd / ru, y * rd / ru))
    }

    /// Does the model stay one-to-one over the whole image? (corners are the farthest points)
    fn valid_over_image(&self) -> bool {
        let r_img = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
            .iter()
            .map(|&(u, v)| {
                let (x, y) = self.to_radial(u, v);
                (x * x + y * y).sqrt()
            })
            .fold(0.0, f64::max);
        self.max_radius() >= r_img
    }
}

/// How much of the lens to estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LensLevel {
    /// One parameter, centre in the middle of the image.
    K1,
    /// Two parameters.
    K1K2,
    /// Two parameters and the centre.
    Full,
}

impl LensLevel {
    /// The most the given number of point pairs can support without fitting noise.
    pub fn for_points(n: usize) -> Option<Self> {
        match n {
            0..=5 => None,
            6..=8 => Some(Self::K1),
            9..=11 => Some(Self::K1K2),
            _ => Some(Self::Full),
        }
    }

    pub const MIN_POINTS: usize = 6;
}

/// A fitted lens with its homography and the RMS error in court metres.
#[derive(Clone, Copy, Debug)]
pub struct LensFit {
    pub lens: Lens,
    pub pano_to_court: Homography,
    pub rms_m: f64,
}

/// Homography and RMS for a given lens.
pub fn fit_homography(lens: &Lens, pano: &[Vec2], court: &[Vec2]) -> Option<LensFit> {
    let mut und = Vec::with_capacity(pano.len());
    for p in pano {
        let (u, v) = lens.undistort(p.x as f64, p.y as f64)?;
        und.push(Vec2::new(u as f32, v as f32));
    }
    let h = Homography::from_points(&und, court)?;
    let mut sum = 0.0f64;
    for (p, c) in und.iter().zip(court) {
        let m = h.apply(*p)?;
        let d = m.dist(*c) as f64;
        if !d.is_finite() {
            return None;
        }
        sum += d * d;
    }
    Some(LensFit { lens: *lens, pano_to_court: h, rms_m: (sum / pano.len().max(1) as f64).sqrt() })
}

/// Estimate the lens (up to `level`) together with the homography from point pairs
/// (`pano` normalized image coordinates, `court` metres). Starts from several strengths so that a
/// strong fisheye does not end in the local minimum next to "no distortion".
pub fn fit(pano: &[Vec2], court: &[Vec2], aspect: f64, level: LensLevel) -> Option<LensFit> {
    if pano.len() != court.len() || pano.len() < LensLevel::MIN_POINTS {
        return None;
    }
    let base = Lens::none(aspect);
    let make = |p: &[f64]| -> Lens {
        let mut l = base;
        l.k1 = p[0];
        if level >= LensLevel::K1K2 {
            l.k2 = p[1];
        }
        if level == LensLevel::Full {
            l.cx = 0.5 + p[2];
            l.cy = 0.5 + p[3];
        }
        l
    };
    let cost = |p: &[f64]| -> f64 {
        let l = make(p);
        if level == LensLevel::Full && (p[2].abs() > 0.15 || p[3].abs() > 0.15) {
            return f64::INFINITY;
        }
        if !l.valid_over_image() {
            return f64::INFINITY;
        }
        fit_homography(&l, pano, court).map(|f| f.rms_m).unwrap_or(f64::INFINITY)
    };
    let dim = match level {
        LensLevel::K1 => 1,
        LensLevel::K1K2 => 2,
        LensLevel::Full => 4,
    };
    let mut best: Option<(Vec<f64>, f64)> = None;
    for k0 in [0.0, -0.15, -0.3, -0.45] {
        let mut start = vec![0.0; dim];
        start[0] = k0;
        let (p, c) = nelder_mead(&cost, &start, 0.08, 400);
        if c.is_finite() && best.as_ref().map(|b| c < b.1).unwrap_or(true) {
            best = Some((p, c));
        }
    }
    let (p, _) = best?;
    fit_homography(&make(&p), pano, court)
}

/// Minimal Nelder–Mead; returns the best point and its cost.
fn nelder_mead(f: &dyn Fn(&[f64]) -> f64, start: &[f64], step: f64, iters: usize) -> (Vec<f64>, f64) {
    let n = start.len();
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n + 1);
    simplex.push((start.to_vec(), f(start)));
    for i in 0..n {
        let mut p = start.to_vec();
        p[i] += if i == 0 { step } else { step * 0.25 };
        let c = f(&p);
        simplex.push((p, c));
    }
    let cmp = |a: &(Vec<f64>, f64), b: &(Vec<f64>, f64)| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal);
    for _ in 0..iters {
        simplex.sort_by(cmp);
        if (simplex[n].1 - simplex[0].1).abs() < 1e-7 && simplex[0].1.is_finite() {
            break;
        }
        let centroid: Vec<f64> = (0..n).map(|j| simplex[..n].iter().map(|s| s.0[j]).sum::<f64>() / n as f64).collect();
        let at = |t: f64| -> Vec<f64> { (0..n).map(|j| centroid[j] + t * (simplex[n].0[j] - centroid[j])).collect() };
        let r = at(-1.0);
        let fr = f(&r);
        if fr < simplex[0].1 {
            let e = at(-2.0);
            let fe = f(&e);
            simplex[n] = if fe < fr { (e, fe) } else { (r, fr) };
        } else if fr < simplex[n - 1].1 {
            simplex[n] = (r, fr);
        } else {
            let c = at(0.5);
            let fc = f(&c);
            if fc < simplex[n].1 {
                simplex[n] = (c, fc);
            } else {
                let b = simplex[0].0.clone();
                for s in simplex.iter_mut().skip(1) {
                    s.0 = (0..n).map(|j| b[j] + 0.5 * (s.0[j] - b[j])).collect();
                    s.1 = f(&s.0);
                }
            }
        }
    }
    simplex.sort_by(cmp);
    let (p, c) = simplex.swap_remove(0);
    (p, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(k1: f64, k2: f64) -> Lens {
        Lens { cx: 0.5, cy: 0.5, k1, k2, aspect: 16.0 / 9.0 }
    }

    #[test]
    fn distort_inverts_undistort() {
        let l = lens(-0.3, 0.02);
        let rm = l.max_radius();
        for &(u, v) in &[(0.1, 0.2), (0.5, 0.5), (0.93, 0.85), (0.02, 0.98), (0.6, 0.4)] {
            let (a, b) = l.undistort(u, v).unwrap();
            let (x, y) = l.distort(a, b, rm).unwrap();
            assert!((x - u).abs() < 1e-6 && (y - v).abs() < 1e-6, "{u},{v} -> {x},{y}");
        }
        assert_eq!(Lens::none(1.7).undistort(0.3, 0.4), Some((0.3, 0.4)));
    }

    /// Synthetic fisheye: a court seen through a homography, then bent by a known lens.
    fn scene(l: &Lens) -> (Vec<Vec2>, Vec<Vec2>, Homography) {
        let court_to_flat = Homography::from_points(
            &[Vec2::new(0.0, 0.0), Vec2::new(28.0, 0.0), Vec2::new(28.0, 15.0), Vec2::new(0.0, 15.0)],
            &[Vec2::new(0.22, 0.38), Vec2::new(0.78, 0.38), Vec2::new(1.02, 0.86), Vec2::new(-0.02, 0.86)],
        )
        .unwrap();
        let rm = l.max_radius();
        let mut court = Vec::new();
        for &x in &[0.0f32, 5.8, 14.0, 22.2, 28.0] {
            for &y in &[0.0f32, 5.05, 9.95, 15.0] {
                court.push(Vec2::new(x, y));
            }
        }
        let pano = court
            .iter()
            .map(|c| {
                let f = court_to_flat.apply(*c).unwrap();
                let (u, v) = l.distort(f.x as f64, f.y as f64, rm).unwrap();
                Vec2::new(u as f32, v as f32)
            })
            .collect();
        (pano, court, court_to_flat)
    }

    #[test]
    fn recovers_a_strong_fisheye() {
        let truth = lens(-0.32, 0.0);
        let (pano, court, _) = scene(&truth);
        // a plain homography cannot explain the bend
        let plain = fit_homography(&Lens::none(16.0 / 9.0), &pano, &court).unwrap();
        assert!(plain.rms_m > 0.5, "plain rms {}", plain.rms_m);
        let f = fit(&pano, &court, 16.0 / 9.0, LensLevel::K1).unwrap();
        assert!(f.rms_m < 0.02, "fisheye rms {}", f.rms_m);
        assert!((f.lens.k1 - truth.k1).abs() < 0.02, "k1 {}", f.lens.k1);
        let f2 = fit(&pano, &court, 16.0 / 9.0, LensLevel::Full).unwrap();
        assert!(f2.rms_m < 0.02, "full rms {}", f2.rms_m);
    }

    #[test]
    fn no_distortion_stays_near_zero() {
        let (pano, court, _) = scene(&lens(0.0, 0.0));
        let f = fit(&pano, &court, 16.0 / 9.0, LensLevel::K1K2).unwrap();
        assert!(f.rms_m < 0.01 && f.lens.k1.abs() < 0.02, "{:?}", f.lens);
    }

    #[test]
    fn level_follows_point_count() {
        assert_eq!(LensLevel::for_points(5), None);
        assert_eq!(LensLevel::for_points(6), Some(LensLevel::K1));
        assert_eq!(LensLevel::for_points(10), Some(LensLevel::K1K2));
        assert_eq!(LensLevel::for_points(20), Some(LensLevel::Full));
    }
}
