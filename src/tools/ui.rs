//! Local web UI (`reco-minimap ui`): projects, model import, calibration, runs and live mode in the
//! browser. A small std-only HTTP server on 127.0.0.1; heavy work runs as child processes of this
//! same executable (the CLI commands), so UI and command line always behave the same.

use crate::{tr, trs};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::calib::CourtCalib;
use super::models;
use super::video;
use crate::sport::{SportProfile, BUILTIN_IDS};

struct Job {
    kind: String,
    lines: Mutex<Vec<(String, bool)>>,
    status: Mutex<String>, // running | cancelling | done | failed | cancelled
    code: Mutex<Option<i32>>,
    child: Mutex<Option<Child>>,
    /// Latest `@@progress` report of the command (see [`super::progress`]).
    progress: Mutex<Value>,
    /// When the first progress fraction arrived (time remaining is measured from there, not
    /// from the start, so loading the model does not inflate it).
    first_progress: Mutex<Option<(std::time::Instant, f64)>>,
    started: std::time::Instant,
    /// Files this job writes; removed again when the user cancels and discards.
    outputs: Vec<PathBuf>,
    /// Appears when the user cancels and keeps: the command then stops at the next frame.
    stop_file: PathBuf,
    /// After a cancel: true = shortened result kept, false = discarded.
    kept: Mutex<Option<bool>>,
}

/// Output files named in a job's arguments (relative ones are inside the project folder).
fn job_outputs(dir: &Path, args: &[String]) -> Vec<PathBuf> {
    args.windows(2)
        .filter(|w| matches!(w[0].as_str(), "--minimap" | "--alpha" | "--pip" | "--json" | "--dets" | "--metrics" | "--ticker-json"))
        .map(|w| {
            let p = PathBuf::from(&w[1]);
            if p.is_absolute() { p } else { dir.join(p) }
        })
        .collect()
}

/// The job as the page sees it (same field names as the Reco Trainer job dialog).
fn job_json(id: u64, j: &Job) -> Value {
    let pr = j.progress.lock().unwrap().clone();
    let status = j.status.lock().unwrap().clone();
    let elapsed = j.started.elapsed().as_secs_f64();
    let frac = pr["progress"].as_f64();
    let eta = match (frac, *j.first_progress.lock().unwrap()) {
        (Some(f), Some((t0, f0))) if status == "running" && f < 1.0 && f - f0 > 0.01 && t0.elapsed().as_secs_f64() > 3.0 => Some(t0.elapsed().as_secs_f64() * (1.0 - f) / (f - f0)),
        _ => None,
    };
    json!({"id": id, "type": j.kind, "status": status, "cancellable": status == "running",
        "keepable": matches!(j.kind.as_str(), "run" | "scales" | "colors" | "selfcheck" | "live"),
        "progress": pr["progress"], "stage": pr["stage"], "message": pr["message"],
        "elapsedSeconds": (elapsed * 10.0).round() / 10.0, "etaSeconds": eta.map(|e| e.round()),
        "kept": *j.kept.lock().unwrap(), "code": *j.code.lock().unwrap()})
}

struct Ui {
    exe: PathBuf,
    jobs: Mutex<HashMap<u64, Arc<Job>>>,
    next: AtomicU64,
    /// Result of the update check started with the server: `None` while it runs (or when it is
    /// switched off), then `{"checked": true, "latest": ..}` with `latest` set if one is newer.
    update: Mutex<Option<Value>>,
}

pub(crate) fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn jres(s: &mut TcpStream, v: Value) {
    respond(s, "200 OK", "application/json; charset=utf-8", v.to_string().as_bytes());
}

fn err(s: &mut TcpStream, msg: impl std::fmt::Display) {
    jres(s, json!({"ok": false, "error": msg.to_string()}));
}

/// The interface can write files and start jobs, so only the page it serves itself may call it.
/// `Host` must be this machine by loopback name (a foreign name resolving to 127.0.0.1 is DNS
/// rebinding), and a browser's `Origin`, when sent, must be that same host (anything else is another
/// web page posting to us). Requests without `Origin` come from non-browser clients such as curl.
pub fn request_allowed(host: Option<&str>, origin: Option<&str>) -> bool {
    let host = match host {
        Some(h) => h.trim(),
        None => return false,
    };
    let name = if host.starts_with('[') { host.split(']').next().unwrap_or("") } else { host.split(':').next().unwrap_or("") };
    if !matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "[::1") {
        return false;
    }
    match origin {
        Some(o) => o.trim().strip_prefix("http://").map(|o| o.eq_ignore_ascii_case(host)).unwrap_or(false),
        None => true,
    }
}

pub(crate) fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

pub(crate) fn read_request(s: &mut TcpStream) -> Option<(String, String, String, String, String)> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let first = head.lines().next().unwrap_or("").to_string();
    let mut it = first.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let full = it.next().unwrap_or("/").to_string();
    let (path, query) = match full.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (full, String::new()),
    };
    let clen = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
    while buf.len() < header_end + clen {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buf[header_end.min(buf.len())..]).to_string();
    Some((method, path, query, body, head))
}

pub(crate) fn qparam(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

fn sanitize(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if n.is_empty() { "lauf".into() } else { n }
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into()))
}

fn default_project_dir() -> PathBuf {
    home_dir().join("reco-minimap-projekte").join("standard")
}

fn native_dialog(kind: &str, prompt: &str) -> Option<String> {
    let esc = prompt.replace('\\', "\\\\").replace('"', "\\\"");
    let out = if cfg!(target_os = "windows") {
        let esc = prompt.replace('\'', "''");
        let pick = if kind == "folder" {
            format!("$d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '{esc}'; if ($d.ShowDialog() -eq 'OK') {{ $d.SelectedPath }}")
        } else {
            format!("$d = New-Object System.Windows.Forms.OpenFileDialog; $d.Title = '{esc}'; if ($d.ShowDialog() -eq 'OK') {{ $d.FileName }}")
        };
        let script = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; {pick}");
        Command::new("powershell").args(["-NoProfile", "-STA", "-Command", &script]).output().ok()?
    } else if cfg!(target_os = "macos") {
        let what = if kind == "folder" { "folder" } else { "file" };
        Command::new("osascript").args(["-e", &format!("POSIX path of (choose {what} with prompt \"{esc}\")")]).output().ok()?
    } else {
        let mut c = Command::new("zenity");
        c.arg("--file-selection").arg(format!("--title={prompt}"));
        if kind == "folder" {
            c.arg("--directory");
        }
        c.output().ok()?
    };
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() { None } else { Some(if p.len() > 3 { p.trim_end_matches(|c| c == '/' || c == '\\').to_string() } else { p }) }
}

/// Show `path` in the file manager, or with `open` hand a result file to its default program.
/// Reports why nothing happened instead of failing silently.
fn reveal(path: &str, open: bool) -> Result<(), String> {
    let p = Path::new(path);
    if path.is_empty() || !p.exists() {
        return Err(tr!("Datei nicht gefunden: {path}", "File not found: {path}", "Fichier introuvable : {path}", "Archivo no encontrado: {path}"));
    }
    if open && !(p.is_file() && matches!(p.extension().and_then(|x| x.to_str()), Some("mp4" | "mov" | "jsonl"))) {
        return Err(tr!("Kein Ergebnis dieses Programms: {path}", "Not a result of this program: {path}", "Pas un résultat de ce programme : {path}", "No es un resultado de este programa: {path}"));
    }
    let run = |prog: &str, args: &[&str]| Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    // Linux file managers cannot select a file through xdg-open: show its folder
    let folder = if p.is_dir() { p } else { p.parent().unwrap_or(p) }.to_string_lossy().to_string();
    let ok = if cfg!(target_os = "macos") {
        if open || p.is_dir() { run("open", &[path]) } else { run("open", &["-R", path]) || run("open", &[&folder]) }
    } else if cfg!(target_os = "windows") {
        // explorer returns a non-zero code even when it worked
        let _ = if open || p.is_dir() { run("explorer", &[path]) } else { run("explorer", &["/select,", path]) };
        true
    } else {
        run("xdg-open", &[if open { path } else { &folder }])
    };
    if ok { Ok(()) } else { Err(tr!("Konnte nicht geöffnet werden: {path}", "Could not be opened: {path}", "Ouverture impossible : {path}", "No se pudo abrir: {path}")) }
}

