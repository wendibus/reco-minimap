//! reco-minimap: command-line tools around the minimap crate.
//!
//!   reco-minimap calibrate <video> ...   click court landmarks in the browser -> court.json
//!   reco-minimap selfcheck <video> ...   verify the RF-DETR model on your footage (text report)
//!   reco-minimap scales    <video> ...   measure player size vs. distance -> scales.json (for --tile auto)
//!   reco-minimap colors    <video> ...   find the dominant jersey colours -> teams.json
//!   reco-minimap run       <video> ...   panorama video / live feed -> minimap (+ PiP, JSON)

use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use reco_minimap::detect::{nms, plan_tiles, Class, Det, Tile};
use reco_minimap::geom::{Field, Vec2};
use reco_minimap::sport::{SportProfile, BUILTIN_IDS};
use reco_minimap::scenes::{SceneDetector, SceneEvent, SceneKind};
use reco_minimap::tools::models;
use reco_minimap::tools::ocr::{self, NumberReader};
use reco_minimap::jersey::{ctc_greedy, ctc_roster, parse_number, parse_roster};
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::team::Team;
use reco_minimap::tools::calib::{self, CourtCalib};
use reco_minimap::tools::colors::{hex, kmeans, lab_dist, parse_hex, pick_teams};
use reco_minimap::tools::infer::{Ep, RfDetr, Units};
use reco_minimap::tools::live::{self, Hub};
use reco_minimap::tools::scales::{floor_points, plan_auto, plan_auto_field, Scales};
use reco_minimap::tools::video::{self, Codec, Frame, FrameReader, FrameWriter, VideoInfo};
use reco_minimap::{ball::BallState, Config, FrameState, Minimap};

