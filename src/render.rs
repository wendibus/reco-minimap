//! Software renderer for the minimap (RGBA, no dependencies).
//! Output can go to OBS (`reco-obs`), a PiP via FFmpeg, or a PNG/PPM.

use crate::ball::BallState;
use crate::geom::{Vec2, COURT_LENGTH, COURT_WIDTH};
use crate::team::Team;
use crate::{AgentOut, FrameState};

pub type Rgb = [u8; 3];

#[derive(Clone, Debug)]
pub struct Theme {
    pub px_per_m: f32,
    pub pad_m: f32,
    pub floor: Rgb,
    pub lines: Rgb,
    pub home: Rgb,
    pub away: Rgb,
    pub referee: Rgb,
    pub ball: Rgb,
    pub holder_ring: Rgb,
    pub background_alpha: u8,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            px_per_m: 30.0,
            pad_m: 1.5,
            floor: [38, 44, 52],
            lines: [225, 228, 232],
            home: [220, 40, 40],
            away: [40, 110, 230],
            referee: [0, 0, 0],
            ball: [255, 140, 0],
            holder_ring: [255, 140, 0],
            background_alpha: 255,
        }
    }
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    /// RGBA, row-major, straight alpha.
    pub rgba: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: Rgb, a: u8) -> Self {
        let mut rgba = vec![0u8; w * h * 4];
        for p in rgba.chunks_exact_mut(4) {
            p.copy_from_slice(&[bg[0], bg[1], bg[2], a]);
        }
        Self { w, h, rgba }
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgb, cov: f32) {
        if cov <= 0.0 {
            return;
        }
        let i = (y * self.w + x) * 4;
        let a = cov.clamp(0.0, 1.0);
        let dst_a = self.rgba[i + 3] as f32 / 255.0;
        let out_a = a + dst_a * (1.0 - a);
        for k in 0..3 {
            let d = self.rgba[i + k] as f32;
            let v = if out_a > 0.0 { (c[k] as f32 * a + d * dst_a * (1.0 - a)) / out_a } else { 0.0 };
            self.rgba[i + k] = v.round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[i + 3] = (out_a * 255.0).round() as u8;
    }

    /// Filled disc (r_in = 0) or ring (r_in > 0), anti-aliased, `alpha` in 0..1.
    pub fn disc(&mut self, cx: f32, cy: f32, r_out: f32, r_in: f32, c: Rgb, alpha: f32) {
        let (x0, x1) = ((cx - r_out - 1.0).floor().max(0.0) as usize, ((cx + r_out + 1.0).ceil() as usize).min(self.w.saturating_sub(1)));
        let (y0, y1) = ((cy - r_out - 1.0).floor().max(0.0) as usize, ((cy + r_out + 1.0).ceil() as usize).min(self.h.saturating_sub(1)));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let outer = (r_out - d + 0.5).clamp(0.0, 1.0);
                let inner = if r_in > 0.0 { (d - r_in + 0.5).clamp(0.0, 1.0) } else { 1.0 };
                self.blend(x, y, c, outer * inner * alpha);
            }
        }
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), thick: f32, c: Rgb, alpha: f32) {
        let r = thick / 2.0;
        let (x0, x1) = ((a.0.min(b.0) - r - 1.0).floor().max(0.0) as usize, ((a.0.max(b.0) + r + 1.0).ceil() as usize).min(self.w.saturating_sub(1)));
        let (y0, y1) = ((a.1.min(b.1) - r - 1.0).floor().max(0.0) as usize, ((a.1.max(b.1) + r + 1.0).ceil() as usize).min(self.h.saturating_sub(1)));
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = (dx * dx + dy * dy).max(1e-9);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - a.0) * dx + (py - a.1) * dy) / l2).clamp(0.0, 1.0);
                let d = ((px - (a.0 + t * dx)).powi(2) + (py - (a.1 + t * dy)).powi(2)).sqrt();
                self.blend(x, y, c, (r - d + 0.5).clamp(0.0, 1.0) * alpha);
            }
        }
    }

    pub fn write_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.w, self.h).into_bytes();
        for p in self.rgba.chunks_exact(4) {
            out.extend_from_slice(&p[..3]);
        }
        out
    }
}

pub struct Renderer {
    pub theme: Theme,
}

impl Renderer {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    pub fn size(&self) -> (usize, usize) {
        let t = &self.theme;
        (((COURT_LENGTH + 2.0 * t.pad_m) * t.px_per_m).round() as usize, ((COURT_WIDTH + 2.0 * t.pad_m) * t.px_per_m).round() as usize)
    }

    fn px(&self, p: Vec2) -> (f32, f32) {
        let t = &self.theme;
        ((p.x + t.pad_m) * t.px_per_m, (p.y + t.pad_m) * t.px_per_m)
    }

