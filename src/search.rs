//! Image-space prediction for opt-in adaptive detection. This is search evidence,
//! not player identity or proof of ball possession. Missing observations survive
//! briefly with growing uncertainty; independent sweeps still recover new objects.
use crate::{
    detect::{Class, Det, Tile},
    scheduler::{RecoverySweep, TileScheduler},
    tracker::BBox,
};

#[derive(Clone)]
struct Evidence {
    bbox: BBox,
    class: Class,
    seen: f64,
    vx: f32,
    vy: f32,
    residual: f32,
}

#[derive(Debug)]
pub struct SearchPlan {
    pub indices: Vec<usize>,
    pub full_sweep: bool,
    pub recovery_tiles: usize,
    pub coasting_targets: usize,
}

pub struct MotionSearch {
    scheduler: TileScheduler,
    recovery: RecoverySweep,
    evidence: Vec<Evidence>,
    last_t: Option<f64>,
}

impl MotionSearch {
    pub fn new(full_interval: f64) -> Self {
        Self {
            scheduler: TileScheduler::new(full_interval),
            recovery: RecoverySweep::default(),
            evidence: Vec::new(),
            last_t: None,
        }
    }

    fn lifetime(class: Class) -> f64 {
        if class == Class::Ball {
            0.4
        } else {
            1.0
        }
    }

    /// A swept region covers both the last observation and its prediction. The
    /// acceleration allowance grows quadratically with elapsed VIDEO time.
    fn region(e: &Evidence, t: f64) -> BBox {
        let dt = (t - e.seen).max(0.0) as f32;
        let (dx, dy) = (e.vx * dt, e.vy * dt);
        let accel = if e.class == Class::Ball { 0.8 } else { 0.15 };
        let pad = e.residual * dt + accel * dt * dt;
        BBox {
            cx: e.bbox.cx + dx * 0.5,
            cy: e.bbox.cy + dy * 0.5,
            w: e.bbox.w + dx.abs() + 2.0 * pad,
            h: e.bbox.h + dy.abs() + 2.0 * pad,
        }
    }

    pub fn select(
        &mut self,
        t: f64,
        tiles: &[Tile],
        w: u32,
        h: u32,
        recovery_budget: usize,
    ) -> SearchPlan {
        if !t.is_finite() || self.last_t.is_some_and(|p| t < p || t - p > 3.0) {
            self.evidence.clear();
        }
        self.evidence
            .retain(|e| t - e.seen <= Self::lifetime(e.class) && t >= e.seen);
        let targets: Vec<_> = self.evidence.iter().map(|e| Self::region(e, t)).collect();
        let coasting_targets = self.evidence.iter().filter(|e| t - e.seen > 0.1).count();
        let (mut indices, full_sweep) = self.scheduler.select(t, tiles, w, h, &targets);
        let before = indices.len();
        if !full_sweep {
            indices.extend(self.recovery.select(tiles.len(), recovery_budget));
            indices.sort_unstable();
            indices.dedup();
        }
        SearchPlan {
            recovery_tiles: indices.len() - before,
            indices,
            full_sweep,
            coasting_targets,
        }
    }

    /// Match only to estimate search motion. All candidates, including unknown
    /// teams and unassigned people, remain searchable. Each old observation can
    /// support at most one new one. No identity decisions are made here.
    pub fn observe(&mut self, t: f64, dets: &[Det]) {
        if !t.is_finite() {
            self.evidence.clear();
            self.last_t = None;
            return;
        }
        if self.last_t.is_some_and(|p| t <= p || t - p > 3.0) {
            self.evidence.clear();
        }
        self.last_t = Some(t);
        self.evidence
            .retain(|e| t - e.seen <= Self::lifetime(e.class));
        let mut used = vec![false; self.evidence.len()];
        let mut next = Vec::new();
        for d in dets
            .iter()
            .filter(|d| {
                [d.cx, d.cy, d.w, d.h, d.conf].iter().all(|x| x.is_finite())
                    && d.w > 0.0
                    && d.h > 0.0
            })
            .take(256)
        {
            let bbox = BBox {
                cx: d.cx,
                cy: d.cy,
                w: d.w,
                h: d.h,
            };
            let best = self
                .evidence
                .iter()
                .enumerate()
                .filter(|(i, e)| !used[*i] && e.class == d.class)
                .filter_map(|(i, e)| {
                    let dt = (t - e.seen) as f32;
                    let dist = ((d.cx - e.bbox.cx - e.vx * dt).powi(2)
                        + (d.cy - e.bbox.cy - e.vy * dt).powi(2))
                    .sqrt();
                    let gate = e.bbox.w.max(e.bbox.h) * 1.5
                        + dt * if d.class == Class::Ball { 2.0 } else { 0.3 };
                    (dist <= gate).then_some((i, dist))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let mut item = Evidence {
                bbox,
                class: d.class,
                seen: t,
                vx: 0.0,
                vy: 0.0,
                residual: 0.0,
            };
            if let Some((i, residual)) = best {
                used[i] = true;
                let e = &self.evidence[i];
                let dt = (t - e.seen) as f32;
                if (0.005..=0.5).contains(&dt) {
                    let limit = if d.class == Class::Ball { 4.0 } else { 1.0 };
                    item.vx = (0.5 * e.vx + 0.5 * (d.cx - e.bbox.cx) / dt).clamp(-limit, limit);
                    item.vy = (0.5 * e.vy + 0.5 * (d.cy - e.bbox.cy) / dt).clamp(-limit, limit);
                    item.residual = (residual / dt).min(limit);
                }
            }
            next.push(item);
        }
        for (i, e) in self.evidence.drain(..).enumerate() {
            if !used[i] && next.len() < 256 {
                next.push(e);
            }
        }
        self.evidence = next;
    }
}
