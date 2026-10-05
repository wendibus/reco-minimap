use reco_minimap::ball::BallState;
use reco_minimap::detect::*;
use reco_minimap::geom::*;
use reco_minimap::team::Team;
use reco_minimap::tracker::hungarian;
use reco_minimap::*;

const HOME: [u8; 3] = [220, 40, 40];
const AWAY: [u8; 3] = [40, 110, 230];

/// Synthetic planar camera: court metres -> normalized panorama (and back).
fn projector() -> (HomographyProjector, Homography) {
    let court = [Vec2::new(0.0, 0.0), Vec2::new(28.0, 0.0), Vec2::new(28.0, 15.0), Vec2::new(0.0, 15.0), Vec2::new(14.0, 7.5)];
    let pano = [Vec2::new(0.05, 0.30), Vec2::new(0.95, 0.30), Vec2::new(0.99, 0.80), Vec2::new(0.01, 0.80), Vec2::new(0.50, 0.53936)];
    let to_court = HomographyProjector::calibrate(&pano, &court).unwrap();
    let to_pano = Homography::from_points(&court, &pano).unwrap();
    (to_court, to_pano)
}

fn player_det(to_pano: &Homography, p: Vec2, class: Class, color: Option<[u8; 3]>) -> Det {
    let f = to_pano.apply(p).unwrap();
    let (w, h) = (0.012, 0.09);
    let mut d = Det::new(class, 0.9, f.x, f.y - h / 2.0, w, h);
    d.color = color;
    d
}

#[test]
fn homography_roundtrip() {
    let (p, _) = projector();
    let c = p.to_court(0.5, 0.53936).unwrap();
    assert!((c.x - 14.0).abs() < 0.01 && (c.y - 7.5).abs() < 0.01, "{:?}", c);
    let inv = p.pano_to_court.inverse().unwrap();
    let back = inv.apply(c).unwrap();
    assert!((back.x - 0.5).abs() < 1e-3);
}

#[test]
fn decoder_maps_classes_and_filters() {
    let cfg = DecodeCfg::basketball_v1();
    // 3 queries: player (logit high), ignore (high), ball below threshold
    let boxes = [0.5, 0.5, 0.1, 0.2, 0.3, 0.3, 0.1, 0.1, 0.7, 0.7, 0.02, 0.02];
    let logits = [-5.0, 4.0, -3.0, -4.0, /* q1 */ -5.0, -5.0, -5.0, 3.0, /* q2 */ -2.0, -4.0, -4.0, -4.0];
    let d = decode_rfdetr(&boxes, &logits, &cfg).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].class, Class::Player);
    assert!(d[0].conf > 0.97);
    assert!(decode_rfdetr(&boxes, &logits[..5], &cfg).is_err());
}

#[test]
fn tiling_covers_roi_and_maps_back() {
    let roi = Tile { x: 0, y: 600, w: 4096, h: 800 };
    let tiles = plan_tiles(roi, 512, 0.25);
    assert!(tiles.iter().all(|t| t.x + t.w <= 4096 && t.y + t.h <= 1400 && t.y >= 600));
    assert_eq!(tiles.iter().map(|t| t.x + t.w).max().unwrap(), 4096); // right edge covered
    let d = Det::new(Class::Ball, 0.9, 0.5, 0.5, 0.02, 0.02);
    let m = tile_to_pano(&d, &Tile { x: 1024, y: 700, w: 512, h: 512 }, 4096, 1500);
    assert!((m.cx - (1024.0 + 256.0) / 4096.0).abs() < 1e-6);
}

#[test]
fn nms_removes_tile_duplicates() {
    let a = Det::new(Class::Player, 0.9, 0.5, 0.5, 0.02, 0.1);
    let b = Det::new(Class::Player, 0.7, 0.502, 0.5, 0.02, 0.1);
    let c = Det::new(Class::Referee, 0.8, 0.5, 0.5, 0.02, 0.1);
    assert_eq!(nms(vec![a, b, c], 0.5).len(), 2);
}

