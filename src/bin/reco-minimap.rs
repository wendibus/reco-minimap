//! reco-minimap: command-line tools around the minimap crate.
//!
//!   reco-minimap calibrate <video> ...   click court landmarks in the browser -> court.json
//!   reco-minimap selfcheck <video> ...   verify the RF-DETR model on your footage (text report)
//!   reco-minimap scales    <video> ...   measure player size vs. distance -> scales.json (for --tile auto)
//!   reco-minimap colors    <video> ...   find the dominant jersey colours -> teams.json
//!   reco-minimap run       <video> ...   panorama video / live feed -> minimap (+ PiP, JSON)

use reco_minimap::{tr, trs};
use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use reco_minimap::detect::{nms, plan_tiles, Class, Det, Tile};
use reco_minimap::geom::{Field, Vec2};
use reco_minimap::sport::{SportProfile, BUILTIN_IDS};
use reco_minimap::scenes::{SceneDetector, SceneEvent, SceneKind};
use reco_minimap::tools::i18n;
use reco_minimap::tools::models;
use reco_minimap::tools::ocr::{self, NumberReader};
use reco_minimap::jersey::{ctc_greedy, ctc_roster, parse_number, parse_roster};
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::team::Team;
use reco_minimap::tools::calib::{self, CourtCalib};
use reco_minimap::tools::colors::{hex, kmeans, lab_dist, parse_hex, pick_teams};
use reco_minimap::tools::infer::{Ep, RfDetr, Units};
use reco_minimap::tools::live::{self, Hub};
use reco_minimap::tools::scales::{floor_points, plan_auto, plan_auto_field_roi, plan_auto_roi, Scales};
use reco_minimap::tools::video::{self, Codec, Frame, FrameReader, FrameWriter, VideoInfo};
use reco_minimap::{ball::BallState, Config, FrameState, Minimap};

/// The help text in the current language.
fn help() -> &'static str {
    match i18n::lang() {
        i18n::Lang::De => include_str!("help/de.txt"),
        i18n::Lang::En => include_str!("help/en.txt"),
        i18n::Lang::Fr => include_str!("help/fr.txt"),
        i18n::Lang::Es => include_str!("help/es.txt"),
    }
}

struct Opts {
    flags: HashMap<String, String>,
    pos: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Self {
        let (mut flags, mut pos) = (HashMap::new(), Vec::new());
        let mut i = 0;
        while i < args.len() {
            if let Some(k) = args[i].strip_prefix("--") {
                if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                    flags.insert(k.to_string(), args[i + 1].clone());
                    i += 1;
                } else {
                    flags.insert(k.to_string(), "true".into());
                }
            } else {
                pos.push(args[i].clone());
            }
            i += 1;
        }
        Self { flags, pos }
    }
    fn s(&self, k: &str) -> Option<&str> {
        self.flags.get(k).map(|s| s.as_str())
    }
    fn f(&self, k: &str, d: f64) -> f64 {
        self.s(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn u(&self, k: &str, d: usize) -> usize {
        self.s(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains_key(k)
    }
}

fn fail<T>(msg: impl std::fmt::Display) -> T {
    eprintln!("{}", tr!("Fehler: {msg}", "Error: {msg}", "Erreur : {msg}", "Error: {msg}"));
    std::process::exit(1)
}

/// Seconds from "90", "1:30" or "0:01:30".
fn parse_clock(s: &str) -> Option<f64> {
    let parts: Vec<f64> = s.trim().split(':').map(|p| p.trim().parse::<f64>().ok().filter(|v| *v >= 0.0)).collect::<Option<_>>()?;
    match parts.as_slice() {
        [s] => Some(*s),
        [m, s] => Some(m * 60.0 + s),
        [h, m, s] => Some(h * 3600.0 + m * 60.0 + s),
        _ => None,
    }
}

fn clock_opt(o: &Opts, key: &str) -> Option<f64> {
    o.s(key).map(|v| parse_clock(v).unwrap_or_else(|| fail(tr!("--{key} {v}: Sekunden oder min:s oder h:min:s erwartet", "--{key} {v}: expected seconds, min:s or h:min:s", "--{key} {v} : secondes, min:s ou h:min:s attendus", "--{key} {v}: se esperan segundos, min:s o h:min:s"))))
}

/// Start of the section to process (`--start`).
fn t_start(o: &Opts) -> f64 {
    clock_opt(o, "start").unwrap_or(0.0)
}

/// Length of the section: `--duration`, or up to `--end`. `None` = to the end of the video.
fn t_duration(o: &Opts) -> Option<f64> {
    if let Some(d) = clock_opt(o, "duration") {
        return Some(d).filter(|d| *d > 0.0);
    }
    let end = clock_opt(o, "end")?;
    if end <= t_start(o) {
        fail::<()>(trs!("--end muss nach --start liegen", "--end must be after --start", "--end doit être après --start", "--end debe ser posterior a --start"));
    }
    Some(end - t_start(o))
}

/// Wall-clock duration, not a time of day: hours intentionally exceed 24.
/// Round estimates to seconds before splitting so minute carries stay correct.
fn duration_hms(seconds: f64) -> String {
    if !seconds.is_finite() { return "?".into(); }
    let total = seconds.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60)
}

#[cfg(test)]
mod duration_tests {
    use super::duration_hms;
    #[test]
    fn duration_handles_boundaries_and_long_runs() {
        for (seconds, expected) in [(0.0,"00:00:00"),(-1.0,"00:00:00"),(59.6,"00:01:00"),(3599.6,"01:00:00"),(33250.0,"09:14:10"),(170304.0,"47:18:24"),(360001.0,"100:00:01")] {
            assert_eq!(duration_hms(seconds), expected);
        }
        assert_eq!(duration_hms(f64::NAN), "?");
        assert_eq!(duration_hms(f64::INFINITY), "?");
    }
}

fn open_info(o: &Opts, input: &str) -> VideoInfo {
    // With --size the source is not probed: ffprobe would consume a one-shot live connection (srt/rtmp listener).
    let mut info = if o.s("size").is_some() {
        VideoInfo { w: 0, h: 0, fps: 25.0, duration: None }
    } else {
        video::probe(input).unwrap_or_else(|e| fail(e))
    };
    if let Some(sz) = o.s("size") {
        let (w, h) = sz.split_once('x').unwrap_or_else(|| fail(trs!("--size erwartet WxH, z.B. 4096x1500", "--size expects WxH, e.g. 4096x1500", "--size attend LxH, p. ex. 4096x1500", "--size espera AnxAl, p. ej. 4096x1500")));
        info.w = w.parse().unwrap_or_else(|_| fail("--size"));
        info.h = h.parse().unwrap_or_else(|_| fail("--size"));
    }
    if let Some(f) = o.s("fps").and_then(|v| v.parse().ok()) {
        info.fps = f;
    }
    info
}

fn plan(o: &Opts, info: &VideoInfo) -> Vec<Tile> {
    let r: Vec<f32> = o.s("roi").map(|s| s.split(',').map(|v| v.trim().parse::<f32>().unwrap_or_else(|_| fail(trs!("--roi: vier Zahlen x0,y0,x1,y1 erwartet", "--roi: four numbers x0,y0,x1,y1 expected", "--roi : quatre nombres x0,y0,x1,y1 attendus", "--roi: se esperan cuatro números x0,y0,x1,y1")))).collect()).unwrap_or_default();
    if o.has("roi") && (r.len() != 4 || r.iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0) || r[2] <= r[0] || r[3] <= r[1]) {
        fail::<()>(trs!("--roi: 0 <= x0 < x1 <= 1 und 0 <= y0 < y1 <= 1 erforderlich", "--roi: 0 <= x0 < x1 <= 1 and 0 <= y0 < y1 <= 1 required", "--roi : 0 <= x0 < x1 <= 1 et 0 <= y0 < y1 <= 1 requis", "--roi: se requiere 0 <= x0 < x1 <= 1 y 0 <= y0 < y1 <= 1"));
    }
    let (x0, y0, x1, y1) = if r.len() == 4 { (r[0], r[1], r[2], r[3]) } else { (0.0, 0.0, 1.0, 1.0) };
    let (w, h) = (info.w as f32, info.h as f32);
    let x = (x0 * w).floor() as u32;
    let y = (y0 * h).floor() as u32;
    let roi = Tile { x, y, w: (x1 * w).ceil() as u32 - x, h: (y1 * h).ceil() as u32 - y };
    if o.s("tile") == Some("auto") {
        let sc = Scales::load(o.s("scales").unwrap_or("scales.json")).unwrap_or_else(|e| fail(e));
        let overlap = o.f("overlap", 0.25) as f32;
        // with a calibration at hand, only tile where the field is (ball detections are used up to 3 m outside)
        let field = o.s("court").filter(|_| !o.has("no-field-mask")).and_then(|c| {
            let (cal, profile) = load_court(c);
            let inv = cal.projector().and_then(|p| p.pano_to_court.inverse())?;
            // columns: where the ball is still used (3 m outside); rows: where players are tracked (1 m)
            Some((floor_points(&inv, &profile.field, 3.0, info.w, info.h), floor_points(&inv, &profile.field, 1.0, info.w, info.h)))
        });
        let tiles = match &field {
            Some((floor, feet)) => plan_auto_field_roi(&sc, info.w, info.h, overlap, roi, floor, Some(feet)),
            None => plan_auto_roi(&sc, info.w, info.h, overlap, roi),
        };
        if tiles.is_empty() {
            fail::<()>(trs!("Keine Auto-Kacheln: scales.json und --roi prüfen", "No auto tiles: check scales.json and --roi", "Aucune tuile automatique : vérifiez scales.json et --roi", "Sin teselas automáticas: revise scales.json y --roi"));
        }
        return tiles;
    }
    let sizes: Vec<u32> = o.s("tile").unwrap_or("512").split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let mut all = Vec::new();
    for sz in if sizes.is_empty() { vec![512] } else { sizes } {
        all.extend(plan_tiles(roi, sz, o.f("overlap", 0.25) as f32));
    }
    all
}

fn load_model(o: &Opts) -> RfDetr {
    let path = o.s("model").unwrap_or_else(|| fail(trs!("--model <ordner|.onnx> fehlt", "--model <folder|.onnx> is missing", "--model <dossier|.onnx> manquant", "falta --model <carpeta|.onnx>")));
    let ep = Ep::parse(o.s("ep").unwrap_or("cpu")).unwrap_or_else(|| fail("--ep: cpu | auto | cuda | tensorrt | directml | coreml"));
    if let Some(d) = o.s("device") {
        d.parse::<i32>().unwrap_or_else(|_| fail("--device: 0, 1, ..."));
        std::env::set_var("RECO_MINIMAP_DEVICE", d);
    }
    let mut m = RfDetr::load(path, ep, o.s("units").map(|u| Units::parse(u).unwrap_or_else(|| fail(trs!("--units: all | ane | gpu | cpu", "--units: all | ane | gpu | cpu", "--units : all | ane | gpu | cpu", "--units: all | ane | gpu | cpu")))).unwrap_or(Units::All), o.u("workers", 1)).unwrap_or_else(|e| fail(e));
    m.cfg.thr_ball = o.f("thr-ball", 0.30) as f32;
    m.cfg.thr_player = thr_players(o).1;
    m.cfg.thr_referee = o.f("thr-ref", 0.50) as f32;
    eprintln!("{}", tr!("Modell: {} ({}x{}, Klassen {:?}), {} Worker", "Model: {} ({}x{}, classes {:?}), {} worker(s)", "Modèle : {} ({}x{}, classes {:?}), {} worker(s)", "Modelo: {} ({}x{}, clases {:?}), {} worker(s)", m.info.id, m.size, m.size, m.info.classes, m.workers()));
    m
}

/// Player thresholds (low, high): detections from `--thr-player-low` on keep an existing track going,
/// only those from `--thr-player` on may start one. Set both to the same value to turn this off.
fn thr_players(o: &Opts) -> (f32, f32) {
    let high = o.f("thr-player", 0.60) as f32;
    (o.s("thr-player-low").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.30).min(high), high)
}

/// Tracking configuration for a game. `--referees N` overrides the number of referee slots from the
/// calibration; with 0 nobody is taken for a referee because of an odd shirt colour either, so such
/// a player stays a player.
fn game_config(o: &Opts, profile: &SportProfile, home: [u8; 3], away: [u8; 3]) -> Config {
    let mut profile = profile.clone();
    if o.has("referees") {
        profile.referees = o.u("referees", profile.referees);
    }
    let mut cfg = Config::for_sport(&profile, home, away);
    cfg.referee_from_color = profile.referees > 0;
    cfg
}

/// Part of a player box that shows the shirt (`--torso top,bottom` as fractions of the box height).
fn torso_band(o: &Opts) -> (f32, f32) {
    let Some(v) = o.s("torso") else {
        return video::TORSO_BAND;
    };
    let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    if p.len() != 2 || !(0.0..1.0).contains(&p[0]) || !(p[0] < p[1] && p[1] <= 1.0) {
        fail::<()>(trs!("--torso erwartet oben,unten als Bruchteile der Boxhöhe, z.B. 0.18,0.50", "--torso expects top,bottom as fractions of the box height, e.g. 0.18,0.50", "--torso attend haut,bas en fractions de la hauteur de la boîte, p. ex. 0.18,0.50", "--torso espera arriba,abajo como fracciones de la altura del recuadro, p. ej. 0.18,0.50"));
    }
    (p[0], p[1])
}

/// Sport profile from --sport (+ --length/--width/--team-size/--referees for `custom`).
fn profile_from_opts(o: &Opts) -> SportProfile {
    let id = o.s("sport").unwrap_or("basketball");
    let mut p = if id == "custom" {
        SportProfile::custom_rect("custom", o.f("length", 40.0) as f32, o.f("width", 20.0) as f32, o.u("team-size", 5), o.u("referees", 2))
    } else {
        SportProfile::builtin(id).unwrap_or_else(|| fail(tr!("--sport {id}: unbekannt (möglich: {})", "--sport {id}: unknown (available: {})", "--sport {id} : inconnu (possibles : {})", "--sport {id}: desconocido (posibles: {})", BUILTIN_IDS.join(", "))))
    };
    if o.has("team-size") {
        p.team_size = o.u("team-size", p.team_size);
    }
    if o.has("referees") {
        p.referees = o.u("referees", p.referees);
    }
    p
}

