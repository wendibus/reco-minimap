#![cfg(feature = "tools")]
use std::path::PathBuf;

use reco_minimap::tools::calib::{CalPoint, CourtCalib};
use reco_minimap::tools::models::{import, inspect, sha256_hex, Sha};

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("reco_minimap_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn make_export(dir: &PathBuf, sport: &str, tamper: bool) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("rfdetr-small.onnx"), b"not a real model, just bytes").unwrap();
    let sha = sha256_hex(&dir.join("rfdetr-small.onnx")).unwrap();
    let sha = if tamper { "0".repeat(64) } else { sha };
    std::fs::write(dir.join("runtime.json"), format!(r#"{{"schemaVersion":1,"modelId":"club-{sport}","artifact":{{"file":"rfdetr-small.onnx","sha256":"{sha}"}},"inputSize":512,"classes":["ball","player","goalkeeper","referee","ignore"],"sport":"{sport}"}}"#)).unwrap();
    std::fs::write(dir.join("model-card.json"), format!(r#"{{"name":"club-{sport}","sport":"{sport}","artifacts":[{{"file":"rfdetr-small.onnx","sha256":"{sha}"}}]}}"#)).unwrap();
}

#[test]
fn import_reco_trainer_export_keeps_sport_classes_and_checksum() {
    let root = tmp("import");
    let src = root.join("export");
    make_export(&src, "football", false);
    let models = root.join("project").join("models");
    let m = import(&src, &models).unwrap();
    assert_eq!(m.sport.as_deref(), Some("football"));
    assert_eq!(m.classes, vec!["ball", "player", "goalkeeper", "referee", "ignore"]);
    assert_eq!(m.sha, Sha::Ok);
    assert!(m.onnx.exists() && m.dir.starts_with(&models));
    // importing the same export again must not overwrite the first copy
    let m2 = import(&src, &models).unwrap();
    assert_ne!(m.dir, m2.dir);
    assert_eq!(reco_minimap::tools::models::list(&models).len(), 2);
}

#[test]
fn damaged_weights_are_rejected() {
    let root = tmp("bad");
    let src = root.join("export");
    make_export(&src, "basketball", true);
    assert_eq!(inspect(&src).unwrap().sha, Sha::Bad);
    let err = import(&src, &root.join("models")).unwrap_err();
    assert!(err.contains("Prüfsumme"), "{err}");
    assert!(!root.join("models").exists() || std::fs::read_dir(root.join("models")).unwrap().count() == 0);
}

#[test]
fn bare_onnx_gets_a_runtime_json() {
    let root = tmp("bare");
    let f = root.join("my model.onnx");
    std::fs::write(&f, b"bytes").unwrap();
    let m = import(&f, &root.join("models")).unwrap();
    assert!(m.dir.join("runtime.json").exists());
    assert_eq!(m.classes, vec!["ball", "player", "referee", "ignore"]);
    assert_eq!(m.sha, Sha::None);
    assert!(inspect(&root.join("nothing")).is_err());
}

#[test]
fn court_json_roundtrip_keeps_sport_and_old_files_still_load() {
    let root = tmp("court");
    let c = CourtCalib {
        pano_w: 1920,
        pano_h: 1080,
        sport: "custom".into(),
        field_m: (30.0, 16.0),
        team_size: 6,
        referees: 1,
        points: (0..4).map(|i| CalPoint { name: format!("p{i}"), pano: reco_minimap::geom::Vec2::new(i as f32 * 0.2, 0.3 + 0.1 * (i % 2) as f32), court: reco_minimap::geom::Vec2::new(i as f32 * 5.0, 3.0 * (i % 2) as f32) }).collect(),
        rms_m: 0.1,
    };
    let p = root.join("court.json");
    c.save(&p.to_string_lossy(), "x.mp4").unwrap();
    let l = CourtCalib::load(&p.to_string_lossy()).unwrap();
    let prof = l.profile();
    assert_eq!((prof.field.length, prof.field.width, prof.team_size, prof.referees), (30.0, 16.0, 6, 1));
    // a version-1 file (before sports existed) means basketball
    let old = r#"{"version":1,"pano_w":1920,"pano_h":1080,"rms_m":0.04,"points":[
      {"name":"a","pano":[0.1,0.3],"court":[0,0]},{"name":"b","pano":[0.9,0.3],"court":[28,0]},
      {"name":"c","pano":[0.95,0.8],"court":[28,15]},{"name":"d","pano":[0.05,0.8],"court":[0,15]}]}"#;
    let op = root.join("old.json");
    std::fs::write(&op, old).unwrap();
    let ol = CourtCalib::load(&op.to_string_lossy()).unwrap();
    assert_eq!(ol.sport, "basketball");
    assert_eq!((ol.profile().team_size, ol.profile().field.length), (5, 28.0));
}

#[test]
fn localize_finds_the_number_on_a_shirt() {
    use reco_minimap::tools::ocr::localize;
    // 60x40 red shirt crop with a white 12x14 block (the "number") off-centre, plus noise-free background
    let (w, h) = (60usize, 40usize);
    let mut rgb = vec![0u8; w * h * 3];
    for i in 0..w * h {
        rgb[i * 3..i * 3 + 3].copy_from_slice(&[200, 30, 30]);
    }
    for y in 10..24 {
        for x in 30..42 {
            rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[255, 255, 255]);
        }
    }
    let c = localize(w, h, &rgb);
    assert!(c.w >= 12 && c.w <= 24 && c.h >= 14 && c.h <= 30, "crop {}x{}", c.w, c.h);
    // plain shirt without any marking: the whole crop comes back unchanged
    let plain = vec![200u8; w * h * 3];
    let c = localize(w, h, &plain);
    assert_eq!((c.w, c.h), (w, h));
}

#[test]
fn ui_only_accepts_its_own_page() {
    use reco_minimap::tools::ui::request_allowed;
    assert!(request_allowed(Some("127.0.0.1:8788"), None)); // curl, scripts
    assert!(request_allowed(Some("127.0.0.1:8788"), Some("http://127.0.0.1:8788")));
    assert!(request_allowed(Some("localhost:8788"), Some("http://localhost:8788")));
    assert!(request_allowed(Some("[::1]:8788"), None));
    // another web page posting to the local server
    assert!(!request_allowed(Some("127.0.0.1:8788"), Some("https://example.com")));
    assert!(!request_allowed(Some("127.0.0.1:8788"), Some("http://127.0.0.1:9999")));
    assert!(!request_allowed(Some("127.0.0.1:8788"), Some("null")));
    // DNS rebinding: a foreign name that resolves to this machine
    assert!(!request_allowed(Some("evil.example:8788"), Some("http://evil.example:8788")));
    assert!(!request_allowed(Some("127.0.0.1.evil.example"), None));
    assert!(!request_allowed(None, None));
}

#[test]
fn torso_colour_snaps_to_a_known_shirt_colour() {
    use reco_minimap::detect::{Class, Det};
    use reco_minimap::tools::video::{Frame, TORSO_BAND};
    // a 100x200 frame: one box filling it; the torso band is 45 % red shirt, 55 % skin tone
    let (w, h) = (100usize, 200usize);
    let (red, skin, blue) = ([200u8, 30, 40], [224u8, 172, 140], [30u8, 60, 200]);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let c = if x < 46 { red } else { skin };
            rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
        }
    }
    let frame = Frame { w, h, rgb };
    let d = Det::new(Class::Player, 0.9, 0.5, 0.5, 1.0, 1.0);
    // the plain median lands on the majority, the skin
    assert_eq!(frame.torso_color(&d), Some(skin));
    // knowing the shirts, the red pixels are a clear share and win
    assert_eq!(frame.torso_color_matching(&d, TORSO_BAND, &[red, blue]), Some(red));
    // neither shirt present: unchanged
    assert_eq!(frame.torso_color_matching(&d, TORSO_BAND, &[blue, [0, 0, 0]]), Some(skin));
}

