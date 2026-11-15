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

use std::collections::HashMap;

use serde_json::{json, Value};

use super::stats::Attempt;
use crate::direction::Direction;
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
}

/// Playing time per team and jersey number.
pub fn players(frames: &[Value]) -> Value {
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
            let number = p["identity"]["player"]["number"].as_u64().filter(|_| p["identity"]["status"] == "candidate").or_else(|| p["jersey"].as_u64());
            if let Some(n) = number {
                *e.numbers.entry(n).or_default() += dt;
            }
        }
    }
    let mut per: HashMap<(String, u64), f64> = HashMap::new();
    let mut without: HashMap<String, f64> = HashMap::new();
    for e in tl.values() {
        let Some(team) = e.team.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|x| x.0.clone()).filter(|t| t == "home" || t == "away") else { continue };
        match e.numbers.iter().max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0))) {
            Some((n, _)) => *per.entry((team, *n)).or_default() += e.secs,
            None => *without.entry(team).or_default() += e.secs,
        }
    }
    let t0 = frames.first().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let t1 = frames.last().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let span = (t1 - t0).max(1e-6);
    let list = |team: &str| {
        let mut v: Vec<(u64, f64)> = per.iter().filter(|((tm, _), _)| tm == team).map(|((_, n), s)| (*n, *s)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(n, s)| json!({"number": n, "seconds": r1(s), "share": r1(s / span * 100.0) / 100.0})).collect::<Vec<_>>()
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

/// Estimated score from the attempts and the restarts after them (see the module docs).
pub fn score(profile: &SportProfile, frames: &[Value], att: &[Attempt], dir: Option<Direction>) -> Value {
    let sport = profile.id.as_str();
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
        Attempt { t, team: team.into(), pos: Vec2::new(0.0, 0.0), kind: kind.into(), zone: "", goal, distance: 5.0, confidence: 0.6 }
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