#[test]
fn hungarian_is_optimal() {
    let c = vec![vec![4.0, 1.0, 3.0], vec![2.0, 0.0, 5.0], vec![3.0, 2.0, 2.0]];
    let r = hungarian(&c);
    let total: f64 = r.iter().enumerate().map(|(i, j)| c[i][j.unwrap()]).sum();
    assert_eq!(total, 5.0);
    // more rows than cols
    let r = hungarian(&[vec![1.0], vec![0.5], vec![2.0]]);
    assert_eq!(r.iter().filter(|x| x.is_some()).count(), 1);
    assert_eq!(r[1], Some(0));
}

fn lineup() -> (Vec<Vec2>, Vec<Vec2>, Vec<Vec2>) {
    let home = (0..5).map(|i| Vec2::new(8.0 + 2.0 * i as f32, 3.0 + 2.2 * i as f32)).collect();
    let away = (0..5).map(|i| Vec2::new(9.0 + 2.0 * i as f32, 3.5 + 2.2 * i as f32)).collect();
    let refs = vec![Vec2::new(14.0, 0.5), Vec2::new(20.0, 14.5)];
    (home, away, refs)
}

fn frame_dets(to_pano: &Homography, home: &[Vec2], away: &[Vec2], refs: &[Vec2], hide: &[usize]) -> Vec<Det> {
    let mut v = Vec::new();
    for (i, p) in home.iter().enumerate() {
        if !hide.contains(&i) {
            v.push(player_det(to_pano, *p, Class::Player, Some(HOME)));
        }
    }
    for p in away {
        v.push(player_det(to_pano, *p, Class::Player, Some(AWAY)));
    }
    for p in refs {
        v.push(player_det(to_pano, *p, Class::Referee, None));
    }
    v
}

#[test]
fn five_five_two_slots_are_stable() {
    let (proj, to_pano) = projector();
    let mut mm = Minimap::new(Config::new(HOME, AWAY), Box::new(proj));
    let (home, away, refs) = lineup();
    let mut last = None;
    for k in 0..50 {
        last = Some(mm.process(k as f64 * 0.04, &frame_dets(&to_pano, &home, &away, &refs, &[])));
    }
    let s = last.unwrap();
    assert_eq!(s.players.iter().filter(|p| p.team == Team::Home).count(), 5);
    assert_eq!(s.players.iter().filter(|p| p.team == Team::Away).count(), 5);
    assert_eq!(s.referees.len(), 2);
    // a 6th detection with home colour must NOT create a 6th dot
    let mut d = frame_dets(&to_pano, &home, &away, &refs, &[]);
    d.push(player_det(&to_pano, Vec2::new(3.0, 12.0), Class::Player, Some(HOME)));
    let s = mm.process(2.1, &d);
    assert_eq!(s.players.iter().filter(|p| p.team == Team::Home).count(), 5);
}

#[test]
fn lost_player_coasts_plausibly_and_reacquires_without_teleport() {
    let (proj, to_pano) = projector();
    let mut mm = Minimap::new(Config::new(HOME, AWAY), Box::new(proj));
    let (mut home, away, refs) = lineup();
    let dt = 0.04f64;
    let mut t = 0.0;
    // player 0 runs +x at 1.5 m/s (stays on court for 12 s); others stand
    let mut speed_max = 0.0f32;
    let mut prev: Option<Vec2> = None;
    let mut track_id = None;
    for k in 0..300 {
        home[0].x = 8.0 + 1.5 * (k as f32 * dt as f32);
        let hide: &[usize] = if (75..175).contains(&k) { &[0] } else { &[] }; // 4 s occlusion
        let s = mm.process(t, &frame_dets(&to_pano, &home, &away, &refs, hide));
        t += dt;
        // find "our" player = home track nearest the true position at start; remember its id
        if k == 10 {
            track_id = s.players.iter().filter(|p| p.team == Team::Home).min_by(|a, b| a.pos.dist(home[0]).partial_cmp(&b.pos.dist(home[0])).unwrap()).map(|p| p.id);
        }
        if let Some(id) = track_id {
            let p = s.players.iter().find(|p| p.id == id).expect("slot must persist");
            if let Some(q) = prev {
                speed_max = speed_max.max(p.pos.dist(q) / dt as f32);
            }
            prev = Some(p.pos);
            assert!(in_court(p.pos, 1.0), "left court: {:?}", p.pos);
            if k == 150 {
                assert!(p.state != reco_minimap::tracker::TrackState::Visible); // occluded now
                assert!(p.alpha < 1.0);
            }
        }
        // slot count never exceeds 5
        assert!(s.players.iter().filter(|p| p.team == Team::Home).count() <= 5);
    }
    assert!(speed_max <= 9.0 * 1.35, "display speed too high: {speed_max}");
    // after reacquire the same id follows the true position again
    let s = mm.process(t, &frame_dets(&to_pano, &home, &away, &refs, &[]));
    let p = s.players.iter().find(|p| Some(p.id) == track_id).unwrap();
    assert!(p.pos.dist(home[0]) < 3.0, "did not reconverge: {:?} vs {:?}", p.pos, home[0]);
}

