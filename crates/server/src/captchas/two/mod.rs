use std::io::Cursor;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Click, SliderHint};
use image::{ImageEncoder, Rgba, RgbaImage};

use crate::captcha::{Captcha, Rendered};
use crate::token;

pub const PUZZLE_W: u32 = 320;
pub const PUZZLE_H: u32 = 240;
pub const EXPECTED_CLICKS: usize = 2;
const PIECE: f64 = 52.0;
const KNOB: f64 = 11.0;
const BBOX: f64 = PIECE + KNOB * 2.0;
const TOLERANCE: f64 = 16.0;
const MIN_TOTAL_MS: f64 = 200.0;
const MAX_TOTAL_MS: f64 = 15_000.0;
const EDGE: f64 = 8.0;

pub struct Slider;

impl Captcha for Slider {
    fn kind(&self) -> &'static str { "two" }
    fn expected_clicks(&self) -> usize { EXPECTED_CLICKS }
    fn puzzle_w(&self) -> f64 { PUZZLE_W as f64 }
    fn puzzle_h(&self) -> f64 { PUZZLE_H as f64 }
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered {
        let mut rng = derive_rng(challenge_key, challenge_id);
        let (tx, ty) = target_pos(&mut rng);
        let (sx, sy) = start_pos(&mut rng, tx, ty);
        let bg = render_background(&mut rng);
        let mask = jigsaw_mask();
        let board = render_board(&bg, &mask, tx, ty);
        let piece = render_piece(&bg, &mask, tx, ty);
        Rendered {
            frames_b64: vec![encode(&board), encode(&piece)],
            motions: vec![],
            slider: Some(SliderHint {
                piece_w: BBOX,
                piece_h: BBOX,
                start_x: sx,
                start_y: sy,
            }),
        }
    }
    fn validate(&self, clicks: &[Click]) -> Result<(), &'static str> {
        if clicks.len() != EXPECTED_CLICKS {
            return Err("wrong click count");
        }
        for c in clicks {
            if !c.x.is_finite() || !c.y.is_finite() || !c.t.is_finite() {
                return Err("non-finite click");
            }
            if c.x < -BBOX || c.x > PUZZLE_W as f64 || c.y < -BBOX || c.y > PUZZLE_H as f64 {
                return Err("click out of bounds");
            }
        }
        if clicks[1].t < clicks[0].t {
            return Err("non-monotonic clicks");
        }
        let total = clicks[1].t - clicks[0].t;
        if total < MIN_TOTAL_MS || total > MAX_TOTAL_MS {
            return Err("solve time implausible");
        }
        Ok(())
    }
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click]) -> Result<(), &'static str> {
        let mut rng = derive_rng(challenge_key, challenge_id);
        let (tx, ty) = target_pos(&mut rng);
        let drop = &clicks[1];
        let dx = drop.x - tx;
        let dy = drop.y - ty;
        if dx.abs() <= TOLERANCE && dy.abs() <= TOLERANCE {
            Ok(())
        } else {
            Err("piece not aligned")
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next() * (hi - lo)
    }
}

fn derive_rng(key: &[u8], challenge_id: &str) -> Rng {
    let mut seed = [0u8; 32];
    token::derive_bytes(key, &format!("slider|{challenge_id}"), &mut seed);
    Rng(u64::from_le_bytes(seed[..8].try_into().unwrap()) | 1)
}

fn target_pos(rng: &mut Rng) -> (f64, f64) {
    let tx = rng.range(BBOX + EDGE, PUZZLE_W as f64 - BBOX - EDGE);
    let ty = rng.range(EDGE, PUZZLE_H as f64 - BBOX - EDGE);
    (tx, ty)
}

fn start_pos(rng: &mut Rng, tx: f64, ty: f64) -> (f64, f64) {
    for _ in 0..40 {
        let sx = rng.range(EDGE, PUZZLE_W as f64 - BBOX - EDGE);
        let sy = rng.range(EDGE, PUZZLE_H as f64 - BBOX - EDGE);
        if (sx - tx).abs() + (sy - ty).abs() > 70.0 {
            return (sx, sy);
        }
    }
    (EDGE, EDGE)
}

