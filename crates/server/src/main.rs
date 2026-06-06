mod difficulty;
mod setup;
mod token;

use axum::{
    extract::{ConnectInfo, State},
    http::{header::CONTENT_TYPE, HeaderValue, Method},
    routing::post,
    Json, Router,
};
use std::{
    collections::HashMap,
    io::Cursor,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Challenge, ChallengeRequest, Click, Mover, Solution, VerifyResponse};
use difficulty::{difficulty_for, ClientProfile, FRAME_COUNT, FRAME_DT_MS};
use image::{ImageEncoder, RgbaImage};
use num_bigint::BigUint;
use rand::{Rng, RngCore};
use token::{now_secs, RedeemLog, TokenClaims, TOKEN_TTL_SECS};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;
use vdf::{VdfParams, VdfProof};

const MODULUS_BITS: u64 = 2048;
const TTL: Duration = Duration::from_secs(120);
const PUZZLE_W: u32 = 320;
const PUZZLE_H: u32 = 240;
const EXPECTED_CLICKS: usize = 3;
const TARGET_R: f64 = 16.0;
const HIT_R: f64 = 22.0;
const MIN_TARGET_SEP: f64 = 60.0;
const NOISE_COUNT: usize = 110;
const MIN_INTER_CLICK_MS: f64 = 40.0;
const MIN_TOTAL_MS: f64 = 250.0;
const MAX_TOTAL_MS: f64 = 15_000.0;
const ORBIT_AMP: f64 = 26.0;
const ORBIT_TURNS: f64 = 1.0;

struct Stored {
    seed_hex: String,
    difficulty: u64,
    born: Instant,
    created_ts: u64,
    movers: Vec<Mover>,
    issuer_ip: String,
}

struct AppState {
    store: Mutex<HashMap<String, Stored>>,
    profiles: Mutex<HashMap<String, ClientProfile>>,
    params: VdfParams,
    modulus_hex: String,
    token_key: Vec<u8>,
    challenge_key: Vec<u8>,
    redeem_log: Mutex<RedeemLog>,
}

#[derive(serde::Deserialize)]
struct ContentReq {
    token: String,
}

#[derive(serde::Serialize)]
struct ContentResp {
    ok: bool,
    content: Option<String>,
}

#[tokio::main]
async fn main() {
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let mut chal_key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut chal_key);
    println!("generating {}-bit trusted-setup modulus...", MODULUS_BITS);
    let gen_start = Instant::now();
    let setup = setup::generate(MODULUS_BITS);
    println!(
        "modulus ready: {} bits in {:?} (prime factors discarded)",
        setup.bits,
        gen_start.elapsed()
    );
    let state = Arc::new(AppState {
        store: Mutex::new(HashMap::new()),
        profiles: Mutex::new(HashMap::new()),
        params: VdfParams::from_modulus_hex(&setup.modulus_hex),
        modulus_hex: setup.modulus_hex,
        token_key: key,
        challenge_key: chal_key,
        redeem_log: Mutex::new(RedeemLog::new()),
    });
    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
            "http://localhost:5174".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5174".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE]);
    let mut app = Router::new()
        .route("/challenge", post(issue))
        .route("/verify", post(verify))
        .route("/content", post(content));
    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "frontend/dist".to_string());
    if std::path::Path::new(&static_dir).is_dir() {
        let index = format!("{static_dir}/index.html");
        app = app.fallback_service(
            ServeDir::new(&static_dir).not_found_service(ServeFile::new(index)),
        );
    }
    let app = app.layer(cors).with_state(state);
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("listening on http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

fn err(message: &str) -> Json<VerifyResponse> {
    Json(VerifyResponse {
        ok: false,
        token: None,
        message: message.into(),
    })
}

fn gen_movers() -> Vec<Mover> {
    let mut rng = rand::thread_rng();
    let mut movers: Vec<Mover> = Vec::with_capacity(EXPECTED_CLICKS);
    let margin = TARGET_R + ORBIT_AMP;
    let mut guard = 0;
    while movers.len() < EXPECTED_CLICKS && guard < 1000 {
        guard += 1;
        let x = rng.gen_range(margin..(PUZZLE_W as f64 - margin));
        let y = rng.gen_range(margin..(PUZZLE_H as f64 - margin));
        if movers
            .iter()
            .all(|m| ((m.x0 - x).powi(2) + (m.y0 - y).powi(2)).sqrt() >= MIN_TARGET_SEP)
        {
            let phase = rng.gen_range(0.0..std::f64::consts::TAU);
            let dir = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
            movers.push(Mover {
                x0: x,
                y0: y,
                vx: phase,
                vy: dir,
            });
        }
    }
    movers
}

fn pos_at(m: &Mover, t: f64) -> (f64, f64) {
    let t_total = FRAME_COUNT as f64 * FRAME_DT_MS;
    let theta = m.vx + m.vy * std::f64::consts::TAU * ORBIT_TURNS * t / t_total;
    (m.x0 + ORBIT_AMP * theta.cos(), m.y0 + ORBIT_AMP * theta.sin())
}

fn fill_circle(img: &mut RgbaImage, cx: f64, cy: f64, r: f64, color: [u8; 4]) {
    let r2 = r * r;
    let w = img.width() as f64;
    let h = img.height() as f64;
    let x0 = (cx - r).floor().max(0.0) as u32;
    let x1 = (cx + r).ceil().min(w) as u32;
    let y0 = (cy - r).floor().max(0.0) as u32;
    let y1 = (cy + r).ceil().min(h) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            if dx * dx + dy * dy <= r2 {
                img.put_pixel(x, y, image::Rgba(color));
            }
        }
    }
}

