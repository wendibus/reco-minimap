#![cfg(feature = "tools")]
//! The live ticker: names in squad lists, the feed endpoints, and the rule that names only leave
//! the program when the operator allows them.
use std::io::{Read, Write};

use reco_minimap::jersey::{parse_roster, parse_roster_keepers, parse_roster_names};
use reco_minimap::tools::live::{self, Hub, TickerCfg};
use serde_json::{json, Value};

#[test]
fn names_follow_the_number_and_do_not_disturb_the_number_lists() {
    let s = "7:Anna Müller, 12=Ben; 1g:Tom\n4, 9 : Eva K2";
    assert_eq!(parse_roster_names(s), vec![(7, "Anna Müller".into()), (12, "Ben".into()), (1, "Tom".into()), (9, "Eva K2".into())]);
    // the numbers are read as before; digits inside a name are not squad numbers, a keeper mark still works
    assert_eq!(parse_roster(s), vec![1, 4, 7, 9, 12]);
    assert_eq!(parse_roster_keepers(s), vec![1]);
    assert_eq!(parse_roster("4, 5;7  10,10 x 23"), vec![4, 5, 7, 10, 23]);
    assert!(parse_roster_names("4, 5, 7").is_empty());
    assert_eq!(parse_roster_names("7:Anna, 7:Anne"), vec![(7, "Anna".into())], "the first name for a number wins");
    assert!(parse_roster_names("7:  ,x:Nobody").is_empty());
}

fn cfg(names: &str) -> TickerCfg {
    TickerCfg {
        names: names.into(),
        home_name: "Sharks".into(),
        away_name: "".into(),
        home_players: parse_roster_names("7:Anna Müller"),
        away_players: parse_roster_names("9:Dana"),
        sport: "football".into(),
    }
}

fn get(addr: &str, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    // one write: the hub reads a request with a single read
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    (out.split_whitespace().nth(1).unwrap().parse().unwrap(), out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string())
}

#[test]
fn the_feed_grows_and_late_viewers_can_catch_up() {
    let hub = Hub::with_ticker("{}".into(), Some(cfg("both")));
    let addr = live::serve(hub.clone(), "127.0.0.1", 0).unwrap();
    let (c, page) = get(&addr, "/ticker");
    assert_eq!(c, 200);
    assert!(page.contains("/ticker.json") && page.contains("<script src=\"/i18n.js\">"));
    let v: Value = serde_json::from_str(&get(&addr, "/ticker.json").1).unwrap();
    assert_eq!(v["seq"], 0);
    assert!(v["events"].as_array().unwrap().is_empty());

    hub.push_events(&[json!({"type": "scene", "kind": "kickoff", "team": "home", "t": 1.0})]);
    hub.push_events(&[]);
    hub.push_events(&[json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 3, "t": 60.0}), json!({"type": "sub", "kind": "possible", "team": "away", "out": 9, "in": null, "t": 90.0})]);
    let all: Value = serde_json::from_str(&get(&addr, "/ticker.json").1).unwrap();
    assert_eq!(all["seq"], 3);
    let seqs: Vec<u64> = all["events"].as_array().unwrap().iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    let late: Value = serde_json::from_str(&get(&addr, "/ticker.json?since=2").1).unwrap();
    assert_eq!(late["events"].as_array().unwrap().len(), 1);
    assert_eq!(late["events"][0]["out"], 9);
    assert_eq!(all["config"]["sport"], "football");
    assert_eq!(all["config"]["teams"]["home"], "Sharks");
}

#[test]
fn names_are_sent_only_when_the_operator_allows_them() {
    for (mode, sent) in [("numbers", false), ("names", true), ("both", true)] {
        let hub = Hub::with_ticker("{}".into(), Some(cfg(mode)));
        let addr = live::serve(hub.clone(), "127.0.0.1", 0).unwrap();
        hub.push_events(&[json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 3, "t": 5.0})]);
        let body = get(&addr, "/ticker.json").1;
        assert_eq!(body.contains("Anna"), sent, "{mode}: {body}");
        assert_eq!(body.contains("Dana"), sent, "{mode}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["config"]["names"], mode);
        // a number-only ticker also leaves the live feed's /config.json untouched: no names there either
        assert!(!get(&addr, "/config.json").1.contains("Anna"));
    }
    // a hub without ticker settings is a numbers-only ticker
    let hub = Hub::new("{}".into());
    let addr = live::serve(hub, "127.0.0.1", 0).unwrap();
    let v: Value = serde_json::from_str(&get(&addr, "/ticker.json").1).unwrap();
    assert_eq!(v["config"]["names"], "numbers");
}

#[test]
fn the_name_mode_is_parsed_in_all_the_ways_people_type_it() {
    assert_eq!(TickerCfg::parse_mode("Numbers"), Some("numbers"));
    assert_eq!(TickerCfg::parse_mode(" namen "), Some("names"));
    assert_eq!(TickerCfg::parse_mode("beides"), Some("both"));
    assert_eq!(TickerCfg::parse_mode("everything"), None);
}

#[test]
fn the_feed_keeps_only_the_newest_entries() {
    let hub = Hub::with_ticker("{}".into(), Some(cfg("numbers")));
    for i in 0..650 {
        hub.push_events(&[json!({"type": "scene", "kind": "corner", "team": "home", "t": i})]);
    }
    let v: Value = serde_json::from_str(&hub.ticker_json(0)).unwrap();
    assert_eq!(v["seq"], 650);
    assert_eq!(v["events"].as_array().unwrap().len(), 600);
    assert_eq!(v["events"][0]["seq"], 51);
}
