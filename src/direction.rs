//! Playing direction: which end (left / right goal or basket) each team attacks, and when the teams
//! change ends. With it, an attempt or set piece at an end belongs to the team attacking that end,
//! whatever colour the tracker gave the player who happened to hold the ball.
//!
//! The direction is given by the user (`--home-attacks l|r`, `--switch-at mm:ss`) or estimated from
//! the attempts of a run: the split of the game into at most two halves in which the teams shot at
//! their end most consistently.

use crate::team::Team;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Direction {
    /// End (`'l'` / `'r'`) the home team attacks at the start.
    pub home_start: char,
    /// Video second at which the teams change ends, if they do.
    pub switch_at: Option<f64>,
}

impl Direction {
    pub fn new(home_start: char, switch_at: Option<f64>) -> Option<Self> {
        matches!(home_start, 'l' | 'r').then_some(Self { home_start, switch_at })
    }

    /// End the home team attacks at video second `t`.
    pub fn home_target(&self, t: f64) -> char {
        if self.switch_at.is_some_and(|s| t >= s) { other(self.home_start) } else { self.home_start }
    }

    /// Team attacking end `side` at second `t`.
    pub fn attacker(&self, side: char, t: f64) -> Team {
        if side == self.home_target(t) { Team::Home } else { Team::Away }
    }

    /// From `"l"` / `"r"` (also `left` / `right`, `links` / `rechts`) and an optional switch time
    /// (`"24:00"`, `"1440"`, empty = no switch). `None` for anything else, e.g. `"auto"`.
    pub fn parse(home: &str, switch: &str) -> Option<Self> {
        let side = match home.trim().to_lowercase().as_str() {
            "l" | "left" | "links" | "gauche" | "izquierda" => 'l',
            "r" | "right" | "rechts" | "droite" | "derecha" => 'r',
            _ => return None,
        };
        Self::new(side, parse_clock(switch))
    }
}

pub fn other(side: char) -> char {
    if side == 'l' { 'r' } else { 'l' }
}

/// `"24:00"` / `"1:02:30"` / `"1440"` -> seconds; empty or invalid -> `None`.
pub fn parse_clock(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for part in s.split(':') {
        let v: f64 = part.trim().replace(',', ".").parse().ok()?;
        if !v.is_finite() || v < 0.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

/// Scene kinds taken towards the end in their `side`: the team doing them attacks that end.
pub fn is_attacking_scene(kind: &str) -> bool {
    matches!(kind, "shot2" | "shot3" | "free_throw" | "corner" | "penalty" | "seven_meter" | "shot")
}

/// One observation: at second `t`, `team` went for end `side`.
#[derive(Clone, Copy, Debug)]
pub struct Obs {
    pub t: f64,
    pub team: Team,
    pub side: char,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub dir: Direction,
    /// Observations that fit the direction, out of `total`.
    pub agree: usize,
    pub total: usize,
}

impl Estimate {
    pub fn share(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.agree as f64 / self.total as f64 }
    }

    /// Enough attempts and a clear majority: safe to re-assign attempts by it.
    pub fn reliable(&self) -> bool {
        self.total >= 6 && self.share() >= 0.7
    }
}

/// Best direction for the observations (teams other than home / away are ignored). A change of ends
/// is only accepted if it explains clearly more observations than none, with a few on each side of it.
pub fn estimate(obs: &[Obs]) -> Option<Estimate> {
    let obs: Vec<Obs> = obs.iter().copied().filter(|o| matches!(o.team, Team::Home | Team::Away) && matches!(o.side, 'l' | 'r')).collect();
    if obs.is_empty() {
        return None;
    }
    let fits = |d: &Direction| obs.iter().filter(|o| d.attacker(o.side, o.t) == o.team).count();
    let best_of = |switch_at: Option<f64>| {
        ['l', 'r'].into_iter().map(|h| Direction { home_start: h, switch_at }).map(|d| (fits(&d), d)).max_by_key(|x| x.0).unwrap()
    };
    let (mut agree, mut dir) = best_of(None);
    let mut ts: Vec<f64> = obs.iter().map(|o| o.t).collect();
    ts.sort_by(|a, b| a.total_cmp(b));
    let min_side = 3.max(obs.len() / 6);
    let need = 3.max(obs.len() * 15 / 100);
    if ts.len() >= 2 * min_side {
        // a change of ends halfway between two neighbouring observations; among equally good ones the
        // widest gap (the break between the halves has no attempts)
        let mut best: Option<(usize, f64, Direction)> = None;
        for i in min_side..=ts.len() - min_side {
            let at = (ts[i - 1] + ts[i]) / 2.0;
            let gap = ts[i] - ts[i - 1];
            let (a, d) = best_of(Some(at));
            if best.is_none_or(|(ba, bg, _)| a > ba || (a == ba && gap > bg)) {
                best = Some((a, gap, d));
            }
        }
        if let Some((a, _, d)) = best.filter(|b| b.0 >= agree + need) {
            agree = a;
            dir = d;
        }
    }
    Some(Estimate { dir, agree, total: obs.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(t: f64, team: Team, side: char) -> Obs {
        Obs { t, team, side }
    }

    #[test]
    fn attacker_changes_at_the_switch() {
        let d = Direction::parse("links", "20:00").unwrap();
        assert_eq!(d.attacker('l', 100.0), Team::Home);
        assert_eq!(d.attacker('r', 100.0), Team::Away);
        assert_eq!(d.attacker('l', 1300.0), Team::Away);
        assert_eq!(Direction::parse("auto", ""), None);
        assert_eq!(parse_clock("1:02:30"), Some(3750.0));
        assert_eq!(parse_clock("90"), Some(90.0));
        assert_eq!(parse_clock("x"), None);
    }

    #[test]
    fn estimate_finds_direction_and_halftime() {
        let mut obs = Vec::new();
        for i in 0..10 {
            let t = i as f64 * 60.0;
            obs.push(o(t, Team::Home, 'r'));
            obs.push(o(t + 5.0, Team::Away, 'l'));
            obs.push(o(t + 1200.0, Team::Home, 'l'));
            obs.push(o(t + 1205.0, Team::Away, 'r'));
        }
        obs.push(o(30.0, Team::Away, 'r')); // one mis-assigned shooter
        let e = estimate(&obs).unwrap();
        assert_eq!(e.dir.home_start, 'r');
        let s = e.dir.switch_at.unwrap();
        assert!(s > 545.0 && s < 1200.0, "{s}");
        assert_eq!(e.agree, 40);
        assert!(e.reliable());
    }

    #[test]
    fn no_switch_without_evidence() {
        let obs: Vec<Obs> = (0..12).map(|i| o(i as f64 * 50.0, if i % 2 == 0 { Team::Home } else { Team::Away }, if i % 2 == 0 { 'l' } else { 'r' })).collect();
        let e = estimate(&obs).unwrap();
        assert_eq!(e.dir, Direction { home_start: 'l', switch_at: None });
        assert_eq!(e.agree, 12);
    }
}
