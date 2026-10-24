//! The tactics board: a small local web page (std-only HTTP server) where a coach moves players,
//! draws arrows and steps through a "what if" line. The scene file stays on this machine.
//!
//! Only this machine may talk to it (loopback `Host`, same-origin `Origin`). With `lan` it listens on
//! the local network instead - for a tablet on the same Wi-Fi - and then every request needs the
//! secret key that is part of the printed link.

use crate::{tr, trs};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::ui::{header, qparam, read_request, request_allowed, respond};
use crate::board::{to_svg, FORMAT, VERSION};

const PAGE: &str = include_str!("board.html");
const I18N_JS: &str = include_str!("i18n.js");

pub struct Options {
    pub scene: PathBuf,
    pub port: u16,
    pub lan: bool,
    pub open: bool,
    pub lang: String,
}

/// A random key for LAN mode (std's hasher is seeded by the operating system).
fn make_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut k = String::new();
    for i in 0..3u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::process::id() as u64);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        k.push_str(&format!("{:016x}", h.finish()));
    }
    k
}

/// The address other devices on this network would use (found by asking the OS which interface it
/// would use for an outside route; nothing is sent).
pub(crate) fn lan_address() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

/// Check that a scene file is one this program wrote (or could read) before it replaces the file.
pub fn validate_scene(text: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if v["format"].as_str() != Some(FORMAT) {
        return Err(trs!("Das ist keine Board-Szene (format fehlt)", "This is not a board scene (format missing)", "Ce n'est pas une scène de tableau (format manquant)", "No es una escena de pizarra (falta format)").to_string());
    }
    if v["version"].as_u64().map(|n| n as u32).unwrap_or(0) > VERSION {
        return Err(trs!("Die Szene ist neuer als dieses Programm - bitte aktualisieren (reco-minimap update)", "The scene is newer than this program - please update (reco-minimap update)", "La scène est plus récente que ce programme - veuillez mettre à jour (reco-minimap update)", "La escena es más nueva que este programa: actualiza (reco-minimap update)").to_string());
    }
    super::board_io::scene_from_json(&v)?;
    Ok(v)
}

/// Write `text` to `path` through a temporary file, so a crash never leaves half a scene behind.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The standalone page: the board with its language table inlined (opens from a file, no server).
pub fn standalone_html() -> String {
    PAGE.replace("<script src=\"/i18n.js\"></script>", &format!("<script>{I18N_JS}</script>"))
}

fn handle(mut s: TcpStream, o: &Options, key: Option<&str>) {
    let Some((method, path, query, body, head)) = read_request(&mut s) else { return };
    if let Some(k) = key {
        // LAN mode: the key travels in the link (?k=...) and in a header for the page's own calls
        let ok = qparam(&query, "k").as_deref() == Some(k) || header(&head, "x-board-key") == Some(k);
        if !ok {
            return respond(&mut s, "403 Forbidden", "text/plain", b"key");
        }
    } else if !request_allowed(header(&head, "host"), header(&head, "origin")) {
        return respond(&mut s, "403 Forbidden", "text/plain", b"forbidden");
    }
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => respond(&mut s, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        ("GET", "/i18n.js") => respond(&mut s, "200 OK", "application/javascript; charset=utf-8", I18N_JS.as_bytes()),
        ("GET", "/scene.json") => match std::fs::read(&o.scene) {
            Ok(b) => respond(&mut s, "200 OK", "application/json; charset=utf-8", &b),
            Err(_) => respond(&mut s, "404 Not Found", "text/plain", b"no scene"),
        },
        ("POST", "/scene.json") => {
            if body.len() > 8 << 20 {
                return respond(&mut s, "413 Payload Too Large", "text/plain", b"too large");
            }
            let msg = match validate_scene(&body) {
                Err(e) => return respond(&mut s, "400 Bad Request", "text/plain; charset=utf-8", e.as_bytes()),
                Ok(v) => v,
            };
            if let Err(e) = write_atomic(&o.scene, &body) {
                return respond(&mut s, "500 Internal Server Error", "text/plain; charset=utf-8", format!("{}: {e}", o.scene.display()).as_bytes());
            }
            // keep the picture next to the scene up to date (first step; the page exports the others)
            if let Ok(sc) = super::board_io::scene_step(&msg, 1) {
                let _ = std::fs::write(o.scene.with_extension("svg"), to_svg(&sc, &Default::default()));
            }
            respond(&mut s, "200 OK", "application/json", b"{\"ok\":true}");
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Serve the board until the process is stopped.
pub fn run(o: &Options) -> Result<(), String> {
    let addr = if o.lan { "0.0.0.0" } else { "127.0.0.1" };
    let listener = TcpListener::bind((addr, o.port)).map_err(|e| format!("{addr}:{}: {e}", o.port))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let key = if o.lan { Some(make_key()) } else { None };
    let lang = &o.lang;
    let url = match &key {
        None => format!("http://127.0.0.1:{port}/?lang={lang}"),
        Some(k) => format!("http://{}:{port}/?lang={lang}&k={k}", lan_address().unwrap_or_else(|| "<this-computer>".into())),
    };
    eprintln!("{}", tr!("Taktikboard: {url}\nSzene: {}\n(Strg+C beendet das Programm.)", "Tactics board: {url}\nScene: {}\n(Ctrl+C stops the program.)", "Tableau tactique : {url}\nScène : {}\n(Ctrl+C arrête le programme.)", "Pizarra táctica: {url}\nEscena: {}\n(Ctrl+C detiene el programa.)", o.scene.display()));
    if o.lan {
        eprintln!("{}", trs!(
            "Netzwerkmodus: Jeder im selben WLAN, der diesen Link hat, kann die Szene ändern. Nur in vertrauten Netzen benutzen.",
            "Network mode: anyone on the same Wi-Fi who has this link can change the scene. Use only on networks you trust.",
            "Mode réseau : toute personne sur le même Wi-Fi ayant ce lien peut modifier la scène. À utiliser uniquement sur des réseaux de confiance.",
            "Modo de red: cualquiera en la misma Wi-Fi que tenga este enlace puede cambiar la escena. Úsalo solo en redes de confianza."));
    } else if o.open {
        super::calib::open_browser(&url);
    }
    for conn in listener.incoming() {
        let Ok(s) = conn else { continue };
        let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
        handle(s, o, key.as_deref());
    }
    Ok(())
}
