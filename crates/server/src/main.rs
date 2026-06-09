mod captcha;
mod captchas;
mod difficulty;
mod setup;
mod token;
mod store;

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, State},
    http::{header::CONTENT_TYPE, HeaderValue, Method},
    routing::post,
    Json, Router,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Challenge, ChallengeRequest, Solution, VerifyResponse};
use difficulty::{difficulty_for, ClientProfile, GlobalLimiter, FRAME_COUNT, FRAME_DT_MS};
use num_bigint::BigUint;
use rand::{RngCore};
use token::{now_secs, TokenClaims, TOKEN_TTL_SECS};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;
use vdf::{VdfParams, VdfProof};

const MODULUS_BITS: u64 = 2048;
const TTL: Duration = Duration::from_secs(120);

struct Stored {
    kind: String,
    seed_hex: String,
    difficulty: u64,
    born: Instant,
    created_ts: u64,
    issuer_ip: String,
}

struct AppState {
    store: Mutex<HashMap<String, Stored>>,
    profiles: Mutex<HashMap<String, ClientProfile>>,
    params: VdfParams,
    modulus_hex: String,
    token_key: Vec<u8>,
    challenge_key: Vec<u8>,
    redeem: store::RedeemStore,
    global: Mutex<GlobalLimiter>,
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
    let _ = dotenvy::from_filename(".env.local");
    let key = load_or_gen_key("REDAPTCHA_TOKEN_KEY");
    let chal_key = load_or_gen_key("REDAPTCHA_CHALLENGE_KEY");
    let modulus_hex = match std::env::var("REDAPTCHA_MODULUS_HEX") {
        Ok(h) if !h.trim().is_empty() => {
            println!("using persisted modulus from env ({} hex chars)", h.trim().len());
            h.trim().to_string()
        }
        _ => {
            println!("generating {}-bit trusted-setup modulus...", MODULUS_BITS);
            let gen_start = Instant::now();
            let setup = setup::generate(MODULUS_BITS);
            println!(
                "modulus ready: {} bits in {:?} (prime factors discarded)",
                setup.bits,
                gen_start.elapsed()
            );
            println!("REDAPTCHA_MODULUS_HEX={}  <- set this in your deploy config to persist across restarts", setup.modulus_hex);
            setup.modulus_hex
        }
    };
    let redeem = store::RedeemStore::from_env().await;
    let state = Arc::new(AppState {
        store: Mutex::new(HashMap::new()),
        profiles: Mutex::new(HashMap::new()),
        params: VdfParams::from_modulus_hex(&modulus_hex),
        modulus_hex,
        token_key: key,
        challenge_key: chal_key,
        redeem,
        global: Mutex::new(GlobalLimiter::new(Instant::now())),
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
        .route("/challenge", post(issue_default))
        .route("/challenge/:kind", post(issue))
        .route("/verify", post(verify))
        .route("/content", post(content));
    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "frontend/dist".to_string());
    if std::path::Path::new(&static_dir).is_dir() {
        let index = format!("{static_dir}/index.html");
        app = app.fallback_service(
            ServeDir::new(&static_dir).not_found_service(ServeFile::new(index)),
        );
    }
    let app = app
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(cors)
        .with_state(state);
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    println!("listening on http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("server error");
}

fn err(message: &str) -> Json<VerifyResponse> {
    Json(VerifyResponse {
        ok: false,
        token: None,
        message: message.into(),
    })
}

fn load_or_gen_key(var: &str) -> Vec<u8> {
    if let Ok(b64) = std::env::var(var) {
        if let Ok(bytes) = B64.decode(b64.trim()) {
            if bytes.len() == 32 {
                return bytes;
            }
        }
        eprintln!("warning: {var} set but invalid (need base64 of 32 bytes); generating ephemeral key");
    }
    let mut k = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    println!("{var}={}  <- set this in your deploy config to persist across restarts", B64.encode(&k));
    k
}

async fn issue_default(
    info: ConnectInfo<SocketAddr>,
    state: State<Arc<AppState>>,
    req: Json<ChallengeRequest>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    issue(info, Path(captchas::DEFAULT_KIND.to_string()), state, req).await
}

async fn issue(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(kind): Path<String>,
    State(s): State<Arc<AppState>>,
    Json(_req): Json<ChallengeRequest>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    let ip = addr.ip().to_string();
    let now = Instant::now();
    let puzzle = match captchas::by_kind(&kind) {
        Some(p) => p,
        None => return Err(err("unknown captcha kind")),
    };
    if !s.global.lock().await.allow(now) {
        return Err(err("server busy"));
    }
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
    let rendered = puzzle.generate(&s.challenge_key, &id);
    let frames_b64 = rendered.frames_b64;
    let motions = rendered.motions;
    let created_ts = now_secs();
    {
        let mut store = s.store.lock().await;
        store.retain(|_, v| v.born.elapsed() <= TTL);
        store.insert(
            id.clone(),
            Stored {
                kind: kind.clone(),
                seed_hex: seed_hex.clone(),
                difficulty,
                born: now,
                created_ts,
                issuer_ip: ip.clone(),
            },
        );
    }
    let canonical = format!("{}|{}|{}|{}|{}", id, kind, seed_hex, difficulty, ip);
    let sig = token::sign_blob(&s.challenge_key, &canonical);
    Ok(Json(Challenge {
        id,
        kind,
        seed_hex,
        modulus_hex: s.modulus_hex.clone(),
        difficulty,
        frames_b64,
        motions,
        frame_count: FRAME_COUNT,
        frame_dt_ms: FRAME_DT_MS,
        puzzle_w: puzzle.puzzle_w(),
        puzzle_h: puzzle.puzzle_h(),
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
        if !s.redeem.try_consume(&req.token).await {
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
    if !s.global.lock().await.allow(Instant::now()) {
        return err("server busy");
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
    let (kind, seed, difficulty, created_ts, issuer_ip) = {
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
            stored.kind.clone(),
            hex::decode(&stored.seed_hex).unwrap(),
            stored.difficulty,
            stored.created_ts,
            stored.issuer_ip.clone(),
        )
    };
    let puzzle = match captchas::by_kind(&kind) {
        Some(p) => p,
        None => return err("unknown captcha kind"),
    };
    if issuer_ip != ip {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.5).await;
        return err("challenge bound to another client");
    }
    let canonical = format!(
        "{}|{}|{}|{}|{}",
        sol.challenge_id,
        kind,
        hex::encode(&seed),
        difficulty,
        issuer_ip
    );
    if !token::verify_blob(&s.challenge_key, &canonical, &sol.sig) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.5).await;
        return err("invalid challenge signature");
    }
    if let Err(m) = puzzle.validate(&sol.clicks) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.3).await;
        return err(m);
    }
    let trail: Vec<(f64, f64, f64)> = sol.trail.iter().map(|p| (p.x, p.y, p.t)).collect();
    let is_touch = sol.input_type.eq_ignore_ascii_case("touch")
        || sol.input_type.eq_ignore_ascii_case("pen");
    match difficulty::grade_trail(&trail, is_touch) {
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
    if let Err(m) = puzzle.grade(&s.challenge_key, &sol.challenge_id, &sol.clicks) {
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