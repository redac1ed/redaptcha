mod captcha;
mod captchas;
mod difficulty;
mod httpfp;
mod instr;
mod setup;
mod token;
mod store;
mod sitekeys;
mod netintel;
mod pow;

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, State},
    http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, Method},
    routing::post,
    Json, Router,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use difficulty::{
    difficulty_for, is_headless, pursuit_coherent, trust_decision, trust_score, ClientProfile,
    GlobalLimiter, TrustDecision, TrustInputs, FRAME_COUNT, FRAME_DT_MS,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use core_types::{Challenge, ChallengeRequest, Solution, VerifyResponse};
use num_bigint::BigUint;
use rand::RngCore;
use sha2::{Digest, Sha256};
use token::{now_secs, TokenClaims, TOKEN_TTL_SECS};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;
use vdf::{VdfParams, VdfProof};
use std::fmt::Write as _;
use parking_lot::Mutex;

const MODULUS_BITS: u64 = 2048;
const TTL: Duration = Duration::from_secs(120);
const MAX_PROFILES: usize = 50_000;
const PROFILE_TTL: Duration = Duration::from_secs(3600);

struct Stored {
    kind: String,
    seed_hex: String,
    difficulty: u64,
    born: Instant,
    created_ts: u64,
    issuer_ip: String,
    nonce: String,
    expires_at: u64,
    expected_instr: Option<u32>,
    pow_bits: Option<u32>,
    site_key: String,
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
    sites: sitekeys::SiteRegistry,
    netintel: Arc<netintel::NetIntel>,
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

#[derive(serde::Deserialize)]
struct SiteVerifyReq { secret: String, response: String }

#[derive(serde::Serialize)]
struct SiteVerifyResp { success: bool, score: Option<f64>, hostname: Option<String> }


pub fn is_production() -> bool {
    matches!(
        std::env::var("REDAPTCHA_ENV").unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "production" | "prod"
    )
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"redaptcha-fp");
    h.update(bytes);
    let d = h.finalize();
    hex::encode(&d[..6])
}

fn client_ip(addr: &SocketAddr, headers: &HeaderMap) -> String {
    if is_production() {
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(last) = xff.split(',').map(str::trim).filter(|s| !s.is_empty()).last() {
                if last.parse::<std::net::IpAddr>().is_ok() {
                    return last.to_string();
                }
            }
        }
    }
    addr.ip().to_string()
}

fn enforce_profile_cap(profiles: &mut HashMap<String, ClientProfile>, ip: &str, now: Instant) {
    if profiles.len() < MAX_PROFILES || profiles.contains_key(ip) {
        return;
    }
    profiles.retain(|_, p| now.duration_since(p.last_seen) <= PROFILE_TTL);
    if profiles.len() < MAX_PROFILES {
        return;
    }
    let mut entries: Vec<(String, Instant)> = profiles
        .iter()
        .map(|(k, p)| (k.clone(), p.last_seen))
        .collect();
    entries.sort_by_key(|&(_, t)| t);
    let excess = profiles.len() + 1 - MAX_PROFILES;
    for (k, _) in entries.into_iter().take(excess) {
        profiles.remove(&k);
    }
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
            if is_production() {
                panic!("REDAPTCHA_MODULUS_HEX not set in production; refusing to generate an ephemeral modulus (would break in-flight challenges on restart)");
            }
            println!("generating {}-bit trusted-setup modulus...", MODULUS_BITS);
            let gen_start = Instant::now();
            let setup = setup::generate(MODULUS_BITS);
            println!(
                "modulus ready: {} bits in {:?} (prime factors discarded)",
                setup.bits,
                gen_start.elapsed()
            );
            println!("set REDAPTCHA_MODULUS_HEX in your deploy config to persist this modulus across restarts (value written to .env.local-style config only, not logged here)");
            setup.modulus_hex
        }
    };
    let redeem = store::RedeemStore::from_env().await;
    let sites = sitekeys::SiteRegistry::from_env();
    if sites.is_empty() {
        println!("site keys: open mode (no REDAPTCHA_SITES set; any site_key accepted)");
    } else {
        println!("site keys: enforced ({} configured)", sites.len());
    }
    let state = Arc::new(AppState {
        store: Mutex::new(HashMap::new()),
        profiles: Mutex::new(HashMap::new()),
        params: VdfParams::from_modulus_hex(&modulus_hex),
        modulus_hex,
        token_key: key,
        challenge_key: chal_key,
        redeem,
        global: Mutex::new(GlobalLimiter::new(Instant::now())),
        sites,
        netintel: Arc::new(netintel::NetIntel::from_env()),
    });
    state.netintel.clone().spawn_refresher();
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(300));
        loop {
            tick.tick().await;
            let now = Instant::now();
            {
                let mut profiles = st.profiles.lock();
                profiles.retain(|_, p| now.duration_since(p.last_seen) <= PROFILE_TTL);
            }
            {
                let mut store = st.store.lock();
                store.retain(|_, v| v.born.elapsed() <= TTL);
            }
        }
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
        .route("/siteverify", post(siteverify))
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
    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr = format!("{host}:{port}");
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
        score: None,
        need_challenge: None, 
    })
}

