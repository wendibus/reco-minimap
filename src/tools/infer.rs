//! RF-DETR inference through ONNX Runtime (`ort`, load-dynamic like reco-detect).

use std::path::{Path, PathBuf};

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::video::Frame;
use crate::detect::{decode_rfdetr, nms, tile_to_pano, BoxFormat, Class, DecodeCfg, Det, Tile};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ep {
    Cpu,
    CoreMl,
}

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub onnx: PathBuf,
    pub input_size: usize,
    pub classes: Vec<String>,
    pub conf: Option<f32>,
    pub id: String,
}

/// Accepts a Reco Trainer model folder (with `runtime.json`) or a bare `.onnx` file.
pub fn resolve_model(p: &str) -> Result<ModelInfo, String> {
    let path = Path::new(p);
    let default_classes = vec!["ball".to_string(), "player".into(), "referee".into(), "ignore".into()];
    if path.is_dir() {
        let rt = path.join("runtime.json");
        let txt = std::fs::read_to_string(&rt).map_err(|e| format!("{}: {e}", rt.display()))?;
        let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("runtime.json: {e}"))?;
        let file = v["artifact"]["file"].as_str().ok_or("runtime.json: artifact.file missing")?;
        let classes = v["classes"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(|s| s.to_string())).collect()).unwrap_or(default_classes);
        Ok(ModelInfo {
            onnx: path.join(file),
            input_size: v["inputSize"].as_u64().unwrap_or(512) as usize,
            classes,
            conf: v["confidenceThreshold"].as_f64().map(|f| f as f32),
            id: v["modelId"].as_str().unwrap_or("model").to_string(),
        })
    } else {
        Ok(ModelInfo { onnx: path.to_path_buf(), input_size: 512, classes: default_classes, conf: None, id: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() })
    }
}

/// With `load-dynamic`, ort deadlocks (instead of failing) if the runtime
/// library cannot be loaded, so check for it first (same reason reco-detect probes).
pub fn ensure_ort_runtime() -> Result<(), String> {
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        return if Path::new(&p).exists() { Ok(()) } else { Err(format!("ORT_DYLIB_PATH={p} does not exist")) };
    }
    let candidates = [
        "/opt/homebrew/lib/libonnxruntime.dylib",
        "/opt/homebrew/opt/onnxruntime/lib/libonnxruntime.dylib",
        "/usr/local/lib/libonnxruntime.dylib",
        "/usr/local/opt/onnxruntime/lib/libonnxruntime.dylib",
        "/usr/lib/libonnxruntime.so",
        "/usr/local/lib/libonnxruntime.so",
        "/usr/lib/x86_64-linux-gnu/libonnxruntime.so",
    ];
    for c in candidates {
        if Path::new(c).exists() {
            std::env::set_var("ORT_DYLIB_PATH", c);
            return Ok(());
        }
    }
    Err("ONNX Runtime library not found. macOS: `brew install onnxruntime`, then\n  export ORT_DYLIB_PATH=/opt/homebrew/lib/libonnxruntime.dylib\n(the runtime must be >= 1.24: this build uses ort rc.12 / api-24 like reco-detect)".into())
}

pub struct RfDetr {
    session: Session,
    pub info: ModelInfo,
    pub size: usize,
    pub cfg: DecodeCfg,
}

fn class_from_name(n: &str) -> Option<Class> {
    match n.to_ascii_lowercase().as_str() {
        "ball" | "basketball" => Some(Class::Ball),
        "player" | "person" => Some(Class::Player),
        "referee" | "ref" | "umpire" => Some(Class::Referee),
        _ => None,
    }
}

impl RfDetr {
    pub fn load(model: &str, ep: Ep) -> Result<Self, String> {
        let info = resolve_model(model)?;
        if !info.onnx.exists() {
            return Err(format!("model file not found: {}", info.onnx.display()));
        }
        ensure_ort_runtime()?;
        let builder = Session::builder().map_err(|e| e.to_string())?;
        #[allow(unused_mut)]
        let mut builder = builder.with_optimization_level(GraphOptimizationLevel::Level3).map_err(|e| e.to_string())?;
        if ep == Ep::CoreMl {
            #[cfg(feature = "coreml")]
            {
                builder = builder
                    .with_execution_providers([ort::ep::CoreML::default().with_compute_units(ort::ep::coreml::ComputeUnits::All).build()])
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(not(feature = "coreml"))]
            return Err("built without CoreML: rebuild with `--features coreml` or use --ep cpu".into());
        }
        let session = builder.commit_from_file(&info.onnx).map_err(|e| e.to_string())?;
        let mut cfg = DecodeCfg::basketball_v1();
        cfg.n_classes = info.classes.len();
        cfg.class_map = info.classes.iter().map(|n| class_from_name(n)).collect();
        cfg.box_format = BoxFormat::CxCyWh;
        Ok(Self { session, size: info.input_size, info, cfg })
    }

    /// One inference on a prepared CHW tensor. Returns (boxes `[nq*4]`, logits `[nq*nc]`, nq, nc).
    pub fn run_raw(&mut self, chw: &[f32]) -> Result<(Vec<f32>, Vec<f32>, usize, usize), String> {
        let sz = self.size;
        let input = TensorRef::from_array_view(([1usize, 3, sz, sz], chw)).map_err(|e| format!("tensor: {e}"))?;
        let outputs = self.session.run(ort::inputs![input]).map_err(|e| format!("ort run: {e}"))?;
        let pick = |name: &str, idx: usize| -> Result<(Vec<i64>, Vec<f32>), String> {
            let v = outputs.get(name).unwrap_or(&outputs[idx]);
            let (shape, data) = v.try_extract_tensor::<f32>().map_err(|e| format!("extract {name}: {e}"))?;
            Ok((shape.iter().copied().collect(), data.to_vec()))
        };
        let (bs, boxes) = pick("dets", 0)?;
        let (ls, logits) = pick("labels", 1)?;
        let nq = *bs.get(1).ok_or("dets shape")? as usize;
        let nc = *ls.get(2).ok_or("labels shape")? as usize;
        Ok((boxes, logits, nq, nc))
    }

    /// Detections in tile-normalized coordinates.
    pub fn detect_tile(&mut self, chw: &[f32]) -> Result<Vec<Det>, String> {
        let (boxes, logits, _nq, nc) = self.run_raw(chw)?;
        if nc != self.cfg.n_classes {
            return Err(format!("model has {nc} class logits, runtime.json lists {}", self.cfg.n_classes));
        }
        decode_rfdetr(&boxes, &logits, &self.cfg).map_err(|e| format!("{e:?}"))
    }

    /// Detections over a set of tiles, mapped to panorama coordinates and NMS-merged.
    pub fn detect(&mut self, frame: &Frame, tiles: &[Tile]) -> Result<Vec<Det>, String> {
        let mut all = Vec::new();
        for t in tiles {
            let chw = frame.tile_tensor(t, self.size);
            for d in self.detect_tile(&chw)? {
                all.push(tile_to_pano(&d, t, frame.w as u32, frame.h as u32));
            }
        }
        Ok(nms(all, 0.5))
    }
}
