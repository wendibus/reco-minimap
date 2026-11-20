//! Field calibration: a tiny local web page (std-only HTTP server on 127.0.0.1)
//! where you click field landmarks on a panorama frame. Nothing leaves the machine.

use crate::{tr, trs};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::video::{grab, grab_png, Frame, VideoInfo};
use crate::autocal::{self, Erase, Hints, LineMap, Placement};
use crate::geom::{HomographyProjector, Projector, Vec2};
use crate::lens::{Lens, LensLevel, LensModel};
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
    /// Lens (fisheye or panorama), estimated together with the homography (`None` = plain image).
    pub lens: Option<Lens>,
}

impl CourtCalib {
    pub fn projector(&self) -> Option<HomographyProjector> {
        let p: Vec<Vec2> = self.points.iter().map(|c| c.pano).collect();
        let c: Vec<Vec2> = self.points.iter().map(|c| c.court).collect();
        match &self.lens {
            Some(l) => crate::lens::fit_homography(l, &p, &c).map(|f| HomographyProjector::new(f.pano_to_court, Some(*l))),
            None => HomographyProjector::calibrate(&p, &c),
        }
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
        let mut v = json!({
            "version": 2, "video": video, "pano_w": self.pano_w, "pano_h": self.pano_h, "rms_m": self.rms_m,
            "sport": self.sport, "court_m": [self.field_m.0, self.field_m.1], "team_size": self.team_size, "referees": self.referees,
            "points": self.points.iter().map(|p| json!({"name": p.name, "pano": [p.pano.x, p.pano.y], "court": [p.court.x, p.court.y]})).collect::<Vec<_>>(),
        });
        if let Some(l) = &self.lens {
            v["lens"] = match l.model {
                LensModel::Division => json!({"model": "division", "cx": l.cx, "cy": l.cy, "k1": l.k1, "k2": l.k2}),
                LensModel::Cylinder => json!({"model": "cylinder", "fov": l.fov, "cy": l.cy}),
            };
        }
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
        let (pano_w, pano_h) = (v["pano_w"].as_u64().unwrap_or(0) as u32, v["pano_h"].as_u64().unwrap_or(0) as u32);
        let lens = v["lens"].as_object().map(|_| {
            let f = |k: &str, d: f64| v["lens"][k].as_f64().unwrap_or(d);
            let aspect = if pano_h > 0 { pano_w as f64 / pano_h as f64 } else { 16.0 / 9.0 };
            match v["lens"]["model"].as_str() {
                Some("cylinder") => Lens::cylinder(aspect, f("fov", std::f64::consts::PI), f("cy", 0.5)),
                _ => Lens { cx: f("cx", 0.5), cy: f("cy", 0.5), k1: f("k1", 0.0), k2: f("k2", 0.0), ..Lens::none(aspect) },
            }
        });
        Ok(Self {
            pano_w,
            pano_h,
            sport: v["sport"].as_str().unwrap_or("basketball").to_string(),
            field_m: (fm.x, fm.y),
            team_size: v["team_size"].as_u64().unwrap_or(5) as usize,
            referees: v["referees"].as_u64().unwrap_or(2) as usize,
            points,
            rms_m: v["rms_m"].as_f64().unwrap_or(0.0) as f32,
            lens: lens.filter(|l| !l.is_identity()),
        })
    }
}

/// Pano->court mapping and RMS error (metres) for the given (landmark index, pano point) pairs.
/// With a lens model the lens is estimated as well, as far as the number of points allows; with fewer
/// than [`LensLevel::MIN_POINTS`] the plain homography is used.
/// One clicked point: a named landmark of the field, or a point somewhere on a field line (pulled in
/// from the picture edge), with its spot on the field in metres and in the picture.
#[derive(Clone, Debug)]
struct Pair {
    /// What the page calls it: the landmark index, or its own id for a line point.
    id: Value,
    name: String,
    court: Vec2,
    pano: Vec2,
}

