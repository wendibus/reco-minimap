//! Renders a small demo of numbered dots to the given PPM path.
use reco_minimap::ball::{BallOut, BallState};
use reco_minimap::geom::Vec2;
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::team::Team;
use reco_minimap::tracker::TrackState;
use reco_minimap::{AgentOut, FrameState};
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/numbers_demo.ppm".into());
    let ag = |id: u32, team: Team, x: f32, y: f32, jersey: Option<u8>| AgentOut { segment: 0, identity: Default::default(), id, referee: false, team, pos: Vec2::new(x, y), vel: Vec2::default(), state: TrackState::Visible, alpha: 1.0, jersey };
    let home = [(5, 4u8), (7, 7), (9, 10), (11, 12), (13, 23)];
    let away = [(6, 3u8), (8, 8), (10, 15), (12, 6), (14, 99)];
    let mut players = Vec::new();
    for (i, (x, n)) in home.iter().enumerate() { players.push(ag(i as u32, Team::Home, *x as f32 + 4.0, 3.0 + i as f32 * 2.2, if i == 3 { None } else { Some(*n) })); }
    for (i, (x, n)) in away.iter().enumerate() { players.push(ag(10 + i as u32, Team::Away, *x as f32 + 8.0, 3.5 + i as f32 * 2.2, Some(*n))); }
    let s = FrameState { t: 0.0, players, referees: vec![], ball: BallOut { state: BallState::Held { track: 2 }, pos: None, vel: None }, events: vec![] };
    let theme = Theme { px_per_m: 34.0, home: [255, 255, 255], away: [47, 107, 255], show_numbers: true, ..Theme::default() };
    std::fs::write(path, Renderer::new(theme).render(&s).write_ppm()).unwrap();
}
