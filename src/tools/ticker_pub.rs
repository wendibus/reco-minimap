//! Taking the ticker to a web site: the widget script, static files for any web space, and the sender
//! that keeps `ticker.json` up to date - in a folder (`--ticker-export`) and/or pushed with HTTP PUT
//! (`--ticker-push`) to WebDAV / Nextcloud, S3 / R2, a Cloudflare Worker or anything that accepts a file.
//!
//! Nothing here needs code on the receiving side: the web site only serves static files, and the widget
//! (plain JavaScript in a shadow root) reads `ticker.json`. Only the released, name-filtered feed leaves
//! this computer; the hub, positions and the release page stay local.

use crate::{tr, trs};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::live::Hub;

const STRINGS: &str = include_str!("ticker_strings.js");
const CORE: &str = include_str!("ticker.js");
const PAGE: &str = include_str!("ticker.html");
const MARK: &str = "<!--TICKER-SCRIPT-->";
/// How often the sender writes even when nothing happened, so viewers can tell it is alive.
const BEAT: Duration = Duration::from_secs(10);

/// The widget: texts + code, wrapped so it adds nothing to the web page it is put on except `<reco-ticker>`
/// and `window.RecoTicker`.
///
/// Pure ASCII (non-ASCII characters are written as `\uXXXX`), so it displays correctly whatever character set
/// a web server announces for `.js` files.
pub fn script() -> String {
    let src = format!("(function(){{\n{STRINGS}{CORE}\n}})();\n");
    let mut out = String::with_capacity(src.len());
    for c in src.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut b = [0u16; 2];
            for u in c.encode_utf16(&mut b) {
                out.push_str(&format!("\\u{:04x}", u));
            }
        }
    }
    out
}

/// The ticker page served by the hub itself (`/ticker`, `/ticker/admin`).
pub fn page_for_hub() -> String {
    PAGE.replace(MARK, "<script src=\"/ticker.js\"></script>")
}

/// The ticker page for a web space: reads `ticker.json` next to it.
pub fn page_static() -> String {
    PAGE.replace("<head>", "<head>\n<meta name=\"ticker-static\" content=\"1\">").replace(MARK, "<script src=\"ticker.js\"></script>")
}

/// A page with the snippets to copy, and that doubles as a test page for the files next to it.
fn index_html() -> String {
    r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Live ticker – embed</title>
<style>body{font:16px/1.5 system-ui,sans-serif;max-width:760px;margin:24px auto;padding:0 16px;color:#1a2229}code,pre{background:#eef1f4;border-radius:6px}pre{padding:12px;overflow:auto}code{padding:1px 5px}h2{margin-top:32px}</style></head>
<body>
<h1>Live ticker</h1>
<p>These files go on your web space as they are (keep them in one folder). The sender updates <code>ticker.json</code>; everything else is static. Replace <code>https://example.org/ticker/</code> with the address of this folder.</p>
<h2>1. Anywhere: an iframe</h2>
<pre>&lt;iframe src="https://example.org/ticker/ticker.html?embed=1&amp;lang=de" style="width:100%;height:480px;border:0" loading="lazy"&gt;&lt;/iframe&gt;</pre>
<p>Options in the address: <code>lang=de|en|fr|es</code>, <code>show=numbers|names|both</code>, <code>theme=dark|light</code>, <code>max=20</code>. Without <code>embed=1</code> you get the full page with header and language switch.
To make the frame as high as its content, add on your page:</p>
<pre>&lt;script&gt;addEventListener('message',e=&gt;{if(e.data&amp;&amp;e.data.type==='reco-ticker-height'){document.querySelector('iframe[src*="ticker.html"]').style.height=e.data.height+'px'}})&lt;/script&gt;</pre>
<h2>2. On the same site: the widget</h2>
<pre>&lt;script src="https://example.org/ticker/ticker.js" data-feed="https://example.org/ticker/ticker.json" data-lang="de" data-max="20"&gt;&lt;/script&gt;</pre>
<p>or as an element: <code>&lt;reco-ticker feed="…/ticker.json" lang="de" max="20"&gt;&lt;/reco-ticker&gt;</code>. It takes the font of your page and has its own isolated style; change colours with CSS variables on the element:
<code>reco-ticker{--rt-home:#c00;--rt-away:#06c;--rt-acc:#c00;--rt-card:#fff;--rt-fg:#222;--rt-line:#ddd;--rt-dim:#666}</code>.
If the files are on another domain than your page, that domain must allow reading <code>ticker.json</code> from yours (CORS); the supplied <code>.htaccess</code> does it for Apache. The iframe needs no such setting.</p>
<h2>Test here</h2>
<reco-ticker feed="ticker.json" max="8"></reco-ticker>
<script src="ticker.js"></script>
</body></html>
"##.to_string()
}

const HTACCESS: &str = "# Apache: let other web sites read the feed, and never serve it from a cache\n<IfModule mod_headers.c>\n<FilesMatch \"\\.json$\">\nHeader set Access-Control-Allow-Origin \"*\"\nHeader set Cache-Control \"no-store\"\n</FilesMatch>\n</IfModule>\n";

/// Write the static files (page, widget, snippets page) into `dir`. `ticker.json` is written by the sender.
pub fn write_assets(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, text) in [("ticker.html", page_static()), ("ticker.js", script()), ("index.html", index_html()), (".htaccess", HTACCESS.to_string())] {
        std::fs::write(dir.join(name), text).map_err(|e| format!("{}: {e}", dir.join(name).display()))?;
    }
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Where the feed goes besides the hub's own `/ticker.json`.
#[derive(Clone, Debug, Default)]
pub struct Sink {
    /// Folder for `ticker.json` and the static files.
    pub dir: Option<PathBuf>,
    /// `http(s)://…` that accepts the file with PUT (or POST).
    pub push: Option<String>,
    pub method: String,
    /// Sent as `Authorization: Bearer <key>`.
    pub key: Option<String>,
}

/// The address without a password, for messages.
pub fn mask_url(u: &str) -> String {
    match (u.find("://"), u.find('@')) {
        (Some(i), Some(a)) if a > i + 3 && !u[i + 3..a].contains('/') => format!("{}://***@{}", &u[..i], &u[a + 1..]),
        _ => u.to_string(),
    }
}

fn push_once(sink: &Sink, body: &str) -> Result<(), String> {
    let url = sink.push.as_deref().unwrap_or("");
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--fail", "--max-time", "20", "-X", if sink.method.is_empty() { "PUT" } else { &sink.method }, "-H", "Content-Type: application/json", "--data-binary", "@-"]);
    if let Some(k) = sink.key.as_deref().filter(|k| !k.is_empty()) {
        cmd.args(["-H", &format!("Authorization: Bearer {k}")]);
    }
    cmd.arg(url).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("curl: {e}"))?;
    child.stdin.take().map(|mut s| s.write_all(body.as_bytes()));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().replace(url, &mask_url(url)))
    }
}