/// Pictures and demo films of the walkthrough (`docs/img`, `docs/media`), built into the program
/// so the guide also shows them offline.
pub fn guide_asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    let data: &'static [u8] = match path {
        "img/de/calibration.webp" => include_bytes!("../../docs/img/de/calibration.webp"),
        "img/de/step1.webp" => include_bytes!("../../docs/img/de/step1.webp"),
        "img/de/step2.webp" => include_bytes!("../../docs/img/de/step2.webp"),
        "img/de/step3-colours.webp" => include_bytes!("../../docs/img/de/step3-colours.webp"),
        "img/de/step3.webp" => include_bytes!("../../docs/img/de/step3.webp"),
        "img/de/step4-results.webp" => include_bytes!("../../docs/img/de/step4-results.webp"),
        "img/de/step4.webp" => include_bytes!("../../docs/img/de/step4.webp"),
        "img/de/step5.webp" => include_bytes!("../../docs/img/de/step5.webp"),
        "img/de/step6.webp" => include_bytes!("../../docs/img/de/step6.webp"),
        "img/de/step7-heat.webp" => include_bytes!("../../docs/img/de/step7-heat.webp"),
        "img/de/step7-shape.webp" => include_bytes!("../../docs/img/de/step7-shape.webp"),
        "img/en/calibration.webp" => include_bytes!("../../docs/img/en/calibration.webp"),
        "img/en/step1.webp" => include_bytes!("../../docs/img/en/step1.webp"),
        "img/en/step2.webp" => include_bytes!("../../docs/img/en/step2.webp"),
        "img/en/step3-colours.webp" => include_bytes!("../../docs/img/en/step3-colours.webp"),
        "img/en/step3.webp" => include_bytes!("../../docs/img/en/step3.webp"),
        "img/en/step4-results.webp" => include_bytes!("../../docs/img/en/step4-results.webp"),
        "img/en/step4.webp" => include_bytes!("../../docs/img/en/step4.webp"),
        "img/en/step5.webp" => include_bytes!("../../docs/img/en/step5.webp"),
        "img/en/step6.webp" => include_bytes!("../../docs/img/en/step6.webp"),
        "img/en/step7-heat.webp" => include_bytes!("../../docs/img/en/step7-heat.webp"),
        "img/en/step7-shape.webp" => include_bytes!("../../docs/img/en/step7-shape.webp"),
        "img/es/calibration.webp" => include_bytes!("../../docs/img/es/calibration.webp"),
        "img/es/step1.webp" => include_bytes!("../../docs/img/es/step1.webp"),
        "img/es/step2.webp" => include_bytes!("../../docs/img/es/step2.webp"),
        "img/es/step3-colours.webp" => include_bytes!("../../docs/img/es/step3-colours.webp"),
        "img/es/step3.webp" => include_bytes!("../../docs/img/es/step3.webp"),
        "img/es/step4-results.webp" => include_bytes!("../../docs/img/es/step4-results.webp"),
        "img/es/step4.webp" => include_bytes!("../../docs/img/es/step4.webp"),
        "img/es/step5.webp" => include_bytes!("../../docs/img/es/step5.webp"),
        "img/es/step6.webp" => include_bytes!("../../docs/img/es/step6.webp"),
        "img/es/step7-heat.webp" => include_bytes!("../../docs/img/es/step7-heat.webp"),
        "img/es/step7-shape.webp" => include_bytes!("../../docs/img/es/step7-shape.webp"),
        "img/fr/calibration.webp" => include_bytes!("../../docs/img/fr/calibration.webp"),
        "img/fr/step1.webp" => include_bytes!("../../docs/img/fr/step1.webp"),
        "img/fr/step2.webp" => include_bytes!("../../docs/img/fr/step2.webp"),
        "img/fr/step3-colours.webp" => include_bytes!("../../docs/img/fr/step3-colours.webp"),
        "img/fr/step3.webp" => include_bytes!("../../docs/img/fr/step3.webp"),
        "img/fr/step4-results.webp" => include_bytes!("../../docs/img/fr/step4-results.webp"),
        "img/fr/step4.webp" => include_bytes!("../../docs/img/fr/step4.webp"),
        "img/fr/step5.webp" => include_bytes!("../../docs/img/fr/step5.webp"),
        "img/fr/step6.webp" => include_bytes!("../../docs/img/fr/step6.webp"),
        "img/fr/step7-heat.webp" => include_bytes!("../../docs/img/fr/step7-heat.webp"),
        "img/fr/step7-shape.webp" => include_bytes!("../../docs/img/fr/step7-shape.webp"),
        "img/overlay.webp" => include_bytes!("../../docs/img/overlay.webp"),
        "media/demo-minimap.mp4" => include_bytes!("../../docs/media/demo-minimap.mp4"),
        "media/demo-minimap.webp" => include_bytes!("../../docs/media/demo-minimap.webp"),
        "media/demo-pip.mp4" => include_bytes!("../../docs/media/demo-pip.mp4"),
        "media/demo-pip.webp" => include_bytes!("../../docs/media/demo-pip.webp"),
        _ => return None,
    };
    let kind = if path.ends_with(".mp4") { "video/mp4" } else { "image/webp" };
    Some((data, kind))
}

/// The walkthrough in the given language (`docs/WALKTHROUGH*.md`), built into the program.
pub fn guide(lang: super::i18n::Lang) -> &'static str {
    use super::i18n::Lang;
    match lang {
        Lang::De => include_str!("../../docs/WALKTHROUGH.de.md"),
        Lang::En => include_str!("../../docs/WALKTHROUGH.md"),
        Lang::Fr => include_str!("../../docs/WALKTHROUGH.fr.md"),
        Lang::Es => include_str!("../../docs/WALKTHROUGH.es.md"),
    }
}

// ---------------------------------------------------------------- project

fn project_files(dir: &Path) -> Value {
    let court = dir.join("court.json");
    let court_v = CourtCalib::load(&court.to_string_lossy()).ok().map(|c| json!({"points": c.points.len(), "rms": c.rms_m, "sport": c.sport, "pano": [c.pano_w, c.pano_h], "lens": c.lens.map(|l| l.model.key())}));
    let rd = |n: &str| std::fs::read_to_string(dir.join(n)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let mut outs = Vec::new();
    if let Ok(rdir) = std::fs::read_dir(dir.join("out")) {
        for e in rdir.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
            if matches!(ext, "mp4" | "mov" | "jsonl") || p.to_string_lossy().ends_with(".liveticker.json") {
                let m = e.metadata().ok();
                outs.push(json!({"name": e.file_name().to_string_lossy(), "path": p.to_string_lossy(), "mb": m.as_ref().map(|m| (m.len() as f64 / 1e5).round() / 10.0), "t": m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs())}));
            }
        }
    }
    outs.sort_by(|a, b| b["t"].as_u64().cmp(&a["t"].as_u64()));
    let board = super::scoreboard::load(&dir.join("scoreboard.json").to_string_lossy()).ok().map(|c| json!({"fields": c.fields().len(), "home": if c.home == crate::scoreboard::HomeSide::Right { "right" } else { "left" }}));
    json!({"court": court_v, "scales": rd("scales.json").map(|v| json!({"n": v["n"]})), "teams": rd("teams.json"), "scoreboard": board, "out": outs})
}

