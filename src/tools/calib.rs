//! Court calibration: a tiny local web page (std-only HTTP server on 127.0.0.1)
//! where you click court landmarks on a panorama frame. Nothing leaves the machine.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::video::{grab_png, VideoInfo};
use crate::geom::{Homography, HomographyProjector, Vec2, COURT_LENGTH, COURT_WIDTH};

/// (name, x, y) in court metres. x: left -> right basket as seen in the
/// panorama, y: far sideline (0) -> near sideline (15).
pub const LANDMARKS: &[(&str, f32, f32)] = &[
    ("Ecke links / hinten", 0.0, 0.0),
    ("Ecke rechts / hinten", 28.0, 0.0),
    ("Ecke rechts / vorn", 28.0, 15.0),
    ("Ecke links / vorn", 0.0, 15.0),
    ("Mittellinie / hinten (Seitenlinie)", 14.0, 0.0),
    ("Mittellinie / vorn (Seitenlinie)", 14.0, 15.0),
    ("Mittelkreis – Mitte", 14.0, 7.5),
    ("Zone links: Grundlinie hinten", 0.0, 5.05),
    ("Zone links: Grundlinie vorn", 0.0, 9.95),
    ("Zone links: Freiwurflinie hinten", 5.8, 5.05),
    ("Zone links: Freiwurflinie vorn", 5.8, 9.95),
    ("Zone rechts: Grundlinie hinten", 28.0, 5.05),
    ("Zone rechts: Grundlinie vorn", 28.0, 9.95),
    ("Zone rechts: Freiwurflinie hinten", 22.2, 5.05),
    ("Zone rechts: Freiwurflinie vorn", 22.2, 9.95),
];

#[derive(Clone, Debug)]
pub struct CalPoint {
    pub name: String,
    pub pano: Vec2,
    pub court: Vec2,
}

#[derive(Clone, Debug)]
pub struct CourtCalib {
    pub pano_w: u32,
    pub pano_h: u32,
    pub points: Vec<CalPoint>,
    pub rms_m: f32,
}

impl CourtCalib {
    pub fn projector(&self) -> Option<HomographyProjector> {
        let p: Vec<Vec2> = self.points.iter().map(|c| c.pano).collect();
        let c: Vec<Vec2> = self.points.iter().map(|c| c.court).collect();
        HomographyProjector::calibrate(&p, &c)
    }

    pub fn save(&self, path: &str, video: &str) -> Result<(), String> {
        let v = json!({
            "version": 1, "video": video, "pano_w": self.pano_w, "pano_h": self.pano_h, "rms_m": self.rms_m,
            "court_m": [COURT_LENGTH, COURT_WIDTH],
            "points": self.points.iter().map(|p| json!({"name": p.name, "pano": [p.pano.x, p.pano.y], "court": [p.court.x, p.court.y]})).collect::<Vec<_>>(),
        });
        std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("{path}: {e}"))
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let v: Value = serde_json::from_str(&txt).map_err(|e| format!("{path}: {e}"))?;
        let pt = |a: &Value| Some(Vec2::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32));
        let mut points = Vec::new();
        for p in v["points"].as_array().ok_or("court.json: points missing")? {
            points.push(CalPoint { name: p["name"].as_str().unwrap_or("").to_string(), pano: pt(&p["pano"]).ok_or("bad pano point")?, court: pt(&p["court"]).ok_or("bad court point")? });
        }
        if points.len() < 4 {
            return Err("court.json: need >= 4 points".into());
        }
        Ok(Self { pano_w: v["pano_w"].as_u64().unwrap_or(0) as u32, pano_h: v["pano_h"].as_u64().unwrap_or(0) as u32, points, rms_m: v["rms_m"].as_f64().unwrap_or(0.0) as f32 })
    }
}

/// Pano->court homography and RMS error (metres) for the given (landmark index, pano point) pairs.
fn solve(pairs: &[(usize, Vec2)]) -> Option<(HomographyProjector, Vec<(usize, f32)>, f32)> {
    let pano: Vec<Vec2> = pairs.iter().map(|p| p.1).collect();
    let court: Vec<Vec2> = pairs.iter().map(|p| Vec2::new(LANDMARKS[p.0].1, LANDMARKS[p.0].2)).collect();
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

fn court_lines() -> Vec<Vec<Vec2>> {
    let (l, w) = (COURT_LENGTH, COURT_WIDTH);
    let seg = |a: Vec2, b: Vec2| -> Vec<Vec2> { (0..=40).map(|i| a.lerp(b, i as f32 / 40.0)).collect() };
    let mut out = Vec::new();
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        vec![seg(Vec2::new(x0, y0), Vec2::new(x1, y0)), seg(Vec2::new(x1, y0), Vec2::new(x1, y1)), seg(Vec2::new(x1, y1), Vec2::new(x0, y1)), seg(Vec2::new(x0, y1), Vec2::new(x0, y0))]
    };
    out.extend(rect(0.0, 0.0, l, w));
    out.push(seg(Vec2::new(l / 2.0, 0.0), Vec2::new(l / 2.0, w)));
    out.extend(rect(0.0, 5.05, 5.8, 9.95));
    out.extend(rect(l - 5.8, 5.05, l, 9.95));
    let circle = |c: Vec2, r: f32| -> Vec<Vec2> { (0..=64).map(|i| { let a = i as f32 / 64.0 * std::f32::consts::TAU; Vec2::new(c.x + r * a.cos(), c.y + r * a.sin()) }).collect() };
    out.push(circle(Vec2::new(l / 2.0, w / 2.0), 1.8));
    out.push(circle(Vec2::new(5.8, w / 2.0), 1.8));
    out.push(circle(Vec2::new(l - 5.8, w / 2.0), 1.8));
    out
}

