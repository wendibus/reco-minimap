//! `simulate`: a finished analysis played back as if it were live - the overlay, the ticker, the
//! `/scoreboard` page and the phone page for the shooters run as in live mode, at the speed of the
//! game or faster. What is confirmed on the phone is saved next to the result
//! (`<stem>.baskets.json`) and then makes the score of the statistics and the videos.
//!
//! Two ways to confirm, chosen by how well the hall scoreboard was read:
//! * **board**: the baskets the scoreboard showed, each with a shooter to name (as live);
//! * **attempts**: every detected shot attempt - in or missed, by whom, 1/2/3 points. Baskets the
//!   detection did not see (free throws) are added on the phone.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::live::Hub;
use crate::geom::Vec2;
use crate::{tr, trs};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Board,
    Attempts,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Board => "board",
            Mode::Attempts => "attempts",
        }
    }
}

/// A card to offer at its time.
struct Offer {
    t: f64,
    event: Value,
    candidates: Vec<u64>,
    attempt: bool,
}

/// Jersey numbers of `team` in `frame`, nearest to a basket first; `first` (the actor) leads.
fn candidates(frame: &Value, team: &str, first: Option<u64>, goals: &[Vec2]) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let players: Vec<&Value> = frame["players"].as_array().into_iter().flatten().filter(|p| p["team"] == team).collect();
    if let Some(n) = first.and_then(|id| players.iter().find(|p| p["id"].as_u64() == Some(id))).and_then(|p| p["jersey"].as_u64()) {
        out.push(n);
    }
    let mut near: Vec<(f32, u64)> = players.iter().filter_map(|p| {
        let n = p["jersey"].as_u64()?;
        let pos = Vec2::new(p["x"].as_f64()? as f32, p["y"].as_f64()? as f32);
        Some((goals.iter().map(|g| g.dist(pos)).fold(f32::MAX, f32::min), n))
    }).collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, n) in near {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.truncate(6);
    out
}

/// What the result offers: the scoreboard's baskets (final, with candidates), the shot attempts,
/// the board readings, and the time span.
struct Scan {
    baskets: Vec<Offer>,
    attempts: Vec<Offer>,
    readings: Vec<crate::scoreboard::Reading>,
    t0: f64,
    t1: f64,
}

fn lines(path: &str) -> Result<impl Iterator<Item = Value>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(std::io::BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str::<Value>(&l).ok()).filter(|v| v["t"].is_number()))
}

fn scan(path: &str, goals: &[Vec2]) -> Result<Scan, String> {
    let (mut raw_scores, mut cand_of): (Vec<crate::live_score::ScoreEvent>, BTreeMap<u32, Vec<u64>>) = (Vec::new(), BTreeMap::new());
    let (mut attempts, mut readings) = (Vec::new(), Vec::new());
    let (mut t0, mut t1) = (f64::MAX, 0.0f64);
    for f in lines(path)? {
        let t = f["t"].as_f64().unwrap_or(0.0);
        t0 = t0.min(t);
        t1 = t1.max(t);
        for e in f["events"].as_array().into_iter().flatten() {
            match e["type"].as_str() {
                Some("score") => {
                    if let Some(se) = crate::live_score::parse_event(e) {
                        if !cand_of.contains_key(&se.sid) {
                            let team = if se.team == crate::team::Team::Home { "home" } else { "away" };
                            let mut c = candidates(&f, team, None, goals);
                            if let Some(n) = se.number.map(|n| n as u64) {
                                c.retain(|x| *x != n);
                                c.insert(0, n);
                            }
                            cand_of.insert(se.sid, c);
                        }
                        raw_scores.push(se);
                    }
                }
                Some("scoreboard") => readings.extend(super::scoreboard::parse_reading(e)),
                Some("scene") if matches!(e["kind"].as_str(), Some("shot2" | "shot3")) && matches!(e["team"].as_str(), Some("home" | "away")) => {
                    let team = e["team"].as_str().unwrap_or("home");
                    let points = if e["kind"] == "shot3" { 3 } else { 2 };
                    let sid = 1_000_000 + attempts.len() as u64;
                    let ev = json!({"type": "score", "sid": sid, "t": e["t"].as_f64().unwrap_or(t), "team": team, "points": points, "kind": "field_goal"});
                    attempts.push(Offer { t: e["t"].as_f64().unwrap_or(t), event: ev, candidates: candidates(&f, team, e["actor"].as_u64(), goals), attempt: true });
                }
                _ => {}
            }
        }
    }
    let baskets = crate::live_score::final_events(&raw_scores)
        .into_iter()
        .filter(|e| matches!(e.kind, crate::live_score::ScoreKind::FieldGoal | crate::live_score::ScoreKind::FreeThrow) && e.points > 0)
        .map(|e| Offer { t: e.t, candidates: cand_of.get(&e.sid).cloned().unwrap_or_default(), event: e.to_json(), attempt: false })
        .collect();
    if t1 <= t0 {
        return Err(trs!("zu wenige Bilder in der Datei", "too few frames in the file", "trop peu d'images dans le fichier", "muy pocas imágenes en el archivo").into());
    }
    Ok(Scan { baskets, attempts, readings, t0, t1 })
}

