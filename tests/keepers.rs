//! Goalkeeper shirt colours: a keeper in a different colour belongs to his team (and is not taken for a referee).
use reco_minimap::detect::*;
use reco_minimap::geom::*;
use reco_minimap::team::{Team, TeamClassifier};
use reco_minimap::tools::colors::{pick_keepers, PlayerSample};
use reco_minimap::*;

const HOME: [u8; 3] = [240, 240, 240]; // white
const AWAY: [u8; 3] = [40, 110, 230]; // blue
const HOME_GK: [u8; 3] = [30, 160, 60]; // green
const AWAY_GK: [u8; 3] = [250, 200, 0]; // yellow
const BLACK: [u8; 3] = [10, 10, 10];

#[test]
fn keeper_colour_maps_to_its_team() {
    let c = TeamClassifier::new(HOME, AWAY).with_keepers(Some(HOME_GK), Some(AWAY_GK));
    assert_eq!(c.classify_role(Some(HOME)), (Team::Home, false));
    assert_eq!(c.classify_role(Some(AWAY)), (Team::Away, false));
    assert_eq!(c.classify_role(Some(HOME_GK)), (Team::Home, true));
    assert_eq!(c.classify_role(Some(AWAY_GK)), (Team::Away, true));
    assert_eq!(c.classify(Some(HOME_GK)), Team::Home);
    assert!(!c.is_outlier(HOME_GK) && !c.is_outlier(AWAY_GK));
}

#[test]
fn without_keeper_colours_the_keeper_is_an_outlier() {
    let c = TeamClassifier::new(HOME, AWAY).with_referee(BLACK);
    assert_eq!(c.classify(Some(HOME_GK)), Team::Unknown);
    assert!(c.is_outlier(HOME_GK));
    // only one team has a keeper kit: the other stays as before
    let c = TeamClassifier::new(HOME, AWAY).with_keepers(Some(HOME_GK), None);
    assert_eq!(c.classify(Some(HOME_GK)), Team::Home);
    assert!(c.is_outlier(AWAY_GK));
}

#[test]
fn a_black_keeper_kit_is_not_a_referee_once_it_is_declared() {
    let c = TeamClassifier::new(HOME, AWAY).with_referee(BLACK);
    assert!(c.is_referee_colored(BLACK));
    let c = c.with_keepers(Some(BLACK), None);
    // equally close to the referee and to the keeper: the strict "closer than any team" test no longer fires
    assert!(!c.is_referee_colored(BLACK));
    assert_eq!(c.classify(Some(BLACK)), Team::Home);
}

fn projector() -> (HomographyProjector, Homography) {
    let court = [Vec2::new(0.0, 0.0), Vec2::new(28.0, 0.0), Vec2::new(28.0, 15.0), Vec2::new(0.0, 15.0), Vec2::new(14.0, 7.5)];
    let pano = [Vec2::new(0.05, 0.30), Vec2::new(0.95, 0.30), Vec2::new(0.99, 0.80), Vec2::new(0.01, 0.80), Vec2::new(0.50, 0.53936)];
    (HomographyProjector::calibrate(&pano, &court).unwrap(), Homography::from_points(&court, &pano).unwrap())
}

fn det(to_pano: &Homography, p: Vec2, class: Class, color: Option<[u8; 3]>) -> Det {
    let f = to_pano.apply(p).unwrap();
    let mut d = Det::new(class, 0.9, f.x, f.y - 0.045, 0.012, 0.09);
    d.color = color;
    d
}

#[test]
fn a_declared_keeper_is_tracked_as_a_team_player_not_a_referee() {
    let run = |declare: bool| {
        let (proj, to_pano) = projector();
        let mut cfg = Config::new(HOME, AWAY);
        cfg.referee_rgb = Some(BLACK);
        if declare {
            cfg.home_gk_rgb = Some(HOME_GK);
        }
        let mut mm = Minimap::new(cfg, Box::new(proj));
        let mut last = None;
        for k in 0..12 {
            let mut ds = Vec::new();
            for i in 0..4 {
                ds.push(det(&to_pano, Vec2::new(8.0 + 2.0 * i as f32, 3.0 + 2.5 * i as f32), Class::Player, Some(HOME)));
                ds.push(det(&to_pano, Vec2::new(14.0 + 2.0 * i as f32, 4.0 + 2.5 * i as f32), Class::Player, Some(AWAY)));
            }
            ds.push(det(&to_pano, Vec2::new(1.5, 7.5 + 0.05 * k as f32), Class::Player, Some(HOME_GK)));
            last = Some(mm.process(k as f64 * 0.2, &ds));
        }
        last.unwrap()
    };
    let with = run(true);
    assert_eq!(with.players.iter().filter(|p| p.team == Team::Home).count(), 5, "keeper counts as the fifth home player");
    assert!(with.referees.is_empty());
    let without = run(false);
    assert_eq!(without.players.iter().filter(|p| p.team == Team::Home).count(), 4, "undeclared, the keeper is lost to the home team");
}

#[test]
fn keeper_colours_are_found_and_assigned_even_after_the_sides_change() {
    let field = Field::new(100.0, 68.0);
    // the team on the keeper's side is the keeper's team - per frame, so changing ends does not matter
    for pattern in [[true; 8], [false; 8], [true, true, true, true, false, false, false, false]] {
        let mut s = Vec::new();
        for (f, left) in pattern.iter().enumerate() {
            // home outfield stands on its own half: near its own keeper
            let (hx, ax) = if *left { (15.0, 60.0) } else { (60.0, 15.0) };
            let (hgk, agk) = if *left { (2.0, 98.0) } else { (98.0, 2.0) };
            for i in 0..5 {
                s.push(PlayerSample { frame: f, pos: Vec2::new(hx + 3.0 * i as f32, 15.0 + 8.0 * i as f32), rgb: HOME });
                s.push(PlayerSample { frame: f, pos: Vec2::new(ax + 3.0 * i as f32, 17.0 + 8.0 * i as f32), rgb: AWAY });
            }
            s.push(PlayerSample { frame: f, pos: Vec2::new(hgk, 34.0), rgb: HOME_GK });
            s.push(PlayerSample { frame: f, pos: Vec2::new(agk, 34.0), rgb: AWAY_GK });
        }
        let pick = pick_keepers(&s, HOME, AWAY, &field);
        let (h, a) = (pick.home.expect("home keeper"), pick.away.expect("away keeper"));
        let near = |c: [u8; 3], t: [u8; 3]| reco_minimap::tools::colors::lab_dist(c, t) < 10.0;
        assert!(near(h.rgb, HOME_GK), "{pattern:?}: home keeper {:?}", h.rgb);
        assert!(near(a.rgb, AWAY_GK), "{pattern:?}: away keeper {:?}", a.rgb);
        assert!(pick.confident);
    }
}

#[test]
fn no_keeper_colour_when_nobody_deviates() {
    let field = Field::new(100.0, 68.0);
    let mut s = Vec::new();
    for f in 0..8 {
        for i in 0..5 {
            s.push(PlayerSample { frame: f, pos: Vec2::new(10.0 + 3.0 * i as f32, 15.0 + 8.0 * i as f32), rgb: HOME });
            s.push(PlayerSample { frame: f, pos: Vec2::new(60.0 + 3.0 * i as f32, 17.0 + 8.0 * i as f32), rgb: AWAY });
        }
        s.push(PlayerSample { frame: f, pos: Vec2::new(2.0, 34.0), rgb: HOME });
    }
    let pick = pick_keepers(&s, HOME, AWAY, &field);
    assert!(pick.home.is_none() && pick.away.is_none());
}