const HELP: &str = "\
reco-minimap – Minimap (Court, Spieler, Schiris, Ball) aus einem Reco-Panorama

  reco-minimap calibrate <video> [--sport football|futsal|basketball|handball|hockey|rugby|lacrosse|american_football|volleyball|custom] [--time 5] [--out court.json] [--lang de|en|fr|es]
                         (custom: --length 40 --width 20 --team-size 5 --referees 2)
  reco-minimap selfcheck <video> --model <ordner|.onnx> [--samples 12] [--annotate <ordner>]
  reco-minimap scales    <video> --model <ordner|.onnx> [--court court.json] [--samples 12] [--out scales.json]
  reco-minimap live      <quelle> --model <..> --court court.json --teams teams.json   Live-Minimap für OBS/Spieldock (Browser-Overlay)
  reco-minimap import-model <ordner|.onnx> --to <modelle-ordner>   Reco-Trainer-Modell prüfen und kopieren
  reco-minimap ui [--port 8788] [--no-open]                          Oberfläche im Browser (Deutsch/English/Français/Español)
  reco-minimap fetch-ocr                                            OCR-Modell für Trikotnummern laden (ca. 11 MB, Apache-2.0)
  reco-minimap ocr-bench <manifest.tsv> [--roster 4,5,7]            Zahlenleser an Ausschnitt-Bildern messen (Zeile: Pfad<TAB>Nummer<TAB>Gruppe)
  reco-minimap replay    <erkennungen.jsonl> --court court.json --teams teams.json [--json out.jsonl] [--minimap out.mp4]
                         Aufgezeichnete Erkennungen (run … --dets <datei>) ohne Video erneut durch Tracking, Ball und
                         Situationen schicken: Sekunden statt Minuten, z.B. um --thr-player oder Farben zu vergleichen
  reco-minimap stats     <states.jsonl>            Zahlen-Report zu einem run-Ergebnis (Slots, Sprünge, Ball)
  reco-minimap colors    <video> --model <ordner|.onnx> [--court court.json] [--home-hint #rrggbb] [--out teams.json]
  reco-minimap run       <video> --model <ordner|.onnx> --court court.json (--teams teams.json | --home #hex --away #hex)
                         [--minimap out.mp4] [--alpha out.mov] [--pip out.mp4] [--json out.jsonl]

<video> ist eine Datei oder jede ffmpeg-Quelle (rtmp://, srt://, udp://, Named Pipe ...) – also auch ein Live-Panorama.
Für Live-Quellen: --size 4096x1500 --fps 25 angeben.

Gemeinsame Optionen:
  --tile auto       Kacheln je Bildzeile passend zur Entfernung (braucht scales.json, siehe `scales`; --scales <datei>)
  --tile 512        Kachelgröße(n) in Pixeln, auch mehrere: --tile 512,1024 (Mehrmaßstab). Kacheln > 512 werden
                    verkleinert, nicht-quadratische mit Rand (Letterbox) eingepasst.
  --overlap 0.25    Überlappung der Kacheln
  --roi x0,y0,x1,y1 Suchbereich als Bruchteile des Panoramas (z.B. 0,0.3,1,0.9 = nur das Spielfeld-Band)
  --workers N       parallele Modell-Sitzungen (z.B. 2-3 mit CoreML; mehr Durchsatz, mehr Speicher)
  --units all|ane|gpu|cpu   CoreML-Recheneinheiten (Standard all); mit `selfcheck` die schnellste ermitteln
  --ep cpu|coreml   Ausführung (coreml nur mit `--features coreml` gebaut)
  --start T [--end T | --duration T]   nur einen Ausschnitt verarbeiten; T in Sekunden oder als min:s bzw. h:min:s
                    (z.B. --start 12:30 --end 22:30). Gilt auch für selfcheck, scales und colors.
  --every N         nur jedes N-te Bild auswerten
  --shirt-match     Farbprobe an die bekannten Trikotfarben anlehnen: hilft bei kräftigen, klar verschiedenen Farben
                    (z.B. Rot gegen Blau) und bei Spielfeldflächen in Teamfarbe; bei Hell gegen Dunkel weglassen
  --referees N      Zahl der Schiedsrichter-Plätze (Standard aus der Kalibrierung); 0 = Spiel ohne Schiedsrichter
  --torso 0.18,0.50 Teil der Spieler-Box (oben,unten als Bruchteil der Höhe), in dem das Trikot liegt; für Figuren mit
                    anderen Proportionen anpassen (colors, run, live)
  --thr-ball 0.30 --thr-player 0.60 --thr-ref 0.50
  --thr-player-low 0.30   (run/live/replay) schwächere Spieler-Erkennungen ab diesem Wert halten einen bestehenden
                    Punkt am Leben, starten aber keinen neuen: so bleiben Spieler im Gedränge erhalten, ohne dass
                    Fehlerkennungen zu Spielern werden. Gleicher Wert wie --thr-player schaltet das ab.

live-Optionen:
  --port 8765 --host 127.0.0.1   Overlay: http://HOST:PORT/  (OBS: Quelle 'Browser', diese URL; Hintergrund bleibt transparent)
                    Daten: /events (SSE), /state.json (Polling)   (--host 0.0.0.0 für OBS auf einem anderen Rechner)
  --in-args '<ffmpeg-Eingabeoptionen>'  vor der Quelle an ffmpeg gereicht. Beispiele:
                      wachsende Datei (Reco schreibt noch):   live file:live.mkv --in-args '-follow 1' --size ...
                      OBS Virtual Camera / Capture-Karte (macOS): live 'OBS Virtual Camera' --in-args '-f avfoundation -framerate 25' --size ...
  --realtime        Datei in Echtzeit abspielen (zum Testen ohne Live-Quelle), mit --start S ab Sekunde S
  Browser-Parameter: ?floor=1 dunkler Boden zum Testen, ?lead=0.2 weiter vorhersagen, ?minalpha=1, ?debug=1

Trikotnummern (run/live; erst `fetch-ocr`):
  --numbers         Nummern lesen und auf den Scheiben anzeigen; erkennt Wechsel (Nummer wechselt / Spieler verlässt und betritt das Feld)
  --roster-home 4,5,7,10   --roster-away 3,6,8     Kader: Erkennung wählt nur unter diesen Nummern (deutlich zuverlässiger)
  --hires <video>   zweites, höher aufgelöstes Panorama (gleiche Zeitachse) für die Nummern; Spieler brauchen ca. 80 px Höhe
  --ocr-model <datei>  (Standard ~/.cache/reco-minimap/ocr/rec.onnx)   --ocr-min-h 45 --ocr-max 4

Spielsituationen (run/live):
  --scenes          erkennt Aufstellungen und Wurfversuche: Basketball Sprungball, Freiwurf, Einwurf, 2er/3er-Versuch;
                    Fußball Anstoß, Elfmeter, Ecke, Einwurf; Handball Anwurf, 7 Meter, Einwurf; Futsal/Hockey Anstoß, Strafstoß, ...
                    (Liste je Sportart: docs/SZENARIEN.de.md); erscheint im Protokoll, als `scenes` im JSON und in `stats`

run-Optionen:
  --ref-color #hex  Schiri-Trikotfarbe (z.B. #000000 oder gelb): erkennt Schiris über die Farbe, auch wenn das Modell sie nicht findet
  --no-field-mask   bei --tile auto die volle Bildbreite kacheln (Standard: mit --court nur dort, wo das Feld liegt)
  --no-smooth       --minimap/--alpha nur mit der Erkennungsrate schreiben (Standard: volle Bildrate durch Interpolation bei --every N)
  --alpha-bg 0.0    Deckkraft der dunklen Unterlage im --alpha-Overlay (0 = transparent, 0.6 = gut lesbar auf hellem Video)
  --min-alpha 0.8   Deckkraft von Punkten, deren Spieler gerade nicht erkannt wird (0.35 = stark verblasst; verblasst Weiß wird grau)
  --draw-home #hex --draw-away #hex   Zeichenfarben der Spieler in der Minimap (die erkannten Farben sind lichtabhängig)
  --scale 30        Pixel pro Meter der Minimap (930x540 bei 30)
  --pip-width 0.28  Breite der Minimap im --pip-Video (Anteil der Panoramabreite)
  --vcodec libx264  (macOS: h264_videotoolbox)
";

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
    eprintln!("Fehler: {msg}");
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
    o.s(key).map(|v| parse_clock(v).unwrap_or_else(|| fail(format!("--{key} {v}: Sekunden oder min:s oder h:min:s erwartet"))))
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
        fail::<()>("--end muss nach --start liegen");
    }
    Some(end - t_start(o))
}

fn open_info(o: &Opts, input: &str) -> VideoInfo {
    // With --size the source is not probed: ffprobe would consume a one-shot live connection (srt/rtmp listener).
    let mut info = if o.s("size").is_some() {
        VideoInfo { w: 0, h: 0, fps: 25.0, duration: None }
    } else {
        video::probe(input).unwrap_or_else(|e| fail(e))
    };
    if let Some(sz) = o.s("size") {
        let (w, h) = sz.split_once('x').unwrap_or_else(|| fail("--size erwartet WxH, z.B. 4096x1500"));
        info.w = w.parse().unwrap_or_else(|_| fail("--size"));
        info.h = h.parse().unwrap_or_else(|_| fail("--size"));
    }
    if let Some(f) = o.s("fps").and_then(|v| v.parse().ok()) {
        info.fps = f;
    }
    info
}

fn plan(o: &Opts, info: &VideoInfo) -> Vec<Tile> {
    let r: Vec<f32> = o.s("roi").map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or_default();
    let (x0, y0, x1, y1) = if r.len() == 4 { (r[0], r[1], r[2], r[3]) } else { (0.0, 0.0, 1.0, 1.0) };
    let (w, h) = (info.w as f32, info.h as f32);
    let roi = Tile { x: (x0 * w) as u32, y: (y0 * h) as u32, w: (((x1 - x0) * w) as u32).max(32), h: (((y1 - y0) * h) as u32).max(32) };
    if o.s("tile") == Some("auto") {
        let sc = Scales::load(o.s("scales").unwrap_or("scales.json")).unwrap_or_else(|e| fail(e));
        let overlap = o.f("overlap", 0.25) as f32;
        // with a calibration at hand, only tile where the field is (ball detections are used up to 3 m outside)
        if let Some(c) = o.s("court").filter(|_| !o.has("no-field-mask")) {
            let (cal, profile) = load_court(c);
            if let Some(inv) = cal.projector().and_then(|p| p.pano_to_court.inverse()) {
                // columns: where the ball is still used (3 m outside); rows: where players are tracked (1 m)
                let feet = floor_points(&inv, &profile.field, 1.0, info.w, info.h);
                return plan_auto_field(&sc, info.w, info.h, overlap, &floor_points(&inv, &profile.field, 3.0, info.w, info.h), Some(&feet));
            }
        }
        return plan_auto(&sc, info.w, info.h, overlap);
    }
    let sizes: Vec<u32> = o.s("tile").unwrap_or("512").split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let mut all = Vec::new();
    for sz in if sizes.is_empty() { vec![512] } else { sizes } {
        all.extend(plan_tiles(roi, sz, o.f("overlap", 0.25) as f32));
    }
    all
}

fn load_model(o: &Opts) -> RfDetr {
    let path = o.s("model").unwrap_or_else(|| fail("--model <ordner|.onnx> fehlt"));
    let ep = match o.s("ep").unwrap_or("cpu") {
        "coreml" => Ep::CoreMl,
        _ => Ep::Cpu,
    };
    let mut m = RfDetr::load(path, ep, o.s("units").map(|u| Units::parse(u).unwrap_or_else(|| fail("--units: all | ane | gpu | cpu"))).unwrap_or(Units::All), o.u("workers", 1)).unwrap_or_else(|e| fail(e));
    m.cfg.thr_ball = o.f("thr-ball", 0.30) as f32;
    m.cfg.thr_player = thr_players(o).1;
    m.cfg.thr_referee = o.f("thr-ref", 0.50) as f32;
    eprintln!("Modell: {} ({}x{}, Klassen {:?}), {} Worker", m.info.id, m.size, m.size, m.info.classes, m.workers());
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
        fail::<()>("--torso erwartet oben,unten als Bruchteile der Boxhöhe, z.B. 0.18,0.50");
    }
    (p[0], p[1])
}