fn render_frames(movers: &[Mover]) -> String {
    let mut rng = rand::thread_rng();
    let cols = FRAME_COUNT;
    let strip_w = PUZZLE_W * cols;
    let mut img = RgbaImage::from_pixel(strip_w, PUZZLE_H, image::Rgba([0, 16, 61, 255]));
    let decoys: Vec<(f64, f64)> = (0..NOISE_COUNT)
        .map(|_| {
            (
                rng.gen_range(0.0..PUZZLE_W as f64),
                rng.gen_range(0.0..PUZZLE_H as f64),
            )
        })
        .collect();
    for f in 0..cols {
        let t = f as f64 * FRAME_DT_MS;
        let ox = f * PUZZLE_W;
        for &(dx, dy) in &decoys {
            fill_circle(&mut img, ox as f64 + dx, dy, 2.0, [255, 94, 140, 255]);
        }
        for m in movers {
            let (x, y) = pos_at(m, t);
            fill_circle(&mut img, ox as f64 + x, y, TARGET_R, [22, 151, 249, 255]);
            fill_circle(&mut img, ox as f64 + x, y, TARGET_R * 0.45, [237, 255, 253, 255]);
        }
    }
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(Cursor::new(&mut buf))
        .write_image(img.as_raw(), strip_w, PUZZLE_H, image::ExtendedColorType::Rgba8)
        .unwrap();
    B64.encode(&buf)
}

fn validate_clicks(clicks: &[Click]) -> Result<(), &'static str> {
    if clicks.len() != EXPECTED_CLICKS {
        return Err("wrong click count");
    }
    let mut last_t = -1.0_f64;
    for c in clicks {
        if !c.x.is_finite() || !c.y.is_finite() || !c.t.is_finite() {
            return Err("non-finite click");
        }
        if c.x < 0.0 || c.x > PUZZLE_W as f64 || c.y < 0.0 || c.y > PUZZLE_H as f64 {
            return Err("click out of bounds");
        }
        if c.t <= last_t {
            return Err("non-monotonic clicks");
        }
        if last_t >= 0.0 && (c.t - last_t) < MIN_INTER_CLICK_MS {
            return Err("clicks too fast");
        }
        last_t = c.t;
    }
    let total = clicks.last().unwrap().t - clicks.first().unwrap().t;
    if total < MIN_TOTAL_MS || total > MAX_TOTAL_MS {
        return Err("solve time implausible");
    }
    Ok(())
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
            let (tx, ty) = pos_at(m, t_seen);
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

async fn issue(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(s): State<Arc<AppState>>,
    Json(_req): Json<ChallengeRequest>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    let ip = addr.ip().to_string();
    let now = Instant::now();
    let difficulty = {
        let mut profiles = s.profiles.lock().await;
        let profile = profiles
            .entry(ip.clone())
            .or_insert_with(|| ClientProfile::new(now));
        profile.roll_window(now);
        if !profile.register_request() {
            return Err(err("rate limited"));
        }
        difficulty_for(profile)
    };
    let id = Uuid::new_v4().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let seed_hex = hex::encode(seed);
    let movers = gen_movers();
    let frames_b64 = render_frames(&movers);
    let created_ts = now_secs();
    {
        let mut store = s.store.lock().await;
        store.retain(|_, v| v.born.elapsed() <= TTL);
        store.insert(
            id.clone(),
            Stored {
                seed_hex: seed_hex.clone(),
                difficulty,
                born: now,
                created_ts,
                movers: movers.clone(),
                issuer_ip: ip.clone(),
            },
        );
    }
    let canonical = format!("{}|{}|{}|{}", id, seed_hex, difficulty, ip);
    let sig = token::sign_blob(&s.challenge_key, &canonical);
    Ok(Json(Challenge {
        id,
        seed_hex,
        modulus_hex: s.modulus_hex.clone(),
        difficulty,
        frames_b64,
        frame_count: FRAME_COUNT,
        frame_dt_ms: FRAME_DT_MS,
        puzzle_w: PUZZLE_W as f64,
        puzzle_h: PUZZLE_H as f64,
        sig,
    }))
}

