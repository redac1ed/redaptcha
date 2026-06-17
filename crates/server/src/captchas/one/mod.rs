use std::io::Cursor;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Click, Mover};
use image::{ImageEncoder, RgbaImage};
use rayon::prelude::*;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

use crate::captcha::{Captcha, Rendered};
use crate::difficulty::{FRAME_COUNT, FRAME_DT_MS};
use crate::token;

pub const PUZZLE_W: u32 = 320;
pub const PUZZLE_H: u32 = 240;
pub const EXPECTED_CLICKS: usize = 3;
const TARGET_R: f64 = 16.0;
const HIT_R: f64 = 18.0;
const ORBIT_AMP_MIN: f64 = 24.0;
const ORBIT_AMP_MAX: f64 = 52.0;
const ORBIT_TURNS_MIN: f64 = 0.5;
const ORBIT_TURNS_MAX: f64 = 1.0;
const N_DECOYS: usize = 5;
const DECOY_R: f64 = 11.0;

pub struct MovingBall;

impl Captcha for MovingBall {
    fn kind(&self) -> &'static str { "one" }
    fn expected_clicks(&self) -> usize { EXPECTED_CLICKS }
    fn puzzle_w(&self) -> f64 { PUZZLE_W as f64 }
    fn puzzle_h(&self) -> f64 { PUZZLE_H as f64 }
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered {
        let movers = gen_movers_from(challenge_key, challenge_id);
        let mut rseed = [0u8; 32];
        token::derive_bytes(challenge_key, &format!("render|{challenge_id}"), &mut rseed);
        let (ring, core) = derive_colors(&rseed);
        let decoys = gen_decoys(&rseed);
        let frames_b64 = movers.par_iter()
            .map(|m| render_panel(m, ring, core, &decoys))
            .collect();
        Rendered { frames_b64, slider: None }
    }
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click], _trail: &[core_types::TrailPoint],) -> Result<(), &'static str> {
        let movers = gen_movers_from(challenge_key, challenge_id);
        grade_clicks(&movers, clicks)
    }
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
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

fn derive_colors(seed: &[u8]) -> ([u8; 4], [u8; 4]) {
    let hue = seed[16] as f64 / 255.0 * 340.0 + 10.0;
    let [r, g, b] = hsl_to_rgb(hue, 0.82, 0.54);
    let [cr, cg, cb] = hsl_to_rgb(hue, 0.20, 0.91);
    ([r, g, b, 255], [cr, cg, cb, 255])
}

struct Decoy {
    x0: f64,
    y0: f64,
    vx: f64,
    vy: f64,
    amp: f64,
    turns: f64,
    color: [u8; 4],
}

