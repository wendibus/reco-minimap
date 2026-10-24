//! Field calibration: a tiny local web page (std-only HTTP server on 127.0.0.1)
//! where you click field landmarks on a panorama frame. Nothing leaves the machine.

use crate::{tr, trs};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::video::{grab_png, VideoInfo};
use crate::geom::{Homography, HomographyProjector, Vec2};
use crate::sport::SportProfile;

#[derive(Clone, Debug)]
pub struct CalPoint {
    /// Landmark key (language independent, e.g. `corner_fl`).
    pub name: String,
    pub pano: Vec2,
    pub court: Vec2,
}

#[derive(Clone, Debug)]
pub struct CourtCalib {
    pub pano_w: u32,
    pub pano_h: u32,
    pub sport: String,
    pub field_m: (f32, f32),
    pub team_size: usize,
    pub referees: usize,
    pub points: Vec<CalPoint>,
    pub rms_m: f32,
}

impl CourtCalib {
    pub fn projector(&self) -> Option<HomographyProjector> {
        let p: Vec<Vec2> = self.points.iter().map(|c| c.pano).collect();
        let c: Vec<Vec2> = self.points.iter().map(|c| c.court).collect();
        HomographyProjector::calibrate(&p, &c)
    }

    /// The sport profile this calibration was made for (custom sports use the stored size).
    pub fn profile(&self) -> SportProfile {
        let mut p = match SportProfile::builtin(&self.sport) {
            Some(p) if self.sport != "custom" => p,
            _ => SportProfile::custom_rect(&self.sport, self.field_m.0, self.field_m.1, self.team_size, self.referees),
        };
        p.team_size = self.team_size.max(1);
        p.referees = self.referees;
        p
    }

    pub fn save(&self, path: &str, video: &str) -> Result<(), String> {
        let v = json!({
            "version": 2, "video": video, "pano_w": self.pano_w, "pano_h": self.pano_h, "rms_m": self.rms_m,
            "sport": self.sport, "court_m": [self.field_m.0, self.field_m.1], "team_size": self.team_size, "referees": self.referees,
            "points": self.points.iter().map(|p| json!({"name": p.name, "pano": [p.pano.x, p.pano.y], "court": [p.court.x, p.court.y]})).collect::<Vec<_>>(),
        });
        std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("{path}: {e}"))
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let v: Value = serde_json::from_str(&txt).map_err(|e| format!("{path}: {e}"))?;
        let pt = |a: &Value| Some(Vec2::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32));
        let mut points = Vec::new();
        for p in v["points"].as_array().ok_or(trs!("court.json: Punkte fehlen", "court.json: points missing", "court.json : points manquants", "court.json: faltan los puntos"))? {
            points.push(CalPoint { name: p["name"].as_str().unwrap_or("").to_string(), pano: pt(&p["pano"]).ok_or(trs!("ungültiger Bildpunkt", "bad pano point", "point d'image invalide", "punto de imagen no válido"))?, court: pt(&p["court"]).ok_or(trs!("ungültiger Feldpunkt", "bad court point", "point de terrain invalide", "punto de campo no válido"))? });
        }
        if points.len() < 4 {
            return Err(trs!("court.json: mindestens 4 Punkte nötig", "court.json: need >= 4 points", "court.json : au moins 4 points nécessaires", "court.json: hacen falta al menos 4 puntos").into());
        }
        // version 1 files (basketball only) have no sport fields
        let fm = pt(&v["court_m"]).unwrap_or(Vec2::new(28.0, 15.0));
        Ok(Self {
            pano_w: v["pano_w"].as_u64().unwrap_or(0) as u32,
            pano_h: v["pano_h"].as_u64().unwrap_or(0) as u32,
            sport: v["sport"].as_str().unwrap_or("basketball").to_string(),
            field_m: (fm.x, fm.y),
            team_size: v["team_size"].as_u64().unwrap_or(5) as usize,
            referees: v["referees"].as_u64().unwrap_or(2) as usize,
            points,
            rms_m: v["rms_m"].as_f64().unwrap_or(0.0) as f32,
        })
    }
}

/// Pano->court homography and RMS error (metres) for the given (landmark index, pano point) pairs.
fn solve(pairs: &[(usize, Vec2)], profile: &SportProfile) -> Option<(HomographyProjector, Vec<(usize, f32)>, f32)> {
    let pano: Vec<Vec2> = pairs.iter().map(|p| p.1).collect();
    let court: Vec<Vec2> = pairs.iter().map(|p| Vec2::new(profile.landmarks[p.0].x, profile.landmarks[p.0].y)).collect();
    let proj = HomographyProjector::calibrate(&pano, &court)?;
    let mut res = Vec::new();
    let mut sum = 0.0;
    for (k, (i, p)) in pairs.iter().enumerate() {
        let c = proj.pano_to_court.apply(*p)?;
        let e = c.dist(court[k]);
        sum += e * e;
        res.push((*i, e));
    }
    Some((proj, res, (sum / pairs.len() as f32).sqrt()))
}

struct Shared {
    png: Vec<u8>,
    info: VideoInfo,
    out_path: String,
    video: String,
    profile: SportProfile,
    lang: String,
    done: AtomicBool,
}