struct Shared {
    png: Vec<u8>,
    info: VideoInfo,
    out_path: String,
    video: String,
    done: AtomicBool,
}

fn parse_points(body: &str) -> Option<Vec<(usize, Vec2)>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out = Vec::new();
    for p in v["points"].as_array()? {
        let id = p["id"].as_u64()? as usize;
        if id >= LANDMARKS.len() {
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
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let (method, path) = {
        let mut it = first.split_whitespace();
        (it.next().unwrap_or(""), it.next().unwrap_or(""))
    };
    let clen = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
    while buf.len() < header_end + clen {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buf[header_end.min(buf.len())..]).to_string();

    match (method, path) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("calib.html").as_bytes()),
        ("GET", "/frame.png") => respond(&mut s, "200 OK", "image/png", &st.png),
        ("GET", "/api/info") => {
            let v = json!({"w": st.info.w, "h": st.info.h, "landmarks": LANDMARKS.iter().map(|l| json!({"name": l.0, "x": l.1, "y": l.2})).collect::<Vec<_>>()});
            respond(&mut s, "200 OK", "application/json", v.to_string().as_bytes());
        }
        ("POST", "/api/solve") => {
            let r = match parse_points(&body).filter(|p| p.len() >= 4).and_then(|p| solve(&p).map(|r| (p, r))) {
                Some((pairs, (_proj, res, rms))) => {
                    let pano: Vec<Vec2> = pairs.iter().map(|p| p.1).collect();
                    let court: Vec<Vec2> = pairs.iter().map(|p| Vec2::new(LANDMARKS[p.0].1, LANDMARKS[p.0].2)).collect();
                    match Homography::from_points(&court, &pano) {
                        Some(inv) => {
                            let overlay: Vec<Vec<[f32; 2]>> = court_lines().iter().map(|pl| pl.iter().filter_map(|c| inv.apply(*c)).map(|p| [p.x, p.y]).collect()).collect();
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
            let r = match parse_points(&body).filter(|p| p.len() >= 4).and_then(|p| solve(&p).map(|r| (p, r))) {
                Some((pairs, (_proj, _res, rms))) => {
                    let cal = CourtCalib {
                        pano_w: st.info.w,
                        pano_h: st.info.h,
                        rms_m: rms,
                        points: pairs.iter().map(|(i, p)| CalPoint { name: LANDMARKS[*i].0.to_string(), pano: *p, court: Vec2::new(LANDMARKS[*i].1, LANDMARKS[*i].2) }).collect(),
                    };
                    match cal.save(&st.out_path, &st.video) {
                        Ok(()) => {
                            st.done.store(true, Ordering::SeqCst);
                            json!({"ok": true, "path": st.out_path, "rms": rms})
                        }
                        Err(e) => json!({"ok": false, "error": e}),
                    }
                }
                None => json!({"ok": false, "error": "need at least 4 valid points"}),
            };
            respond(&mut s, "200 OK", "application/json", r.to_string().as_bytes());
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

fn open_browser(url: &str) {
    let cmd = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(cmd).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Serve the calibration page until the user saves. Returns the saved path.
pub fn run(input: &str, info: &VideoInfo, t: f64, out_path: &str, open: bool) -> Result<(), String> {
    let png = grab_png(input, t)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let st = Arc::new(Shared { png, info: info.clone(), out_path: out_path.to_string(), video: input.to_string(), done: AtomicBool::new(false) });
    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("Kalibrierung: {url}\n(Seite öffnet sich automatisch; sonst URL im Browser öffnen. Strg+C zum Abbrechen.)");
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
            // give the browser a moment to receive the response
            let t0 = *after_done.get_or_insert_with(std::time::Instant::now);
            if t0.elapsed() > Duration::from_millis(800) {
                return Ok(());
            }
        }
    }
}
