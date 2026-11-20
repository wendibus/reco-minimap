//! Hall scoreboard (basketball): the calibration file `scoreboard.json`, reading the board from a
//! video (its own ffmpeg stream that only decodes the board region, at the full frame rate) and
//! the calibration page (local web page on 127.0.0.1, like the field calibration).
//!
//! The reading itself is in [`crate::scoreboard`]; matching score changes to shot attempts in
//! [`crate::scoring`].

use crate::{tr, trs};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::calib::{open_browser, read_request, respond};
use super::video::{FrameReader, VideoInfo};
use crate::scoreboard::cells::CellReader;
use crate::scoreboard::find::{self, Burst, BurstBuilder, Candidate};
use crate::scoreboard::quad::{self, Quad};
use crate::scoreboard::{Board, Channel, Config, DetectCfg, FieldCfg, HomeSide, Reading};

// ---------------------------------------------------------------- file

fn f64s<const N: usize>(v: &Value) -> Option<[f64; N]> {
    let a = v.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut o = [0.0; N];
    for (i, x) in a.iter().enumerate() {
        o[i] = x.as_f64()?;
    }
    Some(o)
}

fn field_json(f: &FieldCfg) -> Value {
    json!({"rect": f.rect, "digits": f.digits, "gap_ratio": f.gap_ratio, "slant": f.slant, "colon": f.colon})
}

fn field_from(v: &Value) -> Option<FieldCfg> {
    Some(FieldCfg {
        rect: f64s::<4>(&v["rect"])?,
        digits: v["digits"].as_u64()? as usize,
        gap_ratio: v["gap_ratio"].as_f64().unwrap_or(0.25),
        slant: v["slant"].as_f64().unwrap_or(0.0),
        colon: v["colon"].as_f64().unwrap_or(0.0),
    })
}

fn quad_from(v: &Value) -> Option<Quad> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut q = [[0.0; 2]; 4];
    for (i, p) in a.iter().enumerate() {
        q[i] = f64s::<2>(p)?;
    }
    Some(q)
}

pub fn to_json(c: &Config) -> Value {
    let d = &c.detect;
    let mut v = json!({
        "reco_minimap_scoreboard": 1,
        "frame": c.frame,
        "home": match c.home { HomeSide::Left => "left", HomeSide::Right => "right" },
        "detect": {"window_ms": d.window_ms, "percentile": d.percentile, "clock_stable_ms": d.clock_stable_ms, "clock_share": d.clock_share,
                   "score_stable_ms": d.score_stable_ms, "score_share": d.score_share, "period_stable_ms": d.period_stable_ms,
                   "min_contrast": d.min_contrast, "threshold": d.threshold, "channel": d.channel.key(), "invert": d.invert},
        "left_score": field_json(&c.left_score), "right_score": field_json(&c.right_score), "clock": field_json(&c.clock),
    });
    if let Some(b) = &c.board {
        v["board"] = json!({"corners": b.corners, "size": b.size});
    }
    if let Some(p) = &c.period {
        v["period"] = field_json(p);
    }
    if let Some(t) = c.time {
        v["time"] = json!(t);
    }
    v
}

pub fn from_json(v: &Value) -> Result<Config, String> {
    let bad = |k: &str| tr!("scoreboard.json: „{k}“ fehlt oder ist ungültig", "scoreboard.json: “{k}” is missing or invalid", "scoreboard.json : « {k} » manquant ou invalide", "scoreboard.json: «{k}» falta o no es válido");
    let frame = f64s::<2>(&v["frame"]).ok_or_else(|| bad("frame"))?;
    let board = match &v["board"] {
        Value::Null => None,
        b => Some(Board { corners: quad_from(&b["corners"]).ok_or_else(|| bad("board.corners"))?, size: f64s::<2>(&b["size"]).map(|s| [s[0] as u32, s[1] as u32]).ok_or_else(|| bad("board.size"))? }),
    };
    let dv = &v["detect"];
    let df = DetectCfg::default();
    let g = |k: &str, d: f64| dv[k].as_f64().unwrap_or(d);
    let detect = DetectCfg {
        window_ms: g("window_ms", df.window_ms),
        percentile: g("percentile", df.percentile),
        clock_stable_ms: g("clock_stable_ms", df.clock_stable_ms),
        clock_share: g("clock_share", df.clock_share),
        score_stable_ms: g("score_stable_ms", df.score_stable_ms),
        score_share: g("score_share", df.score_share),
        period_stable_ms: g("period_stable_ms", df.period_stable_ms),
        min_contrast: g("min_contrast", df.min_contrast),
        threshold: g("threshold", df.threshold),
        channel: dv["channel"].as_str().and_then(Channel::parse).unwrap_or(df.channel),
        invert: dv["invert"].as_bool().unwrap_or(df.invert),
    };
    let c = Config {
        frame: [frame[0] as u32, frame[1] as u32],
        board,
        detect,
        left_score: field_from(&v["left_score"]).ok_or_else(|| bad("left_score"))?,
        right_score: field_from(&v["right_score"]).ok_or_else(|| bad("right_score"))?,
        clock: field_from(&v["clock"]).ok_or_else(|| bad("clock"))?,
        period: field_from(&v["period"]),
        home: if v["home"] == "right" { HomeSide::Right } else { HomeSide::Left },
        time: v["time"].as_f64(),
    };
    c.validate().map_err(|e| format!("scoreboard.json: {e}"))?;
    Ok(c)
}