fn load_project(dir: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(dir.join("models")).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("out")).map_err(|e| e.to_string())?;
    let proj = std::fs::read_to_string(dir.join("project.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| json!({}));
    let models: Vec<Value> = models::list_with_camera(&dir.join("models")).iter().map(|m| m.to_json()).collect();
    Ok(json!({"ok": true, "dir": dir.to_string_lossy(), "project": proj, "models": models, "files": project_files(dir)}))
}

/// Statistics of one result file of the project, for the sport it was calibrated for.
/// Frames per second and seconds before / after a basket in [`basket_strip`].
pub const STRIP_FPS: f64 = 3.0;
pub const STRIP_SPAN: (f64, f64) = (6.5, 1.0);

/// The seconds around a basket as one tall JPEG (frames below each other, [`STRIP_FPS`] per
/// second from `t - STRIP_SPAN.0`), cut to the part of the panorama around `u` (0..1 across), so
/// the far basket is big enough to see who shot.
fn basket_strip(video: &str, t: f64, u: f64) -> Result<Vec<u8>, String> {
    if video.is_empty() || !Path::new(video).is_file() {
        return Err(trs!("Video nicht gefunden", "video not found", "vidéo introuvable", "vídeo no encontrado").into());
    }
    let (start, dur) = ((t - STRIP_SPAN.0).max(0.0), STRIP_SPAN.0 + STRIP_SPAN.1);
    let n = (dur * STRIP_FPS).round() as usize;
    let frac = 0.36;
    let x = format!("min(max(iw*{u:.4}-iw*{h:.3},0),iw*{r:.3})", u = u.clamp(0.0, 1.0), h = frac / 2.0, r = 1.0 - frac);
    let vf = format!("fps={STRIP_FPS},crop=w=iw*{frac}:h=ih*0.68:x='{x}':y=ih*0.32,scale=960:-2,tile=1x{n}");
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{start:.3}"), "-t", &format!("{dur:.3}"), "-i", video, "-vf", &vf, "-frames:v", "1", "-q:v", "4", "-f", "image2pipe", "-vcodec", "mjpeg", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(format!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// Where in the panorama (0..1 across) a field point lies: the inverse of the calibration, by search.
fn pano_u(proj: &dyn crate::geom::Projector, p: crate::geom::Vec2) -> Option<f64> {
    let mut best: Option<(f32, f32, f32)> = None;
    for i in 0..=160 {
        for j in 0..=40 {
            let (u, v) = (i as f32 / 160.0, 0.3 + 0.7 * j as f32 / 40.0);
            if let Some(c) = proj.to_court(u, v) {
                let d = c.dist(p);
                if best.is_none_or(|b| d < b.2) {
                    best = Some((u, v, d));
                }
            }
        }
    }
    best.filter(|b| b.2 < 3.0).map(|b| b.0 as f64)
}

fn stats_for(dir: &Path, file: &str, fixes: Option<&Value>) -> Result<Value, String> {
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { dir.join(file) };
    let txt = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let frames: Vec<Value> = txt.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if frames.len() < 2 {
        return Err(trs!("zu wenige Bilder in der Datei", "too few frames in the file", "trop peu d'images dans le fichier", "muy pocas imágenes en el archivo").into());
    }
    let profile = CourtCalib::load(&dir.join("court.json").to_string_lossy()).map(|c| c.profile()).map_err(|e| e.to_string())?;
    let proj = std::fs::read_to_string(dir.join("project.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| json!({}));
    // the hand checks sent along (newest) or saved in the project
    let fixes = super::stats_players::parse_fixes(fixes.unwrap_or(&proj["score_fixes"]));
    let mut v = super::stats::report_full(&profile, &frames, project_direction(&proj["direction"]), &fixes);
    // per basket: where the players were in the panorama, for the picture strip
    // get_mut, not v["box_score"]["made"]: indexing would turn a missing box score (null) into {"made": null}
    if let (Some(made), Ok(cal)) = (v.get_mut("box_score").and_then(|b| b.get_mut("made")).and_then(Value::as_array_mut), CourtCalib::load(&dir.join("court.json").to_string_lossy())) {
        if let Some(proj) = cal.projector() {
            for m in made.iter_mut() {
                let t = m["t"].as_f64().unwrap_or(0.0);
                let i0 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) < t - 4.0);
                let i1 = frames.partition_point(|f| f["t"].as_f64().unwrap_or(0.0) <= t);
                let pts: Vec<(f64, f64)> = frames[i0..i1.max(i0)].iter().flat_map(|f| f["players"].as_array().into_iter().flatten()).filter_map(|p| Some((p["x"].as_f64()?, p["y"].as_f64()?))).collect();
                if pts.is_empty() {
                    continue;
                }
                let c = crate::geom::Vec2::new((pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64) as f32, (pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64) as f32);
                // towards the nearer basket: the shot is there
                let g = profile.goals.iter().copied().min_by(|a, b| a.dist(c).total_cmp(&b.dist(c))).unwrap_or(c);
                let aim = crate::geom::Vec2::new((c.x + g.x) / 2.0, (c.y + g.y) / 2.0);
                if let Some(u) = pano_u(&proj, aim) {
                    m["u"] = json!((u * 1000.0).round() / 1000.0);
                }
            }
        }
    }
    Ok(v)
}

/// Moments of a run to open in the tactics board: the baskets / goals, attempts, set pieces and
/// substitutions of its ticker file (`<name>.liveticker.json` next to `<name>.jsonl`), oldest first.
/// A score keeps its last update; taken back ones and corrections are left out.
fn board_moments(dir: &Path, file: &str) -> Vec<Value> {
    let f = if Path::new(file).is_absolute() { PathBuf::from(file) } else { dir.join(file) };
    let ticker = f.with_extension("liveticker.json");
    let Some(v) = std::fs::read_to_string(&ticker).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return Vec::new() };
    let mut scores: Vec<Value> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for e in v["events"].as_array().into_iter().flatten() {
        let t = e["t"].as_f64();
        match e["type"].as_str() {
            Some("score") => {
                let sid = e["sid"].clone();
                scores.retain(|x| x["sid"] != sid);
                if e["retracted"] != json!(true) && e["kind"] != "correction" && e["points"].as_i64().unwrap_or(0) > 0 {
                    scores.push(json!({"t": t, "type": "score", "kind": e["kind"], "points": e["points"], "team": e["team"], "number": e["number"], "clock": e["clock"], "sid": sid}));
                }
            }
            Some("scene") if e["phase"] != "formation" || matches!(e["kind"].as_str(), Some("tip_off" | "kickoff" | "free_throw" | "corner" | "penalty" | "seven_meter")) => {
                out.push(json!({"t": t, "type": "scene", "kind": e["kind"], "team": e["team"], "number": e["actor_number"].clone()}));
            }
            Some("sub") if e["kind"] == "confirmed" => out.push(json!({"t": t, "type": "sub", "team": e["team"], "in": e["in"], "out": e["out"]})),
            _ => {}
        }
    }
    out.extend(scores.into_iter().map(|mut x| { x.as_object_mut().map(|o| o.remove("sid")); x }));
    out.retain(|x| x["t"].is_number());
    out.sort_by(|a, b| a["t"].as_f64().unwrap_or(0.0).total_cmp(&b["t"].as_f64().unwrap_or(0.0)));
    out
}

/// The playing direction set in the project (`{"home": "l"|"r"|"auto", "switch": "24:00" | "period:3"}`).
fn project_direction(d: &Value) -> Option<crate::direction::Direction> {
    crate::direction::Direction::parse(&get_s(d, "home"), &get_s(d, "switch"))
}

/// `--home-attacks` / `--switch-at` for a job, if the project has a direction.
fn direction_args(a: &mut Vec<String>, p: &Value) {
    if let Some(d) = project_direction(&p["direction"]) {
        pushs(a, &["--home-attacks", &d.home_start.to_string()]);
        if let Some(p) = d.switch_period {
            pushs(a, &["--switch-period", &p.to_string()]);
        } else if let Some(s) = d.switch_at {
            pushs(a, &["--switch-at", &format!("{s}")]);
        }
    }
}

// ---------------------------------------------------------------- jobs

fn push_line(job: &Job, text: String, progress: bool) {
    if text.trim().is_empty() {
        return;
    }
    let mut l = job.lines.lock().unwrap();
    if progress && l.last().map(|x| x.1).unwrap_or(false) {
        *l.last_mut().unwrap() = (text, true);
    } else {
        l.push((text, progress));
    }
    if l.len() > 3000 {
        l.drain(..500);
    }
}

/// One finished line of the command's output: progress reports go to the job state, the rest to the log.
fn take_line(job: &Job, text: String, cr: bool) {
    match text.find(super::progress::PREFIX) {
        Some(i) => {
            let (before, rest) = text.split_at(i);
            push_line(job, before.to_string(), true);
            if let Some(v) = super::progress::parse(rest) {
                if let Some(f) = v["progress"].as_f64() {
                    job.first_progress.lock().unwrap().get_or_insert((std::time::Instant::now(), f));
                }
                *job.progress.lock().unwrap() = v;
            }
        }
        None => push_line(job, text, cr),
    }
}

fn pump(mut r: impl Read, job: Arc<Job>) {
    let (mut cur, mut buf) = (Vec::new(), [0u8; 4096]);
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !cur.is_empty() {
                            take_line(&job, String::from_utf8_lossy(&cur).to_string(), b == b'\r');
                            cur.clear();
                        }
                    } else {
                        cur.push(b);
                    }
                }
            }
        }
    }
    if !cur.is_empty() {
        take_line(&job, String::from_utf8_lossy(&cur).to_string(), false);
    }
}

