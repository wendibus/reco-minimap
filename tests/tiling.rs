#![cfg(feature = "tools")]
use reco_minimap::{detect::Tile, tools::scales::{Scales, plan_auto, plan_auto_roi}};

fn steep() -> Scales {
    // Regression: the original six-band cap stopped around row 781, not 1006.
    Scales { a: -245.61208, b: 0.4043302, y_min: 651.54584, y_max: 1006.09106, target_px: 28.0, n: 151 }
}

#[test]
fn steep_fit_covers_the_entire_requested_field() {
    let tiles = plan_auto(&steep(),2560,1440,0.25);
    for y in 652..=1006 {
        for x in (0..2560).step_by(31) {
            assert!(tiles.iter().any(|t| x >= t.x && x < t.x+t.w && y >= t.y && y < t.y+t.h), "Uncovered {x},{y}");
        }
    }
    assert!(tiles.iter().all(|t| t.w>0 && t.h>0 && t.x+t.w<=2560 && t.y+t.h<=1440));
}

#[test]
fn auto_roi_is_respected_and_reduces_work_without_lowering_scale() {
    let s=steep();
    let all=plan_auto(&s,2560,1440,0.25);
    let roi=Tile{x:300,y:500,w:1900,h:600};
    let tiles=plan_auto_roi(&s,2560,1440,0.25,roi);
    assert!(tiles.len()<all.len());
    assert!(tiles.iter().all(|t| t.x>=roi.x && t.y>=roi.y && t.x+t.w<=2200 && t.y+t.h<=1100));
    for y in 652..=1006 {
        for x in (300..2200).step_by(31) {
            assert!(tiles.iter().any(|t| x>=t.x && x<t.x+t.w && y>=t.y && y<t.y+t.h));
        }
    }
}

#[test]
fn small_tail_and_invalid_scales_do_not_panic() {
    let mut s=steep();s.y_min=1000.0;s.y_max=1001.0;
    assert!(!plan_auto(&s,2560,1440,0.25).is_empty());
    s.target_px=0.0;assert!(plan_auto(&s,2560,1440,0.25).is_empty());
    s.target_px=28.0;s.b=f32::NAN;assert!(plan_auto(&s,2560,1440,0.25).is_empty());
    assert!(plan_auto(&steep(),0,0,0.25).is_empty());
    assert!(plan_auto_roi(&steep(),2560,1440,0.25,Tile{x:3000,y:0,w:u32::MAX,h:1440}).is_empty());
}
