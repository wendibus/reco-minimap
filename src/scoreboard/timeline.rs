//! Game logic over the raw scoreboard readings.
//!
//! Small or partly hidden boards are misread now and then, and often not at all for a while
//! (people in front of it, flicker, the multiplexed rows of cheap LED boards). Single readings
//! are therefore not trusted. Instead the most plausible course of the game is fitted to all of
//! them (dynamic programming, like a Viterbi decoder):
//!
//! * a score only goes up, by 1, 2 or 3 at a time (larger jumps and corrections cost extra),
//! * the game clock only runs down, by at most a little more than one second per second, and
//!   is reset upwards at the start of a period or break,
//! * a stretch in which the clock ran down without ever stopping and no score changed is a
//!   break timer (half time, quarter break), not a period: in a period the clock stops at every
//!   whistle.
//!
//! The result are clean [`Reading`]s with the period number filled in, plus the score changes
//! and the periods / breaks found.

use super::Reading;

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineCfg {
    /// Length of a time step (s).
    pub bin: f64,
    /// Cost of one score change (in "disagreeing readings"). For sparse input (one reading per
    /// change, as in older result files) it is lowered below one reading, so that every change
    /// is kept.
    pub change_cost: f64,
    /// Extra cost when a score jumps by more than 3 at once.
    pub jump_cost: f64,
    /// Cost when a score goes down (operator correction).
    pub down_cost: f64,
    /// Cost of setting the clock upwards / making it jump.
    pub reset_cost: f64,
    /// Clock steps per time step that still count as "running" (a little above one second per second).
    pub max_run: u32,
    /// A stretch counts as a break timer when its clock ran down by at least this many seconds
    /// without a score change ...
    pub break_min_s: u32,
    /// ... and stood still for less than this share of the time it ran.
    pub break_stop_share: f64,
}

