//! Sport-independent identity candidates, separate from reusable tracking slots.
//!
//! OCR evidence is not proof of a person's identity. A team/number pair is a
//! provisional roster key within this analysis, not a global person identifier.
//!
//! Each frame the numbers of one team are assigned **jointly** (Hungarian
//! method over tracks x roster numbers), so the deductions a person makes come
//! out of the same step:
//! * **read** – the track's own readings confirm the number;
//! * **exclusion** – one number per player at a time: if the best-read number
//!   is held by a teammate who was read more clearly, the runner-up becomes the
//!   answer (e.g. a half-hidden "4" next to a clearly read 4 is the 34);
//! * **lineup** – with exactly `team_size` players of a team on the field, a
//!   known line-up of `team_size` numbers and all others identified, the last
//!   player is the remaining number.
//!
//! Simultaneous equally strong claims by teammates stay conflicts, never
//! resolved by whichever track happened to be visited first.

use crate::{team::Team, tracker::TrackState, AgentOut};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdentityStatus {
    #[default]
    Unknown,
    /// Read: the track's own number readings confirm it.
    Candidate,
    /// Deduced (exclusion or line-up), not read directly.
    Inferred,
    Conflict,
}

impl IdentityStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Candidate => "candidate",
            Self::Inferred => "inferred",
            Self::Conflict => "conflict",
        }
    }

    /// The number may be shown / counted (read or deduced).
    pub fn known(self) -> bool {
        matches!(self, Self::Candidate | Self::Inferred)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerKey {
    pub team: Team,
    pub number: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IdentityAssignment {
    pub status: IdentityStatus,
    /// Retained for inspection even on conflicts; only Candidate / Inferred are unambiguous.
    pub player: Option<PlayerKey>,
    /// How it was found: `"read"`, `"exclusion"` or `"lineup"`.
    pub source: Option<&'static str>,
}

/// What one active (not lost) field player of a team brings to the frame's assignment.
#[derive(Clone, Debug)]
pub struct Claim {
    pub team: Team,
    /// Number the track's readings confirmed on their own.
    pub confirmed: Option<u8>,
    /// Accumulated reading evidence per number.
    pub votes: Vec<(u8, f32)>,
    /// Numbers this player may carry (roster, keeper split); empty = no roster known.
    pub allowed: Vec<u8>,
}

impl Claim {
    fn vote(&self, n: u8) -> f32 {
        self.votes.iter().find(|v| v.0 == n).map(|v| v.1).unwrap_or(0.0)
    }
    fn total(&self) -> f32 {
        self.votes.iter().map(|v| v.1).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SolveCfg {
    /// Players per team on the field at once.
    pub team_size: usize,
    /// Evidence for a number on its own (as in `Config::num_min_score` / `num_min_lead`).
    pub min_score: f32,
    pub min_lead: f32,
}

/// Joint assignment of one frame. `lineup` per team: the numbers currently on the field, if known
/// exactly (see [`Lineup`]). Returns one assignment per claim, in order.
pub fn solve(claims: &[Claim], lineup: &dyn Fn(Team) -> Option<Vec<u8>>, cfg: SolveCfg) -> Vec<IdentityAssignment> {
    let mut out = vec![IdentityAssignment::default(); claims.len()];
    for team in [Team::Home, Team::Away] {
        let idx: Vec<usize> = (0..claims.len()).filter(|&i| claims[i].team == team).collect();
        if idx.is_empty() {
            continue;
        }
        let res = solve_team(&idx.iter().map(|&i| &claims[i]).collect::<Vec<_>>(), lineup(team), cfg);
        for (k, &i) in idx.iter().enumerate() {
            out[i] = res[k];
        }
    }
    out
}

fn solve_team(c: &[&Claim], lineup: Option<Vec<u8>>, cfg: SolveCfg) -> Vec<IdentityAssignment> {
    let team = c[0].team;
    let mk = |status, n: Option<u8>, source| IdentityAssignment { status, player: n.map(|number| PlayerKey { team, number }), source };
    let mut out = vec![IdentityAssignment::default(); c.len()];
    // numbers in play: the roster (all claims of a team share it up to the keeper split), else what was read
    let mut numbers: Vec<u8> = c.iter().flat_map(|x| x.allowed.iter().copied()).collect();
    let roster_known = !numbers.is_empty();
    if !roster_known {
        numbers = c.iter().filter_map(|x| x.confirmed).collect();
    }
    numbers.sort_unstable();
    numbers.dedup();
    let allowed = |x: &Claim, n: u8| x.allowed.is_empty() || x.allowed.contains(&n);

    // 1. confirmed readings; equally strong duplicates stay conflicts, a clearly stronger one wins
    let mut taken: Vec<u8> = Vec::new();
    for n in numbers.iter().copied() {
        let mut holders: Vec<usize> = (0..c.len()).filter(|&i| c[i].confirmed == Some(n)).collect();
        if holders.is_empty() {
            continue;
        }
        holders.sort_by(|&a, &b| c[b].vote(n).total_cmp(&c[a].vote(n)));
        let best = holders[0];
        let clear = holders.get(1).is_none_or(|&s| c[best].vote(n) >= 1.5 * c[s].vote(n).max(0.1));
        if clear {
            out[best] = mk(IdentityStatus::Candidate, Some(n), Some("read"));
        } else {
            for &s in &holders {
                out[s] = mk(IdentityStatus::Conflict, Some(n), None);
            }
        }
        taken.push(n);
    }
    if !roster_known {
        return out;
    }

    // 2. joint assignment of the rest over the numbers not taken: share of each player's evidence
    let open: Vec<usize> = (0..c.len()).filter(|&i| out[i].status == IdentityStatus::Unknown).collect();
    let free: Vec<u8> = numbers.iter().copied().filter(|n| !taken.contains(n)).collect();
    if !open.is_empty() && !free.is_empty() {
        let weight = |i: usize, n: u8| -> f64 {
            let x = c[i];
            if !allowed(x, n) {
                return -1e6;
            }
            (x.vote(n) / (x.total() + 1.0)) as f64
        };
        let rows = open.len();
        let cols = free.len() + rows; // a dummy column per player = "no number"
        let cost: Vec<Vec<f64>> = open.iter().map(|&i| (0..cols).map(|j| if j < free.len() { -weight(i, free[j]) } else { 0.0 }).collect()).collect();
        let pick = hungarian(&cost, rows, cols);
        for (r, &i) in open.iter().enumerate() {
            let Some(&j) = pick.get(r) else { continue };
            if j >= free.len() {
                continue;
            }
            let n = free[j];
            let x = c[i];
            // the number must be clearly this player's best among those still free
            let rival = x.votes.iter().filter(|v| v.0 != n && free.contains(&v.0) && allowed(x, v.0)).map(|v| v.1).fold(0.0f32, f32::max);
            if x.vote(n) > 0.0 && x.vote(n) >= cfg.min_score * 0.5 && x.vote(n) - rival >= cfg.min_lead * 0.5 {
                out[i] = mk(IdentityStatus::Inferred, Some(n), Some("exclusion"));
            }
        }
    }

    // 3. line-up: all players on the field, all but one identified within the known line-up
    if let Some(lu) = lineup.filter(|l| cfg.team_size > 0 && l.len() == cfg.team_size) {
        if c.len() == cfg.team_size {
            let known: Vec<u8> = out.iter().filter(|a| a.status.known()).filter_map(|a| a.player.map(|p| p.number)).collect();
            let missing: Vec<u8> = lu.iter().copied().filter(|n| !known.contains(n)).collect();
            let unknown: Vec<usize> = (0..c.len()).filter(|&i| !out[i].status.known()).collect();
            let all_in_lineup = known.iter().all(|n| lu.contains(n));
            if all_in_lineup && missing.len() == 1 && unknown.len() == 1 {
                let (i, n) = (unknown[0], missing[0]);
                let x = c[i];
                // no clear reading of another number speaks against it
                let against = x.votes.iter().any(|v| v.0 != n && v.1 >= cfg.min_score);
                if allowed(x, n) && !against && out[i].status != IdentityStatus::Conflict {
                    out[i] = mk(IdentityStatus::Inferred, Some(n), Some("lineup"));
                }
            }
        }
    }
    out
}

/// Minimum-cost assignment of `rows` rows to distinct columns (`rows <= cols`), O(rows² · cols).
/// Returns the column of each row.
pub fn hungarian(cost: &[Vec<f64>], rows: usize, cols: usize) -> Vec<usize> {
    if rows == 0 {
        return Vec::new();
    }
    let inf = f64::INFINITY;
    let (mut u, mut v) = (vec![0.0; rows + 1], vec![0.0; cols + 1]);
    let (mut p, mut way) = (vec![0usize; cols + 1], vec![0usize; cols + 1]);
    for i in 1..=rows {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![inf; cols + 1];
        let mut used = vec![false; cols + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let (mut delta, mut j1) = (inf, 0usize);
            for j in 1..=cols {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=cols {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut res = vec![0usize; rows];
    for j in 1..=cols {
        if p[j] > 0 {
            res[p[j] - 1] = j - 1;
        }
    }
    res
}

/// Which numbers of each team are on the field: every number read on an active player counts as
/// on the field for [`Lineup::WINDOW`] seconds. If a team has more numbers than `team_size` in the
/// window (a substitution the readings have not settled yet), its line-up is unknown.
#[derive(Clone, Debug, Default)]
pub struct Lineup {
    seen: Vec<(Team, u8, f64)>,
}

impl Lineup {
    /// How long a number counts as on the field without being read again.
    pub const WINDOW: f64 = 240.0;

    pub fn saw(&mut self, team: Team, number: u8, t: f64) {
        match self.seen.iter_mut().find(|s| s.0 == team && s.1 == number) {
            Some(s) => s.2 = s.2.max(t),
            None => self.seen.push((team, number, t)),
        }
    }

    /// A player went off (confirmed substitution).
    pub fn left(&mut self, team: Team, number: u8) {
        self.seen.retain(|s| !(s.0 == team && s.1 == number));
    }

    /// Exactly `team_size` numbers read recently, or `None`.
    pub fn get(&self, team: Team, t: f64, team_size: usize) -> Option<Vec<u8>> {
        let v: Vec<u8> = self.seen.iter().filter(|s| s.0 == team && t - s.2 <= Self::WINDOW && s.2 <= t + 1e-6).map(|s| s.1).collect();
        (v.len() == team_size).then_some(v)
    }
}

/// Mark duplicate claims as conflicts and hide numbers that are not settled. Keeps what [`solve`]
/// found; for frames without it (replays, tests) the `jersey` field is taken as a reading.
pub fn assign(players: &mut [AgentOut]) {
    let claims: Vec<Option<PlayerKey>> = players
        .iter()
        .map(|a| {
            if a.referee || a.team == Team::Unknown || a.state == TrackState::Lost {
                None
            } else if a.identity.status != IdentityStatus::Unknown {
                a.identity.player
            } else {
                a.jersey.map(|number| PlayerKey { team: a.team, number })
            }
        })
        .collect();
    for (agent, claim) in players.iter_mut().zip(&claims) {
        let status = match claim {
            None => IdentityStatus::Unknown,
            Some(_) if agent.identity.status == IdentityStatus::Conflict => IdentityStatus::Conflict,
            Some(_) if claims.iter().filter(|other| *other == claim).count() > 1 => IdentityStatus::Conflict,
            Some(_) if agent.identity.status == IdentityStatus::Inferred => IdentityStatus::Inferred,
            Some(_) => IdentityStatus::Candidate,
        };
        let source = match status {
            IdentityStatus::Candidate => agent.identity.source.or(Some("read")),
            IdentityStatus::Inferred => agent.identity.source,
            _ => None,
        };
        agent.identity = IdentityAssignment { status, player: *claim, source };
        // Existing renderers consume `jersey`; suppress misleading duplicate labels while
        // preserving the conflicting claim in structured output.
        agent.jersey = if status.known() { claim.map(|p| p.number) } else { None };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: SolveCfg = SolveCfg { team_size: 5, min_score: 3.0, min_lead: 1.5 };

    fn claim(confirmed: Option<u8>, votes: &[(u8, f32)], roster: &[u8]) -> Claim {
        Claim { team: Team::Home, confirmed, votes: votes.to_vec(), allowed: roster.to_vec() }
    }

    const ROSTER: [u8; 8] = [4, 5, 7, 9, 12, 21, 34, 44];

    #[test]
    fn hungarian_finds_the_cheapest_assignment() {
        let cost = vec![vec![4.0, 1.0, 3.0], vec![2.0, 0.0, 5.0], vec![3.0, 2.0, 2.0]];
        let a = hungarian(&cost, 3, 3);
        let total: f64 = a.iter().enumerate().map(|(i, &j)| cost[i][j]).sum();
        assert_eq!(total, 5.0);
    }

    #[test]
    fn lineup_names_the_fifth_player() {
        let mut c: Vec<Claim> = [4, 7, 9, 12].iter().map(|&n| claim(Some(n), &[(n, 5.0)], &ROSTER)).collect();
        c.push(claim(None, &[], &ROSTER));
        let lu = |_: Team| Some(vec![4, 7, 9, 12, 34]);
        let r = solve(&c, &lu, CFG);
        assert_eq!(r[4].status, IdentityStatus::Inferred);
        assert_eq!(r[4].player.unwrap().number, 34);
        assert_eq!(r[4].source, Some("lineup"));
        assert!(r[..4].iter().all(|a| a.status == IdentityStatus::Candidate));
        // without a known line-up, or with only four players visible, nothing is guessed
        assert_eq!(solve(&c, &|_| None, CFG)[4].status, IdentityStatus::Unknown);
        assert_eq!(solve(&c[1..], &lu, CFG)[3].status, IdentityStatus::Unknown);
    }

    #[test]
    fn exclusion_gives_the_runner_up() {
        // a clearly read 4 next to a player whose reads say "4" (half hidden) and a bit of "34"
        let c = vec![claim(Some(4), &[(4, 6.0)], &ROSTER), claim(None, &[(4, 2.0), (34, 1.8)], &ROSTER)];
        let r = solve(&c, &|_| None, CFG);
        assert_eq!(r[0].player.unwrap().number, 4);
        assert_eq!(r[1].status, IdentityStatus::Inferred);
        assert_eq!(r[1].player.unwrap().number, 34);
        assert_eq!(r[1].source, Some("exclusion"));
    }

    #[test]
    fn equal_duplicates_stay_conflicts_a_clear_one_wins() {
        let c = vec![claim(Some(7), &[(7, 4.0)], &ROSTER), claim(Some(7), &[(7, 3.8)], &ROSTER)];
        let r = solve(&c, &|_| None, CFG);
        assert!(r.iter().all(|a| a.status == IdentityStatus::Conflict));
        let c = vec![claim(Some(7), &[(7, 8.0)], &ROSTER), claim(Some(7), &[(7, 2.0), (21, 1.0)], &ROSTER)];
        let r = solve(&c, &|_| None, CFG);
        assert_eq!(r[0].status, IdentityStatus::Candidate);
        assert_ne!(r[1].player.map(|p| p.number), Some(7));
    }

    #[test]
    fn lineup_tracks_substitutions() {
        let mut l = Lineup::default();
        for (i, n) in [4, 7, 9, 12, 34].iter().enumerate() {
            l.saw(Team::Home, *n, i as f64);
        }
        assert_eq!(l.get(Team::Home, 10.0, 5).map(|v| v.len()), Some(5));
        l.saw(Team::Home, 21, 20.0); // a substitute read before the leaver is known: unclear
        assert_eq!(l.get(Team::Home, 21.0, 5), None);
        l.left(Team::Home, 9);
        assert!(l.get(Team::Home, 21.0, 5).unwrap().contains(&21));
        assert_eq!(l.get(Team::Home, 1000.0, 5), None); // too old
    }
}
