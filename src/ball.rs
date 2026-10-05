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

use crate::geom::{Field, Vec2};
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
    /// Velocity in m/s if known (holder's velocity while held).
    pub vel: Option<Vec2>,
}

#[derive(Clone, Debug)]
pub struct BallCfg {
    pub field: Field,
    /// Grow player boxes by this fraction when testing "ball in hands".
    pub hold_expand: f32,
    /// Consecutive ball detections inside the same player's box to call it "held".
    /// Counted in processed frames, so it also works when only every Nth video frame is analysed.
    pub hold_frames: u32,
    /// Consecutive detections outside the holder's box to call it released.
    pub release_frames: u32,
    /// Consecutive fast detections to call a free ball "in flight".
    pub flight_frames: u32,
    /// Court speed (m/s) above which a free ball counts as flying.
    pub flight_speed: f32,
    pub coast_secs: f32,
    pub smooth_tau: f32,
}

impl Default for BallCfg {
    fn default() -> Self {
        Self { field: Field::basketball(), hold_expand: 0.15, hold_frames: 2, release_frames: 2, flight_frames: 2, flight_speed: 3.0, coast_secs: 0.7, smooth_tau: 0.06 }
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
    cand: Option<(u32, u32)>,
    release_count: u32,
    fast_count: u32,
    slow_since: Option<f64>,
    slow_count: u32,
}

fn contains(b: &BBox, x: f32, y: f32, expand: f32) -> bool {
    let (hw, hh) = (b.w * (0.5 + expand), b.h * (0.5 + expand));
    (x - b.cx).abs() <= hw && (y - b.cy).abs() <= hh
}

impl BallTracker {
    pub fn new(cfg: BallCfg) -> Self {
        Self { cfg, state: BallState::Unknown, pos: Vec2::default(), vel: Vec2::default(), have_pos: false, last_seen: f64::NEG_INFINITY, last_t: None, cand: None, release_count: 0, fast_count: 0, slow_since: None, slow_count: 0 }
    }

    /// Where the ball should be now (for gating false positives); `None` if not seen recently.
    pub fn reference(&self, t: f64) -> Option<Vec2> {
        if self.have_pos && (t - self.last_seen) as f32 <= self.cfg.coast_secs { Some(self.pos) } else { None }
    }

    /// Seconds since the ball was last accepted (infinite if never).
    pub fn age(&self, t: f64) -> f32 {
        if self.have_pos { (t - self.last_seen) as f32 } else { f32::INFINITY }
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
        self.pos = self.cfg.field.clamp(self.pos, 1.5);

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
                self.release_count = 0;
                self.cand = None;
            }
            (_, true, Some(id)) => {
                self.release_count = 0;
                self.fast_count = 0;
                match self.cand {
                    Some((cid, n)) if cid == id => {
                        if n + 1 >= self.cfg.hold_frames {
                            self.state = BallState::Held { track: id };
                            self.cand = None;
                        } else {
                            self.cand = Some((cid, n + 1));
                        }
                    }
                    _ => {
                        if self.cfg.hold_frames <= 1 {
                            self.state = BallState::Held { track: id };
                            self.cand = None;
                        } else {
                            self.cand = Some((id, 1));
                        }
                    }
                }
            }
            // ---- ball seen, but in nobody's box
            (BallState::Held { track }, true, None) => {
                self.cand = None;
                self.release_count += 1;
                if self.release_count >= self.cfg.release_frames {
                    let above = players
                        .iter()
                        .find(|p| p.id == track)
                        .and_then(|p| p.bbox)
                        .map(|b| obs.map(|o| o.bbox.cy < b.cy - b.h * 0.5).unwrap_or(false))
                        .unwrap_or(false);
                    self.state = if speed > self.cfg.flight_speed || above { BallState::InFlight } else { BallState::Loose };
                    self.release_count = 0;
                    self.slow_since = None;
                    self.slow_count = 0;
                }
            }
            (BallState::InFlight, true, None) => {
                self.cand = None;
                if speed < 1.0 {
                    self.slow_count += 1;
                    let since = *self.slow_since.get_or_insert(t);
                    if self.slow_count >= 3 && (t - since) >= 0.5 {
                        self.state = BallState::Loose;
                        self.slow_since = None;
                        self.slow_count = 0;
                    }
                } else {
                    self.slow_since = None;
                    self.slow_count = 0;
                }
            }
            (BallState::Loose, true, None) | (BallState::Unknown, true, None) => {
                self.cand = None;
                if speed > self.cfg.flight_speed {
                    self.fast_count += 1;
                } else {
                    self.fast_count = 0;
                }
                self.state = if self.fast_count >= self.cfg.flight_frames { BallState::InFlight } else { BallState::Loose };
            }
            // ---- ball not seen
            (BallState::Held { .. }, false, _) => {} // hidden in hands: stay
            (_, false, _) => {
                self.cand = None;
                self.fast_count = 0;
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
        let vel = match self.state {
            BallState::Held { track } => players.iter().find(|p| p.id == track).map(|p| p.velocity()),
            BallState::Unknown => None,
            _ => if self.have_pos { Some(self.vel) } else { None },
        };
        BallOut { state: self.state, pos, vel }
    }
}
