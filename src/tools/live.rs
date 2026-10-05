//! Live hub: serves the overlay page and the tracking state to OBS (Browser Source),
//! Spieldock or any other consumer. std-only HTTP server.
//!
//!   GET /              overlay page (transparent canvas; add ?floor=1 to test in a normal browser)
//!   GET /events        server-sent events: one JSON message per detection update
//!   GET /state.json    latest message (polling)
//!   GET /config.json   colours

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub struct Hub {
    inner: Mutex<(u64, String)>,
    cv: Condvar,
    config: String,
}

impl Hub {
    pub fn new(config_json: String) -> Arc<Hub> {
        Arc::new(Hub { inner: Mutex::new((0, String::new())), cv: Condvar::new(), config: config_json })
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
    let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
    match path.as_str() {
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
    let listener = TcpListener::bind((host, port)).map_err(|e| format!("{host}:{port}: {e} (Port belegt? --port ändern)"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?.to_string();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let h = hub.clone();
            std::thread::spawn(move || handle(s, h));
        }
    });
    Ok(addr)
}