/// Sport profile from --sport (+ --length/--width/--team-size/--referees for `custom`).
fn profile_from_opts(o: &Opts) -> SportProfile {
    let id = o.s("sport").unwrap_or("basketball");
    let mut p = if id == "custom" {
        SportProfile::custom_rect("custom", o.f("length", 40.0) as f32, o.f("width", 20.0) as f32, o.u("team-size", 5), o.u("referees", 2))
    } else {
        SportProfile::builtin(id).unwrap_or_else(|| fail(format!("--sport {id}: unbekannt (möglich: {})", BUILTIN_IDS.join(", "))))
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
        print!("{HELP}");
        return;
    }
    let o = Opts::parse(&args[1..]);
    if args[0] == "fetch-ocr" {
        let dest = o.s("to").map(std::path::PathBuf::from).unwrap_or_else(ocr::default_model_path);
        match ocr::fetch_model(&dest) {
            Ok(p) => println!("OCR-Modell gespeichert: {}", p.display()),
            Err(e) => fail::<()>(e),
        }
        return;
    }
    if args[0] == "ui" {
        let url = reco_minimap::tools::ui::serve(o.u("port", 8788) as u16, !o.has("no-open"), false).unwrap_or_else(|e| fail(e));
        eprintln!("Oberfläche: {url}\n(Strg+C beendet das Programm; laufende Aufträge werden dabei abgebrochen.)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    let input = o.pos.first().cloned().unwrap_or_else(|| fail("<video> fehlt (siehe --help)"));
    match args[0].as_str() {
        "calibrate" => cmd_calibrate(&o, &input),
        "selfcheck" => cmd_selfcheck(&o, &input),
        "scales" => cmd_scales(&o, &input),
        "stats" => cmd_stats(&o, &input),
        "import-model" => cmd_import_model(&o, &input),
        "ocr-bench" => cmd_ocr_bench(&o, &input),
        "colors" => cmd_colors(&o, &input),
        "run" => cmd_run(&o, &input),
        "replay" => cmd_replay(&o, &input),
        "live" => cmd_live(&o, &input),
        other => fail(format!("unbekannter Befehl '{other}' (calibrate | selfcheck | colors | run)")),
    }
}

// ---------------------------------------------------------------- calibrate

fn cmd_calibrate(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let out = o.s("out").unwrap_or("court.json");
    eprintln!("Panorama {}x{} @ {:.2} fps", info.w, info.h, info.fps);
    let profile = profile_from_opts(o);
    eprintln!("Sportart: {} ({} × {} m)", profile.id, profile.field.length, profile.field.width);
    calib::run(input, &info, o.f("time", 5.0), out, !o.has("no-open"), profile, o.s("lang").unwrap_or("de")).unwrap_or_else(|e| fail(e));
    eprintln!("Fertig: {out}");
}

// ---------------------------------------------------------------- selfcheck

fn cmd_selfcheck(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let samples = o.u("samples", 12).max(1);
    let start = t_start(o);
    let span = t_duration(o).or(info.duration.map(|d| d - start - 1.0)).unwrap_or(60.0).max(1.0);
    eprintln!("Panorama {}x{}, {} Kachel(n) à {} px, {} Stichproben\n", info.w, info.h, tiles.len(), o.u("tile", 512), samples);

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
                fail::<()>(format!("Modell liefert {ncl} Klassen-Logits, runtime.json nennt {nc}"));
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
        eprintln!("  t={t:6.1}s  Spieler {np:2}  Schiris {nr}  Ball {nb}");
    }
    if done == 0 {
        fail::<()>("kein Frame lesbar");
    }

    println!("\n================ Selbsttest ================");
    println!("Inferenz: {:.0} ms/Kachel (Median) -> {:.0} ms/Bild bei {} Kacheln", median(&mut tile_ms.clone()).unwrap_or(0.0), median(&mut tile_ms.clone()).unwrap_or(0.0) * tiles.len() as f32, tiles.len());
    if let Some(f) = &last_frame {
        let mut walls = Vec::new();
        for _ in 0..4 {
            let t0 = Instant::now();
            let _ = model.detect(f, &tiles);
            walls.push(t0.elapsed().as_secs_f32() * 1000.0);
        }
        walls.remove(0); // first call includes warm-up
        println!("Wandzeit pro Bild mit {} Worker(n): {:.0} ms (alle {} Kacheln, Median)", model.workers(), median(&mut walls).unwrap_or(0.0), tiles.len());
    }
    println!("Logits: {lmin:.1} … {lmax:.1}   Boxen-Werte: {bmin:.3} … {bmax:.3}");
    println!("\nKlassen-Index  (laut runtime.json)   Treffer >=0.5   bester Score");
    for i in 0..nc {
        println!("  [{i}] {:<10}  {:>12}   {:>7.2}", model.info.classes.get(i).map(|s| s.as_str()).unwrap_or("?"), argmax_hits[i], best_score[i]);
    }
    println!("\nPro Maßstab (Kachelgröße -> Verkleinerung im Modelleingang):");
    for (sz, (np, nr, nb, hs)) in &per_scale {
        let mut hs = hs.clone();
        let hp = median(&mut hs).unwrap_or(0.0);
        println!("  Kachel {sz:>5} px (x{:.2}): Spieler-Treffer {np:>4}, Spielerhöhe im Panorama {hp:>4.0} px = im Modelleingang {:>4.0} px; Schiris {nr}, Ball {nb}", 512.0 / *sz as f32, hp * 512.0 / *sz as f32);
    }
    if let Some((sz, (np, ..))) = per_scale.iter().max_by_key(|(_, v)| v.0) {
        println!("  -> meiste Spieler bei Kachel {sz} px ({np} Treffer).");
    }
    let mut pl = players.clone();
    let mut rf = refs.clone();
    println!("\nPro Bild (Median): Spieler {:.0}, Schiris {:.0}; Ball in {:.0}% der Bilder", median(&mut pl).unwrap_or(0.0), median(&mut rf).unwrap_or(0.0), 100.0 * frames_with_ball as f32 / done as f32);
    if let (Some(w), Some(h)) = (median(&mut ball_w.clone()), median(&mut ball_h.clone())) {
        println!("Ball-Größe im Panorama (Median): {w:.0} x {h:.0} px");
    }
    if let Some(h) = median(&mut pl_h.clone()) {
        println!("Spielerhöhe im Panorama (Median): {h:.0} px");
    }
    if let Some(m) = median(&mut ball_scores.clone()) {
        println!("Ball-Scores >= 0.05: Median {m:.2}, {} Kandidaten (Schwelle jetzt {:.2})", ball_scores.len(), model.cfg.thr_ball);
    }

    println!("\nBewertung:");
    let mut warn = false;
    if bmax > 1.5 {
        println!("  ! Boxen sind nicht auf 0..1 normalisiert (max {bmax:.1}) – vermutlich Pixel (0..{}); Decoder muss durch die Eingangsgröße teilen.", model.size);
        warn = true;
    } else if bmin < -0.2 {
        println!("  ! Boxen haben negative Werte ({bmin:.2}) – ggf. anderes Boxformat (xyxy).");
        warn = true;
    } else {
        println!("  ok Boxen liegen in 0..1 (cx,cy,w,h plausibel).");
    }
    let total_hits: usize = argmax_hits.iter().sum();
    if total_hits == 0 {
        println!("  ! Keine einzige Erkennung >= 0.5: Vorverarbeitung (RGB/ImageNet-Normalisierung), --roi oder Kachelgröße prüfen.");
        warn = true;
    } else {
        let p_idx = model.info.classes.iter().position(|c| c == "player");
        if let Some(pi) = p_idx {
            let max_i = (0..nc).max_by_key(|&i| argmax_hits[i]).unwrap();
            if max_i != pi {
                println!("  ! Die meisten Treffer liegen auf Index [{max_i}] ('{}'), nicht auf 'player' [{pi}] – Klassenreihenfolge/Offset prüfen.", model.info.classes.get(max_i).map(|s| s.as_str()).unwrap_or("?"));
                warn = true;
            } else {
                println!("  ok Die meisten Treffer liegen auf 'player'.");
            }
        }
    }
    let mp = median(&mut pl).unwrap_or(0.0);
    if mp < 6.0 || mp > 16.0 {
        println!("  ? Spieler pro Bild (Median {mp:.0}) weicht von ~10 ab: --thr-player, --roi, Kachelgröße oder Bildausschnitt prüfen.");
    } else {
        println!("  ok Spieleranzahl plausibel.");
    }
    let rate = frames_with_ball as f32 / done as f32;
    if rate < 0.3 {
        println!("  ? Ball nur in {:.0}% der Bilder: --thr-ball senken (z.B. 0.2), kleinere --tile / mehr --overlap, oder Ball-Training verbessern.", rate * 100.0);
    } else {
        println!("  ok Ball wird in {:.0}% der Bilder gefunden.", rate * 100.0);
    }
    if !warn {
        println!("\nAnnahmen bestätigt – weiter mit: reco-minimap calibrate / colors / run");
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
        let (w, h, rgb) = match load_rgb(path) { Some(x) => x, None => { eprintln!("nicht lesbar: {path}"); continue; } };
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
    println!("{:<12}{:>6}{:>14}{:>14}{:>26}", "Gruppe", "n", "frei richtig", "Kader richtig", "sicher (>=0.8): Anteil/richtig");
    for (g, (n, free, ros, conf, conf_ok)) in &stats {
        let pc = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f32 / b as f32 };
        println!("{:<12}{:>6}{:>13.0}%{:>13.0}%{:>14.0}% / {:.0}%", g, n, pc(*free, *n), pc(*ros, *n), pc(*conf, *n), pc(*conf_ok, *conf));
    }
}

// ---------------------------------------------------------------- import-model

fn cmd_import_model(o: &Opts, input: &str) {
    let to = o.s("to").unwrap_or("models");
    let m = models::import(std::path::Path::new(input), std::path::Path::new(to)).unwrap_or_else(|e| fail(e));
    println!("Importiert: {} -> {}", m.id, m.dir.display());
    println!("  Sportart im Modell: {}", m.sport.as_deref().unwrap_or("(nicht angegeben)"));
    println!("  Klassen: {:?}, Eingabe {}x{}, {:.0} MB", m.classes, m.input_size, m.input_size, m.size_mb);
    println!("  Prüfsumme: {}", match m.sha { models::Sha::Ok => "stimmt", models::Sha::Bad => "STIMMT NICHT", models::Sha::None => "keine vorhanden" });
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
        fail::<()>("zu wenige Bilder in der Datei");
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
    println!("================ Lauf-Report ================");
    println!("{n} Bilder, {:.1} s ({:.1} Bilder/s Ausgabe)", t1 - t0, n as f64 / (t1 - t0).max(1e-6));
    println!("\nSlots: Heim im Mittel {:.1}, Gast {:.1}; genau 5+5 in {:.0}% der Bilder; 2 Schiris in {:.0}% (Schiris im Mittel {:.1})", mean(&home_n), mean(&away_n), 100.0 * f55 as f64 / n as f64, 100.0 * f2 as f64 / n as f64, mean(&ref_n));
    println!("Verschiedene Spieler-IDs: Heim {}, Gast {} (ideal 5 + 5; mehr = Slots werden neu vergeben)", ids.get("home").map(|s| s.len()).unwrap_or(0), ids.get("away").map(|s| s.len()).unwrap_or(0));
    println!("Sichtbar (nicht verdeckt/verloren): {:.0}% der Spieler-Einträge", 100.0 * mean(&vis_frac));
    let mut sp = speeds.clone();
    println!("Geschwindigkeit der Punkte: Median {:.1} m/s, 99% {:.1} m/s, max {:.1} m/s; Schritte an der Tempogrenze (>= 11 m/s): {jumps} von {} ({:.1}%); außerhalb des Felds: {outside}", pct(&mut sp, 0.5), pct(&mut sp, 0.99), pct(&mut sp, 1.0), speeds.len(), 100.0 * jumps as f64 / speeds.len().max(1) as f64);
    let tot: usize = ball_states.values().sum();
    let g = |k: &str| 100.0 * *ball_states.get(k).unwrap_or(&0) as f64 / tot as f64;
    println!("Ball: gehalten {:.0}%, Flug {:.0}%, frei {:.0}%, unbekannt {:.0}%; {flights} Flugphasen, {switches} Ballhalter-Wechsel", g("held"), g("flight"), g("loose"), g("unknown"));
    let mut sub_lines = Vec::new();
    for f in &frames {
        for e in f["events"].as_array().into_iter().flatten().filter(|e| e["type"] == "sub") {
            let n = |k: &str| e[k].as_u64().map(|x| format!("#{x}")).unwrap_or_else(|| "?".into());
            sub_lines.push(format!("  {} {} {}: {} -> {}", fmt_clock(e["t"].as_f64().unwrap_or(0.0)), if e["kind"] == "confirmed" { "Wechsel" } else { "möglicher Wechsel" }, e["team"].as_str().unwrap_or(""), n("out"), n("in")));
        }
    }
    let mut scene_lines = Vec::new();
    for f in &frames {
        for e in f["events"].as_array().into_iter().flatten().filter(|e| e["type"] == "scene") {
            scene_lines.push(format!("  {} {} {} {} (Sicherheit {:.2}){}", fmt_clock(e["t"].as_f64().unwrap_or(0.0)), e["kind"].as_str().unwrap_or(""), e["team"].as_str().unwrap_or(""), e["side"].as_str().unwrap_or(""), e["confidence"].as_f64().unwrap_or(0.0), if e["borderline"] == true { " knapp an der Linie" } else { "" }));
        }
    }
    if !scene_lines.is_empty() {
        println!("Situationen: {} Einträge\n{}", scene_lines.len(), scene_lines.join("\n"));
    }
    println!("Wechsel: {} Einträge{}", sub_lines.len(), if sub_lines.is_empty() { String::new() } else { format!("\n{}", sub_lines.join("\n")) });
    println!("\nBewertung:");
    let ok = |c: bool, m: &str| println!("  {} {m}", if c { "ok" } else { "!" });
    ok(f55 as f64 / n as f64 > 0.6, "5+5 Spieler meist vollständig");
    ok(ids.get("home").map(|s| s.len()).unwrap_or(0) <= 8 && ids.get("away").map(|s| s.len()).unwrap_or(0) <= 8, "wenig Slot-Wechsel (IDs <= 8 je Team)");
    ok((jumps as f64) < 0.03 * speeds.len().max(1) as f64, "wenige Schritte an der Tempogrenze (< 3%)");
    ok(f2 as f64 / n as f64 > 0.5, "Schiris meist erkannt");
    ok(g("unknown") < 40.0, "Ball seltener als 40% unbekannt");
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
    eprintln!("Messe Spielerhöhe über {} Kacheln (4 Maßstäbe) …", tiles.len());
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
        eprint!("\r  Stichprobe {}…  {} Messpunkte", i + 1, pts.len());
    }
    eprintln!();
    if pts.len() < 20 {
        fail::<()>(format!("nur {} Messpunkte – mehr --samples oder --court weglassen", pts.len()));
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
    let sc = Scales::fit(&pts, y_min, y_max, target).unwrap_or_else(|| fail("Anpassung fehlgeschlagen"));
    println!("\nSpielerhöhe im Panorama h(y) = {:.1} + {:.4}*y   ({} Messpunkte)", sc.a, sc.b, sc.n);
    println!("Feld-Bereich y = {:.0} … {:.0} px; Ziel: Spieler {:.0} px hoch im Modelleingang", sc.y_min, sc.y_max, sc.target_px);
    for k in 0..=4 {
        let y = sc.y_min + (sc.y_max - sc.y_min) * k as f32 / 4.0;
        println!("  y={y:>5.0}: Spieler ~{:>3.0} px -> Kachel {:>4.0} px", sc.height_at(y), sc.tile_at(y));
    }
    let auto = plan_auto(&sc, info.w, info.h, o.f("overlap", 0.25) as f32);
    println!("\n--tile auto = {} Kacheln pro Bild (statt {} im Raster).", auto.len(), tiles.len());
    let out = o.s("out").unwrap_or("scales.json");
    std::fs::write(out, serde_json::to_string_pretty(&sc.to_json()).unwrap()).unwrap_or_else(|e| fail(e));
    println!("gespeichert: {out}  ->  danach z.B. `selfcheck … --tile auto`");
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
    let projector = court_loaded.as_ref().map(|c| c.0.projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar")));
    if projector.is_none() {
        eprintln!("Hinweis: ohne --court werden auch Zuschauer/Bank mitgezählt. Besser erst `calibrate` und dann --court court.json.");
    }
    let mut samples: Vec<[u8; 3]> = Vec::new();
    for (i, frame) in sample_frames(o, input, &info).iter().enumerate() {
        let dets = model.detect(frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter().filter(|d| d.class == Class::Player) {
            if let Some(p) = &projector {
                use reco_minimap::geom::Projector;
                let (u, v) = d.foot();
                if !p.to_court(u, v).map(|c| field.contains(c, 0.3)).unwrap_or(false) {
                    continue;
                }
            }
            if let Some(c) = frame.torso_color_in(d, torso_band(o)) {
                samples.push(c);
            }
        }
        eprint!("\r  Stichprobe {}…  {} Farbproben", i + 1, samples.len());
    }
    eprintln!();
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let swatch = |c: [u8; 3]| if tty { format!("\x1b[48;2;{};{};{}m      \x1b[0m  ", c[0], c[1], c[2]) } else { String::new() };
    if samples.len() < 6 {
        fail::<()>("zu wenige Farbproben – andere --samples/--roi oder --thr-player senken");
    }
    let clusters = kmeans(&samples, 3);
    println!("\nHäufigste Farbgruppen (Anteil an {} Proben):", samples.len());
    for (i, c) in clusters.iter().enumerate() {
        println!("  {}  {}{}   {:.0}%", i + 1, swatch(c.rgb), hex(c.rgb), 100.0 * c.count as f32 / samples.len() as f32);
    }
    let (ca, cb, dist) = pick_teams(&samples).unwrap_or_else(|| fail("zu wenige Farbproben für zwei Teams"));
    println!("\nZwei Teams (Zweiteilung, gleiche Farbe bei unterschiedlichem Licht bleibt ein Team):");
    println!("  A: {}{}  ({} Proben)    B: {}{}  ({} Proben)    Abstand {:.0}", swatch(ca.rgb), hex(ca.rgb), ca.count, swatch(cb.rgb), hex(cb.rgb), cb.count, dist);
    if dist < 10.0 {
        fail::<()>(format!("die zwei Teams sind farblich praktisch nicht unterscheidbar (Abstand {dist:.0}); in der Oberfläche die Farben von Hand setzen"));
    }
    if dist < 22.0 {
        println!("  ! Die Farben sind sich ähnlich (Abstand {dist:.0}) – bitte prüfen und bei Bedarf von Hand anpassen.");
    }
    let (a, b) = (ca.rgb, cb.rgb);
    let (home, away) = match o.s("home-hint").and_then(parse_hex) {
        Some(h) => if lab_dist(a, h) <= lab_dist(b, h) { (a, b) } else { (b, a) },
        None => (a, b),
    };
    let out = o.s("out").unwrap_or("teams.json");
    let v = serde_json::json!({"home": hex(home), "away": hex(away)});
    std::fs::write(out, serde_json::to_string_pretty(&v).unwrap()).unwrap_or_else(|e| fail(e));
    println!("\nHeim {}  /  Gast {}  -> {out}", hex(home), hex(away));
    if o.s("home-hint").is_none() {
        println!("(Heim/Gast sind nach Häufigkeit zugeordnet. Mit --home-hint #rrggbb festlegen, welche Farbe 'Heim' ist, oder in {out} tauschen.)");
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
    if !o.has("numbers") {
        return None;
    }
    let path = o.s("ocr-model").map(std::path::PathBuf::from).unwrap_or_else(ocr::default_model_path);
    let reader = NumberReader::load(&path).unwrap_or_else(|e| fail(e));
    eprintln!("Trikotnummern: an (Kader Heim {:?}, Gast {:?}; leer = freie Erkennung)", mcfg.roster_home, mcfg.roster_away);
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
        "confidence": (e.confidence * 100.0).round() / 100.0, "borderline": e.borderline, "t": (e.t * 1000.0).round() / 1000.0})
}

fn scene_name(kind: SceneKind, sport: &str) -> &'static str {
    match (kind, sport) {
        (SceneKind::TipOff, _) => "Sprungball/Startball",
        (SceneKind::Kickoff, "handball") => "Anwurf",
        (SceneKind::Kickoff, _) => "Anstoß",
        (SceneKind::FreeThrow, _) => "Freiwurf",
        (SceneKind::ThrowIn, "basketball") => "Einwurf (Inbound)",
        (SceneKind::ThrowIn, _) => "Einwurf",
        (SceneKind::Corner, _) => "Ecke",
        (SceneKind::Penalty, _) => "Elfmeter/Strafstoß",
        (SceneKind::SevenMeter, _) => "7-Meter",
        (SceneKind::ShotTwo, _) => "Wurfversuch 2 Punkte",
        (SceneKind::ShotThree, _) => "Wurfversuch 3 Punkte",
    }
}

fn log_scenes(sc: &[SceneEvent], sport: &str) {
    for e in sc {
        let side = match e.side { Some('l') => " links", Some('r') => " rechts", _ => "" };
        let team = match e.team { Some(t) => format!(" ({})", team_name(t)), None => String::new() };
        eprintln!("\n  SITUATION {}{}{} bei {}  (Sicherheit {:.2}{})", scene_name(e.kind, sport), side, team, fmt_clock(e.t), e.confidence, if e.borderline { ", knapp an der Linie" } else { "" });
    }
}

fn fmt_clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn team_name(t: Team) -> &'static str {
    match t { Team::Home => "Heim", Team::Away => "Gast", Team::Unknown => "?" }
}

