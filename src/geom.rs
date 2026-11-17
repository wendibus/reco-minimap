//! Basic geometry: 2D vectors, FIBA court dimensions, homography.

use std::ops::{Add, Mul, Sub};

/// FIBA court length (x axis) in metres.
pub const COURT_LENGTH: f32 = 28.0;
/// FIBA court width (y axis) in metres.
pub const COURT_WIDTH: f32 = 15.0;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn dist(self, o: Vec2) -> f32 {
        (self - o).len()
    }
    pub fn lerp(self, o: Vec2, t: f32) -> Vec2 {
        self + (o - self) * t
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

/// Playing field size in metres (x: length, left -> right; y: width, far -> near sideline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub length: f32,
    pub width: f32,
}

impl Field {
    pub const fn new(length: f32, width: f32) -> Self {
        Self { length, width }
    }
    pub const fn basketball() -> Self {
        Self { length: COURT_LENGTH, width: COURT_WIDTH }
    }
    /// True if `p` lies within the field grown by `margin` metres.
    pub fn contains(&self, p: Vec2, margin: f32) -> bool {
        p.x >= -margin && p.x <= self.length + margin && p.y >= -margin && p.y <= self.width + margin
    }
    pub fn clamp(&self, p: Vec2, margin: f32) -> Vec2 {
        Vec2::new(p.x.clamp(-margin, self.length + margin), p.y.clamp(-margin, self.width + margin))
    }
    /// Distance to the nearest field edge in metres (negative outside).
    pub fn edge_dist(&self, p: Vec2) -> f32 {
        p.x.min(self.length - p.x).min(p.y).min(self.width - p.y)
    }
}

impl Default for Field {
    fn default() -> Self {
        Field::basketball()
    }
}

/// True if `p` lies within the (basketball) court rectangle grown by `margin` metres.
pub fn in_court(p: Vec2, margin: f32) -> bool {
    p.x >= -margin && p.x <= COURT_LENGTH + margin && p.y >= -margin && p.y <= COURT_WIDTH + margin
}

/// Clamp `p` into the court rectangle grown by `margin` metres.
pub fn clamp_to_court(p: Vec2, margin: f32) -> Vec2 {
    Vec2::new(
        p.x.clamp(-margin, COURT_LENGTH + margin),
        p.y.clamp(-margin, COURT_WIDTH + margin),
    )
}

/// Maps a point in the panorama (normalized `[0,1]` coords) to court metres.
///
/// Reco panoramas are *not* a pinhole image, so the real mapping is
/// "undo the panorama projection, then homography". Implement this trait
/// on top of reco-core's projection; [`HomographyProjector`] is the plain
/// planar variant and the calibration fallback.
pub trait Projector {
    fn to_court(&self, u: f32, v: f32) -> Option<Vec2>;
}

/// Plain 3x3 homography (row-major), `h[8] == 1`.
#[derive(Clone, Copy, Debug)]
pub struct Homography {
    h: [f64; 9],
}

impl Homography {
    pub fn from_matrix(h: [f64; 9]) -> Self {
        Self { h }
    }

