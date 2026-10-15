//! RF-DETR inference through ONNX Runtime (`ort`, load-dynamic like reco-detect).

use crate::{tr, trs};
use std::path::{Path, PathBuf};

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use super::video::Frame;
use crate::detect::{decode_rfdetr, merge_tiles, BoxFormat, Class, DecodeCfg, Det, Tile};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ep {
    Cpu,
    CoreMl,
    /// NVIDIA GPU via CUDA (Linux, Windows). Needs an ONNX Runtime GPU build + CUDA 12 + cuDNN 9.
    Cuda,
    /// NVIDIA GPU via TensorRT (Linux, Windows): fastest, but the first start builds an engine (minutes, then cached).
    TensorRt,
    /// Any DirectX-12 GPU on Windows (NVIDIA, AMD, Intel) - needs the DirectML ONNX Runtime build.
    DirectMl,
    /// Best available provider for this platform, falling back to the CPU.
    Auto,
}

impl Ep {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cpu" => Some(Ep::Cpu),
            "coreml" => Some(Ep::CoreMl),
            "cuda" => Some(Ep::Cuda),
            "tensorrt" | "trt" => Some(Ep::TensorRt),
            "directml" | "dml" => Some(Ep::DirectMl),
            "auto" => Some(Ep::Auto),
            _ => None,
        }
    }
}

/// GPU adapter index for CUDA / TensorRT / DirectML (`--device N`, passed on as `RECO_MINIMAP_DEVICE`).
pub fn device_id() -> i32 {
    std::env::var("RECO_MINIMAP_DEVICE").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Per-user cache folder `<cache>/reco-minimap/<sub>`: `%LOCALAPPDATA%` on Windows, `$XDG_CACHE_HOME` or `~/.cache` elsewhere.
pub fn cache_dir(sub: &str) -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(std::env::temp_dir).join("reco-minimap").join(sub)
}

/// CoreML compute units (`--units`): which hardware CoreML may use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Units {
    All,
    NeuralEngine,
    Gpu,
    Cpu,
}

impl Units {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "all" => Some(Units::All),
            "ane" | "neuralengine" => Some(Units::NeuralEngine),
            "gpu" => Some(Units::Gpu),
            "cpu" => Some(Units::Cpu),
            _ => None,
        }
    }
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
        let file = v["artifact"]["file"].as_str().ok_or(trs!("runtime.json: artifact.file fehlt", "runtime.json: artifact.file missing", "runtime.json : artifact.file manquant", "runtime.json: falta artifact.file"))?;
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

/// File name of the ONNX Runtime shared library on this platform.
pub fn ort_lib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// Places where an ONNX Runtime usually lives (also next to the program itself).
fn ort_candidates() -> Vec<PathBuf> {
    let name = ort_lib_name();
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        v.push(d.join(name));
    }
    if cfg!(target_os = "macos") {
        for d in ["/opt/homebrew/lib", "/opt/homebrew/opt/onnxruntime/lib", "/usr/local/lib", "/usr/local/opt/onnxruntime/lib"] {
            v.push(Path::new(d).join(name));
        }
    } else if cfg!(target_os = "windows") {
        for var in ["LOCALAPPDATA", "USERPROFILE", "ProgramFiles"] {
            if let Some(b) = std::env::var_os(var) {
                let b = PathBuf::from(b);
                v.push(b.join("reco-minimap").join(name));
                v.push(b.join("onnxruntime").join("lib").join(name));
                v.push(b.join("onnxruntime").join(name));
            }
        }
        v.push(Path::new("C:\\onnxruntime\\lib").join(name));
    } else {
        for d in ["/usr/local/lib", "/usr/lib", "/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/opt/onnxruntime/lib"] {
            v.push(Path::new(d).join(name));
        }
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".local/lib/onnxruntime").join(name));
        }
    }
    v
}

