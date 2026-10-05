//! Jersey-number reading: find the number on a torso crop and read it with a small text-recognition
//! network (PP-OCRv4 recognition model, Apache-2.0, ONNX). Only the *recognition* half is used: the
//! number is located with a simple colour-difference step, so no text detector is needed.
//!
//! The recognition output is kept as per-step probabilities for blank + digits; decoding (free or
//! constrained to a roster) is done by `crate::jersey`.

use std::path::{Path, PathBuf};
use std::process::Command;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::video::Frame;
use crate::jersey::DIGIT_CLASSES;
use crate::tracker::BBox;

/// Class index of digit 0..9 in `ch_PP-OCRv4_rec_infer.onnx` (used if the model carries no dictionary).
const FALLBACK_DIGIT_CLASSES: [usize; 10] = [26, 93, 25, 94, 632, 631, 933, 29, 27, 1109];

pub fn default_model_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".cache/reco-minimap/ocr/rec.onnx")
}

pub struct NumberReader {
    session: Session,
    digit_cls: [usize; 10],
}

/// RGB pixels of a located number (or of the whole torso crop if nothing stood out).
pub struct Crop {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

pub struct Reading {
    /// `steps` x `DIGIT_CLASSES`, row-major.
    pub probs: Vec<f32>,
    pub steps: usize,
}

impl NumberReader {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("OCR-Modell nicht gefunden: {} (herunterladen mit `reco-minimap fetch-ocr`)", path.display()));
        }
        super::infer::ensure_ort_runtime()?;
        let session = Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?
            .commit_from_file(path)
            .map_err(|e| e.to_string())?;
        let mut digit_cls = FALLBACK_DIGIT_CLASSES;
        if let Some(chars) = session.metadata().ok().and_then(|m| m.custom("character")) {
            let lines: Vec<&str> = chars.split('\n').collect();
            for d in 0..10u8 {
                let ch = (b'0' + d) as char;
                if let Some(i) = lines.iter().position(|l| l.chars().count() == 1 && l.starts_with(ch)) {
                    digit_cls[d as usize] = i + 1; // class 0 is the CTC blank
                }
            }
        }
        Ok(Self { session, digit_cls })
    }

    /// Run the recognition on a prepared crop.
    pub fn read(&mut self, crop: &Crop) -> Result<Reading, String> {
        if crop.w < 2 || crop.h < 2 {
            return Err("Ausschnitt zu klein".into());
        }
        let nh = 48usize;
        let nw = (((nh as f32) * crop.w as f32 / crop.h as f32).ceil() as usize).clamp(16, 320);
        let mut data = vec![0f32; 3 * nh * nw];
        // bilinear resize; PP-OCR expects BGR order, (x/255 - 0.5) / 0.5
        let sample = |x: f32, y: f32, c: usize| -> f32 {
            let (x, y) = (x.clamp(0.0, crop.w as f32 - 1.0), y.clamp(0.0, crop.h as f32 - 1.0));
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(crop.w - 1), (y0 + 1).min(crop.h - 1));
            let (fx, fy) = (x - x0 as f32, y - y0 as f32);
            let px = |xx: usize, yy: usize| crop.rgb[(yy * crop.w + xx) * 3 + c] as f32;
            (px(x0, y0) * (1.0 - fx) + px(x1, y0) * fx) * (1.0 - fy) + (px(x0, y1) * (1.0 - fx) + px(x1, y1) * fx) * fy
        };
        for y in 0..nh {
            for x in 0..nw {
                let (sx, sy) = ((x as f32 + 0.5) * crop.w as f32 / nw as f32 - 0.5, (y as f32 + 0.5) * crop.h as f32 / nh as f32 - 0.5);
                for (plane, c) in [2usize, 1, 0].iter().enumerate() {
                    data[plane * nh * nw + y * nw + x] = (sample(sx, sy, *c) / 255.0 - 0.5) / 0.5;
                }
            }
        }
        let input = TensorRef::from_array_view(([1usize, 3, nh, nw], data.as_slice())).map_err(|e| format!("tensor: {e}"))?;
        let outputs = self.session.run(ort::inputs![input]).map_err(|e| format!("ort run: {e}"))?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>().map_err(|e| format!("extract: {e}"))?;
        let (t, c) = (shape[1] as usize, shape[2] as usize);
        let mut probs = vec![0f32; t * DIGIT_CLASSES];
        for step in 0..t {
            probs[step * DIGIT_CLASSES] = out[step * c];
            for d in 0..10 {
                probs[step * DIGIT_CLASSES + 1 + d] = out[step * c + self.digit_cls[d]];
            }
        }
        Ok(Reading { probs, steps: t })
    }
}

