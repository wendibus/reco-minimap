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

/// Playing time per team and jersey number.
pub fn players(frames: &[Value]) -> Value {
    let tl = tracklets(frames);
    let mut per: HashMap<(String, u64), f64> = HashMap::new();
    let mut deduced: HashMap<(String, u64), f64> = HashMap::new();
    let mut without: HashMap<String, f64> = HashMap::new();
    for e in tl.values() {
        let Some(team) = e.team.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|x| x.0.clone()).filter(|t| t == "home" || t == "away") else { continue };
        match e.numbers.iter().max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0))) {
            Some((n, _)) => {
                *per.entry((team.clone(), *n)).or_default() += e.secs;
                *deduced.entry((team, *n)).or_default() += e.inferred;
            }
            None => *without.entry(team).or_default() += e.secs,
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
        let readings = super::scoreboard::readings_in(frames);
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