/// With `load-dynamic`, ort deadlocks (instead of failing) if the runtime
/// library cannot be loaded, so check for it first (same reason reco-detect probes).
pub fn ensure_ort_runtime() -> Result<(), String> {
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        return if Path::new(&p).exists() { Ok(()) } else { Err(tr!("ORT_DYLIB_PATH={p} existiert nicht", "ORT_DYLIB_PATH={p} does not exist", "ORT_DYLIB_PATH={p} n'existe pas", "ORT_DYLIB_PATH={p} no existe")) };
    }
    for c in ort_candidates() {
        if c.exists() {
            std::env::set_var("ORT_DYLIB_PATH", &c);
            return Ok(());
        }
    }
    let hint: &str = if cfg!(target_os = "macos") {
        "macOS: `brew install onnxruntime`; export ORT_DYLIB_PATH=/opt/homebrew/lib/libonnxruntime.dylib"
    } else if cfg!(target_os = "windows") {
        "Windows: https://github.com/microsoft/onnxruntime/releases -> onnxruntime-win-x64(-gpu)-<version>.zip (DirectML: NuGet Microsoft.ML.OnnxRuntime.DirectML); setx ORT_DYLIB_PATH \"C:\\onnxruntime\\lib\\onnxruntime.dll\" (or copy onnxruntime.dll next to reco-minimap.exe)"
    } else {
        "Linux: `pip install onnxruntime-gpu` (CUDA) or `pip install onnxruntime`; export ORT_DYLIB_PATH=<site-packages>/onnxruntime/capi/libonnxruntime.so.<version>"
    };
    Err(trs!(
        "ONNX-Runtime-Bibliothek nicht gefunden. ",
        "ONNX Runtime library not found. ",
        "Bibliothèque ONNX Runtime introuvable. ",
        "No se encuentra la biblioteca ONNX Runtime. "
    )
    .to_string()
        + hint
        + &tr!("\n(die Runtime muss >= 1.24 sein: dieser Build nutzt ort rc.12 / api-24 wie reco-detect)", "\n(the runtime must be >= 1.24: this build uses ort rc.12 / api-24 like reco-detect)", "\n(la version doit être >= 1.24 : ce build utilise ort rc.12 / api-24 comme reco-detect)", "\n(la versión debe ser >= 1.24: esta compilación usa ort rc.12 / api-24 como reco-detect)"))
}

pub struct RfDetr {
    sessions: Vec<Session>,
    /// Accumulated time in tile preprocessing / model inference (for the run summary).
    pub t_prep: std::time::Duration,
    pub t_infer: std::time::Duration,
    pub info: ModelInfo,
    pub size: usize,
    pub cfg: DecodeCfg,
}

fn class_from_name(n: &str) -> Option<Class> {
    match n.to_ascii_lowercase().as_str() {
        "ball" | "basketball" | "puck" | "shuttlecock" => Some(Class::Ball),
        "player" | "person" | "goalkeeper" | "keeper" | "skater" => Some(Class::Player),
        "referee" | "ref" | "umpire" | "linesman" => Some(Class::Referee),
        _ => None,
    }
}

impl RfDetr {
    pub fn load(model: &str, ep: Ep, units: Units, workers: usize) -> Result<Self, String> {
        let info = resolve_model(model)?;
        if !info.onnx.exists() {
            return Err(tr!("Modelldatei nicht gefunden: {}", "model file not found: {}", "fichier du modèle introuvable : {}", "no se encuentra el archivo del modelo: {}", info.onnx.display()));
        }
        ensure_ort_runtime()?;
        let workers = workers.clamp(1, 8);
        let mut sessions = Vec::new();
        for i in 0..workers {
            sessions.push(build_session(&info, ep, units, i == 0)?);
        }
        let mut cfg = DecodeCfg::basketball_v1();
        cfg.n_classes = info.classes.len();
        cfg.class_map = info.classes.iter().map(|n| class_from_name(n)).collect();
        cfg.box_format = BoxFormat::CxCyWh;
        Ok(Self { sessions, t_prep: Default::default(), t_infer: Default::default(), size: info.input_size, info, cfg })
    }

    pub fn workers(&self) -> usize {
        self.sessions.len()
    }

    /// One inference on a prepared CHW tensor. Returns (boxes `[nq*4]`, logits `[nq*nc]`, nq, nc).
    pub fn run_raw(&mut self, chw: &[f32]) -> Result<(Vec<f32>, Vec<f32>, usize, usize), String> {
        let size = self.size;
        run_session(&mut self.sessions[0], size, chw)
    }

