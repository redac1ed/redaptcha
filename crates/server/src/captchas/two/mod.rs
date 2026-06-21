use std::io::Cursor;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Click, SliderHint, TrailPoint};
use image::{ImageEncoder, Rgba, RgbaImage};

use crate::captcha::{Captcha, Rendered};
use crate::token;

pub const PUZZLE_W: u32 = 320;
pub const PUZZLE_H: u32 = 240;
pub const EXPECTED_CLICKS: usize = 2;
const PIECE: f64 = 52.0;
const KNOB: f64 = 11.0;
const BBOX: f64 = PIECE + KNOB * 2.0;
const EDGE: f64 = 10.0;
const HOLE_COUNT: usize = 4;
const SELECT_TOLERANCE: f64 = BBOX * 0.6;
const CONFIRM_MIN_GAP_MS: f64 = 120.0;
const CONFIRM_MAX_GAP_MS: f64 = 14000.0;
const MIN_TRAIL_POINTS: usize = 5;
const MOVE_MIN_PATH_PX: f64 = 24.0;
const MAX_TELEPORT_FRAC: f64 = 0.7;
const PASS_NEAR_PX: f64 = BBOX;

pub struct Slider;

impl Captcha for Slider {
    fn kind(&self) -> &'static str { "two" }
    fn expected_clicks(&self) -> usize { EXPECTED_CLICKS }
    fn rounds(&self) -> u32 { 3 }
    fn puzzle_w(&self) -> f64 { PUZZLE_W as f64 }
    fn puzzle_h(&self) -> f64 { PUZZLE_H as f64 }
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered {
        let mut rng = derive_rng(challenge_key, challenge_id);
        let layout = build_layout(&mut rng);
        let bg = render_background(&mut rng);
        let answer = &layout.holes[layout.answer_idx];
        let board = render_board(&bg, &layout);
        let piece = render_piece(&bg, answer);
        Rendered {
            frames_b64: vec![encode(&board), encode(&piece)],
            slider: Some(SliderHint {
                piece_w: BBOX,
                piece_h: BBOX,
                start_x: layout.piece_x,
                start_y: layout.piece_y,
            }),
        }
    }
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click], _trail: &[TrailPoint]) -> Result<(), &'static str> {
        if clicks.len() != EXPECTED_CLICKS {
            return Err("wrong click count");
        }
        let mut rng = derive_rng(challenge_key, challenge_id);
        let layout = build_layout(&mut rng);
        let select = &clicks[0];
        let confirm = &clicks[1];
        let answer = &layout.holes[layout.answer_idx];
        let (ax, ay) = hole_center(answer);
        let dx = select.x - ax;
        let dy = select.y - ay;
        if (dx * dx + dy * dy).sqrt() > SELECT_TOLERANCE {
            for (i, h) in layout.holes.iter().enumerate() {
                if i == layout.answer_idx {
                    continue;
                }
                let (cx, cy) = hole_center(h);
                if ((select.x - cx).powi(2) + (select.y - cy).powi(2)).sqrt() <= SELECT_TOLERANCE {
                    return Err("wrong shape selected");
                }
            }
            return Err("no hole selected");
        }
        let gap = confirm.t - select.t;
        if gap < CONFIRM_MIN_GAP_MS || gap > CONFIRM_MAX_GAP_MS {
            return Err("confirm timing implausible");
        }
        Ok(())
    }
    fn track(&self, _challenge_key: &[u8], _challenge_id: &str, clicks: &[Click], trail: &[TrailPoint]) -> Result<(), &'static str> {
        if clicks.len() != EXPECTED_CLICKS {
            return Err("wrong click count");
        }
        if trail.len() < MIN_TRAIL_POINTS {
            return Err("no pointer movement");
        }
        let mut path_len = 0.0;
        let mut max_step = 0.0;
        for w in trail.windows(2) {
            let d = ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt();
            path_len += d;
            if d > max_step {
                max_step = d;
            }
        }
        if path_len < MOVE_MIN_PATH_PX {
            return Err("pointer barely moved");
        }
        if max_step / path_len > MAX_TELEPORT_FRAC {
            return Err("teleport detected");
        }
        let select = &clicks[0];
        let mut min_dist = f64::INFINITY;
        for p in trail {
            let d = ((p.x - select.x).powi(2) + (p.y - select.y).powi(2)).sqrt();
            if d < min_dist {
                min_dist = d;
            }
        }
        if min_dist > PASS_NEAR_PX {
            return Err("pointer did not approach selection")
        }
        Ok(())
    }
}

struct Hole {
    x: f64,
    y: f64,
    knobs: [i8; 4],
}

struct Layout {
    holes: Vec<Hole>,
    answer_idx: usize,
    piece_x: f64,
    piece_y: f64,
}

fn hole_center(h: &Hole) -> (f64, f64) {
    (h.x + BBOX / 2.0, h.y + BBOX / 2.0)
}

