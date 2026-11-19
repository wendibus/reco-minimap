//! Statistics per player and the (estimated) score.
//!
//! **Playing time per player:** a tracking slot is not a person, so time is counted per continuous
//! tracklet (`id` + `segment`) and each tracklet goes to the jersey number read most often on it.
//! Tracklets without a number stay "without number" per team. Needs jersey numbers (step 4).
//!
//! **Score:** the tracking does not see the ball go in. What it can see is how play restarts after
//! a score:
//! * goal sports (football, futsal, handball, hockey, lacrosse): a kick-off / throw-off / face-off
//!   shortly after an attempt; the attempting team scored. Kick-offs without such an attempt are
//!   period starts (or goals the tracking missed) and are listed as "possible" when the playing
//!   direction says who kicked off.
//! * basketball: after a made field goal the other team inbounds from the end line under that
//!   basket within a few seconds. Points 2 or 3 by the attempt; made free throws are not detected.
//!
//! Both are estimates and are labelled as such; every scoring event carries its evidence so it can
//! be checked in a clip.
//!
//! **Basketball with a scoreboard** (`run --scoreboard`): the score is read from the board instead.
//! Every +1/+2/+3 is matched to the fitting attempt before it ([`crate::scoring::attribute`]); the
//! shooter gets the points with a probability, marked as read or inferred by how their jersey number
//! was known. Without a fitting attempt the points go to the team only.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::stats::Attempt;
use crate::direction::Direction;
use crate::scoring::{attribute, score_changes, AttributionCfg, Basis, NumberStatus, ShotAttempt, ShotKind, Shooter};
use crate::sport::SportProfile;
use crate::team::Team;

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[derive(Default)]
struct Tracklet {
    secs: f64,
    team: HashMap<String, f64>,
    numbers: HashMap<u64, f64>,
    /// Seconds whose number was deduced rather than read.
    inferred: f64,
}

/// Every continuous tracklet (`id`, `segment`) with its time, team and numbers.
fn tracklets(frames: &[Value]) -> HashMap<(u64, u64), Tracklet> {
    let mut tl: HashMap<(u64, u64), Tracklet> = HashMap::new();
    for (i, fr) in frames.iter().enumerate() {
        let t = fr["t"].as_f64().unwrap_or(0.0);
        let dt = frames.get(i + 1).and_then(|n| n["t"].as_f64()).map(|n| (n - t).clamp(0.0, 1.0)).unwrap_or(0.0);
        if dt <= 0.0 {
            continue;
        }
        for p in fr["players"].as_array().into_iter().flatten() {
            if p["state"].as_str() == Some("lost") {
                continue;
            }
            let key = (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0));
            let e = tl.entry(key).or_default();
            e.secs += dt;
            if let Some(team) = p["team"].as_str() {
                *e.team.entry(team.to_string()).or_default() += dt;
            }
            let status = p["identity"]["status"].as_str().unwrap_or("");
            let number = p["identity"]["player"]["number"].as_u64().filter(|_| matches!(status, "candidate" | "inferred" | "manual")).or_else(|| p["jersey"].as_u64());
            if let Some(n) = number {
                *e.numbers.entry(n).or_default() += dt;
                if status == "inferred" {
                    e.inferred += dt;
                }
            }
        }
    }
    tl
}

