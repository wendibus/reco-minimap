//! `render`: new videos from a finished analysis (`run --json`) and its source video, without
//! detecting again:
//!
//! * **autotrack** (`--track out.mp4`): a 16:9 part of the panorama that follows the play - the
//!   players on the court and the ball - like a camera operator, smoothed so it pans calmly;
//! * **panorama** (`--pip out.mp4`): the whole picture with the minimap in the corner;
//!
//! both with the scoreboard and watermark of `--overlay`, the minimap if wanted, and the sound of
//! the source if wanted (`--audio`).

use std::path::Path;

use serde_json::Value;

use super::overlay::{Overlay, Timeline};
use super::video::{Codec, FrameReader, FrameWriter, VideoInfo};
use crate::ball::{BallOut, BallState};
use crate::geom::{Projector, Vec2};
use crate::identity::IdentityAssignment;
use crate::render::{Renderer, Theme};
use crate::sport::SportProfile;
use crate::team::Team;
use crate::tracker::TrackState;
use crate::{tr, trs, AgentOut, FrameState};

// ---------------------------------------------------------------- result frames back to states

fn team(v: &Value) -> Team {
    match v.as_str() {
        Some("home") => Team::Home,
        Some("away") => Team::Away,
        _ => Team::Unknown,
    }
}

fn agent(v: &Value, referee: bool) -> Option<AgentOut> {
    let f = |k: &str| v[k].as_f64().map(|x| x as f32);
    Some(AgentOut {
        segment: v["segment"].as_u64().unwrap_or(0) as u32,
        identity: IdentityAssignment::default(),
        id: v["id"].as_u64()? as u32,
        referee,
        team: team(&v["team"]),
        keeper: v["keeper"].as_bool().unwrap_or(false),
        pos: Vec2::new(f("x")?, f("y")?),
        vel: Vec2::new(f("vx").unwrap_or(0.0), f("vy").unwrap_or(0.0)),
        state: match v["state"].as_str() {
            Some("occluded") => TrackState::Occluded,
            Some("lost") => TrackState::Lost,
            _ => TrackState::Visible,
        },
        alpha: f("alpha").unwrap_or(1.0),
        jersey: v["jersey"].as_u64().and_then(|n| u8::try_from(n).ok()),
    })
}

/// A line of a result file as the state the minimap draws.
pub fn state_from_json(v: &Value) -> FrameState {
    let b = &v["ball"];
    let pos = match (b["x"].as_f64(), b["y"].as_f64()) {
        (Some(x), Some(y)) => Some(Vec2::new(x as f32, y as f32)),
        _ => None,
    };
    let vel = match (b["vx"].as_f64(), b["vy"].as_f64()) {
        (Some(x), Some(y)) => Some(Vec2::new(x as f32, y as f32)),
        _ => None,
    };
    let state = match b["state"].as_str() {
        Some("held") => b["holder"].as_u64().map(|h| BallState::Held { track: h as u32 }).unwrap_or(BallState::Unknown),
        Some("flight") => BallState::InFlight,
        Some("loose") => BallState::Loose,
        _ => BallState::Unknown,
    };
    FrameState {
        t: v["t"].as_f64().unwrap_or(0.0),
        players: v["players"].as_array().into_iter().flatten().filter_map(|p| agent(p, false)).collect(),
        referees: v["referees"].as_array().into_iter().flatten().filter_map(|p| agent(p, true)).collect(),
        ball: BallOut { state, pos, vel },
        events: Vec::new(),
    }
}

// ---------------------------------------------------------------- where the play is in the picture

/// Field point -> panorama position (u, v in 0..1): the calibration turned around, as a table.
pub struct PanoLookup {
    cell: f32,
    nx: usize,
    ny: usize,
    x0: f32,
    y0: f32,
    uv: Vec<Option<(f32, f32)>>,
}

