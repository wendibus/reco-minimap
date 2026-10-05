//! Jersey-number voting (phase 3). OCR itself lives outside this crate;
//! it feeds `(number, confidence)` readings, this decides when to trust them.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct JerseyVotes {
    score: HashMap<u8, f32>,
}

impl JerseyVotes {
    pub fn add(&mut self, number: u8, conf: f32) {
        *self.score.entry(number).or_insert(0.0) += conf.clamp(0.0, 1.0);
    }

    /// Decayed so that a wrong early read can be overturned.
    pub fn decay(&mut self, factor: f32) {
        for v in self.score.values_mut() {
            *v *= factor;
        }
    }

    /// Best number if it has enough evidence and clearly beats the runner-up.
    /// `roster`: allowed numbers (empty = allow all).
    pub fn best(&self, roster: &[u8], min_score: f32, min_lead: f32) -> Option<u8> {
        let mut v: Vec<(u8, f32)> = self
            .score
            .iter()
            .filter(|(n, _)| roster.is_empty() || roster.contains(n))
            .map(|(n, s)| (*n, *s))
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (n, s) = *v.first()?;
        let second = v.get(1).map(|x| x.1).unwrap_or(0.0);
        if s >= min_score && s - second >= min_lead { Some(n) } else { None }
    }
}