pub fn load(path: &str) -> Result<Config, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    from_json(&serde_json::from_str(&txt).map_err(|e| format!("{path}: {e}"))?)
}

pub fn save(c: &Config, path: &str) -> Result<(), String> {
    std::fs::write(path, serde_json::to_string_pretty(&to_json(c)).unwrap()).map_err(|e| format!("{path}: {e}"))
}

/// A reading as a frame event of the result file (`{"type": "scoreboard", ...}`).
pub fn reading_json(r: &Reading) -> Value {
    json!({"type": "scoreboard", "t": (r.t * 1000.0).round() / 1000.0, "home": r.home, "away": r.away, "clock": r.clock, "period": r.period})
}

/// A reading from a result-file event or a line of `reco-scoreboard --data-out`
/// (`{"t", "home", "guest", "clock": "MM:SS", "period"}`).
pub fn parse_reading(v: &Value) -> Option<Reading> {
    let n = |k: &str| v[k].as_u64().map(|x| x as u32);
    let clock = n("clock").or_else(|| {
        let (m, s) = v["clock"].as_str()?.split_once(':')?;
        Some(m.trim().parse::<u32>().ok()? * 60 + s.trim().parse::<u32>().ok()?)
    });
    Some(Reading { t: v["t"].as_f64()?, home: n("home"), away: n("away").or_else(|| n("guest")), clock, period: n("period") })
}

/// All scoreboard readings in a result file's frames, in time order.
pub fn readings_in(frames: &[Value]) -> Vec<Reading> {
    let mut v: Vec<Reading> = frames.iter().flat_map(|f| f["events"].as_array().into_iter().flatten()).filter(|e| e["type"] == "scoreboard").filter_map(parse_reading).collect();
    v.sort_by(|a, b| a.t.total_cmp(&b.t));
    v
}

/// How well the hall scoreboard was read in a result: the share of readings with both scores,
/// with the clock and the period, and how often a score went down (a misread digit). `ok` is false
/// when the points of the match cannot be trusted - the evaluation then says so.
pub fn quality(readings: &[Reading]) -> Value {
    let n = readings.len();
    if n == 0 {
        return json!({"readings": 0, "ok": false});
    }
    let share = |f: &dyn Fn(&Reading) -> bool| (readings.iter().filter(|r| f(r)).count() as f64 / n as f64 * 100.0).round() / 100.0;
    let both = share(&|r| r.home.is_some() && r.away.is_some());
    let (mut last, mut drops) = ([None::<u32>; 2], 0usize);
    for r in readings {
        for (i, v) in [r.home, r.away].into_iter().enumerate() {
            if let Some(v) = v {
                if last[i].is_some_and(|l| v < l) {
                    drops += 1;
                }
                last[i] = Some(v);
            }
        }
    }
    let span = (readings[n - 1].t - readings[0].t).max(60.0);
    let drops_10 = (drops as f64 / span * 600.0 * 10.0).round() / 10.0;
    json!({"readings": n, "both": both, "home": share(&|r| r.home.is_some()), "away": share(&|r| r.away.is_some()),
        "clock": share(&|r| r.clock.is_some()), "period": share(&|r| r.period.is_some()),
        "drops_per_10min": drops_10, "ok": both >= 0.6 && drops_10 < 3.0})
}

// ---------------------------------------------------------------- reading a video

