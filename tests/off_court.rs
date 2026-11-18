//! People standing outside the lines for longer than an inbound takes (bench, scorer's table) are
//! left out of the players; a short inbound outside the line is not.
use reco_minimap::{
    detect::{Class, Det},
    geom::{Projector, Vec2},
    Config, Minimap,
};

struct Projection;
impl Projector for Projection {
    // image v 0.1..0.9 -> court y 0..15
    fn to_court(&self, u: f32, v: f32) -> Option<Vec2> {
        Some(Vec2::new(u * 28.0, (v - 0.1) * 15.0 / 0.8))
    }
}

fn person(cx: f32, foot: f32) -> Det {
    let mut d = Det::new(Class::Player, 0.9, cx, foot - 0.05, 0.03, 0.1);
    d.color = Some([255, 0, 0]);
    d
}

#[test]
fn the_bench_drops_out_an_inbound_stays() {
    let mut mm = Minimap::new(Config::new([255, 0, 0], [0, 0, 255]), Box::new(Projection));
    let mut seen = Vec::new();
    for i in 0..40 {
        let t = i as f64 * 0.5;
        // on the court the whole time; on the bench (0.6 m outside) the whole time; an inbounder
        // outside the line for 4 s (t 6..10), on the court otherwise
        let inbound_out = (6.0..10.0).contains(&t);
        let dets = [person(0.5, 0.5), person(0.8, 0.068), person(0.2, if inbound_out { 0.07 } else { 0.2 })];
        let s = mm.process(t, &dets);
        seen.push((t, s.players.iter().map(|p| p.pos.x.round() as i32).collect::<Vec<_>>()));
    }
    let at = |t: f64| {
        let mut v = seen.iter().find(|x| (x.0 - t).abs() < 1e-6).unwrap().1.clone();
        v.sort_unstable();
        v
    };
    // court x: inbounder 6, court player 14, bench 22
    assert_eq!(at(4.0), vec![6, 14, 22], "{seen:?}");
    assert_eq!(at(9.5), vec![6, 14], "bench left out, the inbounder still counted: {seen:?}");
    assert_eq!(at(15.0), vec![6, 14], "{seen:?}");
}
