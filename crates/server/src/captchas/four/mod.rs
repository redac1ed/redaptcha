use std::io::Cursor;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Click, TrailPoint};
use image::{ImageEncoder, RgbaImage};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

use crate::captcha::{Captcha, Rendered};
use crate::difficulty::{FRAME_COUNT, FRAME_DT_MS};
use crate::token;

pub const PUZZLE_W: u32 = 320;
pub const PUZZLE_H: u32 = 240;
const TARGET_R: f64 = 14.0;
const N_DECOYS: usize = 3;
const DECOY_R: f64 = 10.0;
const MIN_TRAIL_POINTS: usize = 24;
const TRACK_MIN_MS: f64 = 2200.0;
const SAMPLE_DT_MS: f64 = 40.0;
const MIN_SAMPLES: usize = 12;
const MEAN_ERR_MAX: f64 = 46.0;
const MEAN_ERR_MIN: f64 = 4.0;
const TRACK_BAND_PX: f64 = 60.0;
const MIN_GOOD_FRAC: f64 = 0.55;
const MAX_TELEPORT_FRAC: f64 = 0.6;
const LAG_MIN_MS: f64 = 40.0;
const LAG_MAX_MS: f64 = 420.0;
const MIN_VEL_CORR: f64 = 0.15;

struct Path {
    ax: [f64; 2],
    ay: [f64; 2],
    phase: [f64; 4],
    k: [f64; 4],
    dir: f64,
}

struct Decoy {
    cx: f64,
    cy: f64,
    ax: f64,
    ay: f64,
    phase: f64,
    k: f64,
    dir: f64,
    color: [u8; 4],
}

pub struct Pursuit;

