//! Live hub: serves the overlay page and the tracking state to OBS (Browser Source),
//! Spieldock or any other consumer. std-only HTTP server.
//!
//!   GET /              overlay page (transparent canvas; add ?floor=1 to test in a normal browser)
//!   GET /scoreboard    scoreboard and watermark for OBS (transparent, 1920x1080; the project's `overlay`)
//!   GET /events        server-sent events: one JSON message per detection update
//!   GET /state.json    latest message (polling)
//!   GET /config.json   colours
//!   GET /ticker        live ticker page (substitutions and game situations as a feed; works on a phone)
//!   GET /ticker.json   the feed so far (`?since=N` for only the newer entries) and the ticker settings
//!   GET /pick?key=K    phone page: name the shooter of every basket (only with the secret key of `--pick-key`)

use crate::{tr};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Entries the ticker keeps (a whole match has far fewer events).
const TICKER_KEEP: usize = 600;

/// A sink that writes into a folder (for the example and tests).
pub fn ticker_sink_dir(dir: &str) -> super::ticker_pub::Sink {
    super::ticker_pub::Sink { dir: Some(dir.into()), method: "PUT".into(), ..Default::default() }
}

/// What the ticker page may show. Names are only part of it when the operator allows them
/// (`names` is `names` or `both`); with `numbers` no name ever leaves this program.
#[derive(Clone, Debug)]
pub struct TickerCfg {
    /// `numbers`, `names` or `both`: what the page shows first. Names are sent only for the last two.
    pub names: String,
    pub home_name: String,
    pub away_name: String,
    pub home_players: Vec<(u8, String)>,
    pub away_players: Vec<(u8, String)>,
    pub sport: String,
    /// Manual release: detected events wait in a queue until the operator approves them.
    pub approve: bool,
}

impl TickerCfg {
    pub fn parse_mode(s: &str) -> Option<&'static str> {
        match s.trim().to_ascii_lowercase().as_str() {
            "numbers" | "number" | "nummern" | "n" => Some("numbers"),
            "names" | "name" | "namen" => Some("names"),
            "both" | "beides" | "all" => Some("both"),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        self.json_with(self.names != "numbers")
    }

    /// Everything, names included - for the operator's own page only.
    pub fn to_json_full(&self) -> Value {
        let mut v = self.json_with(true);
        v["names"] = json!("both");
        v
    }

    fn json_with(&self, share: bool) -> Value {
        let map = |v: &Vec<(u8, String)>| -> Value {
            if share { Value::Object(v.iter().map(|(n, s)| (n.to_string(), json!(s))).collect()) } else { json!({}) }
        };
        json!({"names": self.names, "sport": self.sport,
            "teams": {"home": self.home_name, "away": self.away_name},
            "players": {"home": map(&self.home_players), "away": map(&self.away_players)}})
    }
}

pub struct Hub {
    inner: Mutex<(u64, String)>,
    cv: Condvar,
    config: String,
    ticker_cfg: String,
    ticker_full: String,
    ticker: Mutex<Ticker>,
    picks: Mutex<Option<Picks>>,
}

/// The ticker's state: the published feed, the queue waiting for release, and what was taken back.
#[derive(Default)]
struct Ticker {
    seq: u64,
    feed: std::collections::VecDeque<Value>,
    pending: Vec<Value>,
    next_id: u64,
    retracted: Vec<u64>,
    approve: bool,
}

/// Baskets kept on the phone page (open and recently picked).
const PICKS_KEEP: usize = 12;

/// A basket the operator can name the shooter of on the phone (`/pick`).
struct Pick {
    sid: u64,
    event: Value,
    candidates: Vec<u64>,
    chosen: Option<Option<u64>>,
    /// A shot attempt (live replay without a readable scoreboard): made or missed is asked too.
    attempt: bool,
    /// For an attempt: in (true) or missed (false), once chosen.
    made: Option<bool>,
}

