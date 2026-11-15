//! Additional, versioned evidence for detector replay. Cached OCR is applied
//! only to the same visible slot/segment/team AND image box; changed tracking
//! rejects the evidence instead of silently assigning a number to somebody else.
use super::ocr::Reading;
use crate::{
    jersey::DIGIT_CLASSES,
    team::Team,
    tracker::{Track, TrackState},
    Minimap,
};
use serde_json::{json, Value};

pub use super::models::sha256_hex as sha256;

pub fn team_key(team: Team) -> &'static str {
    match team {
        Team::Home => "home",
        Team::Away => "away",
        Team::Unknown => "unknown",
    }
}

pub fn record(track: &Track, reading: &Reading, quality: f32) -> Value {
    let b = track.bbox.unwrap();
    json!({"id":track.id,"segment":track.segment,"team":team_key(track.team),
        "bbox":[b.cx,b.cy,b.w,b.h],"steps":reading.steps,"probs":reading.probs,"quality":quality})
}

/// Returns (applied, rejected). Probabilities are kept, rather than a decoded
/// number, so replay can use a different roster without running the model again.
pub fn apply(mm: &mut Minimap, records: &[Value], t: f64) -> (usize, usize) {
    let (mut applied, mut rejected) = (0, 0);
    let mut seen = std::collections::HashSet::new();
    for r in records {
        let valid = (|| {
            let id = u32::try_from(r["id"].as_u64()?).ok()?;
            let seg = u32::try_from(r["segment"].as_u64()?).ok()?;
            if !seen.insert((id, seg)) {
                return None;
            }
            let tr = mm.tracker().tracks().iter().find(|tr| {
                tr.id == id
                    && tr.segment == seg
                    && team_key(tr.team) == r["team"].as_str().unwrap_or("")
                    && tr.state == TrackState::Visible
                    && (tr.last_seen - t).abs() < 1e-6
            })?;
            let b = tr.bbox?;
            let coords = [b.cx, b.cy, b.w, b.h];
            let stored = r["bbox"].as_array()?;
            if stored.len() != 4
                || coords.iter().zip(stored).any(|(a, b)| {
                    b.as_f64()
                        .is_none_or(|b| !b.is_finite() || (*a as f64 - b).abs() > 1e-5)
                })
            {
                return None;
            }
            let steps = usize::try_from(r["steps"].as_u64()?).ok()?;
            if steps == 0 || steps > 320 {
                return None;
            }
            let values = r["probs"].as_array()?;
            if values.len() != steps * DIGIT_CLASSES {
                return None;
            }
            let probs: Option<Vec<f32>> = values
                .iter()
                .map(|v| {
                    v.as_f64()
                        .filter(|x| x.is_finite() && (0.0..=1.0).contains(x))
                        .map(|x| x as f32)
                })
                .collect();
            Some((id, probs?, steps))
        })();
        if let Some((id, probs, steps)) = valid {
            mm.add_number_probs(id, &probs, steps);
            applied += 1;
        } else {
            rejected += 1;
        }
    }
    (applied, rejected)
}