fn render_background(rng: &mut Rng) -> RgbaImage {
    let h0 = rng.range(0.0, 360.0);
    let h1 = h0 + rng.range(40.0, 140.0);
    let mut img = RgbaImage::new(PUZZLE_W, PUZZLE_H);
    for y in 0..PUZZLE_H {
        for x in 0..PUZZLE_W {
            let fx = x as f64 / PUZZLE_W as f64;
            let fy = y as f64 / PUZZLE_H as f64;
            let h = h0 + (h1 - h0) * fx;
            let s = 0.45 + 0.2 * (fy * 6.28).sin();
            let l = 0.40 + 0.18 * fy + 0.06 * (fx * 9.42).sin();
            let [r, g, b] = hsl_to_rgb(h, s.clamp(0.2, 0.8), l.clamp(0.2, 0.75));
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    img
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (((h * 6.0) % 2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    ]
}

fn jigsaw_mask() -> Vec<Vec<bool>> {
    let n = BBOX as usize;
    let mut m = vec![vec![false; n]; n];
    let l = KNOB;
    let r = KNOB;
    let body_x0 = l;
    let body_y0 = r;
    let body_x1 = l + PIECE;
    let body_y1 = r + PIECE;
    let kr = KNOB;
    let cx_top = l + PIECE / 2.0;
    let cy_top = r;
    let cx_right = l + PIECE;
    let cy_right = r + PIECE / 2.0;
    for y in 0..n {
        for x in 0..n {
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;
            let in_body = fx >= body_x0 && fx <= body_x1 && fy >= body_y0 && fy <= body_y1;
            let in_top = ((fx - cx_top).powi(2) + (fy - cy_top).powi(2)).sqrt() <= kr;
            let in_right = ((fx - cx_right).powi(2) + (fy - cy_right).powi(2)).sqrt() <= kr;
            let socket_left = ((fx - body_x0).powi(2) + (fy - (r + PIECE / 2.0)).powi(2)).sqrt() <= kr * 0.9;
            m[y][x] = (in_body || in_top || in_right) && !socket_left;
        }
    }
    m
}

fn render_board(bg: &RgbaImage, mask: &[Vec<bool>], tx: f64, ty: f64) -> RgbaImage {
    let mut img = bg.clone();
    let n = mask.len();
    for my in 0..n {
        for mx in 0..n {
            if !mask[my][mx] {
                continue;
            }
            let px = tx as i64 + mx as i64;
            let py = ty as i64 + my as i64;
            if px < 0 || py < 0 || px >= PUZZLE_W as i64 || py >= PUZZLE_H as i64 {
                continue;
            }
            let p = img.get_pixel(px as u32, py as u32).0;
            let edge = is_edge(mask, mx, my);
            let v = if edge {
                [235, 245, 255, 255]
            } else {
                [
                    (p[0] as f64 * 0.32) as u8,
                    (p[1] as f64 * 0.32) as u8,
                    (p[2] as f64 * 0.34 + 18.0) as u8,
                    255,
                ]
            };
            img.put_pixel(px as u32, py as u32, Rgba(v));
        }
    }
    img
}

fn render_piece(bg: &RgbaImage, mask: &[Vec<bool>], tx: f64, ty: f64) -> RgbaImage {
    let n = mask.len();
    let mut img = RgbaImage::new(n as u32, n as u32);
    for my in 0..n {
        for mx in 0..n {
            if !mask[my][mx] {
                img.put_pixel(mx as u32, my as u32, Rgba([0, 0, 0, 0]));
                continue;
            }
            let sx = (tx as i64 + mx as i64).clamp(0, PUZZLE_W as i64 - 1) as u32;
            let sy = (ty as i64 + my as i64).clamp(0, PUZZLE_H as i64 - 1) as u32;
            let p = bg.get_pixel(sx, sy).0;
            let v = if is_edge(mask, mx, my) {
                [245, 250, 255, 255]
            } else {
                [p[0], p[1], p[2], 255]
            };
            img.put_pixel(mx as u32, my as u32, Rgba(v));
        }
    }
    img
}

fn is_edge(mask: &[Vec<bool>], x: usize, y: usize) -> bool {
    let n = mask.len();
    if !mask[y][x] {
        return false;
    }
    for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
        let nx = x as i64 + dx;
        let ny = y as i64 + dy;
        if nx < 0 || ny < 0 || nx >= n as i64 || ny >= n as i64 || !mask[ny as usize][nx as usize] {
            return true;
        }
    }
    false
}

fn encode(img: &RgbaImage) -> String {
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        Cursor::new(&mut buf),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Up,
    )
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
        .unwrap();
    B64.encode(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::Click;

    fn click(x: f64, y: f64, t: f64) -> Click {
        Click { x, y, t }
    }

    fn target(key: &[u8], id: &str) -> (f64, f64) {
        let mut rng = derive_rng(key, id);
        target_pos(&mut rng)
    }

    #[test]
    fn grades_aligned_drop() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (tx, ty) = target(key, id);
        let clicks = vec![click(0.0, 0.0, 0.0), click(tx, ty, 900.0)];
        assert!(s.validate(&clicks).is_ok());
        assert!(s.grade(key, id, &clicks).is_ok());
    }

    #[test]
    fn rejects_misaligned_drop() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (tx, ty) = target(key, id);
        let clicks = vec![click(0.0, 0.0, 0.0), click(tx + 40.0, ty, 900.0)];
        assert!(s.validate(&clicks).is_ok());
        assert!(s.grade(key, id, &clicks).is_err());
    }

    #[test]
    fn rejects_wrong_click_count() {
        let s = Slider;
        let clicks = vec![click(0.0, 0.0, 0.0)];
        assert!(s.validate(&clicks).is_err());
    }
}