#[test]
fn ball_held_then_thrown_then_loose() {
    let (proj, to_pano) = projector();
    let mut mm = Minimap::new(Config::new(HOME, AWAY), Box::new(proj));
    let (home, away, refs) = lineup();
    let mut t = 0.0;
    let mut state = None;
    let ball_at_hands = |p: Vec2| {
        let f = to_pano.apply(p).unwrap();
        Det::new(Class::Ball, 0.8, f.x, f.y - 0.05, 0.004, 0.007)
    };
    // 1 s: ball in home[2]'s hands
    for _ in 0..25 {
        let mut d = frame_dets(&to_pano, &home, &away, &refs, &[]);
        d.push(ball_at_hands(home[2]));
        state = Some(mm.process(t, &d));
        t += 0.04;
    }
    let s = state.take().unwrap();
    let BallState::Held { track } = s.ball.state else { panic!("expected Held, got {:?}", s.ball.state) };
    let holder = s.players.iter().find(|p| p.id == track).unwrap();
    assert!(holder.pos.dist(home[2]) < 1.0);
    // ball hidden in hands for 1 s: still held
    for _ in 0..25 {
        state = Some(mm.process(t, &frame_dets(&to_pano, &home, &away, &refs, &[])));
        t += 0.04;
    }
    assert!(matches!(state.take().unwrap().ball.state, BallState::Held { .. }));
    // thrown: flies away fast in the air above everyone
    let mut flying = false;
    for k in 0..12 {
        let x = 12.0 + 1.0 * k as f32;
        let f = to_pano.apply(Vec2::new(x, 5.0)).unwrap();
        let mut d = frame_dets(&to_pano, &home, &away, &refs, &[]);
        d.push(Det::new(Class::Ball, 0.8, f.x, f.y - 0.22, 0.004, 0.007));
        let s = mm.process(t, &d);
        t += 0.04;
        if s.ball.state == BallState::InFlight {
            flying = true;
        }
    }
    assert!(flying, "throw not detected");
}

#[test]
fn renders_and_exports_ppm() {
    let (proj, to_pano) = projector();
    let mut mm = Minimap::new(Config::new(HOME, AWAY), Box::new(proj));
    let (home, away, refs) = lineup();
    let mut s = None;
    for k in 0..10 {
        let mut d = frame_dets(&to_pano, &home, &away, &refs, &[]);
        let f = to_pano.apply(home[1]).unwrap();
        d.push(Det::new(Class::Ball, 0.8, f.x, f.y - 0.05, 0.004, 0.007));
        s = Some(mm.process(k as f64 * 0.04, &d));
    }
    let r = render::Renderer::new(render::Theme::default());
    let c = r.render(&s.unwrap());
    assert_eq!((c.w, c.h), r.size());
    let ppm = c.write_ppm();
    assert!(ppm.len() > c.w * c.h * 3);
    std::fs::write(std::env::temp_dir().join("minimap_test.ppm"), ppm).unwrap();
}
