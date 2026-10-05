//! `reco-minimap`: top-down minimap from a Reco panorama.
//!
//! ```text
//! detections (panorama, normalized) -> Projector -> Tracker (+ BallTracker)
//!                                                    -> FrameState -> Renderer / JSON
//! ```
//! Model inference stays in `reco-detect` (RF-DETR via ORT/CoreML/TensorRT);
//! this crate decodes its output ([`detect::decode_rfdetr`]) and does
//! everything downstream. Pure Rust, no dependencies.

pub mod ball;
pub mod detect;
pub mod geom;
pub mod jersey;
pub mod render;
pub mod team;
pub mod tracker;
#[cfg(feature = "tools")]
pub mod tools;

use ball::{BallCfg, BallObs, BallOut, BallTracker};
use detect::{Class, Det};
use geom::{in_court, Projector};
use team::{Team, TeamClassifier};
use tracker::{BBox, Obs, Track, TrackState, Tracker, TrackerCfg};

#[derive(Clone, Copy, Debug)]
pub struct AgentOut {
    pub id: u32,
    pub referee: bool,
    pub team: Team,
    pub pos: geom::Vec2,
    pub state: TrackState,
    /// 1.0 = seen now; fades towards 0.35 while the track coasts.
    pub alpha: f32,
    pub jersey: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct FrameState {
    pub t: f64,
    pub players: Vec<AgentOut>,
    pub referees: Vec<AgentOut>,
    pub ball: BallOut,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub home_rgb: [u8; 3],
    pub away_rgb: [u8; 3],
    pub tracker: TrackerCfg,
    pub ball: BallCfg,
    /// Roster numbers (per team merged) accepted from OCR; empty = any.
    pub roster: Vec<u8>,
}

impl Config {
    pub fn new(home_rgb: [u8; 3], away_rgb: [u8; 3]) -> Self {
        Self { home_rgb, away_rgb, tracker: TrackerCfg::default(), ball: BallCfg::default(), roster: Vec::new() }
    }
}

pub struct Minimap {
    cfg: Config,
    projector: Box<dyn Projector>,
    classifier: TeamClassifier,
    tracker: Tracker,
    ball: BallTracker,
    last: Option<BallOut>,
}

impl Minimap {
    pub fn new(cfg: Config, projector: Box<dyn Projector>) -> Self {
        let classifier = TeamClassifier::new(cfg.home_rgb, cfg.away_rgb);
        let tracker = Tracker::new(cfg.tracker.clone());
        let ball = BallTracker::new(cfg.ball.clone());
        Self { cfg, projector, classifier, tracker, ball, last: None }
    }

    pub fn tracker(&self) -> &Tracker {
        &self.tracker
    }
    pub fn tracker_mut(&mut self) -> &mut Tracker {
        &mut self.tracker
    }

    /// Feed one frame of panorama detections (already NMS-merged across tiles).
    pub fn process(&mut self, t: f64, dets: &[Det]) -> FrameState {
        let mut obs: Vec<Obs> = Vec::new();
        let mut best_ball: Option<(&Det, geom::Vec2)> = None;
        for d in dets {
            let (u, v) = d.foot();
            let court = match self.projector.to_court(u, v) {
                Some(p) => p,
                None => continue,
            };
            match d.class {
                Class::Ball => {
                    if best_ball.map(|(b, _)| d.conf > b.conf).unwrap_or(true) {
                        best_ball = Some((d, court));
                    }
                }
                Class::Player | Class::Referee => {
                    let referee = d.class == Class::Referee;
                    let margin = if referee { self.cfg.tracker.ref_margin } else { self.cfg.tracker.player_margin };
                    if !in_court(court, margin) {
                        continue; // bench, coaches, spectators
                    }
                    obs.push(Obs {
                        referee,
                        team: if referee { Team::Unknown } else { self.classifier.classify(d.color) },
                        pos: court,
                        bbox: BBox { cx: d.cx, cy: d.cy, w: d.w, h: d.h },
                        conf: d.conf,
                    });
                }
            }
        }
        self.tracker.update(t, &obs);

        let ball_obs = best_ball.map(|(d, court)| BallObs { bbox: BBox { cx: d.cx, cy: d.cy, w: d.w, h: d.h }, court, conf: d.conf });
        let ball = self.ball.update(t, ball_obs, self.tracker.tracks());
        self.last = Some(ball);

        let (mut players, mut referees) = (Vec::new(), Vec::new());
        for tr in self.tracker.tracks() {
            if tr.hits < self.cfg.tracker.confirm_hits {
                continue;
            }
            let out = self.agent_out(tr, t);
            if tr.referee { referees.push(out) } else { players.push(out) }
        }
        FrameState { t, players, referees, ball }
    }

    fn agent_out(&self, tr: &Track, t: f64) -> AgentOut {
        let unseen = (t - tr.last_seen) as f32;
        let alpha = match tr.state {
            TrackState::Visible if unseen < self.cfg.tracker.coast_start => 1.0,
            TrackState::Lost => 0.35,
            _ => (1.0 - 0.65 * unseen / (self.cfg.tracker.lost_after + 1.0)).clamp(0.35, 1.0),
        };
        AgentOut {
            id: tr.id,
            referee: tr.referee,
            team: tr.team,
            pos: tr.pos,
            state: tr.state,
            alpha,
            jersey: tr.jersey.best(&self.cfg.roster, 3.0, 1.5),
        }
    }

    /// Phase 3: feed an OCR reading for a track.
    pub fn add_jersey_reading(&mut self, track: u32, number: u8, conf: f32) {
        if let Some(tr) = self.tracker.tracks_mut().iter_mut().find(|t| t.id == track) {
            tr.jersey.add(number, conf);
        }
    }
}
