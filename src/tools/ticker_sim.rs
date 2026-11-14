//! Live ticker from a processed video: the same feed the live hub would have published, written as
//! `liveticker.json` (format of the hub's `/ticker.json`, readable by `ticker.html` / `ticker.js`), plus a
//! `simulation` block that says how the ticker would have done: how many entries, of which kind, how long
//! the longest silence was. Used by `run --ticker-json`, `reco-minimap ticker <result.jsonl>` and the interface.

use crate::{tr, trs};
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use super::live::{Hub, TickerCfg};

pub struct TickerSim {
    hub: Arc<Hub>,
    frames: usize,
    t_first: Option<f64>,
    t_last: f64,
    times: Vec<f64>,
    subs_confirmed: usize,
    subs_possible: usize,
    scenes: std::collections::BTreeMap<String, usize>,
}

impl TickerSim {
    /// `cfg.approve` is ignored: a simulation shows everything the detection would have sent.
    pub fn new(mut cfg: TickerCfg) -> Self {
        cfg.approve = false;
        Self { hub: Hub::with_ticker("{}".into(), Some(cfg)), frames: 0, t_first: None, t_last: 0.0, times: Vec::new(), subs_confirmed: 0, subs_possible: 0, scenes: Default::default() }
    }

    /// One frame of a result (`{"t": …, "events": [...]}`, as `run --json` writes it).
    pub fn add_frame(&mut self, frame: &Value) {
        let t = frame["t"].as_f64().unwrap_or(self.t_last);
        self.frames += 1;
        self.t_first.get_or_insert(t);
        self.t_last = self.t_last.max(t);
        let Some(events) = frame["events"].as_array().filter(|e| !e.is_empty()) else { return };
        for e in events {
            self.times.push(e["t"].as_f64().unwrap_or(t));
            match (e["type"].as_str(), e["kind"].as_str()) {
                (Some("sub"), Some("confirmed")) => self.subs_confirmed += 1,
                (Some("sub"), _) => self.subs_possible += 1,
                (Some("scene"), Some(k)) => *self.scenes.entry(k.to_string()).or_default() += 1,
                _ => {}
            }
        }
        self.hub.push_events(events);
    }

    pub fn entries(&self) -> usize {
        self.times.len()
    }

    /// Video seconds covered by the result.
    pub fn duration(&self) -> f64 {
        (self.t_last - self.t_first.unwrap_or(0.0)).max(0.0)
    }

    /// Longest stretch without an entry (from the start, between entries, up to the end) and where it began.
    pub fn longest_gap(&self) -> (f64, f64) {
        let start = self.t_first.unwrap_or(0.0);
        let mut ts = self.times.clone();
        ts.sort_by(|a, b| a.total_cmp(b));
        let mut prev = start;
        let mut best = (0.0, start);
        for t in ts.into_iter().chain(std::iter::once(self.t_last)) {
            if t - prev > best.0 {
                best = (t - prev, prev);
            }
            prev = prev.max(t);
        }
        best
    }

    pub fn summary(&self, source: &str) -> Value {
        let d = self.duration();
        let (gap, gap_at) = self.longest_gap();
        let round = |v: f64| (v * 10.0).round() / 10.0;
        json!({
            "source": source, "frames": self.frames, "start": round(self.t_first.unwrap_or(0.0)), "duration": round(d),
            "entries": self.entries(), "per_10min": if d > 0.0 { round(self.entries() as f64 * 600.0 / d) } else { 0.0 },
            "substitutions": {"confirmed": self.subs_confirmed, "possible": self.subs_possible}, "scenes": self.scenes,
            "first_at": self.times.iter().copied().reduce(f64::min).map(round), "longest_gap": round(gap), "longest_gap_at": round(gap_at),
        })
    }

    /// The feed (like the hub's `/ticker.json`) with the `simulation` block.
    pub fn to_json(&self, source: &str) -> Value {
        let mut v: Value = serde_json::from_str(&self.hub.ticker_json(0)).unwrap_or_else(|_| json!({}));
        v["simulation"] = self.summary(source);
        v
    }

