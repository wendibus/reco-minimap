//! Offline identity propagation for a processed video: a number read (or deduced) somewhere on a
//! continuous tracklet (`id` + `segment`) also holds for the rest of it – before the first reading
//! as well as after it. Live this is impossible (the past is already published); for a video the
//! result file is rewritten once at the end of the run.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

/// Fill unknown identities along tracklets. Returns how many player entries were filled.
pub fn backfill(frames: &mut [Value]) -> usize {
    // evidence per tracklet: read frames count fully, deduced ones half
    let mut ev: HashMap<(u64, u64, String), HashMap<u64, f64>> = HashMap::new();
    for fr in frames.iter() {
        for p in fr["players"].as_array().into_iter().flatten() {
            let w = match p["identity"]["status"].as_str() {
                Some("candidate") | Some("manual") => 1.0,
                Some("inferred") if p["identity"]["source"] != "tracklet" => 0.5,
                _ => continue,
            };
            let (Some(n), Some(team)) = (p["identity"]["player"]["number"].as_u64(), p["team"].as_str()) else { continue };
            *ev.entry(key(p, team)).or_default().entry(n).or_default() += w;
        }
    }
    let best: HashMap<(u64, u64, String), u64> = ev
        .into_iter()
        .filter_map(|(k, m)| {
            let total: f64 = m.values().sum();
            let (n, w) = m.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))?;
            (w >= 3.0 && w / total >= 0.6).then_some((k, n))
        })
        .collect();
    let mut filled = 0;
    for fr in frames.iter_mut() {
        let Some(players) = fr["players"].as_array_mut() else { continue };
        let taken: Vec<(String, u64)> = players
            .iter()
            .filter(|p| matches!(p["identity"]["status"].as_str(), Some("candidate" | "inferred" | "manual" | "conflict")))
            .filter_map(|p| Some((p["team"].as_str()?.to_string(), p["identity"]["player"]["number"].as_u64()?)))
            .collect();
        let mut added: Vec<(String, u64)> = Vec::new();
        for p in players.iter_mut() {
            if p["identity"]["status"].as_str().unwrap_or("unknown") != "unknown" || p["state"] == "lost" {
                continue;
            }
            let Some(team) = p["team"].as_str().map(str::to_string).filter(|t| t == "home" || t == "away") else { continue };
            let Some(&n) = best.get(&key(p, &team)) else { continue };
            let slot = (team.clone(), n);
            if taken.contains(&slot) || added.contains(&slot) {
                continue; // someone else carries that number in this frame
            }
            p["identity"] = json!({"status": "inferred", "player": {"team": team, "number": n}, "source": "tracklet"});
            p["jersey"] = json!(n);
            added.push(slot);
            filled += 1;
        }
    }
    filled
}

/// Seconds around a tracklet in which its team's line-up is collected.
const LINEUP_WINDOW: f64 = 30.0;