fn solve(pairs: &[Pair], model: Option<LensModel>, aspect: f64) -> Option<(HomographyProjector, Vec<(Value, f32)>, f32)> {
    let pano: Vec<Vec2> = pairs.iter().map(|p| p.pano).collect();
    let court: Vec<Vec2> = pairs.iter().map(|p| p.court).collect();
    let proj = match LensLevel::for_points(pairs.len()).zip(model) {
        Some((level, model)) => {
            let f = crate::lens::fit_model(&pano, &court, aspect, level, model)?;
            HomographyProjector::new(f.pano_to_court, Some(f.lens))
        }
        None => HomographyProjector::calibrate(&pano, &court)?,
    };
    let mut res = Vec::new();
    let mut sum = 0.0;
    for (k, p) in pairs.iter().enumerate() {
        let c = proj.to_court(p.pano.x, p.pano.y)?;
        let e = c.dist(court[k]);
        sum += e * e;
        res.push((p.id.clone(), e));
    }
    Some((proj, res, (sum / pairs.len() as f32).sqrt()))
}

struct Shared {
    png: Vec<u8>,
    /// The clear view (players removed), shown first when there is one.
    plate_png: Option<Vec<u8>>,
    info: VideoInfo,
    out_path: String,
    video: String,
    profile: SportProfile,
    lang: String,
    /// Lens model the page starts with (existing calibration, `--lens`, or a wide panorama).
    lens: Option<LensModel>,
    /// The frame for the automatic calibration and its line pixels for the last line colour.
    frame: Option<Frame>,
    lines: std::sync::Mutex<Option<(Option<[u8; 3]>, Arc<LineMap>)>>,
    done: AtomicBool,
}

impl Shared {
    fn line_map(&self, color: Option<[u8; 3]>) -> Option<Arc<LineMap>> {
        let f = self.frame.as_ref()?;
        let mut g = self.lines.lock().ok()?;
        if let Some((c, m)) = g.as_ref() {
            if *c == color {
                return Some(m.clone());
            }
        }
        let m = Arc::new(LineMap::from_rgb(&f.rgb, f.w, f.h, color));
        *g = Some((color, m.clone()));
        Some(m)
    }
}

/// Placement (corners in the picture and lens) that the clicked points describe, as the starting
/// point for the automatic calibration.
fn placement_from_points(pairs: &[Pair], profile: &SportProfile, model: Option<LensModel>, aspect: f64) -> Option<Placement> {
    let (proj, _, _) = solve(pairs, model, aspect)?;
    let to_pano = proj.to_pano_fn()?;
    let (l, w) = (profile.field.length, profile.field.width);
    let c = |x: f32, y: f32| to_pano(Vec2::new(x, y));
    let lens = proj.lens.unwrap_or_else(|| match model {
        // too few points for the lens: start the panorama search from a typical field of view
        Some(LensModel::Cylinder) => Lens::cylinder(aspect, std::f64::consts::PI, 0.5),
        _ => Lens::none(aspect),
    });
    if proj.lens.is_none() && model == Some(LensModel::Cylinder) {
        return None; // corners of a flat fit do not carry over to the panorama model
    }
    Some(Placement { corners: [c(0.0, 0.0)?, c(l, 0.0)?, c(l, w)?, c(0.0, w)?], lens })
}

/// How far snapping may move a line, in pixels at 960 px picture width: only the painted line right
/// next to a drawn one should pull, not the neighbouring line of the key or another sport.
pub const SNAP_PX: f32 = 8.0;

/// The clicked points after snapping the field onto the painted lines next to the drawn ones.
pub struct Snapped {
    /// New picture position per point (unchanged where the spot left the picture).
    pub pano: Vec<Vec2>,
    pub before: f64,
    pub after: f64,
    /// Largest shift of a visible line, pixels at 960 px picture width.
    pub moved: f32,
}

fn snap_pairs(map: &LineMap, pairs: &[Pair], profile: &SportProfile, model: Option<LensModel>, aspect: f64, max_px: f32) -> Option<Snapped> {
    let start = placement_from_points(pairs, profile, model, aspect)?;
    let field = (profile.field.length, profile.field.width);
    let r = autocal::snap(map, &profile.lines, field, start, model, max_px)?;
    let m = autocal::Mapper::new(&r.placement, field)?;
    let pano = pairs.iter().map(|p| m.to_image(p.court).filter(|q| q.x >= 0.0 && q.y >= 0.0 && q.x <= 1.0 && q.y <= 1.0).unwrap_or(p.pano)).collect();
    Some(Snapped { pano, before: r.before, after: r.after, moved: r.moved_px })
}