fn load_court(path: &str) -> (CourtCalib, SportProfile) {
    let c = CourtCalib::load(path).unwrap_or_else(|e| fail(e));
    let p = c.profile();
    (c, p)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn median(v: &mut Vec<f32>) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" || args[0] == "help" {
        // `--help --lang fr` and `help --lang fr` work too
        let lang = args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1)).and_then(|l| i18n::Lang::parse(l));
        i18n::set_lang(lang.unwrap_or_else(i18n::from_env));
        print!("{}", help());
        return;
    }
    if matches!(args[0].as_str(), "--version" | "-V" | "version") {
        println!("reco-minimap {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let o = Opts::parse(&args[1..]);
    // language of all messages: --lang, else RECO_MINIMAP_LANG / the system locale, else English
    i18n::set_lang(o.s("lang").and_then(i18n::Lang::parse).unwrap_or_else(i18n::from_env));
    if args[0] == "fetch-ocr" {
        let dest = o.s("to").map(std::path::PathBuf::from).unwrap_or_else(ocr::default_model_path);
        match ocr::fetch_model(&dest) {
            Ok(p) => println!("{}", tr!("OCR-Modell gespeichert: {}", "Number model saved: {}", "Modèle de numéros enregistré : {}", "Modelo de dorsales guardado: {}", p.display())),
            Err(e) => fail::<()>(e),
        }
        return;
    }
    if args[0] == "update" {
        cmd_update(&o);
        return;
    }
    if args[0] == "ui" {
        let url = reco_minimap::tools::ui::serve(o.u("port", 8788) as u16, !o.has("no-open"), false, !o.has("no-update-check")).unwrap_or_else(|e| fail(e));
        eprintln!("{}", tr!("Oberfläche: {url}\n(Strg+C beendet das Programm; laufende Aufträge werden dabei abgebrochen.)", "Interface: {url}\n(Ctrl+C stops the program; running jobs are cancelled.)", "Interface : {url}\n(Ctrl+C arrête le programme ; les tâches en cours sont annulées.)", "Interfaz: {url}\n(Ctrl+C detiene el programa; las tareas en curso se cancelan.)"));
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    let input = o.pos.first().cloned().unwrap_or_else(|| fail(trs!("<video> fehlt (siehe --help)", "<video> is missing (see --help)", "<vidéo> manquante (voir --help)", "falta <vídeo> (ver --help)")));
    match args[0].as_str() {
        "calibrate" => cmd_calibrate(&o, &input),
        "selfcheck" => cmd_selfcheck(&o, &input),
        "scales" => cmd_scales(&o, &input),
        "stats" => cmd_stats(&o, &input),
        "review-identities" => {
            let annotations = o.s("annotations").unwrap_or_else(|| fail(trs!("--annotations fehlt", "--annotations is missing", "--annotations manquant", "falta --annotations")));
            let output = o.s("out").unwrap_or_else(|| fail(trs!("--out fehlt", "--out is missing", "--out manquant", "falta --out")));
            let n = reco_minimap::tools::review::run(std::path::Path::new(&input), std::path::Path::new(annotations), std::path::Path::new(output)).unwrap_or_else(|e| fail(e));
            println!("{}", tr!("{n} Bilder geprüft: {output} (Original unverändert)", "{n} frames reviewed: {output} (original unchanged)", "{n} images vérifiées : {output} (original inchangé)", "{n} fotogramas revisados: {output} (original sin cambios)"));
        }
        "import-model" => cmd_import_model(&o, &input),
        "ocr-bench" => cmd_ocr_bench(&o, &input),
        "colors" => cmd_colors(&o, &input),
        "run" => cmd_run(&o, &input),
        "replay" => cmd_replay(&o, &input),
        "live" => cmd_live(&o, &input),
        other => fail(tr!("unbekannter Befehl '{other}' (calibrate | selfcheck | colors | run)", "unknown command '{other}' (see --help)", "commande inconnue « {other} » (voir --help)", "comando desconocido '{other}' (ver --help)")),
    }
}

// ---------------------------------------------------------------- update

/// `update`: look for a newer version on GitHub; with `--yes` build and install it.
fn cmd_update(o: &Opts) {
    use reco_minimap::tools::update;
    println!("{}", tr!("Installiert: {}   (Quelle: https://github.com/{})", "Installed: {}   (source: https://github.com/{})", "Installée : {}   (source : https://github.com/{})", "Instalada: {}   (fuente: https://github.com/{})", update::CURRENT, update::REPO));
    let latest = match update::latest() {
        Ok(Some(l)) => l,
        Ok(None) => return println!("{}", trs!("Auf GitHub ist noch keine Version veröffentlicht.", "No version has been published on GitHub yet.", "Aucune version n'est encore publiée sur GitHub.", "Aún no hay ninguna versión publicada en GitHub.")),
        Err(e) => fail(e),
    };
    if !update::is_newer(&latest.version, update::CURRENT) {
        return println!("{}", tr!("Aktuell: die neueste veröffentlichte Version ist {}.", "Up to date: the latest published version is {}.", "À jour : la dernière version publiée est {}.", "Al día: la última versión publicada es {}.", latest.version));
    }
    println!("{}", tr!("Neue Version {} verfügbar: {}", "New version {} available: {}", "Nouvelle version {} disponible : {}", "Nueva versión {} disponible: {}", latest.version, latest.url));
    if o.has("check") {
        return;
    }
    if !o.has("yes") {
        return println!("{}", trs!("Installieren mit:  reco-minimap update --yes   (baut aus dem Quelltext, dauert einige Minuten)", "Install with:  reco-minimap update --yes   (builds from source, takes a few minutes)", "Installer avec :  reco-minimap update --yes   (compile depuis les sources, quelques minutes)", "Instalar con:  reco-minimap update --yes   (compila desde el código fuente, tarda unos minutos)"));
    }
    println!("{}", tr!("Installiere {} …", "Installing {} …", "Installation de {} …", "Instalando {} …", latest.version));
    update::install(&latest).unwrap_or_else(|e| fail(e));
    println!("{}", tr!("Version {} ist installiert. Laufende Programme (z. B. die Oberfläche) bitte neu starten.", "Version {} is installed. Please restart running programs (e.g. the interface).", "La version {} est installée. Veuillez relancer les programmes en cours (p. ex. l'interface).", "La versión {} está instalada. Reinicia los programas en ejecución (p. ej. la interfaz).", latest.version));
}

// ---------------------------------------------------------------- calibrate

fn cmd_calibrate(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let out = o.s("out").unwrap_or("court.json");
    eprintln!("{}", tr!("Panorama {}x{} @ {:.2} fps", "Panorama {}x{} @ {:.2} fps", "Panorama {}x{} à {:.2} i/s", "Panorama {}x{} a {:.2} fps", info.w, info.h, info.fps));
    let profile = profile_from_opts(o);
    eprintln!("{}", tr!("Sportart: {} ({} × {} m)", "Sport: {} ({} × {} m)", "Sport : {} ({} × {} m)", "Deporte: {} ({} × {} m)", profile.id, profile.field.length, profile.field.width));
    calib::run(input, &info, o.f("time", 5.0), out, !o.has("no-open"), profile, o.s("lang").unwrap_or("de")).unwrap_or_else(|e| fail(e));
    eprintln!("{}", tr!("Fertig: {out}", "Done: {out}", "Terminé : {out}", "Listo: {out}"));
}

// ---------------------------------------------------------------- selfcheck

fn cmd_selfcheck(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let samples = o.u("samples", 12).max(1);
    let start = t_start(o);
    let span = t_duration(o).or(info.duration.map(|d| d - start - 1.0)).unwrap_or(60.0).max(1.0);
    eprintln!("{}", tr!("Panorama {}x{}, {} Kachel(n) à {} px, {} Stichproben\n", "Panorama {}x{}, {} tile(s) of {} px, {} samples\n", "Panorama {}x{}, {} tuile(s) de {} px, {} échantillons\n", "Panorama {}x{}, {} tesela(s) de {} px, {} muestras\n", info.w, info.h, tiles.len(), o.u("tile", 512), samples));

    let nc = model.cfg.n_classes;
    let (mut lmin, mut lmax) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut bmin, mut bmax) = (f32::INFINITY, f32::NEG_INFINITY);
    let mut argmax_hits = vec![0usize; nc];
    let mut best_score = vec![0f32; nc];
    let mut tile_ms = Vec::new();
    let (mut players, mut refs, mut balls) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ball_w, mut ball_h, mut pl_h, mut ball_scores) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut frames_with_ball = 0usize;
    let mut done = 0usize;
    let mut last_frame: Option<Frame> = None;
    // per tile size: (player hits, referee hits, ball hits, player heights in panorama px)
    let mut per_scale: std::collections::BTreeMap<u32, (usize, usize, usize, Vec<f32>)> = Default::default();

    for s in 0..samples {
        let t = start + span * (s as f64 + 0.5) / samples as f64;
        let frame = match video::grab(input, &info, t) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("  t={t:.1}s: {e}");
                continue;
            }
        };
        let mut dets = Vec::new();
        for tile in &tiles {
            let chw = frame.tile_tensor(tile, model.size);
            let t0 = Instant::now();
            let (boxes, logits, nq, ncl) = model.run_raw(&chw).unwrap_or_else(|e| fail(e));
            tile_ms.push(t0.elapsed().as_secs_f32() * 1000.0);
            if ncl != nc {
                fail::<()>(tr!("Modell liefert {ncl} Klassen-Logits, runtime.json nennt {nc}", "The model returns {ncl} class logits, runtime.json lists {nc}", "Le modèle renvoie {ncl} logits de classe, runtime.json en indique {nc}", "El modelo devuelve {ncl} logits de clase, runtime.json indica {nc}"));
            }
            for v in &logits {
                lmin = lmin.min(*v);
                lmax = lmax.max(*v);
            }
            for v in &boxes {
                bmin = bmin.min(*v);
                bmax = bmax.max(*v);
            }
            for q in 0..nq {
                let l = &logits[q * nc..(q + 1) * nc];
                let (ci, mx) = l.iter().enumerate().fold((0, f32::NEG_INFINITY), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
                let sc = sigmoid(mx);
                best_score[ci] = best_score[ci].max(sc);
                if sc >= 0.5 {
                    argmax_hits[ci] += 1;
                }
                if sc >= 0.05 && model.cfg.class_map.get(ci).copied().flatten() == Some(Class::Ball) {
                    ball_scores.push(sc);
                }
            }
            if let Ok(d) = reco_minimap::detect::decode_rfdetr(&boxes, &logits, &model.cfg) {
                let e = per_scale.entry(tile.w.max(tile.h)).or_insert((0, 0, 0, Vec::new()));
                for d in d {
                    let m = reco_minimap::detect::tile_to_pano(&d, tile, frame.w as u32, frame.h as u32);
                    match m.class {
                        Class::Player => {
                            e.0 += 1;
                            e.3.push(m.h * frame.h as f32);
                        }
                        Class::Referee => e.1 += 1,
                        Class::Ball => e.2 += 1,
                    }
                    dets.push(m);
                }
            }
        }
        let dets = nms(dets, 0.5);
        let (np, nr, nb) = (dets.iter().filter(|d| d.class == Class::Player).count(), dets.iter().filter(|d| d.class == Class::Referee).count(), dets.iter().filter(|d| d.class == Class::Ball).count());
        players.push(np as f32);
        refs.push(nr as f32);
        balls.push(nb as f32);
        if nb > 0 {
            frames_with_ball += 1;
        }
        for d in &dets {
            match d.class {
                Class::Ball => {
                    ball_w.push(d.w * frame.w as f32);
                    ball_h.push(d.h * frame.h as f32);
                }
                Class::Player => pl_h.push(d.h * frame.h as f32),
                _ => {}
            }
        }
        if let Some(dir) = o.s("annotate") {
            let _ = std::fs::create_dir_all(dir);
            let mut f = Frame { w: frame.w, h: frame.h, rgb: frame.rgb.clone() };
            draw_dets(&mut f, &dets);
            let p = format!("{dir}/sample_{s:02}_t{:.1}s.png", t);
            if let Err(e) = f.write_png(&p) {
                eprintln!("  annotate: {e}");
            }
        }
        last_frame = Some(frame);
        done += 1;
        eprintln!("{}", tr!("  t={t:6.1}s  Spieler {np:2}  Schiris {nr}  Ball {nb}", "  t={t:6.1}s  players {np:2}  referees {nr}  ball {nb}", "  t={t:6.1}s  joueurs {np:2}  arbitres {nr}  ballon {nb}", "  t={t:6.1}s  jugadores {np:2}  árbitros {nr}  balón {nb}"));
    }
    if done == 0 {
        fail::<()>(trs!("kein Frame lesbar", "no frame could be read", "aucune image lisible", "no se pudo leer ninguna imagen"));
    }

    println!("{}", trs!("\n================ Selbsttest ================", "\n================ Self-check ================", "\n================ Autotest ================", "\n================ Autocomprobación ================"));
    println!("{}", tr!("Inferenz: {:.0} ms/Kachel (Median) -> {:.0} ms/Bild bei {} Kacheln", "Inference: {:.0} ms/tile (median) -> {:.0} ms/frame with {} tiles", "Inférence : {:.0} ms/tuile (médiane) -> {:.0} ms/image avec {} tuiles", "Inferencia: {:.0} ms/tesela (mediana) -> {:.0} ms/imagen con {} teselas", median(&mut tile_ms.clone()).unwrap_or(0.0), median(&mut tile_ms.clone()).unwrap_or(0.0) * tiles.len() as f32, tiles.len()));
    if let Some(f) = &last_frame {
        let mut walls = Vec::new();
        for _ in 0..4 {
            let t0 = Instant::now();
            let _ = model.detect(f, &tiles);
            walls.push(t0.elapsed().as_secs_f32() * 1000.0);
        }
        walls.remove(0); // first call includes warm-up
        println!("{}", tr!("Wandzeit pro Bild mit {} Worker(n): {:.0} ms (alle {} Kacheln, Median)", "Wall time per frame with {} worker(s): {:.0} ms (all {} tiles, median)", "Temps réel par image avec {} worker(s) : {:.0} ms (les {} tuiles, médiane)", "Tiempo real por imagen con {} worker(s): {:.0} ms (las {} teselas, mediana)", model.workers(), median(&mut walls).unwrap_or(0.0), tiles.len()));
    }
    println!("{}", tr!("Logits: {lmin:.1} … {lmax:.1}   Boxen-Werte: {bmin:.3} … {bmax:.3}", "Logits: {lmin:.1} … {lmax:.1}   box values: {bmin:.3} … {bmax:.3}", "Logits : {lmin:.1} … {lmax:.1}   valeurs des boîtes : {bmin:.3} … {bmax:.3}", "Logits: {lmin:.1} … {lmax:.1}   valores de los recuadros: {bmin:.3} … {bmax:.3}"));
    println!("{}", trs!("\nKlassen-Index  (laut runtime.json)   Treffer >=0.5   bester Score", "\nClass index  (from runtime.json)   hits >=0.5   best score", "\nIndice de classe  (selon runtime.json)   détections >=0.5   meilleur score", "\nÍndice de clase  (según runtime.json)   detecciones >=0.5   mejor puntuación"));
    for i in 0..nc {
        println!("  [{i}] {:<10}  {:>12}   {:>7.2}", model.info.classes.get(i).map(|s| s.as_str()).unwrap_or("?"), argmax_hits[i], best_score[i]);
    }
    println!("{}", trs!("\nPro Maßstab (Kachelgröße -> Verkleinerung im Modelleingang):", "\nPer scale (tile size -> reduction in the model input):", "\nPar échelle (taille de tuile -> réduction à l'entrée du modèle) :", "\nPor escala (tamaño de tesela -> reducción en la entrada del modelo):"));
    for (sz, (np, nr, nb, hs)) in &per_scale {
        let mut hs = hs.clone();
        let hp = median(&mut hs).unwrap_or(0.0);
        println!("{}", tr!("  Kachel {sz:>5} px (x{:.2}): Spieler-Treffer {np:>4}, Spielerhöhe im Panorama {hp:>4.0} px = im Modelleingang {:>4.0} px; Schiris {nr}, Ball {nb}", "  tile {sz:>5} px (x{:.2}): player hits {np:>4}, player height in the panorama {hp:>4.0} px = in the model input {:>4.0} px; referees {nr}, ball {nb}", "  tuile {sz:>5} px (x{:.2}) : joueurs détectés {np:>4}, hauteur des joueurs dans le panorama {hp:>4.0} px = à l'entrée du modèle {:>4.0} px ; arbitres {nr}, ballon {nb}", "  tesela {sz:>5} px (x{:.2}): jugadores detectados {np:>4}, altura de jugador en el panorama {hp:>4.0} px = en la entrada del modelo {:>4.0} px; árbitros {nr}, balón {nb}", 512.0 / *sz as f32, hp * 512.0 / *sz as f32));
    }
    if let Some((sz, (np, ..))) = per_scale.iter().max_by_key(|(_, v)| v.0) {
        println!("{}", tr!("  -> meiste Spieler bei Kachel {sz} px ({np} Treffer).", "  -> most players with tile {sz} px ({np} hits).", "  -> le plus de joueurs avec la tuile {sz} px ({np} détections).", "  -> más jugadores con la tesela de {sz} px ({np} detecciones)."));
    }
    let mut pl = players.clone();
    let mut rf = refs.clone();
    println!("{}", tr!("\nPro Bild (Median): Spieler {:.0}, Schiris {:.0}; Ball in {:.0}% der Bilder", "\nPer frame (median): players {:.0}, referees {:.0}; ball in {:.0}% of the frames", "\nPar image (médiane) : joueurs {:.0}, arbitres {:.0} ; ballon dans {:.0}% des images", "\nPor imagen (mediana): jugadores {:.0}, árbitros {:.0}; balón en el {:.0}% de las imágenes", median(&mut pl).unwrap_or(0.0), median(&mut rf).unwrap_or(0.0), 100.0 * frames_with_ball as f32 / done as f32));
    if let (Some(w), Some(h)) = (median(&mut ball_w.clone()), median(&mut ball_h.clone())) {
        println!("{}", tr!("Ball-Größe im Panorama (Median): {w:.0} x {h:.0} px", "Ball size in the panorama (median): {w:.0} x {h:.0} px", "Taille du ballon dans le panorama (médiane) : {w:.0} x {h:.0} px", "Tamaño del balón en el panorama (mediana): {w:.0} x {h:.0} px"));
    }
    if let Some(h) = median(&mut pl_h.clone()) {
        println!("{}", tr!("Spielerhöhe im Panorama (Median): {h:.0} px", "Player height in the panorama (median): {h:.0} px", "Hauteur des joueurs dans le panorama (médiane) : {h:.0} px", "Altura de jugador en el panorama (mediana): {h:.0} px"));
    }
    if let Some(m) = median(&mut ball_scores.clone()) {
        println!("{}", tr!("Ball-Scores >= 0.05: Median {m:.2}, {} Kandidaten (Schwelle jetzt {:.2})", "Ball scores >= 0.05: median {m:.2}, {} candidates (threshold now {:.2})", "Scores du ballon >= 0.05 : médiane {m:.2}, {} candidats (seuil actuel {:.2})", "Puntuaciones del balón >= 0.05: mediana {m:.2}, {} candidatos (umbral actual {:.2})", ball_scores.len(), model.cfg.thr_ball));
    }

    println!("{}", trs!("\nBewertung:", "\nAssessment:", "\nÉvaluation :", "\nValoración:"));
    let mut warn = false;
    if bmax > 1.5 {
        println!("{}", tr!("  ! Boxen sind nicht auf 0..1 normalisiert (max {bmax:.1}) – vermutlich Pixel (0..{}); Decoder muss durch die Eingangsgröße teilen.", "  ! Boxes are not normalised to 0..1 (max {bmax:.1}) – probably pixels (0..{}); the decoder must divide by the input size.", "  ! Les boîtes ne sont pas normalisées sur 0..1 (max {bmax:.1}) – sans doute des pixels (0..{}) ; le décodeur doit diviser par la taille d'entrée.", "  ! Los recuadros no están normalizados a 0..1 (máx {bmax:.1}) – probablemente píxeles (0..{}); el decodificador debe dividir por el tamaño de entrada.", model.size));
        warn = true;
    } else if bmin < -0.2 {
        println!("{}", tr!("  ! Boxen haben negative Werte ({bmin:.2}) – ggf. anderes Boxformat (xyxy).", "  ! Boxes have negative values ({bmin:.2}) – possibly another box format (xyxy).", "  ! Les boîtes ont des valeurs négatives ({bmin:.2}) – peut-être un autre format (xyxy).", "  ! Los recuadros tienen valores negativos ({bmin:.2}) – quizá otro formato (xyxy)."));
        warn = true;
    } else {
        println!("{}", trs!("  ok Boxen liegen in 0..1 (cx,cy,w,h plausibel).", "  ok Boxes lie in 0..1 (cx,cy,w,h plausible).", "  ok Les boîtes sont dans 0..1 (cx,cy,w,h plausibles).", "  ok Los recuadros están en 0..1 (cx,cy,w,h plausibles)."));
    }
    let total_hits: usize = argmax_hits.iter().sum();
    if total_hits == 0 {
        println!("{}", trs!("  ! Keine einzige Erkennung >= 0.5: Vorverarbeitung (RGB/ImageNet-Normalisierung), --roi oder Kachelgröße prüfen.", "  ! Not a single detection >= 0.5: check the preprocessing (RGB/ImageNet normalisation), --roi or the tile size.", "  ! Aucune détection >= 0.5 : vérifier le prétraitement (RGB/normalisation ImageNet), --roi ou la taille des tuiles.", "  ! Ninguna detección >= 0.5: revisa el preprocesado (RGB/normalización ImageNet), --roi o el tamaño de tesela."));
        warn = true;
    } else {
        let p_idx = model.info.classes.iter().position(|c| c == "player");
        if let Some(pi) = p_idx {
            let max_i = (0..nc).max_by_key(|&i| argmax_hits[i]).unwrap();
            if max_i != pi {
                println!("{}", tr!("  ! Die meisten Treffer liegen auf Index [{max_i}] ('{}'), nicht auf 'player' [{pi}] – Klassenreihenfolge/Offset prüfen.", "  ! Most hits are on index [{max_i}] ('{}'), not on 'player' [{pi}] – check the class order/offset.", "  ! La plupart des détections sont sur l'indice [{max_i}] ('{}'), pas sur 'player' [{pi}] – vérifier l'ordre/le décalage des classes.", "  ! La mayoría de las detecciones están en el índice [{max_i}] ('{}'), no en 'player' [{pi}] – revisa el orden/desplazamiento de las clases.", model.info.classes.get(max_i).map(|s| s.as_str()).unwrap_or("?")));
                warn = true;
            } else {
                println!("{}", trs!("  ok Die meisten Treffer liegen auf 'player'.", "  ok Most hits are on 'player'.", "  ok La plupart des détections sont sur 'player'.", "  ok La mayoría de las detecciones están en 'player'."));
            }
        }
    }
    let mp = median(&mut pl).unwrap_or(0.0);
    if mp < 6.0 || mp > 16.0 {
        println!("{}", tr!("  ? Spieler pro Bild (Median {mp:.0}) weicht von ~10 ab: --thr-player, --roi, Kachelgröße oder Bildausschnitt prüfen.", "  ? Players per frame (median {mp:.0}) is far from the expected number on the field: check --thr-player, --roi, the tile size or the picture area.", "  ? Joueurs par image (médiane {mp:.0}) loin du nombre attendu sur le terrain : vérifier --thr-player, --roi, la taille des tuiles ou le cadrage.", "  ? Jugadores por imagen (mediana {mp:.0}) lejos del número esperado en el campo: revisa --thr-player, --roi, el tamaño de tesela o el encuadre."));
    } else {
        println!("{}", trs!("  ok Spieleranzahl plausibel.", "  ok Number of players plausible.", "  ok Nombre de joueurs plausible.", "  ok Número de jugadores plausible."));
    }
    let rate = frames_with_ball as f32 / done as f32;
    if rate < 0.3 {
        println!("{}", tr!("  ? Ball nur in {:.0}% der Bilder: --thr-ball senken (z.B. 0.2), kleinere --tile / mehr --overlap, oder Ball-Training verbessern.", "  ? Ball in only {:.0}% of the frames: lower --thr-ball (e.g. 0.2), use a smaller --tile / more --overlap, or improve the ball training.", "  ? Ballon dans seulement {:.0}% des images : baisser --thr-ball (p. ex. 0.2), réduire --tile / augmenter --overlap, ou améliorer l'entraînement du ballon.", "  ? Balón solo en el {:.0}% de las imágenes: baja --thr-ball (p. ej. 0.2), usa un --tile menor / más --overlap, o mejora el entrenamiento del balón.", rate * 100.0));
    } else {
        println!("{}", tr!("  ok Ball wird in {:.0}% der Bilder gefunden.", "  ok The ball is found in {:.0}% of the frames.", "  ok Le ballon est trouvé dans {:.0}% des images.", "  ok El balón se encuentra en el {:.0}% de las imágenes.", rate * 100.0));
    }
    if !warn {
        println!("{}", trs!("\nAnnahmen bestätigt – weiter mit: reco-minimap calibrate / colors / run", "\nAssumptions confirmed – continue with: reco-minimap calibrate / colors / run", "\nHypothèses confirmées – continuer avec : reco-minimap calibrate / colors / run", "\nSupuestos confirmados – continúa con: reco-minimap calibrate / colors / run"));
    }
    let _ = std::io::stdout().flush();
}

