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

fn key(p: &Value, team: &str) -> (u64, u64, String) {
    (p["id"].as_u64().unwrap_or(0), p["segment"].as_u64().unwrap_or(0), team.to_string())
}

/// Rewrite a result file (`run --json`) with the propagated identities.
pub fn backfill_file(path: &Path) -> Result<usize, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut frames: Vec<Value> = txt.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let n = backfill(&mut frames);
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
    fn too_little_evidence_changes_nothing() {
        let mut f: Vec<Value> = (0..10).map(|i| fr(i as f64, "unknown", None)).collect();
        f[3] = fr(3.0, "candidate", Some(34));
        assert_eq!(backfill(&mut f), 0);
    }
}