fn get_s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Build the CLI arguments for a job kind. Parameters come as structured JSON, never as a shell string.
fn build_args(kind: &str, dir: &Path, p: &Value) -> Result<Vec<String>, String> {
    let model = get_s(p, "model_dir");
    let video = get_s(p, "video");
    let need = |v: &str, what: &str| if v.is_empty() { Err(tr!("{what} fehlt", "{what} is missing", "{what} : manquant", "falta: {what}")) } else { Ok(()) };
    let mut a: Vec<String> = Vec::new();
    let ep = get_s(p, "ep");
    let units = get_s(p, "units");
    let has_scales = dir.join("scales.json").exists();
    // which part of a player box shows the shirt (only passed when it is a usable range)
    let fig = &p["figures"];
    let torso = match (fig["torso_top"].as_f64(), fig["torso_bottom"].as_f64()) {
        (Some(t), Some(b)) if (0.0..1.0).contains(&t) && t < b && b <= 1.0 => Some(format!("{t},{b}")),
        _ => None,
    };
    let engine = |a: &mut Vec<String>| {
        let workers=get_s(p,"workers").parse::<usize>().unwrap_or(1).clamp(1,4);
        pushs(a,&["--workers",&workers.to_string()]);
        a.push("--ep".into());
        a.push(if ep.is_empty() { "cpu".into() } else { ep.clone() });
        if !units.is_empty() && units != "all" {
            a.push("--units".into());
            a.push(units.clone());
        }
    };
    match kind {
        "calibrate" => {
            need(&video, trs!("Video", "video", "vidéo", "vídeo"))?;
            // the page is shown inside the interface, not in a browser window of its own
            pushs(&mut a, &["calibrate", &video, "--sport", &get_s(p, "sport"), "--time", &get_s(p, "time"), "--out", "court.json", "--lang", &get_s(p, "lang"), "--no-open"]);
            // lens: chosen in the interface; empty = as before / a panorama for very wide pictures
            match p["lens"].as_str().filter(|l| ["none", "fisheye", "panorama"].contains(l)) {
                Some(l) => pushs(&mut a, &["--lens", l]),
                None if p["fisheye"] == true => pushs(&mut a, &["--lens", "fisheye"]),
                None => {}
            }
            if get_s(p, "sport") == "custom" {
                pushs(&mut a, &["--length", &get_s(p, "length"), "--width", &get_s(p, "width")]);
            }
            for k in ["team_size", "referees"] {
                if !get_s(p, k).is_empty() {
                    pushs(&mut a, &[&format!("--{}", k.replace('_', "-")), &get_s(p, k)]);
                }
            }
        }
        "scoreboard_calibrate" => {
            need(&video, trs!("Video", "video", "vidéo", "vídeo"))?;
            pushs(&mut a, &["scoreboard-calibrate", &video, "--time", &get_s(p, "time"), "--out", "scoreboard.json", "--lang", &get_s(p, "lang"), "--no-open"]);
        }
        "selfcheck" | "scales" | "colors" => {
            need(&video, trs!("Video", "video", "vidéo", "vídeo"))?;
            need(&model, trs!("Modell", "model", "modèle", "modelo"))?;
            pushs(&mut a, &[kind, &video, "--model", &model]);
            if kind == "selfcheck" {
                pushs(&mut a, &["--samples", "6"]);
            } else {
                pushs(&mut a, &["--samples", "12", "--court", "court.json", "--out", if kind == "scales" { "scales.json" } else { "teams.json" }]);
            }
            if has_scales && kind != "scales" {
                pushs(&mut a, &["--tile", "auto"]);
            }
            if let (Some(t), "colors") = (&torso, kind) {
                pushs(&mut a, &["--torso", t]);
            }
            engine(&mut a);
        }
        "run" | "live" => {
            need(&model, trs!("Modell", "model", "modèle", "modelo"))?;
            let src = if kind == "run" { video.clone() } else { get_s(p, "source") };
            need(&src, trs!("Quelle/Video", "source/video", "source/vidéo", "fuente/vídeo"))?;
            pushs(&mut a, &[kind, &src, "--model", &model, "--court", "court.json", "--teams", "teams.json", "--tile", if has_scales { "auto" } else { "model" }]);
            engine(&mut a);
            for (flag, key) in [("--draw-home", "draw_home"), ("--draw-away", "draw_away"), ("--ref-color", "ref_color"), ("--min-alpha", "min_alpha")] {
                if !get_s(p, key).is_empty() {
                    pushs(&mut a, &[flag, &get_s(p, key)]);
                }
            }
            if p["scenes"].as_bool() == Some(true) {
                pushs(&mut a, &["--scenes"]);
            }
            // the calibrated hall scoreboard is read along unless switched off (run: its own video stream of
            // the board region; live: from the analysed frames, with who scored for basketball)
            if dir.join("scoreboard.json").exists() && p["scoreboard"].as_bool() != Some(false) {
                pushs(&mut a, &["--scoreboard", "scoreboard.json"]);
            }
            direction_args(&mut a, p);
            if let Some(t) = &torso {
                pushs(&mut a, &["--torso", t]);
            }
            if fig["shirt_match"].as_bool() == Some(true) {
                pushs(&mut a, &["--shirt-match"]);
            }
            if fig["no_referees"].as_bool() == Some(true) {
                pushs(&mut a, &["--referees", "0"]);
            }
            // referees recognised (so they never count as players) but not drawn
            if fig["hide_referees"].as_bool() == Some(true) {
                pushs(&mut a, &["--hide-referees"]);
            }
            let nm = &p["numbers"];
            if nm["enabled"].as_bool() == Some(true) {
                pushs(&mut a, &["--numbers"]);
                for (flag, key) in [("--roster-home", "roster_home"), ("--roster-away", "roster_away")] {
                    if !get_s(nm, key).is_empty() {
                        pushs(&mut a, &[flag, &get_s(nm, key)]);
                    }
                }
                if kind == "run" && !get_s(nm, "hires").is_empty() {
                    pushs(&mut a, &["--hires", &get_s(nm, "hires")]);
                }
            }
            if kind == "run" {
                let name = sanitize(&get_s(p, "name"));
                if !get_s(p, "start").is_empty() {
                    pushs(&mut a, &["--start", &get_s(p, "start")]);
                }
                if !get_s(p, "duration").is_empty() && get_s(p, "duration") != "0" {
                    pushs(&mut a, &["--duration", &get_s(p, "duration")]);
                }
                pushs(&mut a, &["--every", &get_s(p, "every")]);
                if p["adaptive"].as_bool() == Some(true) {
                    pushs(&mut a, &["--adaptive"]);
                }
                if p["distant"].as_bool() == Some(true) {
                    // smaller tiles: distant players larger in the model's view, at the cost of speed
                    pushs(&mut a, &["--tile-for", "512"]);
                }
                let out = |ext: &str| dir.join("out").join(format!("{name}{ext}")).to_string_lossy().into_owned();
                let o = &p["outputs"];
                let (mut any, mut paths) = (false, Vec::new());
                if o["minimap"].as_bool() == Some(true) { paths.push(("--minimap", out(".mp4"))); }
                if o["alpha"].as_bool() == Some(true) { paths.push(("--alpha", out(".mov"))); if !get_s(p, "alpha_bg").is_empty() { paths.push(("--alpha-bg", get_s(p, "alpha_bg"))); } }
                if o["pip"].as_bool() == Some(true) { paths.push(("--pip", out("_pip.mp4"))); }
                if o["json"].as_bool() == Some(true) { paths.push(("--json", out(".jsonl"))); }
                // detections + number readings for `replay` (re-evaluate without the models), and timings
                if o["evidence"].as_bool() == Some(true) {
                    paths.push(("--dets", out(".dets.jsonl")));
                    paths.push(("--metrics", out(".metrics.json")));
                }
                // what a live ticker would have reported for this video
                if o["ticker"].as_bool() == Some(true) {
                    paths.push(("--ticker-json", out(".liveticker.json")));
                    ticker_names(&mut a, p);
                }
                for (f, v) in &paths { if !f.starts_with("--alpha-bg") { any = true; } pushs(&mut a, &[f, v]); }
                if !any {
                    return Err(trs!("keine Ausgabe gewählt", "no output chosen", "aucune sortie choisie", "no se ha elegido ninguna salida").into());
                }
            } else {
                pushs(&mut a, &["--size", &get_s(p, "size"), "--fps", &get_s(p, "fps"), "--port", &get_s(p, "port")]);
                // live ticker: what the public page shows, team names, and whether other devices may reach it
                if matches!(get_s(p, "ticker_names").as_str(), "numbers" | "names" | "both") {
                    pushs(&mut a, &["--ticker-names", &get_s(p, "ticker_names")]);
                }
                for (flag, key) in [("--home-name", "home_name"), ("--away-name", "away_name")] {
                    if !get_s(p, key).trim().is_empty() {
                        pushs(&mut a, &[flag, get_s(p, key).trim()]);
                    }
                }
                if p["ticker_approve"].as_bool() == Some(true) {
                    pushs(&mut a, &["--ticker-approve"]);
                }
                // phone page to name shooters: only with a long random key from the project
                let pk = get_s(p, "pick_key");
                if p["pick"].as_bool() == Some(true) && pk.len() >= 16 && pk.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    pushs(&mut a, &["--pick-key", &pk]);
                }
                // copies of the public feed for a web site (static files; the upload needs curl)
                if p["ticker_export"].as_bool() == Some(true) {
                    pushs(&mut a, &["--ticker-export", &dir.join("out").join("ticker").to_string_lossy()]);
                }
                for (flag, key) in [("--ticker-push", "ticker_push"), ("--ticker-key", "ticker_key")] {
                    if !get_s(p, key).trim().is_empty() {
                        pushs(&mut a, &[flag, get_s(p, key).trim()]);
                    }
                }
                if p["network"].as_bool() == Some(true) {
                    pushs(&mut a, &["--host", "0.0.0.0"]);
                }
                if p["realtime"].as_bool() == Some(true) {
                    pushs(&mut a, &["--realtime"]);
                }
                if p["adaptive"].as_bool() == Some(true) {
                    pushs(&mut a, &["--adaptive-live"]);
                }
            }
        }
        "board" => {
            let f = get_s(p, "file");
            need(&f, trs!("Datei", "file", "fichier", "archivo"))?;
            let at = get_s(p, "at");
            need(&at, trs!("Zeitpunkt", "moment", "instant", "momento"))?;
            let stem = Path::new(&f).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let scene = dir.join("out").join(format!("{}-board-{}.json", sanitize(&stem), sanitize(&at.replace(':', "m"))));
            pushs(&mut a, &["board", &scene.to_string_lossy(), "--port", "0"]);
            // a board that was saved before is opened as it is; only a new moment is cut from the run
            if !scene.exists() {
                if !dir.join("court.json").exists() {
                    return Err(trs!("Kalibrierung fehlt (court.json)", "calibration is missing (court.json)", "calibrage manquant (court.json)", "falta la calibración (court.json)").into());
                }
                pushs(&mut a, &["--states", &f, "--at", &at, "--trail", &get_s(p, "trail"), "--court", "court.json"]);
                if dir.join("teams.json").exists() {
                    pushs(&mut a, &["--teams", "teams.json"]);
                }
                for (flag, key) in [("--draw-home", "draw_home"), ("--draw-away", "draw_away")] {
                    if !get_s(p, key).is_empty() {
                        pushs(&mut a, &[flag, &get_s(p, key)]);
                    }
                }
            }
        }
        "fetch_ocr" => pushs(&mut a, &["fetch-ocr"]),
        "fetch_model" => pushs(&mut a, &["fetch-model"]),
        "update" if is_mac_app() => {
            return Err(trs!(
                "Die Mac-App aktualisiert sich nicht selbst. Lade die neue Version von der Release-Seite und ersetze die App.",
                "The Mac app does not update itself. Download the new version from the release page and replace the app.",
                "L'app Mac ne se met pas à jour elle-même. Téléchargez la nouvelle version depuis la page des versions et remplacez l'app.",
                "La app de Mac no se actualiza sola. Descarga la nueva versión desde la página de versiones y reemplaza la app."
            )
            .into())
        }
        "update" => pushs(&mut a, &["update", "--yes"]),
        "clip" => {
            // highlight clips around key moments of the statistics, into out/clips/
            let at: Vec<String> = p["at"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).map(|t| format!("{t:.2}")).collect()).unwrap_or_default();
            need(&video, trs!("Video", "video", "vidéo", "vídeo"))?;
            if at.is_empty() {
                return Err(trs!("kein Moment gewählt", "no moment chosen", "aucun moment choisi", "ningún momento elegido").into());
            }
            pushs(&mut a, &["clip", &video, "--at", &at.join(","), "--out", "out/clips"]);
            for k in ["before", "after"] {
                if let Some(v) = p[k].as_f64() {
                    pushs(&mut a, &[&format!("--{k}"), &v.to_string()]);
                }
            }
        }
        "reel" => {
            // several moments joined into one video each (highlights, a team, one per player), into out/videos/
            need(&video, trs!("Video", "video", "vidéo", "vídeo"))?;
            let plan = p["plan"].as_array().filter(|a| !a.is_empty()).ok_or(trs!("kein Moment gewählt", "no moment chosen", "aucun moment choisi", "ningún momento elegido"))?;
            let vdir = dir.join("out").join("videos");
            std::fs::create_dir_all(&vdir).map_err(|e| e.to_string())?;
            let pf = vdir.join(".plan.json");
            std::fs::write(&pf, serde_json::to_string(plan).unwrap_or_default()).map_err(|e| e.to_string())?;
            pushs(&mut a, &["reel", &video, "--plan", "out/videos/.plan.json", "--out", "out/videos"]);
        }
        "ticker" => {
            let f = get_s(p, "file");
            need(&f, trs!("Datei", "file", "fichier", "archivo"))?;
            pushs(&mut a, &["ticker", &f]);
            if dir.join("court.json").exists() {
                pushs(&mut a, &["--court", "court.json"]);
            }
            ticker_names(&mut a, p);
            direction_args(&mut a, p);
        }
        "stats" => {
            let f = get_s(p, "file");
            need(&f, trs!("Datei", "file", "fichier", "archivo"))?;
            pushs(&mut a, &["stats", &f]);
            if dir.join("court.json").exists() {
                pushs(&mut a, &["--court", "court.json"]);
            }
        }
        other => return Err(tr!("unbekannter Auftrag: {other}", "unknown job: {other}", "tâche inconnue : {other}", "tarea desconocida: {other}")),
    }
    // the job reports in the language of the page (calibrate already carries it for its own page)
    if kind != "calibrate" && kind != "scoreboard_calibrate" {
        if let Some(l) = p["lang"].as_str().and_then(super::i18n::Lang::parse) {
            pushs(&mut a, &["--lang", l.code()]);
        }
    }
    Ok(a)
}