async fn content(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(s): State<Arc<AppState>>,
    Json(req): Json<ContentReq>,
) -> Json<ContentResp> {
    let ip = addr.ip().to_string();
    {
        let now = Instant::now();
        let mut profiles = s.profiles.lock().await;
        let profile = profiles
            .entry(ip)
            .or_insert_with(|| ClientProfile::new(now));
        profile.roll_window(now);
        if !profile.register_verify(now) {
            return Json(ContentResp {
                ok: false,
                content: None,
            });
        }
    }
    let Some(claims) = token::verify(&s.token_key, &req.token) else {
        return Json(ContentResp { ok: false, content: None });
    };
    let _ = claims;
    {
        let mut log = s.redeem_log.lock().await;
        if !log.try_consume(&req.token) {
            return Json(ContentResp { ok: false, content: None });
        }
    }
    Json(ContentResp {
        ok: true,
        content: Some("unlocked".into()),
    })
}

async fn verify(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(s): State<Arc<AppState>>,
    Json(sol): Json<Solution>,
) -> Json<VerifyResponse> {
    let ip = addr.ip().to_string();
    if sol.output_hex.len() > 4096 || sol.proof_hex.len() > 4096 {
        return err("input too large");
    }
    {
        let now = Instant::now();
        let mut profiles = s.profiles.lock().await;
        let profile = profiles
            .entry(ip.clone())
            .or_insert_with(|| ClientProfile::new(now));
        profile.roll_window(now);
        if !profile.register_verify(now) {
            return err("rate limited");
        }
    }
    let (seed, difficulty, movers, created_ts, issuer_ip) = {
        let mut store = s.store.lock().await;
        let stored = match store.get(&sol.challenge_id) {
            Some(c) => c,
            None => return err("unknown challenge"),
        };
        if stored.born.elapsed() > TTL {
            store.remove(&sol.challenge_id);
            return err("challenge expired");
        }
        (
            hex::decode(&stored.seed_hex).unwrap(),
            stored.difficulty,
            stored.movers.clone(),
            stored.created_ts,
            stored.issuer_ip.clone(),
        )
    };
    if issuer_ip != ip {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.5).await;
        return err("challenge bound to another client");
    }
    let canonical = format!(
        "{}|{}|{}|{}",
        sol.challenge_id,
        hex::encode(&seed),
        difficulty,
        issuer_ip
    );
    if !token::verify_blob(&s.challenge_key, &canonical, &sol.sig) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.5).await;
        return err("invalid challenge signature");
    }
    if let Err(m) = validate_clicks(&sol.clicks) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.3).await;
        return err(m);
    }
    let trail: Vec<(f64, f64, f64)> = sol.trail.iter().map(|p| (p.x, p.y, p.t)).collect();
    match difficulty::grade_trail(&trail) {
        Ok(weight) => {
            if weight > 0.0 {
                flag_profile(&s, &ip, weight).await;
            }
        }
        Err(m) => {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.4).await;
            return err(m);
        }
    }
    if let Some(weight) = difficulty::classify_timing(
        sol.clicks.last().map(|c| c.t).unwrap_or(0.0)
            - sol.clicks.first().map(|c| c.t).unwrap_or(0.0),
        sol.clicks.len(),
    ) {
        flag_profile(&s, &ip, weight).await;
    }
    if let Err(m) = grade_clicks(&movers, &sol.clicks) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.3).await;
        return err(m);
    }
    let mut trajectory = String::new();
    for c in &sol.clicks {
        trajectory.push_str(&format!("{},{},{};", c.x as i64, c.y as i64, c.t as i64));
    }
    let x = s
        .params
        .challenge_element_with_trajectory(&seed, trajectory.as_bytes());
    let output = match BigUint::parse_bytes(sol.output_hex.as_bytes(), 16) {
        Some(v) => v,
        None => return err("invalid output hex"),
    };
    let proof_val = match BigUint::parse_bytes(sol.proof_hex.as_bytes(), 16) {
        Some(v) => v,
        None => return err("invalid proof hex"),
    };
    let vdf_proof = VdfProof {
        output,
        proof: proof_val,
    };
    let ok = s.params.verify(&x, difficulty, &vdf_proof);
    if ok {
        s.store.lock().await.remove(&sol.challenge_id);
        record_profile_success(&s, &ip).await;
        let iat = now_secs();
        let claims = TokenClaims {
            challenge_id: sol.challenge_id.clone(),
            site_key: String::new(),
            hostname: String::new(),
            issued_at: created_ts.max(iat.saturating_sub(TTL.as_secs())),
            expires_at: iat + TOKEN_TTL_SECS,
        };
        let signed = token::sign(&s.token_key, &claims);
        Json(VerifyResponse {
            ok: true,
            token: Some(signed),
            message: "verified".into(),
        })
    } else {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.2).await;
        Json(VerifyResponse {
            ok: false,
            token: None,
            message: "proof invalid".into(),
        })
    }
}

async fn flag_profile(s: &Arc<AppState>, ip: &str, weight: f64) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock().await;
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.flag_anomaly(weight);
}

async fn record_profile_success(s: &Arc<AppState>, ip: &str) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock().await;
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.record_success();
}

async fn record_profile_failure(s: &Arc<AppState>, ip: &str) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock().await;
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.record_failure();
}