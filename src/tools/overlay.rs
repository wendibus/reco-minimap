//! Graphics drawn into the videos: a scoreboard (five looks, each adjustable) and a watermark.
//!
//! The settings are the project's `overlay` object (also `--overlay '<json>'` on the command line):
//!
//! ```json
//! {"scoreboard": {"on": true, "preset": "classic", "anchor": "tl", "scale": 1.0, "opacity": 0.85,
//!                 "clock": true, "period": true, "home": "TIG", "away": "MAN",
//!                 "home_color": "#0042aa", "away_color": "#000000", "bg": "#10141a", "fg": "#ffffff",
//!                 "period_fmt": "Q{n}"},
//!  "watermark": {"on": true, "text": "RecoCam", "anchor": "tr", "scale": 1.0, "opacity": 0.5}}
//! ```
//!
//! Text is set in a bold sans font of the system (Arial, Segoe UI, DejaVu …), looked up at run
//! time - no font ships with the program. `RECO_FONT` may name another TrueType file. Without any
//! font only digits are drawn.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use super::video::Frame;

type Rgb = [u8; 3];

/// Where on the picture a graphic sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "tl" => Self::TopLeft,
            "tc" | "t" => Self::Top,
            "tr" => Self::TopRight,
            "ml" | "l" => Self::Left,
            "mc" | "c" => Self::Center,
            "mr" | "r" => Self::Right,
            "bl" => Self::BottomLeft,
            "bc" | "b" => Self::Bottom,
            "br" => Self::BottomRight,
            _ => return None,
        })
    }

    /// Top-left corner of a `w`x`h` box in a `fw`x`fh` picture, `m` px from the edges.
    fn place(self, fw: f32, fh: f32, w: f32, h: f32, m: f32) -> (f32, f32) {
        let x = match self {
            Self::TopLeft | Self::Left | Self::BottomLeft => m,
            Self::Top | Self::Center | Self::Bottom => (fw - w) / 2.0,
            _ => fw - w - m,
        };
        let y = match self {
            Self::TopLeft | Self::Top | Self::TopRight => m,
            Self::Left | Self::Center | Self::Right => (fh - h) / 2.0,
            _ => fh - h - m,
        };
        (x.round(), y.round())
    }
}

/// The five scoreboard looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Preset {
    /// Two rows (one per team) with a colour stripe, period and clock below - top left.
    Classic,
    /// One wide bar like on TV: names outside, scores in the team colours, clock in the middle.
    Bar,
    /// One small line: `TIG 43 – 66 MAN`.
    Compact,
    /// Large scores with the names below, period and clock between them.
    Big,
    /// No box, only shadowed text.
    Minimal,
}

impl Preset {
    pub const ALL: [&'static str; 5] = ["classic", "bar", "compact", "big", "minimal"];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "classic" => Self::Classic,
            "bar" => Self::Bar,
            "compact" => Self::Compact,
            "big" => Self::Big,
            "minimal" => Self::Minimal,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Scoreboard {
    pub preset: Preset,
    pub anchor: Anchor,
    pub scale: f32,
    /// Opacity of the boxes (text is always opaque).
    pub opacity: f32,
    pub clock: bool,
    pub period: bool,
    pub names: [String; 2],
    pub colors: [Rgb; 2],
    pub bg: Rgb,
    pub fg: Rgb,
    /// `{n}` becomes the period number (e.g. `Q{n}`, `{n}. HZ`).
    pub period_fmt: String,
}

#[derive(Clone, Debug)]
pub struct Watermark {
    pub text: String,
    pub anchor: Anchor,
    pub scale: f32,
    pub opacity: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Overlay {
    pub scoreboard: Option<Scoreboard>,
    pub watermark: Option<Watermark>,
}

/// What the scoreboard shows at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    pub home: u32,
    pub away: u32,
    pub period: Option<u32>,
    /// Game clock in seconds.
    pub clock: Option<u32>,
}

pub fn hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// A short name for the scoreboard: the first three letters, in capitals.
pub fn short_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).take(3).collect::<String>().to_uppercase()
}

