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
