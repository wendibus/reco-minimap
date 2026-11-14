//! Game statistics from a run's `.jsonl` file, per sport, for the charts in the interface
//! (`reco-minimap stats FILE --court court.json --json`).
//!
//! Only what the tracking can see is counted: shot **attempts** (no made / missed, no score),
//! where they were taken, ball possession, where the teams and the ball were, and the moments
//! worth a clip. Basketball attempts come from the scene detector (`--scenes`); for the goal
//! sports an attempt is a fast ball flight that starts within range and points at a goal.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::direction::{estimate, Direction, Obs};
use crate::geom::Vec2;
use crate::team::Team;
use crate::sport::SportProfile;

/// One shot / throw attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    pub t: f64,
    /// "home" | "away" | "none"
    pub team: String,
    pub pos: Vec2,
    /// basketball: "shot2" | "shot3" | "free_throw"; goal sports: "shot" | "seven_meter" | "penalty"
    pub kind: String,
    /// Zone key, see [`zone_of`].
    pub zone: &'static str,
    /// Index into the profile's goals (0 = left).
    pub goal: usize,
    pub distance: f32,
    pub confidence: f32,
}

/// Shot rules of the goal sports: (goal half width m, longest shot m, slowest shot m/s).
fn shot_rules(sport: &str) -> Option<(f32, f32, f32)> {
    match sport {
        "football" => Some((3.66, 35.0, 10.0)),
        "futsal" => Some((1.5, 20.0, 8.0)),
        "handball" => Some((1.5, 14.0, 9.0)),
        "hockey" => Some((1.83, 25.0, 10.0)),
        "lacrosse" => Some((0.915, 20.0, 10.0)),
        _ => None,
    }
}

/// Goal / basket centres; lacrosse goals stand 15 yards inside the end lines.
pub fn goals(p: &SportProfile) -> Vec<Vec2> {
    if p.goals.is_empty() && p.id == "lacrosse" {
        let gl = 13.716;
        return vec![Vec2::new(gl, p.field.width / 2.0), Vec2::new(p.field.length - gl, p.field.width / 2.0)];
    }
    p.goals.clone()
}

/// The zones of a sport in display order (keys are translated by the interface).
pub fn zones(sport: &str) -> &'static [&'static str] {
    match sport {
        "basketball" => &["restricted", "paint", "midrange", "corner3", "above3", "free_throw"],
        "football" => &["goal_area", "penalty_area", "outside_box"],
        "futsal" => &["inside_6", "6_to_10", "outside_10"],
        "handball" => &["6m", "wing", "9m", "seven_meter"],
        "hockey" => &["circle_close", "circle", "outside_circle"],
        "lacrosse" => &["crease", "close", "outside"],
        _ => &[],
    }
}

/// The zone an attempt was taken from, measured from the goal / basket it went to.
pub fn zone_of(sport: &str, kind: &str, pos: Vec2, goal: Vec2, field_len: f32) -> &'static str {
    let d = ((pos.x - goal.x).powi(2) + (pos.y - goal.y).powi(2)).sqrt();
    let along = (pos.x - goal.x).abs(); // distance from the goal line / basket along the field
    let across = (pos.y - goal.y).abs();
    let _ = field_len;
    match sport {
        "basketball" => match kind {
            "free_throw" => "free_throw",
            "shot3" => {
                // the straight part of the 3-point line runs up to 2.99 m from the baseline (basket 1.575 m in)
                if along + 1.575 <= 2.99 + 0.3 { "corner3" } else { "above3" }
            }
            _ => {
                if d <= 1.25 + 0.4 {
                    "restricted"
                } else if along + 1.575 <= 5.8 && across <= 2.45 {
                    "paint"
                } else {
                    "midrange"
                }
            }
        },
        "football" => {
            if along <= 5.5 && across <= 9.16 {
                "goal_area"
            } else if along <= 16.5 && across <= 20.16 {
                "penalty_area"
            } else {
                "outside_box"
            }
        }
        "futsal" => {
            if d <= 6.0 {
                "inside_6"
            } else if d <= 10.0 {
                "6_to_10"
            } else {
                "outside_10"
            }
        }
        "handball" => {
            if kind == "seven_meter" {
                "seven_meter"
            } else if d > 9.0 {
                "9m"
            } else if across > 5.0 && along < 4.0 {
                "wing"
            } else {
                "6m"
            }
        }
        "hockey" => {
            if d <= 7.0 {
                "circle_close"
            } else if d <= 14.63 {
                "circle"
            } else {
                "outside_circle"
            }
        }
        "lacrosse" => {
            if d <= 2.743 + 1.0 {
                "crease"
            } else if d <= 11.0 {
                "close"
            } else {
                "outside"
            }
        }
        _ => "",
    }
}