    /// Detections in tile-normalized coordinates.
    pub fn detect_tile(&mut self, chw: &[f32]) -> Result<Vec<Det>, String> {
        let (boxes, logits, _nq, nc) = self.run_raw(chw)?;
        if nc != self.cfg.n_classes {
            return Err(tr!("Modell hat {nc} Klassen-Logits, runtime.json nennt {}", "model has {nc} class logits, runtime.json lists {}", "le modèle a {nc} logits de classe, runtime.json en indique {}", "el modelo tiene {nc} logits de clase, runtime.json indica {}", self.cfg.n_classes));
        }
        decode_rfdetr(&boxes, &logits, &self.cfg).map_err(|e| format!("{e:?}"))
    }

    /// Detections over a set of tiles, mapped to panorama coordinates and NMS-merged.
    /// Tile preprocessing runs on all cores; inference is spread over the worker sessions
    /// (each model instance has batch size 1).
    pub fn detect(&mut self, frame: &Frame, tiles: &[Tile]) -> Result<Vec<Det>, String> {
        let size = self.size;
        let t0 = std::time::Instant::now();
        let tensors: Vec<Vec<f32>> = std::thread::scope(|sc| {
            let hs: Vec<_> = tiles.iter().map(|t| sc.spawn(move || frame.tile_tensor(t, size))).collect();
            hs.into_iter().map(|h| h.join().expect("tile thread")).collect()
        });
        self.t_prep += t0.elapsed();

        let t1 = std::time::Instant::now();
        let n = self.sessions.len().min(tiles.len()).max(1);
        let cfg = &self.cfg;
        let tensors_ref = &tensors;
        let results: Vec<Result<Vec<(usize, Vec<Det>)>, String>> = std::thread::scope(|sc| {
            let hs: Vec<_> = self
                .sessions
                .iter_mut()
                .take(n)
                .enumerate()
                .map(|(w, sess)| {
                    sc.spawn(move || {
                        let mut out = Vec::new();
                        let mut i = w;
                        while i < tiles.len() {
                            let (b, l, _nq, nc) = run_session(sess, size, &tensors_ref[i])?;
                            if nc != cfg.n_classes {
                                return Err(tr!("Modell hat {nc} Klassen-Logits, runtime.json nennt {}", "model has {nc} class logits, runtime.json lists {}", "le modèle a {nc} logits de classe, runtime.json en indique {}", "el modelo tiene {nc} logits de clase, runtime.json indica {}", cfg.n_classes));
                            }
                            out.push((i, decode_rfdetr(&b, &l, cfg).map_err(|e| format!("{e:?}"))?));
                            i += n;
                        }
                        Ok(out)
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().expect("inference thread")).collect()
        });
        self.t_infer += t1.elapsed();

        // Worker completion/grouping must not change tie-breaking in NMS.
        // Restore the original tile order before merging detections.
        let mut ordered = Vec::new();
        for r in results {
            ordered.extend(r?);
        }
        ordered.sort_by_key(|(i, _)| *i);
        let mut per_tile = Vec::new();
        for (i, dets) in ordered {
            per_tile.push((tiles[i], dets));
        }
        Ok(merge_tiles(&per_tile, frame.w as u32, frame.h as u32, 0.5))
    }
}

/// Register one GPU provider. `Ok(true)` = active, `Ok(false)` = this ONNX Runtime build lacks it.
fn try_provider(builder: &mut ort::session::builder::SessionBuilder, ep: Ep, announce: bool) -> Result<bool, String> {
    use ort::ep::ExecutionProvider;
    let dev = device_id();
    let (name, avail, dispatch) = match ep {
        Ep::Cuda => {
            let p = ort::ep::CUDA::default().with_device_id(dev);
            ("CUDA", p.is_available().map_err(|e| e.to_string())?, p.build().error_on_failure())
        }
        Ep::TensorRt => {
            let cache = cache_dir("tensorrt");
            let _ = std::fs::create_dir_all(&cache);
            let p = ort::ep::TensorRT::default()
                .with_device_id(dev)
                .with_fp16(true)
                .with_engine_cache(true)
                .with_engine_cache_path(cache.to_string_lossy())
                .with_timing_cache(true)
                .with_timing_cache_path(cache.to_string_lossy());
            ("TensorRT", p.is_available().map_err(|e| e.to_string())?, p.build().error_on_failure())
        }
        Ep::DirectMl => {
            let p = ort::ep::DirectML::default().with_device_id(dev);
            ("DirectML", p.is_available().map_err(|e| e.to_string())?, p.build().error_on_failure())
        }
        _ => return Ok(false),
    };
    if !avail {
        return Ok(false);
    }
    let b = std::mem::replace(builder, Session::builder().map_err(|e| e.to_string())?);
    *builder = b.with_execution_providers([dispatch]).map_err(|e| {
        tr!(
            "{name} konnte nicht aktiviert werden: {e}\n(CUDA 12 und cuDNN 9 müssen im Suchpfad liegen: Linux LD_LIBRARY_PATH, Windows PATH)",
            "{name} could not be enabled: {e}\n(CUDA 12 and cuDNN 9 must be on the search path: Linux LD_LIBRARY_PATH, Windows PATH)",
            "{name} n'a pas pu être activé : {e}\n(CUDA 12 et cuDNN 9 doivent être dans le chemin de recherche : Linux LD_LIBRARY_PATH, Windows PATH)",
            "No se pudo activar {name}: {e}\n(CUDA 12 y cuDNN 9 deben estar en la ruta de búsqueda: Linux LD_LIBRARY_PATH, Windows PATH)"
        )
    })?;
    if announce {
        eprintln!("{}", tr!("{name}-Provider: aktiv (Gerät {dev})", "{name} provider: active (device {dev})", "Fournisseur {name} : actif (appareil {dev})", "Proveedor {name}: activo (dispositivo {dev})"));
    }
    Ok(true)
}

fn unavailable(ep: Ep) -> String {
    let n = match ep {
        Ep::Cuda => "CUDA",
        Ep::TensorRt => "TensorRT",
        _ => "DirectML",
    };
    let how: &str = match ep {
        Ep::DirectMl => "Windows: Microsoft.ML.OnnxRuntime.DirectML (NuGet), ORT_DYLIB_PATH -> its onnxruntime.dll",
        Ep::TensorRt => "CUDA setup + TensorRT 10 (libnvinfer / nvinfer_10.dll) on the search path",
        _ => "Linux: pip install onnxruntime-gpu, ORT_DYLIB_PATH -> its libonnxruntime.so*; Windows: onnxruntime-win-x64-gpu-<version>.zip; + CUDA 12.x, cuDNN 9",
    };
    tr!(
        "{n}-Provider ist in dieser ONNX-Runtime NICHT enthalten. {how}. Oder --ep auto / --ep cpu.",
        "The {n} provider is NOT included in this ONNX Runtime. {how}. Or use --ep auto / --ep cpu.",
        "Le fournisseur {n} n'est PAS inclus dans cette ONNX Runtime. {how}. Ou utiliser --ep auto / --ep cpu.",
        "El proveedor {n} NO está incluido en esta ONNX Runtime. {how}. O usa --ep auto / --ep cpu."
    )
}

#[allow(unused_variables)]
fn build_session(info: &ModelInfo, ep: Ep, units: Units, announce: bool) -> Result<Session, String> {
    let builder = Session::builder().map_err(|e| e.to_string())?;
    #[allow(unused_mut)]
    let mut builder = builder.with_optimization_level(GraphOptimizationLevel::Level3).map_err(|e| e.to_string())?;
    let mut ep = ep;
    if matches!(ep, Ep::Cuda | Ep::TensorRt | Ep::DirectMl) && !try_provider(&mut builder, ep, announce)? {
        return Err(unavailable(ep));
    }
    if ep == Ep::Auto {
        // Windows/Linux: CUDA, then DirectML (Windows); macOS: CoreML if built in; otherwise the CPU - and say which.
        let order: &[Ep] = if cfg!(target_os = "windows") { &[Ep::Cuda, Ep::DirectMl] } else if cfg!(target_os = "macos") { &[] } else { &[Ep::Cuda] };
        let mut chosen = false;
        for &e in order {
            if try_provider(&mut builder, e, announce).unwrap_or(false) {
                chosen = true;
                break;
            }
        }
        if !chosen && cfg!(target_os = "macos") && cfg!(feature = "coreml") {
            ep = Ep::CoreMl;
        } else if !chosen && announce {
            eprintln!("{}", trs!("Kein GPU-Provider verfügbar - es läuft auf der CPU.", "No GPU provider available - running on the CPU.", "Aucun fournisseur GPU disponible - exécution sur le CPU.", "Ningún proveedor de GPU disponible: se usa la CPU."));
        }
    }
    if ep == Ep::CoreMl {
        #[cfg(feature = "coreml")]
        {
            use ort::ep::ExecutionProvider;
            let cu = match units {
                Units::All => ort::ep::coreml::ComputeUnits::All,
                Units::NeuralEngine => ort::ep::coreml::ComputeUnits::CPUAndNeuralEngine,
                Units::Gpu => ort::ep::coreml::ComputeUnits::CPUAndGPU,
                Units::Cpu => ort::ep::coreml::ComputeUnits::CPUOnly,
            };
            let cache = cache_dir("coreml").to_string_lossy().into_owned();
            let _ = std::fs::create_dir_all(&cache);
            let coreml = ort::ep::CoreML::default()
                .with_compute_units(cu)
                .with_model_format(ort::ep::coreml::ModelFormat::MLProgram)
                .with_specialization_strategy(ort::ep::coreml::SpecializationStrategy::FastPrediction)
                .with_low_precision_accumulation_on_gpu(true)
                .with_model_cache_dir(cache)
                .with_static_input_shapes(true);
            match coreml.is_available() {
                Ok(true) => {
                    if announce {
                        eprintln!("{}", tr!("CoreML-Provider: verfügbar (MLProgram, Einheiten: {units:?})", "CoreML provider: available (MLProgram, units: {units:?})", "Fournisseur CoreML : disponible (MLProgram, unités : {units:?})", "Proveedor CoreML: disponible (MLProgram, unidades: {units:?})"));
                    }
                }
                Ok(false) => return Err(trs!("CoreML-Provider ist in dieser ONNX-Runtime NICHT enthalten. Nutze eine ONNX Runtime mit CoreML (pip-Paket `onnxruntime` für macOS: ORT_DYLIB_PATH auf dessen libonnxruntime.*.dylib) oder --ep cpu.", "The CoreML provider is NOT included in this ONNX Runtime. Use an ONNX Runtime with CoreML (pip package `onnxruntime` for macOS: point ORT_DYLIB_PATH to its libonnxruntime.*.dylib) or --ep cpu.", "Le fournisseur CoreML n'est PAS inclus dans cette ONNX Runtime. Utiliser une ONNX Runtime avec CoreML (paquet pip `onnxruntime` pour macOS : faire pointer ORT_DYLIB_PATH sur son libonnxruntime.*.dylib) ou --ep cpu.", "El proveedor CoreML NO está incluido en esta ONNX Runtime. Usa una ONNX Runtime con CoreML (paquete pip `onnxruntime` para macOS: apunta ORT_DYLIB_PATH a su libonnxruntime.*.dylib) o --ep cpu.").into()),
                Err(e) => return Err(tr!("CoreML-Prüfung fehlgeschlagen: {e}", "CoreML check failed: {e}", "La vérification de CoreML a échoué : {e}", "La comprobación de CoreML ha fallado: {e}")),
            }
            // error_on_failure: do not silently fall back to the CPU
            builder = builder.with_execution_providers([coreml.build().error_on_failure()]).map_err(|e| tr!("CoreML konnte nicht aktiviert werden: {e}", "CoreML could not be enabled: {e}", "CoreML n'a pas pu être activé : {e}", "No se pudo activar CoreML: {e}"))?;
        }
        #[cfg(not(feature = "coreml"))]
        return Err(trs!("ohne CoreML gebaut: mit `--features tools,coreml` neu bauen oder --ep cpu verwenden", "built without CoreML: rebuild with `--features tools,coreml` or use --ep cpu", "compilé sans CoreML : recompiler avec `--features tools,coreml` ou utiliser --ep cpu", "compilado sin CoreML: recompila con `--features tools,coreml` o usa --ep cpu").into());
    }
    builder.commit_from_file(&info.onnx).map_err(|e| e.to_string())
}

fn run_session(session: &mut Session, sz: usize, chw: &[f32]) -> Result<(Vec<f32>, Vec<f32>, usize, usize), String> {
    let input = TensorRef::from_array_view(([1usize, 3, sz, sz], chw)).map_err(|e| format!("tensor: {e}"))?;
    let outputs = session.run(ort::inputs![input]).map_err(|e| format!("ort run: {e}"))?;
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