    /// Least-squares DLT from `n >= 4` point pairs (`src` -> `dst`).
    pub fn from_points(src: &[Vec2], dst: &[Vec2]) -> Option<Self> {
        if src.len() != dst.len() || src.len() < 4 {
            return None;
        }
        let mut ata = [[0f64; 8]; 8];
        let mut atb = [0f64; 8];
        for (s, d) in src.iter().zip(dst) {
            let (x, y, u, v) = (s.x as f64, s.y as f64, d.x as f64, d.y as f64);
            let rows = [
                ([x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u], u),
                ([0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v], v),
            ];
            for (r, b) in rows.iter() {
                for i in 0..8 {
                    for j in 0..8 {
                        ata[i][j] += r[i] * r[j];
                    }
                    atb[i] += r[i] * b;
                }
            }
        }
        let s = solve8(ata, atb)?;
        Some(Self {
            h: [s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], 1.0],
        })
    }

    /// Least-squares DLT from homogeneous source points (e.g. viewing rays of a panorama, which can
    /// point sideways where a flat image coordinate would not exist) to plane points.
    ///
    /// Normalized and solved for all nine entries (smallest eigenvector of `AᵀA`): for floor rays of
    /// a panorama the entry that [`from_points`](Self::from_points) fixes to 1 is close to 0.
    pub fn from_rays(src: &[[f64; 3]], dst: &[Vec2]) -> Option<Self> {
        if src.len() != dst.len() || src.len() < 4 {
            return None;
        }
        let n = dst.len() as f64;
        let (mx, my) = (dst.iter().map(|d| d.x as f64).sum::<f64>() / n, dst.iter().map(|d| d.y as f64).sum::<f64>() / n);
        let md = dst.iter().map(|d| ((d.x as f64 - mx).powi(2) + (d.y as f64 - my).powi(2)).sqrt()).sum::<f64>() / n;
        if md < 1e-12 {
            return None;
        }
        let sc = std::f64::consts::SQRT_2 / md;
        let mut m = [[0f64; 9]; 9];
        for (r, d) in src.iter().zip(dst) {
            let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
            if len < 1e-12 {
                return None;
            }
            let [a, b, c] = [r[0] / len, r[1] / len, r[2] / len];
            let (x, y) = ((d.x as f64 - mx) * sc, (d.y as f64 - my) * sc);
            let rows = [[a, b, c, 0.0, 0.0, 0.0, -x * a, -x * b, -x * c], [0.0, 0.0, 0.0, a, b, c, -y * a, -y * b, -y * c]];
            for row in rows {
                for i in 0..9 {
                    for j in 0..9 {
                        m[i][j] += row[i] * row[j];
                    }
                }
            }
        }
        let hn = smallest_eigenvector(m);
        // undo the normalization of the plane points: H = T⁻¹ · Hn
        let ti = [[1.0 / sc, 0.0, mx], [0.0, 1.0 / sc, my], [0.0, 0.0, 1.0]];
        let mut h = [0f64; 9];
        for i in 0..3 {
            for j in 0..3 {
                h[i * 3 + j] = (0..3).map(|k| ti[i][k] * hn[k * 3 + j]).sum();
            }
        }
        let norm = h.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !norm.is_finite() || norm < 1e-15 {
            return None;
        }
        for v in h.iter_mut() {
            *v /= norm;
        }
        Some(Self { h })
    }

    pub fn apply(&self, p: Vec2) -> Option<Vec2> {
        let (x, y) = (p.x as f64, p.y as f64);
        let h = &self.h;
        let w = h[6] * x + h[7] * y + h[8];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = (h[0] * x + h[1] * y + h[2]) / w;
        let v = (h[3] * x + h[4] * y + h[5]) / w;
        if !u.is_finite() || !v.is_finite() {
            return None;
        }
        Some(Vec2::new(u as f32, v as f32))
    }

    /// The homogeneous product `H · r`.
    pub fn apply3(&self, r: [f64; 3]) -> [f64; 3] {
        let h = &self.h;
        [h[0] * r[0] + h[1] * r[1] + h[2] * r[2], h[3] * r[0] + h[4] * r[1] + h[5] * r[2], h[6] * r[0] + h[7] * r[1] + h[8] * r[2]]
    }

    pub fn inverse(&self) -> Option<Self> {
        let m = &self.h;
        let (a, b, c, d, e, f, g, h, i) = (m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]);
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if det.abs() < 1e-18 {
            return None;
        }
        let inv = [
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ];
        let s = 1.0 / det;
        let mut out = [0f64; 9];
        for k in 0..9 {
            out[k] = inv[k] * s;
        }
        Some(Self { h: out })
    }
}

