//! Local web UI (`reco-minimap ui`): projects, model import, calibration, runs and live mode in the
//! browser. A small std-only HTTP server on 127.0.0.1; heavy work runs as child processes of this
//! same executable (the CLI commands), so UI and command line always behave the same.

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
}

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
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

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

fn read_request(s: &mut TcpStream) -> Option<(String, String, String, String, String)> {
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

fn qparam(query: &str, key: &str) -> Option<String> {
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
    let out = if cfg!(target_os = "macos") {
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
    if p.is_empty() { None } else { Some(p.trim_end_matches('/').to_string()) }
}

fn reveal(path: &str) {
    let cmd = if cfg!(target_os = "macos") { ("open", vec!["-R", path]) } else if cfg!(target_os = "windows") { ("explorer", vec!["/select,", path]) } else { ("xdg-open", vec![path]) };
    let _ = Command::new(cmd.0).args(&cmd.1).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
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
    let need = |v: &str, what: &str| if v.is_empty() { Err(format!("{what} fehlt")) } else { Ok(()) };
    let mut a: Vec<String> = Vec::new();
    let ep = get_s(p, "ep");
    let units = get_s(p, "units");
    let has_scales = dir.join("scales.json").exists();
    let engine = |a: &mut Vec<String>| {
        a.push("--ep".into());
        a.push(if ep == "cpu" { "cpu".into() } else { "coreml".into() });
        if !units.is_empty() && units != "all" {
            a.push("--units".into());
            a.push(units.clone());
        }
    };
    match kind {
        "calibrate" => {
            need(&video, "Video")?;
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
            need(&video, "Video")?;
            need(&model, "Modell")?;
            pushs(&mut a, &[kind, &video, "--model", &model]);
            if kind == "selfcheck" {
                pushs(&mut a, &["--samples", "6"]);
            } else {
                pushs(&mut a, &["--samples", "12", "--court", "court.json", "--out", if kind == "scales" { "scales.json" } else { "teams.json" }]);
            }
            if has_scales && kind != "scales" {
                pushs(&mut a, &["--tile", "auto"]);
            }
            engine(&mut a);
        }
        "run" | "live" => {
            need(&model, "Modell")?;
            let src = if kind == "run" { video.clone() } else { get_s(p, "source") };
            need(&src, "Quelle/Video")?;
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
                    return Err("keine Ausgabe gewählt".into());
                }
            } else {
                pushs(&mut a, &["--size", &get_s(p, "size"), "--fps", &get_s(p, "fps"), "--port", &get_s(p, "port")]);
                if p["realtime"].as_bool() == Some(true) {
                    pushs(&mut a, &["--realtime"]);
                }
            }
        }
        "fetch_ocr" => pushs(&mut a, &["fetch-ocr"]),
        "stats" => {
            let f = get_s(p, "file");
            need(&f, "Datei")?;
            pushs(&mut a, &["stats", &f]);
            if dir.join("court.json").exists() {
                pushs(&mut a, &["--court", "court.json"]);
            }
        }
        other => return Err(format!("unbekannter Auftrag: {other}")),
    }
    Ok(a)
}

fn pushs(a: &mut Vec<String>, xs: &[&str]) {
    xs.iter().for_each(|x| a.push(x.to_string()));
}

fn start_job(ui: &Arc<Ui>, dir: &Path, kind: &str, params: &Value) -> Result<u64, String> {
    let args = build_args(kind, dir, params)?;
    let mut child = Command::new(&ui.exe).args(&args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
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
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("ui.html").as_bytes()),
        ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
        ("GET", "/api/init") => {
            let sports: Vec<Value> = BUILTIN_IDS.iter().filter_map(|id| SportProfile::builtin(id)).map(|p| json!({"id": p.id, "field": [p.field.length, p.field.width], "team_size": p.team_size, "referees": p.referees})).collect();
            let ort = super::infer::ensure_ort_runtime().err();
            jres(&mut s, json!({"ok": true, "home": home_dir().to_string_lossy(), "default_dir": default_project_dir().to_string_lossy(), "sports": sports,
                                "coreml": cfg!(feature = "coreml"), "ocr": {"path": super::ocr::default_model_path().to_string_lossy(), "exists": super::ocr::default_model_path().exists()}, "ort_error": ort, "version": env!("CARGO_PKG_VERSION"), "dialogs": cfg!(target_os = "macos") || Command::new("zenity").arg("--version").output().is_ok()}));
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
            let v = json!({"home": get_s(&b, "home"), "away": get_s(&b, "away")});
            match std::fs::write(dir.join("teams.json"), serde_json::to_string_pretty(&v).unwrap()) {
                Ok(()) => jres(&mut s, json!({"ok": true})),
                Err(e) => err(&mut s, e),
            }
        }
        ("POST", "/api/dialog") => {
            let p = native_dialog(&get_s(&b, "kind"), &get_s(&b, "prompt"));
            jres(&mut s, json!({"ok": true, "path": p}));
        }
        ("POST", "/api/reveal") => {
            reveal(&get_s(&b, "path"));
            jres(&mut s, json!({"ok": true}));
        }
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
            match start_job(&ui, &dir, &get_s(&b, "kind"), &b["params"]) {
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
                None => err(&mut s, "Auftrag unbekannt"),
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
pub fn serve(port: u16, open: bool, block: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut listener = None;
    for p in port..port.saturating_add(10) {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            listener = Some(l);
            break;
        }
    }
    let listener = listener.ok_or("Kein freier Port (--port ändern)")?;
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().map_err(|e| e.to_string())?.port());
    let ui = Arc::new(Ui { exe, jobs: Mutex::new(HashMap::new()), next: AtomicU64::new(1) });
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