    /// One line for the log.
    pub fn summary_line(&self) -> String {
        let (gap, _) = self.longest_gap();
        let scenes: usize = self.scenes.values().sum();
        tr!(
            "Live-Ticker-Simulation: {} Einträge in {} ({} Wechsel bestätigt, {} möglich, {} Spielsituationen); längste Pause {}",
            "Live ticker simulation: {} entries in {} ({} substitutions confirmed, {} possible, {} game situations); longest silence {}",
            "Simulation du fil en direct : {} entrées en {} ({} changements confirmés, {} possibles, {} situations de jeu) ; plus long silence {}",
            "Simulación del resumen en directo: {} entradas en {} ({} cambios confirmados, {} posibles, {} situaciones de juego); mayor silencio {}",
            self.entries(), clock(self.duration()), self.subs_confirmed, self.subs_possible, scenes, clock(gap)
        )
    }

    /// Write `liveticker.json`; with `export` also the static ticker page next to a `ticker.json` copy.
    pub fn write(&self, path: &Path, source: &str, export: Option<&Path>) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.to_json(source)).unwrap();
        if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        std::fs::write(path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(dir) = export {
            super::ticker_pub::write_assets(dir)?;
            std::fs::write(dir.join("ticker.json"), &text).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        Ok(())
    }
}

/// Build the simulation from a result file (`run --json`).
pub fn from_jsonl(path: &Path, cfg: TickerCfg) -> Result<TickerSim, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut sim = TickerSim::new(cfg);
    for line in txt.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            sim.add_frame(&v);
        }
    }
    if sim.frames == 0 {
        return Err(trs!("keine Bilder in der Datei", "no frames in the file", "aucune image dans le fichier", "no hay imágenes en el archivo").into());
    }
    Ok(sim)
}

/// `out/game.jsonl` -> `out/game.liveticker.json`
pub fn default_path(result: &Path) -> std::path::PathBuf {
    let stem = result.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "ticker".into());
    result.with_file_name(format!("{stem}.liveticker.json"))
}

fn clock(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TickerCfg {
        TickerCfg { names: "numbers".into(), home_name: "Blue".into(), away_name: String::new(), home_players: vec![], away_players: vec![], sport: "basketball".into(), approve: true }
    }

    #[test]
    fn feed_and_summary_from_frames() {
        let mut s = TickerSim::new(cfg());
        s.add_frame(&json!({"t": 0.0, "events": []}));
        s.add_frame(&json!({"t": 30.0, "events": [{"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 12, "t": 30.0}]}));
        s.add_frame(&json!({"t": 100.0, "events": [{"type": "scene", "kind": "free_throw", "team": "away", "t": 100.0}, {"type": "sub", "kind": "possible", "team": "away", "t": 100.0}]}));
        s.add_frame(&json!({"t": 300.0, "events": []}));
        let v = s.to_json("game.jsonl");
        // manual release is off in a simulation: everything is published
        assert_eq!(v["events"].as_array().unwrap().len(), 3);
        assert_eq!(v["config"]["teams"]["home"], "Blue");
        let sim = &v["simulation"];
        assert_eq!(sim["entries"], 3);
        assert_eq!(sim["substitutions"]["confirmed"], 1);
        assert_eq!(sim["substitutions"]["possible"], 1);
        assert_eq!(sim["scenes"]["free_throw"], 1);
        assert_eq!(sim["duration"], 300.0);
        assert_eq!(sim["longest_gap"], 200.0);
        assert_eq!(sim["longest_gap_at"], 100.0);
        assert_eq!(sim["per_10min"], 6.0);
    }

    #[test]
    fn default_name_next_to_result() {
        assert_eq!(default_path(Path::new("out/game.jsonl")), Path::new("out/game.liveticker.json"));
    }
}