fn f(v: &Value, k: &str) -> Option<f32> {
    v[k].as_f64().map(|x| x as f32)
}

fn team_of(frame: &Value, id: u64) -> Option<String> {
    frame["players"].as_array()?.iter().find(|p| p["id"].as_u64() == Some(id)).and_then(|p| p["team"].as_str()).map(|s| s.to_string())
}

fn nearest_goal(goals: &[Vec2], p: Vec2) -> usize {
    (0..goals.len()).min_by(|&a, &b| p.dist(goals[a]).partial_cmp(&p.dist(goals[b])).unwrap()).unwrap_or(0)
}

/// Attempts in a run: basketball from the scene events, goal sports from ball flights.
pub fn attempts(profile: &SportProfile, frames: &[Value]) -> Vec<Attempt> {
    let sport = profile.id.as_str();
    let goals = goals(profile);
    let mut out: Vec<Attempt> = Vec::new();
    // scene events: basketball attempts, handball 7 m, football / hockey penalties
    for fr in frames {
        for e in fr["events"].as_array().into_iter().flatten().filter(|e| e["type"] == "scene") {
            let kind = e["kind"].as_str().unwrap_or("");
            let is_attempt = match sport {
                "basketball" => e["phase"] == "attempt" && matches!(kind, "shot2" | "shot3" | "free_throw"),
                "handball" => kind == "seven_meter",
                "football" | "hockey" | "futsal" => kind == "penalty",
                _ => false,
            };
            if !is_attempt || goals.is_empty() {
                continue;
            }
            let pos = Vec2::new(f(e, "x").unwrap_or(0.0), f(e, "y").unwrap_or(0.0));
            let goal = match e["side"].as_str() {
                Some("l") => 0,
                Some("r") => goals.len() - 1,
                _ => nearest_goal(&goals, pos),
            };
            out.push(Attempt {
                t: e["t"].as_f64().unwrap_or(0.0),
                team: e["team"].as_str().unwrap_or("none").to_string(),
                pos,
                kind: kind.to_string(),
                zone: zone_of(sport, kind, pos, goals[goal], profile.field.length),
                goal,
                distance: pos.dist(goals[goal]),
                confidence: f(e, "confidence").unwrap_or(0.0),
            });
        }
    }
    // goal sports: a fast flight that starts within range and points at a goal
    if let Some((half, range, vmin)) = shot_rules(sport) {
        let mut last_holder: Option<(f64, String)> = None;
        let mut prev_state = String::new();
        let mut last_shot = f64::NEG_INFINITY;
        for (i, fr) in frames.iter().enumerate() {
            let t = fr["t"].as_f64().unwrap_or(0.0);
            let b = &fr["ball"];
            let st = b["state"].as_str().unwrap_or("unknown").to_string();
            if st == "held" {
                if let Some(team) = b["holder"].as_u64().and_then(|h| team_of(fr, h)) {
                    last_holder = Some((t, team));
                }
            }
            let starts = st == "flight" && prev_state != "flight";
            prev_state = st;
            if !starts || t - last_shot < 2.0 {
                continue;
            }
            let (Some(x), Some(y)) = (f(b, "x"), f(b, "y")) else { continue };
            let p = Vec2::new(x, y);
            // the velocity of the first flight frame, or the step to the next frame with a ball position
            let mut vel = match (f(b, "vx"), f(b, "vy")) {
                (Some(vx), Some(vy)) => Vec2::new(vx, vy),
                _ => Vec2::default(),
            };
            if vel.len() < 0.1 {
                if let Some(n) = frames.get(i + 1) {
                    if let (Some(nx), Some(ny), Some(nt)) = (f(&n["ball"], "x"), f(&n["ball"], "y"), n["t"].as_f64()) {
                        let dt = (nt - t).max(1e-3) as f32;
                        vel = Vec2::new((nx - x) / dt, (ny - y) / dt);
                    }
                }
            }
            let speed = vel.len();
            if speed < vmin {
                continue;
            }
            let best = goals
                .iter()
                .enumerate()
                .filter_map(|(gi, g)| {
                    let d = p.dist(*g);
                    if d > range || d < 0.5 {
                        return None;
                    }
                    let to = Vec2::new((g.x - p.x) / d, (g.y - p.y) / d);
                    let cos = (vel.x * to.x + vel.y * to.y) / speed;
                    let tol = ((half + 1.0) / d).atan() + 8f32.to_radians();
                    (cos >= tol.cos()).then_some((gi, d, cos))
                })
                .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
            if let Some((gi, d, cos)) = best {
                last_shot = t;
                let team = last_holder.as_ref().filter(|(ht, _)| t - ht <= 1.5).map(|(_, tm)| tm.clone()).unwrap_or_else(|| "none".into());
                out.push(Attempt { t, team, pos: p, kind: "shot".into(), zone: zone_of(sport, "shot", p, goals[gi], profile.field.length), goal: gi, distance: d, confidence: cos.clamp(0.0, 1.0) });
            }
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    out
}

/// Number of time sections the video is split into for the "per period" chart.
fn periods(sport: &str) -> usize {
    match sport {
        "basketball" | "hockey" | "american_football" | "lacrosse" => 4,
        "volleyball" => 5,
        _ => 2,
    }
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Spread of one team in one frame: (width across the field, depth along it, centre x, centre y).
/// Needs at least three visible players on the field. With seven or more, the player farthest
/// from the team along the length (usually the goalkeeper) is left out of the depth.
fn team_spread(pts: &[(f32, f32)]) -> Option<(f32, f32, f32, f32)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f32;
    let (cx, cy) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
    let mut xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if xs.len() >= 7 {
        if (xs[xs.len() - 1] - cx).abs() > (xs[0] - cx).abs() {
            xs.pop();
        } else {
            xs.remove(0);
        }
    }
    let (ylo, yhi) = pts.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    Some((yhi - ylo, xs[xs.len() - 1] - xs[0], cx, cy))
}

/// Team shape over the run, split into "all frames", "with the ball" and "without the ball"
/// (possession = the team of the last player who held the ball):
/// width and depth over time (`series`, all frames), and per time section and phase the mean
/// width / depth, the team centre and the average position of every player slot (`sections`).
pub fn shape(profile: &SportProfile, frames: &[Value]) -> Value {
    const PHASES: [&str; 3] = ["all", "att", "def"];
    let (l, w) = (profile.field.length, profile.field.width);
    let t0 = frames.first().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let t1 = frames.last().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let span = (t1 - t0).max(1e-6);
    let np = periods(profile.id.as_str());
    let nb = 48usize;
    #[derive(Default, Clone)]
    struct Acc {
        n: usize,
        w: f64,
        d: f64,
        cx: f64,
        cy: f64,
        slots: std::collections::BTreeMap<u64, (f64, f64, usize)>, // id -> (sum x, sum y, frames)
    }
    // bins[bin][team] -> (sum width, sum depth, frames); secs[section][team][phase]; frames per section and phase
    let mut bins = vec![[(0f64, 0f64, 0usize); 2]; nb];
    let mut secs = vec![[[Acc::default(), Acc::default(), Acc::default()], [Acc::default(), Acc::default(), Acc::default()]]; np];
    let mut sec_frames = vec![[[0usize; 3]; 2]; np];
    let mut poss: Option<usize> = None;
    for fr in frames {
        let t = fr["t"].as_f64().unwrap_or(t0);
        let u = ((t - t0) / span).clamp(0.0, 1.0);
        let (b, si) = (((u * nb as f64) as usize).min(nb - 1), ((u * np as f64) as usize).min(np - 1));
        if fr["ball"]["state"] == "held" {
            match fr["ball"]["holder"].as_u64().and_then(|h| team_of(fr, h)).as_deref() {
                Some("home") => poss = Some(0),
                Some("away") => poss = Some(1),
                _ => {}
            }
        }
        let mut pts: [Vec<(u64, f32, f32)>; 2] = [Vec::new(), Vec::new()];
        for p in fr["players"].as_array().into_iter().flatten() {
            if p["state"].as_str() != Some("visible") {
                continue;
            }
            let k = match p["team"].as_str() {
                Some("home") => 0,
                Some("away") => 1,
                _ => continue,
            };
            let (Some(x), Some(y)) = (f(p, "x"), f(p, "y")) else { continue };
            if x < -1.0 || y < -1.0 || x > l + 1.0 || y > w + 1.0 {
                continue;
            }
            pts[k].push((p["id"].as_u64().unwrap_or(u64::MAX), x, y));
        }
        for k in 0..2 {
            let phases: &[usize] = match poss {
                Some(q) if q == k => &[0, 1],
                Some(_) => &[0, 2],
                None => &[0],
            };
            for &ph in phases {
                sec_frames[si][k][ph] += 1;
            }
            let xy: Vec<(f32, f32)> = pts[k].iter().map(|p| (p.1, p.2)).collect();
            let spread = team_spread(&xy);
            if let Some((sw, sd, _, _)) = spread {
                let e = &mut bins[b][k];
                e.0 += sw as f64;
                e.1 += sd as f64;
                e.2 += 1;
            }
            for &ph in phases {
                let acc = &mut secs[si][k][ph];
                for &(id, x, y) in &pts[k] {
                    if id != u64::MAX {
                        let e = acc.slots.entry(id).or_insert((0.0, 0.0, 0));
                        e.0 += x as f64;
                        e.1 += y as f64;
                        e.2 += 1;
                    }
                }
                if let Some((sw, sd, cx, cy)) = spread {
                    acc.n += 1;
                    acc.w += sw as f64;
                    acc.d += sd as f64;
                    acc.cx += cx as f64;
                    acc.cy += cy as f64;
                }
            }
        }
    }
    let team = |e: (f64, f64, usize)| if e.2 == 0 { Value::Null } else { json!({"w": r2(e.0 / e.2 as f64), "d": r2(e.1 / e.2 as f64)}) };
    let series: Vec<Value> = bins
        .iter()
        .enumerate()
        .map(|(i, e)| json!({"t": r2(t0 + span * (i as f64 + 0.5) / nb as f64), "home": team(e[0]), "away": team(e[1])}))
        .collect();
    let side = |a: &Acc, frames: usize| {
        if a.n == 0 {
            return Value::Null;
        }
        let n = a.n as f64;
        let slots: Vec<Value> = a
            .slots
            .iter()
            .map(|(id, e)| json!({"id": id, "x": r2(e.0 / e.2 as f64), "y": r2(e.1 / e.2 as f64), "share": r2(e.2 as f64 / frames.max(1) as f64)}))
            .collect();
        json!({"w": r2(a.w / n), "d": r2(a.d / n), "cx": r2(a.cx / n), "cy": r2(a.cy / n), "frames": a.n, "slots": slots})
    };
    let sections: Vec<Value> = secs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut v = json!({"index": i + 1, "from": r2(t0 + span * i as f64 / np as f64), "to": r2(t0 + span * (i + 1) as f64 / np as f64)});
            for (k, name) in ["home", "away"].iter().enumerate() {
                v[*name] = json!(PHASES.iter().enumerate().map(|(ph, p)| (p.to_string(), side(&s[k][ph], sec_frames[i][k][ph]))).collect::<serde_json::Map<_, _>>());
            }
            v
        })
        .collect();
    let total = |k: usize| {
        PHASES
            .iter()
            .enumerate()
            .map(|(ph, p)| {
                let n: usize = secs.iter().map(|s| s[k][ph].n).sum();
                let v = if n == 0 { Value::Null } else { json!({"w": r2(secs.iter().map(|s| s[k][ph].w).sum::<f64>() / n as f64), "d": r2(secs.iter().map(|s| s[k][ph].d).sum::<f64>() / n as f64), "frames": n}) };
                (p.to_string(), v)
            })
            .collect::<serde_json::Map<_, _>>()
    };
    json!({"series": series, "sections": sections, "home": total(0), "away": total(1)})
}

