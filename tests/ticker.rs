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
        approve: false,
    }
}

fn get(addr: &str, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    // one write: the hub reads a request with a single read
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
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
    assert!(page.contains("<script src=\"/ticker.js\">") && page.contains("<div id=\"app\">"));
    let js = get(&addr, "/ticker.js").1;
    assert!(js.contains("RT_STRINGS") && js.contains("/ticker/admin") && !js.contains("\"/i18n.js\""));
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

fn post(addr: &str, path: &str, marked: bool) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let extra = if marked { "x-ticker-admin: 1\r\n" } else { "" };
    s.write_all(format!("POST {path} HTTP/1.1\r\nHost: {addr}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    (out.split_whitespace().nth(1).unwrap().parse().unwrap(), out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string())
}

fn manual_hub(names: &str) -> (std::sync::Arc<Hub>, String) {
    let mut c = cfg(names);
    c.approve = true;
    let hub = Hub::with_ticker("{}".into(), Some(c));
    let addr = live::serve(hub.clone(), "127.0.0.1", 0).unwrap();
    (hub, addr)
}

fn feed(addr: &str) -> Value {
    serde_json::from_str(&get(addr, "/ticker.json").1).unwrap()
}

#[test]
fn nothing_reaches_the_public_feed_until_it_is_released() {
    let (hub, addr) = manual_hub("numbers");
    hub.push_events(&[json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 3, "t": 5.0}), json!({"type": "scene", "kind": "corner", "team": "away", "t": 9.0})]);
    assert!(feed(&addr)["events"].as_array().unwrap().is_empty(), "queued, not published");
    let admin: Value = serde_json::from_str(&get(&addr, "/ticker/admin.json").1).unwrap();
    assert_eq!(admin["approve"], true);
    let ids: Vec<u64> = admin["pending"].as_array().unwrap().iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2);

    // release the second, discard the first
    assert_eq!(post(&addr, &format!("/ticker/admin/release?id={}", ids[1]), true).0, 200);
    assert_eq!(post(&addr, &format!("/ticker/admin/discard?id={}", ids[0]), true).0, 200);
    let f = feed(&addr);
    assert_eq!(f["events"].as_array().unwrap().len(), 1);
    assert_eq!(f["events"][0]["kind"], "corner");
    assert_eq!(f["events"][0]["seq"], 1, "the public sequence counts released entries");
    assert!(f["events"][0].get("id").is_none(), "queue ids stay private");
    // a viewer who already has seq 1 still gets the next release, whatever order the events were detected in
    hub.push_events(&[json!({"type": "scene", "kind": "penalty", "team": "home", "t": 20.0}), json!({"type": "scene", "kind": "kickoff", "team": "home", "t": 3.0})]);
    assert_eq!(post(&addr, "/ticker/admin/release_all", true).0, 200);
    let later: Value = serde_json::from_str(&get(&addr, "/ticker.json?since=1").1).unwrap();
    assert_eq!(later["events"].as_array().unwrap().len(), 2);
    // releasing something twice, or an unknown id, does nothing
    assert!(!hub.decide(ids[1], true) && !hub.decide(999, true));
    assert_eq!(feed(&addr)["seq"], 3);
}

#[test]
fn a_published_entry_can_be_taken_back_and_the_mode_switched() {
    let (hub, addr) = manual_hub("numbers");
    hub.set_approval(false);
    hub.push_events(&[json!({"type": "scene", "kind": "corner", "team": "away", "t": 9.0}), json!({"type": "scene", "kind": "corner", "team": "home", "t": 19.0})]);
    assert_eq!(feed(&addr)["events"].as_array().unwrap().len(), 2, "automatic mode publishes at once");
    assert_eq!(post(&addr, "/ticker/admin/retract?seq=1", true).0, 200);
    let f = feed(&addr);
    assert_eq!(f["events"].as_array().unwrap().len(), 1);
    assert_eq!(f["retracted"], json!([1]), "viewers who already hold it are told to drop it");
    assert_eq!(post(&addr, "/ticker/admin/mode?approve=1", true).0, 200);
    hub.push_events(&[json!({"type": "scene", "kind": "throw_in", "team": "home", "t": 30.0})]);
    assert_eq!(feed(&addr)["seq"], 2, "back in manual mode nothing new is published");
}

