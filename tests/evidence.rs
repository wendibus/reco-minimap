#![cfg(feature = "tools")]
use reco_minimap::{
    detect::{Class, Det, Norm, Tile},
    geom::{Projector, Vec2},
    tools::{
        cache,
        ocr::{crop_quality, Crop, Reading},
        video::Frame,
    },
    Config, Minimap,
};
use serde_json::json;
struct Projection;
impl Projector for Projection {
    fn to_court(&self, u: f32, v: f32) -> Option<Vec2> {
        Some(Vec2::new(u * 28.0, v * 15.0))
    }
}
fn player() -> Det {
    let mut d = Det::new(Class::Player, 0.9, 0.4, 0.5, 0.1, 0.2);
    d.color = Some([255, 0, 0]);
    d
}

#[test]
fn cached_ocr_rejects_wrong_segment_team_box_and_duplicate_records() {
    let mut mm = Minimap::new(Config::new([255, 0, 0], [0, 0, 255]), Box::new(Projection));
    mm.process(0.0, &[player()]);
    mm.process(0.1, &[player()]);
    let tr = &mm.tracker().tracks()[0];
    let reading = Reading {
        steps: 1,
        probs: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    };
    let good = cache::record(tr, &reading, 1.0);
    assert_eq!(
        cache::apply(&mut mm, &[good.clone(), good.clone()], 0.1),
        (1, 1)
    );
    for (key, value) in [
        ("segment", json!(99)),
        ("team", json!("away")),
        ("bbox", json!([0.8, 0.5, 0.1, 0.2])),
        ("probs", json!([1.0])),
        ("steps", json!(0)),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        assert_eq!(cache::apply(&mut mm, &[bad], 0.1), (0, 1));
    }
    mm.process(0.2, &[]);
    assert_eq!(cache::apply(&mut mm, &[good], 0.2), (0, 1));
}

#[test]
fn preprocessing_reuse_preserves_values_and_clears_old_padding() {
    let frame = Frame {
        w: 8,
        h: 8,
        rgb: (0..192).map(|v| v as u8).collect(),
    };
    let tile = Tile {
        x: 0,
        y: 0,
        w: 8,
        h: 4,
    };
    for norm in [Norm::ImageNet, Norm::Unit] {
        let expected = frame.tile_tensor(&tile, 8, norm);
        let mut reused = vec![99.0; 192];
        frame.tile_tensor_into(&tile, 8, norm, &mut reused);
        assert_eq!(reused, expected);
    }
}

#[test]
fn crop_ranking_rejects_flat_and_rewards_edges() {
    let flat = Crop {
        w: 16,
        h: 16,
        rgb: vec![100; 16 * 16 * 3],
    };
    let mut patterned = Crop {
        w: 16,
        h: 16,
        rgb: flat.rgb.clone(),
    };
    for y in 0..16 {
        for x in 0..16 {
            let v = if x % 4 < 2 { 230 } else { 20 };
            patterned.rgb[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3].fill(v);
        }
    }
    assert_eq!(crop_quality(&flat), 0.0);
    assert!(crop_quality(&patterned) > 1.0);
}

#[test]
fn preprocessing_has_the_original_native_pixel_and_padding_values() {
    let frame = Frame {
        w: 8,
        h: 8,
        rgb: (0..192).map(|v| v as u8).collect(),
    };
    let tile = Tile {
        x: 0,
        y: 0,
        w: 8,
        h: 4,
    };
    let unit = frame.tile_tensor(&tile, 8, Norm::Unit);
    assert!((unit[0] - 114.0 / 255.0).abs() < 1e-6);
    assert_eq!(unit[16], 0.0);
    assert!((unit[64 + 16] - 1.0 / 255.0).abs() < 1e-6);
    let imagenet = frame.tile_tensor(&tile, 8, Norm::ImageNet);
    assert_eq!(imagenet[0], 0.0);
    assert!((imagenet[16] - (-0.485 / 0.229)).abs() < 1e-6);
    let down = frame.tile_tensor(&tile, 4, Norm::Unit);
    assert!((down[4] - 13.5 / 255.0).abs() < 1e-6);
}
