use reco_minimap::ball::{BallOut, BallState};
use reco_minimap::geom::Vec2;
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::team::Team;
use reco_minimap::tracker::TrackState;
use reco_minimap::{AgentOut, FrameState};
fn main() {
    let ag = |id, referee, team, x, y, alpha, st| AgentOut { segment: 0, keeper: false, identity: Default::default(), id, referee, team, pos: Vec2::new(x, y), vel: Vec2::default(), state: st, alpha, jersey: None };
    let v = TrackState::Visible;
    let o = TrackState::Occluded;
    let s = FrameState {
        t: 0.0,
        players: vec![ag(1, false, Team::Home, 8.0, 4.0, 1.0, v), ag(2, false, Team::Home, 10.0, 7.0, 0.35, o), ag(3, false, Team::Home, 12.0, 10.0, 1.0, v),
                      ag(4, false, Team::Away, 15.0, 4.0, 1.0, v), ag(5, false, Team::Away, 17.0, 7.0, 0.35, o), ag(6, false, Team::Away, 19.0, 10.0, 1.0, v)],
        referees: vec![ag(7, true, Team::Unknown, 14.0, 0.5, 1.0, v), ag(8, true, Team::Unknown, 20.0, 14.5, 1.0, v)],
        ball: BallOut { state: BallState::Held { track: 3 }, pos: None, vel: None },
        events: vec![],
    };
    for (name, min_alpha) in [("old", 0.0), ("new", 0.8)] {
        let mut th = Theme { home: [255, 255, 255], away: [26, 42, 108], min_alpha, ..Theme::default() };
        th.px_per_m = 22.0;
        let c = Renderer::new(th).render(&s);
        std::fs::write(format!("/tmp/legend_{name}.ppm"), c.write_ppm()).unwrap();
    }
}
