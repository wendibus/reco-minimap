//! Machine-readable progress and a gentle stop for jobs started by the interface.
//!
//! The interface starts every job as a child process of the same program and sets two variables:
//! `RECO_MINIMAP_PROGRESS=1` makes the commands print `@@progress {json}` lines on stderr (the
//! interface turns them into the progress bar and keeps them out of the log), and
//! `RECO_MINIMAP_STOP=<file>` names a file whose appearance asks the command to stop at the next
//! frame and finish its output files normally, so a cancelled run leaves a shorter, playable result.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::json;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RECO_MINIMAP_PROGRESS").is_some())
}

/// Prefix of a progress line on stderr.
pub const PREFIX: &str = "@@progress ";

/// Reports progress: `fraction` 0..1 (None = unknown), an optional stage (1-based index, count,
/// label) and a short message. Silent unless the interface asked for it.
pub fn report(fraction: Option<f64>, stage: Option<(usize, usize, &str)>, message: &str) {
    if !enabled() {
        return;
    }
    let v = json!({
        "progress": fraction.map(|f| (f.clamp(0.0, 1.0) * 10000.0).round() / 10000.0),
        "stage": stage.map(|(i, n, l)| json!({"index": i, "count": n, "label": l})),
        "message": message,
    });
    eprintln!("{PREFIX}{v}");
}

/// Like [`report`], but at most every `every` (always for the first and the last call).
pub struct Throttle {
    last: Option<Instant>,
    every: Duration,
}

impl Throttle {
    pub fn new(every: Duration) -> Self {
        Throttle { last: None, every }
    }
    pub fn report(&mut self, fraction: Option<f64>, stage: Option<(usize, usize, &str)>, message: &str) {
        let due = self.last.map(|t| t.elapsed() >= self.every).unwrap_or(true) || fraction.map(|f| f >= 1.0).unwrap_or(false);
        if due {
            self.last = Some(Instant::now());
            report(fraction, stage, message);
        }
    }
}

/// True once the interface asked this job to stop gently (checked at most every 200 ms).
pub fn stop_requested() -> bool {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    static STATE: std::sync::Mutex<(Option<Instant>, bool)> = std::sync::Mutex::new((None, false));
    let Some(p) = PATH.get_or_init(|| std::env::var_os("RECO_MINIMAP_STOP").map(Into::into)) else {
        return false;
    };
    let mut s = STATE.lock().unwrap();
    if s.1 {
        return true;
    }
    if s.0.map(|t| t.elapsed() < Duration::from_millis(200)).unwrap_or(false) {
        return false;
    }
    s.0 = Some(Instant::now());
    s.1 = p.exists();
    s.1
}

/// Parses a progress line written by [`report`]; `None` for an ordinary log line.
pub fn parse(line: &str) -> Option<serde_json::Value> {
    line.trim_start().strip_prefix(PREFIX).and_then(|j| serde_json::from_str(j).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_round_trip_and_log_lines_do_not() {
        let v = parse(r#"@@progress {"progress":0.5,"stage":{"index":1,"count":2,"label":"x"},"message":"m"}"#).unwrap();
        assert_eq!(v["progress"], 0.5);
        assert_eq!(v["stage"]["count"], 2);
        assert!(parse("  t=  12.0s  4.1 frames/s").is_none());
        assert!(parse("@@progress not json").is_none());
    }
}