impl PanoLookup {
    pub fn new(proj: &dyn Projector, profile: &SportProfile) -> Self {
        let (l, w) = (profile.field.length, profile.field.width);
        let cell = 0.5f32;
        let (x0, y0) = (-4.0f32, -4.0f32);
        let (nx, ny) = (((l + 8.0) / cell).ceil() as usize + 1, ((w + 8.0) / cell).ceil() as usize + 1);
        let mut best: Vec<Option<(f32, f32, f32)>> = vec![None; nx * ny];
        // sample the picture densely, keep for every field cell the picture point that lands nearest
        let (su, sv) = (600, 160);
        for i in 0..=su {
            for j in 0..=sv {
                let (u, v) = (i as f32 / su as f32, 0.2 + 0.8 * j as f32 / sv as f32);
                let Some(c) = proj.to_court(u, v) else { continue };
                let (gx, gy) = (((c.x - x0) / cell).round(), ((c.y - y0) / cell).round());
                if gx < 0.0 || gy < 0.0 || gx as usize >= nx || gy as usize >= ny {
                    continue;
                }
                let k = gy as usize * nx + gx as usize;
                let d = (c.x - (x0 + gx * cell)).hypot(c.y - (y0 + gy * cell));
                if best[k].is_none_or(|b| d < b.2) {
                    best[k] = Some((u, v, d));
                }
            }
        }
        Self { cell, nx, ny, x0, y0, uv: best.into_iter().map(|b| b.map(|b| (b.0, b.1))).collect() }
    }

    pub fn at(&self, p: Vec2) -> Option<(f32, f32)> {
        let (gx, gy) = (((p.x - self.x0) / self.cell).round(), ((p.y - self.y0) / self.cell).round());
        if gx < 0.0 || gy < 0.0 || gx as usize >= self.nx || gy as usize >= self.ny {
            return None;
        }
        self.uv[gy as usize * self.nx + gx as usize]
    }
}

/// Where the play is in one result frame (panorama u, v): the middle of the players on the court,
/// pulled towards the ball.
fn focus(f: &FrameState, look: &PanoLookup) -> Option<(f32, f32)> {
    let mut pts: Vec<(f32, f32)> = f.players.iter().filter(|p| p.state != TrackState::Lost && p.alpha > 0.5).filter_map(|p| look.at(p.pos)).collect();
    if pts.len() < 2 {
        return None;
    }
    let mid = |k: usize, pts: &mut Vec<(f32, f32)>| {
        pts.sort_by(|a, b| if k == 0 { a.0.total_cmp(&b.0) } else { a.1.total_cmp(&b.1) });
        let p = pts[pts.len() / 2];
        if k == 0 { p.0 } else { p.1 }
    };
    let (mut u, v) = (mid(0, &mut pts), mid(1, &mut pts));
    if let Some(b) = f.ball.pos.filter(|_| f.ball.state != BallState::Unknown).and_then(|p| look.at(p)) {
        u = 0.55 * u + 0.45 * b.0;
    }
    // the feet are low in the picture: keep them in the lower part of the crop, heads and the
    // basket above them
    Some((u, v - 0.06))
}

/// The camera path: (t, u, v) of the crop's centre, smoothed and with a limited pan speed.
pub struct CameraPath {
    pts: Vec<(f64, f32, f32)>,
}

impl CameraPath {
    /// `half_w`: half the crop width in panorama units (the centre stays that far from the edges).
    pub fn new(raw: &[(f64, Option<(f32, f32)>)], half_w: f32, half_h: f32) -> Self {
        // gaps (nobody seen) keep the last known point
        let mut last = (0.5f32, 0.6f32);
        let filled: Vec<(f64, f32, f32)> = raw.iter().map(|(t, p)| {
            if let Some(p) = p {
                last = *p;
            }
            (*t, last.0, last.1)
        }).collect();
        if filled.is_empty() {
            return Self { pts: Vec::new() };
        }
        // Gaussian smoothing over time (sigma 1.2 s)
        let sigma = 1.2f64;
        let mut sm = Vec::with_capacity(filled.len());
        let mut lo = 0usize;
        for i in 0..filled.len() {
            let t = filled[i].0;
            while filled[lo].0 < t - 3.0 * sigma {
                lo += 1;
            }
            let (mut su, mut sv, mut sw) = (0f64, 0f64, 0f64);
            for p in filled[lo..].iter().take_while(|p| p.0 <= t + 3.0 * sigma) {
                let w = (-(p.0 - t).powi(2) / (2.0 * sigma * sigma)).exp();
                su += w * p.1 as f64;
                sv += w * p.2 as f64;
                sw += w;
            }
            sm.push((t, (su / sw) as f32, (sv / sw) as f32));
        }
        // limited pan speed (a calm camera): at most 0.6 crop widths per second
        let max_speed = 1.2 * half_w;
        for i in 1..sm.len() {
            let dt = (sm[i].0 - sm[i - 1].0).max(1e-3) as f32;
            let lim = max_speed * dt;
            let du = (sm[i].1 - sm[i - 1].1).clamp(-lim, lim);
            let dv = (sm[i].2 - sm[i - 1].2).clamp(-lim, lim);
            sm[i].1 = sm[i - 1].1 + du;
            sm[i].2 = sm[i - 1].2 + dv;
        }
        for p in sm.iter_mut() {
            p.1 = p.1.clamp(half_w, (1.0 - half_w).max(half_w));
            p.2 = p.2.clamp(half_h, (1.0 - half_h).max(half_h));
        }
        Self { pts: sm }
    }