    fn poly(&self, c: &mut Canvas, pts: &[Vec2], th: f32) {
        for w in pts.windows(2) {
            c.line(self.px(w[0]), self.px(w[1]), th, self.theme.lines, 1.0);
        }
    }

    fn arc(&self, centre: Vec2, r: f32, a0: f32, a1: f32) -> Vec<Vec2> {
        let n = 48;
        (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                Vec2::new(centre.x + r * a.cos(), centre.y + r * a.sin())
            })
            .collect()
    }

    pub fn draw_court(&self, c: &mut Canvas) {
        let th = (0.08 * self.theme.px_per_m).max(1.5);
        let (l, w) = (COURT_LENGTH, COURT_WIDTH);
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| vec![Vec2::new(x0, y0), Vec2::new(x1, y0), Vec2::new(x1, y1), Vec2::new(x0, y1), Vec2::new(x0, y0)];
        self.poly(c, &rect(0.0, 0.0, l, w), th);
        self.poly(c, &[Vec2::new(l / 2.0, 0.0), Vec2::new(l / 2.0, w)], th);
        self.poly(c, &self.arc(Vec2::new(l / 2.0, w / 2.0), 1.8, 0.0, std::f32::consts::TAU), th);
        let (key_len, key_w, ft_r, basket, tp_r, tp_side) = (5.8f32, 4.9f32, 1.8f32, 1.575f32, 6.75f32, 0.9f32);
        for &left in &[true, false] {
            let s = if left { 1.0 } else { -1.0 };
            let bx = if left { 0.0 } else { l };
            let at = |d: f32| bx + s * d;
            self.poly(c, &rect(at(0.0), (w - key_w) / 2.0, at(key_len), (w + key_w) / 2.0), th);
            let ft = Vec2::new(at(key_len), w / 2.0);
            let (a0, a1) = if left { (-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2) } else { (std::f32::consts::FRAC_PI_2, 3.0 * std::f32::consts::FRAC_PI_2) };
            self.poly(c, &self.arc(ft, ft_r, a0, a1), th);
            let bk = Vec2::new(at(basket), w / 2.0);
            let (x, y) = self.px(bk);
            c.disc(x, y, 0.23 * self.theme.px_per_m, 0.23 * self.theme.px_per_m - th, self.theme.lines, 1.0);
            // three-point line: straight sections 0.9 m from the sidelines + arc
            let dy = w / 2.0 - tp_side;
            let ang = (dy / tp_r).clamp(-1.0, 1.0).asin();
            let ex = tp_r * ang.cos();
            let (p0, p1) = (Vec2::new(at(0.0), tp_side), Vec2::new(at(basket + ex), tp_side));
            let (q0, q1) = (Vec2::new(at(0.0), w - tp_side), Vec2::new(at(basket + ex), w - tp_side));
            self.poly(c, &[p0, p1], th);
            self.poly(c, &[q0, q1], th);
            let (b0, b1) = if left { (-ang, ang) } else { (std::f32::consts::PI - ang, std::f32::consts::PI + ang) };
            self.poly(c, &self.arc(bk, tp_r, b0, b1), th);
        }
    }

    fn agent(&self, c: &mut Canvas, a: &AgentOut, holder: bool) {
        let t = &self.theme;
        let (x, y) = self.px(a.pos);
        let r = 0.45 * t.px_per_m;
        let (fill, edge) = if a.referee { (t.referee, [235u8, 235, 235]) } else if a.team == Team::Home { (t.home, [15u8, 15, 15]) } else { (t.away, [15u8, 15, 15]) };
        c.disc(x, y, r + 1.5, 0.0, edge, a.alpha * 0.9);
        c.disc(x, y, r, 0.0, fill, a.alpha);
        if holder {
            c.disc(x, y, r * 1.75, r * 1.75 - (0.13 * t.px_per_m).max(2.0), t.holder_ring, 1.0);
        }
    }

    pub fn render(&self, s: &FrameState) -> Canvas {
        let (w, h) = self.size();
        let mut c = Canvas::new(w, h, self.theme.floor, self.theme.background_alpha);
        self.draw_court(&mut c);
        let holder = if let BallState::Held { track } = s.ball.state { Some(track) } else { None };
        for a in s.referees.iter().chain(s.players.iter()) {
            self.agent(&mut c, a, holder == Some(a.id));
        }
        // ball only when it is free: small orange dot
        if matches!(s.ball.state, BallState::InFlight | BallState::Loose) {
            if let Some(p) = s.ball.pos {
                let (x, y) = self.px(p);
                let r = 0.20 * self.theme.px_per_m;
                if s.ball.state == BallState::InFlight {
                    c.disc(x + r * 0.5, y + r * 0.9, r * 0.9, 0.0, [0, 0, 0], 0.25); // soft shadow
                }
                c.disc(x, y, r + 1.0, 0.0, [30, 20, 0], 0.9);
                c.disc(x, y, r, 0.0, self.theme.ball, 1.0);
            }
        }
        c
    }
}