fn parse_points(body: &str, n_landmarks: usize) -> Option<Vec<(usize, Vec2)>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out = Vec::new();
    for p in v["points"].as_array()? {
        let id = p["id"].as_u64()? as usize;
        if id >= n_landmarks {
            return None;
        }
        out.push((id, Vec2::new(p["u"].as_f64()? as f32, p["v"].as_f64()? as f32)));
    }
    Some(out)
}

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, st: &Shared) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 1 << 20 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let first = head.lines().next().unwrap_or("");
    let (method, full_path) = {
        let mut it = first.split_whitespace();
        (it.next().unwrap_or(""), it.next().unwrap_or(""))
    };
    let path = full_path.split('?').next().unwrap_or("");
    let clen = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
    while buf.len() < header_end + clen {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buf[header_end.min(buf.len())..]).to_string();
    let n_lm = st.profile.landmarks.len();

    match (method, path) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("calib.html").as_bytes()),
        ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
        ("GET", "/frame.png") => respond(&mut s, "200 OK", "image/png", &st.png),
        ("GET", "/api/info") => {
            let v = json!({"w": st.info.w, "h": st.info.h, "sport": st.profile.id, "field": [st.profile.field.length, st.profile.field.width], "lang": st.lang,
                           "landmarks": st.profile.landmarks.iter().map(|l| json!({"key": l.key, "x": l.x, "y": l.y})).collect::<Vec<_>>(),
                           // field markings in metres for the legend that shows where the current landmark lies
                           "lines": st.profile.lines.iter().map(|pl| pl.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()).collect::<Vec<_>>()});
            respond(&mut s, "200 OK", "application/json", v.to_string().as_bytes());
        }
        ("POST", "/api/solve") => {
            let r = match parse_points(&body, n_lm).filter(|p| p.len() >= 4).and_then(|p| solve(&p, &st.profile).map(|r| (p, r))) {
                Some((pairs, (_proj, res, rms))) => {
                    let pano: Vec<Vec2> = pairs.iter().map(|p| p.1).collect();
                    let court: Vec<Vec2> = pairs.iter().map(|p| Vec2::new(st.profile.landmarks[p.0].x, st.profile.landmarks[p.0].y)).collect();
                    match Homography::from_points(&court, &pano) {
                        Some(inv) => {
                            let overlay: Vec<Vec<[f32; 2]>> = st.profile.densified_lines(0.5).iter().map(|pl| pl.iter().filter_map(|c| inv.apply(*c)).map(|p| [p.x, p.y]).collect()).collect();
                            json!({"ok": true, "rms": rms, "residuals": res.iter().map(|(i, m)| json!({"id": i, "m": m})).collect::<Vec<_>>(), "overlay": overlay})
                        }
                        None => json!({"ok": false}),
                    }
                }
                None => json!({"ok": false}),
            };
            respond(&mut s, "200 OK", "application/json", r.to_string().as_bytes());
        }
        ("POST", "/api/save") => {
            let r = match parse_points(&body, n_lm).filter(|p| p.len() >= 4).and_then(|p| solve(&p, &st.profile).map(|r| (p, r))) {
                Some((pairs, (_proj, _res, rms))) => {
                    let cal = CourtCalib {
                        pano_w: st.info.w,
                        pano_h: st.info.h,
                        sport: st.profile.id.clone(),
                        field_m: (st.profile.field.length, st.profile.field.width),
                        team_size: st.profile.team_size,
                        referees: st.profile.referees,
                        rms_m: rms,
                        points: pairs.iter().map(|(i, p)| CalPoint { name: st.profile.landmarks[*i].key.clone(), pano: *p, court: Vec2::new(st.profile.landmarks[*i].x, st.profile.landmarks[*i].y) }).collect(),
                    };
                    match cal.save(&st.out_path, &st.video) {
                        Ok(()) => {
                            st.done.store(true, Ordering::SeqCst);
                            json!({"ok": true, "path": st.out_path, "rms": rms})
                        }
                        Err(e) => json!({"ok": false, "error": e}),
                    }
                }
                None => json!({"ok": false, "error": trs!("mindestens 4 gültige Punkte nötig", "need at least 4 valid points", "au moins 4 points valides nécessaires", "hacen falta al menos 4 puntos válidos")}),
            };
            respond(&mut s, "200 OK", "application/json", r.to_string().as_bytes());
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

pub(crate) fn open_browser(url: &str) {
    let cmd = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(cmd).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Serve the calibration page until the user saves.
pub fn run(input: &str, info: &VideoInfo, t: f64, out_path: &str, open: bool, profile: SportProfile, lang: &str) -> Result<(), String> {
    let png = grab_png(input, t)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let st = Arc::new(Shared { png, info: info.clone(), out_path: out_path.to_string(), video: input.to_string(), profile, lang: lang.to_string(), done: AtomicBool::new(false) });
    let url = format!("http://127.0.0.1:{port}/?lang={lang}");
    eprintln!("{}", tr!("Kalibrierung: {url}\n(Seite öffnet sich automatisch; sonst URL im Browser öffnen. Strg+C zum Abbrechen.)", "Calibration: {url}\n(The page opens by itself; otherwise open the URL in a browser. Ctrl+C to cancel.)", "Calibrage : {url}\n(La page s'ouvre toute seule ; sinon ouvrir l'URL dans un navigateur. Ctrl+C pour annuler.)", "Calibración: {url}\n(La página se abre sola; si no, abre la URL en un navegador. Ctrl+C para cancelar.)"));
    if open {
        open_browser(&url);
    }
    let mut after_done: Option<std::time::Instant> = None;
    loop {
        match listener.accept() {
            Ok((s, _)) => {
                let _ = s.set_nonblocking(false);
                let st2 = st.clone();
                std::thread::spawn(move || handle(s, &st2));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
        if st.done.load(Ordering::SeqCst) {
            let t0 = *after_done.get_or_insert_with(std::time::Instant::now);
            if t0.elapsed() > Duration::from_millis(800) {
                return Ok(());
            }
        }
    }
}