    pub fn at(&self, t: f64) -> (f32, f32) {
        if self.pts.is_empty() {
            return (0.5, 0.5);
        }
        let i = self.pts.partition_point(|p| p.0 <= t);
        if i == 0 {
            return (self.pts[0].1, self.pts[0].2);
        }
        if i >= self.pts.len() {
            let p = self.pts[self.pts.len() - 1];
            return (p.1, p.2);
        }
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let k = ((t - a.0) / (b.0 - a.0).max(1e-6)) as f32;
        (a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k)
    }
}

// ---------------------------------------------------------------- sound

/// Put the sound of `source` (from `start`, `duration` long) under `video` (in place).
pub fn add_audio(video: &str, source: &str, start: f64, duration: f64) -> Result<(), String> {
    let tmp = format!("{video}.audio.mp4");
    let st = std::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i", video, "-ss", &format!("{start:.3}"), "-t", &format!("{duration:.3}"), "-i", source, "-map", "0:v:0", "-map", "1:a:0?", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-shortest", "-movflags", "+faststart", &tmp])
        .status()
        .map_err(|e| tr!("ffmpeg nicht gefunden ({e})", "ffmpeg not found ({e})", "ffmpeg introuvable ({e})", "no se encuentra ffmpeg ({e})"))?;
    if !st.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(trs!("Ton konnte nicht übernommen werden", "the sound could not be added", "le son n'a pas pu être ajouté", "no se pudo añadir el sonido").into());
    }
    std::fs::rename(&tmp, video).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- the run

pub struct Params<'a> {
    pub result: &'a str,
    pub video: &'a str,
    pub info: &'a VideoInfo,
    pub profile: SportProfile,
    pub projector: Box<dyn Projector>,
    pub theme: Theme,
    pub overlay: Overlay,
    /// Extra score events (hand-confirmed baskets) and whether they replace the file's own.
    pub manual: Option<(Vec<Value>, bool)>,
    pub track: Option<&'a str>,
    pub pip: Option<&'a str>,
    /// Size of the autotrack video.
    pub track_size: (usize, usize),
    /// 1.0: the crop is as tall as the panorama; less zooms in.
    pub zoom: f32,
    pub minimap: bool,
    pub audio: bool,
    pub codec: Codec,
    /// Only this part of the result (start, end in video seconds).
    pub span: Option<(f64, f64)>,
}

/// The result file's frames (the first line may be a header without `t`).
pub fn read_result(path: &str) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["t"].is_number()).collect())
}

/// The score over time of a result: its own score events or, with hand-confirmed baskets, those.
pub fn timeline(frames: &[Value], profile: &SportProfile, manual: Option<&(Vec<Value>, bool)>) -> Timeline {
    let readings = super::scoreboard::readings_in(frames);
    let goals = if profile.goals.len() == 2 { [profile.goals[0], profile.goals[1]] } else { [Vec2::new(0.0, 0.0); 2] };
    let mut ev = match manual {
        Some((_, true)) => Vec::new(),
        _ => crate::live_score::scores_of(frames, &readings, goals),
    };
    if let Some((m, replace)) = manual {
        let extra: Vec<_> = m.iter().filter_map(crate::live_score::parse_event).collect();
        ev = if *replace { crate::live_score::final_events(&extra) } else {
            let mut all = ev;
            all.extend(extra);
            crate::live_score::final_events(&all)
        };
    }
    Timeline::from_events(&ev, &readings)
}