fn gen_decoys(seed: &[u8]) -> Vec<Decoy> {
    let mut st: u64 = u64::from_le_bytes(seed[16..24].try_into().unwrap()) | 1;
    let mut next = move || -> f64 {
        st ^= st << 13; st ^= st >> 7; st ^= st << 17;
        (st >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut out = Vec::with_capacity(N_DECOYS);
    for _ in 0..N_DECOYS {
        let hue = next() * 360.0;
        let [r, g, b] = hsl_to_rgb(hue, 0.75, 0.50);
        let amp = ORBIT_AMP_MIN * 0.6 + next() * (ORBIT_AMP_MAX - ORBIT_AMP_MIN) * 0.6;
        let margin = DECOY_R + amp * 1.4 + 4.0;
        let x = margin + next() * (PUZZLE_W as f64 - 2.0 * margin);
        let y = margin + next() * (PUZZLE_H as f64 - 2.0 * margin);
        out.push(Decoy {
            x0: x, y0: y,
            vx: next() * std::f64::consts::TAU,
            vy: if next() < 0.5 { 1.0 } else { -1.0 },
            amp,
            turns: ORBIT_TURNS_MIN + next() * (ORBIT_TURNS_MAX - ORBIT_TURNS_MIN),
            color: [r, g, b, 255],
        });
    }
    out
}

fn decoy_pos(d: &Decoy, t: f64) -> (f64, f64) {
    let t_total = FRAME_COUNT as f64 * FRAME_DT_MS;
    let w = d.vy * std::f64::consts::TAU * d.turns * t / t_total;
    let px = d.x0 + d.amp * (d.vx + w).sin() + d.amp * 0.4 * (d.vx + 2.3 * w).sin();
    let py = d.y0 + d.amp * (d.vx * 1.7 + 1.3 * w).sin() + d.amp * 0.4 * (d.vx + 1.9 * w).cos();
    (px, py)
}

fn gen_movers_from(key: &[u8], challenge_id: &str) -> Vec<Mover> {
    let mut seed = [0u8; 32];
    token::derive_bytes(key, &format!("movers|{challenge_id}"), &mut seed);
    let mut st: u64 = u64::from_le_bytes(seed[..8].try_into().unwrap()) | 1;
    let mut next = || {
        st ^= st << 13; st ^= st >> 7; st ^= st << 17;
        (st >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut movers: Vec<Mover> = Vec::with_capacity(EXPECTED_CLICKS);
    let mut guard = 0;
    while movers.len() < EXPECTED_CLICKS && guard < 1000 {
        guard += 1;
        let amp = ORBIT_AMP_MIN + next() * (ORBIT_AMP_MAX - ORBIT_AMP_MIN);
        let margin = TARGET_R + amp * 1.4 + 4.0;
        let x = margin + next() * (PUZZLE_W as f64 - 2.0 * margin);
        let y = margin + next() * (PUZZLE_H as f64 - 2.0 * margin);
        let phase = next() * std::f64::consts::TAU;
        let dir = if next() < 0.5 { 1.0 } else { -1.0 };
        let turns = ORBIT_TURNS_MIN + next() * (ORBIT_TURNS_MAX - ORBIT_TURNS_MIN);
        movers.push(Mover { x0: x, y0: y, vx: phase, vy: dir, amp, turns });
    }
    movers
}

fn fill_circle(img: &mut RgbaImage, cx: f64, cy: f64, r: f64, color: [u8; 4]) {
    let r2 = r * r;
    let iw = img.width();
    let ih = img.height();
    let x0 = (cx - r).floor().max(0.0) as u32;
    let x1 = ((cx + r).ceil() as i64).clamp(0, iw as i64) as u32;
    let y0 = (cy - r).floor().max(0.0) as u32;
    let y1 = ((cy + r).ceil() as i64).clamp(0, ih as i64) as u32;
    let buf = img.as_mut();
    for y in y0..y1 {
        let row = (y as usize) * (iw as usize) * 4;
        let dy = y as f64 + 0.5 - cy;
        let dy2 = dy * dy;
        for x in x0..x1 {
            let dx = x as f64 + 0.5 - cx;
            if dx * dx + dy2 <= r2 {
                let i = row + (x as usize) * 4;
                buf[i] = color[0];
                buf[i + 1] = color[1];
                buf[i + 2] = color[2];
                buf[i + 3] = color[3];
            }
        }
    }
}

fn pos_at_center(m: &Mover, t: f64) -> (f64, f64) {
    let t_total = FRAME_COUNT as f64 * FRAME_DT_MS;
    let w = m.vy * std::f64::consts::TAU * m.turns * t / t_total;
    let px = m.x0 + m.amp * (m.vx + w).sin() + m.amp * 0.4 * (m.vx + 2.3 * w).sin();
    let py = m.y0 + m.amp * (m.vx * 1.7 + 1.3 * w).sin() + m.amp * 0.4 * (m.vx + 1.9 * w).cos();
    (px, py)
}

fn render_panel(m: &Mover, ring: [u8; 4], core: [u8; 4], decoys: &[Decoy]) -> String {
    let cols = FRAME_COUNT;
    let strip_w = PUZZLE_W * cols;
    let mut img = RgbaImage::from_pixel(strip_w, PUZZLE_H, image::Rgba([4, 8, 36, 255]));
    for f in 0..cols {
        let t = f as f64 * FRAME_DT_MS;
        let ox = f * PUZZLE_W;
        for d in decoys {
            let (dx, dy) = decoy_pos(d, t);
            fill_circle(&mut img, ox as f64 + dx, dy, DECOY_R, d.color);
        }
        let (x, y) = pos_at_center(m, t);
        fill_circle(&mut img, ox as f64 + x, y, TARGET_R, ring);
        fill_circle(&mut img, ox as f64 + x, y, TARGET_R * 0.42, core);
    }
    let mut buf = Vec::new();
    PngEncoder::new_with_quality(Cursor::new(&mut buf), CompressionType::Fast, FilterType::Up)
        .write_image(img.as_raw(), strip_w, PUZZLE_H, image::ExtendedColorType::Rgba8)
        .unwrap();
    B64.encode(&buf)
}


fn grade_clicks(movers: &[Mover], clicks: &[Click]) -> Result<(), &'static str> {
    let mut used = vec![false; movers.len()];
    for c in clicks {
        let frame = (c.t / FRAME_DT_MS).floor() as i64;
        let seen_frame = frame.rem_euclid(FRAME_COUNT as i64) as f64;
        let t_seen = seen_frame * FRAME_DT_MS;
        let mut matched = None;
        for (i, m) in movers.iter().enumerate() {
            if used[i] {
                continue;
            }
            let (tx, ty) = pos_at_center(m, t_seen);
            let d = ((c.x - tx).powi(2) + (c.y - ty).powi(2)).sqrt();
            if d <= HIT_R {
                matched = Some(i);
                break;
            }
        }
        match matched {
            Some(i) => used[i] = true,
            None => return Err("puzzle not solved"),
        }
    }
    if used.iter().all(|&u| u) {
        Ok(())
    } else {
        Err("puzzle not solved")
    }
}