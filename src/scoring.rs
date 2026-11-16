//! Who scored? Combines the score on the hall scoreboard ([`crate::scoreboard`]) with the shot
//! attempts the scene detection saw before it (basketball).
//!
//! The scoreboard says *that* a team scored and how many points (+1 / +2 / +3), but not who. The
//! scene detection sees attempts (2 points, 3 points, free throws) with the shooter's track, but
//! not whether the ball went in. Each score change is matched to the attempt that most likely
//! produced it:
//!
//! * same team (an attempt of an unknown team counts less, the other team not at all),
//! * fitting kind: +1 ↔ free throw, +2 ↔ 2-point attempt, +3 ↔ 3-point attempt; the neighbours
//!   (a 3 that was a 2 because of a foot on the line, two free throws in one update, an and-one
//!   counted together) are possible but less likely,
//! * time: the board follows the basket by about 1–8 s (operator plus the stabiliser of the
//!   reading); older attempts fade out until [`AttributionCfg::max_lag`],
//! * the last attempt wins: a later attempt of the same team means the earlier one missed
//!   (offensive rebound), so earlier candidates lose weight for every later one.
//!
//! The result is a probability per change, not a fact; without a fitting attempt the points go
//! to the team only. A field-goal attempt is used once; a free-throw attempt can take up to three
//! single points (the same player takes all free throws of a foul).

use crate::scoreboard::Reading;
use crate::team::Team;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotKind {
    Two,
    Three,
    FreeThrow,
}

impl ShotKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "shot2" => Some(ShotKind::Two),
            "shot3" => Some(ShotKind::Three),
            "free_throw" => Some(ShotKind::FreeThrow),
            _ => None,
        }
    }
}

/// How the shooter's jersey number is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberStatus {
    /// Read on the shirt (or set by hand).
    Read,
    /// Deduced (line-up, exclusion, tracklet), not read.
    Inferred,
    /// No number.
    Unknown,
}

