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
    plan_auto_roi(s, frame_w, frame_h, overlap, Tile { x: 0, y: 0, w: frame_w, h: frame_h })
}

/// Cover all requested foot-position bands, including the near sideline.
/// ROI is an explicit pixel-space search boundary, not inferred from sport rules.
/// Keep room for bodies/airborne balls when choosing it. Invalid scales produce
/// no tiles rather than panicking, looping forever or wrapping coordinates.
pub fn plan_auto_roi(s: &Scales, frame_w: u32, frame_h: u32, overlap: f32, roi: Tile) -> Vec<Tile> {
    let mut tiles = Vec::new();
    if ![s.a, s.b, s.y_min, s.y_max, s.target_px, overlap].iter().all(|v| v.is_finite()) || s.target_px <= 0.0 || s.b < 0.0 || s.y_max <= s.y_min {
        return tiles;
    }
    let right = roi.x.saturating_add(roi.w).min(frame_w);
    let bottom = roi.y.saturating_add(roi.h).min(frame_h);
    if right <= roi.x || bottom <= roi.y { return tiles; }
    let mut y = s.y_min.max(roi.y as f32);
    let end = s.y_max.min(bottom as f32);
    // The old six-band cap silently omitted the lower field on steep fits.
    // A minimum 10-pixel advance bounds the loop without truncating coverage.
    while y < end {
        let h0 = s.height_at(y);
        let mut y1 = if s.b > 1e-4 { (h0 * 1.25 - s.a) / s.b } else { end };
        y1 = y1.max(y + 10.0).min(end);
        if end - y1 < 0.05 * (end - s.y_min) {
            y1 = end;
        }
        let side = s.tile_at((y + y1) / 2.0).round() as u32;
        let top = (y - h0 * 1.1).max(roi.y as f32).floor() as u32;
        let bot = ((y1 + 0.1 * h0).ceil() as u32).min(bottom);
        if bot > top {
            let band = Tile { x: roi.x, y: top, w: right - roi.x, h: bot - top };
            tiles.extend(plan_tiles(band, side, overlap));
        }
        y = y1;
    }
    tiles
}
