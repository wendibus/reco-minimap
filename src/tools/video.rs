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
    pub fn open(input: &str, info: &VideoInfo, start: f64, duration: Option<f64>, every: u32) -> Result<Self, String> {
        Self::open_with(input, info, start, duration, every, false, false, &[])
    }

    /// `live`: low-latency input flags. `realtime`: read a file at its native speed (simulates a live feed).
    pub fn open_with(input: &str, info: &VideoInfo, start: f64, duration: Option<f64>, every: u32, live: bool, realtime: bool, in_args: &[String]) -> Result<Self, String> {
        let mut c = Command::new("ffmpeg");
        c.args(["-v", "error", "-nostdin"]);
        if live {
            // nobuffer/low_delay break following a growing Matroska file (-follow 1), so skip them there
            if !in_args.iter().any(|a| a == "-follow") {
                c.args(["-fflags", "nobuffer", "-flags", "low_delay"]);
            }
            c.args(["-probesize", "500000", "-analyzeduration", "1000000"]);
        }
        if realtime {
            c.arg("-re");
        }
        c.args(in_args); // extra ffmpeg input options, e.g. -follow 1  or  -f avfoundation -framerate 25
        if start > 0.0 {
            c.args(["-ss", &format!("{start}")]);
        }
        c.args(["-i", input]);
        if let Some(d) = duration {
            c.args(["-t", &format!("{d}")]);
        }
        c.args(["-an", "-sn"]);
        if every > 1 {
            // only every Nth frame leaves ffmpeg: less pipe traffic and fewer buffers to copy
            c.args(["-vf", &format!("select=not(mod(n\\,{every}))"), "-fps_mode", "vfr"]);
        }
        c.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
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
    FrameReader::open(input, info, t, None, 1)?.next().ok_or_else(|| format!("no frame at {t}s"))
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
    /// Crop `tile`, letterbox it (aspect preserved, centered, padded with the
    /// mean colour) into `size`x`size`, ImageNet-normalise, CHW. Downscaling
    /// averages the source footprint (box filter) to avoid aliasing.
    pub fn tile_tensor(&self, tile: &Tile, size: usize) -> Vec<f32> {
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let mut out = vec![0f32; 3 * size * size]; // 0.0 == mean colour after normalisation
        let (tw, th) = (tile.w as usize, tile.h as usize);
        let s = size as f32 / tw.max(th) as f32; // model px per source px
        let (cw, ch) = ((tw as f32 * s).round() as usize, (th as f32 * s).round() as usize);
        let (padx, pady) = ((size - cw.min(size)) / 2, (size - ch.min(size)) / 2);
        let n = ((1.0 / s).ceil() as usize).clamp(1, 4); // samples per axis inside the footprint
        let (x0, y0) = (tile.x as usize, tile.y as usize);
        let native = (s - 1.0).abs() < 1e-6;
        for y in 0..ch.min(size) {
            for x in 0..cw.min(size) {
                let mut acc = [0f32; 3];
                if native {
                    let i = ((y0 + y) * self.w + x0 + x) * 3;
                    acc = [self.rgb[i] as f32, self.rgb[i + 1] as f32, self.rgb[i + 2] as f32];
                } else {
                    let (fx0, fy0) = (x as f32 / s, y as f32 / s);
                    let (fw, fh) = (1.0 / s, 1.0 / s);
                    let mut cnt = 0f32;
                    for j in 0..n {
                        for i in 0..n {
                            let sx = ((fx0 + fw * (i as f32 + 0.5) / n as f32) as usize).min(tw - 1);
                            let sy = ((fy0 + fh * (j as f32 + 0.5) / n as f32) as usize).min(th - 1);
                            let k = ((y0 + sy) * self.w + x0 + sx) * 3;
                            for c in 0..3 {
                                acc[c] += self.rgb[k + c] as f32;
                            }
                            cnt += 1.0;
                        }
                    }
                    for c in 0..3 {
                        acc[c] /= cnt;
                    }
                }
                let (ox, oy) = (x + padx, y + pady);
                for c in 0..3 {
                    out[c * size * size + oy * size + ox] = (acc[c] / 255.0 - MEAN[c]) / STD[c];
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

    /// Copy a pixel rectangle (clamped to the frame) as RGB: (width, height, data).
    pub fn crop_rgb(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> (usize, usize, Vec<u8>) {
        let (x0, y0) = (x0.clamp(0, self.w as i64) as usize, y0.clamp(0, self.h as i64) as usize);
        let (x1, y1) = (x1.clamp(0, self.w as i64) as usize, y1.clamp(0, self.h as i64) as usize);
        if x1 <= x0 || y1 <= y0 {
            return (0, 0, Vec::new());
        }
        let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
        for y in y0..y1 {
            out.extend_from_slice(&self.rgb[(y * self.w + x0) * 3..(y * self.w + x1) * 3]);
        }
        (x1 - x0, y1 - y0, out)
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