/// The phone page: a secret key in its link, the rosters, the baskets, and the picks waiting to be
/// written by the processing loop.
struct Picks {
    key: String,
    roster: [Vec<u64>; 2],
    cards: Vec<Pick>,
    manual: Vec<Value>,
    /// Baskets may be added on the phone (live replay): the time they get and the next id.
    add: Option<(f64, u64)>,
    /// Replay speed shown on the phone (live replay only).
    speed: Option<f64>,
}

/// Compare without stopping at the first difference (the key guards the page in the network).
fn same_key(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Events waiting for release are capped so an unattended queue cannot grow without bound.
const PENDING_KEEP: usize = 200;

impl Ticker {
    fn publish(&mut self, mut e: Value) {
        self.seq += 1;
        e["seq"] = json!(self.seq);
        e.as_object_mut().map(|o| o.remove("id"));
        self.feed.push_back(e);
        if self.feed.len() > TICKER_KEEP {
            self.feed.pop_front();
        }
    }
}

/// May this request use the operator's release page? Only a program on this computer, called from the page
/// itself: the connection comes from loopback, `Host` is a loopback name, a browser's `Origin` is that same
/// host, and changes carry a header that other web pages cannot send without the hub's consent.
pub fn admin_allowed(peer_loopback: bool, host: Option<&str>, origin: Option<&str>, marked: bool, change: bool) -> bool {
    if !peer_loopback || !super::ui::request_allowed(host, origin) {
        return false;
    }
    !change || marked
}

impl Hub {
    pub fn new(config_json: String) -> Arc<Hub> {
        Self::with_ticker(config_json, None)
    }

    pub fn with_ticker(config_json: String, ticker: Option<TickerCfg>) -> Arc<Hub> {
        let approve = ticker.as_ref().map(|t| t.approve).unwrap_or(false);
        let plain = json!({"names": "numbers", "teams": {"home": "", "away": ""}, "players": {"home": {}, "away": {}}});
        let ticker_cfg = ticker.as_ref().map(|t| t.to_json()).unwrap_or_else(|| plain.clone()).to_string();
        let ticker_full = ticker.as_ref().map(|t| t.to_json_full()).unwrap_or(plain).to_string();
        Arc::new(Hub { inner: Mutex::new((0, String::new())), cv: Condvar::new(), config: config_json, ticker_cfg, ticker_full, ticker: Mutex::new(Ticker { approve, ..Default::default() }), picks: Mutex::new(None) })
    }

    /// New events (substitutions, game situations): published at once, or queued for release in manual mode.
    pub fn push_events(&self, events: &[Value]) {
        let mut g = self.ticker.lock().unwrap();
        for e in events {
            if g.approve {
                g.next_id += 1;
                let mut e = e.clone();
                e["id"] = json!(g.next_id);
                g.pending.push(e);
                if g.pending.len() > PENDING_KEEP {
                    g.pending.remove(0);
                }
            } else {
                g.publish(e.clone());
            }
        }
    }

    /// The published feed after `since`, the entries taken back, and the ticker settings.
    pub fn ticker_json(&self, since: u64) -> String {
        self.ticker_json_beat(since, None)
    }

    /// The same with a heartbeat number: a page reading a static copy of the feed sees the beat change
    /// every few seconds and so knows the sender is still alive.
    pub fn ticker_json_beat(&self, since: u64, beat: Option<u64>) -> String {
        let g = self.ticker.lock().unwrap();
        let cfg: Value = serde_json::from_str(&self.ticker_cfg).unwrap_or(Value::Null);
        let mut v = json!({"config": cfg, "seq": g.seq, "retracted": g.retracted, "events": g.feed.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > since).cloned().collect::<Vec<_>>()});
        if let Some(b) = beat {
            v["beat"] = json!(b);
        }
        v.to_string()
    }

    /// What decides whether the public feed changed: published count and entries taken back.
    pub fn ticker_stamp(&self) -> (u64, usize) {
        let g = self.ticker.lock().unwrap();
        (g.seq, g.retracted.len())
    }

    /// The operator's view: the queue, the latest published entries, and the full settings (names included).
    pub fn ticker_admin_json(&self) -> String {
        let g = self.ticker.lock().unwrap();
        let cfg: Value = serde_json::from_str(&self.ticker_full).unwrap_or(Value::Null);
        let recent: Vec<&Value> = g.feed.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        json!({"config": cfg, "approve": g.approve, "pending": g.pending, "published": recent, "retracted": g.retracted, "seq": g.seq}).to_string()
    }

    /// Release one queued event (`true`) or discard it (`false`). Returns whether it was in the queue.
    pub fn decide(&self, id: u64, release: bool) -> bool {
        let mut g = self.ticker.lock().unwrap();
        let Some(i) = g.pending.iter().position(|e| e["id"].as_u64() == Some(id)) else { return false };
        let e = g.pending.remove(i);
        if release {
            g.publish(e);
        }
        true
    }

    pub fn release_all(&self) -> usize {
        let mut g = self.ticker.lock().unwrap();
        let all = std::mem::take(&mut g.pending);
        let n = all.len();
        all.into_iter().for_each(|e| g.publish(e));
        n
    }

    /// Take a published entry back: viewers' pages drop it the next time they poll.
    pub fn retract(&self, seq: u64) -> bool {
        let mut g = self.ticker.lock().unwrap();
        let Some(i) = g.feed.iter().position(|e| e["seq"].as_u64() == Some(seq)) else { return false };
        g.feed.remove(i);
        g.retracted.push(seq);
        let n = g.retracted.len();
        if n > 400 {
            g.retracted.drain(..n - 400);
        }
        true
    }

    /// Switch between manual release and publishing at once.
    pub fn set_approval(&self, on: bool) {
        self.ticker.lock().unwrap().approve = on;
    }

    /// Turn on the phone page for naming shooters. `key` must be long enough to be guessed by nobody.
    pub fn enable_picks(&self, key: &str, roster_home: Vec<u64>, roster_away: Vec<u64>) -> bool {
        if key.len() < 16 || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        *self.picks.lock().unwrap() = Some(Picks { key: key.to_string(), roster: [roster_home, roster_away], cards: Vec::new(), manual: Vec::new(), add: None, speed: None });
        true
    }

    /// A score event of the live scorer: a new basket becomes a card, a later number updates it, a
    /// retraction removes it. `candidates`: jersey numbers to offer first.
    pub fn offer_pick(&self, ev: &Value, mut candidates: Vec<u64>) {
        let mut g = self.picks.lock().unwrap();
        let Some(p) = g.as_mut() else { return };
        let Some(sid) = ev["sid"].as_u64() else { return };
        if ev["retracted"].as_bool() == Some(true) {
            p.cards.retain(|c| c.sid != sid);
            return;
        }
        if !matches!(ev["kind"].as_str(), Some("free_throw" | "field_goal")) || ev["basis"] == "manual" {
            return;
        }
        if let Some(c) = p.cards.iter_mut().find(|c| c.sid == sid) {
            if c.chosen.is_none() {
                c.event["number"] = ev["number"].clone();
            }
            return;
        }
        // a second free throw: most likely the same shooter as the one just before
        if ev["kind"] == "free_throw" {
            let t = ev["t"].as_f64().unwrap_or(0.0);
            if let Some(Some(n)) = p.cards.iter().rev().find(|c| c.event["kind"] == "free_throw" && c.event["team"] == ev["team"] && (t - c.event["t"].as_f64().unwrap_or(0.0)).abs() < 90.0).and_then(|c| c.chosen) {
                candidates.retain(|x| *x != n);
                candidates.insert(0, n);
            }
        }
        p.cards.push(Pick { sid, event: ev.clone(), candidates, chosen: None, attempt: false, made: None });
        if p.cards.len() > PICKS_KEEP {
            p.cards.remove(0);
        }
    }

    /// Live replay: allow adding baskets on the phone and show the replay clock and speed.
    /// `last_sid`: the highest id of baskets added before (new ones count on from there).
    pub fn enable_adding(&self, speed: f64, last_sid: u64) {
        if let Some(p) = self.picks.lock().unwrap().as_mut() {
            p.add = Some((0.0, last_sid.max(9_000_000)));
            p.speed = Some(speed);
        }
    }

    /// The replay's video time (new baskets added on the phone get it).
    pub fn set_clock(&self, t: f64) {
        if let Some((now, _)) = self.picks.lock().unwrap().as_mut().and_then(|p| p.add.as_mut()) {
            *now = t;
        }
    }

    /// A shot attempt as a card: the operator says whether it went in and who threw it.
    /// `ev`: a score-like event (`sid`, `t`, `team`, `points`, `kind: field_goal`).
    pub fn offer_attempt(&self, ev: &Value, candidates: Vec<u64>) {
        let mut g = self.picks.lock().unwrap();
        let Some(p) = g.as_mut() else { return };
        let Some(sid) = ev["sid"].as_u64() else { return };
        if p.cards.iter().any(|c| c.sid == sid) {
            return;
        }
        p.cards.push(Pick { sid, event: ev.clone(), candidates, chosen: None, attempt: true, made: None });
        // open attempts are kept longer: in a replay several come quickly
        while p.cards.len() > PICKS_KEEP * 2 {
            p.cards.remove(0);
        }
    }

    /// A basket added on the phone (live replay): a new card at the replay time, already made.
    pub fn add_basket(&self, team: &str, points: i64) -> Option<u64> {
        let mut g = self.picks.lock().unwrap();
        let p = g.as_mut()?;
        let (t, next) = p.add.as_mut()?;
        if !matches!(team, "home" | "away") || !(1..=3).contains(&points) {
            return None;
        }
        *next += 1;
        let sid = *next;
        let ev = json!({"type": "score", "sid": sid, "t": *t, "team": team, "points": points, "kind": if points == 1 { "free_throw" } else { "field_goal" }, "added": true});
        // counts at once (shooter unknown until named on the card)
        let mut e = ev.clone();
        e["made"] = json!(true);
        e["number"] = Value::Null;
        e["basis"] = json!("manual");
        p.manual.push(e);
        p.cards.push(Pick { sid, event: ev, candidates: Vec::new(), chosen: Some(None), attempt: true, made: Some(true) });
        Some(sid)
    }

    /// An attempt card decided: `made` false = missed (no shooter), else in by `number` with `points`.
    pub fn choose_attempt(&self, sid: u64, made: bool, number: Option<u64>, points: Option<i64>) -> bool {
        let mut g = self.picks.lock().unwrap();
        let Some(p) = g.as_mut() else { return false };
        let Some(c) = p.cards.iter_mut().find(|c| c.sid == sid && c.attempt) else { return false };
        c.chosen = Some(if made { number } else { None });
        c.made = Some(made);
        if let Some(pts) = points.filter(|x| (1..=3).contains(x)) {
            c.event["points"] = json!(pts);
            c.event["kind"] = json!(if pts == 1 { "free_throw" } else { "field_goal" });
        }
        let mut e = c.event.clone();
        e["made"] = json!(made);
        e["number"] = json!(if made { number } else { None });
        e["basis"] = json!("manual");
        p.manual.push(e);
        true
    }

    /// The operator named the shooter (`None`: not known). The update goes out with the next frame.
    pub fn choose(&self, sid: u64, number: Option<u64>) -> bool {
        let mut g = self.picks.lock().unwrap();
        let Some(p) = g.as_mut() else { return false };
        let Some(c) = p.cards.iter_mut().find(|c| c.sid == sid) else { return false };
        c.chosen = Some(number);
        let mut e = c.event.clone();
        e["update"] = json!(true);
        e["pending"] = json!(false);
        e["retracted"] = json!(false);
        e["number"] = json!(number);
        e["read"] = json!(number.is_some());
        e["basis"] = json!("manual");
        e["confidence"] = json!(if number.is_some() { 1.0 } else { 0.0 });
        e["shooter"] = Value::Null;
        e["others"] = json!([]);
        p.manual.push(e);
        true
    }

    /// Take a pick back (undo): the basket is open again and the ticker shows the scorer's own result.
    pub fn unpick(&self, sid: u64) -> bool {
        let mut g = self.picks.lock().unwrap();
        let Some(p) = g.as_mut() else { return false };
        let Some(c) = p.cards.iter_mut().find(|c| c.sid == sid) else { return false };
        if c.chosen.is_none() {
            return false;
        }
        c.chosen = None;
        if c.attempt && c.event["added"] != true {
            c.made = None;
        }
        let mut e = c.event.clone();
        e["update"] = json!(true);
        e["undone"] = json!(true);
        p.manual.push(e);
        true
    }

    /// Picks made since the last call, as score updates (for the result file and the ticker).
    pub fn take_manual(&self) -> Vec<Value> {
        self.picks.lock().unwrap().as_mut().map(|p| std::mem::take(&mut p.manual)).unwrap_or_default()
    }

    fn pick_ok(&self, key: &str) -> bool {
        self.picks.lock().unwrap().as_ref().is_some_and(|p| same_key(&p.key, key))
    }

    /// The phone page's view: settings with names, rosters and the baskets (newest first).
    pub fn pick_json(&self) -> String {
        let g = self.picks.lock().unwrap();
        let Some(p) = g.as_ref() else { return "{}".into() };
        let cfg: Value = serde_json::from_str(&self.ticker_full).unwrap_or(Value::Null);
        let cards: Vec<Value> = p.cards.iter().rev().map(|c| {
            let e = &c.event;
            json!({"sid": c.sid, "t": e["t"], "clock": e["clock"], "team": e["team"], "points": e["points"], "kind": e["kind"], "to": e["to"],
                "auto": e["number"], "candidates": c.candidates, "picked": c.chosen.is_some(), "number": c.chosen.flatten(),
                "attempt": c.attempt, "made": c.made, "added": e["added"] == true})
        }).collect();
        let replay = p.add.map(|(t, _)| json!({"t": t, "speed": p.speed}));
        json!({"config": cfg, "roster": {"home": p.roster[0], "away": p.roster[1]}, "cards": cards, "replay": replay}).to_string()
    }

    pub fn publish(&self, json: String) {
        let mut g = self.inner.lock().unwrap();
        g.0 += 1;
        g.1 = json;
        self.cv.notify_all();
    }

    pub fn latest(&self) -> String {
        self.inner.lock().unwrap().1.clone()
    }
}

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, hub: Arc<Hub>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 4096];
    let n = match s.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let peer_loopback = s.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false);
    let hdr = |name: &str| req.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    let method = req.split_whitespace().next().unwrap_or("GET").to_string();
    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
    match path.as_str() {
        "/ticker" | "/ticker/" => respond(&mut s, "200 OK", "text/html; charset=utf-8", super::ticker_pub::page_for_hub().as_bytes()),
        "/ticker.js" => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", super::ticker_pub::script().as_bytes()),
        p if p.starts_with("/ticker/admin") => {
            let change = method == "POST";
            if !admin_allowed(peer_loopback, hdr("host").as_deref(), hdr("origin").as_deref(), hdr("x-ticker-admin").is_some(), change) {
                return respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
            }
            let arg = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).and_then(|v| v.parse::<u64>().ok());
            let ok = |s: &mut TcpStream, v: bool| respond(s, "200 OK", "application/json", json!({"ok": v}).to_string().as_bytes());
            match (method.as_str(), p) {
                ("GET", "/ticker/admin") | ("GET", "/ticker/admin/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", super::ticker_pub::page_for_hub().as_bytes()),
                ("GET", "/ticker/admin.json") => respond(&mut s, "200 OK", "application/json; charset=utf-8", hub.ticker_admin_json().as_bytes()),
                ("POST", "/ticker/admin/release") => { let v = arg("id").map(|i| hub.decide(i, true)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/discard") => { let v = arg("id").map(|i| hub.decide(i, false)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/release_all") => { hub.release_all(); ok(&mut s, true) }
                ("POST", "/ticker/admin/retract") => { let v = arg("seq").map(|q| hub.retract(q)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/mode") => { hub.set_approval(arg("approve") == Some(1)); ok(&mut s, true) }
                _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
            }
        }
        p if p == "/pick" || p == "/pick/" || p.starts_with("/pick.") || p.starts_with("/pick/") => {
            let q = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(str::to_string);
            if !hub.pick_ok(&q("key").unwrap_or_default()) {
                return respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
            }
            match (method.as_str(), p) {
                ("GET", "/pick") | ("GET", "/pick/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("pick.html").as_bytes()),
                ("GET", "/pick.json") => respond(&mut s, "200 OK", "application/json; charset=utf-8", hub.pick_json().as_bytes()),
                ("POST", "/pick/add") => {
                    let pts = q("points").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                    let r = hub.add_basket(&q("team").unwrap_or_default(), pts);
                    respond(&mut s, if r.is_some() { "200 OK" } else { "400 Bad Request" }, "application/json", json!({"ok": r.is_some(), "sid": r}).to_string().as_bytes());
                }
                ("POST", "/pick/attempt") => {
                    let sid = q("sid").and_then(|v| v.parse::<u64>().ok());
                    let made = q("made").as_deref() == Some("1");
                    let n = q("n").and_then(|v| v.parse::<u64>().ok());
                    let pts = q("p").and_then(|v| v.parse::<i64>().ok());
                    let ok = match (sid, q("n").as_deref()) {
                        (Some(sid), Some("auto")) => hub.unpick(sid),
                        (Some(sid), _) => hub.choose_attempt(sid, made, n, pts),
                        _ => false,
                    };
                    respond(&mut s, "200 OK", "application/json", json!({"ok": ok}).to_string().as_bytes());
                }
                ("POST", "/pick/choose") => {
                    let sid = q("sid").and_then(|v| v.parse::<u64>().ok());
                    let n = q("n").and_then(|v| v.parse::<u64>().ok());
                    // `n=auto`: undo the pick
                    let v = sid.map(|sid| if q("n").as_deref() == Some("auto") { hub.unpick(sid) } else { hub.choose(sid, n) }).unwrap_or(false);
                    respond(&mut s, "200 OK", "application/json", json!({"ok": v}).to_string().as_bytes())
                }
                _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
            }
        }
        "/ticker.json" => {
            let since = query.split('&').find_map(|kv| kv.strip_prefix("since=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            respond(&mut s, "200 OK", "application/json; charset=utf-8", hub.ticker_json(since).as_bytes());
        }
        "/" => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("live.html").as_bytes()),
        "/scoreboard" => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("scoreboard_overlay.html").as_bytes()),
        "/i18n.js" => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", include_str!("i18n.js").as_bytes()),
        "/config.json" => respond(&mut s, "200 OK", "application/json", hub.config.as_bytes()),
        "/state.json" => {
            let l = hub.latest();
            respond(&mut s, "200 OK", "application/json", if l.is_empty() { b"{}" } else { l.as_bytes() });
        }
        "/events" => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\n";
            if s.write_all(head.as_bytes()).is_err() {
                return;
            }
            let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
            let mut seen = 0u64;
            loop {
                let (ver, msg) = {
                    let mut g = hub.inner.lock().unwrap();
                    if g.0 == seen {
                        let (g2, _) = hub.cv.wait_timeout(g, Duration::from_secs(10)).unwrap();
                        g = g2;
                    }
                    (g.0, g.1.clone())
                };
                let out = if ver != seen && !msg.is_empty() { format!("data: {msg}\n\n") } else { ": ping\n\n".to_string() };
                seen = ver;
                if s.write_all(out.as_bytes()).is_err() || s.flush().is_err() {
                    return;
                }
            }
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Start serving in background threads; returns the bound address.
/// The address to show for links: a server on all interfaces (`0.0.0.0`) is shown with this
/// computer's address in the local network, so the link works on a phone as printed.
pub fn shown_addr(addr: &str) -> String {
    match addr.strip_prefix("0.0.0.0:") {
        Some(port) => format!("{}:{port}", super::board_srv::lan_address().unwrap_or_else(|| "127.0.0.1".into())),
        None => addr.to_string(),
    }
}

/// Whether a server on `port` answers on this computer (`127.0.0.1`) and on its address in the
/// local network - the address a phone uses. (Reaching it from here does not prove that the
/// network lets a phone through: guest networks and client isolation are only seen from the phone.)
pub fn reach(port: u16) -> Value {
    let try_ = |ip: &str| ip.parse::<std::net::IpAddr>().ok().is_some_and(|ip| TcpStream::connect_timeout(&std::net::SocketAddr::new(ip, port), Duration::from_millis(600)).is_ok());
    let lan = super::board_srv::lan_address();
    json!({"local": try_("127.0.0.1"), "lan": lan.as_deref().is_some_and(try_), "ip": lan, "port": port})
}

pub fn serve(hub: Arc<Hub>, host: &str, port: u16) -> Result<String, String> {
    let listener = TcpListener::bind((host, port)).map_err(|e| tr!("{host}:{port}: {e} (Port belegt? --port ändern)", "{host}:{port}: {e} (port in use? change --port)", "{host}:{port} : {e} (port occupé ? changer --port)", "{host}:{port}: {e} (¿puerto ocupado? cambia --port)"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?.to_string();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let h = hub.clone();
            std::thread::spawn(move || handle(s, h));
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod pick_tests {
    use super::*;

    #[test]
    fn attempts_are_decided_and_baskets_added_in_a_replay() {
        let hub = Hub::new("{}".into());
        assert!(hub.enable_picks("abcdefghijklmnop1234", vec![], vec![]));
        hub.enable_adding(4.0, 9_000_003);
        hub.set_clock(50.0);
        hub.offer_attempt(&json!({"type": "score", "sid": 1_000_000, "t": 40.0, "team": "home", "points": 2, "kind": "field_goal"}), vec![8, 12]);
        assert!(hub.choose_attempt(1_000_000, true, Some(8), Some(3)));
        let m = hub.take_manual();
        assert_eq!((m[0]["made"].as_bool(), m[0]["number"].as_u64(), m[0]["points"].as_i64()), (Some(true), Some(8), Some(3)));
        // an added basket counts at once, at the replay clock, with a new id
        assert_eq!(hub.add_basket("away", 1), Some(9_000_004));
        let m = hub.take_manual();
        assert_eq!((m[0]["t"].as_f64(), m[0]["made"].as_bool(), m[0]["team"].as_str()), (Some(50.0), Some(true), Some("away")));
        // undone: the card is open again and the decision is withdrawn
        assert!(hub.unpick(1_000_000));
        assert_eq!(hub.take_manual()[0]["undone"], true);
        let v: Value = serde_json::from_str(&hub.pick_json()).unwrap();
        assert_eq!(v["replay"]["speed"], 4.0);
        let card = v["cards"].as_array().unwrap().iter().find(|c| c["sid"] == 1_000_000).unwrap();
        assert_eq!((card["picked"].as_bool(), card["made"].is_null()), (Some(false), true));
    }

    fn get(addr: &str, method: &str, path: &str) -> (String, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        // one write, like a browser: the hub reads a request in one go
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
        (head.lines().next().unwrap_or("").to_string(), body.to_string())
    }

    fn score(sid: u64, kind: &str, number: Option<u64>) -> Value {
        json!({"type": "score", "sid": sid, "t": 10.0 * sid as f64, "team": "away", "points": if kind == "free_throw" { 1 } else { 2 }, "from": 40, "to": 42, "kind": kind, "basis": "near_basket", "number": number, "update": false, "retracted": false, "pending": number.is_none(), "confidence": 0.4, "clock": 300, "shooter": {"track": 7, "segment": 1}, "others": []})
    }

    #[test]
    fn the_shooter_is_named_on_the_phone() {
        let hub = Hub::with_ticker("{}".into(), None);
        assert!(!hub.enable_picks("short", vec![], vec![]));
        assert!(hub.enable_picks("abcdefghijklmnop1234", vec![4, 6], vec![3, 8, 21]));
        let addr = serve(hub.clone(), "127.0.0.1", 0).unwrap();
        // without the key nothing is shown or changed
        let r = get(&addr, "GET", "/pick?key=wrongwrongwrongwrong");
        assert!(r.0.contains("403"), "{r:?}");
        assert!(get(&addr, "POST", "/pick/choose?key=abcdefghijklmnop1234x&sid=1&n=8").0.contains("403"));
        hub.offer_pick(&score(1, "field_goal", Some(3)), vec![3, 21]);
        let (st, body) = get(&addr, "GET", "/pick.json?key=abcdefghijklmnop1234");
        assert!(st.contains("200"), "{st}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["cards"][0]["candidates"], json!([3, 21]));
        assert_eq!(v["roster"]["away"], json!([3, 8, 21]));
        assert!(get(&addr, "GET", "/pick?key=abcdefghijklmnop1234").1.contains("pick.title"));
        // a tap: an update of the same score with the number, picked up by the processing loop
        assert!(get(&addr, "POST", "/pick/choose?key=abcdefghijklmnop1234&sid=1&n=8").1.contains("true"));
        let m = hub.take_manual();
        assert_eq!(m.len(), 1);
        assert_eq!((m[0]["sid"].as_u64(), m[0]["number"].as_u64(), m[0]["basis"].as_str(), m[0]["update"].as_bool()), (Some(1), Some(8), Some("manual"), Some(true)));
        assert!(hub.take_manual().is_empty());
        // "unknown" works too, and the live score parser reads the pick back
        assert!(hub.choose(1, None));
        let e = crate::live_score::parse_event(&hub.take_manual()[0]).unwrap();
        assert_eq!((e.number, e.basis), (None, crate::live_score::Basis::Manual));
        // the second free throw offers the shooter of the first one first
        hub.offer_pick(&score(2, "free_throw", None), vec![]);
        hub.choose(2, Some(21));
        hub.offer_pick(&score(3, "free_throw", None), vec![3]);
        let v: Value = serde_json::from_str(&hub.pick_json()).unwrap();
        assert_eq!(v["cards"][0]["candidates"], json!([21, 3]));
        // undo: open again, the scorer's own result goes out as an update
        assert!(hub.unpick(2));
        let m = hub.take_manual();
        assert_eq!(m[0]["basis"], "manual", "the pick and its undo arrive in order");
        assert_eq!((m[1]["sid"].as_u64(), m[1]["basis"].as_str(), m[1]["update"].as_bool()), (Some(2), Some("near_basket"), Some(true)));
        assert!(!hub.unpick(2));
        // a score taken back disappears
        hub.offer_pick(&json!({"sid": 3, "retracted": true}), vec![]);
        let v: Value = serde_json::from_str(&hub.pick_json()).unwrap();
        assert!(v["cards"].as_array().unwrap().iter().all(|c| c["sid"] != 3));
    }
}