/// Elimination on the court: a team has at most [`ON_COURT`] players on the court. A tracklet
/// without a number that runs alongside the other known players of its team is the one player of
/// the current line-up who is not seen elsewhere at the same time. Returns how many entries were
/// filled (`source: "lineup"`).
pub fn lineup_fill(frames: &mut [Value], on_court_max: usize, length: f64, width: f64) -> usize {
    let on_court = |p: &Value| {
        let (x, y) = (p["x"].as_f64().unwrap_or(-1.0), p["y"].as_f64().unwrap_or(-1.0));
        p["state"] != "lost" && x > 0.2 && x < length - 0.2 && y > 0.3 && y < width - 0.3
    };
    let mut total = 0;
    // a filled tracklet can make the next one unique: a few rounds
    for _ in 0..3 {
        // per tracklet: team, the frames it is on the court in, and its number if known
        struct Tl {
            team: String,
            frames: Vec<usize>,
            number: Option<u64>,
        }
        let mut tls: HashMap<(u64, u64), Tl> = HashMap::new();
        for (i, fr) in frames.iter().enumerate() {
            for p in fr["players"].as_array().into_iter().flatten().filter(|p| on_court(p)) {
                let Some(team) = p["team"].as_str().filter(|t| *t == "home" || *t == "away") else { continue };
                let e = tls.entry((p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0))).or_insert(Tl { team: team.to_string(), frames: Vec::new(), number: None });
                e.frames.push(i);
                if e.number.is_none() {
                    e.number = p["jersey"].as_u64();
                }
            }
        }
        let t_of = |i: usize| frames[i]["t"].as_f64().unwrap_or(0.0);
        // who of a team is on the court in a frame: number or None
        let mut per_frame: HashMap<(usize, String), Vec<((u64, u64), Option<u64>)>> = HashMap::new();
        for (k, tl) in &tls {
            for &i in &tl.frames {
                per_frame.entry((i, tl.team.clone())).or_default().push((*k, tl.number));
            }
        }
        let mut assign: Vec<((u64, u64), u64)> = Vec::new();
        for (k, tl) in tls.iter().filter(|(_, tl)| tl.number.is_none() && tl.frames.len() >= 4) {
            let (a, b) = (t_of(tl.frames[0]), t_of(*tl.frames.last().unwrap()));
            // the line-up: numbers of that team on the court around it (a few seconds at least)
            let mut seen: HashMap<u64, usize> = HashMap::new();
            for o in tls.values().filter(|o| o.team == tl.team && o.number.is_some()) {
                let n = o.frames.iter().filter(|&&i| (a - LINEUP_WINDOW..=b + LINEUP_WINDOW).contains(&t_of(i))).count();
                *seen.entry(o.number.unwrap()).or_default() += n;
            }
            let lineup: Vec<u64> = seen.into_iter().filter(|(_, n)| *n >= 6).map(|(n, _)| n).collect();
            if lineup.len() > on_court_max {
                continue; // a substitution in the window: not unique
            }
            // frames where the team count fits (no ghost tracks), and who else is there
            let mut fits = 0usize;
            let mut beside: HashMap<u64, usize> = HashMap::new();
            for &i in &tl.frames {
                let here = &per_frame[&(i, tl.team.clone())];
                if here.len() <= on_court_max {
                    fits += 1;
                }
                for (ok, n) in here {
                    if ok != k {
                        if let Some(n) = n {
                            *beside.entry(*n).or_default() += 1;
                        }
                    }
                }
            }
            if fits * 10 < tl.frames.len() * 8 {
                continue;
            }
            let half = tl.frames.len() / 2;
            let left: Vec<u64> = lineup.iter().copied().filter(|n| beside.get(n).copied().unwrap_or(0) <= half / 2).collect();
            // everybody else must be accounted for: 4 known besides it, one left
            let known_beside = lineup.iter().filter(|n| beside.get(n).copied().unwrap_or(0) > half).count();
            if left.len() == 1 && lineup.len() == on_court_max && known_beside == on_court_max - 1 {
                assign.push((*k, left[0]));
            }
        }
        if assign.is_empty() {
            break;
        }
        let map: HashMap<(u64, u64), u64> = assign.into_iter().collect();
        for fr in frames.iter_mut() {
            let Some(players) = fr["players"].as_array_mut() else { continue };
            let taken: Vec<(String, u64)> = players.iter().filter_map(|p| Some((p["team"].as_str()?.to_string(), p["jersey"].as_u64()?))).collect();
            for p in players.iter_mut() {
                let k = (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0));
                let Some(&n) = map.get(&k) else { continue };
                let Some(team) = p["team"].as_str().map(str::to_string) else { continue };
                if p["jersey"].as_u64().is_some() || taken.contains(&(team.clone(), n)) {
                    continue;
                }
                p["identity"] = json!({"status": "inferred", "player": {"team": team, "number": n}, "source": "lineup"});
                p["jersey"] = json!(n);
                total += 1;
            }
        }
    }
    total
}

fn key(p: &Value, team: &str) -> (u64, u64, String) {
    (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0), team.to_string())
}