/// Eigenvector of the smallest eigenvalue of a symmetric 9×9 matrix (cyclic Jacobi rotations).
fn smallest_eigenvector(mut a: [[f64; 9]; 9]) -> [f64; 9] {
    let mut v = [[0f64; 9]; 9];
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..60 {
        let off: f64 = (0..9).flat_map(|i| (0..9).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-22 {
            break;
        }
        for p in 0..9 {
            for q in p + 1..9 {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..9 {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..9 {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vp, vq) = (row[p], row[q]);
                    row[p] = c * vp - s * vq;
                    row[q] = s * vp + c * vq;
                }
            }
        }
    }
    let k = (0..9).min_by(|&i, &j| a[i][i].total_cmp(&a[j][j])).unwrap_or(8);
    let mut out = [0f64; 9];
    for (i, o) in out.iter_mut().enumerate() {
        *o = v[i][k];
    }
    out
}

fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let mut piv = col;
        for r in col + 1..8 {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..8 {
            let f = a[r][col] / a[col][col];
            for c in col..8 {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0f64; 8];
    for r in (0..8).rev() {
        let mut s = b[r];
        for c in r + 1..8 {
            s -= a[r][c] * x[c];
        }
        x[r] = s / a[r][r];
    }
    Some(x)
}

/// Calibration: panorama (normalized) -> court metres. Without a lens a plain homography; with a lens
/// (fisheye or cylindrical panorama, see [`crate::lens`]) every pixel becomes a viewing ray first and
/// the homography maps rays onto the court.
#[derive(Clone, Copy, Debug)]
pub struct HomographyProjector {
    pub pano_to_court: Homography,
    pub lens: Option<crate::lens::Lens>,
    lens_r_max: f64,
    /// Sign of the homogeneous scale for rays that hit the floor (rays are only defined up to scale;
    /// the other sign is the opposite direction).
    z_sign: f64,
}

impl HomographyProjector {
    /// `pano_pts`: normalized panorama coords of known court points,
    /// `court_pts`: the same points in court metres.
    pub fn calibrate(pano_pts: &[Vec2], court_pts: &[Vec2]) -> Option<Self> {
        Some(Self::new(Homography::from_points(pano_pts, court_pts)?, None))
    }

    /// Homography from viewing rays; `lens` `None` (or the identity) = plain image.
    pub fn new(pano_to_court: Homography, lens: Option<crate::lens::Lens>) -> Self {
        let lens = lens.filter(|l| !l.is_identity());
        // the bottom middle of the picture shows floor in every set-up this tool is used with
        let probe = lens.and_then(|l| l.ray(0.5, 0.95)).unwrap_or([0.5, 0.95, 1.0]);
        let z = pano_to_court.apply3(probe)[2];
        Self { pano_to_court, lens, lens_r_max: lens.map(|l| l.max_radius()).unwrap_or(0.0), z_sign: if z < 0.0 { -1.0 } else { 1.0 } }
    }

    /// Court metres -> normalized panorama coordinates (the inverse of [`Projector::to_court`]).
    pub fn to_pano_fn(&self) -> Option<impl Fn(Vec2) -> Option<Vec2>> {
        let inv = self.pano_to_court.inverse()?;
        let (h, lens, rm, zs) = (self.pano_to_court, self.lens, self.lens_r_max, self.z_sign);
        Some(move |c: Vec2| match lens {
            None => inv.apply(c),
            Some(l) => {
                let mut r = inv.apply3([c.x as f64, c.y as f64, 1.0]);
                if h.apply3(r)[2] * zs < 0.0 {
                    r = [-r[0], -r[1], -r[2]];
                }
                l.pixel(r, rm).map(|(u, v)| Vec2::new(u as f32, v as f32))
            }
        })
    }
}

impl Projector for HomographyProjector {
    fn to_court(&self, u: f32, v: f32) -> Option<Vec2> {
        match &self.lens {
            None => self.pano_to_court.apply(Vec2::new(u, v)),
            Some(l) => {
                let q = self.pano_to_court.apply3(l.ray(u as f64, v as f64)?);
                // rays above the horizon never meet the floor
                if q[2] * self.z_sign <= 1e-12 {
                    return None;
                }
                let p = Vec2::new((q[0] / q[2]) as f32, (q[1] / q[2]) as f32);
                (p.x.is_finite() && p.y.is_finite()).then_some(p)
            }
        }
    }
}