#[test]
fn only_the_operator_page_on_this_computer_may_release() {
    let (_hub, addr) = manual_hub("numbers");
    // changes need the marker header (other web pages cannot send it without the hub's consent)
    assert_eq!(post(&addr, "/ticker/admin/release_all", false).0, 403);
    assert_eq!(get(&addr, "/ticker/admin.json").0, 200, "this computer, by its own name");
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.write_all(b"GET /ticker/admin.json HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 403"), "a foreign Host name is refused: {out}");
    use reco_minimap::tools::live::admin_allowed;
    assert!(admin_allowed(true, Some("127.0.0.1:8765"), None, true, true));
    assert!(admin_allowed(true, Some("localhost:8765"), Some("http://localhost:8765"), false, false), "reading needs no marker");
    assert!(!admin_allowed(false, Some("127.0.0.1:8765"), None, true, true), "another computer");
    assert!(!admin_allowed(true, Some("evil.example"), None, true, true), "foreign Host (DNS rebinding)");
    assert!(!admin_allowed(true, Some("127.0.0.1:8765"), Some("http://evil.example"), true, true), "another web page");
    assert!(!admin_allowed(true, Some("127.0.0.1:8765"), None, false, true), "change without the marker");
}

#[test]
fn the_operator_sees_names_even_when_the_public_does_not() {
    let (_hub, addr) = manual_hub("numbers");
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.write_all(format!("GET /ticker/admin.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.contains("Anna"), "{out}");
    assert!(!get(&addr, "/ticker.json").1.contains("Anna"));
}

// ---------------------------------------------------------------- for a web site: export, push, widget

use reco_minimap::tools::ticker_pub::{self, Sink};

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("reco-ticker-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_export_folder_holds_static_files_only() {
    let d = tmpdir("assets");
    ticker_pub::write_assets(&d).unwrap();
    for f in ["ticker.html", "ticker.js", "index.html", ".htaccess"] {
        assert!(d.join(f).exists(), "{f}");
    }
    assert!(std::fs::read_dir(&d).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".php")), "no server-side code");
    let page = std::fs::read_to_string(d.join("ticker.html")).unwrap();
    assert!(page.contains("name=\"ticker-static\"") && page.contains("<script src=\"ticker.js\"></script>") && !page.contains("/ticker.js\""), "relative, no hub paths");
    let js = std::fs::read_to_string(d.join("ticker.js")).unwrap();
    assert!(js.starts_with("(function(){") && js.contains("customElements.define('reco-ticker'") && js.contains("data-feed"));
    assert!(js.contains("const RT_STRINGS"), "the texts travel with the widget");
    assert!(js.is_ascii(), "readable whatever charset the web server announces");
    assert!(js.contains("\\u2190") && js.contains("Wechsel"));
    let idx = std::fs::read_to_string(d.join("index.html")).unwrap();
    assert!(idx.contains("&lt;iframe") && idx.contains("data-feed") && idx.contains("--rt-home"));
    // the widget is valid JavaScript
    if std::process::Command::new("node").arg("--version").output().is_ok() {
        let st = std::process::Command::new("node").arg("--check").arg(d.join("ticker.js")).output().unwrap();
        assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    }
}