fn draw_dets(f: &mut Frame, dets: &[Det]) {
    let (fw, fh) = (f.w as f32, f.h as f32);
    for d in dets {
        let (c, th) = match d.class {
            Class::Player => ([0u8, 220, 255], 2),
            Class::Referee => ([255u8, 230, 0], 2),
            Class::Ball => ([255u8, 140, 0], 3),
        };
        let (x0, y0, x1, y1) = (((d.cx - d.w / 2.0) * fw) as i64, ((d.cy - d.h / 2.0) * fh) as i64, ((d.cx + d.w / 2.0) * fw) as i64, ((d.cy + d.h / 2.0) * fh) as i64);
        f.rect(x0, y0, x1, y1, c, th);
    }
}

// ---------------------------------------------------------------- ocr-bench

fn load_rgb(path: &str) -> Option<(usize, usize, Vec<u8>)> {
    let info = video::probe(path).ok()?;
    let mut r = FrameReader::open(path, &info, 0.0, None, 1).ok()?;
    let f = r.next()?;
    Some((f.w, f.h, f.rgb))
}

fn cmd_ocr_bench(o: &Opts, manifest: &str) {
    let model_path = o.s("ocr-model").map(std::path::PathBuf::from).unwrap_or_else(ocr::default_model_path);
    let mut reader = NumberReader::load(&model_path).unwrap_or_else(|e| fail(e));
    let roster = o.s("roster").map(parse_roster).unwrap_or_default();
    let txt = std::fs::read_to_string(manifest).unwrap_or_else(|e| fail(format!("{manifest}: {e}")));
    let mut stats: std::collections::BTreeMap<String, (usize, usize, usize, usize, usize)> = Default::default(); // n, free ok, roster ok, confident, confident ok
    for line in txt.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 2 {
            continue;
        }
        let (path, label) = (f[0], f[1]);
        let group = f.get(2).unwrap_or(&"all").to_string();
        let (w, h, rgb) = match load_rgb(path) { Some(x) => x, None => { eprintln!("{}", tr!("nicht lesbar: {path}", "cannot read: {path}", "illisible : {path}", "no se puede leer: {path}")); continue; } };
        let crop = ocr::localize(w, h, &rgb);
        let e = stats.entry(group).or_default();
        e.0 += 1;
        if let Ok(r) = reader.read(&crop) {
            let (s, _) = ctc_greedy(&r.probs, r.steps);
            if s == label || parse_number(&s).map(|n| n.to_string()) == Some(label.to_string()) { e.1 += 1; }
            if let Some((n, post)) = ctc_roster(&r.probs, r.steps, &roster) {
                if n.to_string() == label { e.2 += 1; }
                if post >= 0.8 { e.3 += 1; if n.to_string() == label { e.4 += 1; } }
            }
        }
    }
    println!("{:<12}{:>6}{:>14}{:>14}{:>26}", trs!("Gruppe", "group", "groupe", "grupo"), "n", trs!("frei richtig", "free, correct", "libre, correct", "libre, correcto"), trs!("Kader richtig", "roster, correct", "effectif, correct", "plantilla, correcto"), trs!("sicher (>=0.8): Anteil/richtig", "confident (>=0.8): share/correct", "sûr (>=0.8) : part/correct", "seguro (>=0.8): parte/correcto"));
    for (g, (n, free, ros, conf, conf_ok)) in &stats {
        let pc = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f32 / b as f32 };
        println!("{:<12}{:>6}{:>13.0}%{:>13.0}%{:>14.0}% / {:.0}%", g, n, pc(*free, *n), pc(*ros, *n), pc(*conf, *n), pc(*conf_ok, *conf));
    }
}

// ---------------------------------------------------------------- import-model

fn cmd_import_model(o: &Opts, input: &str) {
    let to = o.s("to").unwrap_or("models");
    let m = models::import(std::path::Path::new(input), std::path::Path::new(to)).unwrap_or_else(|e| fail(e));
    println!("{}", tr!("Importiert: {} -> {}", "Imported: {} -> {}", "Importé : {} -> {}", "Importado: {} -> {}", m.id, m.dir.display()));
    println!("{}", tr!("  Sportart im Modell: {}", "  Sport in the model: {}", "  Sport du modèle : {}", "  Deporte del modelo: {}", m.sport.as_deref().unwrap_or(trs!("(nicht angegeben)", "(not stated)", "(non indiqué)", "(no indicado)"))));
    println!("{}", tr!("  Klassen: {:?}, Eingabe {}x{}, {:.0} MB", "  Classes: {:?}, input {}x{}, {:.0} MB", "  Classes : {:?}, entrée {}x{}, {:.0} Mo", "  Clases: {:?}, entrada {}x{}, {:.0} MB", m.classes, m.input_size, m.input_size, m.size_mb));
    println!("{}", tr!("  Prüfsumme: {}", "  Checksum: {}", "  Somme de contrôle : {}", "  Suma de comprobación: {}", match m.sha { models::Sha::Ok => trs!("stimmt", "matches", "correcte", "correcta"), models::Sha::Bad => trs!("STIMMT NICHT", "DOES NOT MATCH", "NE CORRESPOND PAS", "NO COINCIDE"), models::Sha::None => trs!("keine vorhanden", "none present", "aucune", "ninguna") }));
}

// ---------------------------------------------------------------- stats

fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn cmd_stats(o: &Opts, path: &str) {
    let field = o.s("court").map(|p| load_court(p).1.field).unwrap_or(Field::basketball());
    use std::collections::{HashMap, HashSet};
    let txt = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let frames: Vec<serde_json::Value> = txt.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if frames.len() < 2 {
        fail::<()>(trs!("zu wenige Bilder in der Datei", "too few frames in the file", "trop peu d'images dans le fichier", "muy pocas imágenes en el archivo"));
    }
    let n = frames.len();
    let (t0, t1) = (frames[0]["t"].as_f64().unwrap_or(0.0), frames[n - 1]["t"].as_f64().unwrap_or(0.0));
    let (mut home_n, mut away_n, mut ref_n) = (Vec::new(), Vec::new(), Vec::new());
    let (mut f55, mut f2) = (0usize, 0usize);
    let mut ids: HashMap<&str, HashSet<u64>> = HashMap::new();
    let mut prev: HashMap<u64, (f64, f64, f64)> = HashMap::new();
    let (mut speeds, mut vis_frac) = (Vec::new(), Vec::new());
    let (mut outside, mut jumps) = (0usize, 0usize);
    let mut ball_states: HashMap<String, usize> = HashMap::new();
    let (mut switches, mut flights, mut last_holder, mut last_state) = (0usize, 0usize, None::<u64>, String::new());
    for f in &frames {
        let t = f["t"].as_f64().unwrap_or(0.0);
        let (mut h, mut a, mut vis, mut tot) = (0, 0, 0, 0);
        for p in f["players"].as_array().into_iter().flatten() {
            let id = p["id"].as_u64().unwrap_or(0);
            let team = match p["team"].as_str() { Some("home") => { h += 1; "home" } _ => { a += 1; "away" } };
            ids.entry(team).or_default().insert(id);
            tot += 1;
            if p["state"].as_str() == Some("visible") {
                vis += 1;
            }
            let (x, y) = (p["x"].as_f64().unwrap_or(0.0), p["y"].as_f64().unwrap_or(0.0));
            if !field.contains(reco_minimap::geom::Vec2::new(x as f32, y as f32), 1.5) {
                outside += 1;
            }
            if let Some((pt, px, py)) = prev.get(&id) {
                let dt = (t - pt).max(1e-3);
                let sp = ((x - px).powi(2) + (y - py).powi(2)).sqrt() / dt;
                speeds.push(sp);
                if sp >= 11.0 {
                    jumps += 1;
                }
            }
            prev.insert(id, (t, x, y));
        }
        if h == 5 && a == 5 {
            f55 += 1;
        }
        let r = f["referees"].as_array().map(|r| r.len()).unwrap_or(0);
        if r == 2 {
            f2 += 1;
        }
        home_n.push(h as f64);
        away_n.push(a as f64);
        ref_n.push(r as f64);
        if tot > 0 {
            vis_frac.push(vis as f64 / tot as f64);
        }
        let b = &f["ball"];
        let st = b["state"].as_str().unwrap_or("unknown").to_string();
        *ball_states.entry(st.clone()).or_default() += 1;
        if st == "flight" && last_state != "flight" {
            flights += 1;
        }
        let holder = if st == "held" { b["holder"].as_u64() } else { None };
        if holder.is_some() && last_holder.is_some() && holder != last_holder {
            switches += 1;
        }
        if holder.is_some() {
            last_holder = holder;
        }
        last_state = st;
    }
    let mean = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len().max(1) as f64;
    println!("{}", trs!("================ Lauf-Report ================", "================ Run report ================", "================ Rapport de traitement ================", "================ Informe del proceso ================"));
    println!("{}", tr!("{n} Bilder, {:.1} s ({:.1} Bilder/s Ausgabe)", "{n} frames, {:.1} s ({:.1} frames/s output)", "{n} images, {:.1} s ({:.1} images/s en sortie)", "{n} imágenes, {:.1} s ({:.1} imágenes/s de salida)", t1 - t0, n as f64 / (t1 - t0).max(1e-6)));
    println!("{}", tr!("\nSlots: Heim im Mittel {:.1}, Gast {:.1}; genau 5+5 in {:.0}% der Bilder; 2 Schiris in {:.0}% (Schiris im Mittel {:.1})", "\nSlots: home {:.1} on average, away {:.1}; exactly 5+5 in {:.0}% of the frames; 2 referees in {:.0}% (referees {:.1} on average)", "\nPlaces : domicile {:.1} en moyenne, visiteurs {:.1} ; exactement 5+5 dans {:.0}% des images ; 2 arbitres dans {:.0}% (arbitres {:.1} en moyenne)", "\nPuestos: local {:.1} de media, visitante {:.1}; exactamente 5+5 en el {:.0}% de las imágenes; 2 árbitros en el {:.0}% (árbitros {:.1} de media)", mean(&home_n), mean(&away_n), 100.0 * f55 as f64 / n as f64, 100.0 * f2 as f64 / n as f64, mean(&ref_n)));
    println!("{}", tr!("Verschiedene Spieler-IDs: Heim {}, Gast {} (ideal 5 + 5; mehr = Slots werden neu vergeben)", "Distinct player IDs: home {}, away {} (ideal 5 + 5; more = slots are being re-assigned)", "Identifiants de joueurs distincts : domicile {}, visiteurs {} (idéal 5 + 5 ; davantage = des places sont réattribuées)", "IDs de jugador distintos: local {}, visitante {} (ideal 5 + 5; más = los puestos se reasignan)", ids.get("home").map(|s| s.len()).unwrap_or(0), ids.get("away").map(|s| s.len()).unwrap_or(0)));
    println!("{}", tr!("Sichtbar (nicht verdeckt/verloren): {:.0}% der Spieler-Einträge", "Visible (not hidden/lost): {:.0}% of the player entries", "Visibles (ni masqués ni perdus) : {:.0}% des entrées de joueurs", "Visibles (ni ocultos ni perdidos): {:.0}% de las entradas de jugador", 100.0 * mean(&vis_frac)));
    let mut sp = speeds.clone();
    println!("{}", tr!("Geschwindigkeit der Punkte: Median {:.1} m/s, 99% {:.1} m/s, max {:.1} m/s; Schritte an der Tempogrenze (>= 11 m/s): {jumps} von {} ({:.1}%); außerhalb des Felds: {outside}", "Speed of the dots: median {:.1} m/s, 99% {:.1} m/s, max {:.1} m/s; steps at the speed limit (>= 11 m/s): {jumps} of {} ({:.1}%); outside the field: {outside}", "Vitesse des points : médiane {:.1} m/s, 99% {:.1} m/s, max {:.1} m/s ; pas à la vitesse limite (>= 11 m/s) : {jumps} sur {} ({:.1}%) ; hors du terrain : {outside}", "Velocidad de los puntos: mediana {:.1} m/s, 99% {:.1} m/s, máx {:.1} m/s; pasos en el límite de velocidad (>= 11 m/s): {jumps} de {} ({:.1}%); fuera del campo: {outside}", pct(&mut sp, 0.5), pct(&mut sp, 0.99), pct(&mut sp, 1.0), speeds.len(), 100.0 * jumps as f64 / speeds.len().max(1) as f64));
    let tot: usize = ball_states.values().sum();
    let g = |k: &str| 100.0 * *ball_states.get(k).unwrap_or(&0) as f64 / tot as f64;
    println!("{}", tr!("Ball: gehalten {:.0}%, Flug {:.0}%, frei {:.0}%, unbekannt {:.0}%; {flights} Flugphasen, {switches} Ballhalter-Wechsel", "Ball: held {:.0}%, in flight {:.0}%, loose {:.0}%, unknown {:.0}%; {flights} flight phases, {switches} changes of ball holder", "Ballon : tenu {:.0}%, en vol {:.0}%, libre {:.0}%, inconnu {:.0}% ; {flights} phases de vol, {switches} changements de porteur", "Balón: retenido {:.0}%, en vuelo {:.0}%, suelto {:.0}%, desconocido {:.0}%; {flights} fases de vuelo, {switches} cambios de poseedor", g("held"), g("flight"), g("loose"), g("unknown")));
    let mut sub_lines = Vec::new();
    for f in &frames {
        for e in f["events"].as_array().into_iter().flatten().filter(|e| e["type"] == "sub") {
            let n = |k: &str| e[k].as_u64().map(|x| format!("#{x}")).unwrap_or_else(|| "?".into());
            sub_lines.push(format!("  {} {} {}: {} -> {}", fmt_clock(e["t"].as_f64().unwrap_or(0.0)), if e["kind"] == "confirmed" { trs!("Wechsel", "substitution", "changement", "cambio") } else { trs!("möglicher Wechsel", "possible substitution", "changement possible", "posible cambio") }, e["team"].as_str().unwrap_or(""), n("out"), n("in")));
        }
    }
    let mut scene_lines = Vec::new();
    for f in &frames {
        for e in f["events"].as_array().into_iter().flatten().filter(|e| e["type"] == "scene") {
            scene_lines.push(tr!("  {} {} {} {} (Sicherheit {:.2}){}", "  {} {} {} {} (confidence {:.2}){}", "  {} {} {} {} (confiance {:.2}){}", "  {} {} {} {} (confianza {:.2}){}", fmt_clock(e["t"].as_f64().unwrap_or(0.0)), e["kind"].as_str().unwrap_or(""), e["team"].as_str().unwrap_or(""), e["side"].as_str().unwrap_or(""), e["confidence"].as_f64().unwrap_or(0.0), if e["borderline"] == true { trs!(" knapp an der Linie", " close to the line", " près de la ligne", " cerca de la línea") } else { "" }));
        }
    }
    if !scene_lines.is_empty() {
        println!("{}", tr!("Situationen: {} Einträge\n{}", "Situations: {} entries\n{}", "Situations : {} entrées\n{}", "Situaciones: {} entradas\n{}", scene_lines.len(), scene_lines.join("\n")));
    }
    println!("{}", tr!("Wechsel: {} Einträge{}", "Substitutions: {} entries{}", "Changements : {} entrées{}", "Cambios: {} entradas{}", sub_lines.len(), if sub_lines.is_empty() { String::new() } else { format!("\n{}", sub_lines.join("\n")) }));
    println!("{}", trs!("\nBewertung:", "\nAssessment:", "\nÉvaluation :", "\nValoración:"));
    let ok = |c: bool, m: &str| println!("  {} {m}", if c { "ok" } else { "!" });
    ok(f55 as f64 / n as f64 > 0.6, trs!("5+5 Spieler meist vollständig", "5+5 players mostly complete", "5+5 joueurs le plus souvent au complet", "5+5 jugadores casi siempre completos"));
    ok(ids.get("home").map(|s| s.len()).unwrap_or(0) <= 8 && ids.get("away").map(|s| s.len()).unwrap_or(0) <= 8, trs!("wenig Slot-Wechsel (IDs <= 8 je Team)", "few slot changes (IDs <= 8 per team)", "peu de changements de place (identifiants <= 8 par équipe)", "pocos cambios de puesto (IDs <= 8 por equipo)"));
    ok((jumps as f64) < 0.03 * speeds.len().max(1) as f64, trs!("wenige Schritte an der Tempogrenze (< 3%)", "few steps at the speed limit (< 3%)", "peu de pas à la vitesse limite (< 3%)", "pocos pasos en el límite de velocidad (< 3%)"));
    ok(f2 as f64 / n as f64 > 0.5, trs!("Schiris meist erkannt", "referees mostly found", "arbitres le plus souvent trouvés", "árbitros casi siempre detectados"));
    ok(g("unknown") < 40.0, trs!("Ball seltener als 40% unbekannt", "ball unknown less than 40% of the time", "ballon inconnu moins de 40% du temps", "balón desconocido menos del 40% del tiempo"));
}

// ---------------------------------------------------------------- scales

fn cmd_scales(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let mut o2 = Opts { flags: o.flags.clone(), pos: o.pos.clone() };
    if o.s("tile").is_none() || o.s("tile") == Some("auto") {
        o2.flags.insert("tile".into(), "512,768,1024,1536".into());
    }
    let tiles = plan(&o2, &info);
    let target = o.f("target-px", 28.0) as f32;
    let court = o.s("court").map(|p| CourtCalib::load(p).unwrap_or_else(|e| fail(e)));
    let field = court.as_ref().map(|c| c.profile().field).unwrap_or(Field::basketball());
    let projector = court.as_ref().and_then(|c| c.projector());
    eprintln!("{}", tr!("Messe Spielerhöhe über {} Kacheln (4 Maßstäbe) …", "Measuring player height over {} tiles (4 scales) …", "Mesure de la hauteur des joueurs sur {} tuiles (4 échelles) …", "Midiendo la altura de los jugadores en {} teselas (4 escalas) …", tiles.len()));
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for (i, frame) in sample_frames(o, input, &info).iter().enumerate() {
        let dets = model.detect(frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter().filter(|d| d.class == Class::Player && d.conf >= 0.6) {
            let (u, v) = d.foot();
            if let Some(p) = &projector {
                use reco_minimap::geom::Projector;
                if !p.to_court(u, v).map(|c| field.contains(c, 0.5)).unwrap_or(false) {
                    continue;
                }
            }
            pts.push((v * frame.h as f32, d.h * frame.h as f32));
        }
        eprint!("{}", tr!("\r  Stichprobe {}…  {} Messpunkte", "\r  sample {}…  {} measurements", "\r  échantillon {}…  {} mesures", "\r  muestra {}…  {} mediciones", i + 1, pts.len()));
    }
    eprintln!();
    if pts.len() < 20 {
        fail::<()>(tr!("nur {} Messpunkte – mehr --samples oder --court weglassen", "only {} measurements – use more --samples or leave out --court", "seulement {} mesures – augmenter --samples ou retirer --court", "solo {} mediciones – usa más --samples o quita --court", pts.len()));
    }
    // y range: court corners if known, else detection extents (3rd..99th percentile)
    let (mut y_min, mut y_max) = if let Some(c) = &court {
        let (lo, hi) = c.points.iter().fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.pano.y), a.1.max(p.pano.y)));
        (lo * info.h as f32, hi * info.h as f32)
    } else {
        let mut ys: Vec<f32> = pts.iter().map(|p| p.0).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (ys[ys.len() * 3 / 100], ys[(ys.len() * 99 / 100).min(ys.len() - 1)])
    };
    y_min = (y_min - 0.02 * info.h as f32).max(0.0);
    y_max = (y_max + 0.02 * info.h as f32).min(info.h as f32);
    let sc = Scales::fit(&pts, y_min, y_max, target).unwrap_or_else(|| fail(trs!("Anpassung fehlgeschlagen", "the fit failed", "l'ajustement a échoué", "el ajuste ha fallado")));
    println!("{}", tr!("\nSpielerhöhe im Panorama h(y) = {:.1} + {:.4}*y   ({} Messpunkte)", "\nPlayer height in the panorama h(y) = {:.1} + {:.4}*y   ({} measurements)", "\nHauteur des joueurs dans le panorama h(y) = {:.1} + {:.4}*y   ({} mesures)", "\nAltura de jugador en el panorama h(y) = {:.1} + {:.4}*y   ({} mediciones)", sc.a, sc.b, sc.n));
    println!("{}", tr!("Feld-Bereich y = {:.0} … {:.0} px; Ziel: Spieler {:.0} px hoch im Modelleingang", "Field rows y = {:.0} … {:.0} px; target: players {:.0} px tall in the model input", "Lignes du terrain y = {:.0} … {:.0} px ; cible : joueurs de {:.0} px de haut à l'entrée du modèle", "Filas del campo y = {:.0} … {:.0} px; objetivo: jugadores de {:.0} px de alto en la entrada del modelo", sc.y_min, sc.y_max, sc.target_px));
    for k in 0..=4 {
        let y = sc.y_min + (sc.y_max - sc.y_min) * k as f32 / 4.0;
        println!("{}", tr!("  y={y:>5.0}: Spieler ~{:>3.0} px -> Kachel {:>4.0} px", "  y={y:>5.0}: players ~{:>3.0} px -> tile {:>4.0} px", "  y={y:>5.0} : joueurs ~{:>3.0} px -> tuile {:>4.0} px", "  y={y:>5.0}: jugadores ~{:>3.0} px -> tesela {:>4.0} px", sc.height_at(y), sc.tile_at(y)));
    }
    let auto = plan_auto(&sc, info.w, info.h, o.f("overlap", 0.25) as f32);
    println!("{}", tr!("\n--tile auto = {} Kacheln pro Bild (statt {} im Raster).", "\n--tile auto = {} tiles per frame (instead of {} in the grid).", "\n--tile auto = {} tuiles par image (au lieu de {} en grille).", "\n--tile auto = {} teselas por imagen (en lugar de {} en la cuadrícula).", auto.len(), tiles.len()));
    let out = o.s("out").unwrap_or("scales.json");
    std::fs::write(out, serde_json::to_string_pretty(&sc.to_json()).unwrap()).unwrap_or_else(|e| fail(e));
    println!("{}", tr!("gespeichert: {out}  ->  danach z.B. `selfcheck … --tile auto`", "saved: {out}  ->  next e.g. `selfcheck … --tile auto`", "enregistré : {out}  ->  ensuite p. ex. `selfcheck … --tile auto`", "guardado: {out}  ->  después p. ej. `selfcheck … --tile auto`"));
}

// ---------------------------------------------------------------- colors

fn sample_frames(o: &Opts, input: &str, info: &VideoInfo) -> Vec<Frame> {
    let n = o.u("samples", 20).max(1);
    let start = t_start(o);
    let span = t_duration(o).or(info.duration.map(|d| d - start - 1.0)).unwrap_or(60.0).max(1.0);
    (0..n).filter_map(|s| video::grab(input, info, start + span * (s as f64 + 0.5) / n as f64).ok()).collect()
}