fn log_events(events: &[reco_minimap::SubEvent]) {
    let num = |n: Option<u8>| n.map(|x| format!("#{x}")).unwrap_or_else(|| "?".into());
    for e in events {
        match e.kind {
            reco_minimap::SubKind::Confirmed => eprintln!("\n  WECHSEL {} bei {}: {} -> {} (Platz {})", team_name(e.team), fmt_clock(e.t), num(e.out_number), num(e.in_number), e.slot),
            reco_minimap::SubKind::Possible => eprintln!("\n  möglicher Wechsel {} bei {}: {} verlässt das Feld, jemand kommt am Rand (Platz {})", team_name(e.team), fmt_clock(e.t), num(e.out_number), e.slot),
        }
    }
}

fn numbers_summary(nm: &Numbers, mm: &Minimap) {
    let tr = mm.tracker().tracks();
    let known = |team: Team| tr.iter().filter(|x| !x.referee && x.team == team && x.confirmed.is_some()).count();
    let list = |team: Team| tr.iter().filter(|x| !x.referee && x.team == team).map(|x| x.confirmed.map(|n| format!("#{n}")).unwrap_or_else(|| "?".into())).collect::<Vec<_>>().join(" ");
    eprintln!("Trikotnummern: {} Leseversuche, {} angenommen; bestätigt Heim {} / Gast {} ({} | {})", nm.tries, nm.accepted, known(Team::Home), known(Team::Away), list(Team::Home), list(Team::Away));
    let mut hs = nm.heights.clone();
    if let Some(med) = median(&mut hs) {
        let q75 = { hs.sort_by(|a, b| a.partial_cmp(b).unwrap()); hs[(hs.len() * 3 / 4).min(hs.len() - 1)] };
        eprintln!("Spielerhöhe in der Nummern-Quelle: Median {med:.0} px, obere 25 % ab {q75:.0} px -> Ziffern etwa {:.0} px bzw. {:.0} px hoch (lesbar ab ca. 10 px)", med * 0.12, q75 * 0.12);
        if q75 * 0.12 < 9.0 {
            eprintln!("  ! Auch die nahen Spieler sind für Nummern zu klein: ein höher aufgelöstes Panorama mit --hires verwenden (Spieler ab ca. 80 px Höhe).");
        } else if med * 0.12 < 9.0 {
            eprintln!("  ! Nur die nahen Spieler sind groß genug; Nummern der entfernten Spieler kommen nur, wenn sie nah an der Kamera vorbeilaufen (oder mit --hires).");
        }
    }
    let ev = mm.events_log();
    eprintln!("Wechsel: {} bestätigt, {} möglich", ev.iter().filter(|e| e.kind == reco_minimap::SubKind::Confirmed).count(), ev.iter().filter(|e| e.kind == reco_minimap::SubKind::Possible).count());
}