fn build_layout(rng: &mut Rng) -> Layout {
    let cols = 2usize;
    let rows = 2usize;
    let cell_w = (PUZZLE_W as f64 - EDGE * 2.0) / cols as f64;
    let cell_h = (PUZZLE_H as f64 - EDGE * 2.0) / rows as f64;
    let mut holes: Vec<Hole> = Vec::with_capacity(HOLE_COUNT);
    let mut used: Vec<[i8; 4]> = Vec::new();
    for i in 0..HOLE_COUNT {
        let c = i % cols;
        let r = i / cols;
        let jx = rng.range(0.0, (cell_w - BBOX).max(0.0));
        let jy = rng.range(0.0, (cell_h - BBOX).max(0.0));
        let x = EDGE + c as f64 * cell_w + jx;
        let y = EDGE + r as f64 * cell_h + jy;
        let mut knobs = random_knobs(rng);
        let mut guard = 0;
        while used.iter().any(|k| *k == knobs) && guard < 32 {
            knobs = random_knobs(rng);
            guard += 1;
        }
        used.push(knobs);
        holes.push(Hole { x, y, knobs });
    }
    let answer_idx = (rng.next() * HOLE_COUNT as f64) as usize % HOLE_COUNT;
    let piece_x = rng.range(EDGE, PUZZLE_W as f64 - BBOX - EDGE);
    let piece_y = rng.range(EDGE, PUZZLE_H as f64 - BBOX - EDGE);
    Layout { holes, answer_idx, piece_x, piece_y }
}

