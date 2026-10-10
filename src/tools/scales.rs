//! Perspective-aware tiling.
//!
//! The model finds players best when they are ~25-35 px tall in its 512 px
//! input. On a panorama a player is ~25 px tall at the far sideline and
//! ~75 px at the near one, so one fixed tile size cannot serve the whole
//! court. We measure player height vs. foot-y from real detections
//! (`reco-minimap scales`), fit `h(y) = a + b*y`, and cut the court into
//! horizontal bands, each with the tile size that makes players ~`target_px`
//! tall in the model input.

use serde_json::{json, Value};

use crate::detect::{plan_tiles, Tile};
use crate::geom::{Field, Homography, Vec2};

#[derive(Clone, Debug)]
pub struct Scales {
    pub a: f32,
    pub b: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub target_px: f32,
    pub n: usize,
}

fn median(v: &mut Vec<f32>) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

fn ls(points: &[(f32, f32)]) -> (f32, f32) {
    let n = points.len() as f32;
    let (my, mh) = (points.iter().map(|p| p.0).sum::<f32>() / n, points.iter().map(|p| p.1).sum::<f32>() / n);
    let (mut cov, mut var) = (0.0, 0.0);
    for p in points {
        cov += (p.0 - my) * (p.1 - mh);
        var += (p.0 - my) * (p.0 - my);
    }
    let b = if var > 1e-6 { cov / var } else { 0.0 };
    (mh - b * my, b)
}

impl Scales {
    pub fn height_at(&self, y: f32) -> f32 {
        (self.a + self.b * y).max(8.0)
    }

    /// Tile side (source px) that makes a player at foot-y `y` ~`target_px` tall in the input.
    pub fn tile_at(&self, y: f32) -> f32 {
        (512.0 * self.height_at(y) / self.target_px).clamp(384.0, 1536.0)
    }

    /// Robust-ish fit of (foot_y, height) points: LS, trim residuals > 2.5 sigma, refit.
    pub fn fit(points: &[(f32, f32)], y_min: f32, y_max: f32, target_px: f32) -> Option<Self> {
        if points.len() < 20 || y_max <= y_min {
            return None;
        }
        let (mut a, mut b) = ls(points);
        let mut kept: Vec<(f32, f32)> = points.to_vec();
        for _ in 0..2 {
            let mut res: Vec<f32> = kept.iter().map(|p| (p.1 - (a + b * p.0)).abs()).collect();
            let sigma = (1.4826 * median(&mut res)).max(1.0);
            let next: Vec<(f32, f32)> = kept.iter().copied().filter(|p| (p.1 - (a + b * p.0)).abs() <= 2.5 * sigma).collect();
            if next.len() < 10 {
                break;
            }
            kept = next;
            let r = ls(&kept);
            a = r.0;
            b = r.1;
        }
        if b < 0.0 {
            // perspective says near players are larger; a negative slope means noise -> flat
            let mut h: Vec<f32> = kept.iter().map(|p| p.1).collect();
            a = median(&mut h);
            b = 0.0;
        }
        Some(Self { a, b, y_min, y_max, target_px, n: kept.len() })
    }

    pub fn to_json(&self) -> Value {
        json!({"version": 1, "a": self.a, "b": self.b, "y_min": self.y_min, "y_max": self.y_max, "target_px": self.target_px, "n": self.n})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self { a: v["a"].as_f64()? as f32, b: v["b"].as_f64()? as f32, y_min: v["y_min"].as_f64()? as f32, y_max: v["y_max"].as_f64()? as f32, target_px: v["target_px"].as_f64()? as f32, n: v["n"].as_u64().unwrap_or(0) as usize })
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e} (erst `reco-minimap scales` ausführen)"))?;
        Self::from_json(&serde_json::from_str(&txt).map_err(|e| format!("{path}: {e}"))?).ok_or_else(|| format!("{path}: ungültig"))
    }
}

/// Bands over the court; each band is tiled with the size that suits its distance.
pub fn plan_auto(s: &Scales, frame_w: u32, frame_h: u32, overlap: f32) -> Vec<Tile> {
    plan_bands(s, frame_w, frame_h, overlap, None, None)
}