/// Board when the scoreboard was read well enough and showed baskets, else attempts.
fn auto_mode(s: &Scan) -> Mode {
    let ok = super::scoreboard::quality(&s.readings)["ok"].as_bool() == Some(true);
    if ok && s.baskets.len() >= 5 { Mode::Board } else { Mode::Attempts }
}

/// A decision of the phone: made (with number and points) or missed.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub sid: u64,
    pub t: f64,
    pub team: String,
    pub points: i64,
    pub made: bool,
    pub number: Option<u64>,
    pub added: bool,
}

/// The file next to the result: the decisions (to go on later) and, from them, the score events.
pub fn baskets_json(mode: Mode, decisions: &BTreeMap<u64, Decision>, board: &BTreeMap<u64, Value>) -> Value {
    let mut ds: Vec<&Decision> = decisions.values().collect();
    ds.sort_by(|a, b| a.t.total_cmp(&b.t));
    let events: Vec<Value> = match mode {
        Mode::Attempts => {
            let (mut h, mut a) = (0i64, 0i64);
            ds.iter().filter(|d| d.made).map(|d| {
                let s = if d.team == "home" { &mut h } else { &mut a };
                let from = *s;
                *s += d.points;
                json!({"type": "score", "sid": d.sid, "t": d.t, "team": d.team, "points": d.points, "from": from, "to": *s,
                    "kind": if d.points == 1 { "free_throw" } else { "field_goal" }, "basis": "manual", "number": d.number, "read": d.number.is_some(), "confidence": 1.0})
            }).collect()
        }
        // the scoreboard's baskets with the shooters named on the phone
        Mode::Board => ds.iter().filter(|d| d.made).filter_map(|d| {
            let mut e = board.get(&d.sid)?.clone();
            e["number"] = json!(d.number);
            e["read"] = json!(d.number.is_some());
            e["basis"] = json!("manual");
            e["update"] = json!(true);
            e["confidence"] = json!(1.0);
            e["shooter"] = Value::Null;
            e["others"] = json!([]);
            Some(e)
        }).collect(),
    };
    json!({"format": "reco-minimap-baskets", "version": 1, "mode": mode.key(), "replace": mode == Mode::Attempts,
        "decisions": ds.iter().map(|d| json!({"sid": d.sid, "t": d.t, "team": d.team, "points": d.points, "made": d.made, "number": d.number, "added": d.added})).collect::<Vec<_>>(),
        "events": events})
}

fn load_decisions(path: &Path) -> (Option<Mode>, BTreeMap<u64, Decision>) {
    let Some(v) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return (None, BTreeMap::new()) };
    let mode = match v["mode"].as_str() {
        Some("board") => Some(Mode::Board),
        Some("attempts") => Some(Mode::Attempts),
        _ => None,
    };
    let ds = v["decisions"].as_array().into_iter().flatten().filter_map(|d| Some(Decision {
        sid: d["sid"].as_u64()?, t: d["t"].as_f64()?, team: d["team"].as_str()?.to_string(), points: d["points"].as_i64().unwrap_or(2),
        made: d["made"].as_bool().unwrap_or(true), number: d["number"].as_u64(), added: d["added"].as_bool().unwrap_or(false),
    })).map(|d| (d.sid, d)).collect();
    (mode, ds)
}