impl Captcha for Pursuit {
    fn kind(&self) -> &'static str { "four" }
    fn expected_clicks(&self) -> usize { 0 }
    fn rounds(&self) -> u32 { 1 }
    fn puzzle_w(&self) -> f64 { PUZZLE_W as f64 }
    fn puzzle_h(&self) -> f64 { PUZZLE_H as f64 }
    fn generate(&self, challenge_key: &[u8], challenge_id: &str) -> Rendered {
        let path = gen_path(challenge_key, challenge_id);
        let mut rseed = [0u8; 32];
        token::derive_bytes(challenge_key, &format!("pursuit-render|{challenge_id}"), &mut rseed);
        let decoys = gen_decoys(&rseed);
        let noise = bg_noise(&rseed);
        let strip = render_strip(&path, &decoys, &noise);
        Rendered { frames_b64: vec![strip], slider: None }
    }
    fn grade(&self, challenge_key: &[u8], challenge_id: &str, clicks: &[Click], trail: &[TrailPoint]) -> Result<(), &'static str> {
        if !clicks.is_empty() {
            return Err("unexpected interaction");
        }
        let path = gen_path(challenge_key, challenge_id);
        grade_pursuit(&path, trail)
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

fn rng_from(seed: &[u8], lo: usize) -> impl FnMut() -> f64 {
    let mut st: u64 = u64::from_le_bytes(seed[lo..lo + 8].try_into().unwrap()) | 1;
    move || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        (st >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn gen_path(key: &[u8], challenge_id: &str) -> Path {
    let mut seed = [0u8; 32];
    token::derive_bytes(key, &format!("pursuit|{challenge_id}"), &mut seed);
    let mut next = rng_from(&seed, 0);
    let tau = std::f64::consts::TAU;
    let kset = [1.0, 2.0, 3.0];
    Path {
        ax: [50.0 + next() * 25.0, 15.0 + next() * 15.0],
        ay: [38.0 + next() * 20.0, 12.0 + next() * 12.0],
        phase: [next() * tau, next() * tau, next() * tau, next() * tau],
        k: [
            kset[(next() * 3.0) as usize % 3],
            kset[(next() * 3.0) as usize % 3],
            kset[(next() * 3.0) as usize % 3],
            kset[(next() * 3.0) as usize % 3],
        ],
        dir: if next() < 0.5 { 1.0 } else { -1.0 },
    }
}

fn pos_at(p: &Path, t: f64) -> (f64, f64) {
    let t_total = FRAME_COUNT as f64 * FRAME_DT_MS;
    let w = p.dir * std::f64::consts::TAU * t / t_total;
    let x = PUZZLE_W as f64 / 2.0
        + p.ax[0] * (p.phase[0] + p.k[0] * w).sin()
        + p.ax[1] * (p.phase[1] + p.k[1] * w).sin();
    let y = PUZZLE_H as f64 / 2.0
        + p.ay[0] * (p.phase[2] + p.k[2] * w).sin()
        + p.ay[1] * (p.phase[3] + p.k[3] * w).cos();
    (x, y)
}

fn target_seen(p: &Path, t: f64) -> (f64, f64) {
    let frame = (t / FRAME_DT_MS).floor() as i64;
    let seen = frame.rem_euclid(FRAME_COUNT as i64) as f64;
    pos_at(p, seen * FRAME_DT_MS)
}

fn gen_decoys(seed: &[u8]) -> Vec<Decoy> {
    let mut next = rng_from(seed, 8);
    let tau = std::f64::consts::TAU;
    let mut out = Vec::with_capacity(N_DECOYS);
    for _ in 0..N_DECOYS {
        let margin = 40.0;
        out.push(Decoy {
            cx: margin + next() * (PUZZLE_W as f64 - 2.0 * margin),
            cy: margin + next() * (PUZZLE_H as f64 - 2.0 * margin),
            ax: 14.0 + next() * 20.0,
            ay: 12.0 + next() * 18.0,
            phase: next() * tau,
            k: 1.0 + (next() * 2.0).floor(),
            dir: if next() < 0.5 { 1.0 } else { -1.0 },
            color: {
                let [r, g, b] = hsl_to_rgb(next() * 360.0, 0.30, 0.40);
                [r, g, b, 255]
            },
        });
    }
    out
}

fn decoy_pos(d: &Decoy, t: f64) -> (f64, f64) {
    let t_total = FRAME_COUNT as f64 * FRAME_DT_MS;
    let w = d.dir * std::f64::consts::TAU * d.k * t / t_total;
    (d.cx + d.ax * (d.phase + w).sin(), d.cy + d.ay * (d.phase * 1.3 + w).cos())
}

fn bg_noise(seed: &[u8]) -> [u8; 96] {
    let mut st: u64 = u64::from_le_bytes(seed[16..24].try_into().unwrap()) | 1;
    let mut out = [0u8; 96];
    for v in out.iter_mut() {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        *v = (st >> 24) as u8;
    }
    out
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

fn paint_background(img: &mut RgbaImage, noise: &[u8; 96]) {
    let iw = img.width() as usize;
    let ih = img.height() as usize;
    let buf = img.as_mut();
    let cell = 24usize;
    for y in 0..ih {
        let row = y * iw * 4;
        for x in 0..iw {
            let gx = (x / cell) & 7;
            let gy = (y / cell) & 7;
            let n = noise[((gy * 8 + gx) + ((x ^ y) & 1)) % 96] as i32;
            let i = row + x * 4;
            buf[i] = (buf[i] as i32 + ((n % 17) - 8)).clamp(0, 60) as u8;
            buf[i + 1] = (buf[i + 1] as i32 + ((n % 13) - 6)).clamp(0, 70) as u8;
            buf[i + 2] = (buf[i + 2] as i32 + ((n % 23) - 11)).clamp(20, 110) as u8;
        }
    }
}

fn render_strip(path: &Path, decoys: &[Decoy], noise: &[u8; 96]) -> String {
    let cols = FRAME_COUNT;
    let strip_w = PUZZLE_W * cols;
    let mut img = RgbaImage::from_pixel(strip_w, PUZZLE_H, image::Rgba([4, 8, 36, 255]));
    paint_background(&mut img, noise);
    let ring = [34, 211, 238, 255];
    let core = [237, 255, 253, 255];
    for f in 0..cols {
        let t = f as f64 * FRAME_DT_MS;
        let ox = (f * PUZZLE_W) as f64;
        for d in decoys {
            let (dx, dy) = decoy_pos(d, t);
            fill_circle(&mut img, ox + dx, dy, DECOY_R, d.color);
        }
        let (x, y) = pos_at(path, t);
        fill_circle(&mut img, ox + x, y, TARGET_R, ring);
        fill_circle(&mut img, ox + x, y, TARGET_R * 0.45, core);
    }
    let mut buf = Vec::new();
    PngEncoder::new_with_quality(Cursor::new(&mut buf), CompressionType::Fast, FilterType::Up)
        .write_image(img.as_raw(), strip_w, PUZZLE_H, image::ExtendedColorType::Rgba8)
        .unwrap();
    B64.encode(&buf)
}

fn interp(pts: &[(f64, f64, f64)], t: f64) -> (f64, f64) {
    if t <= pts[0].2 {
        return (pts[0].0, pts[0].1);
    }
    let last = pts[pts.len() - 1];
    if t >= last.2 {
        return (last.0, last.1);
    }
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        if t >= a.2 && t <= b.2 {
            let span = b.2 - a.2;
            if span <= 0.0 {
                return (a.0, a.1);
            }
            let r = (t - a.2) / span;
            return (a.0 + (b.0 - a.0) * r, a.1 + (b.1 - a.1) * r);
        }
    }
    (last.0, last.1)
}

fn vels(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(pts.len().saturating_sub(1));
    for w in pts.windows(2) {
        out.push((w[1].0 - w[0].0, w[1].1 - w[0].1));
    }
    out
}

fn best_lag(cv: &[(f64, f64)], tv: &[(f64, f64)]) -> (usize, f64) {
    let maxlag = (LAG_MAX_MS / SAMPLE_DT_MS).ceil() as usize;
    let mut best = (0usize, -1.0f64);
    for lag in 0..=maxlag {
        if lag >= cv.len() {
            break;
        }
        let mut dot = 0.0;
        let mut ncv = 0.0;
        let mut ntv = 0.0;
        for i in lag..cv.len() {
            let (ax, ay) = cv[i];
            let (bx, by) = tv[i - lag];
            dot += ax * bx + ay * by;
            ncv += ax * ax + ay * ay;
            ntv += bx * bx + by * by;
        }
        if ncv > 0.0 && ntv > 0.0 {
            let c = dot / (ncv.sqrt() * ntv.sqrt());
            if c > best.1 {
                best = (lag, c);
            }
        }
    }
    best
}

fn grade_pursuit(path: &Path, trail: &[TrailPoint]) -> Result<(), &'static str> {
    if trail.len() < MIN_TRAIL_POINTS {
        return Err("not enough movement");
    }
    let mut pts: Vec<(f64, f64, f64)> = trail.iter().map(|p| (p.x, p.y, p.t)).collect();
    pts.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let span = pts[pts.len() - 1].2 - pts[0].2;
    if span < TRACK_MIN_MS {
        return Err("tracking too brief");
    }
    let mut path_len = 0.0;
    let mut max_step = 0.0;
    for w in pts.windows(2) {
        let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        path_len += d;
        if d > max_step {
            max_step = d;
        }
    }
    if path_len <= 0.0 || max_step / path_len > MAX_TELEPORT_FRAC {
        return Err("teleport detected");
    }
    let n = (span / SAMPLE_DT_MS).floor() as usize;
    if n < MIN_SAMPLES {
        return Err("tracking too brief");
    }
    let t0 = pts[0].2;
    let mut cur = Vec::with_capacity(n + 1);
    let mut tgt = Vec::with_capacity(n + 1);
    let mut errs = Vec::with_capacity(n + 1);
    let mut good = 0usize;
    for i in 0..=n {
        let t = t0 + i as f64 * SAMPLE_DT_MS;
        let (cx, cy) = interp(&pts, t);
        let (tx, ty) = target_seen(path, t);
        let e = ((cx - tx).powi(2) + (cy - ty).powi(2)).sqrt();
        if e < TRACK_BAND_PX {
            good += 1;
        }
        cur.push((cx, cy));
        tgt.push((tx, ty));
        errs.push(e);
    }
    let mean_err = errs.iter().sum::<f64>() / errs.len() as f64;
    let good_frac = good as f64 / errs.len() as f64;
    if good_frac < MIN_GOOD_FRAC || mean_err > MEAN_ERR_MAX {
        return Err("not following target");
    }
    if mean_err < MEAN_ERR_MIN {
        return Err("inhuman precision");
    }
    let cv = vels(&cur);
    let tv = vels(&tgt);
    let (lag, corr) = best_lag(&cv, &tv);
    if corr < MIN_VEL_CORR {
        return Err("uncorrelated motion");
    }
    let lag_ms = lag as f64 * SAMPLE_DT_MS;
    if lag_ms < LAG_MIN_MS || lag_ms > LAG_MAX_MS {
        return Err("inhuman tracking latency");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synth_trail(path: &Path, lag_ms: f64, noise: f64, dur: f64) -> Vec<TrailPoint> {
        let mut out = Vec::new();
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        let mut t = 0.0;
        while t <= dur {
            let (x, y) = target_seen(path, (t - lag_ms).max(0.0));
            out.push(TrailPoint { x: x + rnd() * noise, y: y + rnd() * noise, t });
            t += 16.0;
        }
        out
    }
    #[test]
    fn path_is_deterministic() {
        let a = gen_path(b"k", "id-1");
        let b = gen_path(b"k", "id-1");
        assert_eq!(pos_at(&a, 137.0), pos_at(&b, 137.0));
    }
    #[test]
    fn rejects_clicks() {
        let clicks = vec![Click { x: 1.0, y: 2.0, t: 3.0 }];
        assert!(Pursuit.grade(b"k", "id", &clicks, &[]).is_err());
    }
    #[test]
    fn rejects_empty_trail() {
        assert!(Pursuit.grade(b"k", "id", &[], &[]).is_err());
    }
    #[test]
    fn accepts_human_like_tracking() {
        let path = gen_path(b"k", "id-7");
        let trail = synth_trail(&path, 180.0, 9.0, 3400.0);
        assert!(grade_pursuit(&path, &trail).is_ok());
    }
    #[test]
    fn rejects_perfect_zero_lag_tracking() {
        let path = gen_path(b"k", "id-7");
        let trail = synth_trail(&path, 0.0, 0.0, 3400.0);
        assert!(grade_pursuit(&path, &trail).is_err());
    }
}