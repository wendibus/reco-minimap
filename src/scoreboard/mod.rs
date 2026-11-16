//! Reading the hall scoreboard (seven-segment LEDs) that is visible in the panorama itself.
//!
//! Ported from the separate `reco-scoreboard` program. The board is a quad in the frame
//! ([`Board`]); the digit blocks ([`FieldCfg`]) lie in the coordinates of the rectified board.
//! [`detect::Detector`] reads every frame (the LEDs flicker, see there) and reports the score,
//! game clock and period once they are stable. [`find`] proposes the board and its digit blocks
//! for the calibration page.
//!
//! All coordinates refer to the frame size `frame` used when calibrating and are scaled to the
//! real video size at run time, so one calibration works for the original and a smaller proxy.
//!
//! Pure Rust, no dependencies; the JSON file and the video stream live in `tools::scoreboard`.

pub mod detect;
pub mod find;
pub mod quad;
pub mod seven_seg;

use quad::{Mat3, Quad};

/// The scoreboard as a quad in the frame. The digit blocks lie in the coordinates of the
/// rectified view `size` (pixels), not in the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Board {
    /// Top left, top right, bottom right, bottom left (frame coordinates)
    pub corners: Quad,
    /// Size of the rectified view [width, height]
    pub size: [u32; 2],
}

