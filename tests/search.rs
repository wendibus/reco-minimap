use reco_minimap::{
    detect::{Class, Det, Tile},
    search::MotionSearch,
};

fn tiles() -> Vec<Tile> {
    (0..8)
        .map(|i| Tile {
            x: i * 100,
            y: 0,
            w: 100,
            h: 100,
        })
        .collect()
}
fn person(x: f32) -> Det {
    Det::new(Class::Player, 0.8, x, 0.5, 0.03, 0.1)
}

#[test]
fn missing_people_keep_a_search_region_then_expire() {
    let ts = tiles();
    let mut s = MotionSearch::new(2.0);
    assert!(s.select(0.0, &ts, 800, 100, 0).full_sweep);
    s.observe(0.0, &[person(0.2)]);
    s.observe(0.1, &[]);
    let coast = s.select(0.2, &ts, 800, 100, 0);
    assert!(!coast.full_sweep);
    assert_eq!(coast.coasting_targets, 1);
    assert!(coast.indices.contains(&1));
    assert!(s.select(1.1, &ts, 800, 100, 0).full_sweep);
}

#[test]
fn motion_search_covers_new_location_before_next_detection() {
    let ts = tiles();
    let mut s = MotionSearch::new(2.0);
    s.select(0.0, &ts, 800, 100, 0);
    s.observe(0.0, &[person(0.10)]);
    s.observe(0.1, &[person(0.18)]);
    let moving = s.select(0.2, &ts, 800, 100, 0);
    assert!(
        moving.indices.contains(&2),
        "predicted motion must cross into tile 2: {:?}",
        moving
    );
    assert!(moving.indices.contains(&1));
}

#[test]
fn independent_recovery_and_full_sweeps_remain_active() {
    let ts = tiles();
    let mut s = MotionSearch::new(0.5);
    s.select(0.0, &ts, 800, 100, 2);
    s.observe(0.0, &[person(0.2)]);
    let mut visited = std::collections::BTreeSet::new();
    for i in 1..5 {
        let t = i as f64 * 0.1;
        let plan = s.select(t, &ts, 800, 100, 2);
        assert!(plan.indices.windows(2).all(|w| w[0] < w[1]));
        visited.extend(plan.indices);
        s.observe(t, &[person(0.2)]);
    }
    assert_eq!(visited.len(), 8);
    assert!(s.select(0.5, &ts, 800, 100, 2).full_sweep);
}

#[test]
fn time_reversal_gap_invalid_evidence_and_empty_plans_recover() {
    let ts = tiles();
    let mut s = MotionSearch::new(2.0);
    s.select(0.0, &ts, 800, 100, 0);
    s.observe(0.0, &[person(0.2)]);
    assert!(s.select(-0.1, &ts, 800, 100, 0).full_sweep);
    s.observe(0.2, &[person(f32::NAN)]);
    assert!(s.select(0.3, &ts, 800, 100, 0).full_sweep);
    s.observe(0.3, &[person(0.2)]);
    assert!(s.select(4.0, &ts, 800, 100, 0).full_sweep);
    assert!(s.select(4.1, &[], 800, 100, 4).indices.is_empty());
}
