//! Writes one PPM per sport profile (court markings only) to the given directory - a quick visual check.
use reco_minimap::ball::{BallOut, BallState};
use reco_minimap::render::{Renderer, Theme};
use reco_minimap::sport::{SportProfile, BUILTIN_IDS};
use reco_minimap::FrameState;
fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp".into());
    for id in BUILTIN_IDS {
        let p = SportProfile::builtin(id).unwrap();
        let theme = Theme { px_per_m: Renderer::default_px_per_m(&p), ..Theme::default() };
        let s = FrameState { t: 0.0, players: vec![], referees: vec![], ball: BallOut { state: BallState::Unknown, pos: None, vel: None }, events: vec![] };
        let c = Renderer::with_profile(theme, p).render(&s);
        std::fs::write(format!("{dir}/sport_{id}.ppm"), c.write_ppm()).unwrap();
    }
}
