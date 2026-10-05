//! Video I/O through the `ffmpeg` / `ffprobe` command-line tools.
//!
//! Input can be anything ffmpeg can read: a file, a named pipe, or a live URL
//! (rtmp/srt/udp/rtsp ...), so a Reco panorama live feed works the same way.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::detect::{Det, Tile};
use crate::render::Canvas;

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub duration: Option<f64>,
}

pub struct Frame {
    pub w: usize,
    pub h: usize,
    /// RGB24, row-major.
    pub rgb: Vec<u8>,
}

fn parse_rate(s: &str) -> Option<f64> {
    if let Some((a, b)) = s.split_once('/') {
        let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
        if b > 0.0 { Some(a / b) } else { None }
    } else {
        s.parse().ok()
    }
}

pub fn probe(input: &str) -> Result<VideoInfo, String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate,avg_frame_rate:format=duration", "-of", "default=noprint_wrappers=1", input])
        .output()
        .map_err(|e| format!("ffprobe not found ({e}). Install ffmpeg (brew install ffmpeg)."))?;
    if !out.status.success() {
        return Err(format!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut w, mut h, mut fps, mut dur) = (0u32, 0u32, None, None);
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "width" => w = v.parse().unwrap_or(0),
                "height" => h = v.parse().unwrap_or(0),
                "avg_frame_rate" => {
                    if let Some(f) = parse_rate(v).filter(|f| *f > 0.0) {
                        fps = Some(f);
                    }
                }
                "r_frame_rate" => {
                    if fps.is_none() {
                        fps = parse_rate(v).filter(|f| *f > 0.0);
                    }
                }
                "duration" => dur = v.parse().ok(),
                _ => {}
            }
        }
    }
    if w == 0 || h == 0 {
        return Err("could not read video size from ffprobe (for live inputs pass --size WxH --fps F)".into());
    }
    Ok(VideoInfo { w, h, fps: fps.unwrap_or(25.0), duration: dur })
}

pub struct FrameReader {
    child: Child,
    stdout: ChildStdout,
    pub w: usize,
    pub h: usize,
}