/// Playing time per team and jersey number. Every tracklet stands for the number seen most on it;
/// per frame a number counts once (two trails with the same number at the same time are one
/// player, not twice the time).
pub fn players(frames: &[Value]) -> Value {
    let tl = tracklets(frames);
    let best: HashMap<(u64, u64), (Option<String>, Option<u64>)> = tl.iter().map(|(k, e)| (*k, tracklet_number(e))).collect();
    let mut per: HashMap<(String, u64), f64> = HashMap::new();
    let mut deduced: HashMap<(String, u64), f64> = HashMap::new();
    let mut without: HashMap<String, f64> = HashMap::new();
    let deduced_share: HashMap<(u64, u64), f64> = tl.iter().map(|(k, e)| (*k, e.inferred / e.numbers.values().sum::<f64>().max(1e-6))).collect();
    for (i, fr) in frames.iter().enumerate() {
        let t = fr["t"].as_f64().unwrap_or(0.0);
        let dt = frames.get(i + 1).and_then(|n| n["t"].as_f64()).map(|n| (n - t).clamp(0.0, 1.0)).unwrap_or(0.0);
        if dt <= 0.0 {
            continue;
        }
        let mut here: HashMap<(String, u64), f64> = HashMap::new();
        for p in fr["players"].as_array().into_iter().flatten() {
            if p["state"].as_str() == Some("lost") {
                continue;
            }
            let key = (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0));
            match best.get(&key) {
                Some((Some(team), Some(n))) => {
                    let d = deduced_share.get(&key).copied().unwrap_or(0.0);
                    let e = here.entry((team.clone(), *n)).or_insert(1.0);
                    *e = e.min(d);
                }
                Some((Some(team), None)) => *without.entry(team.clone()).or_default() += dt,
                _ => {}
            }
        }
        for (k, d) in here {
            *per.entry(k.clone()).or_default() += dt;
            *deduced.entry(k).or_default() += dt * d;
        }
    }
    let t0 = frames.first().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let t1 = frames.last().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let span = (t1 - t0).max(1e-6);
    let list = |team: &str| {
        let mut v: Vec<(u64, f64)> = per.iter().filter(|((tm, _), _)| tm == team).map(|((_, n), s)| (*n, *s)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter()
            .map(|(n, s)| {
                let inf = deduced.get(&(team.to_string(), n)).copied().unwrap_or(0.0);
                json!({"number": n, "seconds": r1(s), "share": r1(s / span * 100.0) / 100.0, "inferred_share": (inf / s.max(1e-6) * 100.0).round() / 100.0})
            })
            .collect::<Vec<_>>()
    };
    json!({
        "home": list("home"), "away": list("away"),
        "without_number": {"home": r1(without.get("home").copied().unwrap_or(0.0)), "away": r1(without.get("away").copied().unwrap_or(0.0))},
        "numbers_seen": !per.is_empty(),
    })
}

/// The tracklet's team and jersey number (the one seen most), if any.
fn tracklet_number(e: &Tracklet) -> (Option<String>, Option<u64>) {
    let team = e.team.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|x| x.0.clone()).filter(|t| t == "home" || t == "away");
    let number = e.numbers.iter().max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0))).map(|(n, _)| *n);
    (team, number)
}

#[derive(Default, Clone, Copy)]
struct Line {
    made: [i64; 3],
    missed: [i64; 3],
    pts_for: i64,
    pts_against: i64,
}

/// Basketball box score from the scoreboard and the tracking, **determined electronically**:
/// per jersey number the playing time, made free throws / 2-pointers / 3-pointers (the live
/// scorer: [`crate::live_score`]), misses (attempts of the scene detection without points after
/// them – only as many as were seen) and the team's points for and against while the player was
/// on the court (plus/minus). `None` without board readings.
/// Shooters checked by hand: (team "home"/"away", that team's score after the basket) -> jersey
/// number, or `None` for "not known". They replace the electronic shooter.
pub type ScoreFixes = HashMap<(String, u32), Option<u64>>;

/// The hand checks of a project (`score_fixes`: {"home:62": 4, "away:45": null}).
pub fn parse_fixes(v: &Value) -> ScoreFixes {
    let mut m = ScoreFixes::new();
    for (k, n) in v.as_object().into_iter().flatten() {
        if let Some((team, to)) = k.split_once(':') {
            if let (Ok(to), "home" | "away") = (to.parse::<u32>(), team) {
                m.insert((team.to_string(), to), n.as_u64());
            }
        }
    }
    m
}

