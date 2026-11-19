//! Live: who scored? The board says *that* a team scored (+1 / +2 / +3), this module says *who*,
//! as soon as the board shows it.
//!
//! * A **buffer** keeps the last seconds of tracking (positions, teams, jersey numbers, scene events).
//! * The **board** is cleaned like afterwards ([`crate::scoreboard::timeline`]: scores only go up in
//!   steps of 1–3, misreads are outvoted), re-fitted every second over the last minutes of
//!   readings. A change is reported once it lies `lag` seconds back, so the readings after it can
//!   confirm it; if a later fit drops it again, it is taken back (`retracted`).
//! * The **shooter** is looked up in the buffer right away:
//!   * +1 is a free throw: the shooter stands alone at the free-throw line of the basket everybody
//!     is at, so it is the scoring team's player nearest that spot (all other players line up along
//!     the lane or stand behind the arc);
//!   * +2 / +3 is a field goal: a shot attempt of the scoring team seen by the scene detection,
//!     otherwise the scoring team's player who was nearest the basket in the seconds before.
//! * The shooter is a tracklet; its **jersey number** comes from the tracking. If it is not known
//!   yet, the event says so and a later update names the player as soon as the number has been
//!   read ([`LiveScorer::priority`] tells the number reader whom to read first).
//!
//! Every result is an estimate with a share of the buffered frames that agreed (`confidence`).

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};

use crate::geom::Vec2;
use crate::scoreboard::timeline::{self, TimelineCfg};
use crate::scoreboard::Reading;
use crate::team::Team;

#[derive(Clone, Debug)]
pub struct ScorerCfg {
    /// Seconds of tracking kept.
    pub buffer_s: f64,
    /// A change is reported when it lies this far back (s): the readings after it confirm it.
    pub lag: f64,
    /// Seconds of board readings fitted each time.
    pub board_window: f64,
    /// Seconds between two fits.
    pub refit_every: f64,
    /// Seconds of readings before the starting score is taken.
    pub warmup: f64,
    /// Field goal: frames between `t - fg_window.0` and `t - fg_window.1` are searched.
    pub fg_window: (f64, f64),
    /// Free throw: frames between `t - ft_window.0` and `t - ft_window.1` are searched.
    pub ft_window: (f64, f64),
    /// Distance of the free-throw spot from the basket (m).
    pub ft_offset: f32,
    /// A free-throw shooter stands within this distance of the spot (m).
    pub ft_radius: f32,
    /// How long an event waits for the shooter's number (s).
    pub number_wait: f64,
    /// How long the candidates are read first after an event (s).
    pub priority_s: f64,
}