/// Check what the sender needs before the match starts (curl for pushing, a writable folder).
pub fn check(sink: &Sink) -> Result<(), String> {
    if let Some(d) = &sink.dir {
        write_assets(d)?;
    }
    if let Some(u) = &sink.push {
        if !(u.starts_with("http://") || u.starts_with("https://")) {
            return Err(tr!("--ticker-push: die Adresse muss mit http:// oder https:// beginnen", "--ticker-push: the address must start with http:// or https://", "--ticker-push : l'adresse doit commencer par http:// ou https://", "--ticker-push: la dirección debe empezar por http:// o https://"));
        }
        if Command::new("curl").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| !s.success()).unwrap_or(true) {
            return Err(trs!("--ticker-push braucht das Programm curl (auf macOS, Linux und Windows 10/11 vorhanden)", "--ticker-push needs the curl program (present on macOS, Linux and Windows 10/11)", "--ticker-push nécessite le programme curl (présent sur macOS, Linux et Windows 10/11)", "--ticker-push necesita el programa curl (presente en macOS, Linux y Windows 10/11)").to_string());
        }
    }
    Ok(())
}

/// Keep the copies of the public feed current: whenever something is released or taken back, and every
/// few seconds as a heartbeat. Runs until the program ends.
pub fn spawn(hub: Arc<Hub>, sink: Sink) {
    std::thread::spawn(move || {
        let (mut last, mut beat, mut at, mut failing) = ((u64::MAX, 0usize), 0u64, Instant::now() - BEAT, false);
        loop {
            let stamp = hub.ticker_stamp();
            if stamp != last || at.elapsed() >= BEAT {
                beat += 1;
                let body = hub.ticker_json_beat(0, Some(beat));
                if let Some(d) = &sink.dir {
                    if let Err(e) = write_atomic(&d.join("ticker.json"), &body) {
                        eprintln!("ticker.json: {e}");
                    }
                }
                if sink.push.is_some() {
                    match push_once(&sink, &body) {
                        Ok(()) if failing => {
                            failing = false;
                            eprintln!("\n{}", trs!("  Ticker-Upload funktioniert wieder.", "  Ticker upload works again.", "  L'envoi du fil fonctionne de nouveau.", "  La subida del resumen vuelve a funcionar."));
                        }
                        Ok(()) => {}
                        Err(e) if !failing => {
                            failing = true;
                            eprintln!("\n{}", tr!("  ! Ticker-Upload fehlgeschlagen ({e}) – es wird weiter versucht.", "  ! Ticker upload failed ({e}) – retrying.", "  ! Échec de l'envoi du fil ({e}) – nouvelle tentative.", "  ! Falló la subida del resumen ({e}); se reintenta."));
                        }
                        Err(_) => {}
                    }
                }
                last = stamp;
                at = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    });
}