/// ffmpeg that decodes only `region` of the video, every frame, as RGB24.
fn spawn_crop(input: &str, region: [usize; 4], start: f64, duration: Option<f64>) -> Result<Child, String> {
    let mut c = Command::new("ffmpeg");
    // no hardware decoding here: for a small crop the copy of every full frame back from the GPU
    // makes it several times slower than decoding in software
    c.args(["-v", "error", "-nostdin"]);
    if start > 0.0 {
        c.args(["-ss", &format!("{start}")]);
    }
    c.args(["-i", input]);
    if let Some(d) = duration {
        c.args(["-t", &format!("{d}")]);
    }
    let [x, y, w, h] = region;
    c.args(["-an", "-sn", "-vf", &format!("crop={w}:{h}:{x}:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    c.stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|e| tr!("ffmpeg nicht gefunden ({e})", "ffmpeg not found ({e})", "ffmpeg introuvable ({e})", "no se encuentra ffmpeg ({e})"))
}

/// The digit reader for `region`. When the reading does not start at the calibration frame, the
/// board's position there (a few seconds around it) becomes the reference for its drift.
fn reader(input: &str, info: &VideoInfo, cfg: &Config, region: [usize; 4], start: f64) -> Result<CellReader, String> {
    let (w, h) = (info.w as usize, info.h as usize);
    let mut r = CellReader::new(cfg, w, h, region, info.fps);
    if let Some(tc) = cfg.time.filter(|tc| (tc - start).abs() > 5.0) {
        let mut probe = CellReader::new(cfg, w, h, region, info.fps);
        let mut child = spawn_crop(input, region, (tc - 5.0).max(0.0), Some(12.0))?;
        if let Some(mut out) = child.stdout.take() {
            let mut buf = vec![0u8; region[2] * region[3] * 3];
            while out.read_exact(&mut buf).is_ok() {
                probe.push_frame(&buf);
            }
        }
        let _ = child.wait();
        if let Some(e) = probe.reference_now() {
            r.set_reference(e);
        }
    }
    Ok(r)
}

/// The scoreboard read from the analysed frames themselves (live sources, where no second video
/// stream of the board region exists). `fps`: frames per second that reach [`FrameBoard::push`].
pub struct FrameBoard {
    reader: CellReader,
    region: [usize; 4],
    fps: f64,
}

impl FrameBoard {
    pub fn new(cfg: &Config, w: usize, h: usize, fps: f64) -> Self {
        let region = cfg.region(w, h, 8);
        Self { reader: CellReader::new(cfg, w, h, region, fps.max(1.0)), region, fps: fps.max(1.0) }
    }

    /// One analysed frame at video time `t`; a reading at the end of every window.
    pub fn push(&mut self, frame: &super::video::Frame, t: f64) -> Option<Reading> {
        let [x, y, w, h] = self.region;
        if x + w > frame.w as usize || y + h > frame.h as usize {
            return None;
        }
        let (cw, ch, rgb) = frame.crop_rgb(x as i64, y as i64, (x + w) as i64, (y + h) as i64);
        if cw != w || ch != h {
            return None;
        }
        let raw = self.reader.push_frame(&rgb)?;
        let tm = t - (self.reader.window_frames() as f64 - 1.0) / 2.0 / self.fps;
        Some(self.reader.reading(tm, &raw))
    }
}

/// The scoreboard read alongside the processing: a thread with its own ffmpeg stream of the board
/// region. Readings arrive in time order through [`Stream::take_until`].
pub struct Stream {
    rx: Receiver<Reading>,
    buffered: Vec<Reading>,
    /// Video time (ms) the thread has read up to.
    progress: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<usize, String>>>,
}

impl Stream {
    pub fn start(input: &str, info: &VideoInfo, cfg: &Config, start: f64, duration: Option<f64>) -> Result<Self, String> {
        let (w, h) = (info.w as usize, info.h as usize);
        let region = cfg.region(w, h, 8);
        let mut child = spawn_crop(input, region, start, duration)?;
        let mut out = child.stdout.take().ok_or("no ffmpeg stdout")?;
        let mut det = reader(input, info, cfg, region, start)?;
        let (tx, rx) = channel();
        let progress = Arc::new(AtomicU64::new((start.max(0.0) * 1000.0) as u64));
        let (finished, stop) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (p2, f2, s2, fps) = (progress.clone(), finished.clone(), stop.clone(), info.fps.max(1.0));
        let handle = std::thread::spawn(move || {
            let mut buf = vec![0u8; region[2] * region[3] * 3];
            let mut i = 0usize;
            let res = loop {
                if s2.load(Ordering::Relaxed) {
                    break Ok(i);
                }
                if out.read_exact(&mut buf).is_err() {
                    break Ok(i);
                }
                let t = start + i as f64 / fps;
                if let Some(raw) = det.push_frame(&buf) {
                    // the reading belongs to the middle of its window
                    let tm = t - (det.window_frames() as f64 - 1.0) / 2.0 / fps;
                    if tx.send(det.reading(tm, &raw)).is_err() {
                        break Ok(i);
                    }
                }
                i += 1;
                p2.store((t * 1000.0) as u64, Ordering::Relaxed);
            };
            let _ = child.kill();
            let _ = child.wait();
            f2.store(true, Ordering::SeqCst);
            res
        });
        Ok(Self { rx, buffered: Vec::new(), progress, finished, stop, handle: Some(handle) })
    }

    /// The readings up to video time `t` received so far (does not wait).
    pub fn take_until(&mut self, t: f64) -> Vec<Reading> {
        self.buffered.extend(self.rx.try_iter());
        let n = self.buffered.partition_point(|r| r.t <= t);
        self.buffered.drain(..n).collect()
    }

    /// Wait until the stream has read up to `t` (or ended), stop it and return the rest up to `t`.
    pub fn finish_until(mut self, t: f64) -> Vec<Reading> {
        while !self.finished.load(Ordering::SeqCst) && (self.progress.load(Ordering::Relaxed) as f64) / 1000.0 < t {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.stop.store(true, Ordering::SeqCst);
        let rest = self.take_until(t);
        if let Some(h) = self.handle.take() {
            // dropping the receiver ends a thread blocked in send; the child is killed there
            drop(std::mem::replace(&mut self.rx, channel().1));
            let _ = h.join();
        }
        rest
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Read the whole range in one go (command `scoreboard`). `on_progress(t)` is called now and then.
pub fn read_video(input: &str, info: &VideoInfo, cfg: &Config, start: f64, duration: Option<f64>, mut on_progress: impl FnMut(f64)) -> Result<Vec<Reading>, String> {
    let (w, h) = (info.w as usize, info.h as usize);
    let region = cfg.region(w, h, 8);
    let mut child = spawn_crop(input, region, start, duration)?;
    let mut out = child.stdout.take().ok_or("no ffmpeg stdout")?;
    let mut det = reader(input, info, cfg, region, start)?;
    let mut buf = vec![0u8; region[2] * region[3] * 3];
    let mut v = Vec::new();
    let fps = info.fps.max(1.0);
    let mut i = 0usize;
    while out.read_exact(&mut buf).is_ok() {
        let t = start + i as f64 / fps;
        if let Some(raw) = det.push_frame(&buf) {
            v.push(det.reading(t - (det.window_frames() as f64 - 1.0) / 2.0 / fps, &raw));
        }
        if i % 250 == 0 {
            on_progress(t);
        }
        i += 1;
    }
    let _ = child.wait();
    Ok(v)
}

// ---------------------------------------------------------------- calibration page

const BURST_SECS: f64 = 6.0;
const BURST_FPS: f64 = 8.0;
/// Width of the rectified, enlarged board view (pixels).
const BOARD_W: usize = 1600;

/// JPEG of an RGB image, through ffmpeg (no image library in this program).
pub(crate) fn encode_jpeg(w: usize, h: usize, rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{w}x{h}"), "-i", "-", "-frames:v", "1", "-q:v", "2", "-f", "image2pipe", "-vcodec", "mjpeg", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| tr!("ffmpeg nicht gefunden ({e})", "ffmpeg not found ({e})", "ffmpeg introuvable ({e})", "no se encuentra ffmpeg ({e})"))?;
    let mut stdin = child.stdin.take().ok_or("no ffmpeg stdin")?;
    let data = rgb.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let mut out = Vec::new();
    child.stdout.take().ok_or("no ffmpeg stdout")?.read_to_end(&mut out).map_err(|e| e.to_string())?;
    let _ = writer.join();
    let _ = child.wait();
    if out.is_empty() {
        return Err(trs!("Bild konnte nicht kodiert werden", "could not encode the image", "impossible d'encoder l'image", "no se pudo codificar la imagen").into());
    }
    Ok(out)
}

fn scan(input: &str, info: &VideoInfo, t: f64) -> Result<(Burst, Vec<Candidate>), String> {
    let every = (info.fps / BURST_FPS).round().max(1.0) as u32;
    let mut rd = FrameReader::open(input, info, t, Some(BURST_SECS), every)?;
    let mut b = BurstBuilder::new(info.w as usize, info.h as usize, 4);
    while let Some(f) = rd.next() {
        b.push(&f.rgb);
    }
    if b.count() < 4 {
        return Err(tr!("Zu wenige Bilder ab {t:.0} s gelesen – einen früheren Zeitpunkt wählen.", "Too few frames read from {t:.0} s – choose an earlier time.", "Trop peu d'images lues à partir de {t:.0} s – choisissez un moment plus tôt.", "Se leyeron muy pocas imágenes desde {t:.0} s: elige un momento anterior."));
    }
    let burst = b.finish();
    let cands = find::find_boards(&burst);
    Ok((burst, cands))
}

/// Brightness of the brightest 10 % of the pixels – how "on" the digits are in this image.
fn bright_score(rgb: &[u8]) -> f64 {
    let mut v: Vec<u8> = rgb.chunks_exact(3).map(|p| p[0].max(p[1]).max(p[2])).collect();
    v.sort_unstable();
    let k = (v.len() / 10).max(1);
    v[v.len() - k..].iter().map(|&x| x as f64).sum::<f64>() / k as f64
}

struct Rectified {
    corners: Quad,
    size: [u32; 2],
    best: Vec<u8>,
    max: Vec<u8>,
}

fn rectify(burst: &Burst, corners: &Quad) -> Result<Rectified, String> {
    let q = quad::order_quad(corners);
    if !quad::is_valid_quad(&q) {
        return Err(trs!("Die vier Ecken ergeben kein gültiges Viereck.", "The four corners do not form a valid quad.", "Les quatre coins ne forment pas un quadrilatère valide.", "Las cuatro esquinas no forman un cuadrilátero válido.").into());
    }
    let (qw, qh) = quad::quad_size(&q);
    let bw = BOARD_W;
    let bh = ((bw as f64 * qh / qw).round() as usize).clamp(40, 1000);
    // the brightest moment of the burst inside the quad
    let (sw, sh) = (240usize, ((240.0 * bh as f64 / bw as f64).round() as usize).max(8));
    let best_i = burst.keep.iter().enumerate().map(|(i, f)| (i, bright_score(&quad::warp_quad(f, burst.w, burst.h, &q, sw, sh)))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0);
    let best_src = best_i.and_then(|i| burst.keep.get(i)).unwrap_or(&burst.first);
    Ok(Rectified { corners: q, size: [bw as u32, bh as u32], best: quad::warp_quad(best_src, burst.w, burst.h, &q, bw, bh), max: quad::warp_quad(&burst.max_rgb, burst.w, burst.h, &q, bw, bh) })
}

struct Session {
    time: f64,
    burst: Burst,
    cands: Vec<Candidate>,
    rect: Option<Rectified>,
    jpg_max: Option<Vec<u8>>,
    jpg_first: Option<Vec<u8>>,
}

fn cands_json(c: &[Candidate]) -> Value {
    Value::Array(c.iter().map(|c| json!({"corners": c.corners, "score": (c.score * 100.0).round() / 100.0})).collect())
}

fn fields_from(v: &Value) -> Vec<(String, FieldCfg)> {
    v.as_object().into_iter().flatten().filter_map(|(k, f)| field_from(f).map(|f| (k.clone(), f))).collect()
}

/// The page's state at start: video size, time, candidates and an existing calibration (scaled to this video).
fn info_json(s: &Session, info: &VideoInfo, existing: &Option<Config>, lang: &str) -> Value {
    let (saved, fields, home) = match existing {
        Some(c) => {
            let (sx, sy) = (info.w as f64 / c.frame[0] as f64, info.h as f64 / c.frame[1] as f64);
            let saved = c.board.as_ref().map(|b| json!({"corners": b.corners.iter().map(|p| [p[0] * sx, p[1] * sy]).collect::<Vec<_>>(), "size": b.size}));
            let mut m = serde_json::Map::new();
            for (name, f) in c.fields() {
                m.insert(name.to_string(), field_json(f));
            }
            (saved.unwrap_or(Value::Null), Value::Object(m), if c.home == HomeSide::Right { "right" } else { "left" })
        }
        None => (Value::Null, Value::Null, "left"),
    };
    json!({"w": info.w, "h": info.h, "time": s.time, "lang": lang, "candidates": cands_json(&s.cands), "saved": saved, "fields": fields, "home": home})
}

fn save_from(body: &str, sess: &mut Session, info: &VideoInfo, prev: Option<&Config>, out: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let corners = quad_from(&v["corners"]).ok_or_else(|| trs!("Ecken fehlen", "corners are missing", "coins manquants", "faltan las esquinas").to_string())?;
    let same = sess.rect.as_ref().is_some_and(|r| r.corners == quad::order_quad(&corners));
    if !same {
        sess.rect = Some(rectify(&sess.burst, &corners)?);
    }
    let r = sess.rect.as_ref().unwrap();
    let mut f: std::collections::HashMap<String, FieldCfg> = fields_from(&v["fields"]).into_iter().collect();
    let mut take = |k: &str, label: &str| f.remove(k).ok_or_else(|| tr!("Es fehlt noch ein Rahmen für: {label}", "A frame is still missing for: {label}", "Il manque encore un cadre pour : {label}", "Aún falta un marco para: {label}"));
    let cfg = Config {
        frame: [info.w, info.h],
        board: Some(Board { corners: r.corners, size: r.size }),
        detect: prev.map(|c| c.detect.clone()).unwrap_or_default(),
        left_score: take("left_score", trs!("Punkte links", "points left", "points à gauche", "puntos izquierda"))?,
        right_score: take("right_score", trs!("Punkte rechts", "points right", "points à droite", "puntos derecha"))?,
        clock: take("clock", trs!("Spielzeit", "game clock", "temps de jeu", "tiempo de juego"))?,
        period: f.remove("period"),
        home: if v["home"] == "right" { HomeSide::Right } else { HomeSide::Left },
        time: Some(sess.time),
    };
    cfg.validate()?;
    save(&cfg, out)
}

fn query(full: &str, key: &str) -> Option<String> {
    full.split_once('?')?.1.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// Serve the scoreboard calibration page until the user saves (or closes it with "Done").
pub fn calibrate(input: &str, info: &VideoInfo, t: f64, out: &str, open: bool, lang: &str) -> Result<(), String> {
    eprintln!("{}", tr!("Suche die Anzeigetafel ({BURST_SECS} s ab {t:.0} s) …", "Looking for the scoreboard ({BURST_SECS} s from {t:.0} s) …", "Recherche du tableau d'affichage ({BURST_SECS} s à partir de {t:.0} s) …", "Buscando el marcador ({BURST_SECS} s desde {t:.0} s) …"));
    let (burst, cands) = scan(input, info, t)?;
    eprintln!("{}", match cands.len() {
        0 => trs!("Keine Anzeige automatisch gefunden – im Browser die vier Ecken von Hand setzen.", "No board found automatically – set the four corners by hand in the browser.", "Aucun tableau trouvé automatiquement – placez les quatre coins à la main dans le navigateur.", "No se encontró el marcador automáticamente: marca las cuatro esquinas a mano en el navegador.").to_string(),
        n => tr!("{n} Kandidat(en) gefunden.", "{n} candidate(s) found.", "{n} candidat(s) trouvé(s).", "{n} candidato(s) encontrado(s)."),
    });
    let existing = load(out).ok();
    let mut sess = Session { time: t, burst, cands, rect: None, jpg_max: None, jpg_first: None };
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let url = format!("http://127.0.0.1:{port}/?lang={lang}");
    eprintln!("{}", tr!("Anzeigetafel kalibrieren: {url}\n(Seite öffnet sich automatisch; sonst URL im Browser öffnen. Strg+C zum Abbrechen.)", "Scoreboard calibration: {url}\n(The page opens by itself; otherwise open the URL in a browser. Ctrl+C to cancel.)", "Calibrage du tableau d'affichage : {url}\n(La page s'ouvre toute seule ; sinon ouvrir l'URL dans un navigateur. Ctrl+C pour annuler.)", "Calibración del marcador: {url}\n(La página se abre sola; si no, abre la URL en un navegador. Ctrl+C para cancelar.)"));
    if open {
        open_browser(&url);
    }
    let js = |s: &mut std::net::TcpStream, v: Value| respond(s, "200 OK", "application/json", v.to_string().as_bytes());
    let mut saved = false;
    for conn in listener.incoming() {
        let Ok(mut s) = conn else { continue };
        let Some((method, full, body)) = read_request(&mut s) else { continue };
        let path = full.split('?').next().unwrap_or("").to_string();
        match (method.as_str(), path.as_str()) {
            ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("scoreboard.html").as_bytes()),
            ("GET", "/theme.css") => respond(&mut s, "200 OK", "text/css; charset=utf-8", include_str!("theme.css").as_bytes()),
            ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
            ("GET", "/api/info") => js(&mut s, info_json(&sess, info, &existing, lang)),
            ("GET", "/frame.jpg") => {
                let single = query(&full, "mode").as_deref() == Some("single");
                let (w, h) = (sess.burst.w, sess.burst.h);
                let cache = if single { &mut sess.jpg_first } else { &mut sess.jpg_max };
                if cache.is_none() {
                    *cache = encode_jpeg(w, h, if single { &sess.burst.first } else { &sess.burst.max_rgb }).ok();
                }
                respond(&mut s, "200 OK", "image/jpeg", cache.as_deref().unwrap_or_default());
            }
            ("GET", "/board.jpg") => match &sess.rect {
                Some(r) => {
                    let src = if query(&full, "mode").as_deref() == Some("max") { &r.max } else { &r.best };
                    match encode_jpeg(r.size[0] as usize, r.size[1] as usize, src) {
                        Ok(b) => respond(&mut s, "200 OK", "image/jpeg", &b),
                        Err(e) => respond(&mut s, "500 Internal Server Error", "text/plain", e.as_bytes()),
                    }
                }
                None => respond(&mut s, "404 Not Found", "text/plain", b"no board"),
            },
            ("POST", "/api/scan") => {
                let t = serde_json::from_str::<Value>(&body).ok().and_then(|v| v["time"].as_f64()).unwrap_or(sess.time).max(0.0);
                match scan(input, info, t) {
                    Ok((b, c)) => {
                        sess = Session { time: t, burst: b, cands: c, rect: None, jpg_max: None, jpg_first: None };
                        js(&mut s, json!({"ok": true, "candidates": cands_json(&sess.cands)}));
                    }
                    Err(e) => js(&mut s, json!({"ok": false, "error": e})),
                }
            }
            ("POST", "/api/board") => {
                let r = serde_json::from_str::<Value>(&body).ok().and_then(|v| quad_from(&v["corners"])).ok_or_else(|| trs!("Ecken fehlen", "corners are missing", "coins manquants", "faltan las esquinas").to_string()).and_then(|c| rectify(&sess.burst, &c));
                match r {
                    Ok(r) => {
                        js(&mut s, json!({"ok": true, "size": r.size, "corners": r.corners}));
                        sess.rect = Some(r);
                    }
                    Err(e) => js(&mut s, json!({"ok": false, "error": e})),
                }
            }
            ("POST", "/api/propose") => match &sess.rect {
                Some(r) => {
                    let v: Vec<u8> = r.max.chunks_exact(3).map(|p| p[0].max(p[1]).max(p[2])).collect();
                    let m: serde_json::Map<String, Value> = find::propose_fields(&v, r.size[0] as usize, r.size[1] as usize).into_iter().map(|(k, f)| (k.to_string(), field_json(&f))).collect();
                    js(&mut s, json!({"ok": true, "fields": m}));
                }
                None => js(&mut s, json!({"ok": false, "error": trs!("noch keine Anzeige gewählt", "no board chosen yet", "aucun tableau choisi", "aún no se ha elegido el marcador")})),
            },
            ("POST", "/api/geometry") => {
                let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let m: serde_json::Map<String, Value> = fields_from(&v["fields"]).into_iter().map(|(k, f)| (k, json!(f.segment_rects()))).collect();
                js(&mut s, Value::Object(m));
            }
            ("POST", "/api/save") => match save_from(&body, &mut sess, info, existing.as_ref(), out) {
                Ok(()) => {
                    saved = true;
                    js(&mut s, json!({"ok": true, "path": out}));
                }
                Err(e) => js(&mut s, json!({"ok": false, "error": e})),
            },
            ("POST", "/api/quit") => {
                js(&mut s, json!({"ok": true}));
                break;
            }
            _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
        }
    }
    if !saved && existing.is_none() {
        return Err(trs!("Nicht gespeichert.", "Not saved.", "Non enregistré.", "No guardado.").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_and_reading_formats() {
        let f = |x: f64, n: usize| FieldCfg { rect: [x, 10.0, 60.0, 40.0], digits: n, gap_ratio: 0.3, slant: 0.05, colon: 0.0 };
        let c = Config {
            frame: [3840, 1080],
            board: Some(Board { corners: [[100.0, 50.0], [400.0, 55.0], [398.0, 120.0], [102.0, 118.0]], size: [1600, 360] }),
            detect: DetectCfg { channel: Channel::Max, ..DetectCfg::default() },
            left_score: f(10.0, 2),
            right_score: f(400.0, 3),
            clock: f(150.0, 4),
            period: None,
            home: HomeSide::Right,
            time: Some(812.5),
        };
        let back = from_json(&to_json(&c)).unwrap();
        assert_eq!(back, c);
        // reco-scoreboard's --data-out lines and our own events
        let r = parse_reading(&json!({"t": 12.5, "home": 10, "guest": 8, "clock": "07:41", "period": 2})).unwrap();
        assert_eq!((r.home, r.away, r.clock, r.period), (Some(10), Some(8), Some(7 * 60 + 41), Some(2)));
        assert_eq!(parse_reading(&reading_json(&r)), Some(r));
        assert!(from_json(&json!({"frame": [10, 10]})).is_err());
    }
}

// ---------------------------------------------------------------- comparison with a match report

/// A score state from a match report (play-by-play): after which event, in which period, at
/// which clock (seconds remaining).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefState {
    pub period: u32,
    pub clock: u32,
    pub home: u32,
    pub away: u32,
}

/// Score states of a match report in the `reco-minimap-reference-events-v1` format
/// (`events: [[period, "MM:SS", team, player, type, score_home, score_away], ...]`), plus the
/// cumulative score at the end of each period (`period_scores`), when given.
pub fn load_reference(path: &str) -> Result<(Vec<RefState>, Vec<(u32, u32)>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let mut states: Vec<RefState> = Vec::new();
    for e in v["events"].as_array().ok_or_else(|| format!("{path}: no events"))? {
        let a = match e.as_array() {
            Some(a) if a.len() >= 7 => a,
            _ => continue,
        };
        let clock = a[1].as_str().and_then(|s| {
            let (m, s) = s.split_once(':')?;
            Some(m.trim().parse::<u32>().ok()? * 60 + s.trim().parse::<u32>().ok()?)
        });
        let (Some(period), Some(clock), Some(home), Some(away)) = (a[0].as_u64(), clock, a[5].as_u64(), a[6].as_u64()) else { continue };
        let s = RefState { period: period as u32, clock, home: home as u32, away: away as u32 };
        if states.last().map(|l| (l.home, l.away) != (s.home, s.away)).unwrap_or(true) {
            states.push(s);
        }
    }
    let mut ends = Vec::new();
    let (mut h, mut a) = (0u32, 0u32);
    for p in v["period_scores"].as_array().into_iter().flatten() {
        if let (Some(ph), Some(pa)) = (p[0].as_u64(), p[1].as_u64()) {
            h += ph as u32;
            a += pa as u32;
            ends.push((h, a));
        }
    }
    Ok((states, ends))
}

/// Result of comparing the scoreboard course with a match report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comparison {
    /// Score states of the report that were found in the same period, with the clock difference
    /// (read − report, seconds).
    pub found: Vec<(RefState, Option<i64>)>,
    /// Score states of the report that never appeared.
    pub missing: Vec<RefState>,
    /// Score states that were read but are not in the report (only between the first and last
    /// state of the report, as the report may be partial).
    pub extra: Vec<(u32, u32, Option<u32>)>,
    /// Score at the end of each period: (read, report).
    pub period_ends: Vec<(u32, Option<(u32, u32)>, (u32, u32))>,
}

pub fn compare(tl: &crate::scoreboard::timeline::Timeline, states: &[RefState], ends: &[(u32, u32)]) -> Comparison {
    let mut c = Comparison::default();
    let read: Vec<(u32, u32, Option<u32>, Option<u32>)> = tl.changes.iter().map(|ch| (ch.score_home, ch.score_away, ch.period, ch.clock)).collect();
    for s in states {
        match read.iter().find(|r| (r.0, r.1) == (s.home, s.away)) {
            Some(r) if r.2 == Some(s.period) => c.found.push((*s, r.3.map(|k| k as i64 - s.clock as i64))),
            Some(_) | None => c.missing.push(*s),
        }
    }
    if let (Some(first), Some(last)) = (states.first(), states.last()) {
        let inside = |h: u32, a: u32| h + a > first.home + first.away && h + a <= last.home + last.away;
        for r in &read {
            if inside(r.0, r.1) && !states.iter().any(|s| (s.home, s.away) == (r.0, r.1)) {
                c.extra.push((r.0, r.1, r.2));
            }
        }
    }
    for (i, e) in ends.iter().enumerate() {
        let p = i as u32 + 1;
        let last = tl.states.iter().rev().find(|s| s.period == Some(p)).and_then(|s| Some((s.home?, s.away?)));
        c.period_ends.push((p, last, *e));
    }
    c
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    fn rd(t: f64, h: Option<u32>, a: Option<u32>) -> Reading {
        Reading { t, home: h, away: a, clock: None, period: None }
    }

    #[test]
    fn a_board_read_mostly_on_one_side_is_not_trusted() {
        let good: Vec<Reading> = (0..200).map(|i| rd(i as f64, Some(i / 20), Some(i / 30))).collect();
        assert_eq!(quality(&good)["ok"], true);
        // home missing in most readings and a flickering away score
        let bad: Vec<Reading> = (0..200).map(|i| rd(i as f64, (i % 10 == 0).then_some(7), Some(if i % 2 == 0 { 14 } else { 4 }))).collect();
        let q = quality(&bad);
        assert_eq!(q["ok"], false);
        assert!(q["home"].as_f64().unwrap() < 0.2, "{q}");
        assert_eq!(quality(&[])["ok"], false);
    }
}