fn pushs(a: &mut Vec<String>, xs: &[&str]) {
    xs.iter().for_each(|x| a.push(x.to_string()));
}

fn start_job(ui: &Arc<Ui>, dir: &Path, kind: &str, params: &Value) -> Result<u64, String> {
    let args = build_args(kind, dir, params)?;
    // two jobs writing the same files ruin both results (the second one empties them)
    let outputs = job_outputs(dir, &args);
    if let Some(busy) = ui.jobs.lock().unwrap().values().find_map(|j| {
        let st = j.status.lock().unwrap().clone();
        (st == "running" || st == "cancelling").then(|| j.outputs.iter().find(|o| outputs.contains(o)).cloned()).flatten()
    }) {
        let name = busy.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(tr!("Ein laufender Auftrag schreibt schon in {name}: erst abwarten oder abbrechen, oder einen anderen Namen wählen.", "A running job already writes {name}: wait for it or cancel it, or choose another name.", "Une tâche en cours écrit déjà {name} : attendre ou l'annuler, ou choisir un autre nom.", "Una tarea en curso ya escribe {name}: espera o cancélala, o elige otro nombre."));
    }
    let id = ui.next.fetch_add(1, Ordering::SeqCst);
    let stop_file = std::env::temp_dir().join(format!("reco-minimap-stop-{}-{id}", std::process::id()));
    let _ = std::fs::remove_file(&stop_file);
    let mut child = Command::new(&ui.exe).args(&args).current_dir(dir).env("RECO_MINIMAP_PROGRESS", "1").env("RECO_MINIMAP_STOP", &stop_file).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| tr!("Start fehlgeschlagen: {e}", "could not start: {e}", "échec du démarrage : {e}", "no se pudo iniciar: {e}"))?;
    let job = Arc::new(Job {
        kind: kind.to_string(),
        lines: Mutex::new(vec![(format!("$ reco-minimap {}", args.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.clone() }).collect::<Vec<_>>().join(" ")), false)]),
        status: Mutex::new("running".into()),
        code: Mutex::new(None),
        child: Mutex::new(None),
        progress: Mutex::new(Value::Null),
        first_progress: Mutex::new(None),
        started: std::time::Instant::now(),
        outputs,
        stop_file,
        kept: Mutex::new(None),
    });
    let (so, se) = (child.stdout.take(), child.stderr.take());
    *job.child.lock().unwrap() = Some(child);
    for r in [so.map(|x| Box::new(x) as Box<dyn Read + Send>), se.map(|x| Box::new(x) as Box<dyn Read + Send>)].into_iter().flatten() {
        let j = job.clone();
        std::thread::spawn(move || pump(r, j));
    }
    {
        let j = job.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            let mut g = j.child.lock().unwrap();
            if let Some(c) = g.as_mut() {
                if let Ok(Some(st)) = c.try_wait() {
                    *j.code.lock().unwrap() = st.code();
                    let _ = std::fs::remove_file(&j.stop_file);
                    let mut status = j.status.lock().unwrap();
                    match status.as_str() {
                        // stopped gently: the files are complete up to the stop
                        "cancelling" => *status = "cancelled".into(),
                        "cancelled" => {
                            if *j.kept.lock().unwrap() == Some(false) {
                                for f in &j.outputs {
                                    let _ = std::fs::remove_file(f);
                                }
                            }
                        }
                        _ => *status = if st.success() { "done".into() } else { "failed".into() },
                    }
                    return;
                }
            } else {
                return;
            }
        });
    }
    ui.jobs.lock().unwrap().insert(id, job);
    Ok(id)
}

// ---------------------------------------------------------------- routes

