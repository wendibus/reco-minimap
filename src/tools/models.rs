//! Import and inspection of models exported by Reco Trainer.
//!
//! A Reco Trainer export is a folder with `runtime.json` (contract, classes, input size,
//! `artifact.file`), `model-card.json` (sport, SHA-256 of the weights) and the `.onnx` file.

use crate::{tr, trs};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sha {
    Ok,
    Bad,
    None,
}

#[derive(Clone, Debug)]
pub struct ModelMeta {
    pub id: String,
    pub dir: PathBuf,
    pub onnx: PathBuf,
    pub sport: Option<String>,
    pub classes: Vec<String>,
    pub input_size: usize,
    pub confidence: Option<f32>,
    pub contract: Option<String>,
    pub sha: Sha,
    pub size_mb: f32,
}

impl ModelMeta {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "dir": self.dir.to_string_lossy(), "onnx": self.onnx.to_string_lossy(), "sport": self.sport,
            "classes": self.classes, "input_size": self.input_size, "confidence": self.confidence, "contract": self.contract,
            "sha": match self.sha { Sha::Ok => "ok", Sha::Bad => "bad", Sha::None => "none" }, "size_mb": (self.size_mb as f64 * 10.0).round() / 10.0,
        })
    }
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

pub fn sha256_hex(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn default_classes() -> Vec<String> {
    ["ball", "player", "referee", "ignore"].iter().map(|s| s.to_string()).collect()
}

/// Inspect a Reco Trainer model folder, or a bare `.onnx` file.
pub fn inspect(path: &Path) -> Result<ModelMeta, String> {
    if path.as_os_str().is_empty() {
        return Err(trs!("Kein Pfad angegeben", "No path given", "Aucun chemin indiqué", "No se ha indicado ninguna ruta").into());
    }
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) != Some("onnx") {
            return Err(trs!("Keine .onnx-Datei", "Not an .onnx file", "Ce n'est pas un fichier .onnx", "No es un archivo .onnx").into());
        }
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f32 / 1e6;
        return Ok(ModelMeta { id: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(), onnx: path.to_path_buf(), sport: None, classes: default_classes(), input_size: 512, confidence: None, contract: None, sha: Sha::None, size_mb: size });
    }
    if !path.is_dir() {
        return Err(tr!("{}: nicht gefunden", "{}: not found", "{} : introuvable", "{}: no encontrado", path.display()));
    }
    let rt = read_json(&path.join("runtime.json"));
    let card = read_json(&path.join("model-card.json"));
    let file = rt
        .as_ref()
        .and_then(|v| v["artifact"]["file"].as_str().map(|s| s.to_string()))
        .or_else(|| card.as_ref().and_then(|v| v["artifacts"][0]["file"].as_str().map(|s| s.to_string())))
        .or_else(|| {
            // fallback: the only .onnx in the folder
            let v: Vec<_> = fs::read_dir(path).ok()?.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("onnx")).collect();
            if v.len() == 1 { Some(v[0].file_name().to_string_lossy().into_owned()) } else { None }
        })
        .ok_or(trs!("Keine runtime.json und keine eindeutige .onnx-Datei im Ordner", "No runtime.json and no single .onnx file in the folder", "Pas de runtime.json ni de fichier .onnx unique dans le dossier", "No hay runtime.json ni un único archivo .onnx en la carpeta"))?;
    let onnx = path.join(&file);
    if !onnx.exists() {
        return Err(tr!("{} fehlt im Ordner", "{} is missing in the folder", "{} manque dans le dossier", "falta {} en la carpeta", file));
    }
    let want = rt
        .as_ref()
        .and_then(|v| v["artifact"]["sha256"].as_str().map(|s| s.to_string()))
        .or_else(|| card.as_ref().and_then(|v| v["artifacts"].as_array()?.iter().find(|a| a["file"].as_str() == Some(&file))?["sha256"].as_str().map(|s| s.to_string())));
    let sha = match want {
        Some(w) => if sha256_hex(&onnx)? == w.to_lowercase() { Sha::Ok } else { Sha::Bad },
        None => Sha::None,
    };
    let classes = rt.as_ref().and_then(|v| v["classes"].as_array()).map(|a| a.iter().filter_map(|c| c.as_str().map(|s| s.to_string())).collect()).unwrap_or_else(default_classes);
    let sport = rt.as_ref().and_then(|v| v["sport"].as_str()).or_else(|| card.as_ref().and_then(|v| v["sport"].as_str())).map(|s| s.to_string());
    let id = rt.as_ref().and_then(|v| v["modelId"].as_str()).or_else(|| card.as_ref().and_then(|v| v["name"].as_str())).map(|s| s.to_string()).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(ModelMeta {
        id,
        dir: path.to_path_buf(),
        sport,
        classes,
        input_size: rt.as_ref().and_then(|v| v["inputSize"].as_u64()).unwrap_or(512) as usize,
        confidence: rt.as_ref().and_then(|v| v["confidenceThreshold"].as_f64()).map(|f| f as f32),
        contract: rt.as_ref().and_then(|v| v["contract"].as_str()).map(|s| s.to_string()),
        sha,
        size_mb: fs::metadata(&onnx).map(|m| m.len()).unwrap_or(0) as f32 / 1e6,
        onnx,
    })
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if n.is_empty() { "model".into() } else { n }
}