fn load_or_gen_key(var: &str) -> Vec<u8> {
    if let Ok(b64) = std::env::var(var) {
        if let Ok(bytes) = B64.decode(b64.trim()) {
            if bytes.len() == 32 {
                println!("{var}: loaded (fingerprint {})", fingerprint(&bytes));
                return bytes;
            }
        }
        if is_production() {
            panic!("{var} set but invalid (need base64 of 32 bytes)");
        }
        eprintln!("warning: {var} set but invalid (need base64 of 32 bytes); generating ephemeral key");
    } else if is_production() {
        panic!("{var} not set in production; refusing to generate an ephemeral secret");
    }
    let mut k = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    println!(
        "{var}: generated ephemeral key (fingerprint {}). Set {var} in your deploy config to persist across restarts; the raw value is intentionally NOT logged.",
        fingerprint(&k)
    );
    k
}

async fn issue_default(
    info: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    state: State<Arc<AppState>>,
    req: Json<ChallengeRequest>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    issue(info, Path(captchas::DEFAULT_KIND.to_string()), headers, state, req).await
}

async fn issue(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    State(s): State<Arc<AppState>>,
    Json(req): Json<ChallengeRequest>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    let ip = client_ip(&addr, &headers);
    if s.netintel.is_enabled() {
        let class = s.netintel.classify(&ip);
        if s.netintel.should_block(class) {
            log_reject(&ip, "blocked vpn/proxy ip", None);
            return Err(err("verification failed"));
        }
    }
    if !s.sites.is_empty() {
        let sk = req.site_key.trim();
        if !s.sites.contains(sk) {
            return Err(err("unknown site key"));
        }
        if !s.sites.host_allowed(sk, &req.hostname) {
            return Err(err("site key not allowed for this host"));
        }
    }
    let site_key = req.site_key.trim().to_string();
    let now = Instant::now();
    let puzzle = match captchas::by_kind(&kind) {
        Some(p) => p,
        None => return Err(err("unknown captcha kind")),
    };
    if !s.global.lock().allow(now) {
        return Err(err("server busy"));
    }
    let (difficulty, pow_bits) = {
        let mut profiles = s.profiles.lock();
        enforce_profile_cap(&mut profiles, &ip, now);
        let profile = profiles
            .entry(ip.clone())
            .or_insert_with(|| ClientProfile::new(now));
        profile.roll_window(now);
        if !profile.register_request() {
            return Err(err("rate limited"));
        }
        if kind == "three" {
            (
                difficulty::passive_difficulty_for(profile),
                Some(pow::params_for(profile.suspicion, profile.fail_ratio())),
            )
        } else {
            (difficulty_for(profile), None)
        }
    };
    let id = Uuid::new_v4().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let seed_hex = hex::encode(seed);
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    let pow = pow_bits.map(|bits| pow::challenge(&seed_hex, bits));
    let puzzle_w = puzzle.puzzle_w();
    let puzzle_h = puzzle.puzzle_h();
    let rendered = {
        let chal_key = s.challenge_key.clone();
        let id = id.clone();
        match tokio::task::spawn_blocking(move || puzzle.generate(&chal_key, &id)).await {
            Ok(r) => r,
            Err(_) => return Err(err("server busy")),
        }
    };
    let frames_b64 = rendered.frames_b64;
    let slider = rendered.slider;
    let created_ts = now_secs();
    let expires_at = created_ts + TTL.as_secs();
    let (instr_json, expected_instr) = if kind == "three" {
        let program = instr::generate(&s.challenge_key, &id, &nonce);
        let exp = instr::expected(&program);
        (serde_json::to_string(&program).ok(), Some(exp))
    } else {
        (None, None)
    };
    {
        let mut store = s.store.lock();
        store.insert(
            id.clone(),
            Stored {
                kind: kind.clone(),
                seed_hex: seed_hex.clone(),
                difficulty,
                born: now,
                created_ts,
                issuer_ip: ip.clone(),
                nonce: nonce.clone(),
                expires_at,
                expected_instr,
                pow_bits,
                site_key: site_key.clone(),
            },
        );
    }
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        id, kind, seed_hex, difficulty, nonce, expires_at, ip
    );
    let sig = token::sign_blob(&s.challenge_key, &canonical);
    Ok(Json(Challenge {
        id,
        kind,
        seed_hex,
        modulus_hex: s.modulus_hex.clone(),
        difficulty,
        nonce,
        expires_at,
        frames_b64,
        frame_count: FRAME_COUNT,
        frame_dt_ms: FRAME_DT_MS,
        puzzle_w,
        puzzle_h,
        instr: instr_json, 
        sig,
        slider,
        pow,
    }))
}