/// Like [`plan_auto`], but each band only spans the part of the panorama where the field is.
/// `floor_px`: floor points of the field (grown by the margin in which detections are still used)
/// in panorama pixels, see [`floor_points`]. The far bands use small tiles, so not tiling the
/// stands left and right of the field saves most there.
///
/// `feet_px`: floor points where players can stand (the field grown by the player margin). The
/// bands then run from its far to its near edge instead of only over the rows in which players
/// happened to be measured; a player at the near sideline is otherwise cut off or missed.
pub fn plan_auto_field(s: &Scales, frame_w: u32, frame_h: u32, overlap: f32, floor_px: &[(f32, f32)], feet_px: Option<&[(f32, f32)]>) -> Vec<Tile> {
    plan_bands(s, frame_w, frame_h, overlap, Some(floor_px), feet_px)
}

/// Floor points of `field` grown by `margin_m`, on a 0.25 m grid, as panorama pixels.
pub fn floor_points(court_to_pano: &Homography, field: &Field, margin_m: f32, frame_w: u32, frame_h: u32) -> Vec<(f32, f32)> {
    let step = 0.25f32;
    let (nx, ny) = (((field.length + 2.0 * margin_m) / step).ceil() as usize, ((field.width + 2.0 * margin_m) / step).ceil() as usize);
    let mut out = Vec::with_capacity((nx + 1) * (ny + 1));
    for j in 0..=ny {
        for i in 0..=nx {
            let c = Vec2::new((-margin_m + i as f32 * step).min(field.length + margin_m), (-margin_m + j as f32 * step).min(field.width + margin_m));
            if let Some(p) = court_to_pano.apply(c) {
                out.push((p.x * frame_w as f32, p.y * frame_h as f32));
            }
        }
    }
    out
}

fn plan_bands(s: &Scales, frame_w: u32, frame_h: u32, overlap: f32, floor_px: Option<&[(f32, f32)]>, feet_px: Option<&[(f32, f32)]>) -> Vec<Tile> {
    let mut tiles = Vec::new();
    // Rows in which feet can stand. The measured range only reaches as far as players happened to
    // stand in the sample frames; with the field known, the bands cover it from sideline to sideline.
    let (mut y_min, mut y_max) = (s.y_min, s.y_max);
    if let Some(pts) = feet_px {
        for p in pts.iter().filter(|p| p.0 >= 0.0 && p.0 <= frame_w as f32) {
            y_min = y_min.min(p.1.max(0.0));
            y_max = y_max.max(p.1.min(frame_h as f32));
        }
    }
    let mut y = y_min;
    let mut bands = 0;
    // every band covers a 25 % growth in player height; the count is only bounded against a degenerate fit
    while y < y_max - 1.0 && bands < 32 {
        let h0 = s.height_at(y);
        let mut y1 = if s.b > 1e-4 { (h0 * 1.25 - s.a) / s.b } else { y_max };
        y1 = y1.clamp(y + 10.0, y_max);
        if y_max - y1 < 0.05 * (y_max - y_min) {
            y1 = y_max;
        }
        let side = s.tile_at((y + y1) / 2.0).round() as u32;
        let top = (y - h0 * 1.1).max(0.0);
        let bot = (y1 + 0.1 * h0).min(frame_h as f32);
        let (band_y, band_y1) = (y, y1);
        y = y1;
        bands += 1;
        let (mut x0, mut x1) = (0.0f32, frame_w as f32);
        if let Some(pts) = floor_px {
            // feet in this band stand between these columns; a body is at most about half its height wide
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            // the first band also serves whoever stands above the measured rows (far corners of the margin)
            let from = if bands == 1 { f32::NEG_INFINITY } else { band_y };
            for p in pts.iter().filter(|p| p.1 >= from && p.1 <= band_y1) {
                lo = lo.min(p.0);
                hi = hi.max(p.0);
            }
            if lo > hi {
                continue; // no field in these rows
            }
            let pad = 0.5 * s.height_at(band_y1) + 8.0;
            (x0, x1) = ((lo - pad).max(0.0), (hi + pad).min(frame_w as f32));
            // a region narrower than the tile would be magnified beyond the intended scale
            let want = (side.min(frame_w)) as f32;
            if x1 - x0 < want {
                let c = ((x0 + x1) / 2.0).clamp(want / 2.0, frame_w as f32 - want / 2.0);
                (x0, x1) = (c - want / 2.0, c + want / 2.0);
            }
        }
        let roi = Tile { x: x0 as u32, y: top as u32, w: ((x1 - x0).ceil() as u32).min(frame_w - x0 as u32), h: ((bot - top) as u32).max(32) };
        tiles.extend(plan_tiles(roi, side, overlap));
    }
    tiles
}
