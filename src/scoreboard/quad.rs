//! Geometry of the scoreboard: homography between the rectified board and the video frame (f64).

pub type Mat3 = [f64; 9];
pub type Quad = [[f64; 2]; 4];

/// Homography mapping the four points `src` onto `dst` (Gaussian elimination with pivoting).
pub fn homography(src: &Quad, dst: &Quad) -> Option<Mat3> {
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i][0], src[i][1]);
        let (u, v) = (dst[i][0], dst[i][1]);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for c in 0..8 {
        let mut p = c;
        for r in c + 1..8 {
            if a[r][c].abs() > a[p][c].abs() {
                p = r;
            }
        }
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        let d = a[c][c];
        for k in c..9 {
            a[c][k] /= d;
        }
        for r in 0..8 {
            if r != c {
                let f = a[r][c];
                if f != 0.0 {
                    for k in c..9 {
                        let t = a[c][k];
                        a[r][k] -= f * t;
                    }
                }
            }
        }
    }
    Some([a[0][8], a[1][8], a[2][8], a[3][8], a[4][8], a[5][8], a[6][8], a[7][8], 1.0])
}

pub fn apply(h: &Mat3, x: f64, y: f64) -> (f64, f64) {
    let w = h[6] * x + h[7] * y + h[8];
    ((h[0] * x + h[1] * y + h[2]) / w, (h[3] * x + h[4] * y + h[5]) / w)
}

/// Rectangle (0,0)-(w,h) as a quad: top left, top right, bottom right, bottom left.
pub fn rect_quad(w: f64, h: f64) -> Quad {
    [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]
}

/// Convex, not degenerate and with a minimum area?
pub fn is_valid_quad(q: &Quad) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross.abs() < 1e-6 {
            return false;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    area(q) > 16.0
}

pub fn area(q: &Quad) -> f64 {
    let mut s = 0.0;
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s.abs() / 2.0
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Mean width and height of the quad (edge lengths).
pub fn quad_size(q: &Quad) -> (f64, f64) {
    ((dist(q[0], q[1]) + dist(q[3], q[2])) / 2.0, (dist(q[0], q[3]) + dist(q[1], q[2])) / 2.0)
}

/// Order points as top left, top right, bottom right, bottom left.
pub fn order_quad(p: &Quad) -> Quad {
    let cx = p.iter().map(|q| q[0]).sum::<f64>() / 4.0;
    let cy = p.iter().map(|q| q[1]).sum::<f64>() / 4.0;
    let mut v: Vec<[f64; 2]> = p.to_vec();
    // clockwise (y points down): ascending angle
    v.sort_by(|a, b| (a[1] - cy).atan2(a[0] - cx).total_cmp(&(b[1] - cy).atan2(b[0] - cx)));
    // start: the point with the smallest x+y (top left)
    let s = (0..4)
        .min_by(|&i, &j| (v[i][0] + v[i][1]).total_cmp(&(v[j][0] + v[j][1])))
        .unwrap_or(0);
    [v[s], v[(s + 1) % 4], v[(s + 2) % 4], v[(s + 3) % 4]]
}

fn bilinear(src: &[u8], sw: usize, sh: usize, x: f64, y: f64) -> [f32; 3] {
    let x = (x - 0.5).clamp(0.0, (sw - 1) as f64);
    let y = (y - 0.5).clamp(0.0, (sh - 1) as f64);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let px = |xx: usize, yy: usize| {
        let o = (yy * sw + xx) * 3;
        [src[o] as f32, src[o + 1] as f32, src[o + 2] as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0f32; 3];
    for k in 0..3 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bot - top) * fy;
    }
    out
}

/// Rectify the quad `quad` of the RGB image `src` (sw × sh) into a dw × dh rectangle.
pub fn warp_quad(src: &[u8], sw: usize, sh: usize, quad: &Quad, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 3];
    let Some(h) = homography(&rect_quad(dw as f64, dh as f64), quad) else {
        return out;
    };
    for y in 0..dh {
        for x in 0..dw {
            let (u, v) = apply(&h, x as f64 + 0.5, y as f64 + 0.5);
            let c = bilinear(src, sw, sh, u, v);
            let o = (y * dw + x) * 3;
            out[o] = c[0].round() as u8;
            out[o + 1] = c[1].round() as u8;
            out[o + 2] = c[2].round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = rect_quad(100.0, 40.0);
        let dst: Quad = [[10.0, 20.0], [210.0, 35.0], [200.0, 95.0], [14.0, 80.0]];
        let h = homography(&src, &dst).unwrap();
        for i in 0..4 {
            let (u, v) = apply(&h, src[i][0], src[i][1]);
            assert!((u - dst[i][0]).abs() < 1e-6 && (v - dst[i][1]).abs() < 1e-6);
        }
        // the centre lies inside the quad
        let (cx, cy) = apply(&h, 50.0, 20.0);
        assert!(cx > 10.0 && cx < 210.0 && cy > 20.0 && cy < 95.0);
    }

    #[test]
    fn validity_and_order() {
        let good: Quad = [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
        assert!(is_valid_quad(&good));
        let bowtie: Quad = [[0.0, 0.0], [100.0, 50.0], [100.0, 0.0], [0.0, 50.0]];
        assert!(!is_valid_quad(&bowtie));
        let shuffled: Quad = [[100.0, 50.0], [0.0, 0.0], [0.0, 50.0], [100.0, 0.0]];
        assert_eq!(order_quad(&shuffled), good);
    }

    #[test]
    fn warp_identity_crop() {
        let (w, h) = (20usize, 10usize);
        let mut img = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                img[(y * w + x) * 3] = (x * 10) as u8;
            }
        }
        let quad: Quad = [[5.0, 2.0], [15.0, 2.0], [15.0, 8.0], [5.0, 8.0]];
        let out = warp_quad(&img, w, h, &quad, 10, 6);
        // output pixel (0,0) is x=5.5 -> ~ column 5
        assert!((out[0] as i32 - 55).abs() <= 6, "{}", out[0]);
        assert!(out[9 * 3] > out[0]);
    }
}
