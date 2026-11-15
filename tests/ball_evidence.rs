use reco_minimap::{
    ball::{BallCfg, BallObs, BallState, BallTracker},
    geom::Vec2,
    team::Team,
    tracker::{BBox, Obs, Tracker, TrackerCfg},
};

fn observation(x: f32, y: f32) -> BallObs {
    BallObs {
        bbox: BBox {
            cx: x,
            cy: y,
            w: 0.01,
            h: 0.01,
        },
        court: Vec2::new(x * 28.0, y * 15.0),
        conf: 0.9,
    }
}
fn people() -> Tracker {
    let mut tracker = Tracker::new(TrackerCfg::default());
    let obs = Obs {
        referee: false,
        team: Team::Home,
        keeper: false,
        pos: Vec2::new(10., 7.),
        bbox: BBox {
            cx: 0.4,
            cy: 0.5,
            w: 0.1,
            h: 0.3,
        },
        conf: 0.9,
    };
    tracker.update(0.0, &[obs.clone()]);
    tracker.update(0.04, &[obs]);
    tracker
}

#[test]
fn a_gap_cannot_join_two_release_observations() {
    let people = people();
    let mut ball = BallTracker::new(BallCfg::default());
    ball.update(0.0, Some(observation(0.4, 0.45)), people.tracks());
    assert!(matches!(
        ball.update(0.04, Some(observation(0.4, 0.45)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
    ball.update(0.08, Some(observation(0.4, 0.2)), people.tracks());
    ball.update(0.12, None, people.tracks());
    assert!(matches!(
        ball.update(0.16, Some(observation(0.4, 0.2)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
    assert!(matches!(
        ball.update(0.20, Some(observation(0.4, 0.2)), people.tracks())
            .state,
        BallState::InFlight
    ));
}

#[test]
fn sparse_observations_and_seeks_do_not_accumulate_false_possession() {
    let people = people();
    let mut ball = BallTracker::new(BallCfg::default());
    ball.update(0.0, Some(observation(0.4, 0.45)), people.tracks());
    assert!(!matches!(
        ball.update(0.8, Some(observation(0.4, 0.45)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
    assert!(matches!(
        ball.update(0.84, Some(observation(0.4, 0.45)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
    assert_eq!(
        ball.update(-1.0, None, people.tracks()).state,
        BallState::Unknown
    );
    assert_eq!(
        ball.update(f64::NAN, None, people.tracks()).state,
        BallState::Unknown
    );
}

#[test]
fn higher_ball_cadence_does_not_confirm_a_five_millisecond_contact() {
    let people = people();
    let mut ball = BallTracker::new(BallCfg::default());
    ball.update(0.0, Some(observation(0.4, 0.45)), people.tracks());
    assert!(!matches!(
        ball.update(0.005, Some(observation(0.4, 0.45)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
    assert!(matches!(
        ball.update(0.025, Some(observation(0.4, 0.45)), people.tracks())
            .state,
        BallState::Held { .. }
    ));
}