/// Torso region of a player box (normalized bbox) in pixels of `frame`: the upper-middle part where a number sits.
pub fn torso_rect(frame: &Frame, b: &BBox) -> (i64, i64, i64, i64) {
    let (fw, fh) = (frame.w as f32, frame.h as f32);
    let top = (b.cy - b.h / 2.0) * fh;
    let (x0, x1) = ((b.cx - b.w * 0.34) * fw, (b.cx + b.w * 0.34) * fw);
    let (y0, y1) = (top + b.h * fh * 0.12, top + b.h * fh * 0.52);
    (x0.floor() as i64, y0.floor() as i64, x1.ceil() as i64, y1.ceil() as i64)
}

/// Find the number on a torso crop: the region that differs most from the shirt colour.
/// Falls back to the whole crop if nothing stands out.
pub fn localize(w: usize, h: usize, rgb: &[u8]) -> Crop {
    let whole = || Crop { w, h, rgb: rgb.to_vec() };
    if w < 6 || h < 6 {
        return whole();
    }
    // median colour per channel (subsampled)
    let step = ((w * h) / 4000).max(1);
    let mut ch: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for i in (0..w * h).step_by(step) {
        for c in 0..3 {
            ch[c].push(rgb[i * 3 + c]);
        }
    }
    let med: Vec<f32> = ch.iter_mut().map(|v| { v.sort_unstable(); v[v.len() / 2] as f32 }).collect();
    let dist = |i: usize| -> f32 { ((0..3).map(|c| (rgb[i * 3 + c] as f32 - med[c]).powi(2)).sum::<f32>()).sqrt() };
    let maxd = (0..w * h).map(dist).fold(0.0f32, f32::max);
    let thr = (0.35 * maxd).max(40.0);
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0usize, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if dist(y * w + x) > thr {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
                n += 1;
            }
        }
    }
    if n < 4 || x1 <= x0 || y1 <= y0 {
        return whole();
    }
    let (px, py) = (((x1 - x0) as f32 * 0.15).max(2.0) as usize, ((y1 - y0) as f32 * 0.25).max(2.0) as usize);
    let (cx0, cy0, cx1, cy1) = (x0.saturating_sub(px), y0.saturating_sub(py), (x1 + px).min(w), (y1 + py).min(h));
    let mut out = Vec::with_capacity((cx1 - cx0) * (cy1 - cy0) * 3);
    for y in cy0..cy1 {
        out.extend_from_slice(&rgb[(y * w + cx0) * 3..(y * w + cx1) * 3]);
    }
    Crop { w: cx1 - cx0, h: cy1 - cy0, rgb: out }
}

/// Download the recognition model (Apache-2.0, from the `rapidocr-onnxruntime` wheel on PyPI) with curl + unzip.
pub fn fetch_model(dest: &Path) -> Result<PathBuf, String> {
    let dir = dest.parent().ok_or("Zielordner")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    eprintln!("Suche das aktuelle OCR-Paket auf PyPI …");
    let meta = Command::new("curl").args(["-fsSL", "https://pypi.org/pypi/rapidocr-onnxruntime/json"]).output().map_err(|e| format!("curl nicht gefunden: {e}"))?;
    if !meta.status.success() {
        return Err("PyPI nicht erreichbar (curl fehlgeschlagen)".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&meta.stdout).map_err(|e| format!("PyPI-Antwort: {e}"))?;
    let url = v["urls"].as_array().and_then(|a| a.iter().find(|u| u["filename"].as_str().map(|f| f.ends_with(".whl")).unwrap_or(false))).and_then(|u| u["url"].as_str()).ok_or("kein Wheel auf PyPI gefunden")?.to_string();
    let wheel = dir.join("rapidocr.whl");
    eprintln!("Lade {url} (ca. 15 MB) …");
    let st = Command::new("curl").args(["-fL", "--progress-bar", "-o"]).arg(&wheel).arg(&url).status().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("Download fehlgeschlagen".into());
    }
    let st = Command::new("unzip").args(["-o", "-j"]).arg(&wheel).arg("*ch_PP-OCRv4_rec_infer.onnx").arg("-d").arg(dir).stdout(std::process::Stdio::null()).status().map_err(|e| format!("unzip nicht gefunden: {e}"))?;
    let _ = std::fs::remove_file(&wheel);
    let extracted = dir.join("ch_PP-OCRv4_rec_infer.onnx");
    if !st.success() || !extracted.exists() {
        return Err("Das Erkennungsmodell wurde im Paket nicht gefunden".into());
    }
    std::fs::rename(&extracted, dest).map_err(|e| e.to_string())?;
    let _ = std::fs::write(dir.join("LICENSE.txt"), "PP-OCRv4 text recognition model (PaddleOCR), ONNX export from the RapidOCR project.\nLicense: Apache-2.0 - https://github.com/RapidAI/RapidOCR , https://github.com/PaddlePaddle/PaddleOCR\n");
    Ok(dest.to_path_buf())
}