/// Copy a model into `<models_dir>/<id>/` (never overwrites an existing import).
pub fn import(src: &Path, models_dir: &Path) -> Result<ModelMeta, String> {
    let meta = inspect(src)?;
    if meta.sha == Sha::Bad {
        return Err(trs!("Prüfsumme der .onnx-Datei stimmt nicht mit model-card.json/runtime.json überein – Datei beschädigt?", "The checksum of the .onnx file does not match model-card.json/runtime.json – is the file damaged?", "La somme de contrôle du fichier .onnx ne correspond pas à model-card.json/runtime.json – fichier endommagé ?", "La suma de comprobación del archivo .onnx no coincide con model-card.json/runtime.json – ¿archivo dañado?").into());
    }
    fs::create_dir_all(models_dir).map_err(|e| format!("{}: {e}", models_dir.display()))?;
    let base = safe_name(&meta.id);
    let mut dest = models_dir.join(&base);
    let mut n = 2;
    while dest.exists() {
        dest = models_dir.join(format!("{base}-{n}"));
        n += 1;
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let file_name = meta.onnx.file_name().ok_or(trs!("Dateiname", "file name", "nom de fichier", "nombre de archivo"))?.to_owned();
    fs::copy(&meta.onnx, dest.join(&file_name)).map_err(|e| tr!("Kopieren: {e}", "Copying: {e}", "Copie : {e}", "Copia: {e}"))?;
    if src.is_dir() {
        for f in ["runtime.json", "model-card.json"] {
            if src.join(f).exists() {
                fs::copy(src.join(f), dest.join(f)).map_err(|e| tr!("Kopieren: {e}", "Copying: {e}", "Copie : {e}", "Copia: {e}"))?;
            }
        }
    }
    if !dest.join("runtime.json").exists() {
        let rt = json!({"schemaVersion": 1, "modelId": meta.id, "artifact": {"file": file_name.to_string_lossy()}, "inputSize": meta.input_size, "classes": meta.classes, "sport": meta.sport, "contract": "rf-detr-imagenet-v1"});
        fs::write(dest.join("runtime.json"), serde_json::to_string_pretty(&rt).unwrap()).map_err(|e| e.to_string())?;
    }
    inspect(&dest)
}

pub fn list(models_dir: &Path) -> Vec<ModelMeta> {
    let mut v: Vec<ModelMeta> = fs::read_dir(models_dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).filter_map(|e| inspect(&e.path()).ok()).collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}
