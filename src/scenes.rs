//! Game situations ("scenes") recognised from tracked positions: restarts and set pieces whose
//! formation is easy to see - tip-off, free throw, throw-in / inbound, kick-off, corner, penalty,
//! 7-metre throw, and basketball shot attempts (2 / 3 points).
//!
//! Everything here is rule-based on the *tracked state* (positions in metres, speeds, ball state), so it
//! inherits the quality of detection and calibration. Dead-ball formations are recognised from where
//! people stand and that they stand still; they must hold for a moment before an event fires, and an
//! event fires once per formation. Shot attempts additionally need the ball: they fire when the ball
//! leaves a holder's hands towards a basket.
//!
//! Thresholds are first estimates validated on constructed scenes only - tune them on real footage.

use std::collections::{HashMap, HashSet};

use crate::ball::BallState;
use crate::geom::Vec2;
use crate::sport::SportProfile;
use crate::team::Team;
use crate::tracker::TrackState;
use crate::{AgentOut, FrameState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SceneKind {
    Kickoff,
    TipOff,
    FreeThrow,
    ThrowIn,
    Corner,
    Penalty,
    SevenMeter,
    ShotTwo,
    ShotThree,
}

impl SceneKind {
    pub fn key(self) -> &'static str {
        match self {
            SceneKind::Kickoff => "kickoff",
            SceneKind::TipOff => "tip_off",
            SceneKind::FreeThrow => "free_throw",
            SceneKind::ThrowIn => "throw_in",
            SceneKind::Corner => "corner",
            SceneKind::Penalty => "penalty",
            SceneKind::SevenMeter => "seven_meter",
            SceneKind::ShotTwo => "shot2",
            SceneKind::ShotThree => "shot3",
        }
    }

    /// Seconds the formation must hold before the event fires.
    fn hold(self) -> f64 {
        match self {
            SceneKind::TipOff => 1.0,
            SceneKind::ThrowIn => 1.2,
            SceneKind::Kickoff | SceneKind::Corner => 1.5,
            SceneKind::FreeThrow | SceneKind::Penalty | SceneKind::SevenMeter => 2.0,
            SceneKind::ShotTwo | SceneKind::ShotThree => 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SceneEvent {
    pub t: f64,
    pub kind: SceneKind,
    /// Team that takes the restart / shot (kicker, thrower, shooter).
    pub team: Option<Team>,
    pub pos: Vec2,
    /// `'l'` / `'r'` end of the field the scene belongs to, if any.
    pub side: Option<char>,
    pub confidence: f32,
    /// Shot attempts only: the release point is within the position error of the 3-point line.
    pub borderline: bool,
}

#[derive(Clone, Debug)]
pub struct SceneCfg {
    /// A player moving slower than this (m/s) counts as standing still.
    pub still_speed: f32,
    /// A formation may disappear this long (s) without resetting its hold timer.
    pub gap_tolerance: f64,
    /// Seconds without the formation before it can fire again.
    pub rearm_gap: f64,
    /// Minimum seconds between two events of the same kind and place.
    pub cooldown: f64,
}

impl Default for SceneCfg {
    fn default() -> Self {
        Self { still_speed: 0.9, gap_tolerance: 0.6, rearm_gap: 1.0, cooldown: 6.0 }
    }
}

struct Cand {
    kind: SceneKind,
    key: u8,
    team: Option<Team>,
    pos: Vec2,
    side: Option<char>,
    conf: f32,
}

struct Entry {
    since: f64,
    last_seen: f64,
    fired: bool,
}

struct PendingShot {
    t0: f64,
    release: Vec2,
    team: Team,
    goal: Vec2,
    start: f32,
    best: f32,
}

pub struct SceneDetector {
    profile: SportProfile,
    pub cfg: SceneCfg,
    active: HashMap<(SceneKind, u8), Entry>,
    last_fire: HashMap<(SceneKind, u8), f64>,
    holder: Option<(Vec2, Team, f64)>,
    prev_state: Option<BallState>,
    pending: Option<PendingShot>,
}

fn in_rect(p: Vec2, r: (f32, f32, f32, f32), pad: f32) -> bool {
    p.x >= r.0 - pad && p.x <= r.2 + pad && p.y >= r.1 - pad && p.y <= r.3 + pad
}

impl SceneDetector {
    pub fn new(profile: SportProfile) -> Self {
        Self { profile, cfg: SceneCfg::default(), active: HashMap::new(), last_fire: HashMap::new(), holder: None, prev_state: None, pending: None }
    }

    pub fn profile(&self) -> &SportProfile {
        &self.profile
    }

    /// Feed the next frame; returns the scenes that fired in it.
    pub fn update(&mut self, s: &FrameState) -> Vec<SceneEvent> {
        let mut out = Vec::new();
        let t = s.t;
        let players: Vec<&AgentOut> = s.players.iter().filter(|p| p.state != TrackState::Lost).collect();
        let mut cands: Vec<Cand> = Vec::new();
        if players.len() >= 4 {
            self.patterns(s, &players, &mut cands);
        }
        let mut present: HashSet<(SceneKind, u8)> = HashSet::new();
        for c in cands {
            let key = (c.kind, c.key);
            present.insert(key);
            let e = self.active.entry(key).or_insert(Entry { since: t, last_seen: t, fired: false });
            if t - e.last_seen > self.cfg.gap_tolerance {
                *e = Entry { since: t, last_seen: t, fired: false };
            }
            e.last_seen = t;
            let last = self.last_fire.get(&key).copied().unwrap_or(f64::NEG_INFINITY);
            if !e.fired && t - e.since >= c.kind.hold() && t - last >= self.cfg.cooldown {
                e.fired = true;
                self.last_fire.insert(key, t);
                out.push(SceneEvent { t, kind: c.kind, team: c.team, pos: c.pos, side: c.side, confidence: c.conf, borderline: false });
            }
        }
        let rearm = self.cfg.rearm_gap;
        self.active.retain(|k, e| present.contains(k) || t - e.last_seen <= rearm);
        self.shots(s, &mut out);
        out
    }

    fn still(&self, p: &AgentOut) -> bool {
        p.vel.len() < self.cfg.still_speed
    }

    fn patterns(&self, s: &FrameState, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let sport = self.profile.id.as_str();
        self.throw_ins(s, players, out);
        match sport {
            "basketball" => {
                self.tip_off(players, out, 1.5, 2.5, 6);
                self.free_throws(players, out);
            }
            "lacrosse" => self.tip_off(players, out, 1.2, 3.0, 4),
            _ => {}
        }
        if matches!(sport, "football" | "futsal" | "handball" | "hockey" | "rugby") {
            self.kickoff(players, out);
        }
        if matches!(sport, "football" | "futsal") {
            self.corners(players, out);
        }
        if matches!(sport, "football" | "futsal" | "handball" | "hockey") {
            self.penalties(players, out);
        }
    }

    /// Two opponents at the centre, everybody else standing around, nobody else close.
    fn tip_off(&self, players: &[&AgentOut], out: &mut Vec<Cand>, r_center: f32, r_pair: f32, min_others: usize) {
        let c = Vec2::new(self.profile.field.length / 2.0, self.profile.field.width / 2.0);
        let mid: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(c) <= r_center).collect();
        if mid.len() != 2 || mid[0].team == mid[1].team || mid.iter().any(|p| p.team == Team::Unknown) {
            return;
        }
        if mid[0].pos.dist(mid[1].pos) > r_pair || !mid.iter().all(|p| self.still(p)) {
            return;
        }
        let others: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(c) > r_center).collect();
        if others.len() < min_others || others.iter().any(|p| p.pos.dist(c) < r_center + 0.9) {
            return;
        }
        let calm = others.iter().filter(|p| p.vel.len() < self.cfg.still_speed * 1.6).count();
        if calm * 10 < others.len() * 7 {
            return;
        }
        out.push(Cand { kind: SceneKind::TipOff, key: 0, team: None, pos: c, side: None, conf: 0.7 });
    }

    /// One shooter at the free-throw line, several players lined up in the lane, dead ball.
    fn free_throws(&self, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let (l, w) = (self.profile.field.length, self.profile.field.width);
        for (end, spot, lane_x) in [('l', Vec2::new(5.8, w / 2.0), (-0.5f32, 6.1f32)), ('r', Vec2::new(l - 5.8, w / 2.0), (l - 6.1, l + 0.5))] {
            let shooters: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(spot) <= 1.1).collect();
            if shooters.len() != 1 || !self.still(shooters[0]) {
                continue;
            }
            let sh = shooters[0];
            let lane = players.iter().filter(|p| p.id != sh.id && p.pos.x >= lane_x.0 && p.pos.x <= lane_x.1 && (p.pos.y - w / 2.0).abs() <= 3.4).count();
            if lane < 3 {
                continue;
            }
            let moving = players.iter().filter(|p| p.vel.len() > self.cfg.still_speed * 1.6).count();
            if moving * 5 > players.len() {
                continue;
            }
            let conf = (0.6 + 0.05 * lane.min(6) as f32).min(0.9);
            out.push(Cand { kind: SceneKind::FreeThrow, key: end as u8, team: Some(sh.team), pos: sh.pos, side: Some(end), conf });
        }
    }

    /// A player standing still at the boundary, alone, with the game gathered around.
    fn throw_ins(&self, s: &FrameState, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let (l, w) = (self.profile.field.length, self.profile.field.width);
        let sport = self.profile.id.as_str();
        for p in players {
            if self.profile.field.edge_dist(p.pos) > 0.8 || !self.still(p) {
                continue;
            }
            if players.iter().filter(|q| q.id != p.id && q.pos.dist(p.pos) <= 9.0).count() < 3 {
                continue;
            }
            if players.iter().any(|q| q.id != p.id && q.pos.dist(p.pos) < 1.2) {
                continue;
            }
            let (x, y) = (p.pos.x, p.pos.y);
            let in_corner = (x <= 2.5 || x >= l - 2.5) && (y <= 2.5 || y >= w - 2.5);
            let on_side = y <= 0.8 || y >= w - 0.8;
            let on_end = x <= 0.8 || x >= l - 0.8;
            let ok = match sport {
                "basketball" => on_side || on_end,
                "football" | "futsal" => on_side && !in_corner,
                _ => on_side,
            };
            if !ok {
                continue;
            }
            let ball_with = matches!(s.ball.state, BallState::Held { track } if track == p.id) || s.ball.pos.map(|b| b.dist(p.pos) <= 1.5).unwrap_or(false);
            out.push(Cand { kind: SceneKind::ThrowIn, key: (p.id % 250) as u8, team: Some(p.team), pos: p.pos, side: None, conf: if ball_with { 0.8 } else { 0.5 } });
        }
    }

    /// Both teams on their own halves, standing still; only the kicking team's taker(s) at the centre.
    fn kickoff(&self, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let (l, w) = (self.profile.field.length, self.profile.field.width);
        if players.len() < 6.max((1.4 * self.profile.team_size as f32).ceil() as usize) {
            return;
        }
        let half = l / 2.0;
        let (home, away): (Vec<&&AgentOut>, Vec<&&AgentOut>) = {
            let h: Vec<_> = players.iter().filter(|p| p.team == Team::Home).collect();
            let a: Vec<_> = players.iter().filter(|p| p.team == Team::Away).collect();
            (h, a)
        };
        if home.len() < 2 || away.len() < 2 {
            return;
        }
        let frac = |v: &[&&AgentOut], left: bool| v.iter().filter(|p| if left { p.pos.x <= half + 1.5 } else { p.pos.x >= half - 1.5 }).count() as f32 / v.len() as f32;
        let split = (frac(&home, true) >= 0.85 && frac(&away, false) >= 0.85) || (frac(&home, false) >= 0.85 && frac(&away, true) >= 0.85);
        if !split {
            return;
        }
        let r_cc = match self.profile.id.as_str() {
            "football" => 9.15,
            "futsal" | "handball" => 3.0,
            "hockey" => 5.0,
            "rugby" => 10.0,
            _ => 4.0,
        };
        let c = Vec2::new(half, w / 2.0);
        let inside: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(c) <= r_cc).collect();
        if inside.is_empty() || inside.len() > 2 || (inside.len() == 2 && inside[0].team != inside[1].team) {
            return;
        }
        let calm = players.iter().filter(|p| p.vel.len() < self.cfg.still_speed * 1.6).count();
        if calm * 10 < players.len() * 8 {
            return;
        }
        out.push(Cand { kind: SceneKind::Kickoff, key: 0, team: Some(inside[0].team), pos: c, side: None, conf: 0.7 });
    }

    /// A taker standing still at a corner, several players waiting in the area next to it.
    fn corners(&self, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let (l, w) = (self.profile.field.length, self.profile.field.width);
        let min_box = if self.profile.id == "football" { 5 } else { 3 };
        for (i, (cx, cy, end)) in [(0.0f32, 0.0f32, 'l'), (0.0, w, 'l'), (l, 0.0, 'r'), (l, w, 'r')].into_iter().enumerate() {
            let corner = Vec2::new(cx, cy);
            let takers: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(corner) <= 2.4 && self.still(p)).collect();
            if takers.len() != 1 {
                continue;
            }
            let k = takers[0];
            let rect = match self.profile.zone_rect(end, 1) {
                Some(r) => r,
                None => continue,
            };
            let inbox: Vec<&&AgentOut> = players.iter().filter(|p| p.id != k.id && in_rect(p.pos, rect, 0.5)).collect();
            if inbox.len() < min_box {
                continue;
            }
            if inbox.iter().map(|p| p.vel.len()).sum::<f32>() / inbox.len() as f32 > 2.0 {
                continue;
            }
            let conf = (0.6 + 0.04 * inbox.len().min(8) as f32).min(0.9);
            out.push(Cand { kind: SceneKind::Corner, key: i as u8, team: Some(k.team), pos: k.pos, side: Some(end), conf });
        }
    }

    /// Penalty / 7 m: the taker at the spot, nobody else near, at most the goalkeeper in the area.
    fn penalties(&self, players: &[&AgentOut], out: &mut Vec<Cand>) {
        let (clear_r, spot_r, kind) = match self.profile.id.as_str() {
            "football" => (8.5f32, 1.8f32, SceneKind::Penalty),
            "futsal" | "hockey" => (4.5, 1.5, SceneKind::Penalty),
            "handball" => (1.9, 1.2, SceneKind::SevenMeter),
            _ => return,
        };
        if players.len() < 5.max((1.2 * self.profile.team_size as f32).ceil() as usize) {
            return;
        }
        for (end, key) in [('l', "spot_l"), ('r', "spot_r")] {
            let spot = match self.profile.landmark(key) {
                Some(s) => s,
                None => continue,
            };
            let takers: Vec<&&AgentOut> = players.iter().filter(|p| p.pos.dist(spot) <= spot_r && self.still(p)).collect();
            if takers.len() != 1 {
                continue;
            }
            let k = takers[0];
            if players.iter().any(|p| p.id != k.id && p.pos.dist(spot) < clear_r) {
                continue;
            }
            if let Some(z) = self.profile.zone_rect(end, 1) {
                if players.iter().filter(|p| p.id != k.id && in_rect(p.pos, z, 0.0)).count() > 1 {
                    continue; // only the goalkeeper may stand in the area
                }
            }
            out.push(Cand { kind, key: end as u8, team: Some(k.team), pos: k.pos, side: Some(end), conf: 0.75 });
        }
    }

    /// Basketball 2 / 3-point attempts: the ball leaves the holder's hands and travels towards a basket.
    fn shots(&mut self, s: &FrameState, out: &mut Vec<SceneEvent>) {
        if self.profile.id != "basketball" || self.profile.goals.len() < 2 {
            return;
        }
        let t = s.t;
        if let BallState::Held { track } = s.ball.state {
            if let Some(p) = s.players.iter().find(|p| p.id == track) {
                self.holder = Some((p.pos, p.team, t));
            }
        }
        let released = matches!(self.prev_state, Some(BallState::Held { .. })) && matches!(s.ball.state, BallState::InFlight);
        if released {
            if let Some((pos, team, th)) = self.holder {
                if t - th <= 1.5 {
                    let (goal, d) = self.profile.goals.iter().map(|g| (*g, pos.dist(*g))).fold((self.profile.goals[0], f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
                    if d <= 13.0 {
                        self.pending = Some(PendingShot { t0: t, release: pos, team, goal, start: d, best: d });
                    }
                }
            }
        }
        self.prev_state = Some(s.ball.state);
        let mut confirmed: Option<PendingShot> = None;
        if let Some(ps) = self.pending.as_mut() {
            if t - ps.t0 > 1.8 {
                self.pending = None;
            } else if let Some(bp) = s.ball.pos {
                ps.best = ps.best.min(bp.dist(ps.goal));
                if ps.best <= ps.start - 2.0 || ps.best <= 2.5 {
                    confirmed = self.pending.take();
                }
            }
        }
        if let Some(ps) = confirmed {
            let (l, w) = (self.profile.field.length, self.profile.field.width);
            // a free throw is reported as a free throw, not as a 2-point attempt
            let ft = [Vec2::new(5.8, w / 2.0), Vec2::new(l - 5.8, w / 2.0)];
            if ft.iter().any(|f| f.dist(ps.release) <= 1.3) {
                return;
            }
            let dist = ps.release.dist(ps.goal);
            let from_base = if ps.goal.x < l / 2.0 { ps.release.x } else { l - ps.release.x };
            let strip = (ps.release.y <= 0.9 || ps.release.y >= w - 0.9) && from_base <= 3.0;
            let three = dist >= 6.8 || strip;
            let borderline = (dist - 6.75).abs() < 0.8 || ((ps.release.y.min(w - ps.release.y) - 0.9).abs() < 0.6 && from_base <= 3.6);
            out.push(SceneEvent {
                t: ps.t0,
                kind: if three { SceneKind::ShotThree } else { SceneKind::ShotTwo },
                team: Some(ps.team),
                pos: ps.release,
                side: Some(if ps.goal.x < l / 2.0 { 'l' } else { 'r' }),
                confidence: if borderline { 0.45 } else { 0.6 },
                borderline,
            });
        }
    }
}