#[test]
fn court_paint_in_a_team_colour_is_not_taken_for_a_shirt() {
    use reco_minimap::detect::{Class, Det};
    use reco_minimap::tools::video::{Frame, TORSO_BAND};
    // red floor everywhere; a player box in the middle whose torso band is 40 % blue shirt, 60 % red floor showing through
    let (w, h) = (300usize, 300usize);
    let (red, blue) = ([200u8, 30, 40], [30u8, 60, 200]);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let shirt = (130..146).contains(&x) && (100..200).contains(&y);
            rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(if shirt { &blue } else { &red });
        }
    }
    let frame = Frame { w, h, rgb };
    let d = Det::new(Class::Player, 0.9, 0.5, 0.5, 100.0 / 300.0, 100.0 / 300.0);
    assert_eq!(frame.torso_color(&d), Some(red), "the plain median is the floor");
    assert_eq!(frame.torso_color_matching(&d, TORSO_BAND, &[red, blue]), Some(blue));

    // a red shirt on the red floor is still red: no other shirt colour is in the box
    let mut rgb = vec![0u8; w * h * 3];
    for px in rgb.chunks_exact_mut(3) {
        px.copy_from_slice(&red);
    }
    let frame = Frame { w, h, rgb };
    assert_eq!(frame.torso_color_matching(&d, TORSO_BAND, &[red, blue]), Some(red));
}

