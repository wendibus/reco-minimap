//! Playing direction: which end (left / right goal or basket) each team attacks, and when the teams
//! change ends. With it, an attempt or set piece at an end belongs to the team attacking that end,
//! whatever colour the tracker gave the player who happened to hold the ball.
//!
//! The direction is given by the user (`--home-attacks l|r`, `--switch-at mm:ss` or
//! `--switch-period N`: the teams change ends when period N starts, e.g. the 3rd quarter in
//! basketball or the 2nd half in football) or estimated from
//! the attempts of a run: the split of the game into at most two halves in which the teams shot at
//! their end most consistently.

use crate::team::Team;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Direction {
    /// End (`'l'` / `'r'`) the home team attacks at the start.
    pub home_start: char,
    /// Video second at which the teams change ends, if they do.
    pub switch_at: Option<f64>,
    /// The teams change ends when this period starts; its video second (`switch_at`) comes from
    /// the scoreboard (see [`Direction::note_period`] / [`Direction::with_periods`]).
    pub switch_period: Option<u32>,
    /// Readings in a row that showed `switch_period` or later (one misread digit does not switch).
    votes: u8,
}

impl Direction {
    pub fn new(home_start: char, switch_at: Option<f64>) -> Option<Self> {
        matches!(home_start, 'l' | 'r').then_some(Self { home_start, switch_at, switch_period: None, votes: 0 })
    }

    /// Ends change when period `period` starts (its time is filled in from the scoreboard).
    pub fn at_period(home_start: char, period: u32) -> Option<Self> {
        Self::new(home_start, None).map(|d| Self { switch_period: Some(period).filter(|p| *p >= 2), ..d })
    }

    /// A scoreboard reading during a run: the first time two readings in a row show the switch
    /// period (or a later one), the teams change ends from there on.
    pub fn note_period(&mut self, period: Option<u32>, t: f64) {
        let (Some(want), None) = (self.switch_period, self.switch_at) else { return };
        match period {
            Some(p) if p >= want => {
                self.votes += 1;
                if self.votes >= 2 {
                    self.switch_at = Some(t);
                }
            }
            Some(_) => self.votes = 0,
            None => {}
        }
    }

    /// The switch time from the scoreboard readings of a whole game (start of `switch_period`).
    pub fn with_periods(mut self, readings: &[crate::scoreboard::Reading]) -> Self {
        if self.switch_at.is_none() && self.switch_period.is_some() {
            let tl = crate::scoreboard::timeline::resolve(readings, &Default::default());
            let want = self.switch_period.unwrap_or(0);
            self.switch_at = tl.segments.iter().find(|s| matches!(s.kind, crate::scoreboard::timeline::SegmentKind::Period(p) if p >= want)).map(|s| s.t0);
            if self.switch_at.is_none() {
                for r in readings {
                    self.note_period(r.period, r.t);
                }
            }
        }
        self
    }

    /// End the home team attacks at video second `t`.
    pub fn home_target(&self, t: f64) -> char {
        if self.switch_at.is_some_and(|s| t >= s) { other(self.home_start) } else { self.home_start }
    }

    /// Team attacking end `side` at second `t`.
    pub fn attacker(&self, side: char, t: f64) -> Team {
        if side == self.home_target(t) { Team::Home } else { Team::Away }
    }

    /// From `"l"` / `"r"` (also `left` / `right`, `links` / `rechts`) and an optional switch: a time
    /// (`"24:00"`, `"1440"`), a period (`"period:3"`) or empty = no switch. `None` for anything else, e.g. `"auto"`.
    pub fn parse(home: &str, switch: &str) -> Option<Self> {
        let side = match home.trim().to_lowercase().as_str() {
            "l" | "left" | "links" | "gauche" | "izquierda" => 'l',
            "r" | "right" | "rechts" | "droite" | "derecha" => 'r',
            _ => return None,
        };
        match switch.trim().strip_prefix("period:").and_then(|p| p.trim().parse::<u32>().ok()) {
            Some(p) => Self::at_period(side, p),
            None => Self::new(side, parse_clock(switch)),
        }
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
        ['l', 'r'].into_iter().filter_map(|h| Direction::new(h, switch_at)).map(|d| (fits(&d), d)).max_by_key(|x| x.0).unwrap()
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
        assert_eq!(e.dir, Direction::new('l', None).unwrap());
        assert_eq!(e.agree, 12);
    }

    #[test]
    fn ends_change_with_the_period_on_the_scoreboard() {
        let mut d = Direction::parse("l", "period:3").unwrap();
        assert_eq!((d.switch_period, d.switch_at), (Some(3), None));
        // one misread 3 does not switch; two readings in a row do
        d.note_period(Some(3), 100.0);
        d.note_period(Some(2), 101.0);
        d.note_period(Some(3), 900.0);
        assert_eq!(d.switch_at, None);
        d.note_period(Some(3), 901.0);
        assert_eq!(d.switch_at, Some(901.0));
        assert_eq!((d.home_target(10.0), d.home_target(1000.0)), ('l', 'r'));
        // a time still works
        assert_eq!(Direction::parse("r", "24:00").unwrap().switch_at, Some(1440.0));
    }
}