pub fn box_score(profile: &SportProfile, frames: &[Value], att: &[Attempt], fixes: &ScoreFixes) -> Option<Value> {
    if profile.id != "basketball" || profile.goals.len() != 2 {
        return None;
    }
    let raw = super::scoreboard::readings_in(frames);
    let carried = frames.iter().any(|f| f["events"].as_array().is_some_and(|e| e.iter().any(|e| e["type"] == "score")));
    if raw.is_empty() && !carried {
        return None;
    }
    let scores = crate::live_score::scores_of(frames, &raw, [profile.goals[0], profile.goals[1]]);
    let tl = tracklets(frames);
    let num_of = |key: (u64, u64)| tl.get(&key).map(tracklet_number).unwrap_or((None, None));
    let team_key = |t: Team| match t {
        Team::Home => "home",
        Team::Away => "away",
        Team::Unknown => "none",
    };
    let mut lines: HashMap<(String, u64), Line> = HashMap::new();
    let mut without: HashMap<String, Line> = HashMap::new();
    let mut team_only: HashMap<String, i64> = HashMap::new();
    let slot = |pts: i32| match pts {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        _ => None,
    };
    // made shots (also listed with their shooter, for the videos per player)
    let mut made_list: Vec<Value> = Vec::new();
    for e in scores.iter().filter(|e| e.points > 0) {
        let team = team_key(e.team).to_string();
        let auto = e.shooter.and_then(|(id, seg)| num_of((id as u64, seg as u64)).1).or(e.number.map(|n| n as u64));
        // a pick on the phone during the game counts like a check by hand (a later check wins)
        let manual = e.basis == crate::live_score::Basis::Manual;
        let picked = manual.then_some(e.number.map(|n| n as u64));
        let fix = fixes.get(&(team.clone(), e.to)).or(picked.as_ref());
        let shooter_no = match fix {
            Some(f) => *f,
            None => auto,
        };
        // the other candidates of the electronic choice, for a quick check by hand
        let mut cands: Vec<u64> = auto.into_iter().collect();
        for (key, _) in &e.others {
            if let Some(n) = num_of((key.0 as u64, key.1 as u64)).1 {
                if !cands.contains(&n) {
                    cands.push(n);
                }
            }
        }
        // then everybody of that team on the court right before, nearest a basket first
        {
            let i0 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < e.t - 4.0);
            let i1 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) <= e.t);
            let mut near: Vec<(f64, u64)> = Vec::new();
            for p in frames[i0..i1.max(i0)].iter().flat_map(|fr| fr["players"].as_array().into_iter().flatten()) {
                let (x, y) = (p["x"].as_f64().unwrap_or(-1.0), p["y"].as_f64().unwrap_or(-1.0));
                if p["team"].as_str() != Some(team.as_str()) || x < 0.2 || x > profile.field.length as f64 - 0.2 || y < 0.3 || y > profile.field.width as f64 - 0.3 {
                    continue;
                }
                let Some(n) = num_of((p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0))).1 else { continue };
                let d = profile.goals.iter().map(|g| ((g.x as f64 - x).powi(2) + (g.y as f64 - y).powi(2)).sqrt()).fold(f64::MAX, f64::min);
                match near.iter_mut().find(|q| q.1 == n) {
                    Some(q) => q.0 = q.0.min(d),
                    None => near.push((d, n)),
                }
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, n) in near {
                if !cands.contains(&n) {
                    cands.push(n);
                }
            }
        }
        made_list.push(json!({"t": r1(e.t), "team": team, "points": e.points, "to": e.to, "kind": e.kind.key(), "number": shooter_no, "auto": auto, "checked": fix.is_some(), "candidates": cands, "basis": e.basis.key()}));
        let Some(k) = slot(e.points) else {
            *team_only.entry(team).or_default() += e.points as i64;
            continue;
        };
        if e.shooter.is_none() && fix.is_none() {
            *team_only.entry(team).or_default() += e.points as i64;
            continue;
        }
        match shooter_no {
            Some(n) => lines.entry((team, n)).or_default().made[k] += 1,
            None => without.entry(team).or_default().made[k] += 1,
        }
    }
    // misses: attempts without points of that team shortly after
    let kinds = |k: &str| match k {
        "free_throw" => Some(0),
        "shot2" => Some(1),
        "shot3" => Some(2),
        _ => None,
    };
    let mut seen = 0usize;
    for a in att {
        let Some(k) = kinds(&a.kind) else { continue };
        if a.team != "home" && a.team != "away" {
            continue;
        }
        seen += 1;
        let made = scores.iter().any(|e| team_key(e.team) == a.team && e.points > 0 && e.t >= a.t - 1.0 && e.t <= a.t + 12.0);
        if made {
            continue;
        }
        let n = a.actor.and_then(|id| {
            let i = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < a.t);
            let fr = frames.get(i).or_else(|| frames.last())?;
            let p = fr["players"].as_array()?.iter().find(|p| p["id"].as_u64() == Some(id))?;
            num_of((id, p["segment"].as_u64().unwrap_or(0))).1
        });
        match n {
            Some(n) => lines.entry((a.team.clone(), n)).or_default().missed[k] += 1,
            None => without.entry(a.team.clone()).or_default().missed[k] += 1,
        }
    }
    // plus/minus: who of each team was on the court when the board changed
    for e in scores.iter().filter(|e| e.points != 0) {
        // the frames of the last seconds before it (a single frame misses hidden players)
        let i0 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < e.t - 3.0);
        let i1 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) <= e.t);
        let mut on: Vec<(String, u64)> = Vec::new();
        for p in frames[i0..i1.max(i0)].iter().flat_map(|fr| fr["players"].as_array().into_iter().flatten()) {
            if p["state"].as_str() == Some("lost") {
                continue;
            }
            // people at the bench or the table are not on the court
            let (x, y) = (p["x"].as_f64().unwrap_or(-1.0), p["y"].as_f64().unwrap_or(-1.0));
            if x < 0.2 || x > profile.field.length as f64 - 0.2 || y < 0.3 || y > profile.field.width as f64 - 0.3 {
                continue;
            }
            if let (Some(team), Some(n)) = num_of((p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0))) {
                if !on.contains(&(team.clone(), n)) {
                    on.push((team, n));
                }
            }
        }
        for (team, n) in on {
            let l = lines.entry((team.clone(), n)).or_default();
            if team == team_key(e.team) {
                l.pts_for += e.points as i64;
            } else {
                l.pts_against += e.points as i64;
            }
        }
    }
    let secs: HashMap<(String, u64), f64> = {
        let pl = players(frames);
        let mut m = HashMap::new();
        for team in ["home", "away"] {
            for p in pl[team].as_array().into_iter().flatten() {
                if let (Some(n), Some(s)) = (p["number"].as_u64(), p["seconds"].as_f64()) {
                    m.insert((team.to_string(), n), s);
                }
            }
        }
        m
    };
    let line_json = |l: &Line| {
        json!({"points": l.made[0] + 2 * l.made[1] + 3 * l.made[2], "ft": l.made[0], "two": l.made[1], "three": l.made[2],
            "missed": {"ft": l.missed[0], "two": l.missed[1], "three": l.missed[2]},
            "on_court": {"for": l.pts_for, "against": l.pts_against, "plus_minus": l.pts_for - l.pts_against}})
    };
    let list = |team: &str| {
        let mut nums: Vec<u64> = lines.keys().filter(|(t, _)| t == team).map(|(_, n)| *n).chain(secs.keys().filter(|(t, _)| t == team).map(|(_, n)| *n)).collect();
        nums.sort_unstable();
        nums.dedup();
        let mut v: Vec<Value> = nums
            .into_iter()
            .map(|n| {
                let l = lines.get(&(team.to_string(), n)).copied().unwrap_or_default();
                let mut j = line_json(&l);
                j["number"] = json!(n);
                j["seconds"] = json!(r1(secs.get(&(team.to_string(), n)).copied().unwrap_or(0.0)));
                j
            })
            .collect();
        v.sort_by(|a, b| b["points"].as_i64().cmp(&a["points"].as_i64()).then(b["seconds"].as_f64().unwrap_or(0.0).total_cmp(&a["seconds"].as_f64().unwrap_or(0.0))));
        v
    };
    let total = |team: &str| scores.iter().filter(|e| team_key(e.team) == team).map(|e| e.points as i64).sum::<i64>();
    Some(json!({
        "electronic": true,
        "home": list("home"), "away": list("away"),
        "without_number": {"home": line_json(&without.get("home").copied().unwrap_or_default()), "away": line_json(&without.get("away").copied().unwrap_or_default())},
        "team_only": {"home": team_only.get("home").copied().unwrap_or(0), "away": team_only.get("away").copied().unwrap_or(0)},
        "points": {"home": total("home"), "away": total("away")},
        "attempts_seen": seen,
        "scores": scores.len(),
        "made": made_list,
    }))
}