impl Board {
    /// Mapping rectified view -> frame.
    pub fn to_frame(&self) -> Option<Mat3> {
        quad::homography(&quad::rect_quad(self.size[0] as f64, self.size[1] as f64), &self.corners)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Brightness (Rec.709) – default
    Luma,
    /// Maximum of R,G,B – good for red/orange LEDs with a weak luma
    Max,
    Red,
    Green,
    Blue,
}

impl Channel {
    pub fn key(self) -> &'static str {
        match self {
            Channel::Luma => "luma",
            Channel::Max => "max",
            Channel::Red => "red",
            Channel::Green => "green",
            Channel::Blue => "blue",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "luma" => Channel::Luma,
            "max" => Channel::Max,
            "red" => Channel::Red,
            "green" => Channel::Green,
            "blue" => Channel::Blue,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectCfg {
    /// Length of the time window per segment (ms), against the flicker.
    pub window_ms: f64,
    /// Percentile within the window (0.8 = "was bright in at least 20 % of the frames").
    pub percentile: f64,
    /// Observation time (ms) for the clock: a new value is taken when it was read in at least
    /// `clock_share` of the valid readings of that time. Short and strict: the clock changes every
    /// second, intermediate states must not slip through.
    pub clock_stable_ms: f64,
    pub clock_share: f64,
    /// Observation time (ms) and majority for score and period: long and lenient, so that every
    /// score arrives even with strong flicker.
    pub score_stable_ms: f64,
    pub score_share: f64,
    pub period_stable_ms: f64,
    /// Minimum distance (0..255) between "on" and "off", otherwise nothing is read.
    pub min_contrast: f64,
    /// Threshold between "off" (0) and "on" (1) in the brightness range. Below 0.5 because lit
    /// segments vary more than dark ones.
    pub threshold: f64,
    pub channel: Channel,
    /// true = dark digits on a bright background.
    pub invert: bool,
}

impl Default for DetectCfg {
    fn default() -> Self {
        Self {
            window_ms: 320.0,
            percentile: 0.8,
            clock_stable_ms: 320.0,
            clock_share: 0.85,
            score_stable_ms: 1200.0,
            score_share: 0.6,
            period_stable_ms: 1500.0,
            min_contrast: 25.0,
            threshold: 0.4,
            channel: Channel::Luma,
            invert: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldCfg {
    /// Frame around the whole digit block: [x, y, width, height]
    pub rect: [f64; 4],
    /// Number of digits in this block
    pub digits: usize,
    /// Gap between digits, relative to the digit width
    pub gap_ratio: f64,
    /// Slant (italic digits): horizontal offset per pixel of height. Positive = leaning right at
    /// the top. The frame includes the slanted digits.
    pub slant: f64,
}

/// Sample rectangles of the 7 segments (a,b,c,d,e,f,g) relative to the digit box (x0,y0,x1,y1, 0..1).
const SEG: [[f64; 4]; 7] = [
    [0.28, 0.02, 0.72, 0.12], // a top
    [0.84, 0.16, 0.98, 0.42], // b top right
    [0.84, 0.58, 0.98, 0.84], // c bottom right
    [0.28, 0.88, 0.72, 0.98], // d bottom
    [0.02, 0.58, 0.16, 0.84], // e bottom left
    [0.02, 0.16, 0.16, 0.42], // f top left
    [0.28, 0.45, 0.72, 0.55], // g middle
];

impl FieldCfg {
    /// Segment rectangles per digit in calibration coordinates: [x0,y0,x1,y1] × 7.
    pub fn segment_rects(&self) -> Vec<[[f64; 4]; 7]> {
        let [x, y, w, h] = self.rect;
        let n = self.digits.max(1) as f64;
        let shear = self.slant * h;
        let usable_w = (w - shear.abs()).max(1.0);
        let dw = usable_w / (n + (n - 1.0) * self.gap_ratio);
        let pitch = dw * (1.0 + self.gap_ratio);
        let x_start = x + shear.abs() / 2.0;
        let mut out = Vec::with_capacity(self.digits);
        for i in 0..self.digits {
            let dx = x_start + i as f64 * pitch;
            let mut rects = [[0.0; 4]; 7];
            for (k, s) in SEG.iter().enumerate() {
                let cy = y + (s[1] + s[3]) / 2.0 * h;
                let off = self.slant * (y + h / 2.0 - cy);
                rects[k] = [dx + s[0] * dw + off, y + s[1] * h, dx + s[2] * dw + off, y + s[3] * h];
            }
            out.push(rects);
        }
        out
    }
}

/// Which side of the board shows the home team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Frame size [width, height] the `board.corners` refer to.
    pub frame: [u32; 2],
    /// The board in the video (four corners). Without it the fields lie directly in frame coordinates.
    pub board: Option<Board>,
    pub detect: DetectCfg,
    pub left_score: FieldCfg,
    pub right_score: FieldCfg,
    pub clock: FieldCfg,
    pub period: Option<FieldCfg>,
    pub home: HomeSide,
}

/// Rectified-view rectangle -> the largest axis-parallel frame rectangle inside the mapped quad.
fn map_rect(h: &Mat3, r: &[f64; 4]) -> [f64; 4] {
    let tl = quad::apply(h, r[0], r[1]);
    let tr = quad::apply(h, r[2], r[1]);
    let br = quad::apply(h, r[2], r[3]);
    let bl = quad::apply(h, r[0], r[3]);
    let (x0, x1) = (tl.0.max(bl.0), tr.0.min(br.0));
    let (y0, y1) = (tl.1.max(tr.1), bl.1.min(br.1));
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        let c = quad::apply(h, (r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        return [c.0 - 0.5, c.1 - 0.5, c.0 + 0.5, c.1 + 0.5];
    }
    [x0, y0, x1, y1]
}

impl Config {
    /// Segment areas of a field in frame coordinates (calibration size `frame`).
    pub fn field_segments(&self, f: &FieldCfg) -> Vec<[[f64; 4]; 7]> {
        let raw = f.segment_rects();
        let Some(h) = self.board.as_ref().and_then(|b| b.to_frame()) else {
            return raw;
        };
        raw.into_iter()
            .map(|d| {
                let mut o = [[0.0; 4]; 7];
                for (i, r) in d.iter().enumerate() {
                    o[i] = map_rect(&h, r);
                }
                o
            })
            .collect()
    }

    pub fn fields(&self) -> Vec<(&'static str, &FieldCfg)> {
        let mut v = vec![("left_score", &self.left_score), ("right_score", &self.right_score), ("clock", &self.clock)];
        if let Some(p) = &self.period {
            v.push(("period", p));
        }
        v
    }

    /// Plausibility check; the error names the offending part.
    pub fn validate(&self) -> Result<(), String> {
        if self.frame[0] == 0 || self.frame[1] == 0 {
            return Err("frame must be > 0".into());
        }
        if let Some(b) = &self.board {
            if b.size[0] == 0 || b.size[1] == 0 {
                return Err("board.size must be > 0".into());
            }
            if !quad::is_valid_quad(&b.corners) || b.to_frame().is_none() {
                return Err("board.corners is not a valid quad".into());
            }
        }
        for (name, f) in self.fields() {
            if !(1..=6).contains(&f.digits) {
                return Err(format!("{name}: digits must be 1..6"));
            }
            if f.rect[2] <= 1.0 || f.rect[3] <= 1.0 {
                return Err(format!("{name}: rect has no area"));
            }
        }
        if !(0.0..=1.0).contains(&self.detect.percentile) {
            return Err("detect.percentile must be 0..1".into());
        }
        Ok(())
    }

    /// Bounding box (pixels of a `w` × `h` video) of everything the detector samples, with a
    /// margin, on even coordinates (for an ffmpeg crop). `[x, y, width, height]`.
    pub fn region(&self, w: usize, h: usize, margin: usize) -> [usize; 4] {
        let (sx, sy) = (w as f64 / self.frame[0] as f64, h as f64 / self.frame[1] as f64);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (_, f) in self.fields() {
            for d in self.field_segments(f) {
                for r in d {
                    x0 = x0.min(r[0] * sx);
                    y0 = y0.min(r[1] * sy);
                    x1 = x1.max(r[2] * sx);
                    y1 = y1.max(r[3] * sy);
                }
            }
        }
        if x0 > x1 {
            return [0, 0, w & !1, h & !1];
        }
        let m = margin as f64;
        let x0 = ((x0 - m).floor().max(0.0) as usize) & !1;
        let y0 = ((y0 - m).floor().max(0.0) as usize) & !1;
        let x1 = ((x1 + m).ceil().min(w as f64) as usize).max(x0 + 2);
        let y1 = ((y1 + m).ceil().min(h as f64) as usize).max(y0 + 2);
        let (cw, ch) = (((x1 - x0) + 1) & !1, ((y1 - y0) + 1) & !1);
        [x0, y0, cw.min((w - x0) & !1).max(2), ch.min((h - y0) & !1).max(2)]
    }
}

/// One stable reading of the board, with the sides already mapped to home / away.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// Video time (s) when this state was accepted (lags the board by the stabiliser time).
    pub t: f64,
    pub home: Option<u32>,
    pub away: Option<u32>,
    /// Game clock in seconds.
    pub clock: Option<u32>,
    pub period: Option<u32>,
}

impl Reading {
    pub fn from_state(t: f64, s: &detect::State, home: HomeSide) -> Self {
        let (h, a) = match home {
            HomeSide::Left => (s.left, s.right),
            HomeSide::Right => (s.right, s.left),
        };
        Self { t, home: h, away: a, clock: s.clock, period: s.period }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_inside_box() {
        let f = FieldCfg { rect: [100.0, 50.0, 140.0, 60.0], digits: 2, gap_ratio: 0.25, slant: 0.0 };
        let r = f.segment_rects();
        assert_eq!(r.len(), 2);
        for d in &r {
            for s in d {
                assert!(s[0] >= 100.0 && s[2] <= 240.0 && s[1] >= 50.0 && s[3] <= 110.0);
            }
        }
        // the second digit lies right of the first
        assert!(r[1][0][0] > r[0][0][2]);
    }

    #[test]
    fn region_covers_fields_on_even_pixels() {
        let f = |x: f64| FieldCfg { rect: [x, 41.0, 51.0, 31.0], digits: 2, gap_ratio: 0.25, slant: 0.0 };
        let c = Config { frame: [1280, 720], board: None, detect: DetectCfg::default(), left_score: f(101.0), right_score: f(401.0), clock: f(251.0), period: None, home: HomeSide::Left };
        // video at half the calibration size
        let [x, y, w, h] = c.region(640, 360, 4);
        assert!(x % 2 == 0 && y % 2 == 0 && w % 2 == 0 && h % 2 == 0);
        assert!(x as f64 <= 50.5 && (x + w) as f64 >= 226.0, "{x} {w}");
        assert!(y as f64 <= 20.5 && (y + h) as f64 >= 36.0, "{y} {h}");
    }
}
