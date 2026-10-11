#![cfg(feature = "tools")]
use reco_minimap::tools::{
    models::sha256_hex,
    review::{run, Review},
};
use serde_json::{json, Value};

fn document() -> Value {
    json!({"schema_version":1,"source_sha256":"a".repeat(64),
        "roster":[{"id":"home-7","name":"Alex","team":"home","number":7}],
        "corrections":[{"slot":3,"segment":1,"start":10.0,"end":20.0,"player_id":"home-7"}]})
}
fn frame(t: f64, segment: u32) -> Value {
    json!({"t":t,"players":[{"id":3,"segment":segment,"team":"home","state":"visible","jersey":null,"identity":{"status":"unknown","player":null}}],"ball":{"state":"unknown"},"events":[]})
}

#[test]
fn corrections_are_segment_and_interval_scoped_and_auditable() {
    let review = Review::parse(&document()).unwrap();
    for (t, segment, applied) in [
        (9.9, 1, false),
        (10.0, 1, true),
        (19.9, 1, true),
        (20.0, 1, false),
        (15.0, 2, false),
    ] {
        let mut f = frame(t, segment);
        review.apply(&mut f).unwrap();
        assert_eq!(f["players"][0]["identity"]["status"] == "manual", applied);
        assert_eq!(
            f["players"][0]["identity_before_review"]["identity"]["status"],
            "unknown"
        );
        assert_eq!(f["ball"]["state"], "unknown");
        assert!(
            review.apply(&mut f).is_err(),
            "use original input for each review"
        );
    }
}

#[test]
fn rejects_ambiguous_rosters_intervals_and_references() {
    let mut d = document();
    let duplicate = d["roster"][0].clone();
    d["roster"].as_array_mut().unwrap().push(duplicate);
    assert!(Review::parse(&d).is_err());
    let mut d = document();
    let duplicate = d["corrections"][0].clone();
    d["corrections"].as_array_mut().unwrap().push(duplicate);
    assert!(Review::parse(&d).is_err());
    let mut d = document();
    d["corrections"][0]["player_id"] = json!("missing");
    assert!(Review::parse(&d).is_err());
}

#[test]
fn manual_unknown_and_conflicts_do_not_invent_certainty() {
    let mut d = document();
    d["corrections"][0]["player_id"] = Value::Null;
    let mut f = frame(12.0, 1);
    f["players"][0]["jersey"] = json!(7);
    Review::parse(&d).unwrap().apply(&mut f).unwrap();
    assert!(f["players"][0]["jersey"].is_null());
    let mut f = frame(12.0, 1);
    f["players"].as_array_mut().unwrap().push(json!({"id":4,"segment":0,"team":"home","state":"visible","jersey":7,"identity":{"status":"candidate","player":{"team":"home","number":7}}}));
    Review::parse(&document()).unwrap().apply(&mut f).unwrap();
    for a in f["players"].as_array().unwrap() {
        assert_eq!(a["identity"]["status"], "conflict");
        assert!(a["jersey"].is_null());
    }
}

#[test]
fn roster_enrichment_keeps_candidate_status_and_wrong_team_is_rejected() {
    let review = Review::parse(&document()).unwrap();
    let mut f = frame(5.0, 1);
    f["players"][0]["identity"] = json!({"status":"candidate","player":{"team":"home","number":7}});
    review.apply(&mut f).unwrap();
    assert_eq!(f["players"][0]["identity"]["name"], "Alex");
    assert_eq!(f["players"][0]["identity"]["status"], "candidate");
    let mut f = frame(12.0, 1);
    f["players"][0]["team"] = json!("away");
    assert!(review.apply(&mut f).is_err());
    let mut f = frame(12.0, 1);
    f["players"][0].as_object_mut().unwrap().remove("segment");
    assert!(review.apply(&mut f).is_err());
}

#[test]
fn streaming_review_preserves_source_and_refuses_overwrite() {
    let dir = std::env::temp_dir().join(format!(
        "reco-review-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let input = dir.join("source.jsonl");
    let annotations = dir.join("review.json");
    let output = dir.join("result.jsonl");
    let original = format!("{}\n", frame(12.0, 1));
    std::fs::write(&input, &original).unwrap();
    let mut d = document();
    std::fs::write(&annotations, d.to_string()).unwrap();
    assert!(run(&input, &annotations, &output).is_err());
    assert!(!output.exists());
    d["source_sha256"] = json!(sha256_hex(&input).unwrap());
    std::fs::write(&annotations, d.to_string()).unwrap();
    assert_eq!(run(&input, &annotations, &output).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&input).unwrap(), original);
    let result = std::fs::read_to_string(&output).unwrap();
    assert!(run(&input, &annotations, &output).is_err());
    assert_eq!(std::fs::read_to_string(&output).unwrap(), result);
    // Wrong segment must fail rather than silently producing an uncorrected file.
    d["corrections"][0]["segment"] = json!(99);
    std::fs::write(&annotations, d.to_string()).unwrap();
    let invalid = dir.join("invalid.jsonl");
    assert!(run(&input, &annotations, &invalid).is_err());
    assert!(!invalid.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