/// How a sport's score can be estimated: `(restart scene kinds after a score, seconds from the attempt to the restart)`.
fn goal_restart(sport: &str) -> Option<(&'static [&'static str], f64)> {
    match sport {
        "football" => Some((&["kickoff"], 120.0)),
        "futsal" => Some((&["kickoff"], 60.0)),
        "handball" => Some((&["kickoff", "throw_off"], 40.0)),
        "hockey" => Some((&["kickoff"], 90.0)),
        "lacrosse" => Some((&["kickoff"], 60.0)),
        _ => None,
    }
}

fn team_str(t: Team) -> &'static str {
    match t {
        Team::Home => "home",
        Team::Away => "away",
        Team::Unknown => "none",
    }
}

fn other(t: &str) -> &'static str {
    match t {
        "home" => "away",
        "away" => "home",
        _ => "none",
    }
}

/// The shooter of an attempt: the tracklet of its track at that time and the number read most
/// often on it (deduced when most of its numbered time was deduced).
fn shooter_of(frames: &[Value], tl: &HashMap<(u64, u64), Tracklet>, a: &Attempt) -> Option<Shooter> {
    let track = a.actor?;
    let i = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < a.t);
    let near = [i, i.saturating_sub(1)].into_iter().filter_map(|k| frames.get(k));
    let p = near.flat_map(|f| f["players"].as_array().into_iter().flatten()).find(|p| p["id"].as_u64() == Some(track))?;
    let segment = p["segment"].as_u64().unwrap_or(0);
    let e = tl.get(&(track, segment));
    let best = e.and_then(|e| e.numbers.iter().max_by(|x, y| x.1.total_cmp(y.1).then(y.0.cmp(x.0))).map(|(n, s)| (*n, *s)));
    let (number, status) = match (best, e) {
        (Some((n, secs)), Some(e)) => (Some(n as u32), if e.inferred > 0.5 * e.numbers.values().sum::<f64>().max(secs) { NumberStatus::Inferred } else { NumberStatus::Read }),
        _ => (None, NumberStatus::Unknown),
    };
    Some(Shooter { track: track as u32, segment: segment as u32, number, status })
}

