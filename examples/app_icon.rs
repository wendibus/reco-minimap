//! Draws the app icon of the Mac app (1024 px PNG with transparency): the orange Reco mark with
//! a small minimap on it. `scripts/package-mac.sh` turns it into the `.icns` file.
//!
//!   cargo run --example app_icon -- mac/AppIcon.png

const SIZE: usize = 1024;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// PNG with stored (uncompressed) deflate blocks: no compression library needed.
fn png(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for row in rgba.chunks_exact(w * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        z.push(if blocks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &raw {
        a = (a + v as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut head = Vec::new();
    head.extend_from_slice(&(w as u32).to_be_bytes());
    head.extend_from_slice(&(h as u32).to_be_bytes());
    head.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &head);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Signed distance to a rounded rectangle centred at (cx, cy).
fn round_rect(x: f32, y: f32, cx: f32, cy: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let (dx, dy) = ((x - cx).abs() - (half_w - r), (y - cy).abs() - (half_h - r));
    dx.max(dy).min(0.0) + (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r
}

fn cover(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

fn over(dst: &mut [f32; 4], c: [f32; 3], a: f32) {
    for i in 0..3 {
        dst[i] = dst[i] * (1.0 - a) + c[i] * a;
    }
    dst[3] = dst[3] + (1.0 - dst[3]) * a;
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "mac/AppIcon.png".into());
    let white = [1.0, 1.0, 1.0];
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    // court: outline, halfway line, centre circle and the two keys, in icon pixels
    let (cx, cy, hw, hh, line) = (512.0f32, 512.0f32, 300.0f32, 170.0f32, 9.0f32);
    let dots: [(f32, f32, [f32; 3]); 7] = [
        (330.0, 430.0, white), (430.0, 560.0, white), (560.0, 440.0, white),
        (470.0, 470.0, [0.08, 0.16, 0.42]), (640.0, 560.0, [0.08, 0.16, 0.42]), (720.0, 450.0, [0.08, 0.16, 0.42]),
        (600.0, 520.0, [0.10, 0.10, 0.11]),
    ];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut px = [0.0f32; 4];
            // the mark: radial gradient like the brand mark in the interface
            let plate = cover(round_rect(fx, fy, 512.0, 512.0, 412.0, 412.0, 185.0));
            if plate > 0.0 {
                let t = (((fx - 370.0).powi(2) + (fy - 320.0).powi(2)).sqrt() / 760.0).clamp(0.0, 1.0);
                let (a, b, c) = ([1.0, 0.77, 0.42], [1.0, 0.62, 0.04], [0.91, 0.42, 0.0]);
                let mix = |p: [f32; 3], q: [f32; 3], k: f32| [p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k, p[2] + (q[2] - p[2]) * k];
                let col = if t < 0.38 { mix(a, b, t / 0.38) } else { mix(b, c, (t - 0.38) / 0.62) };
                over(&mut px, col, plate);
                let outline = round_rect(fx, fy, cx, cy, hw, hh, 22.0).abs() - line / 2.0;
                let inside = round_rect(fx, fy, cx, cy, hw, hh, 22.0) < 0.0;
                let halfway = ((fx - cx).abs() - line / 2.0).max((fy - cy).abs() - hh);
                let circle = (((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() - 62.0).abs() - line / 2.0;
                let key = |side: f32| {
                    let kx = cx + side * (hw - 55.0);
                    (round_rect(fx, fy, kx, cy, 55.0, 80.0, 4.0).abs() - line / 2.0).max(-(side * (fx - (cx + side * hw))))
                };
                let lines = outline.min(halfway).min(if inside { circle.min(key(-1.0)).min(key(1.0)) } else { f32::MAX });
                over(&mut px, white, cover(lines) * 0.92 * plate);
                for (dx, dy, col) in dots {
                    let d = ((fx - dx).powi(2) + (fy - dy).powi(2)).sqrt();
                    over(&mut px, white, cover(d - 31.0) * plate);
                    over(&mut px, col, cover(d - 25.0) * plate);
                }
            }
            let i = (y * SIZE + x) * 4;
            for k in 0..3 {
                rgba[i + k] = (px[k] * 255.0).round() as u8;
            }
            rgba[i + 3] = (px[3] * 255.0).round() as u8;
        }
    }
    std::fs::write(&path, png(&rgba, SIZE, SIZE)).unwrap_or_else(|e| panic!("{path}: {e}"));
    println!("{path}");
}