impl Overlay {
    /// From the project's `overlay` object. `teams`: the full team names (for the default short
    /// names), `colors`: the team colours of the minimap.
    pub fn from_json(v: &Value, teams: [&str; 2], colors: [Rgb; 2]) -> Self {
        let f = |o: &Value, k: &str, d: f64| o[k].as_f64().unwrap_or(d) as f32;
        let b = &v["scoreboard"];
        let scoreboard = (b["on"].as_bool() == Some(true)).then(|| {
            let fallback = [crate::trs!("HEIM", "HOME", "DOM", "LOC"), crate::trs!("GAST", "AWAY", "EXT", "VIS")];
            let name = |k: &str, i: usize| b[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| Some(short_name(teams[i])).filter(|s| !s.is_empty()).unwrap_or_else(|| fallback[i].to_string()));
            Scoreboard {
                preset: b["preset"].as_str().and_then(Preset::parse).unwrap_or(Preset::Classic),
                anchor: b["anchor"].as_str().and_then(Anchor::parse).unwrap_or(Anchor::TopLeft),
                scale: f(b, "scale", 1.0).clamp(0.3, 4.0),
                opacity: f(b, "opacity", 0.85).clamp(0.0, 1.0),
                clock: b["clock"].as_bool() != Some(false),
                period: b["period"].as_bool() != Some(false),
                names: [name("home", 0), name("away", 1)],
                colors: [b["home_color"].as_str().and_then(hex).unwrap_or(colors[0]), b["away_color"].as_str().and_then(hex).unwrap_or(colors[1])],
                bg: b["bg"].as_str().and_then(hex).unwrap_or([16, 20, 26]),
                fg: b["fg"].as_str().and_then(hex).unwrap_or([255, 255, 255]),
                period_fmt: b["period_fmt"].as_str().unwrap_or("Q{n}").to_string(),
            }
        });
        let w = &v["watermark"];
        let watermark = (w["on"].as_bool() == Some(true)).then(|| Watermark {
            text: w["text"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("RecoCam").to_string(),
            anchor: w["anchor"].as_str().and_then(Anchor::parse).unwrap_or(Anchor::TopRight),
            scale: f(w, "scale", 1.0).clamp(0.3, 4.0),
            opacity: f(w, "opacity", 0.5).clamp(0.05, 1.0),
        });
        Self { scoreboard, watermark }
    }

    pub fn is_empty(&self) -> bool {
        self.scoreboard.is_none() && self.watermark.is_none()
    }

    /// Draw everything onto `frame`. `score`: `None` hides the scoreboard (nothing known yet).
    pub fn draw(&self, frame: &mut Frame, score: Option<Score>) {
        if let (Some(b), Some(s)) = (&self.scoreboard, score) {
            draw_scoreboard(frame, b, &s);
        }
        if let Some(w) = &self.watermark {
            draw_watermark(frame, w);
        }
    }
}

// ---------------------------------------------------------------- text

struct Glyph {
    w: usize,
    h: usize,
    xmin: i32,
    ymin: i32,
    advance: f32,
    bitmap: Vec<u8>,
}

struct Fonts {
    font: Option<fontdue::Font>,
    cache: Mutex<HashMap<(char, u32), std::sync::Arc<Glyph>>>,
}

/// Bold sans fonts of the common systems, best first.
const FONT_PATHS: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "C:\\Windows\\Fonts\\arialbd.ttf",
    "C:\\Windows\\Fonts\\segoeuib.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
];

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let env = std::env::var("RECO_FONT").ok();
        let font = env.iter().map(String::as_str).chain(FONT_PATHS.iter().copied()).find_map(|p| {
            let data = std::fs::read(p).ok()?;
            fontdue::Font::from_bytes(data, fontdue::FontSettings { scale: 64.0, ..Default::default() }).ok()
        });
        Fonts { font, cache: Mutex::new(HashMap::new()) }
    })
}

/// True when a system font was found (text other than digits can be drawn).
pub fn has_font() -> bool {
    fonts().font.is_some()
}

