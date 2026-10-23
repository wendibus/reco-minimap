//! Dominant jersey colours via k-means in Lab space.

use crate::geom::{Field, Vec2};
use crate::team::rgb_to_lab;

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    Some([u8::from_str_radix(&s[0..2], 16).ok()?, u8::from_str_radix(&s[2..4], 16).ok()?, u8::from_str_radix(&s[4..6], 16).ok()?])
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub rgb: [u8; 3],
    pub count: usize,
}

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// k-means (k-means++ style deterministic seeding) over RGB samples; returns clusters by size.
pub fn kmeans(samples: &[[u8; 3]], k: usize) -> Vec<Cluster> {
    if samples.len() < k || k == 0 {
        return Vec::new();
    }
    let labs: Vec<[f32; 3]> = samples.iter().map(|&c| rgb_to_lab(c)).collect();
    let mut cent: Vec<[f32; 3]> = vec![labs[0]];
    while cent.len() < k {
        let far = (0..labs.len())
            .max_by(|&a, &b| {
                let da = cent.iter().map(|c| d2(labs[a], *c)).fold(f32::INFINITY, f32::min);
                let db = cent.iter().map(|c| d2(labs[b], *c)).fold(f32::INFINITY, f32::min);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        cent.push(labs[far]);
    }
    let mut assign = vec![0usize; labs.len()];
    for _ in 0..25 {
        for (i, l) in labs.iter().enumerate() {
            assign[i] = (0..k).min_by(|&a, &b| d2(*l, cent[a]).partial_cmp(&d2(*l, cent[b])).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        }
        let mut sum = vec![[0f32; 3]; k];
        let mut n = vec![0usize; k];
        for (i, l) in labs.iter().enumerate() {
            for c in 0..3 {
                sum[assign[i]][c] += l[c];
            }
            n[assign[i]] += 1;
        }
        for j in 0..k {
            if n[j] > 0 {
                for c in 0..3 {
                    cent[j][c] = sum[j][c] / n[j] as f32;
                }
            }
        }
    }
    // representative RGB = per-channel median of the members
    let mut out = Vec::new();
    for j in 0..k {
        let mut ch: [Vec<u8>; 3] = [vec![], vec![], vec![]];
        for (i, s) in samples.iter().enumerate() {
            if assign[i] == j {
                for c in 0..3 {
                    ch[c].push(s[c]);
                }
            }
        }
        if ch[0].is_empty() {
            continue;
        }
        let m = |v: &mut Vec<u8>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        out.push(Cluster { rgb: [m(&mut ch[0]), m(&mut ch[1]), m(&mut ch[2])], count: ch[0].len() });
    }
    out.sort_by(|a, b| b.count.cmp(&a.count));
    out
}

/// Perceptual (Lab) distance between two RGB colours.
pub fn lab_dist(a: [u8; 3], b: [u8; 3]) -> f32 {
    d2(rgb_to_lab(a), rgb_to_lab(b)).sqrt()
}

/// Split the shirt-colour samples into two teams (2-means in Lab, so a team that appears in two
/// lighting variants stays one group). Returns the two groups (largest first) and their Lab distance.
pub fn pick_teams(samples: &[[u8; 3]]) -> Option<(Cluster, Cluster, f32)> {
    let c = kmeans(samples, 2);
    if c.len() < 2 {
        return None;
    }
    let d = lab_dist(c[0].rgb, c[1].rgb);
    Some((c[0].clone(), c[1].clone(), d))
}

/// One shirt-colour sample of a player with where he stood and in which sampled frame.
#[derive(Clone, Copy, Debug)]
pub struct PlayerSample {
    pub frame: usize,
    pub pos: Vec2,
    pub rgb: [u8; 3],
}

/// Proposed goalkeeper shirts. `confident` is false when the team assignment rests on little evidence (check it, or swap).
#[derive(Clone, Debug, Default)]
pub struct KeeperPick {
    pub home: Option<Cluster>,
    pub away: Option<Cluster>,
    pub confident: bool,
}

/// Find the goalkeepers' shirt colours: players in the goal areas whose colour belongs to neither team.
/// A keeper is assigned to the team whose outfield players are, on average, on the keeper's side of the field
/// (the defending team stays near its own goal). With two keeper colours the assignment is made jointly.
pub fn pick_keepers(samples: &[PlayerSample], home: [u8; 3], away: [u8; 3], field: &Field) -> KeeperPick {
    const FAR: f32 = 25.0; // Lab distance from both team colours
    let zone = 0.18 * field.length;
    let team_of = |c: [u8; 3]| -> Option<bool> {
        let (dh, da) = (lab_dist(c, home), lab_dist(c, away));
        if dh.min(da) > 35.0 { None } else { Some(dh < da) }
    };
    let cand: Vec<&PlayerSample> = samples
        .iter()
        .filter(|s| lab_dist(s.rgb, home).min(lab_dist(s.rgb, away)) > FAR && (s.pos.x < zone || s.pos.x > field.length - zone) && s.pos.y > 0.0 && s.pos.y < field.width)
        .collect();
    if cand.len() < 4 {
        return KeeperPick::default();
    }
    let colours: Vec<[u8; 3]> = cand.iter().map(|s| s.rgb).collect();
    let mut clusters = kmeans(&colours, 2);
    if clusters.len() < 2 || lab_dist(clusters[0].rgb, clusters[1].rgb) < 15.0 || clusters[1].count * 4 < clusters[0].count {
        clusters = kmeans(&colours, 1);
    }
    // team evidence per cluster: >0 = home, <0 = away
    let nearest = |c: [u8; 3], cl: &[Cluster]| (0..cl.len()).min_by(|&a, &b| lab_dist(c, cl[a].rgb).partial_cmp(&lab_dist(c, cl[b].rgb)).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0);
    let mut evidence = vec![0f32; clusters.len()];
    for s in &cand {
        let (mut hx, mut hn, mut ax, mut an) = (0.0f32, 0, 0.0f32, 0);
        for o in samples.iter().filter(|o| o.frame == s.frame) {
            match team_of(o.rgb) {
                Some(true) => { hx += o.pos.x; hn += 1; }
                Some(false) => { ax += o.pos.x; an += 1; }
                None => {}
            }
        }
        if hn < 2 || an < 2 {
            continue;
        }
        let left = s.pos.x < field.length / 2.0;
        let (mh, ma) = (hx / hn as f32, ax / an as f32);
        // the team whose players stand nearer the keeper's goal is the keeper's team
        let toward_home = if left { ma - mh } else { mh - ma };
        evidence[nearest(s.rgb, &clusters)] += toward_home.clamp(-10.0, 10.0);
    }
    let mut out = KeeperPick { confident: true, ..Default::default() };
    if clusters.len() == 2 {
        // two keepers belong to different teams: the cluster with the stronger home evidence is the home keeper
        let (h, a) = if evidence[0] >= evidence[1] { (0, 1) } else { (1, 0) };
        out.home = Some(clusters[h].clone());
        out.away = Some(clusters[a].clone());
        out.confident = (evidence[0] - evidence[1]).abs() > 20.0;
    } else {
        let e = evidence[0];
        if e >= 0.0 { out.home = Some(clusters[0].clone()) } else { out.away = Some(clusters[0].clone()) }
        out.confident = e.abs() > 20.0;
    }
    out
}