/// Ports of the pages served by running calibration jobs (from the address they print).
fn embedded_ports(ui: &Ui) -> Vec<u16> {
    let jobs = ui.jobs.lock().unwrap();
    jobs.values()
        .filter(|j| (j.kind == "calibrate" || j.kind == "scoreboard_calibrate") && matches!(j.status.lock().unwrap().as_str(), "running" | "cancelling"))
        .flat_map(|j| {
            j.lines.lock().unwrap().iter().filter_map(|(t, _)| {
                let rest = &t[t.find("http://127.0.0.1:")? + "http://127.0.0.1:".len()..];
                rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
            }).collect::<Vec<u16>>()
        })
        .collect()
}

/// Forward one request to a local page and hand its answer back unchanged.
fn proxy(s: &mut TcpStream, port: u16, method: &str, target: &str, body: &str) -> std::io::Result<()> {
    let mut up = TcpStream::connect(("127.0.0.1", port))?;
    up.set_read_timeout(Some(Duration::from_secs(120)))?;
    write!(up, "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    let mut answer = Vec::new();
    up.read_to_end(&mut answer)?;
    if answer.is_empty() {
        return Err(std::io::Error::other("empty answer"));
    }
    s.write_all(&answer)
}

fn handle(mut s: TcpStream, ui: Arc<Ui>) {
    let (method, path, query, body, head) = match read_request(&mut s) {
        Some(r) => r,
        None => return,
    };
    if !request_allowed(header(&head, "host"), header(&head, "origin")) {
        return respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
    }
    let b: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    // answer in the language of the page that asked (it sends it with every request)
    if let Some(l) = b["lang"].as_str().and_then(super::i18n::Lang::parse) {
        super::i18n::set_lang(l);
    }
    // a calibration page of a running job, passed through on this port so it can be shown inside the
    // interface (the Mac app's window only loads pages from the interface's own port)
    if let Some(rest) = path.strip_prefix("/tool/") {
        let (port, sub) = rest.split_once('/').unwrap_or((rest, ""));
        return match port.parse::<u16>().ok().filter(|p| embedded_ports(&ui).contains(p)) {
            Some(port) => {
                if proxy(&mut s, port, &method, &format!("/{sub}{}", if query.is_empty() { String::new() } else { format!("?{query}") }), &body).is_err() {
                    respond(&mut s, "502 Bad Gateway", "text/plain", b"calibration page not reachable");
                }
            }
            None => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
        };
    }
    match (method.as_str(), path.as_str()) {
        // the React interface (web/, built into webui/ by `npm run build`)
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("webui/index.html").as_bytes()),
        ("GET", "/app.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("webui/app.js").as_bytes()),
        ("GET", "/app.css") => respond(&mut s, "200 OK", "text/css; charset=utf-8", include_str!("webui/app.css").as_bytes()),
        ("GET", "/theme.css") => respond(&mut s, "200 OK", "text/css; charset=utf-8", include_str!("theme.css").as_bytes()),
        ("GET", "/guide") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("guide.html").as_bytes()),
        ("GET", "/guide.md") => {
            let lang = query.split('&').find_map(|kv| kv.strip_prefix("lang=")).and_then(super::i18n::Lang::parse).unwrap_or_else(super::i18n::lang);
            respond(&mut s, "200 OK", "text/markdown; charset=utf-8", guide(lang).as_bytes())
        }
        ("GET", p) if p.starts_with("/docs/") => match guide_asset(&p[6..]) {
            Some((data, kind)) => respond(&mut s, "200 OK", kind, data),
            None => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
        },
        ("GET", "/review") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("review.html").as_bytes()),
        ("GET", "/review.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("review.js").as_bytes()),
        ("GET", "/api/review/sports") => {
            let sports: Vec<Value> = BUILTIN_IDS.iter().filter_map(|id| SportProfile::builtin(id)).map(|p| json!({"id":p.id,"field":[p.field.length,p.field.width],"lines":p.lines.iter().map(|line| line.iter().map(|v| [v.x,v.y]).collect::<Vec<_>>()).collect::<Vec<_>>()})).collect();
            jres(&mut s, json!({"sports":sports}));
        }
        ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
        // the ticker page reading a liveticker.json of a project (simulation from a processed video)
        ("GET", "/ticker") => respond(&mut s, "200 OK", "text/html; charset=utf-8", super::ticker_pub::page_for_hub().as_bytes()),
        ("GET", "/ticker.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", super::ticker_pub::script().as_bytes()),
        ("GET", "/api/ticker-feed") => {
            let file = qparam(&query, "file").map(|v| pct_decode(&v)).unwrap_or_default();
            match std::fs::read(&file) {
                Ok(b) if file.ends_with(".liveticker.json") && Path::new(&file).is_absolute() => respond(&mut s, "200 OK", "application/json; charset=utf-8", &b),
                _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
            }
        }
        ("GET", "/api/init") => {
            let sports: Vec<Value> = BUILTIN_IDS.iter().filter_map(|id| SportProfile::builtin(id)).map(|p| json!({"id": p.id, "field": [p.field.length, p.field.width], "team_size": p.team_size, "referees": p.referees, "keepers": crate::sport::has_keepers(&p.id)})).collect();
            let ort = super::infer::ensure_ort_runtime().err();
            jres(&mut s, json!({"ok": true, "home": home_dir().to_string_lossy(), "default_dir": default_project_dir().to_string_lossy(), "sports": sports,
                                "coreml": cfg!(feature = "coreml"), "camera": {"id": models::CAMERA_ID, "mb": (models::CAMERA_MB as f64 * 10.0).round() / 10.0, "exists": models::camera_model().is_some()}, "ocr": {"path": super::ocr::default_model_path().to_string_lossy(), "exists": super::ocr::default_model_path().exists()}, "ort_error": ort, "version": env!("CARGO_PKG_VERSION"), "lan_ip": super::board_srv::lan_address(), "dialogs": cfg!(target_os = "macos") || cfg!(target_os = "windows") || Command::new("zenity").arg("--version").output().is_ok()}));
        }
        ("GET", "/api/changelog") => {
            // notes per version and language (shown after an update and in "About Reco Minimap")
            let notes: Value = serde_json::from_str(include_str!("changelog.json")).unwrap_or_else(|_| json!({}));
            jres(&mut s, json!({"ok": true, "version": env!("CARGO_PKG_VERSION"), "notes": notes}));
        }
        ("GET", "/api/update") => {
            let u = ui.update.lock().unwrap().clone();
            jres(&mut s, json!({"ok": true, "current": super::update::CURRENT, "update": u}));
        }
        ("POST", "/api/project/load") => {
            let dir = get_s(&b, "dir");
            let dir = if dir.is_empty() { default_project_dir() } else { PathBuf::from(dir) };
            match load_project(&dir) {
                Ok(v) => jres(&mut s, v),
                Err(e) => err(&mut s, e),
            }
        }
        ("POST", "/api/project/save") => {
            let dir = PathBuf::from(get_s(&b, "dir"));
            match std::fs::write(dir.join("project.json"), serde_json::to_string_pretty(&b["project"]).unwrap_or_default()) {
                Ok(()) => jres(&mut s, json!({"ok": true})),
                Err(e) => err(&mut s, e),
            }
        }
        ("POST", "/api/teams/save") => {
            let dir = PathBuf::from(get_s(&b, "dir"));
            let mut v = json!({"home": get_s(&b, "home"), "away": get_s(&b, "away")});
            // goalkeeper colours are optional (an empty value removes them) and only kept for sports with goalkeepers
            let sport = std::fs::read_to_string(dir.join("project.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|p| get_s(&p, "sport")).unwrap_or_default();
            let keepers = sport.is_empty() || crate::sport::has_keepers(&sport);
            for k in ["home_gk", "away_gk"] {
                let c = get_s(&b, k);
                if keepers && c.len() == 7 && c.starts_with('#') {
                    v[k] = json!(c);
                }
            }
            match std::fs::write(dir.join("teams.json"), serde_json::to_string_pretty(&v).unwrap()) {
                Ok(()) => jres(&mut s, json!({"ok": true})),
                Err(e) => err(&mut s, e),
            }
        }
        // for the Mac launcher: may it quit / install an update without cancelling work?
        ("GET", "/api/status") => {
            let busy = ui.jobs.lock().unwrap().values().any(|j| matches!(j.status.lock().unwrap().as_str(), "running" | "cancelling"));
            jres(&mut s, json!({"ok": true, "busy": busy}));
        }
        ("POST", "/api/basket/strip") => match basket_strip(&get_s(&b, "video"), b["t"].as_f64().unwrap_or(0.0), b["u"].as_f64().unwrap_or(0.5)) {
            Ok(jpg) => respond(&mut s, "200 OK", "image/jpeg", &jpg),
            Err(e) => err(&mut s, e),
        },
        ("POST", "/api/board/moments") => jres(&mut s, json!({"ok": true, "moments": board_moments(Path::new(&get_s(&b, "dir")), &get_s(&b, "file"))})),
        ("POST", "/api/stats") => match stats_for(Path::new(&get_s(&b, "dir")), &get_s(&b, "file"), b.get("fixes")) {
            Ok(v) => jres(&mut s, json!({"ok": true, "stats": v})),
            Err(e) => err(&mut s, e),
        },
        ("POST", "/api/dialog") => {
            let p = native_dialog(&get_s(&b, "kind"), &get_s(&b, "prompt"));
            jres(&mut s, json!({"ok": true, "path": p}));
        }
        ("POST", "/api/reveal") => match reveal(&get_s(&b, "path"), b["open"].as_bool().unwrap_or(false)) {
            Ok(()) => jres(&mut s, json!({"ok": true})),
            Err(e) => err(&mut s, e),
        },
        ("POST", "/api/probe") => match video::probe(&get_s(&b, "path")) {
            Ok(i) => jres(&mut s, json!({"ok": true, "w": i.w, "h": i.h, "fps": (i.fps * 100.0).round() / 100.0, "duration": i.duration})),
            Err(e) => err(&mut s, e),
        },
        ("POST", "/api/hub") => jres(&mut s, hub_models(&get_s(&b, "sport"))),
        ("POST", "/api/model/import") => {
            let dir = PathBuf::from(get_s(&b, "dir"));
            match models::import(Path::new(&get_s(&b, "path")), &dir.join("models")) {
                Ok(m) => jres(&mut s, json!({"ok": true, "model": m.to_json()})),
                Err(e) => err(&mut s, e),
            }
        }
        ("POST", "/api/job/start") => {
            let dir = PathBuf::from(get_s(&b, "dir"));
            let mut params = b["params"].clone();
            if params.is_object() && params["lang"].is_null() {
                params["lang"] = b["lang"].clone();
            }
            match start_job(&ui, &dir, &get_s(&b, "kind"), &params) {
                Ok(id) => jres(&mut s, json!({"ok": true, "id": id})),
                Err(e) => err(&mut s, e),
            }
        }
        // jobs still running, so that a reloaded page shows them again
        ("GET", "/api/jobs") => {
            let jobs = ui.jobs.lock().unwrap();
            let mut list: Vec<Value> = jobs.iter().filter(|(_, j)| matches!(j.status.lock().unwrap().as_str(), "running" | "cancelling")).map(|(id, j)| {
                let mut v = job_json(*id, j);
                v["kind"] = json!(j.kind);
                v
            }).collect();
            list.sort_by_key(|v| v["id"].as_u64());
            jres(&mut s, json!({"ok": true, "jobs": list}));
        }
        ("GET", p) if p.starts_with("/api/job/") => {
            let id: u64 = p.trim_start_matches("/api/job/").trim_end_matches("/cancel").parse().unwrap_or(0);
            let job = ui.jobs.lock().unwrap().get(&id).cloned();
            match job {
                Some(j) => {
                    let from: usize = qparam(&query, "from").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let lines = j.lines.lock().unwrap();
                    let new: Vec<Value> = lines.iter().enumerate().skip(from).map(|(_, (t, pr))| json!({"t": t, "p": pr})).collect();
                    // progress lines get replaced, so always resend the last one
                    let (from_eff, new) = if from > 0 && from <= lines.len() && lines.last().map(|x| x.1).unwrap_or(false) && new.is_empty() {
                        (lines.len() - 1, vec![json!({"t": lines.last().unwrap().0, "p": true})])
                    } else {
                        (from, new)
                    };
                    let mut v = job_json(id, &j);
                    v["ok"] = json!(true);
                    v["from"] = json!(from_eff);
                    v["total"] = json!(lines.len());
                    v["lines"] = json!(new);
                    jres(&mut s, v);
                }
                None => err(&mut s, trs!("Auftrag unbekannt", "unknown job", "tâche inconnue", "tarea desconocida")),
            }
        }
        ("POST", p) if p.starts_with("/api/job/") && p.ends_with("/cancel") => {
            let id: u64 = p.trim_start_matches("/api/job/").trim_end_matches("/cancel").trim_end_matches('/').parse().unwrap_or(0);
            if let Some(j) = ui.jobs.lock().unwrap().get(&id).cloned() {
                cancel_job(&j, b["keep"].as_bool().unwrap_or(false));
            }
            jres(&mut s, json!({"ok": true}));
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Cancels a job. `keep`: ask the command to stop at the next frame and finish its files (killed
/// after 30 s if it does not); otherwise stop it at once and remove the files it was writing.
fn cancel_job(j: &Arc<Job>, keep: bool) {
    let mut status = j.status.lock().unwrap();
    if *status != "running" && !(*status == "cancelling" && !keep) {
        return;
    }
    if keep && std::fs::write(&j.stop_file, b"stop").is_ok() {
        *status = "cancelling".into();
        *j.kept.lock().unwrap() = Some(true);
        let j = j.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            if *j.status.lock().unwrap() == "cancelling" {
                if let Some(c) = j.child.lock().unwrap().as_mut() {
                    let _ = c.kill();
                }
            }
        });
        return;
    }
    *status = "cancelled".into();
    *j.kept.lock().unwrap() = Some(false);
    if let Some(c) = j.child.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

/// The running interface, so that quitting the program can stop its jobs.
static RUNNING: std::sync::OnceLock<Arc<Ui>> = std::sync::OnceLock::new();

/// Stops every running job (used when the Mac app quits).
pub fn cancel_all() {
    if let Some(ui) = RUNNING.get() {
        for job in ui.jobs.lock().unwrap().values() {
            if let Some(child) = job.child.lock().unwrap().as_mut() {
                let _ = child.kill();
            }
        }
    }
}

/// True when the program is the Mac app: it is signed as a whole and cannot rebuild itself.
fn is_mac_app() -> bool {
    cfg!(target_os = "macos") && std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false)
}

/// Start the UI server; returns the URL (blocks forever if `block`).
pub fn serve(port: u16, open: bool, block: bool, update_check: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut listener = None;
    for p in port..port.saturating_add(10) {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.ok_or(trs!("Kein freier Port (--port ändern)", "No free port (change --port)", "Aucun port libre (changer --port)", "No hay ningún puerto libre (cambia --port)"))?;
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().map_err(|e| e.to_string())?.port());
    let ui = Arc::new(Ui { exe, jobs: Mutex::new(HashMap::new()), next: AtomicU64::new(1), update: Mutex::new(None) });
    let _ = RUNNING.set(ui.clone());
    if update_check && std::env::var_os("RECO_MINIMAP_NO_UPDATE_CHECK").is_none() {
        // in the background: the page must not wait for the network
        let u = ui.clone();
        std::thread::spawn(move || {
            let found = super::update::available().ok().flatten();
            *u.update.lock().unwrap() = Some(json!({"checked": true, "latest": found.map(|l| json!({"version": l.version, "url": l.url}))}));
        });
    }
    let accept = move || {
        for s in listener.incoming().flatten() {
            let u = ui.clone();
            std::thread::spawn(move || handle(s, u));
        }
    };
    if open {
        let cmd = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
        let _ = Command::new(cmd).arg(&url).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }
    if block {
        accept();
    } else {
        std::thread::spawn(accept);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(kind: &str, figures: Value) -> Vec<String> {
        let p = json!({"model_dir": "/m", "video": "/v.mp4", "source": "/v.mp4", "ep": "cpu", "every": 5, "name": "a",
            "outputs": {"json": true}, "size": "1920x1080", "fps": 25, "port": 8765, "figures": figures});
        build_args(kind, Path::new("/nonexistent-project"), &p).unwrap()
    }

    fn pair(a: &[String], flag: &str) -> Option<String> {
        a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1).cloned())
    }

    #[test]
    fn figure_options_reach_the_commands() {
        let fig = json!({"torso_top": 0.3, "torso_bottom": 0.65, "shirt_match": true, "no_referees": true});
        for kind in ["run", "live"] {
            let a = args(kind, fig.clone());
            assert_eq!(pair(&a, "--torso").as_deref(), Some("0.3,0.65"), "{a:?}");
            assert!(a.contains(&"--shirt-match".to_string()));
            assert_eq!(pair(&a, "--referees").as_deref(), Some("0"));
        }
        // colour detection only needs the torso band
        let a = args("colors", fig.clone());
        assert_eq!(pair(&a, "--torso").as_deref(), Some("0.3,0.65"));
        assert!(!a.contains(&"--shirt-match".to_string()) && pair(&a, "--referees").is_none());
        assert!(pair(&args("scales", fig), "--torso").is_none());
    }

    #[test]
    fn the_board_job_opens_a_saved_board_and_cuts_a_new_moment() {
        let d = std::env::temp_dir().join(format!("reco-ui-board-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("out")).unwrap();
        let p = json!({"file": "/p/game one.jsonl", "at": "12:30", "trail": 4, "lang": "de", "draw_home": "#ffffff"});
        assert!(build_args("board", &d, &p).is_err(), "no calibration yet");
        std::fs::write(d.join("court.json"), "{}").unwrap();
        let a = build_args("board", &d, &p).unwrap();
        assert_eq!(a[0], "board");
        assert!(a[1].ends_with("game_one-board-12m30.json"), "{a:?}");
        assert_eq!(pair(&a, "--states").as_deref(), Some("/p/game one.jsonl"));
        assert_eq!(pair(&a, "--at").as_deref(), Some("12:30"));
        assert_eq!(pair(&a, "--trail").as_deref(), Some("4"));
        assert_eq!(pair(&a, "--draw-home").as_deref(), Some("#ffffff"));
        assert_eq!(pair(&a, "--port").as_deref(), Some("0"));
        assert_eq!(pair(&a, "--lang").as_deref(), Some("de"));
        // once saved, the board opens as the coach left it instead of being cut again
        std::fs::write(&a[1], "{}").unwrap();
        let b = build_args("board", &d, &p).unwrap();
        assert!(!b.contains(&"--states".to_string()), "{b:?}");
        assert!(build_args("board", &d, &json!({"file": "/x.jsonl"})).is_err(), "moment is required");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_live_job_passes_the_ticker_settings() {
        let p = json!({"model_dir": "/m", "source": "/v.mp4", "ep": "cpu", "size": "1920x1080", "fps": 25, "port": 8765,
            "ticker_names": "both", "home_name": " Blue ", "away_name": "", "network": true, "ticker_approve": true, "ticker_export": true, "ticker_push": " https://dav.example.org/t.json ", "ticker_key": ""});
        let a = build_args("live", Path::new("/nonexistent-project"), &p).unwrap();
        assert_eq!(pair(&a, "--ticker-names").as_deref(), Some("both"));
        assert_eq!(pair(&a, "--home-name").as_deref(), Some("Blue"));
        assert!(pair(&a, "--away-name").is_none(), "an empty name is not passed");
        assert_eq!(pair(&a, "--host").as_deref(), Some("0.0.0.0"));
        assert!(a.contains(&"--ticker-approve".to_string()));
        assert!(pair(&a, "--ticker-export").unwrap().ends_with("ticker"));
        assert_eq!(pair(&a, "--ticker-push").as_deref(), Some("https://dav.example.org/t.json"));
        assert!(pair(&a, "--ticker-key").is_none(), "an empty key is not passed");
        // the safe default: nothing about names, local only
        let b = build_args("live", Path::new("/nonexistent-project"), &json!({"model_dir": "/m", "source": "/v.mp4", "ep": "cpu", "size": "1920x1080", "fps": 25, "port": 8765, "ticker_names": "evil"})).unwrap();
        assert!(!b.contains(&"--ticker-names".to_string()) && !b.contains(&"--host".to_string()) && !b.contains(&"--ticker-approve".to_string()) && !b.contains(&"--ticker-export".to_string()) && !b.contains(&"--ticker-push".to_string()), "{b:?}");
    }

    #[test]
    fn jobs_report_in_the_page_language() {
        let p = json!({"model_dir": "/m", "video": "/v.mp4", "ep": "cpu", "every": 5, "name": "a", "outputs": {"json": true}, "lang": "fr"});
        for kind in ["run", "colors", "scales", "selfcheck"] {
            let a = build_args(kind, Path::new("/nonexistent-project"), &p).unwrap();
            assert_eq!(pair(&a, "--lang").as_deref(), Some("fr"), "{kind}: {a:?}");
        }
        assert_eq!(pair(&build_args("update", Path::new("/x"), &json!({"lang": "es"})).unwrap(), "--lang").as_deref(), Some("es"));
        // an unknown language is not passed on; calibrate keeps its own single --lang
        assert!(pair(&build_args("update", Path::new("/x"), &json!({"lang": "xx"})).unwrap(), "--lang").is_none());
        let c = build_args("calibrate", Path::new("/x"), &json!({"video": "/v.mp4", "sport": "basketball", "time": 5, "lang": "es"})).unwrap();
        let sb = build_args("scoreboard_calibrate", Path::new("/x"), &json!({"video": "/v.mp4", "time": 60, "lang": "fr"})).unwrap();
        assert_eq!(sb, ["scoreboard-calibrate", "/v.mp4", "--time", "60", "--out", "scoreboard.json", "--lang", "fr", "--no-open"]);
        assert_eq!(c.iter().filter(|x| *x == "--lang").count(), 1);
        assert!(!c.iter().any(|x| x == "--lens"));
        let f = build_args("calibrate", Path::new("/x"), &json!({"video": "/v.mp4", "sport": "basketball", "time": 5, "lang": "de", "lens": "panorama"})).unwrap();
        assert_eq!(pair(&f, "--lens").as_deref(), Some("panorama"));
        let old = build_args("calibrate", Path::new("/x"), &json!({"video": "/v.mp4", "sport": "basketball", "time": 5, "lang": "de", "fisheye": true})).unwrap();
        assert_eq!(pair(&old, "--lens").as_deref(), Some("fisheye"));
    }

    #[test]
    fn reusable_evidence_reaches_run_jobs() {
        let p = json!({"model_dir":"/main","video":"/video.mp4","ep":"cpu","every":1,"name":"trial","outputs":{"evidence":true}});
        let a = build_args("run", Path::new("/project"), &p).unwrap();
        assert!(pair(&a, "--dets").unwrap().ends_with("trial.dets.jsonl"));
        assert!(pair(&a, "--metrics").unwrap().ends_with("trial.metrics.json"));
        assert_eq!(job_outputs(Path::new("/project"), &a).len(), 2);
    }

    #[test]
    fn defaults_and_bad_torso_values_add_nothing() {
        let a = args("run", json!({"torso_top": 0.18, "torso_bottom": 0.5, "shirt_match": false, "no_referees": false}));
        assert_eq!(pair(&a, "--torso").as_deref(), Some("0.18,0.5"));
        assert!(!a.contains(&"--shirt-match".to_string()) && pair(&a, "--referees").is_none());
        // a project saved before these options existed, and an unusable range
        for fig in [Value::Null, json!({"torso_top": 0.7, "torso_bottom": 0.4}), json!({"torso_top": "x"})] {
            let a = args("run", fig);
            assert!(pair(&a, "--torso").is_none() && !a.contains(&"--shirt-match".to_string()), "{a:?}");
        }
    }
}

/// Models that Reco Trainer's public model hub (github.com/wendibus/reco-models) lists for a sport.
/// Only shown, not downloaded (loading from the hub comes later). The index is cached for 30 minutes.
fn hub_models(sport: &str) -> Value {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Value)>> = std::sync::Mutex::new(None);
    let index = {
        let mut c = CACHE.lock().unwrap();
        match &*c {
            Some((t, v)) if t.elapsed().as_secs() < 1800 => Some(v.clone()),
            _ => {
                let url = std::env::var("RECO_HUB_INDEX_URL").unwrap_or_else(|_| "https://github.com/wendibus/reco-models/releases/download/index/index.json".into());
                let out = Command::new("curl").args(["-fsSL", "--max-time", "8", &url]).output().ok().filter(|o| o.status.success());
                let v = out.and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
                if let Some(v) = &v {
                    *c = Some((std::time::Instant::now(), v.clone()));
                }
                v
            }
        }
    };
    let Some(index) = index else { return json!({"ok": false}) };
    let found: Vec<Value> = index["models"].as_array().into_iter().flatten()
        .filter(|m| m["sport"].as_str() == Some(sport) && !m["deprecated"].as_bool().unwrap_or(false))
        .map(|m| json!({"id": m["id"], "name": m["name"], "size": m["size"], "version": m["version"], "classes": m["classes"]}))
        .collect();
    json!({"ok": true, "models": found, "url": "https://github.com/wendibus/reco-models"})
}

/// `--ticker-names`, `--home-name`, `--away-name` from a job's parameters (live ticker and its simulation).
fn ticker_names(a: &mut Vec<String>, p: &Value) {
    if matches!(get_s(p, "ticker_names").as_str(), "numbers" | "names" | "both") {
        pushs(a, &["--ticker-names", &get_s(p, "ticker_names")]);
    }
    for (flag, key) in [("--home-name", "home_name"), ("--away-name", "away_name")] {
        if !get_s(p, key).trim().is_empty() {
            pushs(a, &[flag, get_s(p, key).trim()]);
        }
    }
}

/// Decode `%XX` and `+` of a query value.
fn pct_decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'%', Some(x)) => {
                out.push(x);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (c, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod pct_tests {
    #[test]
    fn decodes_paths_with_spaces_and_umlauts() {
        assert_eq!(super::pct_decode("%2FUsers%2Fa%20b%2Fsp%C3%BCl+x.liveticker.json"), "/Users/a b/spül x.liveticker.json");
        assert_eq!(super::pct_decode("100%"), "100%");
    }
}