fn random_knobs(rng: &mut Rng) -> [i8; 4] {
    let mut k = [0i8; 4];
    for s in k.iter_mut() {
        let v = (rng.next() * 3.0) as i64;
        *s = (v - 1) as i8;
    }
    if k == [0, 0, 0, 0] {
        k[(rng.next() * 4.0) as usize % 4] = if rng.next() < 0.5 { 1 } else { -1 };
    }
    k
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

fn shape_mask(knobs: &[i8; 4]) -> Vec<Vec<bool>> {
    let n = BBOX as usize;
    let mut m = vec![vec![false; n]; n];
    let l = KNOB;
    let body_x0 = l;
    let body_y0 = l;
    let body_x1 = l + PIECE;
    let body_y1 = l + PIECE;
    let kr = KNOB;
    let mids = [
        (l + PIECE / 2.0, l),
        (l + PIECE, l + PIECE / 2.0),
        (l + PIECE / 2.0, l + PIECE),
        (l, l + PIECE / 2.0),
    ];
    for y in 0..n {
        for x in 0..n {
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;
            let mut inside = fx >= body_x0 && fx <= body_x1 && fy >= body_y0 && fy <= body_y1;
            for (i, &(mx, my)) in mids.iter().enumerate() {
                let d = ((fx - mx).powi(2) + (fy - my).powi(2)).sqrt();
                match knobs[i] {
                    1 => {
                        if d <= kr {
                            inside = true;
                        }
                    }
                    -1 => {
                        if d <= kr * 0.92 {
                            inside = false;
                        }
                    }
                    _ => {}
                }
            }
            m[y][x] = inside;
        }
    }
    m
}

fn stamp_hole(img: &mut RgbaImage, hole: &Hole) {
    let mask = shape_mask(&hole.knobs);
    let n = mask.len();
    for my in 0..n {
        for mx in 0..n {
            if !mask[my][mx] {
                continue;
            }
            let px = hole.x as i64 + mx as i64;
            let py = hole.y as i64 + my as i64;
            if px < 0 || py < 0 || px >= PUZZLE_W as i64 || py >= PUZZLE_H as i64 {
                continue;
            }
            let p = img.get_pixel(px as u32, py as u32).0;
            let edge = is_edge(&mask, mx, my);
            let f = if edge { 0.70 } else { 0.80 };
            let v = [
                (p[0] as f64 * f) as u8,
                (p[1] as f64 * f) as u8,
                (p[2] as f64 * f + 6.0).min(255.0) as u8,
                255,
            ];
            img.put_pixel(px as u32, py as u32, Rgba(v));
        }
    }
}

fn render_board(bg: &RgbaImage, layout: &Layout) -> RgbaImage {
    let mut img = bg.clone();
    for h in &layout.holes {
        stamp_hole(&mut img, h);
    }
    img
}

fn render_piece(bg: &RgbaImage, answer: &Hole) -> RgbaImage {
    let mask = shape_mask(&answer.knobs);
    let n = mask.len();
    let mut img = RgbaImage::new(n as u32, n as u32);
    let src_x = answer.x;
    let src_y = answer.y;
    for my in 0..n {
        for mx in 0..n {
            if !mask[my][mx] {
                img.put_pixel(mx as u32, my as u32, Rgba([0, 0, 0, 0]));
                continue;
            }
            let sx = (src_x as i64 + mx as i64).clamp(0, PUZZLE_W as i64 - 1) as u32;
            let sy = (src_y as i64 + my as i64).clamp(0, PUZZLE_H as i64 - 1) as u32;
            let p = bg.get_pixel(sx, sy).0;
            let v = if is_edge(&mask, mx, my) {
                [
                    (p[0] as f64 * 0.55 + 90.0).min(255.0) as u8,
                    (p[1] as f64 * 0.55 + 95.0).min(255.0) as u8,
                    (p[2] as f64 * 0.55 + 100.0).min(255.0) as u8,
                    255,
                ]
            } else {
                [
                    (p[0] as f64 * 0.85 + 26.0).min(255.0) as u8,
                    (p[1] as f64 * 0.85 + 26.0).min(255.0) as u8,
                    (p[2] as f64 * 0.85 + 30.0).min(255.0) as u8,
                    255,
                ]
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
    fn answer_center(key: &[u8], id: &str) -> (f64, f64) {
        let mut rng = derive_rng(key, id);
        let layout = build_layout(&mut rng);
        hole_center(&layout.holes[layout.answer_idx])
    }
    fn wrong_center(key: &[u8], id: &str) -> (f64, f64) {
        let mut rng = derive_rng(key, id);
        let layout = build_layout(&mut rng);
        let wi = (layout.answer_idx + 1) % layout.holes.len();
        hole_center(&layout.holes[wi])
    }
    fn approach_trail(sx: f64, sy: f64, ex: f64, ey: f64) -> Vec<TrailPoint> {
        let n = 14;
        (0..=n)
            .map(|i| {
                let f = i as f64 / n as f64;
                let e = f * f * (3.0 - 2.0 * f);
                let bow = (f * std::f64::consts::PI).sin() * 6.0;
                TrailPoint {
                    x: sx + (ex - sx) * e + bow,
                    y: sy + (ey - sy) * e - bow,
                    t: (f * 1300.0).round(),
                }
            })
            .collect()
    }
    #[test]
    fn accepts_correct_hole_and_confirm() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (ax, ay) = answer_center(key, id);
        let clicks = vec![click(ax, ay, 1300.0), click(40.0, 220.0, 1600.0)];
        let trail = approach_trail(20.0, 20.0, ax, ay);
        assert!(s.grade(key, id, &clicks, &trail).is_ok());
        assert!(s.track(key, id, &clicks, &trail).is_ok());
    }
    #[test]
    fn rejects_wrong_hole() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (wx, wy) = wrong_center(key, id);
        let clicks = vec![click(wx, wy, 1300.0), click(40.0, 220.0, 1600.0)];
        let trail = approach_trail(20.0, 20.0, wx, wy);
        assert!(s.grade(key, id, &clicks, &trail).is_err());
    }
    #[test]
    fn rejects_empty_space() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let clicks = vec![click(2.0, 2.0, 1300.0), click(40.0, 220.0, 1600.0)];
        let trail = approach_trail(20.0, 20.0, 2.0, 2.0);
        assert!(s.grade(key, id, &clicks, &trail).is_err());
    }
    #[test]
    fn rejects_instant_confirm() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (ax, ay) = answer_center(key, id);
        let clicks = vec![click(ax, ay, 1300.0), click(40.0, 220.0, 1310.0)];
        let trail = approach_trail(20.0, 20.0, ax, ay);
        assert!(s.grade(key, id, &clicks, &trail).is_err());
    }
    #[test]
    fn rejects_wrong_click_count() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let clicks = vec![click(0.0, 0.0, 0.0)];
        let trail = approach_trail(20.0, 20.0, 100.0, 100.0);
        assert!(s.grade(key, id, &clicks, &trail).is_err());
    }
    #[test]
    fn track_rejects_teleport() {
        let s = Slider;
        let key = b"test-key-0000000000000000000000";
        let id = "abc-123";
        let (ax, ay) = answer_center(key, id);
        let clicks = vec![click(ax, ay, 1300.0), click(40.0, 220.0, 1600.0)];
        let trail = vec![
            TrailPoint { x: 5.0, y: 5.0, t: 0.0 },
            TrailPoint { x: 6.0, y: 6.0, t: 50.0 },
            TrailPoint { x: 7.0, y: 6.0, t: 100.0 },
            TrailPoint { x: 8.0, y: 7.0, t: 150.0 },
            TrailPoint { x: ax, y: ay, t: 200.0 },
        ];
        assert!(s.track(key, id, &clicks, &trail).is_err());
    }
    #[test]
    fn distinct_hole_shapes() {
        let key = b"test-key-0000000000000000000000";
        let mut rng = derive_rng(key, "shape-check");
        let layout = build_layout(&mut rng);
        for i in 0..layout.holes.len() {
            for j in (i + 1)..layout.holes.len() {
                assert_ne!(layout.holes[i].knobs, layout.holes[j].knobs);
            }
        }
    }
}