pub fn render(p: Params, mut step: impl FnMut(f64, &str)) -> Result<Vec<String>, String> {
    let frames = read_result(p.result)?;
    if frames.len() < 2 {
        return Err(trs!("zu wenige Bilder in der Datei", "too few frames in the file", "trop peu d'images dans le fichier", "muy pocas imágenes en el archivo").into());
    }
    let states: Vec<FrameState> = frames.iter().map(state_from_json).collect();
    let (mut t0, mut t1) = (states[0].t, states[states.len() - 1].t);
    if let Some((a, b)) = p.span {
        (t0, t1) = (t0.max(a), t1.min(b));
    }
    if t1 - t0 < 1.0 {
        return Err(trs!("der gewählte Abschnitt liegt nicht in der Analyse", "the chosen part is not in the analysis", "la partie choisie n'est pas dans l'analyse", "la parte elegida no está en el análisis").into());
    }
    let tl = timeline(&frames, &p.profile, p.manual.as_ref());
    drop(frames);
    let (fw, fh) = (p.info.w as usize, p.info.h as usize);
    let every = if p.info.fps > 50.0 { 2 } else { 1 };
    let fps = p.info.fps / every as f64;
    let mut outs = Vec::new();

    // autotrack: crop size and camera path
    let crop_h = ((fh as f32 * p.zoom.clamp(0.3, 1.0)) as usize).min(fh) & !1;
    let crop_w = ((crop_h * 16 / 9).min(fw)) & !1;
    let cam = p.track.map(|_| {
        step(0.0, &trs!("Kamerafahrt wird berechnet …", "Working out the camera path …", "Calcul du mouvement de caméra …", "Calculando el movimiento de cámara …"));
        let look = PanoLookup::new(p.projector.as_ref(), &p.profile);
        let raw: Vec<(f64, Option<(f32, f32)>)> = states.iter().map(|s| (s.t, focus(s, &look))).collect();
        CameraPath::new(&raw, crop_w as f32 / fw as f32 / 2.0, crop_h as f32 / fh as f32 / 2.0)
    });

    // the minimap: as wide as 28 % (panorama) / 30 % (autotrack) of the picture
    let mm_for = |width: usize, frac: f32| {
        let mut th = p.theme.clone();
        let kf = (p.profile.field.length / 28.0).sqrt().max(0.8);
        th.px_per_m = (frac * width as f32 / (p.profile.field.length + 2.0 * th.pad_m * kf)).max(1.0);
        th.background_alpha = 215;
        Renderer::with_profile(th, p.profile.clone())
    };
    let r_pip = (p.minimap && p.pip.is_some()).then(|| mm_for(fw, 0.28));
    let r_track = (p.minimap && p.track.is_some()).then(|| mm_for(crop_w, 0.30));

    let mut w_pip = match p.pip {
        Some(path) => Some(FrameWriter::open(path, fw, fh, fps, false, &p.codec)?),
        None => None,
    };
    let mut w_track = match p.track {
        Some(path) => Some(FrameWriter::open_scaled(path, crop_w, crop_h, fps, p.track_size, &p.codec)?),
        None => None,
    };
    let dur = t1 - t0;
    let mut reader = FrameReader::open(p.video, p.info, t0, Some(dur), every)?;
    let (mut k, mut i) = (0u64, 0usize);
    let total = (dur * fps).max(1.0);
    let started = std::time::Instant::now();
    while let Some(mut frame) = reader.next() {
        if super::progress::stop_requested() {
            break;
        }
        let t = t0 + k as f64 * every as f64 / p.info.fps;
        k += 1;
        while i + 1 < states.len() && states[i + 1].t <= t {
            i += 1;
        }
        let state = if i + 1 < states.len() {
            let (a, b) = (&states[i], &states[i + 1]);
            a.lerp(b, (((t - a.t) / (b.t - a.t).max(1e-6)) as f32).clamp(0.0, 1.0))
        } else {
            states[i].clone()
        };
        let score = Some(tl.at(t));
        if let (Some(w), Some(cam)) = (w_track.as_mut(), cam.as_ref()) {
            let (u, v) = cam.at(t);
            let x = ((u * fw as f32) as usize).saturating_sub(crop_w / 2);
            let y = ((v * fh as f32) as usize).saturating_sub(crop_h / 2);
            let mut c = frame.crop(x, y, crop_w, crop_h);
            if let Some(r) = r_track.as_ref() {
                let m = r.render(&state);
                c.blit(&m, c.w.saturating_sub(m.w + 24), c.h.saturating_sub(m.h + 24));
            }
            p.overlay.draw(&mut c, score);
            w.write(&c.rgb)?;
        }
        if let Some(w) = w_pip.as_mut() {
            if let Some(r) = r_pip.as_ref() {
                let m = r.render(&state);
                frame.blit(&m, frame.w.saturating_sub(m.w + 24), frame.h.saturating_sub(m.h + 24));
            }
            p.overlay.draw(&mut frame, score);
            w.write(&frame.rgb)?;
        }
        if k % 30 == 0 {
            let f = (k as f64 / total).min(1.0);
            let el = started.elapsed().as_secs_f64();
            let rest = if f > 0.01 { el / f - el } else { 0.0 };
            step(f, &tr!("{} von {} · noch ~{} min", "{} of {} · ~{} min left", "{} sur {} · encore ~{} min", "{} de {} · quedan ~{} min", clock(t - t0), clock(dur), (rest / 60.0).ceil() as u64));
        }
    }
    for (w, path) in [(w_track, p.track), (w_pip, p.pip)] {
        if let (Some(w), Some(path)) = (w, path) {
            w.finish()?;
            if p.audio {
                step(1.0, &trs!("Ton wird übernommen …", "Adding the sound …", "Ajout du son …", "Añadiendo el sonido …"));
                add_audio(path, p.video, t0, dur)?;
            }
            outs.push(path.to_string());
        }
    }
    Ok(outs)
}