/// The full statistics of a run as JSON (see the module documentation).
pub fn report(profile: &SportProfile, frames: &[Value]) -> Value {
    report_with(profile, frames, None)
}

/// End (`'l'` / `'r'`) an attempt went for; `None` if the sport has no two ends.
fn attempt_side(goals: usize, a: &Attempt) -> Option<char> {
    (goals == 2).then(|| if a.goal == 0 { 'l' } else { 'r' })
}

/// Playing direction for the attempts: the given one, or else an estimate from the attempts
/// themselves. A given or reliable direction re-assigns each attempt to the team attacking its end.
/// Returns the `direction` block of the report.
pub fn apply_direction(profile: &SportProfile, att: &mut [Attempt], given: Option<Direction>) -> Value {
    let ng = goals(profile).len();
    let team = |s: &str| match s {
        "home" => Team::Home,
        "away" => Team::Away,
        _ => Team::Unknown,
    };
    let obs: Vec<Obs> = att.iter().filter_map(|a| Some(Obs { t: a.t, team: team(&a.team), side: attempt_side(ng, a)? })).collect();
    let est = estimate(&obs);
    let (dir, source) = match (given, est) {
        (Some(d), _) => (Some(d), "given"),
        (None, Some(e)) if e.reliable() => (Some(e.dir), "estimated"),
        _ => (None, "unknown"),
    };
    let fits = |d: &Direction| obs.iter().filter(|o| d.attacker(o.side, o.t) == o.team).count();
    let mut reassigned = 0;
    if let Some(d) = dir {
        for a in att.iter_mut() {
            if let Some(side) = attempt_side(ng, a) {
                let k = if d.attacker(side, a.t) == Team::Home { "home" } else { "away" };
                if a.team != k {
                    a.team = k.into();
                    reassigned += 1;
                }
            }
        }
    }
    let shown = dir.or(est.map(|e| e.dir));
    json!({
        "source": source,
        "home_start": shown.map(|d| d.home_start.to_string()),
        "switch_at": shown.and_then(|d| d.switch_at).map(r2),
        "agree": shown.map(|d| fits(&d)).unwrap_or(0),
        "total": obs.len(),
        "reassigned": reassigned,
    })
}