fn save(path: &Path, v: &Value) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub struct Params<'a> {
    pub result: &'a str,
    pub goals: Vec<Vec2>,
    pub speed: f64,
    pub from: Option<f64>,
    pub mode: Option<Mode>,
    pub baskets: PathBuf,
}

/// The score the pages show at `t`: board mode the board's own score, attempts mode the made picks.
fn score_at(mode: Mode, t: f64, decisions: &BTreeMap<u64, Decision>, board_scores: &[(f64, u32, u32)], readings: &[crate::scoreboard::Reading]) -> Value {
    let (mut h, mut a) = (0i64, 0i64);
    match mode {
        Mode::Attempts => {
            for d in decisions.values().filter(|d| d.made && d.t <= t) {
                if d.team == "home" { h += d.points } else { a += d.points }
            }
        }
        Mode::Board => {
            if let Some(s) = board_scores.iter().rev().find(|s| s.0 <= t) {
                (h, a) = (s.1 as i64, s.2 as i64);
            }
        }
    }
    let j = readings.partition_point(|r| r.t <= t);
    let period = readings[..j].iter().rev().find_map(|r| r.period);
    let clock = (j > 0).then(|| readings[j - 1]).filter(|r| t - r.t < 3.0).and_then(|r| r.clock);
    json!({"home": h, "away": a, "period": period, "clock": clock})
}