impl NumberStatus {
    pub fn key(self) -> &'static str {
        match self {
            NumberStatus::Read => "read",
            NumberStatus::Inferred => "inferred",
            NumberStatus::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shooter {
    /// Tracking slot and its generation (a slot is not a person).
    pub track: u32,
    pub segment: u32,
    pub number: Option<u32>,
    pub status: NumberStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotAttempt {
    pub t: f64,
    /// `Team::Unknown` when the scene detection could not tell.
    pub team: Team,
    pub kind: ShotKind,
    pub shooter: Option<Shooter>,
    /// Confidence of the scene detection (0..1).
    pub confidence: f32,
}

/// A change of one team's score on the board.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoreChange {
    pub t: f64,
    pub team: Team,
    /// Signed: negative is a correction on the board.
    pub points: i32,
    /// Score of that team before and after.
    pub from: u32,
    pub to: u32,
}

/// Score changes (per team) from the stable board readings. The first reading of a team is its
/// starting score, not a change.
pub fn score_changes(readings: &[Reading]) -> Vec<ScoreChange> {
    let mut out = Vec::new();
    let (mut home, mut away): (Option<u32>, Option<u32>) = (None, None);
    for r in readings {
        for (team, last, now) in [(Team::Home, &mut home, r.home), (Team::Away, &mut away, r.away)] {
            let Some(now) = now else { continue };
            if let Some(prev) = *last {
                if now != prev {
                    out.push(ScoreChange { t: r.t, team, points: now as i32 - prev as i32, from: prev, to: now });
                }
            }
            *last = Some(now);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Matched to an attempt.
    Attempt,
    /// No fitting attempt: the points count for the team only.
    TeamOnly,
    /// The board went down (an operator correction); it cancels the latest fitting credit.
    Correction,
    /// More than 3 points at once (missed updates, a reset): team only, no shooter.
    Jump,
}

impl Basis {
    pub fn key(self) -> &'static str {
        match self {
            Basis::Attempt => "attempt",
            Basis::TeamOnly => "team_only",
            Basis::Correction => "correction",
            Basis::Jump => "jump",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Credit {
    pub change: ScoreChange,
    pub basis: Basis,
    /// Index into the attempts.
    pub attempt: Option<usize>,
    pub shooter: Option<Shooter>,
    /// Probability that the attempt (and so its shooter) produced these points.
    pub probability: f32,
    /// Fitting attempts that were considered.
    pub candidates: usize,
    /// Cancelled by a later correction on the board.
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub struct AttributionCfg {
    /// Oldest attempt (s before the change) that can still explain it.
    pub max_lag: f64,
    /// Latest attempt (s after the change): the board can be quick, the attempt time late.
    pub max_lead: f64,
    /// Weight of "the real attempt was not detected" in the probability.
    pub missed: f32,
    /// Below this weight an attempt does not count.
    pub min_weight: f32,
}

impl Default for AttributionCfg {
    fn default() -> Self {
        Self { max_lag: 20.0, max_lead: 1.0, missed: 0.35, min_weight: 0.08 }
    }
}

fn kind_weight(points: i32, kind: ShotKind) -> f32 {
    match (points, kind) {
        (1, ShotKind::FreeThrow) | (2, ShotKind::Two) | (3, ShotKind::Three) => 1.0,
        (2, ShotKind::Three) => 0.3,        // foot on the line, release point near the arc
        (2, ShotKind::FreeThrow) => 0.35,   // two free throws in one update
        (3, ShotKind::Two) => 0.35,         // and-one counted together, or a misread arc
        (3, ShotKind::FreeThrow) => 0.2,    // three free throws
        (1, ShotKind::Two) => 0.1,
        (1, ShotKind::Three) => 0.05,
        _ => 0.0,
    }
}

fn time_weight(lag: f64, cfg: &AttributionCfg) -> f32 {
    if lag < -cfg.max_lead || lag > cfg.max_lag {
        0.0
    } else if lag < 0.5 {
        0.5
    } else if lag <= 8.0 {
        1.0
    } else {
        (1.0 - 0.75 * (lag - 8.0) / (cfg.max_lag - 8.0).max(1e-6)) as f32
    }
}

fn team_weight(change: Team, attempt: Team) -> f32 {
    if attempt == change {
        1.0
    } else if attempt == Team::Unknown {
        0.4
    } else {
        0.0
    }
}

/// Match every score change to the most likely attempt (see the module docs). `attempts` must be
/// sorted by time.
pub fn attribute(changes: &[ScoreChange], attempts: &[ShotAttempt], cfg: &AttributionCfg) -> Vec<Credit> {
    // remaining points an attempt can still explain
    let mut capacity: Vec<i32> = attempts.iter().map(|a| if a.kind == ShotKind::FreeThrow { 3 } else { 1 }).collect();
    let mut out: Vec<Credit> = Vec::with_capacity(changes.len());
    for ch in changes {
        if ch.points < 0 {
            // a correction cancels the latest credit of the team with the same points (else any)
            let target = out
                .iter()
                .rposition(|c| !c.cancelled && c.change.team == ch.team && c.change.points == -ch.points && ch.t - c.change.t <= 120.0)
                .or_else(|| out.iter().rposition(|c| !c.cancelled && c.change.team == ch.team && c.change.points > 0 && ch.t - c.change.t <= 120.0));
            if let Some(i) = target {
                out[i].cancelled = true;
                if let Some(a) = out[i].attempt {
                    capacity[a] += if attempts[a].kind == ShotKind::FreeThrow { out[i].change.points } else { 1 };
                }
            }
            out.push(Credit { change: *ch, basis: Basis::Correction, attempt: None, shooter: None, probability: 1.0, candidates: 0, cancelled: false });
            continue;
        }
        if ch.points > 3 {
            out.push(Credit { change: *ch, basis: Basis::Jump, attempt: None, shooter: None, probability: 1.0, candidates: 0, cancelled: false });
            continue;
        }
        let fits = |i: usize, a: &ShotAttempt| {
            let need = if a.kind == ShotKind::FreeThrow { ch.points } else { 1 };
            capacity[i] >= need && team_weight(ch.team, a.team) > 0.0 && time_weight(ch.t - a.t, cfg) > 0.0 && kind_weight(ch.points, a.kind) > 0.0
        };
        let cands: Vec<usize> = attempts.iter().enumerate().filter(|(i, a)| fits(*i, a)).map(|(i, _)| i).collect();
        let mut weights: Vec<(usize, f32)> = cands
            .iter()
            .map(|&i| {
                let a = &attempts[i];
                // every later fitting attempt of the team says this one probably missed
                let later = cands.iter().filter(|&&j| attempts[j].t > a.t && kind_weight(ch.points, attempts[j].kind) >= 0.3 && attempts[j].team != Team::Unknown).count();
                let w = kind_weight(ch.points, a.kind) * time_weight(ch.t - a.t, cfg) * team_weight(ch.team, a.team) * (0.5 + 0.5 * a.confidence.clamp(0.0, 1.0)) * 0.35f32.powi(later as i32);
                (i, w)
            })
            .filter(|(_, w)| *w >= cfg.min_weight)
            .collect();
        weights.sort_by(|a, b| b.1.total_cmp(&a.1).then(attempts[b.0].t.total_cmp(&attempts[a.0].t)));
        let sum: f32 = weights.iter().map(|w| w.1).sum();
        match weights.first() {
            Some(&(i, w)) => {
                capacity[i] -= if attempts[i].kind == ShotKind::FreeThrow { ch.points } else { 1 };
                let p = w / (sum + cfg.missed);
                out.push(Credit { change: *ch, basis: Basis::Attempt, attempt: Some(i), shooter: attempts[i].shooter, probability: (p * 100.0).round() / 100.0, candidates: weights.len(), cancelled: false });
            }
            None => out.push(Credit { change: *ch, basis: Basis::TeamOnly, attempt: None, shooter: None, probability: 1.0, candidates: 0, cancelled: false }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(t: f64, h: u32, a: u32) -> Reading {
        Reading { t, home: Some(h), away: Some(a), clock: None, period: None }
    }

    fn shot(t: f64, team: Team, kind: ShotKind, track: u32, number: Option<u32>) -> ShotAttempt {
        ShotAttempt { t, team, kind, shooter: Some(Shooter { track, segment: 0, number, status: if number.is_some() { NumberStatus::Read } else { NumberStatus::Unknown } }), confidence: 0.6 }
    }

    #[test]
    fn changes_from_readings() {
        let r = [rd(0.0, 10, 8), Reading { t: 1.0, home: None, away: Some(8), clock: None, period: None }, rd(5.0, 12, 8), rd(9.0, 12, 11), rd(20.0, 11, 11)];
        let c = score_changes(&r);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].team, c[0].points, c[0].from, c[0].to), (Team::Home, 2, 10, 12));
        assert_eq!((c[1].team, c[1].points), (Team::Away, 3));
        assert_eq!(c[2].points, -1);
    }

    #[test]
    fn the_last_fitting_attempt_of_the_team_gets_the_points() {
        let att = [
            shot(10.0, Team::Home, ShotKind::Two, 1, Some(7)),  // missed, offensive rebound
            shot(14.0, Team::Home, ShotKind::Two, 2, Some(11)), // made
            shot(15.0, Team::Away, ShotKind::Three, 5, Some(4)),
        ];
        let ch = [ScoreChange { t: 17.0, team: Team::Home, points: 2, from: 0, to: 2 }];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].basis, Basis::Attempt);
        assert_eq!(c[0].shooter.unwrap().number, Some(11));
        assert!(c[0].probability > 0.55 && c[0].probability < 1.0, "{}", c[0].probability);
        assert_eq!(c[0].candidates, 2);
    }

    #[test]
    fn kind_and_order_both_count() {
        // the board says +3: a lone 3-point attempt is a clear match
        let ch = [ScoreChange { t: 25.0, team: Team::Away, points: 3, from: 5, to: 8 }];
        let att = [shot(20.0, Team::Away, ShotKind::Three, 3, Some(9)), shot(22.0, Team::Home, ShotKind::Two, 4, Some(5))];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].shooter.unwrap().number, Some(9));
        assert!(c[0].probability >= 0.65, "{}", c[0].probability);
        // a later 2-point attempt of the same team means the 3 probably missed (and-one 2+1):
        // the later shooter, but with much less certainty
        let att = [shot(20.0, Team::Away, ShotKind::Three, 3, Some(9)), shot(23.0, Team::Away, ShotKind::Two, 4, Some(5))];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].shooter.unwrap().number, Some(5), "{c:?}");
        assert!(c[0].probability < 0.4, "{}", c[0].probability);
    }

    #[test]
    fn free_throws_go_to_the_same_shooter_and_no_attempt_means_team_only() {
        let att = [shot(30.0, Team::Home, ShotKind::FreeThrow, 6, Some(23))];
        let ch = [
            ScoreChange { t: 33.0, team: Team::Home, points: 1, from: 10, to: 11 },
            ScoreChange { t: 40.0, team: Team::Home, points: 1, from: 11, to: 12 },
            ScoreChange { t: 90.0, team: Team::Away, points: 2, from: 3, to: 5 }, // nothing seen
        ];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].shooter.unwrap().number, Some(23));
        assert_eq!(c[1].shooter.unwrap().number, Some(23));
        assert_eq!(c[2].basis, Basis::TeamOnly);
        assert!(c[2].shooter.is_none());
    }

    #[test]
    fn a_field_goal_is_used_once_and_a_correction_cancels_it() {
        let att = [shot(50.0, Team::Home, ShotKind::Two, 1, Some(7))];
        let ch = [
            ScoreChange { t: 53.0, team: Team::Home, points: 2, from: 0, to: 2 },
            ScoreChange { t: 56.0, team: Team::Home, points: 2, from: 2, to: 4 }, // operator hit +2 twice
            ScoreChange { t: 60.0, team: Team::Home, points: -2, from: 4, to: 2 },
        ];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].basis, Basis::Attempt);
        assert_eq!(c[1].basis, Basis::TeamOnly);
        assert!(c[1].cancelled && !c[0].cancelled);
        assert_eq!(c[2].basis, Basis::Correction);
    }

    #[test]
    fn attempts_too_old_or_of_the_other_team_do_not_count() {
        let att = [shot(0.0, Team::Home, ShotKind::Two, 1, Some(7)), shot(29.0, Team::Away, ShotKind::Two, 2, Some(8))];
        let ch = [ScoreChange { t: 30.0, team: Team::Home, points: 2, from: 0, to: 2 }];
        assert_eq!(attribute(&ch, &att, &AttributionCfg::default())[0].basis, Basis::TeamOnly);
        // an attempt of an unknown team still can, with less weight
        let att = [ShotAttempt { team: Team::Unknown, ..shot(27.0, Team::Home, ShotKind::Two, 3, None) }];
        let c = attribute(&ch, &att, &AttributionCfg::default());
        assert_eq!(c[0].basis, Basis::Attempt);
        assert!(c[0].probability < 0.5);
    }
}