fn cmd_colors(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let court_loaded = o.s("court").map(|p| load_court(p));
    let field = court_loaded.as_ref().map(|c| c.1.field).unwrap_or(Field::basketball());
    let projector = court_loaded.as_ref().map(|c| c.0.projector().unwrap_or_else(|| fail(trs!("court.json: Homographie nicht lösbar", "court.json: the homography cannot be solved", "court.json : l'homographie ne peut pas être résolue", "court.json: no se puede resolver la homografía"))));
    if projector.is_none() {
        eprintln!("{}", trs!("Hinweis: ohne --court werden auch Zuschauer/Bank mitgezählt. Besser erst `calibrate` und dann --court court.json.", "Note: without --court, spectators and the bench are counted too. Better run `calibrate` first and then pass --court court.json.", "Remarque : sans --court, les spectateurs et le banc sont aussi comptés. Mieux vaut lancer `calibrate` puis passer --court court.json.", "Nota: sin --court también se cuentan espectadores y banquillo. Mejor ejecuta antes `calibrate` y pasa --court court.json."));
    }
    let mut samples: Vec<[u8; 3]> = Vec::new();
    let mut located: Vec<reco_minimap::tools::colors::PlayerSample> = Vec::new();
    for (i, frame) in sample_frames(o, input, &info).iter().enumerate() {
        let dets = model.detect(frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter().filter(|d| d.class == Class::Player) {
            let mut at = None;
            if let Some(p) = &projector {
                use reco_minimap::geom::Projector;
                let (u, v) = d.foot();
                match p.to_court(u, v) {
                    Some(c) if field.contains(c, 0.3) => at = Some(c),
                    _ => continue,
                }
            }
            if let Some(c) = frame.torso_color_in(d, torso_band(o)) {
                samples.push(c);
                if let Some(pos) = at {
                    located.push(reco_minimap::tools::colors::PlayerSample { frame: i, pos, rgb: c });
                }
            }
        }
        eprint!("{}", tr!("\r  Stichprobe {}…  {} Farbproben", "\r  sample {}…  {} colour samples", "\r  échantillon {}…  {} échantillons de couleur", "\r  muestra {}…  {} muestras de color", i + 1, samples.len()));
    }
    eprintln!();
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let swatch = |c: [u8; 3]| if tty { format!("\x1b[48;2;{};{};{}m      \x1b[0m  ", c[0], c[1], c[2]) } else { String::new() };
    if samples.len() < 6 {
        fail::<()>(trs!("zu wenige Farbproben – andere --samples/--roi oder --thr-player senken", "too few colour samples – change --samples/--roi or lower --thr-player", "trop peu d'échantillons de couleur – changer --samples/--roi ou baisser --thr-player", "muy pocas muestras de color – cambia --samples/--roi o baja --thr-player"));
    }
    let clusters = kmeans(&samples, 3);
    println!("{}", tr!("\nHäufigste Farbgruppen (Anteil an {} Proben):", "\nMost frequent colour groups (share of {} samples):", "\nGroupes de couleur les plus fréquents (part de {} échantillons) :", "\nGrupos de color más frecuentes (parte de {} muestras):", samples.len()));
    for (i, c) in clusters.iter().enumerate() {
        println!("  {}  {}{}   {:.0}%", i + 1, swatch(c.rgb), hex(c.rgb), 100.0 * c.count as f32 / samples.len() as f32);
    }
    let (ca, cb, dist) = pick_teams(&samples).unwrap_or_else(|| fail(trs!("zu wenige Farbproben für zwei Teams", "too few colour samples for two teams", "trop peu d'échantillons de couleur pour deux équipes", "muy pocas muestras de color para dos equipos")));
    println!("{}", trs!("\nZwei Teams (Zweiteilung, gleiche Farbe bei unterschiedlichem Licht bleibt ein Team):", "\nTwo teams (split in two; the same colour in different light stays one team):", "\nDeux équipes (partage en deux ; une même couleur sous des éclairages différents reste une équipe) :", "\nDos equipos (división en dos; el mismo color con distinta luz sigue siendo un equipo):"));
    println!("{}", tr!("  A: {}{}  ({} Proben)    B: {}{}  ({} Proben)    Abstand {:.0}", "  A: {}{}  ({} samples)    B: {}{}  ({} samples)    distance {:.0}", "  A : {}{}  ({} échantillons)    B : {}{}  ({} échantillons)    distance {:.0}", "  A: {}{}  ({} muestras)    B: {}{}  ({} muestras)    distancia {:.0}", swatch(ca.rgb), hex(ca.rgb), ca.count, swatch(cb.rgb), hex(cb.rgb), cb.count, dist));
    if dist < 10.0 {
        fail::<()>(tr!("die zwei Teams sind farblich praktisch nicht unterscheidbar (Abstand {dist:.0}); in der Oberfläche die Farben von Hand setzen", "the two teams can hardly be told apart by colour (distance {dist:.0}); set the colours by hand in the interface", "les deux équipes sont presque impossibles à distinguer par la couleur (distance {dist:.0}) ; régler les couleurs à la main dans l'interface", "los dos equipos apenas se distinguen por el color (distancia {dist:.0}); ajusta los colores a mano en la interfaz"));
    }
    if dist < 22.0 {
        println!("{}", tr!("  ! Die Farben sind sich ähnlich (Abstand {dist:.0}) – bitte prüfen und bei Bedarf von Hand anpassen.", "  ! The colours are similar (distance {dist:.0}) – please check and adjust by hand if needed.", "  ! Les couleurs sont proches (distance {dist:.0}) – à vérifier et à ajuster à la main si besoin.", "  ! Los colores se parecen (distancia {dist:.0}) – revísalos y ajústalos a mano si hace falta."));
    }
    let (a, b) = (ca.rgb, cb.rgb);
    let (home, away) = match o.s("home-hint").and_then(parse_hex) {
        Some(h) => if lab_dist(a, h) <= lab_dist(b, h) { (a, b) } else { (b, a) },
        None => (a, b),
    };
    let out = o.s("out").unwrap_or("teams.json");
    let mut v = serde_json::json!({"home": hex(home), "away": hex(away)});
    let keeper_sport = court_loaded.as_ref().map(|c| reco_minimap::sport::has_keepers(&c.1.id)).unwrap_or(false);
    if keeper_sport && !o.has("no-gk") {
        let pick = reco_minimap::tools::colors::pick_keepers(&located, home, away, &field);
        if pick.home.is_none() && pick.away.is_none() {
            println!("{}", trs!("\nTorwarttrikots: keine abweichende Farbe im Torraum gefunden (Torleute tragen vermutlich Teamfarbe, oder zu wenige Proben – mehr --samples). Von Hand setzen: --home-gk / --away-gk oder in der Oberfläche.", "\nGoalkeeper shirts: no deviating colour found in the goal areas (keepers probably wear the team colour, or too few samples – raise --samples). Set by hand: --home-gk / --away-gk or in the interface.", "\nMaillots de gardien : aucune couleur différente trouvée dans les surfaces (les gardiens portent sans doute la couleur de l'équipe, ou trop peu d'échantillons – augmenter --samples). À régler à la main : --home-gk / --away-gk ou dans l'interface.", "\nCamisetas de portero: no se encontró ningún color distinto en las áreas (probablemente llevan el color del equipo, o hay pocas muestras – sube --samples). A mano: --home-gk / --away-gk o en la interfaz."));
        } else {
            if let Some(c) = &pick.home {
                v["home_gk"] = serde_json::json!(hex(c.rgb));
                println!("{}", tr!("\nTorwart Heim: {}{}", "\nGoalkeeper home: {}{}", "\nGardien domicile : {}{}", "\nPortero local: {}{}", swatch(c.rgb), hex(c.rgb)));
            }
            if let Some(c) = &pick.away {
                v["away_gk"] = serde_json::json!(hex(c.rgb));
                println!("{}", tr!("Torwart Gast: {}{}", "Goalkeeper away: {}{}", "Gardien visiteurs : {}{}", "Portero visitante: {}{}", swatch(c.rgb), hex(c.rgb)));
            }
            if !pick.confident {
                println!("{}", trs!("  ! Welcher Torwart zu welchem Team gehört, ist unsicher – bitte prüfen und bei Bedarf tauschen (Oberfläche: Torwartfarben tauschen, Kommandozeile: --gk-swap beim Start).", "  ! Which keeper belongs to which team is uncertain – please check and swap if needed (interface: swap goalkeeper colours; command line: --gk-swap when starting).", "  ! L'attribution des gardiens aux équipes est incertaine – vérifier et échanger si besoin (interface : échanger les couleurs des gardiens ; ligne de commande : --gk-swap au lancement).", "  ! No es seguro qué portero es de qué equipo – revísalo y cámbialo si hace falta (interfaz: intercambiar colores de porteros; línea de comandos: --gk-swap al iniciar)."));
            }
        }
    }
    std::fs::write(out, serde_json::to_string_pretty(&v).unwrap()).unwrap_or_else(|e| fail(e));
    println!("{}", tr!("\nHeim {}  /  Gast {}  -> {out}", "\nHome {}  /  away {}  -> {out}", "\nDomicile {}  /  visiteurs {}  -> {out}", "\nLocal {}  /  visitante {}  -> {out}", hex(home), hex(away)));
    if o.s("home-hint").is_none() {
        println!("{}", tr!("(Heim/Gast sind nach Häufigkeit zugeordnet. Mit --home-hint #rrggbb festlegen, welche Farbe 'Heim' ist, oder in {out} tauschen.)", "(Home/away are assigned by frequency. Use --home-hint #rrggbb to say which colour is 'home', or swap them in {out}.)", "(Domicile/visiteurs sont attribués selon la fréquence. Utiliser --home-hint #rrggbb pour indiquer la couleur « domicile », ou les échanger dans {out}.)", "(Local/visitante se asignan por frecuencia. Usa --home-hint #rrggbb para indicar qué color es 'local', o intercámbialos en {out}.)"));
    }
}

// ---------------------------------------------------------------- numbers + events

struct Numbers {
    reader: NumberReader,
    min_h: f32,
    max_reads: usize,
    tick: u64,
    tries: usize,
    accepted: usize,
    /// Heights (px in the OCR source) of the players seen, to judge whether the source is big enough.
    heights: Vec<f32>,
}

/// `--numbers`: load the OCR model and the roster options into the config. Returns the reader if enabled.
fn numbers_setup(o: &Opts, mcfg: &mut Config) -> Option<Numbers> {
    mcfg.roster_home = o.s("roster-home").map(parse_roster).unwrap_or_default();
    mcfg.roster_away = o.s("roster-away").map(parse_roster).unwrap_or_default();
    // goalkeepers are marked in the roster: "1g, 4, 5, 12tw"
    mcfg.keepers_home = o.s("roster-home").map(reco_minimap::jersey::parse_roster_keepers).unwrap_or_default();
    mcfg.keepers_away = o.s("roster-away").map(reco_minimap::jersey::parse_roster_keepers).unwrap_or_default();
    if !o.has("numbers") {
        return None;
    }
    let path = o.s("ocr-model").map(std::path::PathBuf::from).unwrap_or_else(ocr::default_model_path);
    let reader = NumberReader::load(&path).unwrap_or_else(|e| fail(e));
    eprintln!("{}", tr!("Trikotnummern: an (Kader Heim {:?}, Gast {:?}; leer = freie Erkennung)", "Jersey numbers: on (roster home {:?}, away {:?}; empty = free reading)", "Numéros de maillot : activés (effectif domicile {:?}, visiteurs {:?} ; vide = lecture libre)", "Dorsales: activados (plantilla local {:?}, visitante {:?}; vacío = lectura libre)", mcfg.roster_home, mcfg.roster_away));
    Some(Numbers { reader, min_h: o.f("ocr-min-h", 45.0) as f32, max_reads: o.u("ocr-max", 4), tick: 0, tries: 0, accepted: 0, heights: Vec::new() })
}

/// Try to read numbers of the players seen in this frame. `src` is the frame to crop from (a higher-resolution
/// version of the panorama if available; bbox coordinates are normalized, so resolution does not matter).
fn read_numbers(nm: &mut Numbers, mm: &mut Minimap, src: &Frame, t: f64) {
    nm.tick += 1;
    let mut cands: Vec<(u32, reco_minimap::tracker::BBox, f32, bool)> = mm
        .tracker()
        .tracks()
        .iter()
        .filter(|x| !x.referee && x.state == reco_minimap::tracker::TrackState::Visible && (x.last_seen - t).abs() < 1e-6 && x.bbox.is_some())
        .map(|x| (x.id, x.bbox.unwrap(), x.bbox.unwrap().h * src.h as f32, x.confirmed.is_some()))
        .collect();
    if nm.heights.len() < 5000 {
        nm.heights.extend(cands.iter().map(|c| c.2));
    }
    cands.retain(|c| c.2 >= nm.min_h);
    // unknown numbers first, then the biggest (= best readable) boxes
    cands.sort_by(|a, b| a.3.cmp(&b.3).then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)));
    for (id, bb, _, known) in cands.into_iter().take(nm.max_reads) {
        if known && (nm.tick + id as u64) % 12 != 0 {
            continue; // known numbers are only re-checked now and then (this also notices a substitute)
        }
        let (x0, y0, x1, y1) = ocr::torso_rect(src, &bb);
        let (w, h, rgb) = src.crop_rgb(x0, y0, x1, y1);
        if w < 6 || h < 6 {
            continue;
        }
        nm.tries += 1;
        if let Ok(r) = nm.reader.read(&ocr::localize(w, h, &rgb)) {
            if mm.add_number_probs(id, &r.probs, r.steps).is_some() {
                nm.accepted += 1;
            }
        }
    }
}

fn scene_json(e: &SceneEvent) -> serde_json::Value {
    serde_json::json!({"type": "scene", "kind": e.kind.key(), "team": match e.team { Some(Team::Home) => "home", Some(Team::Away) => "away", _ => "none" },
        "side": e.side.map(|c| c.to_string()), "x": (e.pos.x * 100.0).round() / 100.0, "y": (e.pos.y * 100.0).round() / 100.0,
        "confidence": (e.confidence * 100.0).round() / 100.0, "borderline": e.borderline, "t": (e.t * 1000.0).round() / 1000.0,
        "started_at":e.started_at,"evidence":e.evidence,"phase":if e.kind==SceneKind::FreeThrow && e.evidence.contains(&"ball_release_towards_basket") {"attempt"} else if matches!(e.kind,SceneKind::ShotTwo|SceneKind::ShotThree) {"attempt"} else {"formation"},
        "related_substitutions":e.related_substitutions.iter().map(|sub|serde_json::json!({"t":sub.t,"slot":sub.slot,"team":match sub.team {Team::Home=>"home",Team::Away=>"away",Team::Unknown=>"unknown"},"status":match sub.kind {reco_minimap::SubKind::Possible=>"possible",reco_minimap::SubKind::Confirmed=>"confirmed"},"out_number":sub.out_number,"in_number":sub.in_number})).collect::<Vec<_>>()})
}

fn scene_name(kind: SceneKind, sport: &str) -> &'static str {
    match (kind, sport) {
        (SceneKind::TipOff, _) => trs!("Sprungball/Startball", "tip-off", "entre-deux", "salto inicial"),
        (SceneKind::Kickoff, "handball") => trs!("Anwurf", "throw-off", "engagement", "saque de centro"),
        (SceneKind::Kickoff, _) => trs!("Anstoß", "kick-off", "coup d'envoi", "saque inicial"),
        (SceneKind::FreeThrow, _) => trs!("Freiwurf", "free throw", "lancer franc", "tiro libre"),
        (SceneKind::ThrowIn, "basketball") => trs!("Einwurf (Inbound)", "throw-in (inbound)", "remise en jeu", "saque de banda"),
        (SceneKind::ThrowIn, _) => trs!("Einwurf", "throw-in", "touche", "saque de banda"),
        (SceneKind::Corner, _) => trs!("Ecke", "corner", "corner", "córner"),
        (SceneKind::Penalty, _) => trs!("Elfmeter/Strafstoß", "penalty", "penalty", "penalti"),
        (SceneKind::SevenMeter, _) => trs!("7-Meter", "7 m throw", "jet de 7 m", "lanzamiento de 7 m"),
        (SceneKind::ShotTwo, _) => trs!("Wurfversuch 2 Punkte", "2-point attempt", "tir à 2 points", "tiro de 2 puntos"),
        (SceneKind::ShotThree, _) => trs!("Wurfversuch 3 Punkte", "3-point attempt", "tir à 3 points", "tiro de 3 puntos"),
    }
}

fn log_scenes(sc: &[SceneEvent], sport: &str) {
    for e in sc {
        let side = match e.side { Some('l') => trs!(" links", " left", " à gauche", " izquierda"), Some('r') => trs!(" rechts", " right", " à droite", " derecha"), _ => "" };
        let team = match e.team { Some(t) => format!(" ({})", team_name(t)), None => String::new() };
        eprintln!("{}", tr!("\n  SITUATION {}{}{} bei {}  (Sicherheit {:.2}{})", "\n  SITUATION {}{}{} at {}  (confidence {:.2}{})", "\n  SITUATION {}{}{} à {}  (confiance {:.2}{})", "\n  SITUACIÓN {}{}{} en {}  (confianza {:.2}{})", scene_label(e, sport), side, team, fmt_clock(e.t), e.confidence, if e.borderline { trs!(", knapp an der Linie", ", close to the line", ", près de la ligne", ", cerca de la línea") } else { "" }));
    }
}

fn scene_label(e:&SceneEvent,sport:&str)-> &'static str {
    if e.kind==SceneKind::FreeThrow {
        if e.evidence.contains(&"ball_release_towards_basket") { trs!("Freiwurfversuch", "free-throw attempt", "tentative de lancer franc", "intento de tiro libre") } else { trs!("Freiwurfaufstellung", "free-throw line-up", "alignement de lancer franc", "formación de tiro libre") }
    } else {scene_name(e.kind,sport)}
}

fn fmt_clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn team_name(t: Team) -> &'static str {
    match t { Team::Home => trs!("Heim", "home", "domicile", "local"), Team::Away => trs!("Gast", "away", "visiteurs", "visitante"), Team::Unknown => "?" }
}

fn log_events(events: &[reco_minimap::SubEvent]) {
    let num = |n: Option<u8>| n.map(|x| format!("#{x}")).unwrap_or_else(|| "?".into());
    for e in events {
        match e.kind {
            reco_minimap::SubKind::Confirmed => eprintln!("{}", tr!("\n  WECHSEL {} bei {}: {} -> {} (Platz {})", "\n  SUBSTITUTION {} at {}: {} -> {} (slot {})", "\n  CHANGEMENT {} à {} : {} -> {} (place {})", "\n  CAMBIO {} en {}: {} -> {} (puesto {})", team_name(e.team), fmt_clock(e.t), num(e.out_number), num(e.in_number), e.slot)),
            reco_minimap::SubKind::Possible => eprintln!("{}", tr!("\n  möglicher Wechsel {} bei {}: {} verlässt das Feld, jemand kommt am Rand (Platz {})", "\n  possible substitution {} at {}: {} leaves the field, someone enters at the sideline (slot {})", "\n  changement possible {} à {} : {} quitte le terrain, quelqu'un entre par la touche (place {})", "\n  posible cambio {} en {}: {} sale del campo, alguien entra por la banda (puesto {})", team_name(e.team), fmt_clock(e.t), num(e.out_number), e.slot)),
        }
    }
}