// ---------------------------------------------------------------- live

struct Latest {
    slot: std::sync::Mutex<Option<(Instant, Frame)>>,
    cv: std::sync::Condvar,
    eof: std::sync::atomic::AtomicBool,
    dropped: std::sync::atomic::AtomicU64,
}

fn cmd_live(o: &Opts, input: &str) {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    let info = open_info(o, input);
    if info.w == 0 {
        fail::<()>("--size WxH angeben, z.B. --size 1920x1080 (Live-Quellen werden nicht vorab geprüft)");
    }
    let mut model = load_model(o);
    model.cfg.thr_player = thr_players(o).0;
    let tiles = plan(o, &info);
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail("--court court.json fehlt")));
    let projector = cal.projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar"));
    let (home, away) = read_teams(o);
    // shirt colours the torso sample may snap to (teams, and the referees' if given)
    let shirts: Vec<[u8; 3]> = [Some(home), Some(away), o.s("ref-color").and_then(parse_hex)].into_iter().flatten().collect();
    let torso = torso_band(o);
    let shirt_match = o.has("shirt-match");
    let mut mcfg = game_config(o, &profile, home, away);
    mcfg.referee_rgb = o.s("ref-color").and_then(parse_hex);
    mcfg.tracker.spawn_conf = { let (low, high) = thr_players(o); if low < high { high } else { 0.0 } };
    let mut numbers = numbers_setup(o, &mut mcfg);
    let mut scenes = o.has("scenes").then(|| SceneDetector::new(profile.clone()));
    let mut mm = Minimap::new(mcfg, Box::new(projector));

    let draw_home = o.s("draw-home").and_then(parse_hex).unwrap_or(home);
    let draw_away = o.s("draw-away").and_then(parse_hex).unwrap_or(away);
    let hub = Hub::new(serde_json::json!({"home": hex(draw_home), "away": hex(draw_away), "referee": "#000000", "ball": "#ff8c00", "floor": "#262c34", "linecolor": "#e1e4e8",
        "sport": profile.id, "numbers": o.has("numbers"), "field": [profile.field.length, profile.field.width], "lines": profile.lines.iter().map(|pl| pl.iter().map(|p| [(p.x * 100.0).round() / 100.0, (p.y * 100.0).round() / 100.0]).collect::<Vec<_>>()).collect::<Vec<_>>()}).to_string());
    let host = o.s("host").unwrap_or("127.0.0.1").to_string();
    let addr = live::serve(hub.clone(), &host, o.u("port", 8765) as u16).unwrap_or_else(|e| fail(e));
    eprintln!("\nLive-Overlay: http://{addr}/");
    eprintln!("  OBS: Quelle hinzufügen -> Browser -> URL oben, Breite 1400 / Höhe 800 (oder passend), Hintergrund bleibt transparent.");
    eprintln!("  Zum Testen im Browser: http://{addr}/?floor=1&debug=1      Daten: http://{addr}/state.json\n");

    let mut jsonl = o.s("json").map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    let reader = FrameReader::open_with(input, &info, t_start(o), None, 1, true, o.has("realtime"), &o.s("in-args").map(|a| a.split_whitespace().map(|x| x.to_string()).collect::<Vec<_>>()).unwrap_or_default()).unwrap_or_else(|e| fail(e));
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

    let t0 = Instant::now();
    let (mut n, mut last_report, mut sum_age) = (0u64, Instant::now(), 0f32);
    loop {
        let (arrival, mut frame) = {
            let mut g = latest.slot.lock().unwrap();
            loop {
                if let Some(x) = g.take() {
                    break x;
                }
                if latest.eof.load(Ordering::SeqCst) {
                    eprintln!("\nQuelle beendet.");
                    return;
                }
                g = latest.cv.wait_timeout(g, std::time::Duration::from_millis(500)).unwrap().0;
            }
        };
        let t = arrival.duration_since(t0).as_secs_f64();
        let mut dets = model.detect(&frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter_mut().filter(|d| d.class == Class::Player) {
            d.color = if shirt_match { frame.torso_color_matching(d, torso, &shirts) } else { frame.torso_color_in(d, torso) };
        }
        let state = mm.process(t, &dets);
        if let Some(nm) = numbers.as_mut() {
            read_numbers(nm, &mut mm, &frame, t);
        }
        log_events(&state.events);
        let sc = scenes.as_mut().map(|d| d.update(&state)).unwrap_or_default();
        log_scenes(&sc, &profile.id);
        frame.rgb = Vec::new(); // free the big buffer early
        let age_ms = arrival.elapsed().as_secs_f64() * 1000.0;
        let mut v = state_json_with(&state, &sc);
        v["age_ms"] = serde_json::json!((age_ms * 10.0).round() / 10.0);
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
            eprint!("\r  {:.1} Updates/s · Latenz Bild->Ergebnis {:.0} ms · verworfene Bilder {} · {} Spieler, {} Schiris   ", n as f32 / el, sum_age / n.max(1) as f32, latest.dropped.load(Ordering::Relaxed), state.players.len(), state.referees.len());
            last_report = Instant::now();
            n = 0;
            sum_age = 0.0;
        }
    }
}