fn glyph(c: char, px: f32) -> Option<std::sync::Arc<Glyph>> {
    let f = fonts();
    let font = f.font.as_ref()?;
    let key = (c, (px * 4.0).round() as u32);
    if let Some(g) = f.cache.lock().unwrap().get(&key) {
        return Some(g.clone());
    }
    let (m, bitmap) = font.rasterize(c, px);
    let g = std::sync::Arc::new(Glyph { w: m.width, h: m.height, xmin: m.xmin, ymin: m.ymin, advance: m.advance_width, bitmap });
    let mut c2 = f.cache.lock().unwrap();
    if c2.len() > 4000 {
        c2.clear();
    }
    c2.insert(key, g.clone());
    Some(g)
}

/// 5x7 digits for when no font is found.
const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// Width of `s` at font size `px` (cap height about 0.72 px).
pub fn text_width(s: &str, px: f32) -> f32 {
    if has_font() {
        s.chars().filter_map(|c| glyph(c, px)).map(|g| g.advance).sum()
    } else {
        let cell = px * 0.72 / 7.0;
        s.chars().map(|c| if c.is_ascii_digit() { 6.0 * cell } else if c == ' ' { 3.0 * cell } else { 2.0 * cell }).sum()
    }
}

fn blend(frame: &mut Frame, x: i64, y: i64, c: Rgb, a: f32) {
    if x < 0 || y < 0 || x as usize >= frame.w || y as usize >= frame.h || a <= 0.0 {
        return;
    }
    let i = (y as usize * frame.w + x as usize) * 3;
    let a = a.min(1.0);
    for k in 0..3 {
        frame.rgb[i + k] = (c[k] as f32 * a + frame.rgb[i + k] as f32 * (1.0 - a)).round() as u8;
    }
}

/// Draw `s` with its baseline at `base` starting at `x`. Returns the width.
pub fn draw_text(frame: &mut Frame, s: &str, x: f32, base: f32, px: f32, c: Rgb, a: f32) -> f32 {
    let mut pen = x;
    if !has_font() {
        let cell = px * 0.72 / 7.0;
        let top = base - 7.0 * cell;
        for ch in s.chars() {
            if let Some(d) = ch.to_digit(10) {
                for (row, bits) in DIGITS[d as usize].iter().enumerate() {
                    for col in 0..5 {
                        if bits >> (4 - col) & 1 == 1 {
                            fill_rect(frame, pen + col as f32 * cell, top + row as f32 * cell, cell, cell, c, a, 0.0);
                        }
                    }
                }
                pen += 6.0 * cell;
            } else {
                pen += if ch == ' ' { 3.0 } else { 2.0 } * cell;
            }
        }
        return pen - x;
    }
    for ch in s.chars() {
        let Some(g) = glyph(ch, px) else { continue };
        let gx = (pen + g.xmin as f32).round() as i64;
        let gy = (base - g.ymin as f32 - g.h as f32).round() as i64;
        for yy in 0..g.h {
            for xx in 0..g.w {
                let v = g.bitmap[yy * g.w + xx];
                if v > 0 {
                    blend(frame, gx + xx as i64, gy + yy as i64, c, a * v as f32 / 255.0);
                }
            }
        }
        pen += g.advance;
    }
    pen - x
}

/// Text centred in a box (`cx` centre, `cy` middle of the cap height).
fn text_centered(frame: &mut Frame, s: &str, cx: f32, cy: f32, px: f32, c: Rgb, a: f32) {
    let w = text_width(s, px);
    draw_text(frame, s, cx - w / 2.0, cy + px * 0.36, px, c, a);
}

/// Text with a soft dark shadow (readable on any picture).
fn text_shadow(frame: &mut Frame, s: &str, x: f32, base: f32, px: f32, c: Rgb, a: f32) -> f32 {
    let o = (px / 18.0).max(1.0);
    draw_text(frame, s, x + o, base + o, px, [0, 0, 0], a * 0.6);
    draw_text(frame, s, x, base, px, c, a)
}

