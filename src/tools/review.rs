//! Offline identity review, without rerunning inference or changing source data.
//!
//! An annotation document is bound to the exact source SHA-256. Corrections use
//! (slot, segment, half-open time interval), never a reusable slot alone. The
//! roster belongs to this analysis and does not establish global person IDs.
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

#[derive(Clone, Debug)]
struct Player {
    id: String,
    name: String,
    team: String,
    number: u8,
}
#[derive(Clone, Debug)]
struct Correction {
    slot: u64,
    segment: u64,
    start: f64,
    end: f64,
    player: Option<String>,
}

pub struct Review {
    source_sha256: String,
    roster: Vec<Player>,
    corrections: Vec<Correction>,
}

fn string(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("Missing non-empty {key}"))
}
fn integer(v: &Value, key: &str) -> Result<u64, String> {
    v[key].as_u64().ok_or_else(|| format!("Invalid {key}"))
}
fn time(v: &Value, key: &str) -> Result<f64, String> {
    v[key]
        .as_f64()
        .filter(|x| x.is_finite() && *x >= 0.0)
        .ok_or_else(|| format!("Invalid {key}"))
}

impl Review {
    pub fn parse(v: &Value) -> Result<Self, String> {
        if v["schema_version"].as_u64() != Some(1) {
            return Err("Expected schema_version 1".into());
        }
        let source_sha256 = string(v, "source_sha256")?.to_lowercase();
        if source_sha256.len() != 64 || !source_sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid source_sha256".into());
        }
        let mut roster: Vec<Player> = Vec::new();
        for p in v["roster"].as_array().ok_or("Missing roster array")? {
            let player = Player {
                id: string(p, "id")?,
                name: string(p, "name")?,
                team: string(p, "team")?,
                number: u8::try_from(integer(p, "number")?).map_err(|_| "Invalid number")?,
            };
            if !matches!(player.team.as_str(), "home" | "away") || player.number > 99 {
                return Err("Expected home/away and number 0..99".into());
            }
            if roster
                .iter()
                .any(|q| q.id == player.id || (q.team == player.team && q.number == player.number))
            {
                return Err("Duplicate roster ID or team/number".into());
            }
            roster.push(player);
        }
        let mut corrections: Vec<Correction> = Vec::new();
        for c in v["corrections"]
            .as_array()
            .ok_or("Missing corrections array")?
        {
            let player = match c.get("player_id") {
                Some(Value::Null) => None, // Explicitly unknown, not a missing field.
                Some(Value::String(id)) if roster.iter().any(|p| p.id == *id) => Some(id.clone()),
                _ => return Err("player_id must name a roster entry or be null".into()),
            };
            let correction = Correction {
                slot: integer(c, "slot")?,
                segment: integer(c, "segment")?,
                start: time(c, "start")?,
                end: time(c, "end")?,
                player,
            };
            if correction.end <= correction.start {
                return Err("Expected start < end".into());
            }
            if corrections.iter().any(|q| {
                q.slot == correction.slot
                    && q.segment == correction.segment
                    && q.start < correction.end
                    && correction.start < q.end
            }) {
                return Err("Overlapping corrections for the same track segment".into());
            }
            corrections.push(correction);
        }
        Ok(Self {
            source_sha256,
            roster,
            corrections,
        })
    }

    /// Enrich a frame, preserving the original identity fields for audit. Ball
    /// and event history stay untouched: identity corrections do not prove that
    /// a possession or substitution detector was correct.
    pub fn apply(&self, frame: &mut Value) -> Result<(), String> {
        if frame.get("identity_review").is_some() {
            return Err("Input is already reviewed; use the original analysis".into());
        }
        let t = time(frame, "t")?;
        let players = frame["players"]
            .as_array_mut()
            .ok_or("Missing players array")?;
        for a in players.iter_mut() {
            if !a.is_object() {
                return Err("Player must be an object".into());
            }
            if a.get("identity_before_review").is_some() {
                return Err("Input is already reviewed; use the original analysis".into());
            }
            let slot = integer(a, "id")?;
            let segment = integer(a, "segment")?;
            a["identity_before_review"] = json!({"identity": a["identity"], "jersey": a["jersey"]});
            let correction = self
                .corrections
                .iter()
                .find(|c| c.slot == slot && c.segment == segment && c.start <= t && t < c.end);
            if let Some(c) = correction {
                if let Some(id) = &c.player {
                    let p = self
                        .roster
                        .iter()
                        .find(|p| p.id == *id)
                        .expect("validated roster reference");
                    if a["team"].as_str() != Some(p.team.as_str()) {
                        return Err(format!("Team mismatch for slot {slot}; identity review cannot change team classification"));
                    }
                    a["identity"] = json!({"status":"manual", "player":{"team":p.team,"number":p.number}, "roster_id":p.id, "name":p.name, "source":"manual", "interval":{"start":c.start,"end":c.end}});
                    a["jersey"] = json!(p.number);
                } else {
                    a["identity"] = json!({"status":"unknown","player":null,"source":"manual","interval":{"start":c.start,"end":c.end}});
                    a["jersey"] = Value::Null;
                }
            } else if let Some(p) = self.roster.iter().find(|p| {
                a["identity"]["player"]["team"].as_str() == Some(p.team.as_str())
                    && a["identity"]["player"]["number"].as_u64() == Some(p.number as u64)
            }) {
                // A matching roster name does not upgrade an automatic candidate.
                a["identity"]["roster_id"] = json!(p.id);
                a["identity"]["name"] = json!(p.name);
            }
        }
        // Neither two manual claims nor manual-vs-automatic collisions silently
        // choose a winner. Preserve provenance, mark both and suppress labels.
        let claims: Vec<_> = players
            .iter()
            .map(|a| {
                if a["state"] == "lost" {
                    None
                } else {
                    let p = &a["identity"]["player"];
                    p["team"]
                        .as_str()
                        .zip(p["number"].as_u64())
                        .map(|(t, n)| (t.to_owned(), n))
                }
            })
            .collect();
        for (a, claim) in players.iter_mut().zip(&claims) {
            if claim.is_some() && claims.iter().filter(|q| *q == claim).count() > 1 {
                a["identity"]["status"] = json!("conflict");
                a["jersey"] = Value::Null;
            }
        }
        Ok(())
    }
}

