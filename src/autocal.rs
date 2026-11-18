//! Automatic field calibration: find the painted field lines in a frame and fit the field model to them.
//!
//! 1. **Floor:** the floor colour is taken from the bottom middle of the picture; per column the floor
//!    reaches up to where floor-coloured pixels stop (gaps for players allowed). Ceiling, windows and
//!    walls above it are ignored.
//! 2. **Line pixels:** painted lines are thin and differ from the floor on both sides. With a picked
//!    line colour, a pixel counts when it is close to that colour and clearly closer than its
//!    neighbours a few pixels away (in one of four directions) – so in a multi-sport hall only the
//!    lines of one sport count, dark or bright. Without a colour any thin bright or dark line counts.
//! 3. **Distance map:** for every pixel the distance to the nearest line pixel (chamfer transform).
//! 4. **Fit:** the field is placed by its four corners in the picture plus the lens (fisheye strength
//!    or panorama field of view). A placement is good when the field lines, drawn with it, run along
//!    line pixels. A coarse search over field shapes seen from a sideline camera gives starting points,
//!    Nelder–Mead refines the best of them.
//!
//! Which corners lie **outside the picture** is the hint the user gives: it rules out the many
//! placements where a smaller or shifted field fits some of the lines.

use crate::geom::{Homography, Vec2};
use crate::lens::{Lens, LensModel};

/// Working image width at most; distance caps are given for a 960 px wide working image and scaled.
const WORK_W: usize = 1920;

/// A correction to the detected lines: points in picture coordinates (0..1), radius as a share of
/// the picture width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Erase {
    Piece(Vec2),
    Brush(Vec2, f32),
}

pub struct LineMap {
    pub w: usize,
    pub h: usize,
    /// 1 = line pixel.
    pub mask: Vec<u8>,
    /// First floor row per column (working pixels).
    pub floor_top: Vec<usize>,
    /// Distance to the nearest line pixel, in working pixels.
    dist: Vec<f32>,
}

