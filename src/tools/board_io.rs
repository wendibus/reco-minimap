//! Reading and writing tactics-board scenes (JSON) and cutting one out of a recorded run (`run --json`).

use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::{json, Value};

use crate::board::{BoardBall, BoardPlayer, BoardScene, FORMAT, VERSION};
use crate::geom::{Field, Vec2};
use crate::sport::SportProfile;
use crate::team::Team;
use crate::tools::colors::{hex, parse_hex};
use crate::{tr, trs};

fn r2(v: f32) -> f64 {
    ((v as f64) * 100.0).round() / 100.0
}

fn pt(p: Vec2) -> Value {
    json!([r2(p.x), r2(p.y)])
}

pub fn scene_to_json(s: &BoardScene) -> Value {
    let team = |t: Team| if t == Team::Home { "home" } else { "away" };
    json!({
        "format": FORMAT, "version": VERSION, "sport": s.sport, "t": (s.t * 1000.0).round() / 1000.0,
        "field": {"length": s.field.length, "width": s.field.width},
        "teams": {
            "home": {"color": hex(s.home), "keeper_color": s.home_keeper.map(hex)},
            "away": {"color": hex(s.away), "keeper_color": s.away_keeper.map(hex)},
        },
        "players": s.players.iter().map(|p| json!({
            "id": p.id, "team": team(p.team), "number": p.number, "keeper": p.keeper, "x": r2(p.pos.x), "y": r2(p.pos.y),
            "trail": p.trail.iter().map(|q| pt(*q)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "referees": s.referees.iter().map(|p| json!({"x": r2(p.x), "y": r2(p.y)})).collect::<Vec<_>>(),
        "ball": {"state": s.ball.state, "holder": s.ball.holder, "x": s.ball.pos.map(|p| r2(p.x)), "y": s.ball.pos.map(|p| r2(p.y))},
        "lines": s.lines.iter().map(|l| l.iter().map(|p| pt(*p)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "goals": s.goals.iter().map(|p| pt(*p)).collect::<Vec<_>>(),
    })
}

fn num(v: &Value) -> Option<f32> {
    v.as_f64().map(|f| f as f32)
}

fn point(v: &Value) -> Option<Vec2> {
    let a = v.as_array()?;
    Some(Vec2::new(num(a.first()?)?, num(a.get(1)?)?))
}

pub fn scene_from_json(v: &Value) -> Result<BoardScene, String> {
    if v["format"].as_str() != Some(FORMAT) {
        return Err(trs!("keine Taktikboard-Szene (\"format\" fehlt)", "not a tactics-board scene (\"format\" is missing)", "pas une scène de tableau tactique (« format » manquant)", "no es una escena de pizarra táctica (falta \"format\")").into());
    }
    let ver = v["version"].as_u64().unwrap_or(0) as u32;
    if ver == 0 || ver > VERSION {
        return Err(tr!("Szenen-Version {ver} wird nicht unterstützt (dieses Programm kennt bis {VERSION})", "scene version {ver} is not supported (this program knows up to {VERSION})", "la version de scène {ver} n'est pas prise en charge (ce programme connaît jusqu'à {VERSION})", "no se admite la versión de escena {ver} (este programa conoce hasta {VERSION})"));
    }
    let colour = |x: &Value| x.as_str().and_then(parse_hex);
    let team = |x: &Value| if x.as_str() == Some("home") { Team::Home } else { Team::Away };
    let field = Field::new(num(&v["field"]["length"]).ok_or("field.length")?, num(&v["field"]["width"]).ok_or("field.width")?);
    let player = |p: &Value| -> Option<BoardPlayer> {
        Some(BoardPlayer {
            id: p["id"].as_u64()? as u32,
            team: team(&p["team"]),
            number: p["number"].as_u64().map(|n| n as u8),
            keeper: p["keeper"].as_bool().unwrap_or(false),
            pos: Vec2::new(num(&p["x"])?, num(&p["y"])?),
            trail: p["trail"].as_array().map(|t| t.iter().filter_map(point).collect()).unwrap_or_default(),
        })
    };
    let players: Vec<BoardPlayer> = v["players"].as_array().map(|a| a.iter().filter_map(player).collect()).unwrap_or_default();
    let b = &v["ball"];
    Ok(BoardScene {
        sport: v["sport"].as_str().unwrap_or("custom").to_string(),
        field,
        t: v["t"].as_f64().unwrap_or(0.0),
        home: colour(&v["teams"]["home"]["color"]).unwrap_or([220, 40, 40]),
        away: colour(&v["teams"]["away"]["color"]).unwrap_or([40, 110, 230]),
        home_keeper: colour(&v["teams"]["home"]["keeper_color"]),
        away_keeper: colour(&v["teams"]["away"]["keeper_color"]),
        players,
        referees: v["referees"].as_array().map(|a| a.iter().filter_map(|p| Some(Vec2::new(num(&p["x"])?, num(&p["y"])?))).collect()).unwrap_or_default(),
        ball: BoardBall {
            state: b["state"].as_str().unwrap_or("unknown").to_string(),
            holder: b["holder"].as_u64().map(|h| h as u32),
            pos: match (num(&b["x"]), num(&b["y"])) { (Some(x), Some(y)) => Some(Vec2::new(x, y)), _ => None },
        },
        lines: v["lines"].as_array().map(|a| a.iter().map(|l| l.as_array().map(|p| p.iter().filter_map(point).collect()).unwrap_or_default()).collect()).unwrap_or_default(),
        goals: v["goals"].as_array().map(|a| a.iter().filter_map(point).collect()).unwrap_or_default(),
    })
}

/// The `"t"` of a state line without parsing the whole (long) line.
fn quick_t(line: &str) -> Option<f64> {
    let rest = line.strip_prefix("{\"t\":")?;
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+'))?;
    rest[..end].parse().ok()
}

/// Colours and stripe options for a scene cut from a run.
#[derive(Clone, Copy, Debug)]
pub struct SceneColours {
    pub home: [u8; 3],
    pub away: [u8; 3],
    pub home_keeper: Option<[u8; 3]>,
    pub away_keeper: Option<[u8; 3]>,
}

/// Cut the moment nearest to `at` (seconds) out of a states file written by `run --json`, with the last `trail_secs`
/// of movement. Errors if the file holds no state lines.
pub fn scene_from_states(path: &Path, at: f64, trail_secs: f64, profile: &SportProfile, colours: SceneColours) -> Result<BoardScene, String> {
    let open = || std::fs::File::open(path).map(BufReader::new).map_err(|e| format!("{}: {e}", path.display()));
    // pass 1: the frame time nearest to `at`
    let mut best: Option<(f64, f64)> = None;
    for line in open()?.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(t) = quick_t(&line) {
            let d = (t - at).abs();
            if best.map(|b| d < b.1).unwrap_or(true) {
                best = Some((t, d));
            }
        }
    }
    let (t0, dist) = best.ok_or_else(|| tr!("{}: keine Zustandszeilen (erwartet: Ausgabe von `run --json`)", "{}: no state lines (expected the output of `run --json`)", "{} : aucune ligne d'état (attendu : sortie de `run --json`)", "{}: sin líneas de estado (se espera la salida de `run --json`)", path.display()))?;
    if dist > 5.0 {
        return Err(tr!("zu {at:.1} s gibt es nichts in der Datei (nächster Zeitpunkt {t0:.1} s)", "nothing at {at:.1} s in the file (nearest time {t0:.1} s)", "rien à {at:.1} s dans le fichier (instant le plus proche : {t0:.1} s)", "no hay nada en {at:.1} s en el archivo (instante más cercano: {t0:.1} s)"));
    }
    // pass 2: the frame itself and the frames of the trail window
    let mut frames: Vec<(f64, Value)> = Vec::new();
    for line in open()?.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(t) = quick_t(&line) {
            if t <= t0 + 1e-9 && t >= t0 - trail_secs - 1e-9 {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    frames.push((t, v));
                }
            }
        }
    }
    let (_, last) = frames.iter().find(|(t, _)| (*t - t0).abs() < 1e-9).ok_or("frame")?.clone();
    let mut players = Vec::new();
    for p in last["players"].as_array().into_iter().flatten() {
        let team = match p["team"].as_str() { Some("home") => Team::Home, Some("away") => Team::Away, _ => continue };
        if p["state"].as_str() == Some("lost") {
            continue;
        }
        let (id, segment) = (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0));
        let pos = Vec2::new(num(&p["x"]).unwrap_or(0.0), num(&p["y"]).unwrap_or(0.0));
        // trail: same slot and segment, at most one point per 0.25 s, current position excluded
        let mut trail: Vec<Vec2> = Vec::new();
        let mut last_t = f64::NEG_INFINITY;
        for (t, f) in frames.iter().filter(|(t, _)| *t < t0 - 1e-9) {
            if t - last_t < 0.25 {
                continue;
            }
            if let Some(q) = f["players"].as_array().and_then(|a| a.iter().find(|q| q["id"].as_u64() == Some(id) && q["segment"].as_u64() == Some(segment))) {
                if let (Some(x), Some(y)) = (num(&q["x"]), num(&q["y"])) {
                    trail.push(Vec2::new(x, y));
                    last_t = *t;
                }
            }
        }
        players.push(BoardPlayer { id: id as u32, team, number: p["jersey"].as_u64().map(|n| n as u8), keeper: p["keeper"].as_bool().unwrap_or(false), pos, trail });
    }
    let referees = last["referees"].as_array().map(|a| a.iter().filter_map(|p| Some(Vec2::new(num(&p["x"])?, num(&p["y"])?))).collect()).unwrap_or_default();
    let b = &last["ball"];
    let ball = BoardBall {
        state: b["state"].as_str().unwrap_or("unknown").to_string(),
        holder: b["holder"].as_u64().map(|h| h as u32),
        pos: match (num(&b["x"]), num(&b["y"])) { (Some(x), Some(y)) => Some(Vec2::new(x, y)), _ => None },
    };
    let keepers = crate::sport::has_keepers(&profile.id);
    Ok(BoardScene {
        sport: profile.id.clone(),
        field: profile.field,
        t: t0,
        home: colours.home,
        away: colours.away,
        home_keeper: if keepers { colours.home_keeper } else { None },
        away_keeper: if keepers { colours.away_keeper } else { None },
        players,
        referees,
        ball,
        lines: profile.lines.clone(),
        goals: profile.goals.clone(),
    })
}