/// `/api/snap`: the clicked points moved so the drawn lines lie on the painted ones.
fn snap_json(st: &Shared, body: &str, model: Option<LensModel>, aspect: f64) -> Value {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let Some(map) = lines_for(st, &v) else { return json!({"ok": false, "error": "no frame"}) };
    let Some(pairs) = parse_points(body, &st.profile).filter(|x| x.len() >= 4) else { return json!({"ok": false}) };
    let max_px = v["max"].as_f64().map(|x| x as f32).unwrap_or(SNAP_PX);
    match snap_pairs(&map, &pairs, &st.profile, model, aspect, max_px) {
        Some(r) => json!({"ok": true, "before": r.before, "after": r.after, "moved_px": r.moved * st.info.w as f32 / 960.0,
                          "points": pairs.iter().zip(&r.pano).map(|(p, q)| json!({"id": p.id, "u": q.x, "v": q.y})).collect::<Vec<_>>()}),
        None => json!({"ok": false}),
    }
}

/// Snap an existing calibration file onto the painted lines of a frame (`court-snap`). Writes `out`
/// only when the lines fit better. Returns (agreement before, after, largest shift in frame pixels).
pub fn snap_file(input: &str, info: &VideoInfo, t: f64, court: &str, out: Option<&str>, color: Option<[u8; 3]>, max_px: f32) -> Result<(f64, f64, f32), String> {
    let cal = CourtCalib::load(court)?;
    let profile = cal.profile();
    let frame = grab(input, info, t)?;
    let map = LineMap::from_rgb(&frame.rgb, frame.w, frame.h, color);
    let aspect = info.w as f64 / info.h.max(1) as f64;
    let model = cal.lens.map(|l| l.model);
    let pairs: Vec<Pair> = cal.points.iter().enumerate().map(|(k, p)| Pair { id: json!(k), name: p.name.clone(), court: p.court, pano: p.pano }).collect();
    let r = snap_pairs(&map, &pairs, &profile, model, aspect, max_px).ok_or_else(|| trs!("Die Linien liegen schon so gut wie möglich auf den erkannten Linien.", "The lines already lie on the detected lines as well as possible.", "Les lignes sont déjà au mieux sur les lignes détectées.", "Las líneas ya están lo mejor posible sobre las detectadas.").to_string())?;
    if let Some(out) = out {
        let moved: Vec<Pair> = pairs.iter().zip(&r.pano).map(|(p, q)| Pair { pano: *q, ..p.clone() }).collect();
        let (proj, _, rms) = solve(&moved, model, aspect).ok_or("solve")?;
        let new = CourtCalib { rms_m: rms, lens: proj.lens, points: moved.iter().map(|p| CalPoint { name: p.name.clone(), pano: p.pano, court: p.court }).collect(), ..cal };
        new.save(out, input)?;
    }
    Ok((r.before, r.after, r.moved * info.w as f32 / 960.0))
}

/// `[r, g, b]` from a JSON array.
fn color_of(v: &Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_u64()?.min(255) as u8, a.get(1)?.as_u64()?.min(255) as u8, a.get(2)?.as_u64()?.min(255) as u8])
}

/// Hand corrections of the line picture: `"erase": [[u, v, r], ...]`, r = 0 for a click on a line
/// piece, else the brush radius as a share of the picture width.
fn erase_of(v: &Value) -> Vec<Erase> {
    v["erase"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let (u, w, r) = (e[0].as_f64()? as f32, e[1].as_f64()? as f32, e[2].as_f64().unwrap_or(0.0) as f32);
                    let c = Vec2::new(u, w);
                    Some(if r > 0.0 { Erase::Brush(c, r.min(0.2)) } else { Erase::Piece(c) })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The line map for a request body (`color`, `erase`).
fn lines_for(st: &Shared, v: &Value) -> Option<Arc<LineMap>> {
    let map = st.line_map(color_of(&v["color"]))?;
    let erase = erase_of(v);
    Some(if erase.is_empty() { map } else { Arc::new(map.without(&erase)) })
}

/// Automatic calibration: the landmarks of the fitted field that lie in the picture.
fn auto_json(st: &Shared, body: &str, model: Option<LensModel>, aspect: f64) -> Value {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let mut outside = [false; 4];
    for (i, o) in outside.iter_mut().enumerate() {
        *o = v["outside"][i].as_bool().unwrap_or(false);
    }
    let Some(map) = lines_for(st, &v) else { return json!({"ok": false, "error": "no frame"}) };
    let p = &st.profile;
    let start = parse_points(body, p).filter(|x| x.len() >= 4).and_then(|x| placement_from_points(&x, p, model, aspect));
    let field = (p.field.length, p.field.width);
    let Some(r) = autocal::fit(&map, &p.lines, field, aspect, Hints { outside, lens: model }, start) else { return json!({"ok": false}) };
    let Some(m) = autocal::Mapper::new(&r.placement, field) else { return json!({"ok": false}) };
    let pts: Vec<Value> = p
        .landmarks
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let q = m.to_image(Vec2::new(l.x, l.y))?;
            (q.x > 0.003 && q.y > 0.003 && q.x < 0.997 && q.y < 0.997).then(|| json!({"id": i, "u": q.x, "v": q.y}))
        })
        .collect();
    json!({"ok": true, "points": pts, "agreement": r.agreement})
}