#[test]
fn update_versions_compare_and_parse() {
    use reco_minimap::tools::update::{is_newer, parse_version};
    assert_eq!(parse_version("v0.3.0"), Some([0, 3, 0]));
    assert_eq!(parse_version("1.2"), Some([1, 2, 0]));
    assert_eq!(parse_version("2.0.1-rc1"), Some([2, 0, 1]));
    assert_eq!(parse_version("release"), None);
    assert_eq!(parse_version("1.2.3.4"), None);
    assert!(is_newer("0.3.0", "0.2.9"));
    assert!(is_newer("v0.10.0", "0.9.5"), "numbers, not text");
    assert!(!is_newer("0.2.0", "0.2.0"));
    assert!(!is_newer("0.1.9", "0.2.0"));
    assert!(!is_newer("nightly", "0.2.0"), "an unreadable version is never an update");
}

#[test]
fn update_reads_githubs_answers() {
    use reco_minimap::tools::update::{install_args, latest_from_release, latest_from_tags};
    use serde_json::json;
    let rel = json!({"tag_name": "v0.3.0", "html_url": "https://github.com/wendibus/reco-minimap/releases/tag/v0.3.0", "draft": false, "prerelease": false});
    let l = latest_from_release(&rel).unwrap();
    assert_eq!((l.version.as_str(), l.tag.as_str()), ("0.3.0", "v0.3.0"));
    assert!(l.url.ends_with("/v0.3.0"));
    // drafts, pre-releases and "not found" are not versions to install
    assert!(latest_from_release(&json!({"tag_name": "v0.4.0", "prerelease": true})).is_none());
    assert!(latest_from_release(&json!({"message": "Not Found"})).is_none());
    // a tag that is not a plain version name never reaches cargo
    assert!(latest_from_release(&json!({"tag_name": "v1.0.0 --config evil"})).is_none());

    let tags = json!([{"name": "v0.2.0"}, {"name": "v0.10.1"}, {"name": "v0.9.0"}, {"name": "experiment"}]);
    assert_eq!(latest_from_tags(&tags).unwrap().tag, "v0.10.1");
    assert!(latest_from_tags(&json!([])).is_none());

    let a = install_args(&l);
    let at = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].as_str());
    assert_eq!(a[0], "install");
    assert_eq!(at("--git"), Some("https://github.com/wendibus/reco-minimap"));
    assert_eq!(at("--tag"), Some("v0.3.0"));
    assert!(at("--features").unwrap().starts_with("tools"));
}
