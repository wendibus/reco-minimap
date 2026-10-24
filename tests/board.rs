#![cfg(feature = "tools")]
//! Tactics-board scenes: cutting a moment out of a run, the JSON format, and the SVG drawing.
use std::path::PathBuf;

use reco_minimap::board::{to_svg, SvgStyle};
use reco_minimap::sport::SportProfile;
use reco_minimap::tools::board_io::{scene_from_json, scene_from_states, scene_to_json, SceneColours};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("reco-minimap-board-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Frames every 0.2 s from t = 10.0 to 12.0: home #7 walks along x, a home keeper stands still, one away player, one referee.
fn states(dir: &PathBuf, keeper_in_second_half: bool) -> PathBuf {
    let mut out = String::new();
    for k in 0..=10 {
        let t = 10.0 + 0.2 * k as f64;
        let x = 5.0 + 0.5 * k as f32;
        let ball = if k == 10 { r#"{"state":"flight","holder":null,"x":9.5,"y":4.0,"vx":0.0,"vy":0.0}"# } else { r#"{"state":"held","holder":1,"x":5.0,"y":4.0,"vx":0.0,"vy":0.0}"# };
        let keeper = keeper_in_second_half;
        out.push_str(&format!(
            r#"{{"t":{t},"players":[{{"id":1,"segment":0,"team":"home","x":{x},"y":4.0,"vx":2.5,"vy":0.0,"state":"visible","alpha":1.0,"jersey":7,"keeper":false}},{{"id":2,"segment":0,"team":"home","x":1.5,"y":7.5,"vx":0.0,"vy":0.0,"state":"visible","alpha":1.0,"jersey":1,"keeper":{keeper}}},{{"id":3,"segment":0,"team":"away","x":12.0,"y":6.0,"vx":0.0,"vy":0.0,"state":"occluded","alpha":0.6,"jersey":null,"keeper":false}},{{"id":4,"segment":0,"team":"away","x":20.0,"y":6.0,"vx":0.0,"vy":0.0,"state":"lost","alpha":0.35,"jersey":null,"keeper":false}}],"referees":[{{"x":14.0,"y":0.5}}],"ball":{ball},"events":[]}}"#
        ));
        out.push('\n');
    }
    let p = dir.join("states.jsonl");
    std::fs::write(&p, out).unwrap();
    p
}

fn colours() -> SceneColours {
    SceneColours { home: [255, 255, 255], away: [47, 107, 255], home_keeper: None, away_keeper: None }
}

#[test]
fn a_moment_is_cut_out_with_trail_and_lost_players_left_out() {
    let d = tmp("cut");
    let f = states(&d, true);
    let s = scene_from_states(&f, 12.0, 1.0, &SportProfile::basketball(), colours()).unwrap();
    assert!((s.t - 12.0).abs() < 1e-9);
    assert_eq!(s.players.len(), 3, "the lost player is not on the board");
    let p7 = s.players.iter().find(|p| p.number == Some(7)).unwrap();
    assert!((p7.pos.x - 10.0).abs() < 1e-3);
    // a trail of the last second, one point per 0.25 s at most, current position not repeated
    assert!(p7.trail.len() >= 3 && p7.trail.len() <= 5, "{}", p7.trail.len());
    assert!(p7.trail.iter().all(|q| q.x < p7.pos.x));
    assert!(p7.trail.windows(2).all(|w| w[0].x < w[1].x), "oldest first");
    assert_eq!(s.referees.len(), 1);
    assert_eq!(s.ball.state, "flight");
    assert!(!s.lines.is_empty(), "field lines travel with the scene");
    // nearest frame wins
    let near = scene_from_states(&f, 11.07, 0.0, &SportProfile::basketball(), colours()).unwrap();
    assert!((near.t - 11.0).abs() < 1e-9 || (near.t - 11.2).abs() < 1e-9);
    // far from anything in the file
    assert!(scene_from_states(&f, 600.0, 0.0, &SportProfile::basketball(), colours()).is_err());
}

#[test]
fn the_scene_survives_json_and_rejects_foreign_or_newer_files() {
    let d = tmp("json");
    let f = states(&d, true);
    let s = scene_from_states(&f, 11.0, 2.0, &SportProfile::football(), SceneColours { home_keeper: Some([30, 160, 60]), ..colours() }).unwrap();
    let v = scene_to_json(&s);
    assert_eq!(v["format"], "reco-minimap-board");
    assert_eq!(v["version"], 1);
    let back = scene_from_json(&serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap()).unwrap();
    assert_eq!(back.players.len(), s.players.len());
    assert_eq!(back.home_keeper, Some([30, 160, 60]));
    assert_eq!(back.sport, "football");
    assert!((back.field.length - 105.0).abs() < 1e-3);
    assert!(scene_from_json(&serde_json::json!({"hello": 1})).is_err());
    let mut newer = v.clone();
    newer["version"] = serde_json::json!(99);
    assert!(scene_from_json(&newer).is_err());
}

#[test]
fn keeper_stripes_only_for_sports_with_a_keeper_kit() {
    let d = tmp("sport");
    let f = states(&d, true);
    let ball = scene_from_states(&f, 11.0, 0.0, &SportProfile::basketball(), SceneColours { home_keeper: Some([0, 200, 0]), ..colours() }).unwrap();
    assert_eq!(ball.home_keeper, None, "no keeper colour in basketball");
    let fb = scene_from_states(&f, 11.0, 0.0, &SportProfile::football(), SceneColours { home_keeper: Some([0, 200, 0]), ..colours() }).unwrap();
    assert_eq!(fb.home_keeper, Some([0, 200, 0]));
}

#[test]
fn the_svg_is_well_formed_and_carries_what_a_board_needs() {
    let d = tmp("svg");
    let f = states(&d, true);
    let s = scene_from_states(&f, 11.0, 1.0, &SportProfile::football(), colours()).unwrap();
    let svg = to_svg(&s, &SvgStyle::default());
    assert!(svg.starts_with("<svg ") && svg.trim_end().ends_with("</svg>"));
    assert!(svg.contains("viewBox=\"-1.5 -1.5 108 71\""), "{}", &svg[..200.min(svg.len())]);
    assert_eq!(svg.matches("class=\"player ").count(), 3);
    assert_eq!(svg.matches("keeper\"").count(), 1, "one keeper");
    assert!(svg.contains("fill=\"url(#stripes-home)\""));
    assert!(svg.contains("data-number=\"7\"") && svg.contains(">7</text>"));
    assert!(svg.contains("class=\"holder\""), "ball holder ring");
    assert!(svg.contains("class=\"referee\""));
    assert!(svg.contains("<polyline class=\"trail"));
    // switches
    let plain = to_svg(&s, &SvgStyle { stripes: false, trails: false, numbers: false, ..SvgStyle::default() });
    assert!(!plain.contains("fill=\"url(#stripes-home)\"") && !plain.contains("class=\"trail") && !plain.contains("<text"));
    // a keeper-less scene draws no stripes
    let none = scene_from_states(&states(&tmp("svg2"), false), 11.0, 0.0, &SportProfile::football(), colours()).unwrap();
    assert!(!to_svg(&none, &SvgStyle::default()).contains("fill=\"url(#stripes-home)\""));
}

#[test]
fn the_command_line_writes_scene_and_svg() {
    use std::process::Command;
    let d = tmp("cli");
    let f = states(&d, true);
    std::fs::write(d.join("court.json"), r#"{"version":2,"sport":"basketball","pano_w":1920,"pano_h":1080,"rms_m":0.04,"points":[
      {"name":"a","pano":[0.1,0.3],"court":[0,0]},{"name":"b","pano":[0.9,0.3],"court":[28,0]},
      {"name":"c","pano":[0.95,0.8],"court":[28,15]},{"name":"d","pano":[0.05,0.8],"court":[0,15]}]}"#).unwrap();
    std::fs::write(d.join("teams.json"), r##"{"home":"#ffffff","away":"#2f6bff"}"##).unwrap();
    let bin = env!("CARGO_BIN_EXE_reco-minimap");
    let o = Command::new(bin).current_dir(&d).args(["snapshot", f.to_str().unwrap(), "--court", "court.json", "--teams", "teams.json", "--at", "0:11", "--trail", "1", "--out", "scene.json", "--svg", "scene.svg"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(d.join("scene.json").exists() && d.join("scene.svg").exists());
    // the scene file can be redrawn with other colours and options
    let o = Command::new(bin).current_dir(&d).args(["svg", "scene.json", "--draw-home", "#ff0000", "--no-trails", "--out", "again.svg"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let again = std::fs::read_to_string(d.join("again.svg")).unwrap();
    assert!(again.contains("#ff0000") && !again.contains("class=\"trail"));
    // a foreign JSON file is refused with a message, not a crash
    std::fs::write(d.join("x.json"), "{}").unwrap();
    let o = Command::new(bin).current_dir(&d).args(["svg", "x.json"]).output().unwrap();
    assert!(!o.status.success());
}

// ---------------------------------------------------------------- marks, steps, board server

use reco_minimap::tools::board_io::scene_step;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

fn base_scene() -> Value {
    json!({
        "format": "reco-minimap-board", "version": 1, "sport": "football", "t": 1.0,
        "field": {"length": 40.0, "width": 20.0},
        "teams": {"home": {"color": "#ff0000", "keeper_color": null}, "away": {"color": "#0000ff", "keeper_color": null}},
        "players": [
            {"id": 1, "team": "home", "number": 7, "keeper": false, "x": 10.0, "y": 5.0, "trail": []},
            {"id": 2, "team": "away", "number": 3, "keeper": false, "x": 30.0, "y": 15.0, "trail": []}
        ],
        "referees": [], "marks": [{"type": "arrow", "color": "#ffcc00", "points": [[1.0, 1.0], [8.0, 6.0]]}],
        "ball": {"state": "held", "holder": 1, "x": 10.0, "y": 5.0},
        "lines": [[[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0], [0.0, 0.0]]], "goals": [],
        "steps": [
            {"name": "", "players": [{"id": 1, "x": 10.0, "y": 5.0}, {"id": 2, "x": 30.0, "y": 15.0}], "ball": {"state": "held", "holder": 1, "x": 10.0, "y": 5.0}, "marks": []},
            {"name": "", "players": [{"id": 1, "x": 22.0, "y": 9.0}], "ball": {"state": "free", "holder": null, "x": 25.0, "y": 9.0}, "marks": [{"type": "pen", "color": "#ffffff", "points": [[2.0, 2.0], [3.0, 3.0], [4.0, 2.0]]}]}
        ]
    })
}

#[test]
fn marks_are_drawn_and_survive_the_round_trip() {
    let v = base_scene();
    let s = scene_from_json(&v).unwrap();
    assert_eq!(s.marks.len(), 1);
    let svg = to_svg(&s, &SvgStyle::default());
    assert!(svg.contains("class=\"mark arrow\"") && svg.contains("<polygon"), "{svg}");
    let again = scene_from_json(&scene_to_json(&s)).unwrap();
    assert_eq!(again.marks, s.marks);
    // a mark with a single point draws nothing and does not break the picture
    let mut bad = v.clone();
    bad["marks"] = json!([{"type": "pen", "color": "#fff", "points": [[1.0, 1.0]]}, {"type": "wobble", "points": []}]);
    let sb = scene_from_json(&bad).unwrap();
    assert!(!to_svg(&sb, &SvgStyle::default()).contains("class=\"mark"));
}

#[test]
fn a_step_takes_positions_ball_and_marks_from_the_steps_list() {
    let v = base_scene();
    let s2 = scene_step(&v, 2).unwrap();
    let p = |id: u32| s2.players.iter().find(|p| p.id == id).unwrap().pos;
    assert_eq!((p(1).x, p(1).y), (22.0, 9.0));
    assert_eq!((p(2).x, p(2).y), (30.0, 15.0), "a player missing from the step stays where the scene has them");
    assert_eq!(s2.ball.holder, None);
    assert_eq!(s2.marks.len(), 1);
    assert_eq!(scene_step(&v, 1).unwrap().marks.len(), 1, "step 1 is the scene itself");
    let e = scene_step(&v, 3).unwrap_err();
    assert!(e.contains('3') && e.contains('2'), "{e}");
}

struct Server {
    child: Child,
    base: String,
    host: String,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_board(scene: &PathBuf, extra: &[&str]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_reco-minimap"))
        .arg("board").arg(scene).args(["--port", "0", "--no-open", "--lang", "en"]).args(extra)
        .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let mut r = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let url = line.split_whitespace().last().unwrap().to_string();
    assert!(url.starts_with("http://"), "{line}");
    let hostport = url.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let (host, port) = hostport.rsplit_once(':').unwrap();
    // keep draining stderr so the child never blocks on a full pipe
    std::thread::spawn(move || { let mut s = String::new(); let _ = r.read_to_string(&mut s); });
    Server { child, base: url.split('?').nth(1).unwrap_or("").to_string(), host: host.to_string(), port: port.parse().unwrap() }
}

/// One raw HTTP request to `127.0.0.1:port` with the given Host header; returns (status, body).
fn http(port: u16, method: &str, path: &str, host: &str, body: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string())
}

#[test]
fn the_board_server_serves_saves_and_refuses_strangers() {
    let d = tmp("srv");
    let scene = d.join("scene.json");
    std::fs::write(&scene, serde_json::to_string(&base_scene()).unwrap()).unwrap();
    let srv = start_board(&scene, &[]);
    let me = format!("127.0.0.1:{}", srv.port);

    let (c, page) = http(srv.port, "GET", "/", &me, "");
    assert_eq!(c, 200);
    assert!(page.contains("<script src=\"/i18n.js\"></script>") && page.contains("id=\"board\""));
    assert_eq!(http(srv.port, "GET", "/i18n.js", &me, "").0, 200);
    let (c, body) = http(srv.port, "GET", "/scene.json", &me, "");
    assert_eq!(c, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["players"].as_array().unwrap().len(), 2);

    // save: the file and the picture next to it are replaced
    let mut edited = base_scene();
    edited["players"][0]["x"] = json!(15.5);
    let (c, _) = http(srv.port, "POST", "/scene.json", &me, &edited.to_string());
    assert_eq!(c, 200);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&scene).unwrap()).unwrap();
    assert_eq!(saved["players"][0]["x"], json!(15.5));
    assert_eq!(saved["steps"].as_array().unwrap().len(), 2, "unknown and board-only keys are kept as sent");
    assert!(std::fs::read_to_string(d.join("scene.svg")).unwrap().starts_with("<svg") || std::fs::read_to_string(d.join("scene.svg")).unwrap().contains("<svg"));
    assert!(!d.join("scene.json.tmp").exists());

    // refused: not a scene, a newer format, a foreign Host (DNS rebinding)
    assert_eq!(http(srv.port, "POST", "/scene.json", &me, "{\"hello\":1}").0, 400);
    let mut newer = base_scene();
    newer["version"] = json!(99);
    assert_eq!(http(srv.port, "POST", "/scene.json", &me, &newer.to_string()).0, 400);
    assert_eq!(http(srv.port, "GET", "/scene.json", "evil.example:80", "").0, 403);
    assert_eq!(http(srv.port, "POST", "/scene.json", "evil.example:80", &edited.to_string()).0, 403);
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&scene).unwrap()).unwrap();
    assert_eq!(after["players"][0]["x"], json!(15.5), "refused requests change nothing");
}

#[test]
fn network_mode_needs_the_key() {
    let d = tmp("lan");
    let scene = d.join("scene.json");
    std::fs::write(&scene, serde_json::to_string(&base_scene()).unwrap()).unwrap();
    let srv = start_board(&scene, &["--lan"]);
    let key = srv.base.split('&').find_map(|kv| kv.strip_prefix("k=")).expect("link carries a key").to_string();
    assert!(key.len() >= 32);
    let me = format!("{}:{}", srv.host, srv.port);
    assert_eq!(http(srv.port, "GET", "/scene.json", &me, "").0, 403, "no key");
    assert_eq!(http(srv.port, "GET", "/scene.json?k=wrong", &me, "").0, 403, "wrong key");
    assert_eq!(http(srv.port, "GET", &format!("/scene.json?k={key}"), &me, "").0, 200);
    assert_eq!(http(srv.port, "POST", &format!("/scene.json?k={key}"), &me, &base_scene().to_string()).0, 200);
}

#[test]
fn the_board_page_can_be_exported_and_a_missing_scene_is_explained() {
    let d = tmp("export");
    let scene = d.join("scene.json");
    std::fs::write(&scene, serde_json::to_string(&base_scene()).unwrap()).unwrap();
    let out = d.join("board.html");
    let st = Command::new(env!("CARGO_BIN_EXE_reco-minimap")).arg("board").arg(&scene).args(["--export"]).arg(&out).args(["--lang", "en"]).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(!html.contains("src=\"/i18n.js\"") && html.contains("const I18N"), "language table is inlined");
    let miss = Command::new(env!("CARGO_BIN_EXE_reco-minimap")).arg("board").arg(d.join("nope.json")).args(["--lang", "en"]).output().unwrap();
    assert!(!miss.status.success());
    assert!(String::from_utf8_lossy(&miss.stderr).contains("--states"), "{}", String::from_utf8_lossy(&miss.stderr));
}