/// The lens model the request asks for (`"lens": "fisheye" | "panorama" | "none"`; older pages send
/// `"fisheye": true`).
fn lens_choice(body: &str) -> Option<LensModel> {
    let v = serde_json::from_str::<Value>(body).ok()?;
    match v["lens"].as_str() {
        Some(s) => LensModel::parse(s),
        None => (v["fisheye"] == true).then_some(LensModel::Division),
    }
}

fn lens_json(p: &HomographyProjector) -> Value {
    p.lens.map(|l| json!({"model": l.model.key(), "k1": l.k1, "k2": l.k2, "cy": l.cy, "fov": l.fov.to_degrees()})).unwrap_or(Value::Null)
}

/// `?c=r,g,b` of the line picture.
fn line_color_query(full_path: &str) -> Option<[u8; 3]> {
    let q = full_path.split_once('?')?.1;
    let c = q.split('&').find_map(|kv| kv.strip_prefix("c="))?;
    let v: Vec<u8> = c.split(',').filter_map(|x| x.parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// `points`: `{id, u, v}` for a landmark (id = its index), `{id, court: [x, y], u, v}` for a point on
/// a field line.
fn parse_points(body: &str, profile: &SportProfile) -> Option<Vec<Pair>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out = Vec::new();
    for p in v["points"].as_array()? {
        let pano = Vec2::new(p["u"].as_f64()? as f32, p["v"].as_f64()? as f32);
        if let Some(c) = p["court"].as_array() {
            let court = Vec2::new(c.first()?.as_f64()? as f32, c.get(1)?.as_f64()? as f32);
            out.push(Pair { id: p["id"].clone(), name: "line_point".into(), court, pano });
        } else {
            let id = p["id"].as_u64()? as usize;
            let l = profile.landmarks.get(id)?;
            out.push(Pair { id: json!(id), name: l.key.clone(), court: Vec2::new(l.x, l.y), pano });
        }
    }
    Some(out)
}

pub(crate) fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Method, full path (with query) and body of one HTTP request.
pub(crate) fn read_request(s: &mut TcpStream) -> Option<(String, String, String)> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
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
    let first = head.lines().next().unwrap_or("");
    let (method, full_path) = {
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
    Some((method.to_string(), full_path.to_string(), body))
}

fn handle(mut s: TcpStream, st: &Shared) {
    let Some((method, full_path, body)) = read_request(&mut s) else { return };
    let (method, path) = (method.as_str(), full_path.split('?').next().unwrap_or(""));
    let (model, aspect) = (lens_choice(&body), st.info.w as f64 / st.info.h.max(1) as f64);

    match (method, path) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("calib.html").as_bytes()),
        ("GET", "/theme.css") => respond(&mut s, "200 OK", "text/css; charset=utf-8", include_str!("theme.css").as_bytes()),
        ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
        ("GET", "/frame.png") => respond(&mut s, "200 OK", "image/png", st.plate_png.as_ref().unwrap_or(&st.png)),
        ("GET", "/original.png") => respond(&mut s, "200 OK", "image/png", &st.png),
        ("GET", "/lines.jpg") => match st.line_map(line_color_query(&full_path)).map(|m| super::scoreboard::encode_jpeg(m.w, m.h, &m.mask_rgb())) {
            Some(Ok(jpg)) => respond(&mut s, "200 OK", "image/jpeg", &jpg),
            _ => respond(&mut s, "404 Not Found", "text/plain", b"no frame"),
        },
        ("POST", "/api/lines") => match lines_for(st, &serde_json::from_str(&body).unwrap_or(Value::Null)).map(|m| super::scoreboard::encode_jpeg(m.w, m.h, &m.mask_rgb())) {
            Some(Ok(jpg)) => respond(&mut s, "200 OK", "image/jpeg", &jpg),
            _ => respond(&mut s, "404 Not Found", "text/plain", b"no frame"),
        },
        ("POST", "/api/snap") => respond(&mut s, "200 OK", "application/json", snap_json(st, &body, model, aspect).to_string().as_bytes()),
        ("POST", "/api/auto") => respond(&mut s, "200 OK", "application/json", auto_json(st, &body, model, aspect).to_string().as_bytes()),
        ("GET", "/api/info") => {
            let v = json!({"w": st.info.w, "h": st.info.h, "sport": st.profile.id, "lens": st.lens.map(|m| m.key()).unwrap_or("none"), "lens_min": LensLevel::MIN_POINTS, "field": [st.profile.field.length, st.profile.field.width], "lang": st.lang, "plate": st.plate_png.is_some(),
                           "landmarks": st.profile.landmarks.iter().map(|l| json!({"key": l.key, "x": l.x, "y": l.y})).collect::<Vec<_>>(),
                           // field markings in metres for the legend that shows where the current landmark lies
                           "lines": st.profile.lines.iter().map(|pl| pl.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()).collect::<Vec<_>>()});
            respond(&mut s, "200 OK", "application/json", v.to_string().as_bytes());
        }
        ("POST", "/api/solve") => {
            let r = match parse_points(&body, &st.profile).filter(|p| p.len() >= 4).and_then(|p| solve(&p, model, aspect)) {
                Some((proj, res, rms)) => match proj.to_pano_fn() {
                    Some(to_pano) => {
                        // straight lines bend through a lens: a fine step keeps the drawn curves smooth
                        let step = if proj.lens.is_some() { 0.25 } else { 0.5 };
                        let overlay: Vec<Vec<[f32; 2]>> = st.profile.densified_lines(step).iter().map(|pl| pl.iter().filter_map(|c| to_pano(*c)).map(|p| [p.x, p.y]).collect()).collect();
                        // the same lines with their field coordinates, so the page can offer the spots
                        // where a line leaves the picture as handles: [u, v, x, y] per vertex
                        let lines: Vec<Vec<[f32; 4]>> = st
                            .profile
                            .densified_lines(step)
                            .iter()
                            .map(|pl| pl.iter().filter_map(|c| to_pano(*c).map(|p| [p.x, p.y, c.x, c.y])).collect())
                            .collect();
                        json!({"ok": true, "rms": rms, "lens": lens_json(&proj), "residuals": res.iter().map(|(i, m)| json!({"id": i, "m": m})).collect::<Vec<_>>(), "overlay": overlay, "lines": lines})
                    }
                    None => json!({"ok": false}),
                },
                None => json!({"ok": false}),
            };
            respond(&mut s, "200 OK", "application/json", r.to_string().as_bytes());
        }
        ("POST", "/api/save") => {
            let r = match parse_points(&body, &st.profile).filter(|p| p.len() >= 4).and_then(|p| solve(&p, model, aspect).map(|r| (p, r))) {
                Some((pairs, (proj, _res, rms))) => {
                    let cal = CourtCalib {
                        pano_w: st.info.w,
                        pano_h: st.info.h,
                        sport: st.profile.id.clone(),
                        field_m: (st.profile.field.length, st.profile.field.width),
                        team_size: st.profile.team_size,
                        referees: st.profile.referees,
                        rms_m: rms,
                        lens: proj.lens,
                        points: pairs.iter().map(|p| CalPoint { name: p.name.clone(), pano: p.pano, court: p.court }).collect(),
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
/// `plate`: a clear view of the hall (players removed, see `video::median_plate`) - shown first and
/// used by the automatic calibration; the frame at `t` stays available on the page.
/// `original`: the single frame when the source cannot seek (a live stream).
pub fn run(input: &str, info: &VideoInfo, t: f64, out_path: &str, open: bool, profile: SportProfile, lang: &str, lens: Option<LensModel>, plate: Option<Frame>, original: Option<Frame>) -> Result<(), String> {
    let png = match &original {
        Some(f) => f.png()?,
        None => grab_png(input, t)?,
    };
    let plate_png = match &plate {
        Some(p) => Some(p.png()?),
        None => None,
    };
    // the automatic calibration works on the decoded frame; without it the page still works by hand
    let frame = plate.or(original).or_else(|| grab(input, info, t).ok());
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let st = Arc::new(Shared { png, plate_png, info: info.clone(), out_path: out_path.to_string(), video: input.to_string(), profile, lang: lang.to_string(), lens, frame, lines: std::sync::Mutex::new(None), done: AtomicBool::new(false) });
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