// ---------------------------------------------------------------- run

fn read_teams(o: &Opts) -> ([u8; 3], [u8; 3]) {
    if let (Some(h), Some(a)) = (o.s("home").and_then(parse_hex), o.s("away").and_then(parse_hex)) {
        return (h, a);
    }
    if let Some(p) = o.s("teams") {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))).unwrap_or_else(|e| fail(format!("{p}: {e}")));
        if let (Some(h), Some(a)) = (v["home"].as_str().and_then(parse_hex), v["away"].as_str().and_then(parse_hex)) {
            return (h, a);
        }
    }
    fail("Trikotfarben fehlen: --teams teams.json oder --home #rrggbb --away #rrggbb (Farben ermitteln: `reco-minimap colors`)")
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
        serde_json::json!({"id": a.id, "team": match a.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" },
            "x": (a.pos.x * 100.0).round() / 100.0, "y": (a.pos.y * 100.0).round() / 100.0, "vx": (a.vel.x * 100.0).round() / 100.0, "vy": (a.vel.y * 100.0).round() / 100.0, "state": format!("{:?}", a.state).to_lowercase(), "alpha": (a.alpha * 100.0).round() / 100.0, "jersey": a.jersey})
    };
    serde_json::json!({"t": (s.t * 1000.0).round() / 1000.0, "players": s.players.iter().map(agent).collect::<Vec<_>>(), "referees": s.referees.iter().map(agent).collect::<Vec<_>>(), "ball": ball_json(s),
        "events": s.events.iter().map(|e| serde_json::json!({"type": "sub", "kind": if e.kind == reco_minimap::SubKind::Confirmed { "confirmed" } else { "possible" }, "team": match e.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" }, "slot": e.slot, "out": e.out_number, "in": e.in_number, "t": (e.t * 1000.0).round() / 1000.0})).chain(scenes.iter().map(scene_json)).collect::<Vec<_>>()})
}

fn cmd_run(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    model.cfg.thr_player = thr_players(o).0;
    let tiles = plan(o, &info);
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail("--court court.json fehlt (erst `reco-minimap calibrate`)")));
    if cal.pano_w > 0 && cal.pano_h > 0 {
        let (a, b) = (cal.pano_w as f32 / cal.pano_h as f32, info.w as f32 / info.h as f32);
        if (a - b).abs() / a > 0.02 {
            eprintln!("Warnung: court.json wurde für {}x{} erstellt, das Video ist {}x{} (anderes Seitenverhältnis) – neu kalibrieren!", cal.pano_w, cal.pano_h, info.w, info.h);
        }
    }
    let projector = cal.projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar"));
    let (home, away) = read_teams(o);
    // shirt colours the torso sample may snap to (teams, and the referees' if given)
    let shirts: Vec<[u8; 3]> = [Some(home), Some(away), o.s("ref-color").and_then(parse_hex)].into_iter().flatten().collect();
    let torso = torso_band(o);
    let shirt_match = o.has("shirt-match");
    eprintln!("Heim {}  Gast {}  | Kalibrierfehler {:.2} m", hex(home), hex(away), cal.rms_m);

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
    mcfg.tracker.spawn_conf = { let (low, high) = thr_players(o); if low < high { high } else { 0.0 } };
    let mut numbers = numbers_setup(o, &mut mcfg);
    let mut scenes = o.has("scenes").then(|| SceneDetector::new(profile.clone()));
    let mut scene_log: Vec<SceneEvent> = Vec::new();
    let classifier = {
        let c = reco_minimap::team::TeamClassifier::new(home, away);
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
        fail::<()>("kein Ausgang gewählt: --minimap, --alpha, --pip, --json und/oder --dets");
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
        eprintln!("Nummern aus {} ({}x{}), Erkennung aus dem kleineren Panorama", p, hi.w, hi.h);
        FrameReader::open(p, &hi, t_start(o), t_duration(o), every as u32).unwrap_or_else(|e| fail(e))
    });
    let t_start = t_start(o);
    let (mut idx, mut done) = (0u64, 0u64);
    let t0 = Instant::now();
    let mut last_report = Instant::now();
    eprintln!("Verarbeite {}x{}, {} Kacheln/Bild, jedes {}. Bild …  (Strg+C beendet; Datei wird dann evtl. unvollständig)", info.w, info.h, tiles.len(), every);
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
            let eta = end.map(|d| format!("  Rest ~{:.0} s", ((d - t).max(0.0) as f32) / speed.max(0.001))).unwrap_or_default();
            eprint!("\r  t={t:7.1}s  {fps:4.1} Bilder/s (Quelle {:.1}/s){eta}   ", info.fps / every as f64);
        }
    }
    eprintln!("\n{done} Bilder in {:.1} s ({:.2} Bilder/s).", t0.elapsed().as_secs_f32(), done as f32 / t0.elapsed().as_secs_f32().max(0.001));
    let per = |d: std::time::Duration| d.as_secs_f32() * 1000.0 / done.max(1) as f32;
    eprintln!("Zeit pro Bild (ms): Decode {:.0}, Kachel-Vorverarbeitung {:.0}, Inferenz {:.0}, Tracking {:.0}, Rendern/Schreiben {:.0}", per(d_decode), per(model.t_prep), per(model.t_infer), per(d_track), per(d_out));
    let ts = mm.tracker().team_stats;
    eprintln!("Team-Zuordnung: {} Erkennungen trotz anderer Trikotfarbe über die Position zugeordnet, {} Teamwechsel eines Tracks", ts.cross_matches, ts.flips);
    if let Some(nm) = numbers.as_ref() {
        numbers_summary(nm, &mm);
    }
    if scenes.is_some() {
        let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
        for e in &scene_log {
            *counts.entry(scene_name(e.kind, &profile.id)).or_default() += 1;
        }
        eprintln!("Situationen: {}", if counts.is_empty() { "keine erkannt".to_string() } else { counts.iter().map(|(k, v)| format!("{k} {v}×")).collect::<Vec<_>>().join(", ") });
    }
    eprintln!("Erkennungen gesamt: Spieler {n_player} (davon {n_unknown} ohne Teamfarbe, {n_refcolor} in Schiri-Farbe), Schiri-Klasse {n_ref}, Ball {n_ball} (≈{:.1} pro Bild)", n_ball as f32 / done.max(1) as f32);
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
    let head: serde_json::Value = lines.next().and_then(|l| serde_json::from_str(l).ok()).filter(|v: &serde_json::Value| v["reco_minimap_dets"].as_u64() == Some(1)).unwrap_or_else(|| fail(format!("{path}: keine Erkennungs-Datei (erzeugen mit `run … --dets <datei>`)")));
    let rate = head["fps"].as_f64().unwrap_or(25.0) / head["every"].as_f64().unwrap_or(1.0).max(1.0);
    let rec = |k: &str| head["thr"][k].as_f64().unwrap_or(0.0) as f32;
    let (thr_ball, thr_player, thr_ref) = (o.f("thr-ball", 0.30) as f32, thr_players(o).0, o.f("thr-ref", 0.50) as f32);
    for (name, want, have) in [("--thr-ball", thr_ball, rec("ball")), ("--thr-player(-low)", thr_player, rec("player")), ("--thr-ref", thr_ref, rec("referee"))] {
        if want < have - 1e-6 {
            eprintln!("Hinweis: {name} {want} liegt unter der Aufnahme-Schwelle {have}; es gilt {have}.");
        }
    }
    let (cal, profile) = load_court(o.s("court").unwrap_or_else(|| fail("--court court.json fehlt")));
    let projector = cal.projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar"));
    let (home, away) = read_teams(o);
    let mut mcfg = game_config(o, &profile, home, away);
    mcfg.referee_rgb = o.s("ref-color").and_then(parse_hex);
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
        let (t, mut dets) = serde_json::from_str(line).ok().and_then(|v| dets_from_json(&v)).unwrap_or_else(|| fail(format!("{path}: Zeile {} unlesbar", n + 2)));
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
    eprintln!("{n} Bilder in {:.2} s wiedergegeben ({:.1} Bilder/s in der Aufnahme).", t0.elapsed().as_secs_f32(), rate);
    eprintln!("Team-Zuordnung: {} Erkennungen trotz anderer Trikotfarbe über die Position zugeordnet, {} Teamwechsel eines Tracks", ts.cross_matches, ts.flips);
    if scenes.is_some() {
        eprintln!("Situationen: {n_scene}");
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