impl Default for TimelineCfg {
    fn default() -> Self {
        Self { bin: 1.0, change_cost: 2.5, jump_cost: 150.0, down_cost: 150.0, reset_cost: 60.0, max_run: 2, break_min_s: 30, break_stop_share: 0.6 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// A period of the game, numbered from 1.
    Period(u32),
    /// A break timer running down (half time, quarter break).
    Break,
}

/// A stretch between two clock resets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub t0: f64,
    pub t1: f64,
    pub clock0: u32,
    pub clock1: u32,
    /// Seconds of video time in which the clock stood still.
    pub stopped_s: f64,
    pub kind: SegmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub t: f64,
    pub period: Option<u32>,
    pub clock: Option<u32>,
    /// true = home team
    pub home: bool,
    pub from: u32,
    pub to: u32,
    pub score_home: u32,
    pub score_away: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Timeline {
    /// One cleaned state per time step (only steps from the first reading on).
    pub states: Vec<Reading>,
    pub changes: Vec<Change>,
    pub segments: Vec<Segment>,
}

/// Readings of one quantity per time step: (value, count).
type Obs = Vec<Vec<(u32, u32)>>;

fn bins(rs: &[Reading], t0: f64, n: usize, bin: f64, f: impl Fn(&Reading) -> Option<u32>) -> Obs {
    let mut o: Obs = vec![Vec::new(); n];
    for r in rs {
        let Some(v) = f(r) else { continue };
        let i = (((r.t - t0) / bin).floor().max(0.0) as usize).min(n - 1);
        match o[i].iter_mut().find(|(k, _)| *k == v) {
            Some((_, c)) => *c += 1,
            None => o[i].push((v, 1)),
        }
    }
    o
}

/// The most plausible non-decreasing course of one score.
fn fit_score(obs: &Obs, cfg: &TimelineCfg) -> Vec<Option<u32>> {
    let n = obs.len();
    let top = obs.iter().flatten().map(|(v, _)| *v).max();
    let Some(top) = top else { return vec![None; n] };
    let m = (top + 4) as usize;
    let cost_at = |i: usize, v: usize| -> f64 {
        let tot: u32 = obs[i].iter().map(|(_, c)| c).sum();
        let hit: u32 = obs[i].iter().filter(|(k, _)| *k as usize == v).map(|(_, c)| c).sum();
        (tot - hit) as f64
    };
    // the first reading decides the start: before it every value is free
    let first = obs.iter().position(|o| !o.is_empty()).unwrap_or(0);
    let mut dp: Vec<f64> = (0..m).map(|v| cost_at(first, v)).collect();
    let mut back: Vec<Vec<u16>> = Vec::with_capacity(n);
    for i in first + 1..n {
        let mut nd = vec![f64::INFINITY; m];
        let mut bk = vec![0u16; m];
        // best predecessor below v that is more than 3 lower (prefix minimum)
        let mut pre = (f64::INFINITY, 0usize);
        // best predecessor above v (suffix minimum)
        let mut suf = vec![(f64::INFINITY, 0usize); m + 1];
        for u in (0..m).rev() {
            suf[u] = if dp[u] < suf[u + 1].0 { (dp[u], u) } else { suf[u + 1] };
        }
        for v in 0..m {
            if v >= 4 && dp[v - 4] < pre.0 {
                pre = (dp[v - 4], v - 4);
            }
            let mut best = (dp[v], v);
            for d in 1..=3usize.min(v) {
                let c = dp[v - d] + cfg.change_cost;
                if c < best.0 {
                    best = (c, v - d);
                }
            }
            let c = pre.0 + cfg.change_cost + cfg.jump_cost;
            if c < best.0 {
                best = (c, pre.1);
            }
            let c = suf[v + 1].0 + cfg.down_cost;
            if c < best.0 {
                best = (c, suf[v + 1].1);
            }
            nd[v] = best.0 + cost_at(i, v);
            bk[v] = best.1 as u16;
        }
        dp = nd;
        back.push(bk);
    }
    let mut v = (0..m).min_by(|a, b| dp[*a].total_cmp(&dp[*b])).unwrap_or(0);
    let mut out = vec![None; n];
    out[n - 1] = Some(v as u32);
    for i in (first + 1..n).rev() {
        v = back[i - first - 1][v] as usize;
        out[i - 1] = Some(v as u32);
    }
    // move every change to the first step that actually shows the new value (during a stretch
    // without readings the fit is free to change anywhere)
    let mut i = first + 1;
    while i < n {
        if out[i] != out[i - 1] {
            let (old, new) = (out[i - 1], out[i]);
            let end = (i..n).find(|&j| out[j] != new).unwrap_or(n);
            let seen = (i..end).find(|&j| obs[j].iter().any(|(k, _)| Some(*k) == new));
            if let Some(s) = seen {
                for o in out.iter_mut().take(s).skip(i) {
                    *o = old;
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// The most plausible course of the clock: stands still, runs down, or is set (cost).
fn fit_clock(obs: &Obs, cfg: &TimelineCfg) -> Vec<Option<u32>> {
    let n = obs.len();
    let top = obs.iter().flatten().map(|(v, _)| *v).max();
    let Some(top) = top else { return vec![None; n] };
    let m = (top + 2) as usize;
    let r = cfg.max_run as usize;
    // a reading one second off is no contradiction (time steps and clock ticks are not aligned)
    let cost_at = |i: usize, v: usize| -> f64 {
        obs[i].iter().filter(|(k, _)| (*k as i64 - v as i64).abs() > 1).map(|(_, c)| *c as f64).sum::<f64>()
            + obs[i].iter().filter(|(k, _)| (*k as i64 - v as i64).abs() == 1).map(|(_, c)| *c as f64 * 0.25).sum::<f64>()
    };
    let first = obs.iter().position(|o| !o.is_empty()).unwrap_or(0);
    let mut dp: Vec<f64> = (0..m).map(|v| cost_at(first, v)).collect();
    let mut back: Vec<Vec<u16>> = Vec::with_capacity(n);
    for i in first + 1..n {
        let (gmin, garg) = dp.iter().enumerate().fold((f64::INFINITY, 0), |a, (k, &c)| if c < a.0 { (c, k) } else { a });
        let mut nd = vec![f64::INFINITY; m];
        let mut bk = vec![0u16; m];
        for v in 0..m {
            let mut best = (dp[v], v);
            for d in 1..=r {
                if v + d < m && dp[v + d] < best.0 {
                    best = (dp[v + d], v + d);
                }
            }
            if gmin + cfg.reset_cost < best.0 {
                best = (gmin + cfg.reset_cost, garg);
            }
            nd[v] = best.0 + cost_at(i, v);
            bk[v] = best.1 as u16;
        }
        dp = nd;
        back.push(bk);
    }
    let mut v = (0..m).min_by(|a, b| dp[*a].total_cmp(&dp[*b])).unwrap_or(0);
    let mut out = vec![None; n];
    out[n - 1] = Some(v as u32);
    for i in (first + 1..n).rev() {
        v = back[i - first - 1][v] as usize;
        out[i - 1] = Some(v as u32);
    }
    out
}

/// Fit the course of the game to raw readings (in time order or not).
pub fn resolve(raw: &[Reading], cfg: &TimelineCfg) -> Timeline {
    if raw.is_empty() {
        return Timeline::default();
    }
    let t0 = raw.iter().map(|r| r.t).fold(f64::INFINITY, f64::min);
    let t1 = raw.iter().map(|r| r.t).fold(f64::NEG_INFINITY, f64::max);
    let n = (((t1 - t0) / cfg.bin).floor() as usize + 1).max(1);
    let sparse = raw.len() * 2 < n;
    let lowered = TimelineCfg { change_cost: cfg.change_cost.min(0.9), ..cfg.clone() };
    let cfg = if sparse { &lowered } else { cfg };
    let home = fit_score(&bins(raw, t0, n, cfg.bin, |r| r.home), cfg);
    let away = fit_score(&bins(raw, t0, n, cfg.bin, |r| r.away), cfg);
    let clock = fit_clock(&bins(raw, t0, n, cfg.bin, |r| r.clock), cfg);
    let t_of = |i: usize| t0 + i as f64 * cfg.bin;

    // stretches between resets (the clock going up or jumping down by more than a run step)
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut s = 0usize;
    for i in 1..n {
        let (a, b) = (clock[i - 1], clock[i]);
        let reset = match (a, b) {
            (Some(a), Some(b)) => b > a || a - b > cfg.max_run,
            _ => false,
        };
        if reset {
            segs.push((s, i - 1));
            s = i;
        }
    }
    segs.push((s, n - 1));
    // only points that were added count (a correction downwards is no sign of play)
    let up = |s: &[Option<u32>], i: usize| matches!((s[i - 1], s[i]), (Some(x), Some(y)) if y > x);
    let scored = |a: usize, b: usize| (a + 1..=b).any(|i| up(&home, i) || up(&away, i));
    let first_clock = |a: usize, b: usize| (a..=b).find_map(|i| clock[i]);
    // the full period length: the value the clock is usually set to in stretches with points
    // (median: a break timer can be longer, e.g. 15 minutes at half time, and one start may be misread)
    let mut starts: Vec<u32> = segs.iter().filter(|&&(a, b)| scored(a, b)).filter_map(|&(a, b)| first_clock(a, b)).collect();
    if starts.is_empty() {
        starts = segs.iter().filter_map(|&(a, b)| first_clock(a, b)).collect();
    }
    starts.sort_unstable();
    let full = starts.get(starts.len() / 2).copied().unwrap_or(0);
    let mut segments = Vec::new();
    let mut period = 0u32;
    for &(a, b) in &segs {
        // before the first reading the clock is unknown
        let (Some(c0), Some(c1)) = (first_clock(a, b), clock[b]) else { continue };
        // total stand-still of the clock (a stand-still at the very end, at 0:00 or a timer that
        // is left standing, does not count)
        let trailing = (a + 1..=b).rev().take_while(|&i| clock[i] == clock[i - 1]).count();
        let still = (a + 1..=b - trailing).filter(|&i| clock[i].is_some() && clock[i] == clock[i - 1]).count();
        let stopped_s = still as f64 * cfg.bin;
        let ran = c0.saturating_sub(c1);
        // a period stops at every whistle (more stand-still than running time is normal); a break
        // timer runs through and nobody scores
        let kind = if ran >= cfg.break_min_s && !scored(a, b) && stopped_s < cfg.break_stop_share * ran as f64 {
            SegmentKind::Break
        } else if ran >= cfg.break_min_s || scored(a, b) {
            // a period starts with the clock set to its full length; a stretch that starts lower
            // continues the period before it (the clock was misread or set by hand)
            let continues = c0 + cfg.break_min_s < full && segments.iter().rev().find_map(|s: &Segment| match s.kind {
                SegmentKind::Period(_) => Some(true),
                SegmentKind::Break => Some(false),
            }) == Some(true);
            if !continues {
                period += 1;
            }
            SegmentKind::Period(period.max(1))
        } else {
            // too short to tell: belongs to whatever is around it
            continue;
        };
        segments.push(Segment { t0: t_of(a), t1: t_of(b) + cfg.bin, clock0: c0, clock1: c1, stopped_s, kind });
    }
    let period_at = |t: f64| {
        segments.iter().rev().find(|s| s.t0 <= t + 1e-9).and_then(|s| match s.kind {
            SegmentKind::Period(p) => Some(p),
            SegmentKind::Break => None,
        })
    };
    // during a break the last period stays the current one for the score
    let last_period_at = |t: f64| segments.iter().rev().filter(|s| s.t0 <= t + 1e-9).find_map(|s| match s.kind {
        SegmentKind::Period(p) => Some(p),
        SegmentKind::Break => None,
    });

    let states: Vec<Reading> = (0..n).map(|i| Reading { t: t_of(i), home: home[i], away: away[i], clock: clock[i], period: period_at(t_of(i)).or(last_period_at(t_of(i))) }).collect();
    let mut changes = Vec::new();
    for i in 1..n {
        for (is_home, ser) in [(true, &home), (false, &away)] {
            if let (Some(a), Some(b)) = (ser[i - 1], ser[i]) {
                if a != b {
                    changes.push(Change { t: t_of(i), period: states[i].period, clock: clock[i], home: is_home, from: a, to: b, score_home: home[i].unwrap_or(0), score_away: away[i].unwrap_or(0) });
                }
            }
        }
    }
    Timeline { states, changes, segments }
}

/// The cleaned states as readings, only where something changed (like the raw stream).
pub fn changed_readings(tl: &Timeline) -> Vec<Reading> {
    let mut out: Vec<Reading> = Vec::new();
    for s in &tl.states {
        let same = out.last().map(|l| l.home == s.home && l.away == s.away && l.clock == s.clock && l.period == s.period).unwrap_or(false);
        if !same {
            out.push(*s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(t: f64, h: Option<u32>, a: Option<u32>, c: Option<u32>) -> Reading {
        Reading { t, home: h, away: a, clock: c, period: None }
    }

    /// A tiny game: period 1 with stops and scores, a running break timer, period 2.
    fn game() -> Vec<Reading> {
        let mut v = Vec::new();
        let (mut h, mut a) = (0u32, 0u32);
        let mut t = 0.0;
        // period 1: clock 600 -> 400, stopping 10 s every 50 s; scores at some points
        let mut c = 600u32;
        while c > 400 {
            for _ in 0..4 {
                v.push(rd(t, Some(h), Some(a), Some(c)));
                t += 0.25;
            }
            c -= 1;
            if c % 50 == 0 {
                for _ in 0..40 {
                    v.push(rd(t, Some(h), Some(a), Some(c)));
                    t += 0.25;
                }
                if c % 100 == 0 { h += 2 } else { a += 3 }
            }
        }
        // break timer 120 -> 0, no stops
        for c in (0..=120u32).rev() {
            for _ in 0..4 {
                v.push(rd(t, Some(h), Some(a), Some(c)));
                t += 0.25;
            }
        }
        // period 2 from 600, one basket
        let mut c = 600u32;
        while c > 500 {
            for _ in 0..4 {
                v.push(rd(t, Some(h), Some(a), Some(c)));
                t += 0.25;
            }
            c -= 1;
            if c == 550 {
                for _ in 0..40 {
                    v.push(rd(t, Some(h), Some(a), Some(c)));
                    t += 0.25;
                }
                h += 1;
            }
        }
        v
    }

    #[test]
    fn clean_game_is_kept() {
        let tl = resolve(&game(), &TimelineCfg::default());
        let kinds: Vec<_> = tl.segments.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![SegmentKind::Period(1), SegmentKind::Break, SegmentKind::Period(2)]);
        let ch: Vec<(bool, u32, u32)> = tl.changes.iter().map(|c| (c.home, c.from, c.to)).collect();
        assert_eq!(ch, vec![(false, 0, 3), (true, 0, 2), (false, 3, 6), (true, 2, 4), (true, 4, 5)]);
        assert_eq!(tl.changes.last().unwrap().period, Some(2));
        assert_eq!(tl.changes[0].period, Some(1));
    }

    #[test]
    fn misreads_and_gaps_are_bridged() {
        let mut v = game();
        // every 7th reading wrong, a long gap without any reading (board hidden)
        for (i, r) in v.iter_mut().enumerate() {
            if i % 7 == 3 {
                r.home = r.home.map(|h| h + 8);
                r.away = Some(1);
                r.clock = r.clock.map(|c| c / 3);
            }
        }
        let n = v.len();
        v.drain(n / 5..n / 5 + 300);
        let tl = resolve(&v, &TimelineCfg::default());
        let ch: Vec<(bool, u32, u32)> = tl.changes.iter().map(|c| (c.home, c.from, c.to)).collect();
        assert_eq!(ch, vec![(false, 0, 3), (true, 0, 2), (false, 3, 6), (true, 2, 4), (true, 4, 5)]);
        let kinds: Vec<_> = tl.segments.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![SegmentKind::Period(1), SegmentKind::Break, SegmentKind::Period(2)]);
    }

    #[test]
    fn empty_input() {
        let tl = resolve(&[], &TimelineCfg::default());
        assert!(tl.states.is_empty() && tl.changes.is_empty());
    }
}