impl FrameReader {
    pub fn open(input: &str, info: &VideoInfo, start: f64, duration: Option<f64>) -> Result<Self, String> {
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-nostdin"]);
        if start > 0.0 {
            c.args(["-ss", &format!("{start}")]);
        }
        c.args(["-i", input]);
        if let Some(d) = duration {
            c.args(["-t", &format!("{d}")]);
        }
        c.args(["-an", "-sn", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
        let mut child = c.stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|e| format!("ffmpeg not found ({e})"))?;
        let stdout = child.stdout.take().ok_or("no ffmpeg stdout")?;
        Ok(Self { child, stdout, w: info.w as usize, h: info.h as usize })
    }

    pub fn next(&mut self) -> Option<Frame> {
        let mut buf = vec![0u8; self.w * self.h * 3];
        self.stdout.read_exact(&mut buf).ok()?;
        Some(Frame { w: self.w, h: self.h, rgb: buf })
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decode a single frame at `t` seconds.
pub fn grab(input: &str, info: &VideoInfo, t: f64) -> Result<Frame, String> {
    FrameReader::open(input, info, t, None)?.next().ok_or_else(|| format!("no frame at {t}s"))
}

/// PNG bytes of the frame at `t` (for the calibration page).
pub fn grab_png(input: &str, t: f64) -> Result<Vec<u8>, String> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-ss", &format!("{t}"), "-i", input, "-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .map_err(|e| format!("ffmpeg not found ({e})"))?;
    if out.stdout.is_empty() {
        return Err(format!("ffmpeg produced no frame: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

pub enum Codec {
    /// H.264 mp4 (`vcodec` e.g. libx264, h264_videotoolbox).
    H264(String),
    /// ProRes 4444 with alpha (.mov) - for OBS overlays.
    ProRes4444,
}

pub struct FrameWriter {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl FrameWriter {
    pub fn open(path: &str, w: usize, h: usize, fps: f64, rgba: bool, codec: &Codec) -> Result<Self, String> {
        let mut c = Command::new("ffmpeg");
        c.args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", if rgba { "rgba" } else { "rgb24" }, "-s", &format!("{w}x{h}"), "-framerate", &format!("{fps}"), "-i", "-"]);
        match codec {
            Codec::H264(v) => {
                c.args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-c:v", v, "-pix_fmt", "yuv420p"]);
                if v == "libx264" {
                    c.args(["-crf", "18", "-preset", "veryfast"]);
                }
            }
            Codec::ProRes4444 => {
                c.args(["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le"]);
            }
        }
        c.arg(path);
        let mut child = c.stdin(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|e| format!("ffmpeg not found ({e})"))?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<(), String> {
        self.stdin.as_mut().ok_or("writer closed")?.write_all(buf).map_err(|e| format!("ffmpeg write failed: {e}"))
    }

    pub fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let st = self.child.wait().map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("ffmpeg exited with an error".into()) }
    }
}

impl Frame {
    /// Crop `tile`, resize (bilinear) to `size`x`size`, ImageNet-normalise, CHW.
    pub fn tile_tensor(&self, tile: &Tile, size: usize) -> Vec<f32> {
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let mut out = vec![0f32; 3 * size * size];
        let (tw, th) = (tile.w as f32, tile.h as f32);
        let native = tile.w as usize == size && tile.h as usize == size;
        for y in 0..size {
            for x in 0..size {
                let rgb = if native {
                    let i = ((tile.y as usize + y) * self.w + tile.x as usize + x) * 3;
                    [self.rgb[i] as f32, self.rgb[i + 1] as f32, self.rgb[i + 2] as f32]
                } else {
                    let sx = ((x as f32 + 0.5) * tw / size as f32 - 0.5).max(0.0);
                    let sy = ((y as f32 + 0.5) * th / size as f32 - 0.5).max(0.0);
                    let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                    let (x1, y1) = ((x0 + 1).min(tile.w as usize - 1), (y0 + 1).min(tile.h as usize - 1));
                    let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
                    let px = |xx: usize, yy: usize, c: usize| self.rgb[((tile.y as usize + yy) * self.w + tile.x as usize + xx) * 3 + c] as f32;
                    let mut v = [0f32; 3];
                    for c in 0..3 {
                        let top = px(x0, y0, c) * (1.0 - fx) + px(x1, y0, c) * fx;
                        let bot = px(x0, y1, c) * (1.0 - fx) + px(x1, y1, c) * fx;
                        v[c] = top * (1.0 - fy) + bot * fy;
                    }
                    v
                };
                for c in 0..3 {
                    out[c * size * size + y * size + x] = (rgb[c] / 255.0 - MEAN[c]) / STD[c];
                }
            }
        }
        out
    }

    /// Median colour of the torso region of a player box (robust against skin/background).
    pub fn torso_color(&self, d: &Det) -> Option<[u8; 3]> {
        let (fw, fh) = (self.w as f32, self.h as f32);
        let x0 = ((d.cx - d.w * 0.22) * fw).max(0.0) as usize;
        let x1 = (((d.cx + d.w * 0.22) * fw) as usize).min(self.w.saturating_sub(1));
        let top = (d.cy - d.h / 2.0) * fh;
        let y0 = (top + d.h * fh * 0.18).max(0.0) as usize;
        let y1 = ((top + d.h * fh * 0.50) as usize).min(self.h.saturating_sub(1));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let mut ch: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let (sx, sy) = (((x1 - x0) / 8).max(1), ((y1 - y0) / 8).max(1));
        let mut y = y0;
        while y <= y1 {
            let mut x = x0;
            while x <= x1 {
                let i = (y * self.w + x) * 3;
                for c in 0..3 {
                    ch[c].push(self.rgb[i + c]);
                }
                x += sx;
            }
            y += sy;
        }
        let med = |v: &mut Vec<u8>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        Some([med(&mut ch[0]), med(&mut ch[1]), med(&mut ch[2])])
    }

    pub fn rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: [u8; 3], th: i64) {
        let (w, h) = (self.w as i64, self.h as i64);
        for t in 0..th {
            for x in x0..=x1 {
                for &y in &[y0 + t, y1 - t] {
                    if x >= 0 && x < w && y >= 0 && y < h {
                        let i = ((y * w + x) * 3) as usize;
                        self.rgb[i..i + 3].copy_from_slice(&c);
                    }
                }
            }
            for y in y0..=y1 {
                for &x in &[x0 + t, x1 - t] {
                    if x >= 0 && x < w && y >= 0 && y < h {
                        let i = ((y * w + x) * 3) as usize;
                        self.rgb[i..i + 3].copy_from_slice(&c);
                    }
                }
            }
        }
    }

    pub fn blit(&mut self, c: &Canvas, ox: usize, oy: usize) {
        for y in 0..c.h {
            let fy = oy + y;
            if fy >= self.h {
                break;
            }
            for x in 0..c.w {
                let fx = ox + x;
                if fx >= self.w {
                    break;
                }
                let s = (y * c.w + x) * 4;
                let a = c.rgba[s + 3] as f32 / 255.0;
                let d = (fy * self.w + fx) * 3;
                for k in 0..3 {
                    self.rgb[d + k] = (c.rgba[s + k] as f32 * a + self.rgb[d + k] as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }

    pub fn write_png(&self, path: &str) -> Result<(), String> {
        let mut w = FrameWriterPng::open(path, self.w, self.h)?;
        w.write(&self.rgb)?;
        w.finish()
    }
}

struct FrameWriterPng {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl FrameWriterPng {
    fn open(path: &str, w: usize, h: usize) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{w}x{h}"), "-i", "-", "-frames:v", "1", path])
            .stdin(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("ffmpeg not found ({e})"))?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }
    fn write(&mut self, b: &[u8]) -> Result<(), String> {
        self.stdin.as_mut().ok_or("closed")?.write_all(b).map_err(|e| e.to_string())
    }
    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        self.child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