/// Rewrite a result file (`run --json`) with the propagated identities.
/// `lineup`: (players per team on the court, field length, width) for the elimination
/// ([`lineup_fill`]) after the propagation along tracklets.
pub fn backfill_file(path: &Path, lineup: Option<(usize, f64, f64)>) -> Result<usize, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut frames: Vec<Value> = txt.lines().enumerate().filter(|(_,l)|!l.trim().is_empty())
        .map(|(i,l)|serde_json::from_str(l).map_err(|e|format!("{}:{}: {e}",path.display(),i+1)))
        .collect::<Result<_,_>>()?;
    let mut n = backfill(&mut frames);
    if let Some((k, l, w)) = lineup {
        n += lineup_fill(&mut frames, k, l, w);
    }
    if n > 0 {
        let mut out = String::with_capacity(txt.len() + n * 64);
        for f in &frames {
            out.push_str(&f.to_string());
            out.push('\n');
        }
        let tmp = path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(t: f64, status: &str, num: Option<u64>) -> Value {
        json!({"t": t, "players": [
            {"id": 3, "segment": 1, "team": "home", "state": "visible", "jersey": num, "identity": {"status": status, "player": num.map(|n| json!({"team": "home", "number": n}))}},
            {"id": 4, "segment": 0, "team": "home", "state": "visible", "jersey": null, "identity": {"status": "unknown", "player": null}}
        ]})
    }

    #[test]
    fn number_runs_back_and_forth_along_the_tracklet() {
        let mut f: Vec<Value> = (0..10).map(|i| fr(i as f64, "unknown", None)).collect();
        for (i, item) in f.iter_mut().enumerate().take(8).skip(4) {
            *item = fr(i as f64, "candidate", Some(34));
        }
        assert_eq!(backfill(&mut f), 6);
        assert_eq!(f[0]["players"][0]["jersey"], 34);
        assert_eq!(f[0]["players"][0]["identity"]["status"], "inferred");
        assert_eq!(f[0]["players"][0]["identity"]["source"], "tracklet");
        assert_eq!(f[9]["players"][0]["jersey"], 34);
        assert_eq!(f[0]["players"][1]["jersey"], Value::Null); // other tracklet untouched
    }

    #[test]
    fn malformed_input_is_never_silently_rewritten() {
        let path=std::env::temp_dir().join(format!("reco-backfill-invalid-{}.jsonl",std::process::id()));
        let original="{\"t\":0}\ninvalid JSON\n";
        std::fs::write(&path,original).unwrap();
        assert!(backfill_file(&path, None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(),original);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_fifth_player_on_the_court_is_found_by_elimination() {
        let pl = |id: u64, x: f64, n: Option<u64>| json!({"id": id, "segment": 0, "team": "home", "state": "visible", "x": x, "y": 7.0, "jersey": n, "identity": {"status": if n.is_some() { "candidate" } else { "unknown" }, "player": n.map(|n| json!({"team": "home", "number": n}))}});
        let mut f: Vec<Value> = (0..12).map(|i| json!({"t": i as f64, "players": [pl(1, 3.0, Some(4)), pl(2, 6.0, Some(6)), pl(3, 9.0, Some(10)), pl(4, 12.0, Some(13)), pl(5, 15.0, None)]})).collect();
        // earlier the fifth number was seen on another tracklet of the line-up
        for x in f.iter_mut().take(8) {
            x["players"].as_array_mut().unwrap().push(pl(9, 20.0, Some(15)));
            x["players"].as_array_mut().unwrap().remove(4);
        }
        assert!(lineup_fill(&mut f, 5, 28.0, 15.0) > 0);
        assert_eq!(f[10]["players"][4]["jersey"], 15);
        assert_eq!(f[10]["players"][4]["identity"]["source"], "lineup");
        // six numbers around: a substitution, nothing is guessed
        let mut g = f.clone();
        for x in g.iter_mut() {
            for p in x["players"].as_array_mut().unwrap() {
                if p["identity"]["source"] == "lineup" { p["jersey"] = Value::Null; p["identity"] = json!({"status": "unknown", "player": null}); }
            }
        }
        for x in g.iter_mut().take(8) {
            x["players"].as_array_mut().unwrap().push(pl(11, 24.0, Some(9)));
        }
        assert_eq!(lineup_fill(&mut g, 5, 28.0, 15.0), 0);
    }

    #[test]
    fn too_little_evidence_changes_nothing() {
        let mut f: Vec<Value> = (0..10).map(|i| fr(i as f64, "unknown", None)).collect();
        f[3] = fr(3.0, "candidate", Some(34));
        assert_eq!(backfill(&mut f), 0);
    }
}
