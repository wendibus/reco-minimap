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

/// Is this an FTP-family address (`ftp://`, `ftps://`, `sftp://`)? There the file is overwritten in place
/// on the web space, the way any FTP client would - no program has to run on the server.
pub fn is_file_transfer(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.starts_with("ftp://") || u.starts_with("ftps://") || u.starts_with("sftp://")
}

/// Split `scheme://[user:pass@]host[:port]/path/name` into (address up to the path, directory path, file name).
/// `sftp://host/~/dir/f` keeps its `~` (home-relative); the directory is returned as curl's rename command needs it.
fn split_target(url: &str) -> Option<(String, String, String)> {
    let i = url.find("://")? + 3;
    let slash = url[i..].find('/')? + i;
    let (head, path) = url.split_at(slash);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let (dir, name) = path.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    Some((head.to_string(), dir.to_string(), name.to_string()))
}

/// The curl arguments that upload `body` to a file on an FTP/SFTP server: written under a temporary name first and
/// renamed afterwards, so a visitor never reads a half-written `ticker.json`.
fn transfer_args(url: &str) -> Option<Vec<String>> {
    let (head, dir, name) = split_target(url)?;
    let tmp = format!("{name}.part");
    let is_sftp = url.to_ascii_lowercase().starts_with("sftp://");
    let mut a: Vec<String> = vec!["--ftp-create-dirs".into(), "-T".into(), "-".into(), format!("{head}{dir}/{tmp}")];
    a.push("-Q".into());
    if is_sftp {
        let base = match dir.strip_prefix("/~") {
            Some(rest) => rest.trim_start_matches('/').to_string(), // home-relative
            None if dir.is_empty() => "/".to_string(), // the server's root
            None => dir.clone(),
        };
        let join = |f: &str| if base.is_empty() { f.to_string() } else if base == "/" { format!("/{f}") } else { format!("{}/{f}", base.trim_end_matches('/')) };
        a.push(format!("-rename {} {}", join(&tmp), join(&name)));
    } else {
        a.push(format!("-RNFR {tmp}"));
        a.push("-Q".into());
        a.push(format!("-RNTO {name}"));
    }
    Some(a)
}

fn push_once(sink: &Sink, body: &str) -> Result<(), String> {
    let url = sink.push.as_deref().unwrap_or("");
    let mut cmd = Command::new("curl");
    if is_file_transfer(url) {
        let args = transfer_args(url).ok_or_else(|| trs!("Adresse ohne Dateiname (…/ticker.json erwartet)", "address without a file name (expected …/ticker.json)", "adresse sans nom de fichier (…/ticker.json attendu)", "dirección sin nombre de archivo (se espera …/ticker.json)").to_string())?;
        cmd.args(["-sS", "--fail", "--max-time", "30"]).args(&args);
    } else {
        cmd.args(["-sS", "--fail", "--max-time", "20", "-X", if sink.method.is_empty() { "PUT" } else { &sink.method }, "-H", "Content-Type: application/json", "--data-binary", "@-"]);
        if let Some(k) = sink.key.as_deref().filter(|k| !k.is_empty()) {
            cmd.args(["-H", &format!("Authorization: Bearer {k}")]);
        }
        cmd.arg(url);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
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
        if !(u.starts_with("http://") || u.starts_with("https://") || is_file_transfer(u)) {
            return Err(tr!("--ticker-push: die Adresse muss mit sftp://, ftps://, ftp://, https:// oder http:// beginnen", "--ticker-push: the address must start with sftp://, ftps://, ftp://, https:// or http://", "--ticker-push : l'adresse doit commencer par sftp://, ftps://, ftp://, https:// ou http://", "--ticker-push: la dirección debe empezar por sftp://, ftps://, ftp://, https:// o http://"));
        }
        if is_file_transfer(u) && split_target(u).is_none() {
            return Err(tr!("--ticker-push: {} braucht einen Dateinamen, z. B. sftp://benutzer:passwort@server/pfad/ticker.json", "--ticker-push: {} needs a file name, e.g. sftp://user:password@server/path/ticker.json", "--ticker-push : {} nécessite un nom de fichier, p. ex. sftp://utilisateur:motdepasse@serveur/chemin/ticker.json", "--ticker-push: {} necesita un nombre de archivo, p. ej. sftp://usuario:contraseña@servidor/ruta/ticker.json", mask_url(u)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftp_uploads_go_to_a_temporary_name_and_are_renamed() {
        let a = transfer_args("ftp://u:p@host:21/www/club/ticker.json").unwrap();
        assert!(a.contains(&"ftp://u:p@host:21/www/club/ticker.json.part".to_string()), "{a:?}");
        assert!(a.contains(&"-RNFR ticker.json.part".to_string()) && a.contains(&"-RNTO ticker.json".to_string()), "{a:?}");
        // the file goes to the root of the account too
        assert!(transfer_args("ftp://host/ticker.json").unwrap().contains(&"ftp://host/ticker.json.part".to_string()));
    }

    #[test]
    fn sftp_renames_with_full_or_home_relative_paths() {
        let abs = transfer_args("sftp://u@host/var/www/ticker.json").unwrap();
        assert!(abs.contains(&"-rename /var/www/ticker.json.part /var/www/ticker.json".to_string()), "{abs:?}");
        let home = transfer_args("sftp://u@host/~/htdocs/ticker/ticker.json").unwrap();
        assert!(home.contains(&"-rename htdocs/ticker/ticker.json.part htdocs/ticker/ticker.json".to_string()), "{home:?}");
        assert!(home.contains(&"sftp://u@host/~/htdocs/ticker/ticker.json.part".to_string()), "{home:?}");
        let top = transfer_args("sftp://u@host/ticker.json").unwrap();
        assert!(top.contains(&"-rename /ticker.json.part /ticker.json".to_string()), "{top:?}");
    }

    #[test]
    fn an_address_without_a_file_name_is_refused_and_the_scheme_is_told_apart() {
        assert!(transfer_args("sftp://u@host/dir/").is_none());
        assert!(transfer_args("sftp://u@host").is_none());
        assert!(is_file_transfer("SFTP://u@h/f") && is_file_transfer("ftps://h/f") && is_file_transfer("ftp://h/f"));
        assert!(!is_file_transfer("https://h/f"));
        let bad = check(&Sink { push: Some("sftp://u@host/dir/".into()), ..Default::default() });
        assert!(bad.unwrap_err().contains("ticker.json"));
    }
}