impl Default for ScorerCfg {
    fn default() -> Self {
        Self { buffer_s: 30.0, lag: 3.0, board_window: 600.0, refit_every: 1.0, warmup: 15.0, fg_window: (8.0, 0.5), ft_window: (10.0, 0.5), ft_offset: 4.225, ft_radius: 2.5, number_wait: 90.0, priority_s: 20.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub id: u32,
    pub segment: u32,
    pub team: Team,
    pub pos: Vec2,
    pub number: Option<u32>,
    /// The number was read on this tracklet (not only deduced).
    pub read: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attempt {
    pub actor: u32,
    pub team: Team,
    pub points: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub t: f64,
    pub players: Vec<Player>,
    pub attempts: Vec<Attempt>,
    /// Tracklet id holding the ball (ball tracking), if any.
    pub holder: Option<u32>,
}

fn team_of(s: &str) -> Team {
    match s {
        "home" => Team::Home,
        "away" => Team::Away,
        _ => Team::Unknown,
    }
}

fn team_key(t: Team) -> &'static str {
    match t {
        Team::Home => "home",
        Team::Away => "away",
        Team::Unknown => "unknown",
    }
}

impl Snapshot {
    /// From one frame of a result (`run --json` / the live state): players with position, team and
    /// identity, and the shot attempts among the scene events.
    pub fn from_frame(v: &Value) -> Option<Self> {
        let t = v["t"].as_f64()?;
        let players = v["players"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let id = p["id"].as_u64()? as u32;
                let pos = Vec2::new(p["x"].as_f64()? as f32, p["y"].as_f64()? as f32);
                let ident = &p["identity"];
                let number = ident["player"]["number"].as_u64().map(|n| n as u32).or_else(|| p["jersey"].as_u64().map(|n| n as u32));
                let status = ident["status"].as_str().unwrap_or("");
                Some(Player { id, segment: p["segment"].as_u64().unwrap_or(0) as u32, team: team_of(p["team"].as_str().unwrap_or("")), pos, number, read: matches!(status, "confirmed" | "read") })
            })
            .collect();
        let attempts = v["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["type"] == "scene")
            .filter_map(|e| {
                let points = match e["kind"].as_str()? {
                    "shot2" => 2,
                    "shot3" => 3,
                    _ => return None,
                };
                Some(Attempt { actor: e["actor"].as_u64()? as u32, team: team_of(e["team"].as_str().unwrap_or("")), points })
            })
            .collect();
        let holder = v["ball"]["holder"].as_u64().map(|h| h as u32);
        Some(Snapshot { t, players, attempts, holder })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreKind {
    FreeThrow,
    FieldGoal,
    /// A step down on the board: an operator correction.
    Correction,
    /// More than 3 points at once: the board skipped updates.
    Jump,
}

impl ScoreKind {
    pub fn key(self) -> &'static str {
        match self {
            ScoreKind::FreeThrow => "free_throw",
            ScoreKind::FieldGoal => "field_goal",
            ScoreKind::Correction => "correction",
            ScoreKind::Jump => "jump",
        }
    }
}

/// How the shooter was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Alone at the free-throw line.
    FreeThrowLine,
    /// The scene detection saw the shot.
    Attempt,
    /// The scoring team's last player with the ball before the score.
    LastHolder,
    /// Nearest the basket before the score.
    NearBasket,
    /// Nobody of the scoring team in the buffer.
    TeamOnly,
}

impl Basis {
    pub fn key(self) -> &'static str {
        match self {
            Basis::FreeThrowLine => "free_throw_line",
            Basis::Attempt => "attempt",
            Basis::LastHolder => "ball_holder",
            Basis::NearBasket => "near_basket",
            Basis::TeamOnly => "team_only",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScoreEvent {
    /// Same id for an event and its later updates.
    pub sid: u32,
    /// Video time the board first showed the new score.
    pub t: f64,
    pub team: Team,
    pub points: i32,
    pub from: u32,
    pub to: u32,
    pub kind: ScoreKind,
    pub basis: Basis,
    /// Shooter tracklet (`id`, `segment`).
    pub shooter: Option<(u32, u32)>,
    pub number: Option<u32>,
    pub read: bool,
    /// Share of the searched frames that agreed on the shooter (0..1).
    pub confidence: f32,
    /// Board clock when it changed (s left).
    pub clock: Option<u32>,
    /// Waiting for the shooter's number.
    pub pending: bool,
    /// This is an update of an earlier event with the same `sid`.
    pub update: bool,
    /// A later fit of the board dropped this change: it did not happen.
    pub retracted: bool,
    /// Other candidates with their share.
    pub others: Vec<((u32, u32), f32)>,
}

impl ScoreEvent {
    pub fn to_json(&self) -> Value {
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        json!({"type": "score", "sid": self.sid, "t": r(self.t), "team": team_key(self.team), "points": self.points, "from": self.from, "to": self.to,
            "kind": self.kind.key(), "basis": self.basis.key(), "number": self.number, "read": self.read, "pending": self.pending, "update": self.update, "retracted": self.retracted,
            "confidence": (self.confidence * 100.0).round() / 100.0, "clock": self.clock,
            "shooter": self.shooter.map(|(id, seg)| json!({"track": id, "segment": seg})),
            "others": self.others.iter().map(|((id, seg), s)| json!({"track": id, "segment": seg, "share": (s * 100.0).round() / 100.0})).collect::<Vec<_>>()})
    }
}

pub struct LiveScorer {
    cfg: ScorerCfg,
    baskets: [Vec2; 2],
    buf: VecDeque<Snapshot>,
    raw: VecDeque<Reading>,
    last_fit: f64,
    /// Reported changes still open to a retraction: (team, score after) -> event.
    reported: Vec<ScoreEvent>,
    /// Score per team as last reported.
    current: [Option<u32>; 2],
    clocks: VecDeque<(f64, u32)>,
    next_sid: u32,
    /// Events still waiting for a number.
    waiting: Vec<ScoreEvent>,
    /// Tracklets to read first, until a time.
    prio: HashMap<(u32, u32), f64>,
    /// Latest number per tracklet (from the tracking).
    numbers: HashMap<(u32, u32), (u32, bool)>,
}

impl LiveScorer {
    /// `baskets`: the two basket centres in field coordinates (`SportProfile::goals`).
    pub fn new(baskets: [Vec2; 2], cfg: ScorerCfg) -> Self {
        Self { cfg, baskets, buf: VecDeque::new(), raw: VecDeque::new(), last_fit: f64::NEG_INFINITY, reported: Vec::new(), current: [None, None], clocks: VecDeque::new(), next_sid: 0, waiting: Vec::new(), prio: HashMap::new(), numbers: HashMap::new() }
    }

    /// The current score as the filter accepted it.
    pub fn score(&self) -> (Option<u32>, Option<u32>) {
        (self.current[0], self.current[1])
    }

    /// One tracked frame. Returns updates of earlier events whose shooter's number is now known.
    pub fn push_frame(&mut self, s: Snapshot) -> Vec<ScoreEvent> {
        let t = s.t;
        for p in &s.players {
            if let Some(n) = p.number {
                self.numbers.insert((p.id, p.segment), (n, p.read));
            }
        }
        self.buf.push_back(s);
        while self.buf.front().is_some_and(|f| f.t < t - self.cfg.buffer_s) {
            self.buf.pop_front();
        }
        self.prio.retain(|_, until| *until >= t);
        let mut out = Vec::new();
        let (cfg_wait, numbers) = (self.cfg.number_wait, &self.numbers);
        self.waiting.retain_mut(|e| {
            let Some(key) = e.shooter else { return false };
            if let Some(&(n, read)) = numbers.get(&key) {
                e.number = Some(n);
                e.read = read;
                e.pending = false;
                e.update = true;
                out.push(e.clone());
                return false;
            }
            t - e.t < cfg_wait
        });
        for e in &self.waiting {
            if let Some(k) = e.shooter {
                self.prio.insert(k, t + 1.0);
            }
        }
        out
    }

    /// Tracklets whose jersey number should be read first now (shooters and candidates of recent
    /// scores without a known number).
    pub fn priority(&self) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = self.prio.keys().copied().filter(|k| !self.numbers.contains_key(k)).collect();
        v.sort_unstable();
        v
    }

    /// One raw board reading. Returns the scores it confirms (and retractions of earlier ones).
    pub fn push_reading(&mut self, r: &Reading) -> Vec<ScoreEvent> {
        if let Some(c) = r.clock {
            self.clocks.push_back((r.t, c));
            while self.clocks.front().is_some_and(|f| f.0 < r.t - 60.0) {
                self.clocks.pop_front();
            }
        }
        self.raw.push_back(*r);
        while self.raw.front().is_some_and(|f| f.t < r.t - self.cfg.board_window) {
            self.raw.pop_front();
        }
        if r.t - self.last_fit < self.cfg.refit_every {
            return Vec::new();
        }
        self.last_fit = r.t;
        self.refit(r.t)
    }

    fn refit(&mut self, now: f64) -> Vec<ScoreEvent> {
        let raw: Vec<Reading> = self.raw.iter().copied().collect();
        let tl = timeline::resolve(&raw, &TimelineCfg::default());
        let settled = now - self.cfg.lag;
        let mut out = Vec::new();
        // reported changes the fit no longer has (only recent ones: older ones are history)
        let has = |team: Team, to: u32| tl.changes.iter().any(|c| (c.home == (team == Team::Home)) && c.to == to);
        let fit_start = raw.first().map(|r| r.t).unwrap_or(now);
        let mut keep = Vec::new();
        for e in std::mem::take(&mut self.reported) {
            if e.t < now - 120.0 || e.t < fit_start + 30.0 {
                continue;
            }
            if has(e.team, e.to) {
                keep.push(e);
            } else {
                let mut x = e.clone();
                x.update = true;
                x.retracted = true;
                self.waiting.retain(|w| w.sid != x.sid);
                let i = if x.team == Team::Home { 0 } else { 1 };
                self.current[i] = Some(x.from);
                out.push(x);
            }
        }
        self.reported = keep;
        // the fitted score at the settled time per team: report the steps up to it
        for (i, team) in [Team::Home, Team::Away].into_iter().enumerate() {
            let home = team == Team::Home;
            let state = tl.states.iter().rev().find(|s| s.t <= settled).and_then(|s| if home { s.home } else { s.away });
            let Some(state) = state else { continue };
            let Some(cur) = self.current[i] else {
                // the first settled score is the start, not a change (after enough readings)
                if settled - fit_start >= self.cfg.warmup {
                    self.current[i] = Some(state);
                }
                continue;
            };
            if state == cur {
                continue;
            }
            let steps: Vec<_> = tl.changes.iter().filter(|c| c.home == home && c.t <= settled && c.from >= cur && c.to <= state && c.to > cur).cloned().collect();
            if steps.is_empty() || state < cur {
                // a step down or a jump the fit cannot explain step by step: one change
                let t = tl.changes.iter().rev().find(|c| c.home == home && c.t <= settled).map(|c| c.t).unwrap_or(settled);
                let e = self.attribute(team, cur, state, t);
                self.reported.push(e.clone());
                out.push(e);
            } else {
                for c in steps {
                    let e = self.attribute(team, c.from, c.to, c.t);
                    if e.pending {
                        self.waiting.push(e.clone());
                    }
                    self.reported.push(e.clone());
                    out.push(e);
                }
            }
            self.current[i] = Some(state);
        }
        out
    }

    fn clock_at(&self, t: f64) -> Option<u32> {
        self.clocks.iter().rev().find(|c| c.0 <= t + 0.01).or(self.clocks.back()).map(|c| c.1)
    }

    fn attribute(&mut self, team: Team, from: u32, to: u32, t: f64) -> ScoreEvent {
        let points = to as i32 - from as i32;
        let kind = match points {
            1 => ScoreKind::FreeThrow,
            2 | 3 => ScoreKind::FieldGoal,
            p if p < 0 => ScoreKind::Correction,
            _ => ScoreKind::Jump,
        };
        self.next_sid += 1;
        let mut e = ScoreEvent { sid: self.next_sid, t, team, points, from, to, kind, basis: Basis::TeamOnly, shooter: None, number: None, read: false, confidence: 0.0, clock: self.clock_at(t), pending: false, update: false, retracted: false, others: Vec::new() };
        let (w0, w1) = match kind {
            ScoreKind::FreeThrow => self.cfg.ft_window,
            ScoreKind::FieldGoal => self.cfg.fg_window,
            _ => return e,
        };
        let frames: Vec<&Snapshot> = self.buf.iter().filter(|f| f.t >= t - w0 && f.t <= t - w1).collect();
        let Some((basis, ranked)) = pick_shooter(&frames, self.baskets, team, kind, points, t, (w0, w1), &self.cfg) else { return e };
        let until = t + self.cfg.priority_s;
        for (k, _) in ranked.iter().take(3) {
            self.prio.insert(*k, until);
        }
        if let Some(&(k, share)) = ranked.first() {
            e.basis = basis;
            e.shooter = Some(k);
            e.confidence = share;
            match self.numbers.get(&k) {
                Some(&(n, read)) => {
                    e.number = Some(n);
                    e.read = read;
                }
                None => e.pending = true,
            }
            e.others = ranked[1..].iter().take(2).copied().collect();
        }
        e
    }
}

/// Inside the court (with a small margin): people at the bench or the table are no shooters.
fn on_court(baskets: [Vec2; 2], p: Vec2) -> bool {
    // field coordinates start at 0; the baskets stand as far from both end lines
    let (b0, b1) = (baskets[0].x.min(baskets[1].x), baskets[0].x.max(baskets[1].x));
    let (len, wid) = (b1 + b0, 2.0 * baskets[0].y);
    p.x >= 0.2 && p.x <= len - 0.2 && p.y >= 0.3 && p.y <= wid - 0.3
}

/// Who scored, from the frames before a score at `t` (the window `t - w.0 ..= t - w.1`): the
/// candidates of the scoring team with their share (best first) and how they were found.
#[allow(clippy::too_many_arguments)]
pub fn pick_shooter(frames: &[&Snapshot], baskets: [Vec2; 2], team: Team, kind: ScoreKind, points: i32, t: f64, w: (f64, f64), cfg: &ScorerCfg) -> Option<(Basis, Vec<((u32, u32), f32)>)> {
    let (w0, w1) = w;
    let on_court = |p: Vec2| on_court(baskets, p);
    // the basket everybody is at: the one nearest the players' centre
    let (sx, n) = frames.iter().flat_map(|f| f.players.iter()).fold((0.0f32, 0usize), |a, p| (a.0 + p.pos.x, a.1 + 1));
    if n == 0 {
        return None;
    }
    let cx = sx / n as f32;
    let basket = *baskets.iter().min_by(|a, b| (a.x - cx).abs().total_cmp(&(b.x - cx).abs())).unwrap();
    let mut votes: HashMap<(u32, u32), f32> = HashMap::new();
    let mut total = 0.0f32;
    let mut basis = Basis::NearBasket;
    if kind == ScoreKind::FreeThrow {
        // the free-throw spot lies from the basket towards the middle of the court
        let mid = (baskets[0].x + baskets[1].x) / 2.0;
        let dir = if basket.x < mid { 1.0 } else { -1.0 };
        let spot = Vec2::new(basket.x + dir * cfg.ft_offset, basket.y);
        basis = Basis::FreeThrowLine;
        for f in frames {
            let best = f.players.iter().filter(|p| p.team == team && on_court(p.pos)).map(|p| (p, p.pos.dist(spot))).min_by(|a, b| a.1.total_cmp(&b.1));
            total += 1.0;
            if let Some((p, _)) = best.filter(|b| b.1 <= cfg.ft_radius) {
                *votes.entry((p.id, p.segment)).or_default() += 1.0;
            }
        }
    } else {
        // an attempt of the scoring team that fits the points wins outright (latest one)
        let att = frames.iter().rev().flat_map(|f| f.attempts.iter().map(move |a| (f, a))).find(|(_, a)| a.team == team && (a.points as i32 == points || points == 3));
        if let Some((f, a)) = att {
            if let Some(p) = f.players.iter().find(|p| p.id == a.actor) {
                votes.insert((p.id, p.segment), 1.0);
                total = 1.0;
                basis = Basis::Attempt;
            }
        }
        if votes.is_empty() {
            // the scoring team's last player with the ball, near enough to the basket for a shot
            let held = frames.iter().rev().find_map(|f| {
                let h = f.holder?;
                f.players.iter().find(|p| p.id == h && p.team == team && on_court(p.pos) && p.pos.dist(basket) <= 9.0)
            });
            if let Some(p) = held {
                votes.insert((p.id, p.segment), 1.0);
                total = 1.0;
                basis = Basis::LastHolder;
            }
        }
        if votes.is_empty() {
            // later frames count more: right before the score the shooter is at the basket
            let span = (w0 - w1).max(1e-6);
            for f in frames {
                let wt = (0.5 + (f.t - (t - w0)) / span) as f32;
                let best = f.players.iter().filter(|p| p.team == team && on_court(p.pos)).min_by(|a, b| a.pos.dist(basket).total_cmp(&b.pos.dist(basket)));
                total += wt;
                if let Some(p) = best {
                    *votes.entry((p.id, p.segment)).or_default() += wt;
                }
            }
        }
    }
    let mut ranked: Vec<((u32, u32), f32)> = votes.into_iter().map(|(k, v)| (k, v / total.max(1e-6))).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    Some((basis, ranked))
}

/// The final state of every score (an update replaces its event; taken back ones are dropped).
pub fn final_events(events: &[ScoreEvent]) -> Vec<ScoreEvent> {
    let mut last: std::collections::BTreeMap<u32, ScoreEvent> = Default::default();
    for e in events {
        last.insert(e.sid, e.clone());
    }
    last.into_values().filter(|e| !e.retracted).collect()
}

/// Replays a result as if live: its frames and the raw board readings in time order through a
/// [`LiveScorer`]. Returns every event with the video time it was reported at.
pub fn replay(frames: &[Value], readings: &[Reading], baskets: [Vec2; 2], cfg: ScorerCfg) -> Vec<(f64, ScoreEvent)> {
    let mut readings = readings.to_vec();
    readings.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut sc = LiveScorer::new(baskets, cfg);
    let mut out = Vec::new();
    let mut ri = 0;
    for f in frames {
        let Some(t) = f["t"].as_f64() else { continue };
        while ri < readings.len() && readings[ri].t <= t {
            out.extend(sc.push_reading(&readings[ri]).into_iter().map(|e| (t, e)));
            ri += 1;
        }
        if let Some(s) = Snapshot::from_frame(f) {
            out.extend(sc.push_frame(s).into_iter().map(|e| (t, e)));
        }
    }
    out
}

/// A score event back from its JSON (`type: "score"`, as [`ScoreEvent::to_json`] writes it).
pub fn parse_event(v: &Value) -> Option<ScoreEvent> {
    if v["type"] != "score" {
        return None;
    }
    let kind = match v["kind"].as_str()? {
        "free_throw" => ScoreKind::FreeThrow,
        "field_goal" => ScoreKind::FieldGoal,
        "correction" => ScoreKind::Correction,
        _ => ScoreKind::Jump,
    };
    let basis = match v["basis"].as_str().unwrap_or("") {
        "free_throw_line" => Basis::FreeThrowLine,
        "attempt" => Basis::Attempt,
        "ball_holder" => Basis::LastHolder,
        "near_basket" => Basis::NearBasket,
        _ => Basis::TeamOnly,
    };
    let shooter = v["shooter"]["track"].as_u64().map(|id| (id as u32, v["shooter"]["segment"].as_u64().unwrap_or(0) as u32));
    let b = |k: &str| v[k].as_bool().unwrap_or(false);
    Some(ScoreEvent {
        sid: v["sid"].as_u64()? as u32, t: v["t"].as_f64()?, team: team_of(v["team"].as_str().unwrap_or("")), points: v["points"].as_i64()? as i32,
        from: v["from"].as_u64().unwrap_or(0) as u32, to: v["to"].as_u64().unwrap_or(0) as u32, kind, basis, shooter,
        number: v["number"].as_u64().map(|n| n as u32), read: b("read"), confidence: v["confidence"].as_f64().unwrap_or(0.0) as f32,
        clock: v["clock"].as_u64().map(|c| c as u32), pending: b("pending"), update: b("update"), retracted: b("retracted"),
        others: v["others"].as_array().into_iter().flatten().filter_map(|o| Some(((o["track"].as_u64()? as u32, o["segment"].as_u64().unwrap_or(0) as u32), o["share"].as_f64().unwrap_or(0.0) as f32))).collect(),
    })
}

/// The scores of a result: the live events it carries, otherwise a replay over its board readings.
pub fn scores_of(frames: &[Value], readings: &[Reading], baskets: [Vec2; 2]) -> Vec<ScoreEvent> {
    let carried: Vec<ScoreEvent> = frames.iter().flat_map(|f| f["events"].as_array().into_iter().flatten()).filter_map(parse_event).collect();
    if !carried.is_empty() {
        return final_events(&carried);
    }
    let ev: Vec<ScoreEvent> = replay(frames, readings, baskets, ScorerCfg::default()).into_iter().map(|x| x.1).collect();
    final_events(&ev)
}

/// The scorers of a match report (`reco-minimap-reference-events-v1`): (team, that team's score
/// after the points) -> (jersey number, points). Only made shots count.
pub fn reference_scorers(v: &Value) -> HashMap<(Team, u32), (Option<u32>, u32)> {
    let mut out = HashMap::new();
    for e in v["events"].as_array().into_iter().flatten() {
        let Some(a) = e.as_array().filter(|a| a.len() >= 7) else { continue };
        let points = match a[4].as_str().unwrap_or("") {
            "2pt" => 2,
            "3pt" => 3,
            "ft_made" => 1,
            _ => continue,
        };
        let team = team_of(a[2].as_str().unwrap_or(""));
        let score = if team == Team::Home { a[5].as_u64() } else { a[6].as_u64() };
        if let Some(s) = score {
            out.insert((team, s as u32), (a[3].as_u64().map(|n| n as u32), points));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(t: f64, h: u32, a: u32, clock: u32) -> Reading {
        Reading { t, home: Some(h), away: Some(a), clock: Some(clock), period: None }
    }

    fn pl(id: u32, team: Team, x: f32, y: f32, number: Option<u32>) -> Player {
        Player { id, segment: 0, team, pos: Vec2::new(x, y), number, read: number.is_some() }
    }

    fn scorer() -> LiveScorer {
        LiveScorer::new([Vec2::new(1.575, 7.5), Vec2::new(26.425, 7.5)], ScorerCfg { warmup: 2.0, ..Default::default() })
    }

    #[test]
    fn one_misread_is_not_a_score() {
        let mut s = scorer();
        let mut ev = Vec::new();
        let seq: Vec<u32> = [10; 10].into_iter().chain([17, 10]).chain([12; 20]).collect();
        for (i, h) in seq.iter().enumerate() {
            ev.extend(s.push_reading(&rd(i as f64 * 0.5, *h, 8, 300)));
        }
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert_eq!((ev[0].from, ev[0].to, ev[0].kind), (10, 12, ScoreKind::FieldGoal));
        assert!((ev[0].t - 6.0).abs() < 1.01, "{}", ev[0].t);
    }

    #[test]
    fn free_throw_shooter_is_alone_at_the_line() {
        let mut s = scorer();
        for i in 0..20 {
            let t = i as f64 * 0.5;
            // home attacks the right basket; #13 at the free-throw spot, others on the lane
            s.push_frame(Snapshot { t, players: vec![pl(1, Team::Home, 22.2, 7.6, Some(13)), pl(2, Team::Home, 24.0, 5.0, Some(4)), pl(3, Team::Away, 24.0, 10.0, None), pl(4, Team::Away, 25.0, 5.0, None)], attempts: vec![], holder: None });
        }
        let mut ev = Vec::new();
        for i in 0..24 {
            ev.extend(s.push_reading(&rd(5.0 + i as f64 * 0.5, if i < 10 { 20 } else { 21 }, 8, 300)));
        }
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].kind, ev[0].basis, ev[0].number), (ScoreKind::FreeThrow, Basis::FreeThrowLine, Some(13)));
    }

    #[test]
    fn field_goal_waits_for_the_number() {
        let mut s = scorer();
        for i in 0..20 {
            s.push_frame(Snapshot { t: i as f64 * 0.5, players: vec![pl(7, Team::Away, 3.0, 7.0, None), pl(8, Team::Away, 10.0, 3.0, Some(5)), pl(9, Team::Home, 5.0, 7.0, Some(4))], attempts: vec![], holder: None });
        }
        let mut ev = Vec::new();
        for i in 0..24 {
            ev.extend(s.push_reading(&rd(9.0 + i as f64 * 0.5, 30, if i < 10 { 10 } else { 12 }, 200)));
        }
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].team, ev[0].shooter, ev[0].pending), (Team::Away, Some((7, 0)), true));
        assert_eq!(s.priority(), vec![(7, 0)]);
        let up = s.push_frame(Snapshot { t: 14.0, players: vec![pl(7, Team::Away, 3.0, 7.0, Some(24))], attempts: vec![], holder: None });
        assert_eq!(up.len(), 1);
        assert_eq!((up[0].sid, up[0].number, up[0].update), (ev[0].sid, Some(24), true));
        // the JSON round trip keeps what the statistics need
        let back = parse_event(&up[0].to_json()).unwrap();
        assert_eq!((back.sid, back.team, back.points, back.shooter, back.number, back.kind), (up[0].sid, Team::Away, 2, Some((7, 0)), Some(24), ScoreKind::FieldGoal));
        assert_eq!(final_events(&[ev[0].clone(), up[0].clone()]).len(), 1);
    }
}