/// A filled rectangle with rounded corners (radius `r`), anti-aliased at the edges.
pub fn fill_rect(frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32, r: f32) {
    if w <= 0.0 || h <= 0.0 || a <= 0.0 {
        return;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (x0, y0, x1, y1) = (x.floor() as i64, y.floor() as i64, (x + w).ceil() as i64, (y + h).ceil() as i64);
    for py in y0.max(0)..y1.min(frame.h as i64) {
        for px in x0.max(0)..x1.min(frame.w as i64) {
            // coverage of the pixel by the box (edges), then by the rounded corner
            let cx = ((px as f32 + 1.0).min(x + w) - (px as f32).max(x)).clamp(0.0, 1.0);
            let cy = ((py as f32 + 1.0).min(y + h) - (py as f32).max(y)).clamp(0.0, 1.0);
            let mut cov = cx * cy;
            if r > 0.0 {
                let (qx, qy) = (px as f32 + 0.5, py as f32 + 0.5);
                let dx = if qx < x + r { x + r - qx } else if qx > x + w - r { qx - (x + w - r) } else { 0.0 };
                let dy = if qy < y + r { y + r - qy } else if qy > y + h - r { qy - (y + h - r) } else { 0.0 };
                if dx > 0.0 && dy > 0.0 {
                    cov *= (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                }
            }
            blend(frame, px, py, c, a * cov);
        }
    }
}

/// Black or white, whichever reads better on `c`.
fn on(c: Rgb) -> Rgb {
    let l = 0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32;
    if l > 150.0 { [12, 14, 18] } else { [255, 255, 255] }
}

/// A team colour that stands out from the box: one too close to `bg` is lightened (or darkened).
fn visible(c: Rgb, bg: Rgb) -> Rgb {
    let d: f32 = (0..3).map(|k| (c[k] as f32 - bg[k] as f32).powi(2)).sum::<f32>().sqrt();
    if d >= 90.0 {
        return c;
    }
    let lum = |c: Rgb| 0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32;
    let target = if lum(bg) < 128.0 { 255.0 } else { 0.0 };
    let mix = 0.45;
    [0, 1, 2].map(|k| (c[k] as f32 * (1.0 - mix) + target * mix).round() as u8)
}

fn clock_text(s: u32) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

// ---------------------------------------------------------------- scoreboard

/// The line under or between the scores: period and clock, as far as known and wanted.
fn info_line(b: &Scoreboard, s: &Score) -> String {
    let mut parts = Vec::new();
    if b.period {
        if let Some(p) = s.period {
            parts.push(b.period_fmt.replace("{n}", &p.to_string()));
        }
    }
    if b.clock {
        if let Some(c) = s.clock {
            parts.push(clock_text(c));
        }
    }
    parts.join("  ")
}

fn draw_scoreboard(frame: &mut Frame, b: &Scoreboard, s: &Score) {
    let mut b = b.clone();
    b.colors = b.colors.map(|c| visible(c, b.bg));
    let b = &b;
    let u = frame.h.min(frame.w * 9 / 16).max(1) as f32 / 1080.0 * b.scale;
    let (fw, fh) = (frame.w as f32, frame.h as f32);
    let m = 28.0 * u;
    let scores = [s.home.to_string(), s.away.to_string()];
    let info = info_line(b, s);
    let (bg, fg, op) = (b.bg, b.fg, b.opacity);
    match b.preset {
        Preset::Classic => {
            let (row, stripe, name_px, score_px, pad) = (50.0 * u, 9.0 * u, 28.0 * u, 34.0 * u, 16.0 * u);
            let name_w = b.names.iter().map(|n| text_width(n, name_px)).fold(0.0, f32::max).max(70.0 * u);
            let score_w = 74.0 * u;
            let w = stripe + pad + name_w + pad + score_w;
            let foot = if info.is_empty() { 0.0 } else { 34.0 * u };
            let h = 2.0 * row + foot;
            let (x, y) = b.anchor.place(fw, fh, w, h, m);
            fill_rect(frame, x, y, w, h, bg, op, 8.0 * u);
            for i in 0..2 {
                let ry = y + i as f32 * row;
                fill_rect(frame, x, ry + 6.0 * u, stripe, row - 12.0 * u, b.colors[i], 1.0, 2.0 * u);
                draw_text(frame, &b.names[i], x + stripe + pad, ry + row / 2.0 + name_px * 0.36, name_px, fg, 1.0);
                fill_rect(frame, x + w - score_w, ry + 5.0 * u, score_w - 5.0 * u, row - 10.0 * u, [255, 255, 255], 0.10 * op.max(0.3), 5.0 * u);
                text_centered(frame, &scores[i], x + w - score_w / 2.0 - 2.5 * u, ry + row / 2.0, score_px, fg, 1.0);
            }
            if foot > 0.0 {
                fill_rect(frame, x, y + 2.0 * row, w, 1.0 * u.max(1.0), fg, 0.25, 0.0);
                text_centered(frame, &info, x + w / 2.0, y + 2.0 * row + foot / 2.0, 21.0 * u, fg, 0.9);
            }
        }
        Preset::Bar => {
            let (h, name_px, score_px, pad) = (56.0 * u, 26.0 * u, 34.0 * u, 18.0 * u);
            let name_w = b.names.iter().map(|n| text_width(n, name_px)).fold(0.0, f32::max).max(60.0 * u);
            let score_w = 78.0 * u;
            let mid_w = if info.is_empty() { 14.0 * u } else { text_width(&info, 22.0 * u) + 2.0 * pad };
            let stripe = 8.0 * u;
            let w = 2.0 * (stripe + pad + name_w + pad + score_w) + mid_w;
            let (x, y) = b.anchor.place(fw, fh, w, h, m);
            fill_rect(frame, x, y, w, h, bg, op, 6.0 * u);
            // home: stripe | name | score ; away mirrored
            fill_rect(frame, x, y, stripe, h, b.colors[0], 1.0, 0.0);
            fill_rect(frame, x + w - stripe, y, stripe, h, b.colors[1], 1.0, 0.0);
            let nx0 = x + stripe + pad;
            draw_text(frame, &b.names[0], nx0 + name_w - text_width(&b.names[0], name_px), y + h / 2.0 + name_px * 0.36, name_px, fg, 1.0);
            let sx0 = nx0 + name_w + pad;
            fill_rect(frame, sx0, y, score_w, h, b.colors[0], 1.0, 0.0);
            text_centered(frame, &scores[0], sx0 + score_w / 2.0, y + h / 2.0, score_px, on(b.colors[0]), 1.0);
            let mx = sx0 + score_w;
            if !info.is_empty() {
                text_centered(frame, &info, mx + mid_w / 2.0, y + h / 2.0, 22.0 * u, fg, 0.9);
            }
            let sx1 = mx + mid_w;
            fill_rect(frame, sx1, y, score_w, h, b.colors[1], 1.0, 0.0);
            text_centered(frame, &scores[1], sx1 + score_w / 2.0, y + h / 2.0, score_px, on(b.colors[1]), 1.0);
            draw_text(frame, &b.names[1], sx1 + score_w + pad, y + h / 2.0 + name_px * 0.36, name_px, fg, 1.0);
        }
        Preset::Compact => {
            let (h, px, pad, stripe) = (40.0 * u, 24.0 * u, 12.0 * u, 6.0 * u);
            let sep = "–";
            let line = [b.names[0].as_str(), &scores[0], sep, &scores[1], b.names[1].as_str()];
            let gaps = 10.0 * u;
            let tw: f32 = line.iter().map(|t| text_width(t, px)).sum::<f32>() + gaps * 4.0;
            let info_w = if info.is_empty() { 0.0 } else { text_width(&info, 18.0 * u) + 2.0 * pad };
            let w = 2.0 * stripe + 2.0 * pad + tw + info_w;
            let (x, y) = b.anchor.place(fw, fh, w, h, m);
            fill_rect(frame, x, y, w, h, bg, op, h / 2.0);
            fill_rect(frame, x + h * 0.25, y + h * 0.25, stripe, h * 0.5, b.colors[0], 1.0, stripe / 2.0);
            let mut pen = x + stripe + pad + h * 0.15;
            for (i, t) in line.iter().enumerate() {
                pen += draw_text(frame, t, pen, y + h / 2.0 + px * 0.36, px, fg, if i == 2 { 0.6 } else { 1.0 }) + gaps;
            }
            let ax = pen - gaps + pad * 0.6;
            fill_rect(frame, ax, y + h * 0.25, stripe, h * 0.5, b.colors[1], 1.0, stripe / 2.0);
            if !info.is_empty() {
                draw_text(frame, &info, ax + stripe + pad, y + h / 2.0 + 18.0 * u * 0.36, 18.0 * u, fg, 0.8);
            }
        }
        Preset::Big => {
            let (score_px, name_px, pad) = (78.0 * u, 22.0 * u, 22.0 * u);
            let col = scores.iter().map(|s| text_width(s, score_px)).fold(0.0, f32::max).max(b.names.iter().map(|n| text_width(n, name_px)).fold(0.0, f32::max)).max(110.0 * u);
            let mid = if info.is_empty() { 40.0 * u } else { text_width(&info, 24.0 * u).max(40.0 * u) + pad };
            let (w, h) = (2.0 * col + mid + 2.0 * pad, 130.0 * u);
            let (x, y) = b.anchor.place(fw, fh, w, h, m);
            fill_rect(frame, x, y, w, h, bg, op, 12.0 * u);
            for i in 0..2 {
                let cx = if i == 0 { x + pad + col / 2.0 } else { x + w - pad - col / 2.0 };
                text_centered(frame, &scores[i], cx, y + 52.0 * u, score_px, fg, 1.0);
                fill_rect(frame, cx - col * 0.35, y + 92.0 * u, col * 0.7, 5.0 * u, b.colors[i], 1.0, 2.5 * u);
                text_centered(frame, &b.names[i], cx, y + 112.0 * u, name_px, fg, 0.95);
            }
            let mx = x + w / 2.0;
            if info.is_empty() {
                text_centered(frame, ":", mx, y + 52.0 * u, score_px * 0.7, fg, 0.7);
            } else {
                text_centered(frame, &info, mx, y + 56.0 * u, 24.0 * u, fg, 0.9);
            }
        }
        Preset::Minimal => {
            let px = 38.0 * u;
            let line = format!("{} {}  :  {} {}", b.names[0], scores[0], scores[1], b.names[1]);
            let tw = text_width(&line, px);
            let info_px = 20.0 * u;
            let h = px + if info.is_empty() { 0.0 } else { info_px * 1.5 };
            let (x, y) = b.anchor.place(fw, fh, tw, h, m);
            let dot = 10.0 * u;
            text_shadow(frame, &line, x, y + px * 0.8, px, fg, 1.0);
            fill_rect(frame, x, y + px + 2.0 * u, text_width(&b.names[0], px), 4.0 * u, b.colors[0], 1.0, 2.0 * u);
            let aw = text_width(&b.names[1], px);
            fill_rect(frame, x + tw - aw, y + px + 2.0 * u, aw, 4.0 * u, b.colors[1], 1.0, 2.0 * u);
            if !info.is_empty() {
                let iw = text_width(&info, info_px);
                text_shadow(frame, &info, x + (tw - iw) / 2.0, y + px + info_px * 1.35 + dot * 0.2, info_px, fg, 0.9);
            }
        }
    }
}

fn draw_watermark(frame: &mut Frame, w: &Watermark) {
    let u = frame.h.min(frame.w * 9 / 16).max(1) as f32 / 1080.0 * w.scale;
    let px = 34.0 * u;
    let tw = text_width(&w.text, px);
    let (x, y) = w.anchor.place(frame.w as f32, frame.h as f32, tw, px, 28.0 * u);
    text_shadow(frame, &w.text, x, y + px * 0.8, px, [255, 255, 255], w.opacity);
}

// ---------------------------------------------------------------- the score over time

/// The score at every moment of a result, from its score events (live picks and hand-confirmed
/// baskets included) and the period / clock of the board readings.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    /// (t, home, away) after every change, in time order.
    changes: Vec<(f64, u32, u32)>,
    /// (t, period, clock) of the board readings that had one.
    board: Vec<(f64, Option<u32>, Option<u32>)>,
}

impl Timeline {
    /// `events`: the final score events of a result ([`crate::live_score::scores_of`]): the score
    /// of a team is its `to` after the latest of its events.
    pub fn from_events(events: &[crate::live_score::ScoreEvent], readings: &[crate::scoreboard::Reading]) -> Self {
        let mut ev: Vec<&crate::live_score::ScoreEvent> = events.iter().filter(|e| !e.retracted).collect();
        ev.sort_by(|a, b| a.t.total_cmp(&b.t));
        let (mut h, mut a) = (0u32, 0u32);
        let mut changes = Vec::new();
        for e in ev {
            match e.team {
                crate::team::Team::Home => h = e.to,
                crate::team::Team::Away => a = e.to,
                _ => continue,
            }
            changes.push((e.t, h, a));
        }
        let board = readings.iter().filter(|r| r.period.is_some() || r.clock.is_some()).map(|r| (r.t, r.period, r.clock)).collect();
        Self { changes, board }
    }

    pub fn at(&self, t: f64) -> Score {
        let i = self.changes.partition_point(|c| c.0 <= t);
        let (home, away) = if i == 0 { (0, 0) } else { (self.changes[i - 1].1, self.changes[i - 1].2) };
        // the period stays until the next reading; a clock only while it is fresh
        let j = self.board.partition_point(|b| b.0 <= t);
        let period = self.board[..j].iter().rev().find_map(|b| b.1);
        let clock = (j > 0).then(|| self.board[j - 1]).filter(|b| t - b.0 < 3.0).and_then(|b| b.2);
        Score { home, away, period, clock }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(w: usize, h: usize) -> Frame {
        Frame { w, h, rgb: vec![90; w * h * 3] }
    }

    #[test]
    fn every_preset_draws_inside_the_picture_at_every_anchor() {
        for p in Preset::ALL {
            for a in ["tl", "tc", "tr", "ml", "mc", "mr", "bl", "bc", "br"] {
                let o = Overlay::from_json(&json!({"scoreboard": {"on": true, "preset": p, "anchor": a, "scale": 1.0}, "watermark": {"on": true}}), ["Tigers", "Mantas"], [[0, 66, 170], [0, 0, 0]]);
                let mut f = frame(640, 360);
                o.draw(&mut f, Some(Score { home: 43, away: 66, period: Some(3), clock: Some(321) }));
                assert!(f.rgb.iter().any(|&v| v != 90), "{p} {a} drew nothing");
            }
        }
    }

    #[test]
    fn settings_fall_back_to_sensible_defaults() {
        let o = Overlay::from_json(&json!({"scoreboard": {"on": true}, "watermark": {"on": true, "text": " "}}), ["Tigers", "Mantas"], [[1, 2, 3], [4, 5, 6]]);
        let b = o.scoreboard.unwrap();
        assert_eq!((b.preset, b.anchor), (Preset::Classic, Anchor::TopLeft));
        assert_eq!(b.names, ["TIG".to_string(), "MAN".to_string()]);
        assert_eq!(b.colors, [[1, 2, 3], [4, 5, 6]]);
        assert_eq!(o.watermark.unwrap().text, "RecoCam");
        assert!(Overlay::from_json(&json!({}), ["a", "b"], [[0; 3]; 2]).is_empty());
    }

    #[test]
    fn the_timeline_adds_up_the_baskets_and_keeps_the_period() {
        let e = |sid: u32, t: f64, team: &str, to: u32, retracted: bool| crate::live_score::parse_event(&json!({"type": "score", "sid": sid, "t": t, "team": team, "points": 2, "kind": "field_goal", "to": to, "retracted": retracted})).unwrap();
        let ev = vec![e(1, 10.0, "home", 2, false), e(2, 20.0, "away", 3, false), e(3, 25.0, "away", 6, true), e(4, 30.0, "home", 3, false)];
        let rd = vec![crate::scoreboard::Reading { t: 5.0, home: None, away: None, clock: Some(600), period: Some(2) }];
        let tl = Timeline::from_events(&ev, &rd);
        assert_eq!(tl.at(0.0), Score { home: 0, away: 0, period: None, clock: None });
        assert_eq!(tl.at(6.0), Score { home: 0, away: 0, period: Some(2), clock: Some(600) });
        assert_eq!((tl.at(26.0).home, tl.at(26.0).away), (2, 3));
        assert_eq!((tl.at(31.0).home, tl.at(31.0).away, tl.at(31.0).period, tl.at(31.0).clock), (3, 3, Some(2), None));
    }
}
