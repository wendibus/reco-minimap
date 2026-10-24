//! Live hub: serves the overlay page and the tracking state to OBS (Browser Source),
//! Spieldock or any other consumer. std-only HTTP server.
//!
//!   GET /              overlay page (transparent canvas; add ?floor=1 to test in a normal browser)
//!   GET /events        server-sent events: one JSON message per detection update
//!   GET /state.json    latest message (polling)
//!   GET /config.json   colours
//!   GET /ticker        live ticker page (substitutions and game situations as a feed; works on a phone)
//!   GET /ticker.json   the feed so far (`?since=N` for only the newer entries) and the ticker settings

use crate::{tr};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Entries the ticker keeps (a whole match has far fewer events).
const TICKER_KEEP: usize = 600;

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
        Arc::new(Hub { inner: Mutex::new((0, String::new())), cv: Condvar::new(), config: config_json, ticker_cfg, ticker_full, ticker: Mutex::new(Ticker { approve, ..Default::default() }) })
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
        let g = self.ticker.lock().unwrap();
        let cfg: Value = serde_json::from_str(&self.ticker_cfg).unwrap_or(Value::Null);
        json!({"config": cfg, "seq": g.seq, "retracted": g.retracted, "events": g.feed.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > since).cloned().collect::<Vec<_>>()}).to_string()
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
        "/ticker" | "/ticker/" => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("ticker.html").as_bytes()),
        p if p.starts_with("/ticker/admin") => {
            let change = method == "POST";
            if !admin_allowed(peer_loopback, hdr("host").as_deref(), hdr("origin").as_deref(), hdr("x-ticker-admin").is_some(), change) {
                return respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
            }
            let arg = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).and_then(|v| v.parse::<u64>().ok());
            let ok = |s: &mut TcpStream, v: bool| respond(s, "200 OK", "application/json", json!({"ok": v}).to_string().as_bytes());
            match (method.as_str(), p) {
                ("GET", "/ticker/admin") | ("GET", "/ticker/admin/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("ticker.html").as_bytes()),
                ("GET", "/ticker/admin.json") => respond(&mut s, "200 OK", "application/json; charset=utf-8", hub.ticker_admin_json().as_bytes()),
                ("POST", "/ticker/admin/release") => { let v = arg("id").map(|i| hub.decide(i, true)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/discard") => { let v = arg("id").map(|i| hub.decide(i, false)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/release_all") => { hub.release_all(); ok(&mut s, true) }
                ("POST", "/ticker/admin/retract") => { let v = arg("seq").map(|q| hub.retract(q)).unwrap_or(false); ok(&mut s, v) }
                ("POST", "/ticker/admin/mode") => { hub.set_approval(arg("approve") == Some(1)); ok(&mut s, true) }
                _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
            }
        }
        "/ticker.json" => {
            let since = query.split('&').find_map(|kv| kv.strip_prefix("since=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            respond(&mut s, "200 OK", "application/json; charset=utf-8", hub.ticker_json(since).as_bytes());
        }
        "/" => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("live.html").as_bytes()),
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
