//! Offline identity review, without rerunning inference or changing source data.
//!
//! An annotation document is bound to the exact source SHA-256. Corrections use
//! (slot, segment, half-open time interval), never a reusable slot alone. The
//! roster belongs to this analysis and does not establish global person IDs.
use crate::{tr, trs};
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
        .ok_or_else(|| tr!("{key} fehlt oder ist leer", "Missing non-empty {key}", "{key} manquant ou vide", "Falta {key} o está vacío"))
}
fn integer(v: &Value, key: &str) -> Result<u64, String> {
    v[key].as_u64().ok_or_else(|| tr!("Ungültiger Wert für {key}", "Invalid {key}", "Valeur invalide pour {key}", "Valor no válido para {key}"))
}
fn time(v: &Value, key: &str) -> Result<f64, String> {
    v[key]
        .as_f64()
        .filter(|x| x.is_finite() && *x >= 0.0)
        .ok_or_else(|| tr!("Ungültiger Wert für {key}", "Invalid {key}", "Valeur invalide pour {key}", "Valor no válido para {key}"))
}

impl Review {
    pub fn parse(v: &Value) -> Result<Self, String> {
        if v["schema_version"].as_u64() != Some(1) {
            return Err(trs!("schema_version 1 erwartet", "Expected schema_version 1", "schema_version 1 attendu", "Se esperaba schema_version 1").into());
        }
        let source_sha256 = string(v, "source_sha256")?.to_lowercase();
        if source_sha256.len() != 64 || !source_sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(trs!("Ungültiger source_sha256", "Invalid source_sha256", "source_sha256 invalide", "source_sha256 no válido").into());
        }
        let mut roster: Vec<Player> = Vec::new();
        for p in v["roster"].as_array().ok_or(trs!("Liste roster fehlt", "Missing roster array", "Liste roster manquante", "Falta la lista roster"))? {
            let player = Player {
                id: string(p, "id")?,
                name: string(p, "name")?,
                team: string(p, "team")?,
                number: u8::try_from(integer(p, "number")?).map_err(|_| trs!("Ungültige Nummer", "Invalid number", "Numéro invalide", "Número no válido"))?,
            };
            if !matches!(player.team.as_str(), "home" | "away") || player.number > 99 {
                return Err(trs!("home/away und Nummer 0..99 erwartet", "Expected home/away and number 0..99", "home/away et numéro 0..99 attendus", "Se esperaban home/away y un número 0..99").into());
            }
            if roster
                .iter()
                .any(|q| q.id == player.id || (q.team == player.team && q.number == player.number))
            {
                return Err(trs!("Kader-ID oder Mannschaft/Nummer doppelt", "Duplicate roster ID or team/number", "ID d'effectif ou équipe/numéro en double", "ID de plantilla o equipo/número duplicado").into());
            }
            roster.push(player);
        }
        let mut corrections: Vec<Correction> = Vec::new();
        for c in v["corrections"]
            .as_array()
            .ok_or(trs!("Liste corrections fehlt", "Missing corrections array", "Liste corrections manquante", "Falta la lista corrections"))?
        {
            let player = match c.get("player_id") {
                Some(Value::Null) => None, // Explicitly unknown, not a missing field.
                Some(Value::String(id)) if roster.iter().any(|p| p.id == *id) => Some(id.clone()),
                _ => return Err(trs!("player_id muss einen Kadereintrag nennen oder null sein", "player_id must name a roster entry or be null", "player_id doit désigner une entrée de l'effectif ou être null", "player_id debe nombrar una entrada de la plantilla o ser null").into()),
            };
            let correction = Correction {
                slot: integer(c, "slot")?,
                segment: integer(c, "segment")?,
                start: time(c, "start")?,
                end: time(c, "end")?,
                player,
            };
            if correction.end <= correction.start {
                return Err(trs!("start < end erwartet", "Expected start < end", "start < end attendu", "Se esperaba start < end").into());
            }
            if corrections.iter().any(|q| {
                q.slot == correction.slot
                    && q.segment == correction.segment
                    && q.start < correction.end
                    && correction.start < q.end
            }) {
                return Err(trs!("Korrekturen desselben Tracking-Abschnitts überschneiden sich", "Overlapping corrections for the same track segment", "Des corrections du même segment de suivi se chevauchent", "Las correcciones del mismo segmento de seguimiento se solapan").into());
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
            return Err(trs!("Die Eingabe ist bereits korrigiert; bitte die Originalanalyse verwenden", "Input is already reviewed; use the original analysis", "L'entrée est déjà corrigée ; utilisez l'analyse d'origine", "La entrada ya está revisada; use el análisis original").into());
        }
        let t = time(frame, "t")?;
        let players = frame["players"]
            .as_array_mut()
            .ok_or(trs!("Liste players fehlt", "Missing players array", "Liste players manquante", "Falta la lista players"))?;
        for a in players.iter_mut() {
            if !a.is_object() {
                return Err(trs!("Ein Spieler muss ein Objekt sein", "Player must be an object", "Un joueur doit être un objet", "Un jugador debe ser un objeto").into());
            }
            if a.get("identity_before_review").is_some() {
                return Err(trs!("Die Eingabe ist bereits korrigiert; bitte die Originalanalyse verwenden", "Input is already reviewed; use the original analysis", "L'entrée est déjà corrigée ; utilisez l'analyse d'origine", "La entrada ya está revisada; use el análisis original").into());
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
                        return Err(tr!("Slot {slot}: Mannschaft passt nicht; die Identitätsprüfung kann die Mannschaftszuordnung nicht ändern", "Team mismatch for slot {slot}; identity review cannot change team classification", "Slot {slot} : équipe différente ; la revue d'identité ne peut pas modifier l'attribution d'équipe", "Slot {slot}: el equipo no coincide; la revisión de identidad no puede cambiar la asignación de equipo"));
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
        return Err(trs!("SHA-256 der Quelle passt nicht zu den Anmerkungen", "Source SHA-256 does not match annotations", "Le SHA-256 de la source ne correspond pas aux annotations", "El SHA-256 de la fuente no coincide con las anotaciones").into());
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
                serde_json::from_str(&line).map_err(|e| tr!("Zeile {}: {e}", "Line {}: {e}", "Ligne {} : {e}", "Línea {}: {e}", index + 1))?;
            review
                .apply(&mut frame)
                .map_err(|e| tr!("Zeile {}: {e}", "Line {}: {e}", "Ligne {} : {e}", "Línea {}: {e}", index + 1))?;
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
            return Err(tr!(
                "Korrektur {} trifft keinen Frame/Tracking-Abschnitt",
                "Correction {} matches no frame/track segment",
                "La correction {} ne correspond à aucune image ni segment de suivi",
                "La corrección {} no coincide con ningún fotograma o segmento de seguimiento",
                index + 1
            ));
        }
        writer.flush().map_err(|e| e.to_string())?;
        Ok(count)
    })();
    if result.is_err() {
        fs::remove_file(output)
            .map_err(|e| tr!("Review fehlgeschlagen; die unvollständige Ausgabe konnte nicht entfernt werden: {e}", "Review failed; incomplete output could not be removed: {e}", "Échec de la revue ; la sortie incomplète n'a pas pu être supprimée : {e}", "La revisión falló; no se pudo eliminar la salida incompleta: {e}"))?;
    }
    result
}
