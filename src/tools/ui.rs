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
    lines: Mutex<Vec<(String, bool)>>,
    status: Mutex<String>, // running | done | failed | cancelled
    code: Mutex<Option<i32>>,
    child: Mutex<Option<Child>>,
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
        "img/en/calibration.webp" => include_bytes!("../../docs/img/en/calibration.webp"),
        "img/en/step1.webp" => include_bytes!("../../docs/img/en/step1.webp"),
        "img/en/step2.webp" => include_bytes!("../../docs/img/en/step2.webp"),
        "img/en/step3-colours.webp" => include_bytes!("../../docs/img/en/step3-colours.webp"),
        "img/en/step3.webp" => include_bytes!("../../docs/img/en/step3.webp"),
        "img/en/step4-results.webp" => include_bytes!("../../docs/img/en/step4-results.webp"),
        "img/en/step4.webp" => include_bytes!("../../docs/img/en/step4.webp"),
        "img/en/step5.webp" => include_bytes!("../../docs/img/en/step5.webp"),
        "img/en/step6.webp" => include_bytes!("../../docs/img/en/step6.webp"),
        "img/es/calibration.webp" => include_bytes!("../../docs/img/es/calibration.webp"),
        "img/es/step1.webp" => include_bytes!("../../docs/img/es/step1.webp"),
        "img/es/step2.webp" => include_bytes!("../../docs/img/es/step2.webp"),
        "img/es/step3-colours.webp" => include_bytes!("../../docs/img/es/step3-colours.webp"),
        "img/es/step3.webp" => include_bytes!("../../docs/img/es/step3.webp"),
        "img/es/step4-results.webp" => include_bytes!("../../docs/img/es/step4-results.webp"),
        "img/es/step4.webp" => include_bytes!("../../docs/img/es/step4.webp"),
        "img/es/step5.webp" => include_bytes!("../../docs/img/es/step5.webp"),
        "img/es/step6.webp" => include_bytes!("../../docs/img/es/step6.webp"),
        "img/fr/calibration.webp" => include_bytes!("../../docs/img/fr/calibration.webp"),
        "img/fr/step1.webp" => include_bytes!("../../docs/img/fr/step1.webp"),
        "img/fr/step2.webp" => include_bytes!("../../docs/img/fr/step2.webp"),
        "img/fr/step3-colours.webp" => include_bytes!("../../docs/img/fr/step3-colours.webp"),
        "img/fr/step3.webp" => include_bytes!("../../docs/img/fr/step3.webp"),
        "img/fr/step4-results.webp" => include_bytes!("../../docs/img/fr/step4-results.webp"),
        "img/fr/step4.webp" => include_bytes!("../../docs/img/fr/step4.webp"),
        "img/fr/step5.webp" => include_bytes!("../../docs/img/fr/step5.webp"),
        "img/fr/step6.webp" => include_bytes!("../../docs/img/fr/step6.webp"),
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
    let court_v = CourtCalib::load(&court.to_string_lossy()).ok().map(|c| json!({"points": c.points.len(), "rms": c.rms_m, "sport": c.sport, "pano": [c.pano_w, c.pano_h]}));
    let rd = |n: &str| std::fs::read_to_string(dir.join(n)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let mut outs = Vec::new();
    if let Ok(rdir) = std::fs::read_dir(dir.join("out")) {
        for e in rdir.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
            if matches!(ext, "mp4" | "mov" | "jsonl") {
                let m = e.metadata().ok();
                outs.push(json!({"name": e.file_name().to_string_lossy(), "path": p.to_string_lossy(), "mb": m.as_ref().map(|m| (m.len() as f64 / 1e5).round() / 10.0), "t": m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs())}));
            }
        }
    }
    outs.sort_by(|a, b| b["t"].as_u64().cmp(&a["t"].as_u64()));
    json!({"court": court_v, "scales": rd("scales.json").map(|v| json!({"n": v["n"]})), "teams": rd("teams.json"), "out": outs})
}