fn team_of(s: &str) -> Team {
    match s {
        "home" => Team::Home,
        "away" => Team::Away,
        _ => Team::Unknown,
    }
}

/// Basketball with a calibrated scoreboard: the real score, every change on the board with the
/// attempt and shooter that most likely produced it, and the points per player.
fn score_from_board(frames: &[Value], att: &[Attempt], readings: &[crate::scoreboard::Reading]) -> Value {
    let tl = tracklets(frames);
    let shots: Vec<(usize, ShotAttempt)> = att
        .iter()
        .enumerate()
        .filter_map(|(i, a)| Some((i, ShotAttempt { t: a.t, team: team_of(&a.team), kind: ShotKind::parse(&a.kind)?, shooter: shooter_of(frames, &tl, a), confidence: a.confidence })))
        .collect();
    let attempts: Vec<ShotAttempt> = shots.iter().map(|s| s.1).collect();
    let changes = score_changes(readings);
    let credits = attribute(&changes, &attempts, &AttributionCfg::default());
    let last = |f: fn(&crate::scoreboard::Reading) -> Option<u32>| readings.iter().rev().find_map(f);
    let first = |f: fn(&crate::scoreboard::Reading) -> Option<u32>| readings.iter().find_map(f);
    let mut events = Vec::new();
    // points per (team, number); shooter without number; team only
    let mut per: HashMap<(&str, u32), (i64, i64, i64)> = HashMap::new();
    let mut no_number: HashMap<&str, i64> = HashMap::new();
    let mut team_only: HashMap<&str, i64> = HashMap::new();
    for c in &credits {
        let team = team_str(c.change.team);
        let a = c.attempt.map(|i| &att[shots[i].0]);
        let sh = c.shooter;
        events.push(json!({"t": r1(c.change.t), "team": team, "points": c.change.points, "from": c.change.from, "to": c.change.to,
            "basis": c.basis.key(), "probability": c.probability, "candidates": c.candidates, "cancelled": c.cancelled,
            "attempt": a.map(|a| json!({"t": r1(a.t), "kind": a.kind, "x": r1(a.pos.x as f64), "y": r1(a.pos.y as f64)})),
            "shooter": sh.map(|s| json!({"number": s.number, "status": s.status.key(), "track": s.track, "segment": s.segment}))}));
        if c.cancelled || c.change.points <= 0 {
            continue;
        }
        let pts = c.change.points as i64;
        match (c.basis, sh) {
            (Basis::Attempt, Some(Shooter { number: Some(n), status, .. })) => {
                let e = per.entry((team, n)).or_default();
                e.0 += pts;
                if status == NumberStatus::Inferred {
                    e.2 += pts;
                } else {
                    e.1 += pts;
                }
            }
            (Basis::Attempt, _) => *no_number.entry(team).or_default() += pts,
            _ => *team_only.entry(team).or_default() += pts,
        }
    }
    let list = |team: &str| {
        let mut v: Vec<(u32, (i64, i64, i64))> = per.iter().filter(|((t, _), _)| *t == team).map(|((_, n), p)| (*n, *p)).collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(n, (p, read, inf))| json!({"number": n, "points": p, "read": read, "inferred": inf})).collect::<Vec<_>>()
    };
    // the score over time, only where score or period change (not every clock tick)
    let mut timeline: Vec<Value> = Vec::new();
    let mut prev: Option<(Option<u32>, Option<u32>, Option<u32>)> = None;
    for r in readings {
        let k = (r.home, r.away, r.period);
        if prev != Some(k) {
            timeline.push(json!({"t": r1(r.t), "home": r.home, "away": r.away, "period": r.period, "clock": r.clock}));
            prev = Some(k);
        }
    }
    let n = |m: &HashMap<&str, i64>, k: &str| m.get(k).copied().unwrap_or(0);
    json!({"supported": true, "method": "scoreboard",
        "home": last(|r| r.home).unwrap_or(0), "away": last(|r| r.away).unwrap_or(0),
        "start": {"home": first(|r| r.home), "away": first(|r| r.away)},
        "period": last(|r| r.period), "clock": last(|r| r.clock),
        "events": events, "timeline": timeline,
        "players": {"home": list("home"), "away": list("away")},
        "no_number": {"home": n(&no_number, "home"), "away": n(&no_number, "away")},
        "team_only": {"home": n(&team_only, "home"), "away": n(&team_only, "away")}})
}