async fn siteverify(
    State(s): State<Arc<AppState>>,
    Json(req): Json<SiteVerifyReq>,
) -> Json<SiteVerifyResp> {
    let Some(claims) = token::verify(&s.token_key, &req.response) else {
        return Json(SiteVerifyResp { success: false, score: None, hostname: None });
    };
    if !s.sites.is_empty() && !s.sites.secret_matches(&claims.site_key, &req.secret) {
        return Json(SiteVerifyResp { success: false, score: None, hostname: None });
    }
    if !s.redeem.try_consume(&req.response).await {
        return Json(SiteVerifyResp { success: false, score: None, hostname: None });
    }
    Json(SiteVerifyResp { success: true, score: Some(claims.score), hostname: Some(claims.hostname) })
}

async fn content(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(s): State<Arc<AppState>>,
    Json(req): Json<ContentReq>,
) -> Json<ContentResp> {
    let ip = client_ip(&addr, &headers);
    {
        let now = Instant::now();
        let mut profiles = s.profiles.lock();
        enforce_profile_cap(&mut profiles, &ip, now);
        let profile = profiles
            .entry(ip.clone())
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
    if claims.ip != ip {
        return Json(ContentResp { ok: false, content: None });
    }
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
    headers: HeaderMap,
    State(s): State<Arc<AppState>>,
    Json(sol): Json<Solution>,
) -> Json<VerifyResponse> {
    let ip = client_ip(&addr, &headers);
    if s.netintel.is_enabled() {
        let class = s.netintel.classify(&ip);
        if s.netintel.should_block(class) {
            log_reject(&ip, "blocked vpn/proxy ip", None);
            return err("verification failed");
        }
    }
    if sol.output_hex.len() > 4096 || sol.proof_hex.len() > 4096 {
        return err("input too large");
    }
    if !s.global.lock().allow(Instant::now()) {
        return err("server busy");
    }
    {
        let now = Instant::now();
        let mut profiles = s.profiles.lock();
        enforce_profile_cap(&mut profiles, &ip, now);
        let profile = profiles
            .entry(ip.clone())
            .or_insert_with(|| ClientProfile::new(now));
        profile.roll_window(now);
        if !profile.register_verify(now) {
            return err("rate limited");
        }
    }
    let (kind, seed, difficulty, created_ts, issuer_ip, nonce, expires_at, expected_instr, pow_bits, site_key) = {
        let mut store = s.store.lock();
        let stored = match store.get(&sol.challenge_id) {
            Some(c) => c,
            None => return err("unknown challenge"),
        };
        if stored.born.elapsed() > TTL {
            store.remove(&sol.challenge_id);
            return err("challenge expired");
        }
        let seed = match hex::decode(&stored.seed_hex) {
            Ok(s) => s,
            Err(_) => {
                store.remove(&sol.challenge_id);
                return err("corrupt challenge");
            }
        };
        (
            stored.kind.clone(),
            seed,
            stored.difficulty,
            stored.created_ts,
            stored.issuer_ip.clone(),
            stored.nonce.clone(),
            stored.expires_at,
            stored.expected_instr,
            stored.pow_bits,
            stored.site_key.clone(),
        )
    };
    if now_secs() > expires_at {
        return err("challenge expired");
    }
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
        "{}|{}|{}|{}|{}|{}|{}",
        sol.challenge_id,
        kind,
        hex::encode(&seed),
        difficulty,
        nonce,
        expires_at,
        issuer_ip
    );
    if !token::verify_blob(&s.challenge_key, &canonical, &sol.sig) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.5).await;
        return err("invalid challenge signature");
    }
    let (header_fp, ja4_fp) = if httpfp::is_disabled() {
        (1.0, 1.0)
    } else {
        let hf = match httpfp::browser_fingerprint(&headers) {
            Ok(score) => score,
            Err(reason) => {
                record_profile_failure(&s, &ip).await;
                flag_profile(&s, &ip, 0.5).await;
                log_reject(&ip, reason, None);
                return Json(VerifyResponse {
                    ok: false,
                    token: None,
                    message: "verification failed".into(),
                    score: Some(0.0),
                    need_challenge: None,
                });
            }
        };
        let jf = match httpfp::ja4_consistent(&headers) {
            Ok(score) => score,
            Err(reason) => {
                record_profile_failure(&s, &ip).await;
                flag_profile(&s, &ip, 0.6).await;
                log_reject(&ip, reason, None);
                return Json(VerifyResponse {
                    ok: false,
                    token: None,
                    message: "verification failed".into(),
                    score: Some(0.0),
                    need_challenge: None,
                });
            }
        };
        (hf, jf)
    };
    let is_touch = sol.input_type.eq_ignore_ascii_case("touch")
        || sol.input_type.eq_ignore_ascii_case("pen")
        || sol.telemetry.has_touch;
    let trail: Vec<(f64, f64, f64)> = sol.trail.iter().map(|p| (p.x, p.y, p.t)).collect();
    let trail_weight = match difficulty::grade_trail(&trail, is_touch) {
        Ok(w) => w,
        Err(_) => 0.2,
    };
    let regularity_weight = difficulty::trail_regularity(&trail, is_touch);
    let timing_weight = difficulty::classify_timing(
        sol.clicks.last().map(|c| c.t).unwrap_or(0.0)
            - sol.clicks.first().map(|c| c.t).unwrap_or(0.0),
        sol.clicks.len(),
    )
    .unwrap_or(0.0);
    if let Err(m) = puzzle.grade(&s.challenge_key, &sol.challenge_id, &sol.clicks, &sol.trail) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.3).await;
        return err(m);
    }
    if !is_touch && puzzle.kind() == "one" {
        let click_tuples: Vec<(f64, f64, f64)> = sol.clicks.iter().map(|c| (c.x, c.y, c.t)).collect();
        if let Err(m) = pursuit_coherent(&trail, &click_tuples) {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.3).await;
            log_reject(&ip, m, None);
            return err("verification failed");
        }
        if let Err(m) = puzzle.track(&s.challenge_key, &sol.challenge_id, &sol.clicks, &sol.trail) {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.35).await;
            log_reject(&ip, m, None);
            return err("verification failed");
        }
    }
    if !is_touch && puzzle.kind() == "two" {
        if let Err(m) = puzzle.track(&s.challenge_key, &sol.challenge_id, &sol.clicks, &sol.trail) {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.35).await;
            log_reject(&ip, m, None);
            return err("verification failed");
        }
    }
    let (suspicion, fail_ratio) = {
        let mut profiles = s.profiles.lock();
        let p = profiles
            .entry(ip.clone())
            .or_insert_with(|| ClientProfile::new(Instant::now()));
        (p.suspicion, p.fail_ratio())
    };
    let t = &sol.telemetry;
    let grade_score = 1.0;
    let trust_inputs = TrustInputs {
        trail_weight,
        timing_weight,
        grade_score,
        suspicion,
        fail_ratio,
        regularity_weight,
        page_load_to_first_move_ms: t.page_load_to_first_move_ms,
        focus_events: t.focus_events,
        blur_events: t.blur_events,
        scroll_events: t.scroll_events,
        key_events: t.key_events,
        move_events: t.move_events,
        has_touch: t.has_touch,
        max_pressure: t.max_pressure,
        webdriver: t.webdriver,
        input_type: sol.input_type.clone(),
    };
    if is_headless(&trust_inputs) {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.4).await;
        log_reject(&ip, "headless signature", None);
        return Json(VerifyResponse {
            ok: false,
            token: None,
            message: "verification failed".into(),
            score: Some(0.0),
            need_challenge: None,
        });
    }
    let mut trust = trust_score(&trust_inputs, trail.len()) * header_fp * ja4_fp;
    if kind == "three" {
        if !difficulty::nonce_echo_ok(&sol.telemetry, &nonce) {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.4).await;
            log_reject(&ip, "nonce echo mismatch", None);
            return Json(VerifyResponse {
                ok: false,
                token: None,
                message: "verification failed".into(),
                score: Some(0.0),
                need_challenge: None,
            });
        }
        trust *= difficulty::attestation_consistent(&sol.telemetry);
        let instr_ok = match (expected_instr, sol.instr_result) {
            (Some(exp), Some(got)) => exp == got,
            _ => false,
        };
        if !instr_ok {
            flag_profile(&s, &ip, 0.1).await;
            log_reject(&ip, "instrumentation failed", Some(trust));
            return Json(VerifyResponse {
                ok: true,
                token: None,
                message: "additional verification required".into(),
                score: Some(trust),
                need_challenge: Some("two".to_string()),
            });
        }
        let pow_ok = match (sol.pow_nonce, sol.pow_hash_hex.as_deref(), pow_bits) {
            (Some(n), Some(h), Some(bits)) => pow::verify(&seed, bits, n, h),
            _ => false,
        };
        if !pow_ok {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.4).await;
            log_reject(&ip, "pow failed", Some(trust));
            return Json(VerifyResponse {
                ok: false,
                token: None,
                message: "verification failed".into(),
                score: Some(0.0),
                need_challenge: None,
            });
        }
        let solve_secs = now_secs().saturating_sub(created_ts);
        if solve_secs < difficulty::PASSIVE_MIN_SOLVE_SECS {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.5).await;
            log_reject(&ip, "passive solved too fast", Some(trust));
            return Json(VerifyResponse {
                ok: false,
                token: None,
                message: "verification failed".into(),
                score: Some(0.0),
                need_challenge: None,
            });
        }
        if trust < difficulty::PASSIVE_PASS_THRESHOLD {
            log_reject(&ip, "passive low trust", Some(trust));
            return Json(VerifyResponse {
                ok: true,
                token: None,
                message: "additional verification required".into(),
                score: Some(trust),
                need_challenge: Some("two".to_string()),
            });
        }
    }
    match trust_decision(trust) {
        TrustDecision::Pass => {}
        TrustDecision::StepUp => {
            flag_profile(&s, &ip, 0.10).await;
            log_reject(&ip, "trust step-up", Some(trust));
            let need_challenge = if kind == "three" {
                Some("two".to_string())
            } else {
                None
            };
            return Json(VerifyResponse {
                ok: true,
                token: None,
                message: "additional verification required".into(),
                score: Some(trust),
                need_challenge,
            });
        }
        TrustDecision::Fail => {
            record_profile_failure(&s, &ip).await;
            flag_profile(&s, &ip, 0.25).await;
            log_reject(&ip, "trust fail", Some(trust));
            return Json(VerifyResponse {
                ok: false,
                token: None,
                message: "verification failed".into(),
                score: Some(trust),
                need_challenge: None,
            });
        }
    }
    let mut trajectory = String::with_capacity(sol.clicks.len() * 16);
    for c in &sol.clicks {
        let _ = write!(trajectory, "{},{},{};", c.x as i64, c.y as i64, c.t as i64);
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
    let rounds_required = puzzle.rounds();
    if rounds_required > 1 {
        let now = Instant::now();
        let count = {
            let mut profiles = s.profiles.lock();
            let profile = profiles
                .entry(ip.clone())
                .or_insert_with(|| ClientProfile::new(now));
            profile.record_round(&kind, now, Duration::from_secs(120))
        };
        if count < rounds_required {
            return Json(VerifyResponse {
                ok: true,
                token: None,
                message: format!("round {} of {} complete", count, rounds_required),
                score: Some(trust),
                need_challenge: None,
            });
        }
        s.profiles.lock().entry(ip.clone()).and_modify(|p| p.reset_rounds(&kind));
    }
    let ok = {
        let s2 = s.clone();
        match tokio::task::spawn_blocking(move || s2.params.verify(&x, difficulty, &vdf_proof)).await
        {
            Ok(v) => v,
            Err(_) => return err("server busy"),
        }
    };
    if ok {
        {
            let now = Instant::now();
            let mut profiles = s.profiles.lock();
            let profile = profiles
                .entry(ip.clone())
                .or_insert_with(|| ClientProfile::new(now));
            if !profile.register_solve(now) {
                log_reject(&ip, "solve rate limited", Some(trust));
                return Json(VerifyResponse {
                    ok: false,
                    token: None,
                    message: "rate limited".into(),
                    score: Some(trust),
                    need_challenge: None,
                });
            }
        }
        s.store.lock().remove(&sol.challenge_id);
        record_profile_success(&s, &ip).await;
        let iat = now_secs();
        let claims = TokenClaims {
            challenge_id: sol.challenge_id.clone(),
            site_key: site_key.clone(),
            hostname: String::new(),
            ip: ip.clone(),
            issued_at: created_ts.max(iat.saturating_sub(TTL.as_secs())),
            expires_at: iat + TOKEN_TTL_SECS,
            score: trust,
        };
        let signed = token::sign(&s.token_key, &claims);
        Json(VerifyResponse {
            ok: true,
            token: Some(signed),
            message: "verified".into(),
            score: Some(trust),
            need_challenge: None,
        })
    } else {
        record_profile_failure(&s, &ip).await;
        flag_profile(&s, &ip, 0.2).await;
        Json(VerifyResponse {
            ok: false,
            token: None,
            message: "proof invalid".into(),
            score: Some(trust),
            need_challenge: None,
        })
    }
}

fn log_reject(ip: &str, reason: &str, score: Option<f64>) {
    match score {
        Some(s) => eprintln!("reject ip={ip} reason={reason} score={s:.3}"),
        None => eprintln!("reject ip={ip} reason={reason}"),
    }
}

async fn flag_profile(s: &Arc<AppState>, ip: &str, weight: f64) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock();
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.flag_anomaly(weight);
}

async fn record_profile_success(s: &Arc<AppState>, ip: &str) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock();
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.record_success();
}

async fn record_profile_failure(s: &Arc<AppState>, ip: &str) {
    let now = Instant::now();
    let mut profiles = s.profiles.lock();
    let p = profiles
        .entry(ip.to_string())
        .or_insert_with(|| ClientProfile::new(now));
    p.record_failure();
}