fn numbers_summary(nm: &Numbers, mm: &Minimap) {
    let tr = mm.tracker().tracks();
    let known = |team: Team| tr.iter().filter(|x| !x.referee && x.team == team && x.confirmed.is_some()).count();
    let list = |team: Team| tr.iter().filter(|x| !x.referee && x.team == team).map(|x| x.confirmed.map(|n| format!("#{n}")).unwrap_or_else(|| "?".into())).collect::<Vec<_>>().join(" ");
    eprintln!("{}", tr!("Trikotnummern: {} Leseversuche, {} angenommen; bestätigt Heim {} / Gast {} ({} | {})", "Jersey numbers: {} reading attempts, {} accepted; confirmed home {} / away {} ({} | {})", "Numéros de maillot : {} tentatives de lecture, {} acceptées ; confirmés domicile {} / visiteurs {} ({} | {})", "Dorsales: {} intentos de lectura, {} aceptados; confirmados local {} / visitante {} ({} | {})", nm.tries, nm.accepted, known(Team::Home), known(Team::Away), list(Team::Home), list(Team::Away)));
    let mut hs = nm.heights.clone();
    if let Some(med) = median(&mut hs) {
        let q75 = { hs.sort_by(|a, b| a.partial_cmp(b).unwrap()); hs[(hs.len() * 3 / 4).min(hs.len() - 1)] };
        eprintln!("{}", tr!("Spielerhöhe in der Nummern-Quelle: Median {med:.0} px, obere 25 % ab {q75:.0} px -> Ziffern etwa {:.0} px bzw. {:.0} px hoch (lesbar ab ca. 10 px)", "Player height in the number source: median {med:.0} px, top 25 % from {q75:.0} px -> digits about {:.0} px and {:.0} px tall (readable from about 10 px)", "Hauteur des joueurs dans la source des numéros : médiane {med:.0} px, les 25 % les plus grands à partir de {q75:.0} px -> chiffres d'environ {:.0} px et {:.0} px de haut (lisibles à partir d'environ 10 px)", "Altura de jugador en la fuente de dorsales: mediana {med:.0} px, 25 % superior desde {q75:.0} px -> cifras de unos {:.0} px y {:.0} px de alto (legibles desde unos 10 px)", med * 0.12, q75 * 0.12));
        if q75 * 0.12 < 9.0 {
            eprintln!("{}", trs!("  ! Auch die nahen Spieler sind für Nummern zu klein: ein höher aufgelöstes Panorama mit --hires verwenden (Spieler ab ca. 80 px Höhe).", "  ! Even the near players are too small for numbers: use a higher-resolution panorama with --hires (players of about 80 px or more).", "  ! Même les joueurs proches sont trop petits pour les numéros : utiliser un panorama de plus haute résolution avec --hires (joueurs d'environ 80 px ou plus).", "  ! Incluso los jugadores cercanos son demasiado pequeños para los dorsales: usa un panorama de mayor resolución con --hires (jugadores de unos 80 px o más)."));
        } else if med * 0.12 < 9.0 {
            eprintln!("{}", trs!("  ! Nur die nahen Spieler sind groß genug; Nummern der entfernten Spieler kommen nur, wenn sie nah an der Kamera vorbeilaufen (oder mit --hires).", "  ! Only the near players are large enough; numbers of far players only come when they pass close to the camera (or with --hires).", "  ! Seuls les joueurs proches sont assez grands ; les numéros des joueurs éloignés n'apparaissent que lorsqu'ils passent près de la caméra (ou avec --hires).", "  ! Solo los jugadores cercanos son lo bastante grandes; los dorsales de los lejanos solo salen cuando pasan cerca de la cámara (o con --hires)."));
        }
    }
    let ev = mm.events_log();
    eprintln!("{}", tr!("Wechsel: {} bestätigt, {} möglich", "Substitutions: {} confirmed, {} possible", "Changements : {} confirmés, {} possibles", "Cambios: {} confirmados, {} posibles", ev.iter().filter(|e| e.kind == reco_minimap::SubKind::Confirmed).count(), ev.iter().filter(|e| e.kind == reco_minimap::SubKind::Possible).count()));
}

// ---------------------------------------------------------------- live

struct Latest {
    slot: std::sync::Mutex<Option<(Instant, Frame)>>,
    cv: std::sync::Condvar,
    eof: std::sync::atomic::AtomicBool,
    dropped: std::sync::atomic::AtomicU64,
}

fn cmd_live(o: &Opts, input: &str) {
    use reco_minimap::tools::async_ocr::{BackgroundOcr, Identity, Job};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    let info = open_info(o, input);
    if info.w == 0 {
        fail::<()>(trs!("--size WxH angeben, z.B. --size 1920x1080 (Live-Quellen werden nicht vorab geprüft)", "pass --size WxH, e.g. --size 1920x1080 (live sources are not probed beforehand)", "indiquer --size LxH, p. ex. --size 1920x1080 (les sources en direct ne sont pas analysées au préalable)", "indica --size AnxAl, p. ej. --size 1920x1080 (las fuentes en directo no se analizan antes)"));
    }
    let mut model = load_model(o);
    model.cfg.thr_player = thr_players(o).0;
    let tiles = plan(o, &info);
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail(trs!("--court court.json fehlt", "--court court.json is missing", "--court court.json manquant", "falta --court court.json"))));
    let projector = cal.projector().unwrap_or_else(|| fail(trs!("court.json: Homographie nicht lösbar", "court.json: the homography cannot be solved", "court.json : l'homographie ne peut pas être résolue", "court.json: no se puede resolver la homografía")));
    let teams = read_teams(o);
    let (home, away) = (teams.home, teams.away);
    // shirt colours the torso sample may snap to (teams, and the referees' if given)
    let shirts: Vec<[u8; 3]> = teams.shirts(o.s("ref-color").and_then(parse_hex));
    let torso = torso_band(o);
    let shirt_match = o.has("shirt-match");
    let mut mcfg = game_config(o, &profile, home, away);
    mcfg.referee_rgb = o.s("ref-color").and_then(parse_hex);
    teams.apply(&mut mcfg);
    mcfg.tracker.spawn_conf = { let (low, high) = thr_players(o); if low < high { high } else { 0.0 } };
    let mut numbers = numbers_setup(o, &mut mcfg).map(|nm| BackgroundOcr::new(nm.reader,nm.min_h,nm.max_reads));
    let mut scenes = o.has("scenes").then(|| SceneDetector::new(profile.clone()));
    let mut mm = Minimap::new(mcfg, Box::new(projector));

    let draw_home = o.s("draw-home").and_then(parse_hex).unwrap_or(home);
    let draw_away = o.s("draw-away").and_then(parse_hex).unwrap_or(away);
    let hub = Hub::new(serde_json::json!({"home": hex(draw_home), "away": hex(draw_away), "referee": "#000000", "ball": "#ff8c00", "floor": "#262c34", "linecolor": "#e1e4e8",
        "sport": profile.id, "numbers": o.has("numbers"), "field": [profile.field.length, profile.field.width], "lines": profile.lines.iter().map(|pl| pl.iter().map(|p| [(p.x * 100.0).round() / 100.0, (p.y * 100.0).round() / 100.0]).collect::<Vec<_>>()).collect::<Vec<_>>()}).to_string());
    let host = o.s("host").unwrap_or("127.0.0.1").to_string();
    let addr = live::serve(hub.clone(), &host, o.u("port", 8765) as u16).unwrap_or_else(|e| fail(e));
    eprintln!("{}", tr!("\nLive-Overlay: http://{addr}/", "\nLive overlay: http://{addr}/", "\nIncrustation en direct : http://{addr}/", "\nSuperposición en directo: http://{addr}/"));
    eprintln!("{}", trs!("  OBS: Quelle hinzufügen -> Browser -> URL oben, Breite 1400 / Höhe 800 (oder passend), Hintergrund bleibt transparent.", "  OBS: add a source -> Browser -> the URL above, width 1400 / height 800 (or as needed); the background stays transparent.", "  OBS : ajouter une source -> Navigateur -> l'URL ci-dessus, largeur 1400 / hauteur 800 (ou selon besoin) ; le fond reste transparent.", "  OBS: añade una fuente -> Navegador -> la URL de arriba, ancho 1400 / alto 800 (o lo que convenga); el fondo queda transparente."));
    eprintln!("{}", tr!("  Zum Testen im Browser: http://{addr}/?floor=1&debug=1      Daten: http://{addr}/state.json\n", "  To test in a browser: http://{addr}/?floor=1&debug=1      data: http://{addr}/state.json\n", "  Pour tester dans un navigateur : http://{addr}/?floor=1&debug=1      données : http://{addr}/state.json\n", "  Para probar en un navegador: http://{addr}/?floor=1&debug=1      datos: http://{addr}/state.json\n"));

    let mut jsonl = o.s("json").map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    let reader = FrameReader::open_with(input, &info, t_start(o), t_duration(o), 1, true, o.has("realtime"), &o.s("in-args").map(|a| a.split_whitespace().map(|x| x.to_string()).collect::<Vec<_>>()).unwrap_or_default()).unwrap_or_else(|e| fail(e));
    // Set the clock before the producer starts, so its first arrival never
    // precedes the origin. Queue capacity remains exactly one newest frame.
    let t0 = Instant::now();
    let latest = Arc::new(Latest { slot: Default::default(), cv: Default::default(), eof: Default::default(), dropped: Default::default() });
    {
        let latest = latest.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            while let Some(f) = reader.next() {
                let mut g = latest.slot.lock().unwrap();
                if g.replace((Instant::now(), f)).is_some() {
                    latest.dropped.fetch_add(1, Ordering::Relaxed); // detector too slow: keep only the newest frame
                }
                latest.cv.notify_one();
            }
            latest.eof.store(true, Ordering::SeqCst);
            latest.cv.notify_all();
        });
    }

    let mut scheduler = reco_minimap::scheduler::TileScheduler::new(o.f("sweep-secs",2.0));
    let mut ball_search = reco_minimap::scheduler::RecoverySweep::default();
    let mut ball_boxes: Vec<reco_minimap::tracker::BBox> = Vec::new();
    let mut ball_seen = f64::NEG_INFINITY;
    let mut person_boxes: Vec<reco_minimap::tracker::BBox> = Vec::new();
    let mut persons_seen = f64::NEG_INFINITY;
    let mut last_ocr = f64::NEG_INFINITY;
    let (mut ocr_applied,mut ocr_stale,mut ocr_errors)=(0usize,0usize,0usize);
    let ocr_interval = o.f("ocr-secs",2.0);
    if !ocr_interval.is_finite() || ocr_interval < 0.0 { fail::<()>(trs!("--ocr-secs muss endlich und >= 0 sein", "--ocr-secs must be finite and >= 0", "--ocr-secs doit être fini et >= 0", "--ocr-secs debe ser finito y >= 0")); }
    let (mut n, mut last_report, mut sum_age) = (0u64, Instant::now(), 0f32);
    loop {
        let (arrival, mut frame) = {
            let mut g = latest.slot.lock().unwrap();
            loop {
                if let Some(x) = g.take() {
                    break x;
                }
                if latest.eof.load(Ordering::SeqCst) {
                    eprintln!("{}", trs!("\nQuelle beendet.", "\nThe source has ended.", "\nLa source est terminée.", "\nLa fuente ha terminado."));
                    return;
                }
                g = latest.cv.wait_timeout(g, std::time::Duration::from_millis(500)).unwrap().0;
            }
        };
        let t = arrival.duration_since(t0).as_secs_f64();
        // Search around ALL detections, including unknown teams and people not
        // assigned a tracker slot. Identity uncertainty must not hide a player
        // from the next detector pass.
        let mut targets = if t-persons_seen <= 1.0 { person_boxes.clone() } else { Vec::new() };
        if t-ball_seen <= 0.5 { targets.extend_from_slice(&ball_boxes); }
        let (mut indices,full_sweep) = if o.has("adaptive-live") { scheduler.select(t,&tiles,info.w,info.h,&targets) } else { ((0..tiles.len()).collect(),true) };
        let person_tile_count=indices.len();
        // Ball recovery must not depend solely on player/previous-ball boxes.
        // Rotate through unobserved areas using the same calibrated scales and
        // parallel inference pool, then merge once on this frame's timestamp.
        if !full_sweep {
            indices.extend(ball_search.select(tiles.len(),o.u("ball-search-tiles",4)));
            indices.sort_unstable(); indices.dedup();
        }
        let recovery_tiles=indices.len()-person_tile_count;
        let selected: Vec<_> = indices.iter().map(|&i|tiles[i]).collect();
        let inference_start = Instant::now();
        let mut dets = model.detect(&frame, &selected).unwrap_or_else(|e| fail(e));
        let inference_ms = inference_start.elapsed().as_secs_f64()*1000.0;
        person_boxes = dets.iter().filter(|d|d.class!=Class::Ball).map(|d|reco_minimap::tracker::BBox{cx:d.cx,cy:d.cy,w:d.w,h:d.h}).collect();
        if !person_boxes.is_empty() { persons_seen=t; }
        ball_boxes = dets.iter().filter(|d|d.class==Class::Ball).map(|d|reco_minimap::tracker::BBox{cx:d.cx,cy:d.cy,w:d.w,h:d.h}).collect();
        if !ball_boxes.is_empty() { ball_seen=t; }
        for d in dets.iter_mut().filter(|d| d.class == Class::Player) {
            d.color = if shirt_match { frame.torso_color_matching(d, torso, &shirts) } else { frame.torso_color_in(d, torso) };
        }
        // Apply completed OCR before building this frame's public identity state.
        // No mutable tracker references are sent to the worker.
        if let Some(result)=numbers.as_mut().and_then(|nm|nm.poll()) {
            ocr_errors+=result.errors;
            for (key,reading) in result.readings {
                if mm.tracker().tracks().iter().any(|tr|key.matches(tr,t)) {
                    if mm.add_number_probs(key.id,&reading.probs,reading.steps).is_some() { ocr_applied+=1; }
                } else { ocr_stale+=1; }
            }
        }
        let state = mm.process(t, &dets);
        if let Some(nm) = numbers.as_mut() {
            if nm.ready() && t-last_ocr >= ocr_interval {
                let mut candidates: Vec<_>=mm.tracker().tracks().iter().filter(|tr|!tr.referee && tr.state==reco_minimap::tracker::TrackState::Visible && (tr.last_seen-t).abs()<1e-6)
                    .filter_map(|tr|tr.bbox.map(|bb|(tr,bb))).filter(|(_,bb)|bb.h*frame.h as f32>=nm.min_h).collect();
                candidates.sort_by(|(a,ab),(b,bb)|a.confirmed.is_some().cmp(&b.confirmed.is_some()).then(bb.h.total_cmp(&ab.h)));
                let jobs=candidates.into_iter().take(nm.max_reads).filter_map(|(tr,bb)| {
                    let (x0,y0,x1,y1)=ocr::torso_rect(&frame,&bb);
                    let (w,h,rgb)=frame.crop_rgb(x0,y0,x1,y1);
                    (w>=6 && h>=6).then_some(Job{identity:Identity{id:tr.id,segment:tr.segment,team:tr.team,observed_at:t},crop:ocr::Crop{w,h,rgb}})
                }).collect();
                if nm.submit(jobs) { last_ocr=t; }
            }
        }
        log_events(&state.events);
        let sc = scenes.as_mut().map(|d| d.update(&state)).unwrap_or_default();
        log_scenes(&sc, &profile.id);
        frame.rgb = Vec::new(); // free the big buffer early
        let age_ms = arrival.elapsed().as_secs_f64() * 1000.0;
        let mut v = state_json_with(&state, &sc);
        v["age_ms"] = serde_json::json!((age_ms * 10.0).round() / 10.0);
        v["ocr"] = serde_json::json!({"background":numbers.is_some(),"pending":numbers.as_ref().map(|n|!n.ready()).unwrap_or(false),"applied":ocr_applied,"stale":ocr_stale,"errors":ocr_errors});
        v["performance"] = serde_json::json!({"tiles":selected.len(),"total_tiles":tiles.len(),"full_sweep":full_sweep,"inference_ms":inference_ms,"dropped_frames":latest.dropped.load(Ordering::Relaxed),"queue_capacity":1,"age_scope":"decoded_frame_to_result","recovery_tiles":recovery_tiles,"workers":model.workers(),
            "detected_players":dets.iter().filter(|d|d.class==Class::Player).count(),"detected_balls":ball_boxes.len()});
        let line = v.to_string();
        if let Some(j) = jsonl.as_mut() {
            let _ = writeln!(j, "{line}");
            let _ = j.flush(); // live log: keep it readable while running
        }
        hub.publish(line);
        n += 1;
        sum_age += age_ms as f32;
        if last_report.elapsed().as_secs_f32() > 3.0 {
            let el = last_report.elapsed().as_secs_f32();
            eprint!("{}", tr!("\r  {:.1} Updates/s · Latenz Bild->Ergebnis {:.0} ms · verworfene Bilder {} · {} Spieler, {} Schiris   ", "\r  {:.1} updates/s · latency frame->result {:.0} ms · dropped frames {} · {} players, {} referees   ", "\r  {:.1} mises à jour/s · latence image->résultat {:.0} ms · images ignorées {} · {} joueurs, {} arbitres   ", "\r  {:.1} actualizaciones/s · latencia imagen->resultado {:.0} ms · imágenes descartadas {} · {} jugadores, {} árbitros   ", n as f32 / el, sum_age / n.max(1) as f32, latest.dropped.load(Ordering::Relaxed), state.players.len(), state.referees.len()));
            last_report = Instant::now();
            n = 0;
            sum_age = 0.0;
        }
    }
}