#[test]
fn the_sender_keeps_ticker_json_current_with_a_heartbeat_and_without_hidden_names() {
    let d = tmpdir("sender");
    let hub = Hub::with_ticker("{}".into(), Some(cfg("numbers")));
    let sink = Sink { dir: Some(d.clone()), method: "PUT".into(), ..Default::default() };
    ticker_pub::check(&sink).unwrap();
    ticker_pub::spawn(hub.clone(), sink);
    let read = || -> Option<Value> { serde_json::from_str(&std::fs::read_to_string(d.join("ticker.json")).ok()?).ok() };
    let wait = |f: &dyn Fn(&Value) -> bool| { for _ in 0..60 { if let Some(v) = read() { if f(&v) { return v; } } std::thread::sleep(std::time::Duration::from_millis(100)); } panic!("timed out"); };
    let first = wait(&|v| v["beat"].as_u64().is_some());
    assert_eq!(first["events"].as_array().unwrap().len(), 0);
    hub.push_events(&[json!({"type": "sub", "kind": "confirmed", "team": "home", "out": 7, "in": 3, "t": 5.0})]);
    let v = wait(&|v| v["seq"] == 1);
    assert!(v["beat"].as_u64().unwrap() > first["beat"].as_u64().unwrap(), "a change writes a new beat");
    assert!(!std::fs::read_to_string(d.join("ticker.json")).unwrap().contains("Anna"), "numbers only: no names in the file");
    assert!(!d.join("ticker.json.tmp").exists());
}

#[test]
fn queued_events_stay_out_of_the_exported_feed() {
    let d = tmpdir("queue");
    let mut c = cfg("both");
    c.approve = true;
    let hub = Hub::with_ticker("{}".into(), Some(c));
    ticker_pub::spawn(hub.clone(), Sink { dir: Some(d.clone()), method: "PUT".into(), ..Default::default() });
    hub.push_events(&[json!({"type": "scene", "kind": "corner", "team": "home", "t": 1.0})]);
    std::thread::sleep(std::time::Duration::from_millis(1800));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(d.join("ticker.json")).unwrap()).unwrap();
    assert!(v["events"].as_array().unwrap().is_empty());
    hub.release_all();
    std::thread::sleep(std::time::Duration::from_millis(1800));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(d.join("ticker.json")).unwrap()).unwrap();
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
}

#[test]
fn the_upload_sends_the_feed_with_put_and_the_key() {
    if std::process::Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    // a one-request web server that keeps what it was sent
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                buf.extend_from_slice(&tmp[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some((h, b)) = text.split_once("\r\n\r\n") {
                    let want = h.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if b.len() >= want || n == 0 {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = s.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
        }
    });
    let hub = Hub::with_ticker("{}".into(), Some(cfg("numbers")));
    hub.push_events(&[json!({"type": "scene", "kind": "corner", "team": "home", "t": 1.0})]);
    let sink = Sink { push: Some(format!("http://127.0.0.1:{port}/dav/ticker.json")), method: "PUT".into(), key: Some("s3cret".into()), ..Default::default() };
    ticker_pub::check(&sink).unwrap();
    ticker_pub::spawn(hub, sink);
    let req = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("an upload arrives");
    assert!(req.starts_with("PUT /dav/ticker.json "), "{req}");
    assert!(req.to_ascii_lowercase().contains("authorization: bearer s3cret"), "{req}");
    let body: Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["events"][0]["kind"], "corner");
    assert!(body["beat"].as_u64().is_some());
}

#[test]
fn the_push_address_is_checked_and_passwords_are_not_printed() {
    assert!(ticker_pub::check(&Sink { push: Some("gopher://x/y".into()), ..Default::default() }).is_err());
    assert!(ticker_pub::check(&Sink { push: Some("https://example.org/t.json".into()), ..Default::default() }).is_ok() || std::process::Command::new("curl").arg("--version").output().is_err());
    assert_eq!(ticker_pub::mask_url("https://user:pw@dav.example.org/f/t.json"), "https://***@dav.example.org/f/t.json");
    assert_eq!(ticker_pub::mask_url("https://example.org/a@b"), "https://example.org/a@b");
}
