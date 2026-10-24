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
        let share = self.names != "numbers";
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
    ticker: Mutex<(u64, std::collections::VecDeque<Value>)>,
}

impl Hub {
    pub fn new(config_json: String) -> Arc<Hub> {
        Self::with_ticker(config_json, None)
    }

    pub fn with_ticker(config_json: String, ticker: Option<TickerCfg>) -> Arc<Hub> {
        let ticker_cfg = ticker.map(|t| t.to_json()).unwrap_or_else(|| json!({"names": "numbers", "teams": {"home": "", "away": ""}, "players": {"home": {}, "away": {}}})).to_string();
        Arc::new(Hub { inner: Mutex::new((0, String::new())), cv: Condvar::new(), config: config_json, ticker_cfg, ticker: Mutex::new((0, Default::default())) })
    }

    /// Add events (substitutions, game situations) to the ticker feed; each gets a running `seq`.
    pub fn push_events(&self, events: &[Value]) {
        if events.is_empty() {
            return;
        }
        let mut g = self.ticker.lock().unwrap();
        for e in events {
            g.0 += 1;
            let mut e = e.clone();
            e["seq"] = json!(g.0);
            g.1.push_back(e);
            if g.1.len() > TICKER_KEEP {
                g.1.pop_front();
            }
        }
    }

    /// The ticker feed after `since` as JSON, with the ticker settings.
    pub fn ticker_json(&self, since: u64) -> String {
        let g = self.ticker.lock().unwrap();
        let cfg: Value = serde_json::from_str(&self.ticker_cfg).unwrap_or(Value::Null);
        json!({"config": cfg, "seq": g.0, "events": g.1.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > since).cloned().collect::<Vec<_>>()}).to_string()
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
    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
    match path.as_str() {
        "/ticker" | "/ticker/" => respond(&mut s, "200 OK", "text/html; charset=utf-8", include_str!("ticker.html").as_bytes()),
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
