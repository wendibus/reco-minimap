//! Ball state machine: Held / InFlight / Loose / Unknown.
//!
//! The ball is tiny and often hidden in hands, so "ball not detected while
//! held" is normal and does NOT release it. A release needs a ball
//! detection that stays outside the holder's box.
//!
//! Caveat: a flying ball is above the floor, so projecting it onto the floor
//! plane displaces it (mostly along the camera's depth axis). The minimap
//! shows it as a small orange dot at the smoothed projected position, clamped
//! to the court. Offline refinement (parabola fit between release and catch)
//! is a later step.

use crate::geom::{clamp_to_court, Vec2};
use crate::tracker::{BBox, Track, TrackState};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BallState {
    Held { track: u32 },
    InFlight,
    Loose,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct BallOut {
    pub state: BallState,
    pub pos: Option<Vec2>,
}

#[derive(Clone, Debug)]
pub struct BallCfg {
    /// Grow player boxes by this fraction when testing "ball in hands".
    pub hold_expand: f32,
    pub hold_confirm: f32,
    pub release_confirm: f32,
    /// Court speed (m/s) above which a free ball counts as flying.
    pub flight_speed: f32,
    pub coast_secs: f32,
    pub smooth_tau: f32,
}

impl Default for BallCfg {
    fn default() -> Self {
        Self { hold_expand: 0.15, hold_confirm: 0.10, release_confirm: 0.10, flight_speed: 3.0, coast_secs: 0.7, smooth_tau: 0.06 }
    }
}

/// Ball detection for one frame.
#[derive(Clone, Copy, Debug)]
pub struct BallObs {
    pub bbox: BBox,
    /// Ground-projected court position (see module caveat).
    pub court: Vec2,
    pub conf: f32,
}

pub struct BallTracker {
    pub cfg: BallCfg,
    state: BallState,
    pos: Vec2,
    vel: Vec2,
    have_pos: bool,
    last_seen: f64,
    last_t: Option<f64>,
    cand: Option<(u32, f64)>,
    release_since: Option<f64>,
    slow_since: Option<f64>,
}

fn contains(b: &BBox, x: f32, y: f32, expand: f32) -> bool {
    let (hw, hh) = (b.w * (0.5 + expand), b.h * (0.5 + expand));
    (x - b.cx).abs() <= hw && (y - b.cy).abs() <= hh
}

impl BallTracker {
    pub fn new(cfg: BallCfg) -> Self {
        Self { cfg, state: BallState::Unknown, pos: Vec2::default(), vel: Vec2::default(), have_pos: false, last_seen: f64::NEG_INFINITY, last_t: None, cand: None, release_since: None, slow_since: None }
    }

    pub fn state(&self) -> BallState {
        self.state
    }

    pub fn update(&mut self, t: f64, obs: Option<BallObs>, players: &[Track]) -> BallOut {
        let dt = match self.last_t {
            Some(p) => ((t - p) as f32).clamp(0.001, 0.5),
            None => 0.04,
        };
        self.last_t = Some(t);

        // smooth free-ball position
        if let Some(o) = obs {
            if self.have_pos {
                let a = 1.0 - (-dt / self.cfg.smooth_tau).exp();
                let np = self.pos.lerp(o.court, a);
                let inst = (np - self.pos) * (1.0 / dt);
                self.vel = self.vel.lerp(inst, 0.4);
                self.pos = np;
            } else {
                self.pos = o.court;
                self.have_pos = true;
            }
            self.last_seen = t;
        } else if self.have_pos && (t - self.last_seen) as f32 <= self.cfg.coast_secs {
            self.pos = self.pos + self.vel * dt;
            self.vel = self.vel * (-dt / 0.5).exp();
        }
        self.pos = clamp_to_court(self.pos, 1.5);

        // who could be holding the ball (image space)
        let mut cand: Option<u32> = None;
        if let Some(o) = obs {
            let mut best = f32::INFINITY;
            for p in players {
                if p.state == TrackState::Lost {
                    continue;
                }
                if let Some(b) = &p.bbox {
                    let ex = if matches!(self.state, BallState::Held { track } if track == p.id) { self.cfg.hold_expand * 2.0 } else { self.cfg.hold_expand };
                    if contains(b, o.bbox.cx, o.bbox.cy, ex) {
                        let d = (o.bbox.cx - b.cx).abs() / b.w.max(1e-6) + (o.bbox.cy - b.cy).abs() / b.h.max(1e-6) * 0.5;
                        if d < best {
                            best = d;
                            cand = Some(p.id);
                        }
                    }
                }
            }
        }

        let speed = self.vel.len();
        match (self.state, obs.is_some(), cand) {
            // ---- ball seen inside somebody's box
            (BallState::Held { track }, true, Some(id)) if id == track => {
                self.release_since = None;
                self.cand = None;
            }
            (_, true, Some(id)) => {
                self.release_since = None;
                match self.cand {
                    Some((cid, since)) if cid == id => {
                        if (t - since) as f32 >= self.cfg.hold_confirm {
                            self.state = BallState::Held { track: id };
                            self.cand = None;
                        }
                    }
                    _ => self.cand = Some((id, t)),
                }
            }
            // ---- ball seen, but in nobody's box
            (BallState::Held { track }, true, None) => {
                self.cand = None;
                let since = *self.release_since.get_or_insert(t);
                if (t - since) as f32 >= self.cfg.release_confirm {
                    let above = players.iter().find(|p| p.id == track).and_then(|p| p.bbox).map(|b| obs.map(|o| o.bbox.cy < b.cy - b.h * 0.5).unwrap_or(false)).unwrap_or(false);
                    self.state = if speed > self.cfg.flight_speed || above { BallState::InFlight } else { BallState::Loose };
                    self.release_since = None;
                    self.slow_since = None;
                }
            }
            (BallState::InFlight, true, None) => {
                self.cand = None;
                if speed < 1.0 {
                    let since = *self.slow_since.get_or_insert(t);
                    if (t - since) >= 0.5 {
                        self.state = BallState::Loose;
                        self.slow_since = None;
                    }
                } else {
                    self.slow_since = None;
                }
            }
            (BallState::Loose, true, None) | (BallState::Unknown, true, None) => {
                self.cand = None;
                self.state = if speed > self.cfg.flight_speed { BallState::InFlight } else { BallState::Loose };
            }
            // ---- ball not seen
            (BallState::Held { .. }, false, _) => {} // hidden in hands: stay
            (_, false, _) => {
                self.cand = None;
                if (t - self.last_seen) as f32 > self.cfg.coast_secs {
                    self.state = BallState::Unknown;
                }
            }
        }

        // holder gone -> unknown
        if let BallState::Held { track } = self.state {
            if !players.iter().any(|p| p.id == track && p.state != TrackState::Lost) {
                self.state = BallState::Unknown;
            }
        }

        let pos = match self.state {
            BallState::Held { track } => players.iter().find(|p| p.id == track).map(|p| p.pos),
            BallState::Unknown => None,
            _ => {
                if self.have_pos { Some(self.pos) } else { None }
            }
        };
        BallOut { state: self.state, pos }
    }
}
