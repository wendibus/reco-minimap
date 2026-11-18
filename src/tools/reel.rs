//! Videos made of several moments of one game (`reco-minimap reel`): highlights, everything of one
//! team, or one video per player.
//!
//! A plan is a list of reels, each with a label and its moments (time, seconds before, seconds
//! after). Overlapping moments are merged, every stretch is cut from the game video and the
//! stretches are joined into one file.
//!
//! **File names**: `YYYY.MM.DD_<label>.mp4` with the date of the game (the video's recording date,
//! else the date of the file). If the output folder already holds that name from *another* game
//! (two games on one day), a number is added: `_2`, `_3`, … The folder keeps a small index
//! (`.reels.json`) of which game each file came from, so making the same video again replaces it.

use crate::{tr, trs};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use super::progress;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub t: f64,
    pub before: f64,
    pub after: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reel {
    pub label: String,
    pub moments: Vec<Moment>,
}

/// `[{"label": "Spieler13", "moments": [{"t": 812.4, "before": 8, "after": 3}, …]}, …]`;
/// `before` / `after` default to `before` / `after`.
pub fn parse_plan(v: &Value, before: f64, after: f64) -> Result<Vec<Reel>, String> {
    let list = v.as_array().ok_or(trs!("Plan: Liste erwartet", "plan: a list is expected", "plan : une liste est attendue", "plan: se espera una lista"))?;
    let mut out = Vec::new();
    for r in list {
        let label = r["label"].as_str().map(str::to_string).unwrap_or_default();
        let moments: Vec<Moment> = r["moments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let t = m["t"].as_f64().or_else(|| m.as_f64())?;
                Some(Moment { t, before: m["before"].as_f64().unwrap_or(before).max(0.0), after: m["after"].as_f64().unwrap_or(after).max(0.5) })
            })
            .collect();
        if !moments.is_empty() {
            out.push(Reel { label, moments });
        }
    }
    Ok(out)
}

/// The stretches to cut: every moment's window, overlapping or touching ones (gap < `join_gap`)
/// merged, in time order, clipped to the video.
pub fn spans(moments: &[Moment], duration: Option<f64>, join_gap: f64) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = moments.iter().map(|m| ((m.t - m.before).max(0.0), m.t + m.after)).map(|(a, b)| (a, duration.map_or(b, |d| b.min(d)))).filter(|(a, b)| b > a).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 + join_gap => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// A label as part of a file name: letters, digits, `-`, `_`; spaces become `_`.
pub fn clean_label(s: &str) -> String {
    let s: String = s.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else if c.is_whitespace() { '_' } else { '-' }).collect();
    let s = s.trim_matches(|c| c == '-' || c == '_').to_string();
    if s.is_empty() {
        "Video".into()
    } else {
        s
    }
}

/// Civil date (UTC) of days since 1970-01-01.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// `YYYY.MM.DD` from an ISO date (`2026-11-15T13:02:11Z`).
fn from_iso(s: &str) -> Option<String> {
    let d = s.get(0..10)?;
    let p: Vec<&str> = d.split('-').collect();
    (p.len() == 3 && p[0].len() == 4 && p.iter().all(|x| x.chars().all(|c| c.is_ascii_digit()))).then(|| format!("{}.{}.{}", p[0], p[1], p[2]))
}

/// The game's date as `YYYY.MM.DD`: the recording date stored in the video, else the file's date.
pub fn game_date(video: &Path) -> String {
    let tag = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format_tags=creation_time", "-of", "default=nw=1:nk=1"])
        .arg(video)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| from_iso(s.trim()));
    if let Some(d) = tag {
        return d;
    }
    let secs = std::fs::metadata(video).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}.{m:02}.{d:02}")
}

const INDEX: &str = ".reels.json";

