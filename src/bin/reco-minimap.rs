//! reco-minimap: command-line tools around the minimap crate.
//!
//!   reco-minimap calibrate <video> ...   click court landmarks in the browser -> court.json
//!   reco-minimap selfcheck <video> ...   verify the RF-DETR model on your footage (text report)
//!   reco-minimap colors    <video> ...   find the dominant jersey colours -> teams.json
//!   reco-minimap run       <video> ...   panorama video / live feed -> minimap (+ PiP, JSON)

use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use reco_minimap::detect::{nms, plan_tiles, Class, Det, Tile};
use reco_minimap::geom::Vec2;
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::team::Team;
use reco_minimap::tools::calib::{self, CourtCalib};
use reco_minimap::tools::colors::{hex, kmeans, lab_dist, parse_hex};
use reco_minimap::tools::infer::{Ep, RfDetr};
use reco_minimap::tools::video::{self, Codec, Frame, FrameReader, FrameWriter, VideoInfo};
use reco_minimap::{ball::BallState, Config, FrameState, Minimap};

const HELP: &str = "\
reco-minimap – Minimap (Court, Spieler, Schiris, Ball) aus einem Reco-Panorama

  reco-minimap calibrate <video> [--time 5] [--out court.json] [--no-open]
  reco-minimap selfcheck <video> --model <ordner|.onnx> [--samples 12] [--annotate <ordner>]
  reco-minimap colors    <video> --model <ordner|.onnx> [--court court.json] [--home-hint #rrggbb] [--out teams.json]
  reco-minimap run       <video> --model <ordner|.onnx> --court court.json (--teams teams.json | --home #hex --away #hex)
                         [--minimap out.mp4] [--alpha out.mov] [--pip out.mp4] [--json out.jsonl]

<video> ist eine Datei oder jede ffmpeg-Quelle (rtmp://, srt://, udp://, Named Pipe ...) – also auch ein Live-Panorama.
Für Live-Quellen: --size 4096x1500 --fps 25 angeben.

Gemeinsame Optionen:
  --tile 512        Kachelgröße in Pixeln (Modell-Eingang ist 512; nativ ist am besten für den Ball)
  --overlap 0.25    Überlappung der Kacheln
  --roi x0,y0,x1,y1 Suchbereich als Bruchteile des Panoramas (z.B. 0,0.3,1,0.9 = nur das Spielfeld-Band)
  --ep cpu|coreml   Ausführung (coreml nur mit `--features coreml` gebaut)
  --start S --duration S --every N   Ausschnitt / jedes N-te Bild verarbeiten
  --thr-ball 0.30 --thr-player 0.60 --thr-ref 0.50

run-Optionen:
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

fn open_info(o: &Opts, input: &str) -> VideoInfo {
    let mut info = match (o.s("size"), video::probe(input)) {
        (_, Ok(i)) => i,
        (Some(_), Err(_)) => VideoInfo { w: 0, h: 0, fps: 25.0, duration: None },
        (None, Err(e)) => fail(e),
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
    plan_tiles(roi, o.u("tile", 512) as u32, o.f("overlap", 0.25) as f32)
}

fn load_model(o: &Opts) -> RfDetr {
    let path = o.s("model").unwrap_or_else(|| fail("--model <ordner|.onnx> fehlt"));
    let ep = match o.s("ep").unwrap_or("cpu") {
        "coreml" => Ep::CoreMl,
        _ => Ep::Cpu,
    };
    let mut m = RfDetr::load(path, ep).unwrap_or_else(|e| fail(e));
    m.cfg.thr_ball = o.f("thr-ball", 0.30) as f32;
    m.cfg.thr_player = o.f("thr-player", 0.60) as f32;
    m.cfg.thr_referee = o.f("thr-ref", 0.50) as f32;
    eprintln!("Modell: {} ({}x{}, Klassen {:?})", m.info.id, m.size, m.size, m.info.classes);
    m
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
    let input = o.pos.first().cloned().unwrap_or_else(|| fail("<video> fehlt (siehe --help)"));
    match args[0].as_str() {
        "calibrate" => cmd_calibrate(&o, &input),
        "selfcheck" => cmd_selfcheck(&o, &input),
        "colors" => cmd_colors(&o, &input),
        "run" => cmd_run(&o, &input),
        other => fail(format!("unbekannter Befehl '{other}' (calibrate | selfcheck | colors | run)")),
    }
}

// ---------------------------------------------------------------- calibrate

fn cmd_calibrate(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let out = o.s("out").unwrap_or("court.json");
    eprintln!("Panorama {}x{} @ {:.2} fps", info.w, info.h, info.fps);
    calib::run(input, &info, o.f("time", 5.0), out, !o.has("no-open")).unwrap_or_else(|e| fail(e));
    eprintln!("Fertig: {out}");
}

// ---------------------------------------------------------------- selfcheck

fn cmd_selfcheck(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let samples = o.u("samples", 12).max(1);
    let start = o.f("start", 0.0);
    let span = o.s("duration").and_then(|v| v.parse().ok()).or(info.duration.map(|d| d - start - 1.0)).unwrap_or(60.0).max(1.0);
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
                for d in d {
                    dets.push(reco_minimap::detect::tile_to_pano(&d, tile, frame.w as u32, frame.h as u32));
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
        done += 1;
        eprintln!("  t={t:6.1}s  Spieler {np:2}  Schiris {nr}  Ball {nb}");
    }
    if done == 0 {
        fail::<()>("kein Frame lesbar");
    }

    println!("\n================ Selbsttest ================");
    println!("Inferenz: {:.0} ms/Kachel (Median) -> {:.0} ms/Bild bei {} Kacheln", median(&mut tile_ms.clone()).unwrap_or(0.0), median(&mut tile_ms.clone()).unwrap_or(0.0) * tiles.len() as f32, tiles.len());
    println!("Logits: {lmin:.1} … {lmax:.1}   Boxen-Werte: {bmin:.3} … {bmax:.3}");
    println!("\nKlassen-Index  (laut runtime.json)   Treffer >=0.5   bester Score");
    for i in 0..nc {
        println!("  [{i}] {:<10}  {:>12}   {:>7.2}", model.info.classes.get(i).map(|s| s.as_str()).unwrap_or("?"), argmax_hits[i], best_score[i]);
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

// ---------------------------------------------------------------- colors

fn sample_frames(o: &Opts, input: &str, info: &VideoInfo) -> Vec<Frame> {
    let n = o.u("samples", 20).max(1);
    let start = o.f("start", 0.0);
    let span = o.s("duration").and_then(|v| v.parse().ok()).or(info.duration.map(|d| d - start - 1.0)).unwrap_or(60.0).max(1.0);
    (0..n).filter_map(|s| video::grab(input, info, start + span * (s as f64 + 0.5) / n as f64).ok()).collect()
}

fn cmd_colors(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let projector = o.s("court").map(|p| CourtCalib::load(p).unwrap_or_else(|e| fail(e)).projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar")));
    if projector.is_none() {
        eprintln!("Hinweis: ohne --court werden auch Zuschauer/Bank mitgezählt. Besser erst `calibrate` und dann --court court.json.");
    }
    let mut samples: Vec<[u8; 3]> = Vec::new();
    for (i, frame) in sample_frames(o, input, &info).iter().enumerate() {
        let dets = model.detect(frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter().filter(|d| d.class == Class::Player) {
            if let Some(p) = &projector {
                use reco_minimap::geom::{in_court, Projector};
                let (u, v) = d.foot();
                if !p.to_court(u, v).map(|c| in_court(c, 0.3)).unwrap_or(false) {
                    continue;
                }
            }
            if let Some(c) = frame.torso_color(d) {
                samples.push(c);
            }
        }
        eprint!("\r  Stichprobe {}…  {} Farbproben", i + 1, samples.len());
    }
    eprintln!();
    let clusters = kmeans(&samples, 3);
    if clusters.len() < 2 {
        fail::<()>("zu wenige Farbproben – andere --samples/--roi oder --thr-player senken");
    }
    println!("\nGefundene Trikotfarben (Anteil an {} Proben):", samples.len());
    for (i, c) in clusters.iter().enumerate() {
        println!("  {}  \x1b[48;2;{};{};{}m      \x1b[0m  {}   {:.0}%", i + 1, c.rgb[0], c.rgb[1], c.rgb[2], hex(c.rgb), 100.0 * c.count as f32 / samples.len() as f32);
    }
    // two largest clusters that are visibly different
    let a = clusters[0].rgb;
    let b = clusters.iter().skip(1).find(|c| lab_dist(c.rgb, a) > 25.0).map(|c| c.rgb).unwrap_or_else(|| fail("die zwei häufigsten Farben sind zu ähnlich – Trikots nicht unterscheidbar"));
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
    println!("Cluster 3 sind meist Schiris/Bank/Rauschen und werden ignoriert.");
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
    serde_json::json!({"state": state, "holder": tr, "x": s.ball.pos.map(|p| p.x), "y": s.ball.pos.map(|p| p.y)})
}

fn state_json(s: &FrameState) -> serde_json::Value {
    let agent = |a: &reco_minimap::AgentOut| {
        serde_json::json!({"id": a.id, "team": match a.team { Team::Home => "home", Team::Away => "away", Team::Unknown => "none" },
            "x": (a.pos.x * 100.0).round() / 100.0, "y": (a.pos.y * 100.0).round() / 100.0, "state": format!("{:?}", a.state).to_lowercase(), "alpha": (a.alpha * 100.0).round() / 100.0, "jersey": a.jersey})
    };
    serde_json::json!({"t": (s.t * 1000.0).round() / 1000.0, "players": s.players.iter().map(agent).collect::<Vec<_>>(), "referees": s.referees.iter().map(agent).collect::<Vec<_>>(), "ball": ball_json(s)})
}

fn cmd_run(o: &Opts, input: &str) {
    let info = open_info(o, input);
    let mut model = load_model(o);
    let tiles = plan(o, &info);
    let cal = CourtCalib::load(o.s("court").unwrap_or_else(|| fail("--court court.json fehlt (erst `reco-minimap calibrate`)"))).unwrap_or_else(|e| fail(e));
    if cal.pano_w > 0 && cal.pano_h > 0 {
        let (a, b) = (cal.pano_w as f32 / cal.pano_h as f32, info.w as f32 / info.h as f32);
        if (a - b).abs() / a > 0.02 {
            eprintln!("Warnung: court.json wurde für {}x{} erstellt, das Video ist {}x{} (anderes Seitenverhältnis) – neu kalibrieren!", cal.pano_w, cal.pano_h, info.w, info.h);
        }
    }
    let projector = cal.projector().unwrap_or_else(|| fail("court.json: Homographie nicht lösbar"));
    let (home, away) = read_teams(o);
    eprintln!("Heim {}  Gast {}  | Kalibrierfehler {:.2} m", hex(home), hex(away), cal.rms_m);

    let every = o.u("every", 1).max(1);
    let out_fps = info.fps / every as f64;
    let ppm = o.f("scale", 30.0) as f32;
    let mut theme = Theme { px_per_m: ppm, home, away, ..Theme::default() };
    let mut mm = Minimap::new(Config::new(home, away), Box::new(projector));

    let codec = Codec::H264(o.s("vcodec").unwrap_or("libx264").to_string());
    let (mut w_min, mut w_alpha, mut w_pip, mut jsonl) = (None, None, None, None);
    let r_min = Renderer::new(theme.clone());
    if let Some(p) = o.s("minimap") {
        let (w, h) = r_min.size();
        w_min = Some(FrameWriter::open(p, w, h, out_fps, true, &codec).unwrap_or_else(|e| fail(e)));
    }
    let mut r_alpha = None;
    if let Some(p) = o.s("alpha") {
        let mut t2 = theme.clone();
        t2.background_alpha = 0;
        let r = Renderer::new(t2);
        let (w, h) = r.size();
        w_alpha = Some(FrameWriter::open(p, w, h, out_fps, true, &Codec::ProRes4444).unwrap_or_else(|e| fail(e)));
        r_alpha = Some(r);
    }
    let mut r_pip = None;
    if let Some(p) = o.s("pip") {
        let frac = o.f("pip-width", 0.28) as f32;
        theme.px_per_m = (frac * info.w as f32 / (reco_minimap::geom::COURT_LENGTH + 2.0 * theme.pad_m)).max(4.0);
        theme.background_alpha = 215;
        r_pip = Some(Renderer::new(theme.clone()));
        w_pip = Some(FrameWriter::open(p, info.w as usize, info.h as usize, out_fps, false, &codec).unwrap_or_else(|e| fail(e)));
    }
    if let Some(p) = o.s("json") {
        jsonl = Some(std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| fail(format!("{p}: {e}")))));
    }
    if w_min.is_none() && w_alpha.is_none() && w_pip.is_none() && jsonl.is_none() {
        fail::<()>("kein Ausgang gewählt: --minimap, --alpha, --pip und/oder --json");
    }

    let mut reader = FrameReader::open(input, &info, o.f("start", 0.0), o.s("duration").and_then(|v| v.parse().ok())).unwrap_or_else(|e| fail(e));
    let t_start = o.f("start", 0.0);
    let (mut idx, mut done) = (0u64, 0u64);
    let t0 = Instant::now();
    let mut last_report = Instant::now();
    eprintln!("Verarbeite {}x{}, {} Kacheln/Bild, jedes {}. Bild …  (Strg+C beendet; Datei wird dann evtl. unvollständig)", info.w, info.h, tiles.len(), every);
    while let Some(mut frame) = reader.next() {
        let this = idx;
        idx += 1;
        if this % every as u64 != 0 {
            continue;
        }
        let t = t_start + this as f64 / info.fps;
        let mut dets = model.detect(&frame, &tiles).unwrap_or_else(|e| fail(e));
        for d in dets.iter_mut().filter(|d| d.class == Class::Player) {
            d.color = frame.torso_color(d);
        }
        let state = mm.process(t, &dets);
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
            let _ = writeln!(j, "{}", state_json(&state));
        }
        done += 1;
        if last_report.elapsed().as_secs_f32() > 2.0 {
            last_report = Instant::now();
            let el = t0.elapsed().as_secs_f32();
            let fps = done as f32 / el;
            let speed = fps * every as f32 / info.fps as f32; // source seconds per wall second
            let eta = info.duration.map(|d| format!("  Rest ~{:.0} s", ((d - t).max(0.0) as f32) / speed.max(0.001))).unwrap_or_default();
            eprint!("\r  t={t:7.1}s  {fps:4.1} Bilder/s (Quelle {:.1}/s){eta}   ", info.fps / every as f64);
        }
    }
    eprintln!("\n{done} Bilder in {:.1} s ({:.2} Bilder/s).", t0.elapsed().as_secs_f32(), done as f32 / t0.elapsed().as_secs_f32().max(0.001));
    for w in [w_min, w_alpha, w_pip].into_iter().flatten() {
        w.finish().unwrap_or_else(|e| fail(e));
    }
    if let Some(mut j) = jsonl {
        let _ = j.flush();
    }
    let _ = Vec2::default();
}