/// Stream one frame at a time; never overwrite any existing file. On error the
/// newly created incomplete output is removed, source and annotations unchanged.
pub fn run(input: &Path, annotations: &Path, output: &Path) -> Result<usize, String> {
    let document = fs::read(annotations).map_err(|e| e.to_string())?;
    use sha2::{Digest, Sha256};
    let annotations_sha256 = format!("{:x}", Sha256::digest(&document));
    let review = Review::parse(&serde_json::from_slice(&document).map_err(|e| e.to_string())?)?;
    if super::models::sha256_hex(input)? != review.source_sha256 {
        return Err("Source SHA-256 does not match annotations".into());
    }
    let reader = BufReader::new(fs::File::open(input).map_err(|e| e.to_string())?);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        let mut count = 0;
        let mut matched = vec![false; review.corrections.len()];
        for (index, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| e.to_string())?;
            let mut frame: Value =
                serde_json::from_str(&line).map_err(|e| format!("Line {}: {e}", index + 1))?;
            review
                .apply(&mut frame)
                .map_err(|e| format!("Line {}: {e}", index + 1))?;
            for (c, seen) in review.corrections.iter().zip(&mut matched) {
                let t = frame["t"].as_f64().expect("validated timestamp");
                if c.start <= t
                    && t < c.end
                    && frame["players"]
                        .as_array()
                        .expect("validated players")
                        .iter()
                        .any(|a| {
                            a["id"].as_u64() == Some(c.slot)
                                && a["segment"].as_u64() == Some(c.segment)
                        })
                {
                    *seen = true;
                }
            }
            frame["identity_review"] = json!({"schema_version":1,"source_sha256":review.source_sha256,"annotations_sha256":annotations_sha256});
            serde_json::to_writer(&mut writer, &frame).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
            count += 1;
        }
        if let Some(index) = matched.iter().position(|seen| !seen) {
            return Err(format!(
                "Correction {} matches no frame/track segment",
                index + 1
            ));
        }
        writer.flush().map_err(|e| e.to_string())?;
        Ok(count)
    })();
    if result.is_err() {
        fs::remove_file(output)
            .map_err(|e| format!("Review failed; incomplete output could not be removed: {e}"))?;
    }
    result
}