fn clock(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `<stem>.baskets.json` next to a result: the baskets confirmed in a live replay.
pub fn baskets_path(result: &Path) -> std::path::PathBuf {
    let stem = result.file_stem().and_then(|s| s.to_str()).unwrap_or("result");
    result.with_file_name(format!("{stem}.baskets.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_result_line_becomes_a_state() {
        let v = json!({"t": 3.5, "players": [{"id": 4, "segment": 1, "team": "away", "x": 10.0, "y": 5.0, "vx": 1.0, "vy": 0.0, "state": "occluded", "alpha": 0.6, "jersey": 23}],
            "referees": [{"id": 9, "team": "none", "x": 1.0, "y": 2.0}], "ball": {"state": "held", "holder": 4, "x": 10.0, "y": 5.0}});
        let s = state_from_json(&v);
        assert_eq!((s.t, s.players.len(), s.referees.len()), (3.5, 1, 1));
        let p = &s.players[0];
        assert_eq!((p.id, p.segment, p.team, p.state, p.jersey), (4, 1, Team::Away, TrackState::Occluded, Some(23)));
        assert!(matches!(s.ball.state, BallState::Held { track: 4 }));
    }

    #[test]
    fn the_camera_pans_calmly_and_stays_inside_the_picture() {
        // the play jumps from the left to the right end
        let raw: Vec<(f64, Option<(f32, f32)>)> = (0..200).map(|i| (i as f64 * 0.1, Some((if i < 100 { 0.1 } else { 0.9 }, 0.6)))).collect();
        let cam = CameraPath::new(&raw, 0.24, 0.5);
        assert!((cam.at(0.0).0 - 0.24).abs() < 1e-3, "clamped to the left edge: {:?}", cam.at(0.0));
        // one second after the jump it is on its way, not there
        let mid = cam.at(11.0).0;
        assert!(mid > 0.3 && mid < 0.76, "{mid}");
        assert!((cam.at(19.9).0 - 0.76).abs() < 1e-3, "{:?}", cam.at(19.9));
        // never faster than 1.2 half-widths per second
        let mut prev = cam.at(0.0).0;
        for i in 1..200 {
            let u = cam.at(i as f64 * 0.1).0;
            assert!((u - prev).abs() <= 1.2 * 0.24 * 0.1 + 1e-4);
            prev = u;
        }
    }
}