fn load_project(dir: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(dir.join("models")).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("out")).map_err(|e| e.to_string())?;
    let proj = std::fs::read_to_string(dir.join("project.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_else(|| json!({}));
    let models: Vec<Value> = models::list(&dir.join("models")).iter().map(|m| m.to_json()).collect();
    Ok(json!({"ok": true, "dir": dir.to_string_lossy(), "project": proj, "models": models, "files": project_files(dir)}))
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

fn pump(mut r: impl Read, job: Arc<Job>) {
    let (mut cur, mut buf) = (Vec::new(), [0u8; 4096]);
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !cur.is_empty() {
                            push_line(&job, String::from_utf8_lossy(&cur).to_string(), b == b'\r');
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
        push_line(&job, String::from_utf8_lossy(&cur).to_string(), false);
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
            pushs(&mut a, &["calibrate", &video, "--sport", &get_s(p, "sport"), "--time", &get_s(p, "time"), "--out", "court.json", "--lang", &get_s(p, "lang")]);
            if get_s(p, "sport") == "custom" {
                pushs(&mut a, &["--length", &get_s(p, "length"), "--width", &get_s(p, "width")]);
            }
            for k in ["team_size", "referees"] {
                if !get_s(p, k).is_empty() {
                    pushs(&mut a, &[&format!("--{}", k.replace('_', "-")), &get_s(p, k)]);
                }
            }
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
            pushs(&mut a, &[kind, &src, "--model", &model, "--court", "court.json", "--teams", "teams.json", "--tile", if has_scales { "auto" } else { "512" }]);
            engine(&mut a);
            for (flag, key) in [("--draw-home", "draw_home"), ("--draw-away", "draw_away"), ("--ref-color", "ref_color"), ("--min-alpha", "min_alpha")] {
                if !get_s(p, key).is_empty() {
                    pushs(&mut a, &[flag, &get_s(p, key)]);
                }
            }
            if p["scenes"].as_bool() == Some(true) {
                pushs(&mut a, &["--scenes"]);
            }
            if let Some(t) = &torso {
                pushs(&mut a, &["--torso", t]);
            }
            if fig["shirt_match"].as_bool() == Some(true) {
                pushs(&mut a, &["--shirt-match"]);
            }
            if fig["no_referees"].as_bool() == Some(true) {
                pushs(&mut a, &["--referees", "0"]);
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
                let out = |ext: &str| dir.join("out").join(format!("{name}{ext}")).to_string_lossy().into_owned();
                let o = &p["outputs"];
                let (mut any, mut paths) = (false, Vec::new());
                if o["minimap"].as_bool() == Some(true) { paths.push(("--minimap", out(".mp4"))); }
                if o["alpha"].as_bool() == Some(true) { paths.push(("--alpha", out(".mov"))); if !get_s(p, "alpha_bg").is_empty() { paths.push(("--alpha-bg", get_s(p, "alpha_bg"))); } }
                if o["pip"].as_bool() == Some(true) { paths.push(("--pip", out("_pip.mp4"))); }
                if o["json"].as_bool() == Some(true) { paths.push(("--json", out(".jsonl"))); }
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
        "update" => pushs(&mut a, &["update", "--yes"]),
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
    if kind != "calibrate" {
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
    let mut child = Command::new(&ui.exe).args(&args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| tr!("Start fehlgeschlagen: {e}", "could not start: {e}", "échec du démarrage : {e}", "no se pudo iniciar: {e}"))?;
    let id = ui.next.fetch_add(1, Ordering::SeqCst);
    let job = Arc::new(Job { lines: Mutex::new(vec![(format!("$ reco-minimap {}", args.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.clone() }).collect::<Vec<_>>().join(" ")), false)]), status: Mutex::new("running".into()), code: Mutex::new(None), child: Mutex::new(None) });
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
                    let cancelled = *j.status.lock().unwrap() == "cancelled";
                    *j.code.lock().unwrap() = st.code();
                    if !cancelled {
                        *j.status.lock().unwrap() = if st.success() { "done".into() } else { "failed".into() };
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
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("ui.html").as_bytes()),
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
        ("GET", "/api/init") => {
            let sports: Vec<Value> = BUILTIN_IDS.iter().filter_map(|id| SportProfile::builtin(id)).map(|p| json!({"id": p.id, "field": [p.field.length, p.field.width], "team_size": p.team_size, "referees": p.referees, "keepers": crate::sport::has_keepers(&p.id)})).collect();
            let ort = super::infer::ensure_ort_runtime().err();
            jres(&mut s, json!({"ok": true, "home": home_dir().to_string_lossy(), "default_dir": default_project_dir().to_string_lossy(), "sports": sports,
                                "coreml": cfg!(feature = "coreml"), "ocr": {"path": super::ocr::default_model_path().to_string_lossy(), "exists": super::ocr::default_model_path().exists()}, "ort_error": ort, "version": env!("CARGO_PKG_VERSION"), "lan_ip": super::board_srv::lan_address(), "dialogs": cfg!(target_os = "macos") || cfg!(target_os = "windows") || Command::new("zenity").arg("--version").output().is_ok()}));
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
            // goalkeeper colours are optional: an empty value removes them
            for k in ["home_gk", "away_gk"] {
                let c = get_s(&b, k);
                if c.len() == 7 && c.starts_with('#') {
                    v[k] = json!(c);
                }
            }
            match std::fs::write(dir.join("teams.json"), serde_json::to_string_pretty(&v).unwrap()) {
                Ok(()) => jres(&mut s, json!({"ok": true})),
                Err(e) => err(&mut s, e),
            }
        }
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
                    jres(&mut s, json!({"ok": true, "status": *j.status.lock().unwrap(), "code": *j.code.lock().unwrap(), "from": from_eff, "total": lines.len(), "lines": new}));
                }
                None => err(&mut s, trs!("Auftrag unbekannt", "unknown job", "tâche inconnue", "tarea desconocida")),
            }
        }
        ("POST", p) if p.starts_with("/api/job/") && p.ends_with("/cancel") => {
            let id: u64 = p.trim_start_matches("/api/job/").trim_end_matches("/cancel").trim_end_matches('/').parse().unwrap_or(0);
            if let Some(j) = ui.jobs.lock().unwrap().get(&id).cloned() {
                *j.status.lock().unwrap() = "cancelled".into();
                if let Some(c) = j.child.lock().unwrap().as_mut() {
                    let _ = c.kill();
                }
            }
            jres(&mut s, json!({"ok": true}));
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
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
            "ticker_names": "both", "home_name": " Sharks ", "away_name": "", "network": true, "ticker_approve": true, "ticker_export": true, "ticker_push": " https://dav.example.org/t.json ", "ticker_key": ""});
        let a = build_args("live", Path::new("/nonexistent-project"), &p).unwrap();
        assert_eq!(pair(&a, "--ticker-names").as_deref(), Some("both"));
        assert_eq!(pair(&a, "--home-name").as_deref(), Some("Sharks"));
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
        assert_eq!(c.iter().filter(|x| *x == "--lang").count(), 1);
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