fn lum(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

impl LineMap {
    /// From an RGB24 frame; `color`: the colour of the field's lines, if picked.
    pub fn from_rgb(rgb: &[u8], w: usize, h: usize, color: Option<[u8; 3]>) -> LineMap {
        let f = (w as f32 / WORK_W as f32).max(1.0);
        let (ww, wh) = (((w as f32 / f).round() as usize).max(8), ((h as f32 / f).round() as usize).max(8));
        // box-filtered downscale
        let mut img = vec![[0f32; 3]; ww * wh];
        for y in 0..wh {
            let (y0, y1) = (((y as f32 * f) as usize).min(h - 1), (((y + 1) as f32 * f) as usize).clamp(1, h));
            for x in 0..ww {
                let (x0, x1) = (((x as f32 * f) as usize).min(w - 1), (((x + 1) as f32 * f) as usize).clamp(1, w));
                let (mut s, mut n) = ([0f32; 3], 0f32);
                for yy in y0..y1.max(y0 + 1) {
                    for xx in x0..x1.max(x0 + 1) {
                        let i = (yy * w + xx) * 3;
                        for k in 0..3 {
                            s[k] += rgb[i + k] as f32;
                        }
                        n += 1.0;
                    }
                }
                img[y * ww + x] = [s[0] / n, s[1] / n, s[2] / n];
            }
        }
        let floor_c = floor_color(&img, ww, wh);
        let (bg, gw, gh) = background(&img, ww, wh);
        let floor_top = floor_region(&img, &bg, gw, gh, ww, wh, floor_c);
        let resp: Vec<f32> = match color {
            Some(c) => {
                let line = [c[0] as f32, c[1] as f32, c[2] as f32];
                // against the local floor (reflections make it lighter or yellower in places)
                let closeness: Vec<f32> = (0..ww * wh)
                    .map(|i| {
                        let (cx, cy) = (((i % ww) / CELL).min(gw - 1), ((i / ww) / CELL).min(gh - 1));
                        closeness(img[i], bg[cy * gw + cx], line)
                    })
                    .collect();
                ridges(&closeness, ww, wh, 1.0)
            }
            None => {
                let l: Vec<f32> = img.iter().map(|p| lum(*p) / 255.0).collect();
                let bright = ridges(&l, ww, wh, 1.0);
                let dark = ridges(&l, ww, wh, -1.0);
                bright.iter().zip(&dark).map(|(a, b)| a.max(*b)).collect()
            }
        };
        let thr = {
            let mut on_floor: Vec<f32> = (0..ww * wh).filter(|i| i / ww >= floor_top[i % ww]).map(|i| resp[i]).collect();
            on_floor.sort_by(|a, b| a.total_cmp(b));
            let p995 = on_floor.get(on_floor.len() * 995 / 1000).copied().unwrap_or(0.0);
            (0.3 * p995).max(if color.is_some() { 0.08 } else { 0.05 })
        };
        // sharpened video draws a dark seam beside every bright line (and a bright one beside dark
        // lines): with a picked colour, drop hits right next to a line of another colour
        let near_other = match color {
            Some(c) => {
                let line = [c[0] as f32, c[1] as f32, c[2] as f32];
                let other: Vec<bool> = (0..ww * wh)
                    .map(|i| {
                        let (cx, cy) = (((i % ww) / CELL).min(gw - 1), ((i / ww) / CELL).min(gh - 1));
                        is_other_line(img[i], bg[cy * gw + cx], line)
                    })
                    .collect();
                dilate(&other, ww, wh, ((ww as f32 / 640.0).round() as usize).max(2))
            }
            None => vec![false; ww * wh],
        };
        let mask: Vec<u8> = (0..ww * wh).map(|i| (resp[i] > thr && !near_other[i] && i / ww >= floor_top[i % ww]) as u8).collect();
        let dist = chamfer(&mask, ww, wh);
        LineMap { w: ww, h: wh, mask, floor_top, dist }
    }

    /// The map without what was struck out by hand: `Erase::Piece` removes the connected line piece
    /// at a point, `Erase::Brush` every line pixel within a circle.
    pub fn without(&self, erase: &[Erase]) -> LineMap {
        let (w, h) = (self.w, self.h);
        let mut mask = self.mask.clone();
        for e in erase {
            match *e {
                Erase::Brush(c, r) => {
                    let (cx, cy, r) = (c.x * w as f32, c.y * h as f32, (r * w as f32).max(1.0));
                    let (y0, y1) = (((cy - r).floor().max(0.0)) as usize, ((cy + r).ceil().max(0.0) as usize).min(h));
                    let (x0, x1) = (((cx - r).floor().max(0.0)) as usize, ((cx + r).ceil().max(0.0) as usize).min(w));
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) <= r {
                                mask[y * w + x] = 0;
                            }
                        }
                    }
                }
                Erase::Piece(c) => {
                    // nearest line pixel within a few pixels of the click
                    let r = (w / 240).max(4) as isize;
                    let (cx, cy) = ((c.x * w as f32) as isize, (c.y * h as f32) as isize);
                    let mut best: Option<(isize, usize)> = None;
                    for y in (cy - r).max(0)..(cy + r + 1).min(h as isize) {
                        for x in (cx - r).max(0)..(cx + r + 1).min(w as isize) {
                            let i = y as usize * w + x as usize;
                            let d = (x - cx).pow(2) + (y - cy).pow(2);
                            if mask[i] > 0 && d <= r * r && best.is_none_or(|(bd, _)| d < bd) {
                                best = Some((d, i));
                            }
                        }
                    }
                    let Some((_, start)) = best else { continue };
                    let mut stack = vec![start];
                    mask[start] = 0;
                    while let Some(i) = stack.pop() {
                        let (x, y) = ((i % w) as isize, (i / w) as isize);
                        for dy in -1..=1isize {
                            for dx in -1..=1isize {
                                let (nx, ny) = (x + dx, y + dy);
                                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                    let j = ny as usize * w + nx as usize;
                                    if mask[j] > 0 {
                                        mask[j] = 0;
                                        stack.push(j);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let dist = chamfer(&mask, w, h);
        LineMap { w, h, mask, floor_top: self.floor_top.clone(), dist }
    }

    /// Distance to the nearest line pixel at a picture point, bilinear between pixel centres so the
    /// cost changes smoothly when the field moves by less than a pixel.
    fn dist_at(&self, u: f32, v: f32) -> Option<f32> {
        let (x, y) = (u * self.w as f32 - 0.5, v * self.h as f32 - 0.5);
        if x < -0.5 || y < -0.5 || x > self.w as f32 - 0.5 || y > self.h as f32 - 0.5 {
            return None;
        }
        let (x, y) = (x.clamp(0.0, (self.w - 1) as f32), y.clamp(0.0, (self.h - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let d = |xx: usize, yy: usize| self.dist[yy * self.w + xx];
        let top = d(x0, y0) * (1.0 - fx) + d(x1, y0) * fx;
        let bottom = d(x0, y1) * (1.0 - fx) + d(x1, y1) * fx;
        Some(top * (1.0 - fy) + bottom * fy)
    }

    /// Line pixels magenta, the floor black, everything ignored above it dark grey (drawn with
    /// "screen" over the frame).
    pub fn mask_rgb(&self) -> Vec<u8> {
        (0..self.w * self.h)
            .flat_map(|i| {
                if self.mask[i] > 0 {
                    [255u8, 0, 255]
                } else if i / self.w < self.floor_top[i % self.w] {
                    [45, 45, 45]
                } else {
                    [0, 0, 0]
                }
            })
            .collect()
    }

    /// Working pixels per 960 px of width (distance caps scale with it).
    fn px_scale(&self) -> f32 {
        self.w as f32 / 960.0
    }
}

/// How far a pixel is pulled from the floor towards the line colour, in units of the full step: 1 at
/// the line colour, 0 at the floor. Thin or distant lines blend with the floor, so the pull is weaker
/// there; what counts is its direction. Colours off the floor–line direction (other sports' lines,
/// players) are faded out by the angle between the two.
fn closeness(p: [f32; 3], floor: [f32; 3], line: [f32; 3]) -> f32 {
    let d = [line[0] - floor[0], line[1] - floor[1], line[2] - floor[2]];
    let dn = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if dn < 10.0 {
        return 0.0; // line colour equals the floor: nothing to find
    }
    let q = [p[0] - floor[0], p[1] - floor[1], p[2] - floor[2]];
    let qn = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt().max(1e-3);
    let proj = (q[0] * d[0] + q[1] * d[1] + q[2] * d[2]) / dn;
    let gate = ((proj / qn - 0.85) / 0.1).clamp(0.0, 1.0);
    (proj / dn).clamp(0.0, 1.0) * gate
}

/// A pixel clearly off the floor in a direction other than the picked line colour's: another sport's
/// line (or a player).
fn is_other_line(p: [f32; 3], floor: [f32; 3], line: [f32; 3]) -> bool {
    let d = [line[0] - floor[0], line[1] - floor[1], line[2] - floor[2]];
    let dn = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
    let q = [p[0] - floor[0], p[1] - floor[1], p[2] - floor[2]];
    let qn = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    qn > 35.0 && (q[0] * d[0] + q[1] * d[1] + q[2] * d[2]) / (qn * dn) < 0.5
}

/// Square dilation of a boolean map by `r` pixels (separable).
fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut a = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            a[y * w + x] = (x.saturating_sub(r)..(x + r + 1).min(w)).any(|xx| m[y * w + xx]);
        }
    }
    let mut b = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            b[y * w + x] = (y.saturating_sub(r)..(y + r + 1).min(h)).any(|yy| a[yy * w + x]);
        }
    }
    b
}

/// Median colour of the bottom middle of the picture: the floor in front of the camera.
fn floor_color(img: &[[f32; 3]], w: usize, h: usize) -> [f32; 3] {
    let mut ch: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for y in (h * 3 / 4)..(h * 97 / 100).max(h * 3 / 4 + 1) {
        for x in (w * 3 / 10)..(w * 7 / 10) {
            for k in 0..3 {
                ch[k].push(img[y * w + x][k]);
            }
        }
    }
    let med = |v: &mut Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    [med(&mut ch[0]), med(&mut ch[1]), med(&mut ch[2])]
}

/// Cell size of the coarse background grid (working pixels): big enough that a median per cell
/// drops lines and most of a player, small enough to follow the floor's edge.
const CELL: usize = 8;

/// Median colour per CELL x CELL block: the local floor without its lines.
fn background(img: &[[f32; 3]], w: usize, h: usize) -> (Vec<[f32; 3]>, usize, usize) {
    let (gw, gh) = ((w / CELL).max(1), (h / CELL).max(1));
    let mut g = vec![[0f32; 3]; gw * gh];
    let mut ch: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for cy in 0..gh {
        for cx in 0..gw {
            for v in ch.iter_mut() {
                v.clear();
            }
            for y in cy * CELL..((cy + 1) * CELL).min(h) {
                for x in cx * CELL..((cx + 1) * CELL).min(w) {
                    for k in 0..3 {
                        ch[k].push(img[y * w + x][k]);
                    }
                }
            }
            for k in 0..3 {
                ch[k].sort_by(|a, b| a.total_cmp(b));
                g[cy * gw + cx][k] = ch[k].get(ch[k].len() / 2).copied().unwrap_or(0.0);
            }
        }
    }
    (g, gw, gh)
}

fn cdist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Per column the first row of the floor. The floor is grown from the bottom over the background grid
/// as long as neighbouring cells change only a little (reflections change slowly, the step to a wall,
/// mat or window is sharp); upwards it has to be smoother than sideways. Then smoothed across columns.
fn floor_region(img: &[[f32; 3]], bg: &[[f32; 3]], gw: usize, gh: usize, w: usize, h: usize, floor: [f32; 3]) -> Vec<usize> {
    const UP: f32 = 16.0;
    const SIDE: f32 = 28.0;
    let mut seen = vec![false; gw * gh];
    let mut queue = std::collections::VecDeque::new();
    for cx in 0..gw {
        for cy in gh.saturating_sub(2)..gh {
            let i = cy * gw + cx;
            if cdist(bg[i], floor) < 90.0 {
                seen[i] = true;
                queue.push_back(i);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let (cx, cy) = (i % gw, i / gw);
        let mut go = |j: usize, t: f32| {
            if !seen[j] && cdist(bg[j], bg[i]) < t {
                seen[j] = true;
                queue.push_back(j);
            }
        };
        if cy > 0 {
            go(i - gw, UP);
        }
        if cy + 1 < gh {
            go(i + gw, SIDE);
        }
        if cx > 0 {
            go(i - 1, SIDE);
        }
        if cx + 1 < gw {
            go(i + 1, SIDE);
        }
    }
    let col_top: Vec<usize> = (0..gw).map(|cx| (0..gh).find(|&cy| seen[cy * gw + cx]).map_or(h, |cy| cy * CELL)).collect();
    // to the pixel: the first row from the top of the topmost floor cell that looks like the floor
    // cell below it (the cell may still hold some wall)
    let top: Vec<usize> = (0..w)
        .map(|x| {
            let cx = (x / CELL).min(gw - 1);
            let t = col_top[cx];
            if t >= h {
                return h;
            }
            let below = bg[((t / CELL + 1).min(gh - 1)) * gw + cx];
            (t..(t + CELL).min(h)).find(|&y| cdist(img[y * w + x], below) < SIDE).unwrap_or(t)
        })
        .collect();
    // median across neighbouring columns (players standing at the edge of the floor)
    let r = (w / 60).max(3);
    let med: Vec<usize> = (0..w)
        .map(|x| {
            let mut v: Vec<usize> = top[x.saturating_sub(r)..(x + r + 1).min(w)].to_vec();
            v.sort_unstable();
            v[v.len() / 2]
        })
        .collect();
    // a group of players near the far side still cuts a notch: take the highest floor edge nearby
    let r = (w / 30).max(3);
    (0..w).map(|x| *med[x.saturating_sub(r)..(x + r + 1).min(w)].iter().min().unwrap_or(&h)).collect()
}

/// Ridge test on a scalar image: `sign` 1 = higher than both neighbours `d` px away along one of four
/// directions, -1 = lower. Returns the ridge strength per pixel.
fn ridges(v: &[f32], w: usize, h: usize, sign: f32) -> Vec<f32> {
    let d = ((w as f32 / 480.0).round() as isize).max(2);
    let dirs: [(isize, isize); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];
    let mut resp = vec![0f32; w * h];
    let at = |x: isize, y: isize| v[y as usize * w + x as usize] * sign;
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
    resp
}

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


/// A placement of the field: its four corners in the picture (normalized coordinates, may lie outside
/// `[0, 1]`) in the order far-left, far-right, near-right, near-left, and the lens.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub corners: [Vec2; 4],
    pub lens: Lens,
}

/// Maps court metres into the picture for one placement.
pub struct Mapper {
    /// viewing ray -> court
    h: Homography,
    /// court -> viewing ray
    g: Homography,
    lens: Lens,
    r_max: f64,
    z_sign: f64,
}

impl Mapper {
    pub fn new(p: &Placement, field: (f32, f32)) -> Option<Mapper> {
        let (l, w) = field;
        let court = [Vec2::new(0.0, 0.0), Vec2::new(l, 0.0), Vec2::new(l, w), Vec2::new(0.0, w)];
        let mut rays = [[0f64; 3]; 4];
        for (r, c) in rays.iter_mut().zip(&p.corners) {
            *r = p.lens.ray(c.x as f64, c.y as f64)?;
        }
        let h = Homography::from_rays(&rays, &court)?;
        let g = h.inverse()?;
        let z = h.apply3(rays[0])[2];
        let r_max = p.lens.max_radius();
        Some(Mapper { h, g, lens: p.lens, r_max, z_sign: if z < 0.0 { -1.0 } else { 1.0 } })
    }

    pub fn to_image(&self, c: Vec2) -> Option<Vec2> {
        let mut r = self.g.apply3([c.x as f64, c.y as f64, 1.0]);
        if self.h.apply3(r)[2] * self.z_sign < 0.0 {
            r = [-r[0], -r[1], -r[2]];
        }
        let (u, v) = self.lens.pixel(r, self.r_max)?;
        (u.is_finite() && v.is_finite()).then(|| Vec2::new(u as f32, v as f32))
    }
}

/// What the user says about the picture.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hints {
    /// Corners outside the picture, order far-left, far-right, near-right, near-left.
    pub outside: [bool; 4],
    /// Lens model to use (`None` = plain image).
    pub lens: Option<LensModel>,
}

struct Problem<'a> {
    map: &'a LineMap,
    pts: Vec<Vec2>,
    field: (f32, f32),
    hints: Hints,
    /// Snapping: where the start placement draws each sample, and how far (pixels at 960 px width)
    /// the lines may move from there.
    anchor: Option<(Vec<Option<Vec2>>, f32)>,
}

impl Problem<'_> {
    /// Mean capped squared distance of the visible line samples (in units of `cap`), plus penalties for
    /// corners on the wrong side of the picture edge and for a field that is mostly out of view.
    fn cost(&self, p: &Placement, cap: f32) -> f64 {
        let Some(m) = Mapper::new(p, self.field) else { return 1e9 };
        let cap = cap * self.map.px_scale();
        let (l, w) = self.field;
        let corners = [Vec2::new(0.0, 0.0), Vec2::new(l, 0.0), Vec2::new(l, w), Vec2::new(0.0, w)];
        let mut pen = shape_penalty(&p.corners);
        // along both sidelines the field must run left to right in the picture, without folding
        // several metres onto one spot (a collapsed panorama fit)
        for y in [0.0, w] {
            let u: Vec<f32> = (0..=4).filter_map(|k| m.to_image(Vec2::new(l * k as f32 / 4.0, y))).map(|q| q.x).collect();
            if u.windows(2).any(|a| a[1] - a[0] < 0.01) {
                pen += 1.0;
            }
        }
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
        if let Some((start, limit)) = &self.anchor {
            // only lines right next to the drawn ones count: a placement that moves any visible
            // line further than `limit` is rejected
            let (sx, sy) = (self.map.w as f32 / self.map.px_scale(), self.map.h as f32 / self.map.px_scale());
            let moved = self.pts.iter().zip(start).filter_map(|(c, a)| Some((m.to_image(*c)?, (*a)?))).map(|(q, a)| ((q.x - a.x) * sx).hypot((q.y - a.y) * sy)).fold(0.0f32, f32::max);
            if moved > *limit {
                pen += 1.0 + (moved - limit) as f64 / *limit as f64;
            }
        }
        sum / n as f64 + pen + 0.3 * (1.0 - n as f64 / self.pts.len() as f64)
    }

    /// Share of visible line samples within `px` (at 960 px width) of a line pixel.
    fn agreement(&self, p: &Placement, px: f32) -> f64 {
        let Some(m) = Mapper::new(p, self.field) else { return 0.0 };
        let px = px * self.map.px_scale();
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

/// Starting lenses of the coarse search.
fn start_lenses(model: Option<LensModel>, aspect: f64) -> Vec<Lens> {
    let base = Lens::none(aspect);
    match model {
        None => vec![base],
        Some(LensModel::Division) => [0.0, -0.15, -0.3].iter().map(|&k1| Lens { k1, ..base }).collect(),
        Some(LensModel::Cylinder) => [2.4, 3.0, 3.6].iter().map(|&f| Lens::cylinder(aspect, f, 0.5)).collect(),
    }
}

/// Fit the field (`length`, `width` in metres, its `lines` as polylines) to the line map.
/// `start`: an optional placement to refine (e.g. from points the user already clicked).
pub fn fit(map: &LineMap, lines: &[Vec<Vec2>], field: (f32, f32), aspect: f64, hints: Hints, start: Option<Placement>) -> Option<AutoResult> {
    let coarse = Problem { map, pts: sample_lines(lines, 1.0), field, hints, anchor: None };
    let fine = Problem { map, pts: sample_lines(lines, 0.3), field, hints, anchor: None };

    // coarse search over field shapes seen from a sideline camera, one thread per starting lens
    let lin = |a: f32, b: f32, n: usize| (0..n).map(move |i| a + (b - a) * i as f32 / (n - 1) as f32);
    let grid = |lens: Lens| -> Vec<(f64, Placement)> {
        let mut out = Vec::new();
        for yf in lin(0.05, 0.75, 15) {
            for dy in lin(0.08, 1.3, 15) {
                let yn = yf + dy;
                for hf in lin(0.12, 0.75, 10) {
                    for g in lin(1.0, 4.0, 10) {
                        let hn = hf * g;
                        for cx in [0.42f32, 0.5, 0.58] {
                            let p = Placement { corners: [Vec2::new(cx - hf, yf), Vec2::new(cx + hf, yf), Vec2::new(cx + hn, yn), Vec2::new(cx - hn, yn)], lens };
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
    let lenses = start_lenses(hints.lens, aspect);
    let mut cands: Vec<(f64, Placement)> = std::thread::scope(|sc| {
        let hs: Vec<_> = lenses.iter().map(|&l| sc.spawn(move || grid(l))).collect();
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
                    // the last stage twice: a fresh simplex gets out of a collapsed one
                    for (cap, step) in [(16.0f32, 0.03f64), (6.0, 0.01), (3.0, 0.004), (3.0, 0.002)] {
                        p = refine(fine_ref, p, cap, step);
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

/// Points along the field lines, `step` metres apart.
fn sample_lines(lines: &[Vec<Vec2>], step: f32) -> Vec<Vec2> {
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
}

/// Result of [`snap`].
#[derive(Clone, Copy, Debug)]
pub struct SnapResult {
    pub placement: Placement,
    /// Share of the visible line samples on detected line pixels (within 2 px) before and after.
    pub before: f64,
    pub after: f64,
    /// Largest shift of a visible field line, in pixels at 960 px picture width.
    pub moved_px: f32,
}

/// Pull a placement that is already nearly right (e.g. from clicked points) onto the painted lines
/// right next to it. Only a local search: no line moves further than `max_px` (pixels at 960 px
/// width), corners stay on their side of the picture edge. `None` when nothing fits better.
pub fn snap(map: &LineMap, lines: &[Vec<Vec2>], field: (f32, f32), start: Placement, model: Option<LensModel>, max_px: f32) -> Option<SnapResult> {
    let m0 = Mapper::new(&start, field)?;
    let (l, w) = field;
    let mut outside = [false; 4];
    for (o, c) in outside.iter_mut().zip([Vec2::new(0.0, 0.0), Vec2::new(l, 0.0), Vec2::new(l, w), Vec2::new(0.0, w)]) {
        *o = !m0.to_image(c).map(|q| q.x > 0.005 && q.y > 0.005 && q.x < 0.995 && q.y < 0.995).unwrap_or(false);
    }
    let pts = sample_lines(lines, 0.25);
    let anchor: Vec<Option<Vec2>> = pts.iter().map(|c| m0.to_image(*c)).collect();
    let pr = Problem { map, pts, field, hints: Hints { outside, lens: model }, anchor: Some((anchor, max_px)) };
    let before = pr.agreement(&start, 2.0);
    let mut p = start;
    // coarse-to-fine distance caps; the caps stay small so only the neighbouring lines pull
    for (cap, step) in [(8.0f32, 0.004f64), (4.0, 0.002), (2.5, 0.001), (2.5, 0.0005)] {
        p = refine(&pr, p, cap, step);
    }
    let after = pr.agreement(&p, 2.0);
    // the shape rules of the coarse search may already be broken by the start (a 180° panorama looks
    // flat): only nothing may get worse than it was
    if pr.cost(&p, 2.5) >= pr.cost(&start, 2.5) || after < before + 0.005 {
        return None;
    }
    let m1 = Mapper::new(&p, field)?;
    let (sw, sh) = (map.w as f32 / map.px_scale(), map.h as f32 / map.px_scale());
    let moved_px = pr.pts.iter().filter_map(|c| Some((m0.to_image(*c)?, m1.to_image(*c)?))).filter(|(a, _)| a.x >= 0.0 && a.y >= 0.0 && a.x <= 1.0 && a.y <= 1.0).map(|(a, b)| ((a.x - b.x) * sw).hypot((a.y - b.y) * sh)).fold(0.0f32, f32::max);
    if moved_px > max_px * 1.05 {
        return None;
    }
    Some(SnapResult { placement: p, before, after, moved_px })
}

fn refine(pr: &Problem, p: Placement, cap: f32, step: f64) -> Placement {
    let mut x: Vec<f64> = p.corners.iter().flat_map(|c| [c.x as f64, c.y as f64]).collect();
    let model = pr.hints.lens;
    match model {
        Some(LensModel::Division) => x.push(p.lens.k1),
        Some(LensModel::Cylinder) => {
            x.push(p.lens.fov / 10.0); // scaled so one simplex step suits all parameters
            x.push(p.lens.cy);
        }
        None => {}
    }
    let base = p.lens;
    let unpack = |x: &[f64]| {
        let mut lens = base;
        match model {
            Some(LensModel::Division) => lens.k1 = x[8].clamp(-0.6, 0.1),
            Some(LensModel::Cylinder) => {
                lens.fov = (x[8] * 10.0).clamp(0.5, 6.0);
                lens.cy = x[9].clamp(0.0, 1.0);
            }
            None => {}
        }
        Placement { corners: [0, 1, 2, 3].map(|i| Vec2::new(x[2 * i] as f32, x[2 * i + 1] as f32)), lens }
    };
    let f = |x: &[f64]| pr.cost(&unpack(x), cap);
    let (best, _) = nelder_mead(&f, &x, step, 300 * x.len());
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

    fn field(prof: &SportProfile) -> (f32, f32) {
        (prof.field.length, prof.field.width)
    }

    fn paint(rgb: &mut [u8], w: usize, h: usize, m: &Mapper, lines: &[Vec<Vec2>], c: [u8; 3], r: isize) {
        for pl in lines {
            for (i, p) in pl.iter().enumerate().skip(1) {
                let q = pl[i - 1];
                let n = ((p.dist(q) / 0.02).ceil() as usize).max(1);
                for k in 0..=n {
                    let Some(px) = m.to_image(q.lerp(*p, k as f32 / n as f32)) else { continue };
                    let (cx, cy) = ((px.x * w as f32) as isize, (px.y * h as f32) as isize);
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let (x, y) = (cx + dx, cy + dy);
                            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                                let i = (y as usize * w + x as usize) * 3;
                                rgb[i..i + 3].copy_from_slice(&c);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Floor-coloured frame with mild texture and the field lines of `prof` in white.
    fn render(prof: &SportProfile, p: &Placement, w: usize, h: usize) -> Vec<u8> {
        let m = Mapper::new(p, field(prof)).unwrap();
        let mut rgb = vec![0u8; w * h * 3];
        for (i, px) in rgb.chunks_mut(3).enumerate() {
            let t = (((i % w) * 7 + (i / w) * 13) % 17) as u8;
            px.copy_from_slice(&[140 + t, 105 + t, 70 + t]);
        }
        paint(&mut rgb, w, h, &m, &prof.densified_lines(0.5), [240, 240, 240], 1);
        rgb
    }

    /// Largest distance (pixels of a `w`×`h` frame) between where the two placements put a landmark
    /// that is in the picture.
    fn worst_landmark_error(prof: &SportProfile, a: &Placement, b: &Placement, w: f32, h: f32) -> f32 {
        let (ma, mb) = (Mapper::new(a, field(prof)).unwrap(), Mapper::new(b, field(prof)).unwrap());
        prof.landmarks
            .iter()
            .filter_map(|l| {
                let c = Vec2::new(l.x, l.y);
                let (p, q) = (ma.to_image(c)?, mb.to_image(c)?);
                let inside = |v: Vec2| v.x > 0.0 && v.y > 0.0 && v.x < 1.0 && v.y < 1.0;
                (inside(p) || inside(q)).then(|| ((p.x - q.x) * w).hypot((p.y - q.y) * h))
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn finds_a_basketball_court_with_near_corners_out_of_view() {
        let prof = SportProfile::basketball();
        let (w, h) = (1280, 720);
        let aspect = w as f64 / h as f64;
        let truth = Placement { corners: [Vec2::new(0.27, 0.36), Vec2::new(0.74, 0.37), Vec2::new(1.12, 1.06), Vec2::new(-0.1, 1.04)], lens: Lens::none(aspect) };
        let rgb = render(&prof, &truth, w, h);
        let map = LineMap::from_rgb(&rgb, w, h, None);
        let hints = Hints { outside: [false, false, true, true], lens: None };
        let r = fit(&map, &prof.lines, field(&prof), aspect, hints, None).expect("found");
        let err = worst_landmark_error(&prof, &r.placement, &truth, w as f32, h as f32);
        assert!(err < 6.0, "landmarks off by {err} px, agreement {}, got {:?}", r.agreement, r.placement);
        assert!(r.agreement > 0.8, "agreement {}", r.agreement);
    }

    /// A calibration a few pixels off snaps onto the painted lines; one far off is left alone.
    #[test]
    fn snaps_a_nearly_right_court_onto_the_lines() {
        let prof = SportProfile::basketball();
        let (w, h) = (1280, 720);
        let aspect = w as f64 / h as f64;
        let truth = Placement { corners: [Vec2::new(0.27, 0.36), Vec2::new(0.74, 0.37), Vec2::new(1.12, 1.06), Vec2::new(-0.1, 1.04)], lens: Lens::none(aspect) };
        let map = LineMap::from_rgb(&render(&prof, &truth, w, h), w, h, None);
        let off = |d: f32| Placement { corners: [Vec2::new(0.27 + d, 0.36 + d * 0.5), Vec2::new(0.74 + d * 0.6, 0.37 - d * 0.4), Vec2::new(1.12, 1.06), Vec2::new(-0.1, 1.04)], ..truth };
        let start = off(0.004);
        let before = worst_landmark_error(&prof, &start, &truth, w as f32, h as f32);
        let r = snap(&map, &prof.lines, field(&prof), start, None, 8.0).expect("snapped");
        let after = worst_landmark_error(&prof, &r.placement, &truth, w as f32, h as f32);
        assert!(after < before * 0.5 && after < 2.5, "landmarks off by {before} px before, {after} px after");
        assert!(r.after > r.before, "agreement {} -> {}", r.before, r.after);
        // 40 px off: no line lies close enough, the snap must not jump to some other line
        assert!(snap(&map, &prof.lines, field(&prof), off(0.04), None, 8.0).map(|r| r.moved_px <= 8.4).unwrap_or(true));
    }

    #[test]
    fn finds_a_fisheye_full_court() {
        let prof = SportProfile::basketball();
        let (w, h) = (1280, 720);
        let aspect = w as f64 / h as f64;
        let lens = Lens { k1: -0.3, ..Lens::none(aspect) };
        let truth = Placement { corners: [Vec2::new(0.26, 0.42), Vec2::new(0.74, 0.42), Vec2::new(0.9, 0.8), Vec2::new(0.1, 0.8)], lens };
        let rgb = render(&prof, &truth, w, h);
        let map = LineMap::from_rgb(&rgb, w, h, None);
        let hints = Hints { outside: [false; 4], lens: Some(LensModel::Division) };
        let r = fit(&map, &prof.lines, field(&prof), aspect, hints, None).expect("found");
        let err = worst_landmark_error(&prof, &r.placement, &truth, w as f32, h as f32);
        assert!(err < 6.0, "landmarks off by {err} px, k1 {}, agreement {}", r.placement.lens.k1, r.agreement);
    }

    /// A 180° panorama of a multi-sport hall: dark basketball lines, bright and green lines of other
    /// sports, a bright wall with window bars above the floor. The line colour is picked.
    #[test]
    fn finds_a_panorama_court_by_its_line_colour() {
        let prof = SportProfile::basketball();
        let (w, h) = (1920usize, 520usize);
        let aspect = w as f64 / h as f64;
        let lens = Lens::cylinder(aspect, 2.8, 0.38);
        // camera 1.5 m behind the near sideline at the halfway line, 5 m high: about 160° wide, the
        // near corners lie beside the picture
        let img = |x: f32, y: f32| -> Vec2 {
            let (dx, dz, dy) = ((x - 14.0) as f64, (15.0 + 1.5 - y) as f64, 5.0f64);
            let th = dx.atan2(dz);
            Vec2::new((0.5 + th / lens.fov) as f32, (lens.cy + dy / dx.hypot(dz) * aspect / lens.fov) as f32)
        };
        let corners = [img(0.0, 0.0), img(28.0, 0.0), img(28.0, 15.0), img(0.0, 15.0)];
        let truth = Placement { corners, lens };
        assert!(corners[2].x > 1.0 && corners[3].x < 0.0 && corners[0].x > 0.0, "{corners:?}");
        let m = Mapper::new(&truth, field(&prof)).unwrap();
        // floor below the wall line (just behind the far sideline), wall and windows above
        let wall_y = (img(14.0, -4.0).y * h as f32) as usize;
        let mut rgb = vec![0u8; w * h * 3];
        for (i, px) in rgb.chunks_mut(3).enumerate() {
            let (x, y) = (i % w, i / w);
            let t = ((x * 7 + y * 13) % 11) as u8;
            if y < wall_y {
                let bar = x % 60 < 3 || y % 40 < 3;
                px.copy_from_slice(if bar { &[60, 60, 60] } else { &[235, 238, 240] });
            } else {
                px.copy_from_slice(&[205 + t / 2, 160 + t, 70 + t]);
            }
        }
        // other sports: a handball-sized field in white, shifted, and green lines
        let other = |dx: f32, dy: f32, l: f32, wd: f32| vec![vec![Vec2::new(dx, dy), Vec2::new(dx + l, dy), Vec2::new(dx + l, dy + wd), Vec2::new(dx, dy + wd), Vec2::new(dx, dy)], vec![Vec2::new(dx + l / 2.0, dy), Vec2::new(dx + l / 2.0, dy + wd)]];
        let densify = |ls: Vec<Vec<Vec2>>| ls;
        paint(&mut rgb, w, h, &m, &densify(other(-6.0, -2.5, 40.0, 20.0)), [245, 245, 245], 1);
        paint(&mut rgb, w, h, &m, &densify(other(2.0, -1.0, 24.0, 17.0)), [60, 150, 70], 1);
        let dark = [40u8, 45, 90];
        paint(&mut rgb, w, h, &m, &prof.densified_lines(0.5), dark, 1);
        let map = LineMap::from_rgb(&rgb, w, h, Some(dark));
        // wall and windows are not floor
        assert!(map.floor_top[map.w / 2] + CELL >= wall_y * map.w / w, "floor top {} wall {}", map.floor_top[map.w / 2], wall_y);
        let hints = Hints { outside: [false, false, true, true], lens: Some(LensModel::Cylinder) };
        let r = fit(&map, &prof.lines, field(&prof), aspect, hints, None).expect("found");
        let err = worst_landmark_error(&prof, &r.placement, &truth, w as f32, h as f32);
        assert!(err < 6.0, "landmarks off by {err} px, fov {}, cy {}, agreement {}", r.placement.lens.fov, r.placement.lens.cy, r.agreement);
    }

    #[test]
    fn line_map_marks_thin_lines_and_picks_the_colour() {
        let (w, h) = (200, 100);
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let c: [u8; 3] = match x {
                    60..=61 => [250, 250, 250], // bright line
                    98..=99 => [30, 30, 80],     // dark blue line
                    130..=131 => [50, 150, 60],  // green line
                    _ => [200, 160, 80],         // floor
                };
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
            }
        }
        let any = LineMap::from_rgb(&rgb, w, h, None);
        let at = |m: &LineMap, x: usize| (x.saturating_sub(1)..=x + 1).any(|xx| m.mask[50 * m.w + xx] == 1);
        assert!(at(&any, 60) && at(&any, 98), "bright and dark lines without a colour");
        assert!(!at(&any, 30));
        let blue = LineMap::from_rgb(&rgb, w, h, Some([30, 30, 80]));
        assert!(at(&blue, 98) && !at(&blue, 60) && !at(&blue, 130), "only the picked colour");
        // struck out by hand: a click removes the whole line, a brush only its circle
        let gone = any.without(&[Erase::Piece(Vec2::new(60.5 / w as f32, 0.2))]);
        assert!(!at(&gone, 60) && at(&gone, 98));
        let brushed = any.without(&[Erase::Brush(Vec2::new(99.0 / w as f32, 0.5), 0.03)]);
        assert!(!at(&brushed, 98) && brushed.mask[10 * brushed.w + 98] + brushed.mask[10 * brushed.w + 99] > 0);
    }
}