/// [`report`] with a known playing direction (see [`apply_direction`]).
pub fn report_with(profile: &SportProfile, frames: &[Value], given: Option<Direction>) -> Value {
    let sport = profile.id.as_str();
    let (l, w) = (profile.field.length, profile.field.width);
    let n = frames.len();
    let t0 = frames.first().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let t1 = frames.last().and_then(|f| f["t"].as_f64()).unwrap_or(0.0);
    let mut att = attempts(profile, frames);
    let direction = apply_direction(profile, &mut att, given);
    let att = att;

    // possession (ball held by a player of a team), ball in thirds, position heat maps
    let nx = 24usize;
    let ny = ((nx as f32 * w / l).round() as usize).clamp(6, 24);
    let cell = |x: f32, y: f32| -> Option<usize> {
        if x < 0.0 || y < 0.0 || x > l || y > w {
            return None;
        }
        let (cx, cy) = (((x / l) * nx as f32) as usize, ((y / w) * ny as f32) as usize);
        Some(cy.min(ny - 1) * nx + cx.min(nx - 1))
    };
    let mut heat: HashMap<&str, Vec<u32>> = HashMap::new();
    for k in ["home", "away", "ball"] {
        heat.insert(k, vec![0; nx * ny]);
    }
    let (mut held_home, mut held_away, mut ball_seen) = (0usize, 0usize, 0usize);
    let mut thirds = [0usize; 3];
    for fr in frames {
        for p in fr["players"].as_array().into_iter().flatten() {
            if p["state"].as_str() != Some("visible") {
                continue;
            }
            let team = match p["team"].as_str() {
                Some("home") => "home",
                Some("away") => "away",
                _ => continue,
            };
            if let (Some(x), Some(y)) = (f(p, "x"), f(p, "y")) {
                if let Some(c) = cell(x, y) {
                    heat.get_mut(team).unwrap()[c] += 1;
                }
            }
        }
        let b = &fr["ball"];
        if let (Some(x), Some(y)) = (f(b, "x"), f(b, "y")) {
            ball_seen += 1;
            if let Some(c) = cell(x, y) {
                heat.get_mut("ball").unwrap()[c] += 1;
            }
            thirds[((x / l * 3.0).floor().clamp(0.0, 2.0)) as usize] += 1;
        }
        if b["state"] == "held" {
            match b["holder"].as_u64().and_then(|h| team_of(fr, h)).as_deref() {
                Some("home") => held_home += 1,
                Some("away") => held_away += 1,
                _ => {}
            }
        }
    }
    let held = (held_home + held_away).max(1) as f64;

    // attempts per zone and per time section
    let zone_list: Vec<Value> = zones(sport)
        .iter()
        .map(|z| {
            let c = |team: &str| att.iter().filter(|a| a.zone == *z && a.team == team).count();
            let all = att.iter().filter(|a| a.zone == *z).count();
            json!({"zone": z, "count": all, "home": c("home"), "away": c("away"), "share": if att.is_empty() { 0.0 } else { r2(all as f64 / att.len() as f64) }})
        })
        .collect();
    let np = periods(sport);
    let span = (t1 - t0).max(1e-6);
    let period_list: Vec<Value> = (0..np)
        .map(|i| {
            let (a, b) = (t0 + span * i as f64 / np as f64, t0 + span * (i + 1) as f64 / np as f64);
            let inside = |x: &&Attempt| x.t >= a && (x.t < b || (i + 1 == np && x.t <= b));
            let c = |team: &str| att.iter().filter(inside).filter(|x| x.team == team).count();
            json!({"index": i + 1, "from": r2(a), "to": r2(b), "home": c("home"), "away": c("away"), "none": c("none")})
        })
        .collect();

    // key moments: attempts, other situations, substitutions
    let mut moments: Vec<Value> = att.iter().map(|a| json!({"t": r2(a.t), "type": "attempt", "kind": a.kind, "team": a.team, "zone": a.zone})).collect();
    for fr in frames {
        for e in fr["events"].as_array().into_iter().flatten() {
            let t = e["t"].as_f64().unwrap_or(0.0);
            match e["type"].as_str() {
                Some("scene") if !att.iter().any(|a| (a.t - t).abs() < 0.01 && a.kind == e["kind"].as_str().unwrap_or("")) => {
                    moments.push(json!({"t": r2(t), "type": "scene", "kind": e["kind"], "team": e["team"]}));
                }
                Some("sub") if e["kind"] == "confirmed" => {
                    moments.push(json!({"t": r2(t), "type": "sub", "team": e["team"], "out": e["out"], "in": e["in"]}));
                }
                _ => {}
            }
        }
    }
    moments.sort_by(|a, b| a["t"].as_f64().partial_cmp(&b["t"].as_f64()).unwrap());

    let count = |team: &str| att.iter().filter(|a| a.team == team).count();
    let by_kind = |team: &str| {
        let mut m: std::collections::BTreeMap<String, usize> = Default::default();
        for a in att.iter().filter(|a| a.team == team) {
            *m.entry(a.kind.clone()).or_default() += 1;
        }
        json!(m)
    };
    let attempts_json: Vec<Value> = att
        .iter()
        .map(|a| json!({"t": r2(a.t), "team": a.team, "x": r2(a.pos.x as f64), "y": r2(a.pos.y as f64), "kind": a.kind, "zone": a.zone, "goal": a.goal, "distance": r2(a.distance as f64), "confidence": r2(a.confidence as f64)}))
        .collect();
    json!({
        "reco_minimap_stats": 1,
        "sport": sport,
        "field": {"length": l, "width": w},
        "lines": profile.lines.iter().map(|pl| pl.iter().map(|p| [r2(p.x as f64), r2(p.y as f64)]).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "goals": goals(profile).iter().map(|g| [r2(g.x as f64), r2(g.y as f64)]).collect::<Vec<_>>(),
        "frames": n, "t0": r2(t0), "t1": r2(t1),
        "has_attempts": sport == "basketball" || shot_rules(sport).is_some(),
        "attempts": attempts_json,
        "totals": {"home": count("home"), "away": count("away"), "none": count("none"), "home_kinds": by_kind("home"), "away_kinds": by_kind("away")},
        "zones": zone_list,
        "periods": period_list,
        "possession": {"home": r2(held_home as f64 / held), "away": r2(held_away as f64 / held), "frames_held": held_home + held_away, "ball_seen": if n == 0 { 0.0 } else { r2(ball_seen as f64 / n as f64) }},
        "thirds": thirds.iter().map(|c| r2(*c as f64 / ball_seen.max(1) as f64)).collect::<Vec<_>>(),
        "heat": {"nx": nx, "ny": ny, "home": heat["home"], "away": heat["away"], "ball": heat["ball"]},
        "shape": shape(profile, frames),
        "direction": direction,
        "moments": moments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f64, ball: Value, players: Value, events: Value) -> Value {
        json!({"t": t, "players": players, "referees": [], "ball": ball, "events": events})
    }

    #[test]
    fn a_fast_flight_towards_a_goal_is_a_football_attempt_of_the_holding_team() {
        let p = SportProfile::football();
        let pl = json!([{"id": 3, "team": "away", "x": 85.0, "y": 30.0, "state": "visible"}]);
        let frames = vec![
            frame(10.0, json!({"state": "held", "holder": 3, "x": 85.0, "y": 30.0}), pl.clone(), json!([])),
            frame(10.2, json!({"state": "flight", "x": 86.0, "y": 31.0, "vx": 20.0, "vy": 3.0}), pl.clone(), json!([])),
            frame(10.4, json!({"state": "flight", "x": 90.0, "y": 31.6, "vx": 20.0, "vy": 3.0}), pl.clone(), json!([])),
            // a slow pass the other way is not a shot
            frame(20.0, json!({"state": "held", "holder": 3, "x": 60.0, "y": 30.0}), pl.clone(), json!([])),
            frame(20.2, json!({"state": "flight", "x": 59.0, "y": 30.0, "vx": -6.0, "vy": 0.0}), pl.clone(), json!([])),
        ];
        let a = attempts(&p, &frames);
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].team, "away");
        assert_eq!(a[0].goal, 1);
        assert_eq!(a[0].zone, "outside_box");
        let r = report(&p, &frames);
        assert_eq!(r["totals"]["away"], 1);
        assert_eq!(r["zones"][2]["count"], 1);
        assert_eq!(r["possession"]["away"], 1.0);
        assert_eq!(r["periods"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn basketball_attempts_come_from_scene_events_and_get_zones() {
        let p = SportProfile::basketball();
        let ev = |kind: &str, x: f64, y: f64| json!([{"type": "scene", "phase": "attempt", "kind": kind, "team": "home", "side": "r", "x": x, "y": y, "confidence": 0.8, "t": 1.0}]);
        let frames = vec![
            frame(1.0, json!({"state": "flight"}), json!([]), ev("shot3", 27.5, 0.8)),
            frame(2.0, json!({"state": "flight"}), json!([]), ev("shot2", 26.0, 7.5)),
            frame(3.0, json!({"state": "flight"}), json!([]), ev("shot3", 19.0, 7.5)),
            frame(4.0, json!({"state": "flight"}), json!([]), ev("free_throw", 22.2, 7.5)),
            frame(5.0, json!({"state": "flight"}), json!([]), json!([{"type": "scene", "phase": "formation", "kind": "tip_off", "team": "none", "x": 14.0, "y": 7.5, "t": 5.0}])),
        ];
        let z: Vec<&str> = attempts(&p, &frames).iter().map(|a| a.zone).collect();
        assert_eq!(z, ["corner3", "restricted", "above3", "free_throw"]);
        let r = report(&p, &frames);
        assert_eq!(r["totals"]["home"], 4);
        assert_eq!(r["moments"].as_array().unwrap().len(), 5, "the tip-off is a key moment too");
    }

    #[test]
    fn sports_without_goals_still_get_possession_and_heat() {
        let p = SportProfile::volleyball();
        let frames = vec![frame(0.0, json!({"state": "loose", "x": 4.0, "y": 4.0}), json!([{"id": 1, "team": "home", "x": 3.0, "y": 3.0, "state": "visible"}]), json!([]))];
        let r = report(&p, &frames);
        assert_eq!(r["has_attempts"], false);
        assert_eq!(r["heat"]["home"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).sum::<u64>(), 1);
    }

    #[test]
    fn team_shape_measures_width_depth_and_average_slots() {
        let p = SportProfile::basketball();
        let pl = |dx: f64| json!([
            {"id": 0, "team": "home", "x": 5.0 + dx, "y": 3.0, "state": "visible"},
            {"id": 1, "team": "home", "x": 9.0 + dx, "y": 7.5, "state": "visible"},
            {"id": 2, "team": "home", "x": 5.0 + dx, "y": 12.0, "state": "visible"},
            {"id": 5, "team": "away", "x": 20.0, "y": 7.0, "state": "visible"},
            {"id": 6, "team": "away", "x": 22.0, "y": 8.0, "state": "hidden"}
        ]);
        // home holds the ball for the first 30 frames, then the away team (slot 5)
        let ball = |i: i32| if i < 30 { json!({"state": "held", "holder": 1}) } else { json!({"state": "held", "holder": 5}) };
        let frames: Vec<Value> = (0..40).map(|i| frame(i as f64, ball(i), pl(if i < 20 { 0.0 } else { 10.0 }), json!([]))).collect();
        let s = shape(&p, &frames);
        assert_eq!(s["home"]["all"]["w"], 9.0);
        assert_eq!(s["home"]["all"]["d"], 4.0);
        assert_eq!(s["home"]["att"]["frames"], 30);
        assert_eq!(s["home"]["def"]["frames"], 10);
        assert!(s["away"]["all"].is_null(), "one visible player is not a shape");
        let secs = s["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 4);
        assert_eq!(secs[0]["home"]["all"]["slots"][1]["x"], 9.0);
        assert_eq!(secs[3]["home"]["all"]["slots"][1]["x"], 19.0);
        assert_eq!(secs[3]["home"]["all"]["cx"], 16.33);
        assert!(secs[3]["home"]["att"].is_null(), "home has no ball in the last section");
        assert_eq!(secs[3]["home"]["def"]["slots"][0]["share"], 1.0);
        assert_eq!(s["series"].as_array().unwrap().len(), 48);
        // with eleven players the goalkeeper does not stretch the depth
        let mut pts: Vec<(f32, f32)> = (0..10).map(|i| (40.0 + (i % 4) as f32 * 5.0, 5.0 + i as f32 * 6.0)).collect();
        pts.push((2.0, 34.0));
        assert_eq!(team_spread(&pts).unwrap().1, 15.0);
    }

    #[test]
    fn playing_direction_reassigns_attempts() {
        let p = SportProfile::basketball();
        let shot = |t: f64, team: &str, side: &str| frame(t, json!({"state": "flight"}), json!([]), json!([{"type": "scene", "phase": "attempt", "kind": "shot2", "team": team, "side": side, "x": if side == "l" { 4.0 } else { 24.0 }, "y": 7.5, "confidence": 0.6, "t": t}]));
        // home attacks right, away left; one shot has the wrong team colour
        let mut frames: Vec<Value> = (0..8).map(|i| shot(i as f64 * 10.0, if i % 2 == 0 { "home" } else { "away" }, if i % 2 == 0 { "r" } else { "l" })).collect();
        frames.push(shot(90.0, "away", "r"));
        let r = report(&p, &frames);
        assert_eq!(r["direction"]["source"], "estimated");
        assert_eq!(r["direction"]["home_start"], "r");
        assert_eq!(r["direction"]["reassigned"], 1);
        assert_eq!(r["totals"]["home"], 5);
        // given the other way round, everything flips
        let r = report_with(&p, &frames, Direction::new('l', None));
        assert_eq!(r["direction"]["source"], "given");
        assert_eq!(r["totals"]["home"], 4);
        // too few attempts: shown, but nothing re-assigned
        let r = report(&p, &frames[..3]);
        assert_eq!(r["direction"]["source"], "unknown");
        assert_eq!(r["direction"]["reassigned"], 0);
    }
}
