//! Update check against the project's GitHub repository, and installing a newer version.
//!
//! The check asks GitHub for the latest release (or, if the repository has no releases, its
//! version tags) and compares it with the version this program was built from. Installing runs
//! `cargo install --git ... --tag <tag>` with the same features as the running build, so it needs
//! the Rust toolchain that was used to install the program in the first place. Nothing is sent
//! except the request itself; no video, frame or project data is involved.

use std::process::{Command, Stdio};

use serde_json::Value;

/// `owner/name` of the repository that is asked for new versions.
pub const REPO: &str = "wendibus/reco-minimap";

/// The version this program was built from.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A published version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latest {
    /// `major.minor.patch` without a leading `v`.
    pub version: String,
    /// The git tag as published (what `cargo install --tag` needs).
    pub tag: String,
    /// Page describing the version.
    pub url: String,
}

/// `1.2.3` or `v1.2.3` (a missing patch or minor counts as 0; a suffix such as `-rc1` is ignored).
pub fn parse_version(s: &str) -> Option<[u32; 3]> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut out = [0u32; 3];
    let mut n = 0;
    for part in core.split('.') {
        if n == 3 {
            return None;
        }
        out[n] = part.parse().ok()?;
        n += 1;
    }
    (n > 0).then_some(out)
}

/// True if `candidate` is a higher version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// A tag is only passed on to `cargo` if it looks like one.
fn plain_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 64 && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn latest_of(tag: &str, url: Option<&str>) -> Option<Latest> {
    let v = parse_version(tag)?;
    plain_tag(tag).then(|| Latest {
        version: format!("{}.{}.{}", v[0], v[1], v[2]),
        tag: tag.to_string(),
        url: url.map(str::to_string).unwrap_or_else(|| format!("https://github.com/{REPO}/releases/tag/{tag}")),
    })
}

/// The version in GitHub's answer to `releases/latest`. Drafts and pre-releases do not count.
pub fn latest_from_release(v: &Value) -> Option<Latest> {
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    latest_of(v["tag_name"].as_str()?, v["html_url"].as_str())
}

/// The highest version among GitHub's answer to `tags`.
pub fn latest_from_tags(v: &Value) -> Option<Latest> {
    v.as_array()?.iter().filter_map(|t| latest_of(t["name"].as_str()?, None)).max_by_key(|l| parse_version(&l.version))
}

fn get(url: &str) -> Option<Value> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "8", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: reco-minimap", url])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| serde_json::from_slice(&out.stdout).ok()).flatten()
}

/// Ask GitHub for the latest published version. `Ok(None)`: none is published.
pub fn latest() -> Result<Option<Latest>, String> {
    if let Some(l) = get(&format!("https://api.github.com/repos/{REPO}/releases/latest")).as_ref().and_then(latest_from_release) {
        return Ok(Some(l));
    }
    // no release (or a pre-release): fall back to version tags
    match get(&format!("https://api.github.com/repos/{REPO}/tags?per_page=100")) {
        Some(tags) => Ok(latest_from_tags(&tags)),
        None => Err(format!("GitHub nicht erreichbar (https://github.com/{REPO})")),
    }
}

/// The latest published version if it is newer than this program.
pub fn available() -> Result<Option<Latest>, String> {
    Ok(latest()?.filter(|l| is_newer(&l.version, CURRENT)))
}

/// Arguments of the `cargo install` that fetches and builds `latest`.
pub fn install_args(latest: &Latest) -> Vec<String> {
    let features = if cfg!(feature = "coreml") { "tools,coreml" } else { "tools" };
    ["install", "--git", &format!("https://github.com/{REPO}"), "--tag", &latest.tag, "--features", features, "--locked", "--force", "reco-minimap"].iter().map(|s| s.to_string()).collect()
}

/// Build and install `latest` with cargo; its output goes to this program's terminal.
/// The running program is replaced on disk and keeps running: restart it afterwards.
pub fn install(latest: &Latest) -> Result<(), String> {
    if !plain_tag(&latest.tag) {
        return Err(format!("unerwarteter Tag: {}", latest.tag));
    }
    let status = Command::new("cargo")
        .args(install_args(latest))
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("cargo nicht gefunden ({e}). Die Aktualisierung baut das Programm aus dem Quelltext und braucht dafür Rust: https://rustup.rs"))?;
    if status.success() { Ok(()) } else { Err("cargo install ist fehlgeschlagen (Ausgabe siehe oben)".into()) }
}