// ---------------------------------------------------------------- run

/// Shirt colours of both teams and, optionally, of their goalkeepers (`home_gk` / `away_gk` in teams.json,
/// or `--home-gk` / `--away-gk`; `--no-gk` ignores them, `--gk-swap` exchanges the two keeper colours).
#[derive(Clone, Copy, Debug)]
struct Teams {
    home: [u8; 3],
    away: [u8; 3],
    home_gk: Option<[u8; 3]>,
    away_gk: Option<[u8; 3]>,
}

impl Teams {
    fn apply(&self, cfg: &mut Config) {
        cfg.home_gk_rgb = self.home_gk;
        cfg.away_gk_rgb = self.away_gk;
    }
    /// Colours a torso sample may snap to with `--shirt-match`.
    fn shirts(&self, referee: Option<[u8; 3]>) -> Vec<[u8; 3]> {
        [Some(self.home), Some(self.away), self.home_gk, self.away_gk, referee].into_iter().flatten().collect()
    }
}

fn read_teams(o: &Opts) -> Teams {
    let mut t = None;
    if let (Some(h), Some(a)) = (o.s("home").and_then(parse_hex), o.s("away").and_then(parse_hex)) {
        t = Some(Teams { home: h, away: a, home_gk: None, away_gk: None });
    } else if let Some(p) = o.s("teams") {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))).unwrap_or_else(|e| fail(format!("{p}: {e}")));
        if let (Some(h), Some(a)) = (v["home"].as_str().and_then(parse_hex), v["away"].as_str().and_then(parse_hex)) {
            t = Some(Teams { home: h, away: a, home_gk: v["home_gk"].as_str().and_then(parse_hex), away_gk: v["away_gk"].as_str().and_then(parse_hex) });
        }
    }
    let mut t = t.unwrap_or_else(|| {
        fail(trs!("Trikotfarben fehlen: --teams teams.json oder --home #rrggbb --away #rrggbb (Farben ermitteln: `reco-minimap colors`)", "Shirt colours are missing: --teams teams.json or --home #rrggbb --away #rrggbb (detect them with `reco-minimap colors`)", "Couleurs de maillot manquantes : --teams teams.json ou --home #rrggbb --away #rrggbb (les détecter avec `reco-minimap colors`)", "Faltan los colores de camiseta: --teams teams.json o --home #rrggbb --away #rrggbb (detéctalos con `reco-minimap colors`)"))
    });
    if let Some(c) = o.s("home-gk").and_then(parse_hex) {
        t.home_gk = Some(c);
    }
    if let Some(c) = o.s("away-gk").and_then(parse_hex) {
        t.away_gk = Some(c);
    }
    if o.has("no-gk") {
        t.home_gk = None;
        t.away_gk = None;
    }
    if o.has("gk-swap") {
        std::mem::swap(&mut t.home_gk, &mut t.away_gk);
    }
    t
}

fn ball_json(s: &FrameState) -> serde_json::Value {
    let (state, tr) = match s.ball.state {
        BallState::Held { track } => ("held", Some(track)),
        BallState::InFlight => ("flight", None),
        BallState::Loose => ("loose", None),
        BallState::Unknown => ("unknown", None),
    };
    serde_json::json!({"state": state, "holder": tr, "x": s.ball.pos.map(|p| p.x), "y": s.ball.pos.map(|p| p.y), "vx": s.ball.vel.map(|v| v.x), "vy": s.ball.vel.map(|v| v.y)})
}