/// The file for a reel: `<date>_<label>.mp4`, or with `_2`, `_3`, … when that name belongs to
/// another game (by the index of the folder).
pub fn file_for(dir: &Path, date: &str, label: &str, source: &str) -> PathBuf {
    let index: Value = std::fs::read_to_string(dir.join(INDEX)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
    let base = format!("{date}_{}", clean_label(label));
    for n in 1.. {
        let name = if n == 1 { format!("{base}.mp4") } else { format!("{base}_{n}.mp4") };
        let path = dir.join(&name);
        let owner = index[&name].as_str();
        if owner == Some(source) || (owner.is_none() && !path.exists()) {
            return path;
        }
    }
    unreachable!()
}

fn remember(dir: &Path, file: &Path, source: &str) {
    let p = dir.join(INDEX);
    let mut index: Value = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
    if let Some(name) = file.file_name().map(|n| n.to_string_lossy().to_string()) {
        index[name] = json!(source);
        let _ = std::fs::write(&p, serde_json::to_string_pretty(&index).unwrap_or_default());
    }
}

/// Cut the stretches of one reel from `video` and join them into `out`. `step(done, total)`.
pub fn make(video: &str, stretches: &[(f64, f64)], out: &Path, mut step: impl FnMut(usize, usize)) -> Result<(), String> {
    if stretches.is_empty() {
        return Err(trs!("keine Momente", "no moments", "aucun moment", "ningún momento").into());
    }
    let dir = out.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".reel-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let result = (|| {
        let mut list = String::new();
        for (i, (a, b)) in stretches.iter().enumerate() {
            if progress::stop_requested() {
                return Err(trs!("abgebrochen", "cancelled", "annulé", "cancelado").to_string());
            }
            step(i, stretches.len());
            let part = tmp.join(format!("{i:04}.mp4"));
            // every part with the same settings, so they can be joined without encoding again
            let st = Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{a:.3}"), "-i", video, "-t", &format!("{:.3}", b - a)])
                .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ar", "48000", "-ac", "2", "-b:a", "128k"])
                .arg(&part)
                .status()
                .map_err(|e| tr!("ffmpeg nicht gefunden ({e})", "ffmpeg not found ({e})", "ffmpeg introuvable ({e})", "no se encuentra ffmpeg ({e})"))?;
            if !st.success() {
                return Err(tr!("ffmpeg konnte den Ausschnitt ab {:.0} s nicht schneiden", "ffmpeg could not cut the stretch from {:.0} s", "ffmpeg n'a pas pu couper le passage à partir de {:.0} s", "ffmpeg no pudo cortar el tramo desde {:.0} s", a));
            }
            list.push_str(&format!("file '{}'\n", part.file_name().unwrap().to_string_lossy()));
        }
        let lp = tmp.join("list.txt");
        std::fs::write(&lp, list).map_err(|e| e.to_string())?;
        let st = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&lp)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(out)
            .status()
            .map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(tr!("ffmpeg konnte {} nicht zusammenfügen", "ffmpeg could not join {}", "ffmpeg n'a pas pu assembler {}", "ffmpeg no pudo unir {}", out.display()));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Make every reel of a plan into `dir`; returns the files written.
pub fn make_all(video: &str, reels: &[Reel], dir: &Path, duration: Option<f64>) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let source = std::fs::canonicalize(video).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| video.to_string());
    let date = game_date(Path::new(video));
    let total: usize = reels.iter().map(|r| spans(&r.moments, duration, 1.0).len()).sum::<usize>().max(1);
    let mut done = 0usize;
    let mut files = Vec::new();
    for r in reels {
        let st = spans(&r.moments, duration, 1.0);
        let out = file_for(dir, &date, &r.label, &source);
        let name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        make(video, &st, &out, |i, _| progress::report(Some((done + i) as f64 / total as f64), None, &tr!("{}: Ausschnitt {} von {}", "{}: stretch {} of {}", "{} : passage {} sur {}", "{}: tramo {} de {}", name, i + 1, st.len())))?;
        done += st.len();
        remember(dir, &out, &source);
        files.push(out);
    }
    progress::report(Some(1.0), None, "");
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_moments_become_one_stretch() {
        let m = |t: f64| Moment { t, before: 6.0, after: 4.0 };
        assert_eq!(spans(&[m(100.0), m(30.0), m(105.0), m(200.0)], Some(203.0), 1.0), vec![(24.0, 34.0), (94.0, 109.0), (194.0, 203.0)]);
        assert_eq!(spans(&[m(2.0)], None, 1.0), vec![(0.0, 6.0)]);
    }

    #[test]
    fn dates_and_labels() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_407), (2025, 11, 15));
        assert_eq!(from_iso("2026-11-15T13:02:11.000000Z").as_deref(), Some("2026.11.15"));
        assert_eq!(from_iso("junk"), None);
        assert_eq!(clean_label("Spieler 13 / Anna"), "Spieler_13_-_Anna");
        assert_eq!(clean_label("  "), "Video");
    }

    #[test]
    fn a_second_game_on_the_same_day_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("reel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = file_for(&dir, "2026.11.15", "Spieler13", "/games/a.mp4");
        assert_eq!(a.file_name().unwrap(), "2026.11.15_Spieler13.mp4");
        std::fs::write(&a, b"x").unwrap();
        remember(&dir, &a, "/games/a.mp4");
        // the same game again: the same file
        assert_eq!(file_for(&dir, "2026.11.15", "Spieler13", "/games/a.mp4"), a);
        // another game that day: _2
        let b = file_for(&dir, "2026.11.15", "Spieler13", "/games/b.mp4");
        assert_eq!(b.file_name().unwrap(), "2026.11.15_Spieler13_2.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_from_json() {
        let v = json!([{"label": "Highlights", "moments": [{"t": 10.0}, {"t": 50.0, "before": 9}]}, {"label": "leer", "moments": []}]);
        let r = parse_plan(&v, 6.0, 4.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].moments[1], Moment { t: 50.0, before: 9.0, after: 4.0 });
    }
}