/// Estimated score from the attempts and the restarts after them (see the module docs). For
/// basketball with scoreboard readings in the frames, the board's score with the scorers instead.
pub fn score(profile: &SportProfile, frames: &[Value], att: &[Attempt], dir: Option<Direction>) -> Value {
    let sport = profile.id.as_str();
    if sport == "basketball" {
        // the raw readings of the board -> the most plausible course of the game
        let raw = super::scoreboard::readings_in(frames);
        let readings = crate::scoreboard::timeline::changed_readings(&crate::scoreboard::timeline::resolve(&raw, &Default::default()));
        if readings.iter().any(|r| r.home.is_some() || r.away.is_some()) {
            return score_from_board(frames, att, &readings);
        }
    }
    let scenes: Vec<&Value> = frames.iter().flat_map(|f| f["events"].as_array().into_iter().flatten()).filter(|e| e["type"] == "scene").collect();
    let t0 = frames.first().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let mut events: Vec<Value> = Vec::new();
    let method;
    if sport == "basketball" {
        method = "baseline_inbound";
        let l = profile.field.length as f64;
        for a in att.iter().filter(|a| matches!(a.kind.as_str(), "shot2" | "shot3")) {
            let side_l = a.goal == 0;
            let inbound = scenes.iter().find(|e| {
                let t = e["t"].as_f64().unwrap_or(-1.0);
                let x = e["x"].as_f64().unwrap_or(l / 2.0);
                e["kind"] == "throw_in" && t > a.t && t - a.t <= 10.0 && if side_l { x <= 1.5 } else { x >= l - 1.5 }
            });
            let Some(ib) = inbound else { continue };
            let ib_team = ib["team"].as_str().unwrap_or("none");
            // the inbounding team must be the other one; with a known direction it is the defender of that end
            let defender = dir.map(|d| other(team_str(d.attacker(if side_l { 'l' } else { 'r' }, a.t))));
            if ib_team == a.team || defender.is_some_and(|d| d != other(&a.team)) {
                continue;
            }
            let sure = ib_team == other(&a.team);
            events.push(json!({"t": r1(a.t), "team": a.team, "points": if a.kind == "shot3" { 3 } else { 2 }, "kind": a.kind,
                "evidence": ["attempt", "end_line_inbound"], "restart_t": r1(ib["t"].as_f64().unwrap_or(a.t)), "confidence": if sure { "likely" } else { "possible" }}));
        }
    } else if let Some((kinds, window)) = goal_restart(sport) {
        method = "restart_after_attempt";
        let mut last_goal = f64::NEG_INFINITY;
        let mut ks: Vec<&&Value> = scenes.iter().filter(|e| kinds.contains(&e["kind"].as_str().unwrap_or(""))).collect();
        ks.sort_by(|a, b| a["t"].as_f64().unwrap_or(0.0).total_cmp(&b["t"].as_f64().unwrap_or(0.0)));
        for k in ks {
            let t = k["t"].as_f64().unwrap_or(0.0);
            if t - t0 < 30.0 || t - last_goal < 30.0 {
                continue; // start of the video, or the same restart seen twice
            }
            if dir.and_then(|d| d.switch_at).is_some_and(|s| (s - 60.0..s + 600.0).contains(&t)) {
                continue; // start of the second half
            }
            let kicker = k["team"].as_str().filter(|s| *s == "home" || *s == "away");
            let shot = att.iter().filter(|a| a.t < t && t - a.t <= window && (a.team == "home" || a.team == "away")).max_by(|a, b| a.t.total_cmp(&b.t));
            let (team, evidence, conf): (&str, Vec<&str>, &str) = match (shot, kicker) {
                (Some(a), Some(kt)) if kt != a.team => (a.team.as_str(), vec!["attempt", "restart_by_other_team"], "likely"),
                (Some(_), Some(_)) => continue, // the attempting team kicks off: no goal by it
                (Some(a), None) => (a.team.as_str(), vec!["attempt", "restart"], "possible"),
                (None, Some(kt)) if dir.is_some() => (other(kt), vec!["restart_by_other_team"], "possible"),
                _ => continue,
            };
            last_goal = t;
            events.push(json!({"t": r1(shot.map(|a| a.t).unwrap_or(t)), "team": team, "points": 1, "kind": "goal", "evidence": evidence, "restart_t": r1(t), "confidence": conf}));
        }
    } else {
        return json!({"supported": false, "home": 0, "away": 0, "events": []});
    }
    let sum = |team: &str| events.iter().filter(|e| e["team"] == team).map(|e| e["points"].as_u64().unwrap_or(0)).sum::<u64>();
    let sum_likely = |team: &str| events.iter().filter(|e| e["team"] == team && e["confidence"] == "likely").map(|e| e["points"].as_u64().unwrap_or(0)).sum::<u64>();
    json!({"supported": true, "method": method, "home": sum("home"), "away": sum("away"),
        "likely": {"home": sum_likely("home"), "away": sum_likely("away")}, "events": events})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;

    fn pl(id: u64, seg: u64, team: &str, num: Option<u64>) -> Value {
        json!({"id": id, "segment": seg, "team": team, "state": "visible", "jersey": num, "identity": {"status": "unknown", "player": null}})
    }

    #[test]
    fn minutes_go_to_the_number_of_each_tracklet() {
        let mut frames = Vec::new();
        for i in 0..=120 {
            let t = i as f64;
            // slot 1: #7 for the first minute, then a new tracklet that reads #12
            let p1 = if i < 60 { pl(1, 0, "home", if i % 3 == 0 { Some(7) } else { None }) } else { pl(1, 1, "home", Some(12)) };
            frames.push(json!({"t": t, "players": [p1, pl(2, 0, "away", None)], "events": []}));
        }
        let v = players(&frames);
        let secs = |n: u64| v["home"].as_array().unwrap().iter().find(|p| p["number"] == n).map(|p| p["seconds"].as_f64().unwrap());
        assert_eq!(secs(7), Some(60.0));
        assert_eq!(secs(12), Some(60.0));
        assert_eq!(v["home"].as_array().unwrap().len(), 2);
        assert_eq!(v["without_number"]["away"], 120.0);
    }

    fn att(t: f64, team: &str, kind: &str, goal: usize) -> Attempt {
        Attempt { t, team: team.into(), pos: Vec2::new(0.0, 0.0), kind: kind.into(), zone: "", goal, distance: 5.0, confidence: 0.6, actor: None }
    }

    #[test]
    fn basketball_points_per_player_from_the_scoreboard() {
        let p = SportProfile::basketball();
        // #11 (home, read) and #4 (away, deduced) on the court the whole time
        let pl = |id: u64, team: &str, n: u64, status: &str| json!({"id": id, "segment": 0, "team": team, "state": "visible", "jersey": null,
            "identity": {"status": status, "player": {"team": team, "number": n}}});
        let mut frames = Vec::new();
        for i in 0..=60 {
            let t = i as f64;
            let mut ev = Vec::new();
            if i == 10 {
                ev.push(json!({"type": "scene", "phase": "attempt", "kind": "shot2", "team": "home", "side": "r", "x": 24.0, "y": 7.5, "confidence": 0.6, "t": t, "actor": 1}));
            }
            if i == 30 {
                ev.push(json!({"type": "scene", "phase": "attempt", "kind": "shot3", "team": "away", "side": "l", "x": 7.5, "y": 7.5, "confidence": 0.6, "t": t, "actor": 2}));
            }
            let board = |h: u32, a: u32| json!({"type": "scoreboard", "t": t, "home": h, "away": a, "clock": 300 - i, "period": 1});
            match i {
                0 => ev.push(board(20, 18)),
                13 => ev.push(board(22, 18)),
                34 => ev.push(board(22, 21)),
                50 => ev.push(board(24, 21)), // nothing seen before it: team only
                _ => {}
            }
            frames.push(json!({"t": t, "players": [pl(1, "home", 11, "candidate"), pl(2, "away", 4, "inferred")], "events": ev}));
        }
        let att = super::super::stats::attempts(&p, &frames);
        assert_eq!(att.len(), 2);
        let s = score(&p, &frames, &att, None);
        assert_eq!(s["method"], "scoreboard");
        assert_eq!((s["home"].as_u64(), s["away"].as_u64()), (Some(24), Some(21)));
        assert_eq!(s["players"]["home"][0], json!({"number": 11, "points": 2, "read": 2, "inferred": 0}));
        assert_eq!(s["players"]["away"][0], json!({"number": 4, "points": 3, "read": 0, "inferred": 3}));
        assert_eq!(s["team_only"]["home"], 2);
        assert_eq!(s["events"][0]["shooter"]["status"], "read");
        assert_eq!(s["events"][2]["basis"], "team_only");
        assert_eq!(s["timeline"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn basketball_box_score_per_player() {
        let p = SportProfile::basketball();
        let pl = |id: u64, team: &str, n: u64, x: f64, y: f64| json!({"id": id, "segment": 0, "team": team, "state": "visible", "jersey": null, "x": x, "y": y,
            "identity": {"status": "candidate", "player": {"team": team, "number": n}}});
        let mut frames = Vec::new();
        for i in 0..=140 {
            let t = i as f64 * 0.5;
            let mut ev = Vec::new();
            let shot = |kind: &str, team: &str, actor: u64, x: f64| json!({"type": "scene", "phase": "attempt", "kind": kind, "team": team, "side": if x > 14.0 { "r" } else { "l" }, "x": x, "y": 7.5, "confidence": 0.6, "t": t, "actor": actor});
            match i {
                20 => ev.push(shot("shot3", "away", 2, 20.0)), // missed: no away points after it
                36 => ev.push(shot("shot2", "home", 1, 24.0)),
                76 => ev.push(shot("shot3", "away", 2, 8.0)),
                _ => {}
            }
            let (h, a) = (if t < 20.0 { 20 } else { 22 }, if t < 40.0 { 18 } else { 21 });
            ev.push(json!({"type": "scoreboard", "t": t, "home": h, "away": a, "clock": 300 - i / 2, "period": 1}));
            let players = if t < 30.0 { vec![pl(1, "home", 11, 24.0, 7.5), pl(2, "away", 4, 22.0, 10.0)] } else { vec![pl(1, "home", 11, 4.0, 10.0), pl(2, "away", 4, 3.0, 7.5)] };
            frames.push(json!({"t": t, "players": players, "events": ev}));
        }
        let att = super::super::stats::attempts(&p, &frames);
        let b = box_score(&p, &frames, &att, &ScoreFixes::new()).expect("box score");
        assert_eq!(b["electronic"], true);
        let home = &b["home"][0];
        assert_eq!((home["number"].as_u64(), home["points"].as_i64(), home["two"].as_i64()), (Some(11), Some(2), Some(1)));
        assert_eq!(home["on_court"], json!({"for": 2, "against": 3, "plus_minus": -1}));
        let away = &b["away"][0];
        assert_eq!((away["number"].as_u64(), away["points"].as_i64(), away["three"].as_i64()), (Some(4), Some(3), Some(1)));
        assert_eq!(away["missed"]["three"], 1);
        assert_eq!(away["on_court"]["plus_minus"], 1);
        assert_eq!(b["points"], json!({"home": 2, "away": 3}));
    }

    #[test]
    fn football_goal_from_kickoff_after_attempt() {
        let p = SportProfile::football();
        let ko = |t: f64, team: &str| json!({"t": t, "players": [], "events": [{"type": "scene", "kind": "kickoff", "team": team, "t": t}]});
        let frames = vec![ko(0.0, "home"), ko(300.0, "away"), ko(900.0, "away"), json!({"t": 1000.0, "events": []})];
        let a = [att(250.0, "home", "shot", 1), att(880.0, "away", "shot", 0)];
        let s = score(&p, &frames, &a, None);
        assert_eq!(s["home"], 1); // kick-off by away 50 s after a home shot
        assert_eq!(s["away"], 0); // away shot, away kicks off: no goal
        assert_eq!(s["events"][0]["confidence"], "likely");
    }

    #[test]
    fn basketball_made_shot_from_end_line_inbound() {
        let p = SportProfile::basketball();
        let ib = |t: f64, team: &str, x: f64| json!({"t": t, "players": [], "events": [{"type": "scene", "kind": "throw_in", "team": team, "x": x, "y": 3.0, "t": t}]});
        let frames = vec![json!({"t": 0.0, "events": []}), ib(14.0, "away", 27.6), ib(40.0, "home", 14.0), json!({"t": 60.0, "events": []})];
        let a = [att(10.0, "home", "shot3", 1), att(35.0, "away", "shot2", 0)];
        let s = score(&p, &frames, &a, None);
        assert_eq!(s["home"], 3);
        assert_eq!(s["away"], 0); // side-line inbound: not after a made basket
        assert_eq!(score(&SportProfile::volleyball(), &frames, &a, None)["supported"], false);
    }
}