/// Play the result back until it ends and the operator stops (the phone page stays usable after
/// the end). `step`: progress (fraction, message).
pub fn run(hub: Arc<Hub>, p: Params, mut step: impl FnMut(f64, &str)) -> Result<usize, String> {
    let sc = scan(p.result, &p.goals)?;
    let (old_mode, mut decisions) = load_decisions(&p.baskets);
    let mode = p.mode.or(old_mode).unwrap_or_else(|| auto_mode(&sc));
    eprintln!("{}", match mode {
        Mode::Board => tr!("Körbe von der Anzeigetafel: {} zum Bestätigen", "Baskets from the scoreboard: {} to confirm", "Paniers du tableau : {} à confirmer", "Canastas del marcador: {} por confirmar", sc.baskets.len()),
        Mode::Attempts => tr!("Anzeigetafel nicht verlässlich – Wurfversuche zum Bestätigen: {}", "Scoreboard not reliable – shot attempts to confirm: {}", "Tableau peu fiable – tentatives à confirmer : {}", "Marcador poco fiable: intentos por confirmar: {}", sc.attempts.len()),
    });
    let board_events: BTreeMap<u64, Value> = sc.baskets.iter().filter_map(|o| Some((o.event["sid"].as_u64()?, o.event.clone()))).collect();
    let mut board_scores: Vec<(f64, u32, u32)> = Vec::new();
    {
        let (mut h, mut a) = (0u32, 0u32);
        for o in &sc.baskets {
            let to = o.event["to"].as_u64().unwrap_or(0) as u32;
            if o.event["team"] == "home" { h = to } else { a = to }
            board_scores.push((o.t, h, a));
        }
    }
    // baskets added on the phone before: their cards come again at their time
    let added: Vec<Offer> = decisions.values().filter(|d| d.added).map(|d| Offer {
        t: d.t, candidates: Vec::new(), attempt: true,
        event: json!({"type": "score", "sid": d.sid, "t": d.t, "team": d.team, "points": d.points, "kind": if d.points == 1 { "free_throw" } else { "field_goal" }, "added": true}),
    }).collect();
    let mut offers: Vec<&Offer> = match mode {
        Mode::Board => sc.baskets.iter().collect(),
        Mode::Attempts => sc.attempts.iter().collect(),
    };
    offers.extend(added.iter());
    offers.sort_by(|a, b| a.t.total_cmp(&b.t));
    let start = p.from.unwrap_or(sc.t0).clamp(sc.t0, sc.t1);
    let speed = p.speed.clamp(0.25, 32.0);
    hub.enable_adding(speed, decisions.keys().copied().filter(|s| *s >= 9_000_000).max().unwrap_or(0));
    // cards decided earlier: offered again as decided (from the start of the replay on)
    let decide_again = |o: &Offer, d: &Decision| {
        let sid = o.event["sid"].as_u64().unwrap_or(0);
        if o.attempt { hub.choose_attempt(sid, d.made, d.number, Some(d.points)); } else { hub.choose(sid, d.number); }
    };
    let mut next_offer = offers.partition_point(|o| o.t < start);
    // the decisions before the start still count; their cards are not shown again
    let wall = Instant::now();
    let mut frames = lines(p.result)?.skip_while(move |f| f["t"].as_f64().unwrap_or(0.0) < start).peekable();
    let mut ended = false;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    loop {
        if super::progress::stop_requested() {
            break;
        }
        let t = start + wall.elapsed().as_secs_f64() * speed;
        hub.set_clock(t.min(sc.t1));
        // frames up to now: published (the newest only, older ones would be stale) with their events
        let mut newest: Option<Value> = None;
        while let Some(f) = frames.peek() {
            if f["t"].as_f64().unwrap_or(0.0) > t {
                break;
            }
            let mut f = frames.next().unwrap();
            // the scoreboard's score events are replaced by the confirmed ones in attempts mode
            if mode == Mode::Attempts {
                if let Some(a) = f["events"].as_array_mut() {
                    a.retain(|e| e["type"] != "score");
                }
            }
            if let Some(ev) = f["events"].as_array() {
                hub.push_events(ev);
            }
            newest = Some(f);
        }
        if let Some(mut f) = newest {
            f["score"] = score_at(mode, t, &decisions, &board_scores, &sc.readings);
            f["age_ms"] = json!(0);
            hub.publish(f.to_string());
        }
        if frames.peek().is_none() && !ended {
            ended = true;
            eprintln!("{}", trs!("Ende der Aufnahme erreicht – die Handy-Seite bleibt offen, bis du beendest.", "End of the recording – the phone page stays open until you stop.", "Fin de l'enregistrement – la page du téléphone reste ouverte jusqu'à l'arrêt.", "Fin de la grabación: la página del móvil sigue abierta hasta que termines."));
        }
        while next_offer < offers.len() && offers[next_offer].t <= t {
            let o = offers[next_offer];
            if o.attempt { hub.offer_attempt(&o.event, o.candidates.clone()); } else { hub.offer_pick(&o.event, o.candidates.clone()); }
            if let Some(d) = o.event["sid"].as_u64().and_then(|sid| decisions.get(&sid)).cloned() {
                decide_again(o, &d);
            }
            next_offer += 1;
        }
        // decisions of the phone
        let manual = hub.take_manual();
        if !manual.is_empty() {
            for m in manual {
                let Some(sid) = m["sid"].as_u64() else { continue };
                if m["undone"] == true {
                    decisions.remove(&sid);
                    continue;
                }
                decisions.insert(sid, Decision {
                    sid, t: m["t"].as_f64().unwrap_or(t), team: m["team"].as_str().unwrap_or("home").to_string(), points: m["points"].as_i64().unwrap_or(2),
                    made: m["made"].as_bool().unwrap_or(true), number: m["number"].as_u64(), added: m["added"] == true,
                });
            }
            save(&p.baskets, &baskets_json(mode, &decisions, &board_events))?;
        }
        if last_report.elapsed() >= Duration::from_millis(500) {
            last_report = Instant::now();
            let made = decisions.values().filter(|d| d.made).count();
            let f = ((t - start) / (sc.t1 - start).max(1.0)).clamp(0.0, 1.0);
            step(f, &tr!("{} von {} · {} Körbe bestätigt", "{} of {} · {} baskets confirmed", "{} sur {} · {} paniers confirmés", "{} de {} · {} canastas confirmadas", clock(t.min(sc.t1)), clock(sc.t1), made));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    if !decisions.is_empty() {
        save(&p.baskets, &baskets_json(mode, &decisions, &board_events))?;
    }
    Ok(decisions.values().filter(|d| d.made).count())
}

fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The confirmed baskets of a result (if any) put into its frames: in attempts mode they replace
/// the scoreboard's score events, in board mode they name the shooters of its baskets.
pub fn apply_baskets(result: &Path, frames: &mut [Value]) -> bool {
    let Some(v) = std::fs::read_to_string(super::render_video::baskets_path(result)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return false };
    let events = v["events"].as_array().cloned().unwrap_or_default();
    if events.is_empty() || frames.is_empty() {
        return false;
    }
    if v["replace"].as_bool() == Some(true) {
        for f in frames.iter_mut() {
            if let Some(a) = f["events"].as_array_mut() {
                a.retain(|e| e["type"] != "score");
            }
        }
    }
    for e in events {
        let t = e["t"].as_f64().unwrap_or(0.0);
        // board updates go after the original event (same frame or later): into the last frame
        let i = if v["replace"].as_bool() == Some(true) { frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < t).min(frames.len() - 1) } else { frames.len() - 1 };
        if !frames[i]["events"].is_array() {
            frames[i]["events"] = json!([]);
        }
        if let Some(a) = frames[i]["events"].as_array_mut() {
            a.push(e);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(sid: u64, t: f64, team: &str, points: i64, made: bool, number: Option<u64>) -> Decision {
        Decision { sid, t, team: team.into(), points, made, number, added: false }
    }

    #[test]
    fn confirmed_attempts_become_score_events_that_add_up() {
        let mut ds = BTreeMap::new();
        for x in [d(3, 30.0, "home", 3, true, Some(8)), d(1, 10.0, "home", 2, true, None), d(2, 20.0, "away", 2, false, None), d(4, 40.0, "away", 1, true, Some(21))] {
            ds.insert(x.sid, x);
        }
        let v = baskets_json(Mode::Attempts, &ds, &BTreeMap::new());
        assert_eq!(v["replace"], true);
        let ev = v["events"].as_array().unwrap();
        assert_eq!(ev.len(), 3, "the miss is no basket");
        assert_eq!((ev[0]["from"].as_i64(), ev[0]["to"].as_i64()), (Some(0), Some(2)));
        assert_eq!((ev[1]["from"].as_i64(), ev[1]["to"].as_i64(), ev[1]["number"].as_u64()), (Some(2), Some(5), Some(8)));
        assert_eq!((ev[2]["kind"].as_str(), ev[2]["to"].as_i64()), (Some("free_throw"), Some(1)));
        // they parse as score events and make the timeline of the videos
        let parsed: Vec<_> = ev.iter().filter_map(crate::live_score::parse_event).collect();
        let tl = crate::tools::overlay::Timeline::from_events(&parsed, &[]);
        let s = tl.at(45.0);
        assert_eq!((s.home, s.away), (5, 1));
        assert_eq!(v["decisions"].as_array().unwrap().len(), 4, "the miss is kept to go on later");
    }

    #[test]
    fn confirmed_baskets_replace_the_boards_score_in_the_statistics_frames() {
        let dir = std::env::temp_dir().join(format!("reco-sim-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let result = dir.join("game.jsonl");
        let mut ds = BTreeMap::new();
        ds.insert(1, d(1, 5.0, "home", 3, true, Some(8)));
        std::fs::write(super::super::render_video::baskets_path(&result), baskets_json(Mode::Attempts, &ds, &BTreeMap::new()).to_string()).unwrap();
        let mut frames = vec![json!({"t": 0.0, "events": [{"type": "score", "sid": 7, "team": "away", "points": 11, "t": 0.0}]}), json!({"t": 5.2, "events": []})];
        assert!(apply_baskets(&result, &mut frames));
        assert!(frames[0]["events"].as_array().unwrap().is_empty());
        assert_eq!(frames[1]["events"][0]["number"], 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