fn state_json_with(s: &FrameState, scenes: &[SceneEvent]) -> serde_json::Value {
    let agent = |a: &reco_minimap::AgentOut| {
        serde_json::json!({"id": a.id, "segment": a.segment,
            "identity": {"status": a.identity.status.key(), "player": a.identity.player.map(|p| serde_json::json!({"team": match p.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" }, "number": p.number}))},
            "team": match a.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" },
            "x": (a.pos.x * 100.0).round() / 100.0, "y": (a.pos.y * 100.0).round() / 100.0, "vx": (a.vel.x * 100.0).round() / 100.0, "vy": (a.vel.y * 100.0).round() / 100.0, "state": format!("{:?}", a.state).to_lowercase(), "alpha": (a.alpha * 100.0).round() / 100.0, "jersey": a.jersey, "keeper": a.keeper})
    };
    serde_json::json!({"t": (s.t * 1000.0).round() / 1000.0, "players": s.players.iter().map(agent).collect::<Vec<_>>(), "referees": s.referees.iter().map(agent).collect::<Vec<_>>(), "ball": ball_json(s),
        "events": s.events.iter().map(|e| serde_json::json!({"type": "sub", "kind": if e.kind == reco_minimap::SubKind::Confirmed { "confirmed" } else { "possible" }, "team": match e.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" }, "slot": e.slot, "out": e.out_number, "in": e.in_number, "t": (e.t * 1000.0).round() / 1000.0})).chain(scenes.iter().map(scene_json)).collect::<Vec<_>>()})
}

fn cmd_run(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    model.cfg.thr_player = thr_players(o).0;
    let tiles = plan(o, &info);
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail(trs!("--court court.json fehlt (erst `reco-minimap calibrate`)", "--court court.json is missing (run `reco-minimap calibrate` first)", "--court court.json manquant (lancer d'abord `reco-minimap calibrate`)", "falta --court court.json (ejecuta antes `reco-minimap calibrate`)"))));
    if cal.pano_w > 0 && cal.pano_h > 0 {
        let (a, b) = (cal.pano_w as f32 / cal.pano_h as f32, info.w as f32 / info.h as f32);
        if (a - b).abs() / a > 0.02 {
            eprintln!("{}", tr!("Warnung: court.json wurde für {}x{} erstellt, das Video ist {}x{} (anderes Seitenverhältnis) – neu kalibrieren!", "Warning: court.json was made for {}x{}, the video is {}x{} (different aspect ratio) – calibrate again!", "Attention : court.json a été créé pour {}x{}, la vidéo fait {}x{} (autre rapport d'aspect) – recalibrer !", "Aviso: court.json se creó para {}x{}, el vídeo es {}x{} (otra relación de aspecto) – ¡vuelve a calibrar!", cal.pano_w, cal.pano_h, info.w, info.h));
        }
    }
    let projector = cal.projector().unwrap_or_else(|| fail(trs!("court.json: Homographie nicht lösbar", "court.json: the homography cannot be solved", "court.json : l'homographie ne peut pas être résolue", "court.json: no se puede resolver la homografía")));
    let teams = read_teams(o);
    let (home, away) = (teams.home, teams.away);
    // shirt colours the torso sample may snap to (teams, and the referees' if given)
    let shirts: Vec<[u8; 3]> = teams.shirts(o.s("ref-color").and_then(parse_hex));
    let torso = torso_band(o);
    let shirt_match = o.has("shirt-match");
    eprintln!("{}", tr!("Heim {}  Gast {}  | Kalibrierfehler {:.2} m", "Home {}  away {}  | calibration error {:.2} m", "Domicile {}  visiteurs {}  | erreur de calibrage {:.2} m", "Local {}  visitante {}  | error de calibración {:.2} m", hex(home), hex(away), cal.rms_m));

    let every = o.u("every", 1).max(1);
    let out_fps = info.fps / every as f64;
    // minimap/alpha are rendered at the full source rate by interpolating between detections
    let smooth = every > 1 && !o.has("no-smooth");
    let map_fps = if smooth { info.fps } else { out_fps };
    let ppm = o.s("scale").and_then(|v| v.parse().ok()).unwrap_or_else(|| Renderer::default_px_per_m(&profile));
    let draw_home = o.s("draw-home").and_then(parse_hex).unwrap_or(home);
    let draw_away = o.s("draw-away").and_then(parse_hex).unwrap_or(away);
    let mut theme = Theme { px_per_m: ppm, home: draw_home, away: draw_away, min_alpha: o.f("min-alpha", 0.8) as f32, show_numbers: o.has("numbers"), ..Theme::default() };
    let mut mcfg = game_config(o, &profile, home, away);
    mcfg.referee_rgb = o.s("ref-color").and_then(parse_hex);
    teams.apply(&mut mcfg);
    mcfg.tracker.spawn_conf = { let (low, high) = thr_players(o); if low < high { high } else { 0.0 } };
    let mut numbers = numbers_setup(o, &mut mcfg);
    let mut scenes = o.has("scenes").then(|| SceneDetector::new(profile.clone()));
    let mut scene_log: Vec<SceneEvent> = Vec::new();
    let classifier = {
        let c = reco_minimap::team::TeamClassifier::new(home, away).with_keepers(teams.home_gk, teams.away_gk);
        match mcfg.referee_rgb { Some(r) => c.with_referee(r), None => c }
    };
    let mut mm = Minimap::new(mcfg, Box::new(projector));
    let (mut n_player, mut n_ref, mut n_ball, mut n_unknown, mut n_refcolor) = (0usize, 0usize, 0usize, 0usize, 0usize);

    let codec = Codec::H264(o.s("vcodec").unwrap_or("libx264").to_string());
    let (mut w_min, mut w_alpha, mut w_pip, mut jsonl) = (None, None, None, None);
    let r_min = Renderer::with_profile(theme.clone(), profile.clone());
    if let Some(p) = o.s("minimap") {
        let (w, h) = r_min.size();
        w_min = Some(FrameWriter::open(p, w, h, map_fps, true, &codec).unwrap_or_else(|e| fail(e)));
    }
    let mut r_alpha = None;
    if let Some(p) = o.s("alpha") {
        let mut t2 = theme.clone();
        t2.background_alpha = (o.f("alpha-bg", 0.0).clamp(0.0, 1.0) * 255.0) as u8;
        let r = Renderer::with_profile(t2, profile.clone());
        let (w, h) = r.size();
        w_alpha = Some(FrameWriter::open(p, w, h, map_fps, true, &Codec::ProRes4444).unwrap_or_else(|e| fail(e)));
        r_alpha = Some(r);
    }
    let mut r_pip = None;
    if let Some(p) = o.s("pip") {
        let frac = o.f("pip-width", 0.28) as f32;
        let kf = (profile.field.length / 28.0).sqrt().max(0.8);
        theme.px_per_m = (frac * info.w as f32 / (profile.field.length + 2.0 * theme.pad_m * kf)).max(1.0);
        theme.background_alpha = 215;
        r_pip = Some(Renderer::with_profile(theme.clone(), profile.clone()));
        w_pip = Some(FrameWriter::open(p, info.w as usize, info.h as usize, out_fps, false, &codec).unwrap_or_else(|e| fail(e)));
    }
    if let Some(p) = o.s("json") {
        jsonl = Some(std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    }
    let mut dets_out = o.s("dets").map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    if w_min.is_none() && w_alpha.is_none() && w_pip.is_none() && jsonl.is_none() && dets_out.is_none() {
        fail::<()>(trs!("kein Ausgang gewählt: --minimap, --alpha, --pip, --json und/oder --dets", "no output chosen: --minimap, --alpha, --pip, --json and/or --dets", "aucune sortie choisie : --minimap, --alpha, --pip, --json et/ou --dets", "no se ha elegido ninguna salida: --minimap, --alpha, --pip, --json y/o --dets"));
    }
    if let Some(w) = dets_out.as_mut() {
        let head = serde_json::json!({"reco_minimap_dets": 1, "video": input, "w": info.w, "h": info.h, "fps": info.fps, "every": every, "start": t_start(o),
            "thr": {"ball": model.cfg.thr_ball, "player": model.cfg.thr_player, "referee": model.cfg.thr_referee}});
        let _ = writeln!(w, "{head}");
    }

    let mut reader = FrameReader::open(input, &info, t_start(o), t_duration(o), every as u32).unwrap_or_else(|e| fail(e));
    // optional second, higher-resolution version of the same panorama for reading numbers
    let mut hires = o.s("hires").map(|p| {
        let hi = video::probe(p).unwrap_or_else(|e| fail(format!("--hires: {e}")));
        eprintln!("{}", tr!("Nummern aus {} ({}x{}), Erkennung aus dem kleineren Panorama", "Numbers from {} ({}x{}), detection from the smaller panorama", "Numéros depuis {} ({}x{}), détection sur le panorama plus petit", "Dorsales desde {} ({}x{}), detección en el panorama más pequeño", p, hi.w, hi.h));
        FrameReader::open(p, &hi, t_start(o), t_duration(o), every as u32).unwrap_or_else(|e| fail(e))
    });
    let t_start = t_start(o);
    let (mut idx, mut done) = (0u64, 0u64);
    let t0 = Instant::now();
    let mut last_report = Instant::now();
    eprintln!("{}", tr!("Verarbeite {}x{}, {} Kacheln/Bild, jedes {}. Bild …  (Strg+C beendet; Datei wird dann evtl. unvollständig)", "Processing {}x{}, {} tiles/frame, every {}. frame …  (Ctrl+C stops; the file may then be incomplete)", "Traitement de {}x{}, {} tuiles/image, une image sur {} …  (Ctrl+C arrête ; le fichier peut alors être incomplet)", "Procesando {}x{}, {} teselas/imagen, una de cada {} imágenes …  (Ctrl+C detiene; el archivo puede quedar incompleto)", info.w, info.h, tiles.len(), every));
    let mut prev_state: Option<FrameState> = None;
    let (mut d_decode, mut d_track, mut d_out) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
    loop {
        let td = Instant::now();
        let mut frame = match reader.next() {
            Some(f) => f,
            None => break,
        };
        d_decode += td.elapsed();
        let this = idx * every as u64; // ffmpeg only delivers every Nth frame
        idx += 1;
        let t = t_start + this as f64 / info.fps;
        let mut dets = model.detect(&frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter_mut().filter(|d| d.class == Class::Player) {
            d.color = if shirt_match { frame.torso_color_matching(d, torso, &shirts) } else { frame.torso_color_in(d, torso) };
            n_player += 1;
            if classifier.classify(d.color) == Team::Unknown {
                n_unknown += 1;
            }
            if d.color.map(|c| classifier.is_referee_colored(c)).unwrap_or(false) {
                n_refcolor += 1;
            }
        }
        n_ref += dets.iter().filter(|d| d.class == Class::Referee).count();
        n_ball += dets.iter().filter(|d| d.class == Class::Ball).count();
        if let Some(w) = dets_out.as_mut() {
            let _ = writeln!(w, "{}", dets_json(t, &dets));
        }
        let tt = Instant::now();
        let state = mm.process(t, &dets);
        if let Some(nm) = numbers.as_mut() {
            let hf = hires.as_mut().and_then(|r| r.next());
            read_numbers(nm, &mut mm, hf.as_ref().unwrap_or(&frame), t);
        }
        log_events(&state.events);
        let sc = scenes.as_mut().map(|d| d.update(&state)).unwrap_or_default();
        log_scenes(&sc, &profile.id);
        scene_log.extend(sc.iter().cloned());
        d_track += tt.elapsed();
        let to = Instant::now();
        // in-between frames (interpolated), then the detected one
        if smooth {
            if let Some(prev) = &prev_state {
                for j in 1..every {
                    let mid = prev.lerp(&state, j as f32 / every as f32);
                    if let Some(w) = w_min.as_mut() {
                        w.write(&r_min.render(&mid).rgba).unwrap_or_else(|e| fail(e));
                    }
                    if let (Some(w), Some(r)) = (w_alpha.as_mut(), r_alpha.as_ref()) {
                        w.write(&r.render(&mid).rgba).unwrap_or_else(|e| fail(e));
                    }
                }
            }
        }
        if let Some(w) = w_min.as_mut() {
            w.write(&r_min.render(&state).rgba).unwrap_or_else(|e| fail(e));
        }
        if let (Some(w), Some(r)) = (w_alpha.as_mut(), r_alpha.as_ref()) {
            w.write(&r.render(&state).rgba).unwrap_or_else(|e| fail(e));
        }
        if let (Some(w), Some(r)) = (w_pip.as_mut(), r_pip.as_ref()) {
            draw_dets(&mut frame, &dets);
            let c = r.render(&state);
            let (ox, oy) = (frame.w.saturating_sub(c.w + 24), frame.h.saturating_sub(c.h + 24));
            frame.blit(&c, ox, oy);
            w.write(&frame.rgb).unwrap_or_else(|e| fail(e));
        }
        if let Some(j) = jsonl.as_mut() {
            let _ = writeln!(j, "{}", state_json_with(&state, &sc));
        }
        prev_state = Some(state.clone());
        d_out += to.elapsed();
        done += 1;
        if last_report.elapsed().as_secs_f32() > 2.0 {
            last_report = Instant::now();
            let el = t0.elapsed().as_secs_f32();
            let fps = done as f32 / el;
            let speed = fps * every as f32 / info.fps as f32; // source seconds per wall second
            let end = t_duration(o).map(|d| t_start + d).or(info.duration);
            let eta = end.map(|d| tr!("  Rest ~{}", "  ~{} left", "  reste ~{}", "  quedan ~{}", duration_hms((d - t).max(0.0) / speed.max(0.001) as f64))).unwrap_or_else(|| trs!("  Rest unbekannt", "  time left unknown", "  reste inconnu", "  resto desconocido").into());
            eprint!("{}", tr!("\r  t={t:7.1}s  {fps:4.1} Bilder/s (Quelle {:.1}/s){eta}   ", "\r  t={t:7.1}s  {fps:4.1} frames/s (source {:.1}/s){eta}   ", "\r  t={t:7.1}s  {fps:4.1} images/s (source {:.1}/s){eta}   ", "\r  t={t:7.1}s  {fps:4.1} imágenes/s (fuente {:.1}/s){eta}   ", info.fps / every as f64));
        }
    }
    eprintln!("{}", tr!("\n{done} Bilder in {} ({:.2} Bilder/s).", "\n{done} frames in {} ({:.2} frames/s).", "\n{done} images en {} ({:.2} images/s).", "\n{done} imágenes en {} ({:.2} imágenes/s).", duration_hms(t0.elapsed().as_secs_f64()), done as f32 / t0.elapsed().as_secs_f32().max(0.001)));
    let per = |d: std::time::Duration| d.as_secs_f32() * 1000.0 / done.max(1) as f32;
    eprintln!("{}", tr!("Zeit pro Bild (ms): Decode {:.0}, Kachel-Vorverarbeitung {:.0}, Inferenz {:.0}, Tracking {:.0}, Rendern/Schreiben {:.0}", "Time per frame (ms): decode {:.0}, tile preprocessing {:.0}, inference {:.0}, tracking {:.0}, render/write {:.0}", "Temps par image (ms) : décodage {:.0}, prétraitement des tuiles {:.0}, inférence {:.0}, suivi {:.0}, rendu/écriture {:.0}", "Tiempo por imagen (ms): decodificación {:.0}, preprocesado de teselas {:.0}, inferencia {:.0}, seguimiento {:.0}, render/escritura {:.0}", per(d_decode), per(model.t_prep), per(model.t_infer), per(d_track), per(d_out)));
    let ts = mm.tracker().team_stats;
    eprintln!("{}", tr!("Team-Zuordnung: {} Erkennungen trotz anderer Trikotfarbe über die Position zugeordnet, {} Teamwechsel eines Tracks", "Team assignment: {} detections assigned by position despite another shirt colour, {} team changes of a track", "Attribution des équipes : {} détections attribuées par la position malgré une autre couleur de maillot, {} changements d'équipe d'une piste", "Asignación de equipo: {} detecciones asignadas por posición pese a otro color de camiseta, {} cambios de equipo de una pista", ts.cross_matches, ts.flips));
    if let Some(nm) = numbers.as_ref() {
        numbers_summary(nm, &mm);
    }
    if scenes.is_some() {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for e in &scene_log {
            *counts.entry(scene_label(e, &profile.id)).or_default() += 1;
        }
        eprintln!("{}", tr!("Situationen: {}", "Situations: {}", "Situations : {}", "Situaciones: {}", if counts.is_empty() { trs!("keine erkannt", "none found", "aucune détectée", "ninguna detectada").to_string() } else { counts.iter().map(|(k, v)| format!("{k} {v}×")).collect::<Vec<_>>().join(", ") }));
    }
    eprintln!("{}", tr!("Erkennungen gesamt: Spieler {n_player} (davon {n_unknown} ohne Teamfarbe, {n_refcolor} in Schiri-Farbe), Schiri-Klasse {n_ref}, Ball {n_ball} (≈{:.1} pro Bild)", "Detections in total: players {n_player} ({n_unknown} of them without a team colour, {n_refcolor} in the referee colour), referee class {n_ref}, ball {n_ball} (≈{:.1} per frame)", "Détections au total : joueurs {n_player} (dont {n_unknown} sans couleur d'équipe, {n_refcolor} à la couleur des arbitres), classe arbitre {n_ref}, ballon {n_ball} (≈{:.1} par image)", "Detecciones en total: jugadores {n_player} ({n_unknown} sin color de equipo, {n_refcolor} con el color de árbitro), clase árbitro {n_ref}, balón {n_ball} (≈{:.1} por imagen)", n_ball as f32 / done.max(1) as f32));
    // hold the last state for the frames after the final detection
    if smooth {
        if let Some(last) = &prev_state {
            for _ in 1..every {
                if let Some(w) = w_min.as_mut() {
                    w.write(&r_min.render(last).rgba).unwrap_or_else(|e| fail(e));
                }
                if let (Some(w), Some(r)) = (w_alpha.as_mut(), r_alpha.as_ref()) {
                    w.write(&r.render(last).rgba).unwrap_or_else(|e| fail(e));
                }
            }
        }
    }
    for w in [w_min, w_alpha, w_pip].into_iter().flatten() {
        w.finish().unwrap_or_else(|e| fail(e));
    }
    if let Some(mut j) = jsonl {
        let _ = j.flush();
    }
    if let Some(mut w) = dets_out {
        let _ = w.flush();
    }
    let _ = Vec2::default();
}

// ---------------------------------------------------------------- record + replay

/// One line of a detections file: `{"t": seconds, "d": [[class, conf, cx, cy, w, h, [r,g,b] | null], ...]}`
/// (class 0 ball, 1 player, 2 referee; box in normalized panorama coordinates; torso colour as sampled).
fn dets_json(t: f64, dets: &[Det]) -> serde_json::Value {
    // shortest decimal that reads back as the same f32: a replay then reproduces the run exactly
    let r = |v: f32| v.to_string().parse::<f64>().unwrap_or(v as f64);
    let rows: Vec<serde_json::Value> = dets
        .iter()
        .map(|d| {
            let c = match d.class { Class::Ball => 0, Class::Player => 1, Class::Referee => 2 };
            serde_json::json!([c, r(d.conf), r(d.cx), r(d.cy), r(d.w), r(d.h), d.color])
        })
        .collect();
    serde_json::json!({"t": t, "d": rows})
}

fn dets_from_json(v: &serde_json::Value) -> Option<(f64, Vec<Det>)> {
    let t = v["t"].as_f64()?;
    let mut out = Vec::new();
    for row in v["d"].as_array()? {
        let f = |i: usize| row[i].as_f64().map(|x| x as f32);
        let class = match row[0].as_u64()? { 0 => Class::Ball, 1 => Class::Player, 2 => Class::Referee, _ => return None };
        let mut d = Det::new(class, f(1)?, f(2)?, f(3)?, f(4)?, f(5)?);
        if let Some(c) = row[6].as_array().filter(|c| c.len() == 3) {
            d.color = Some([c[0].as_u64()? as u8, c[1].as_u64()? as u8, c[2].as_u64()? as u8]);
        }
        out.push(d);
    }
    Some((t, out))
}

/// Tracking, ball and situations from recorded detections: everything after the detector, without the video.
/// Thresholds can only be raised above the ones used while recording, so record with low ones to compare.
fn cmd_replay(o: &Opts, path: &str) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: serde_json::Value = lines.next().and_then(|l| serde_json::from_str(l).ok()).filter(|v: &serde_json::Value| v["reco_minimap_dets"].as_u64() == Some(1)).unwrap_or_else(|| fail(tr!("{path}: keine Erkennungs-Datei (erzeugen mit `run … --dets <datei>`)", "{path}: not a detections file (create one with `run … --dets <file>`)", "{path} : ce n'est pas un fichier de détections (en créer un avec `run … --dets <fichier>`)", "{path}: no es un archivo de detecciones (créalo con `run … --dets <archivo>`)")));
    let rate = head["fps"].as_f64().unwrap_or(25.0) / head["every"].as_f64().unwrap_or(1.0).max(1.0);
    let rec = |k: &str| head["thr"][k].as_f64().unwrap_or(0.0) as f32;
    let (thr_ball, thr_player, thr_ref) = (o.f("thr-ball", 0.30) as f32, thr_players(o).0, o.f("thr-ref", 0.50) as f32);
    for (name, want, have) in [("--thr-ball", thr_ball, rec("ball")), ("--thr-player(-low)", thr_player, rec("player")), ("--thr-ref", thr_ref, rec("referee"))] {
        if want < have - 1e-6 {
            eprintln!("{}", tr!("Hinweis: {name} {want} liegt unter der Aufnahme-Schwelle {have}; es gilt {have}.", "Note: {name} {want} is below the recording threshold {have}; {have} applies.", "Remarque : {name} {want} est sous le seuil d'enregistrement {have} ; {have} s'applique.", "Nota: {name} {want} está por debajo del umbral de grabación {have}; se aplica {have}."));
        }
    }
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail(trs!("--court court.json fehlt", "--court court.json is missing", "--court court.json manquant", "falta --court court.json"))));
    let projector = cal.projector().unwrap_or_else(|| fail(trs!("court.json: Homographie nicht lösbar", "court.json: the homography cannot be solved", "court.json : l'homographie ne peut pas être résolue", "court.json: no se puede resolver la homografía")));
    let teams = read_teams(o);
    let (home, away) = (teams.home, teams.away);
    let mut mcfg = game_config(o, &profile, home, away);
    mcfg.referee_rgb = o.s("ref-color").and_then(parse_hex);
    teams.apply(&mut mcfg);
    mcfg.tracker.spawn_conf = { let (low, high) = thr_players(o); if low < high { high } else { 0.0 } };
    let mut scenes = o.has("scenes").then(|| SceneDetector::new(profile.clone()));
    let mut mm = Minimap::new(mcfg, Box::new(projector));
    let mut jsonl = o.s("json").map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    let theme = Theme { px_per_m: o.s("scale").and_then(|v| v.parse().ok()).unwrap_or_else(|| Renderer::default_px_per_m(&profile)), home: o.s("draw-home").and_then(parse_hex).unwrap_or(home), away: o.s("draw-away").and_then(parse_hex).unwrap_or(away), min_alpha: o.f("min-alpha", 0.8) as f32, ..Theme::default() };
    let renderer = Renderer::with_profile(theme, profile.clone());
    let mut w_min = o.s("minimap").map(|p| {
        let (w, h) = renderer.size();
        FrameWriter::open(p, w, h, rate, true, &Codec::H264(o.s("vcodec").unwrap_or("libx264").to_string())).unwrap_or_else(|e| fail(e))
    });
    let (from, to) = (clock_opt(o, "start"), t_duration(o).map(|d| t_start(o) + d));
    let t0 = Instant::now();
    let (mut n, mut n_scene) = (0u64, 0usize);
    for line in lines {
        let (t, mut dets) = serde_json::from_str(line).ok().and_then(|v| dets_from_json(&v)).unwrap_or_else(|| fail(tr!("{path}: Zeile {} unlesbar", "{path}: line {} cannot be read", "{path} : ligne {} illisible", "{path}: la línea {} no se puede leer", n + 2)));
        if from.map(|f| t < f).unwrap_or(false) || to.map(|e| t >= e).unwrap_or(false) {
            continue;
        }
        dets.retain(|d| d.conf >= match d.class { Class::Ball => thr_ball, Class::Player => thr_player, Class::Referee => thr_ref });
        let state = mm.process(t, &dets);
        log_events(&state.events);
        let sc = scenes.as_mut().map(|d| d.update(&state)).unwrap_or_default();
        log_scenes(&sc, &profile.id);
        n_scene += sc.len();
        if let Some(j) = jsonl.as_mut() {
            let _ = writeln!(j, "{}", state_json_with(&state, &sc));
        }
        if let Some(w) = w_min.as_mut() {
            w.write(&renderer.render(&state).rgba).unwrap_or_else(|e| fail(e));
        }
        n += 1;
    }
    if let Some(w) = w_min {
        w.finish().unwrap_or_else(|e| fail(e));
    }
    if let Some(mut j) = jsonl {
        let _ = j.flush();
    }
    let ts = mm.tracker().team_stats;
    eprintln!("{}", tr!("{n} Bilder in {:.2} s wiedergegeben ({:.1} Bilder/s in der Aufnahme).", "{n} frames replayed in {:.2} s ({:.1} frames/s in the recording).", "{n} images rejouées en {:.2} s ({:.1} images/s dans l'enregistrement).", "{n} imágenes reproducidas en {:.2} s ({:.1} imágenes/s en la grabación).", t0.elapsed().as_secs_f32(), rate));
    eprintln!("{}", tr!("Team-Zuordnung: {} Erkennungen trotz anderer Trikotfarbe über die Position zugeordnet, {} Teamwechsel eines Tracks", "Team assignment: {} detections assigned by position despite another shirt colour, {} team changes of a track", "Attribution des équipes : {} détections attribuées par la position malgré une autre couleur de maillot, {} changements d'équipe d'une piste", "Asignación de equipo: {} detecciones asignadas por posición pese a otro color de camiseta, {} cambios de equipo de una pista", ts.cross_matches, ts.flips));
    if scenes.is_some() {
        eprintln!("{}", tr!("Situationen: {n_scene}", "Situations: {n_scene}", "Situations : {n_scene}", "Situaciones: {n_scene}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_times() {
        assert_eq!(parse_clock("90"), Some(90.0));
        assert_eq!(parse_clock("1:30"), Some(90.0));
        assert_eq!(parse_clock("0:12:30.5"), Some(750.5));
        assert_eq!(parse_clock("1:02:03"), Some(3723.0));
        assert_eq!(parse_clock("abc"), None);
        assert_eq!(parse_clock("1:2:3:4"), None);
        assert_eq!(parse_clock("-5"), None);
        let o = Opts::parse(&["--start".into(), "12:30".into(), "--end".into(), "22:30".into()]);
        assert_eq!((t_start(&o), t_duration(&o)), (750.0, Some(600.0)));
        let o = Opts::parse(&["--start".into(), "60".into()]);
        assert_eq!((t_start(&o), t_duration(&o)), (60.0, None));
    }

    #[test]
    fn detections_survive_recording() {
        let mut p = Det::new(Class::Player, 0.8123, 1.0 / 3.0, 0.4 + f32::EPSILON, 0.01, 0.09);
        p.color = Some([12, 200, 7]);
        let dets = vec![p, Det::new(Class::Ball, 0.31, 0.25, 0.75, 0.004, 0.007), Det::new(Class::Referee, 0.6, 0.1, 0.2, 0.01, 0.08)];
        let line = dets_json(12.3456, &dets).to_string();
        let (t, back) = dets_from_json(&serde_json::from_str(&line).unwrap()).unwrap();
        assert_eq!(t, 12.3456);
        assert_eq!(back.len(), 3);
        for (a, b) in dets.iter().zip(&back) {
            assert_eq!((a.class, a.color), (b.class, b.color));
            assert_eq!((a.conf, a.cx, a.cy, a.w, a.h), (b.conf, b.cx, b.cy, b.w, b.h));
        }
    }
}
