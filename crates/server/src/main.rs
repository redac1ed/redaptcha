use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderValue, Method},
    routing::post,
    Json, Router,
};
use core_types::{Challenge, Click, Solution, VerifyResponse};
use hmac::{Hmac, Mac};
use num_bigint::BigUint;
use rand::RngCore;
use sha2::Sha256;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use uuid::Uuid;
use vdf::{VdfParams, VdfProof};

const MODULUS_HEX: &str =
    "c7970ceedcc3b0754490201a7aa613cd73911081c790f5f1a8726f463550bb5b\
     7ff0db8e1ea1189ec72f93d1650011bd721aeeacc2acde32a04107f0648c2813\
     a31f5b0b7765ff8b44b4b6ffc93384b646eb09c7cf5e8592d40ea33c80039f35\
     b4f14a04b51f7bfd781be4d1673164ba8eb991c2c4d730bbbe35f592bdef524a";
const DIFFICULTY: u64 = 50_000;
const TTL: Duration = Duration::from_secs(120);
const PUZZLE_W: f64 = 320.0;
const PUZZLE_H: f64 = 240.0;
const EXPECTED_CLICKS: usize = 3;
const MIN_INTER_CLICK_MS: f64 = 40.0;
const MIN_TOTAL_MS: f64 = 250.0;
const MAX_TOTAL_MS: f64 = 15_000.0;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_CHALLENGES_PER_WINDOW: u32 = 20;

type HmacSha256 = Hmac<Sha256>;

struct Stored {
    seed_hex: String,
    difficulty: u64,
    born: Instant,
    ip: String,
}
struct RateEntry {
    count: u32,
    window_start: Instant,
    active: u32,
}
struct AppState {
    store: Mutex<HashMap<String, Stored>>,
    rate: Mutex<HashMap<String, RateEntry>>,
    params: VdfParams,
    token_key: Vec<u8>,
}

#[tokio::main]
async fn main() {
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let state = Arc::new(AppState {
        store: Mutex::new(HashMap::new()),
        rate: Mutex::new(HashMap::new()),
        params: VdfParams::from_modulus_hex(MODULUS_HEX),
        token_key: key,
    });
    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods([Method::POST]);
    let app = Router::new()
        .route("/challenge", post(issue))
        .route("/verify", post(verify))
        .layer(cors)
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on http://localhost:3000");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn sign_token(key: &[u8], challenge_id: &str, exp: u64) -> String {
    let payload = format!("{}.{}", challenge_id, exp);
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(payload.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    format!("{}.{}", payload, sig)
}

fn err(message: &str) -> Json<VerifyResponse> {
    Json(VerifyResponse {
        ok: false,
        token: None,
        message: message.into(),
    })
}

fn rate_ok(state: &AppState, ip: &str, rate: &mut HashMap<String, RateEntry>) -> bool {
    let now = Instant::now();
    let entry = rate.entry(ip.to_string()).or_insert(RateEntry {
        count: 0,
        window_start: now,
        active: 0,
    });
    if now.duration_since(entry.window_start) > RATE_WINDOW {
        entry.count = 0;
        entry.window_start = now;
    }
    if entry.active >= 1 {
        return false;
    }
    if entry.count >= MAX_CHALLENGES_PER_WINDOW {
        return false;
    }
    entry.count += 1;
    entry.active += 1;
    let _ = state;
    true
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
        if c.x < 0.0 || c.x > PUZZLE_W || c.y < 0.0 || c.y > PUZZLE_H {
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

async fn issue(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(s): State<Arc<AppState>>,
) -> Result<Json<Challenge>, Json<VerifyResponse>> {
    let ip = addr.ip().to_string();
    {
        let mut rate = s.rate.lock().await;
        if !rate_ok(&s, &ip, &mut rate) {
            return Err(err("rate limited"));
        }
    }
    let id = Uuid::new_v4().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let seed_hex = hex::encode(seed);
    {
        let mut store = s.store.lock().await;
        store.retain(|_, v| v.born.elapsed() <= TTL);
        store.insert(
            id.clone(),
            Stored {
                seed_hex: seed_hex.clone(),
                difficulty: DIFFICULTY,
                born: Instant::now(),
                ip,
            },
        );
    }
    Ok(Json(Challenge {
        id,
        seed_hex,
        modulus_hex: MODULUS_HEX.to_string(),
        difficulty: DIFFICULTY,
    }))
}

fn release_active(rate: &mut HashMap<String, RateEntry>, ip: &str) {
    if let Some(e) = rate.get_mut(ip) {
        if e.active > 0 {
            e.active -= 1;
        }
    }
}

async fn verify(
    State(s): State<Arc<AppState>>,
    Json(sol): Json<Solution>,
) -> Json<VerifyResponse> {
    if sol.output_hex.len() > 4096 || sol.proof_hex.len() > 4096 {
        return err("input too large");
    }
    let (seed, difficulty, ip) = {
        let mut store = s.store.lock().await;
        let stored = match store.get(&sol.challenge_id) {
            Some(c) => c,
            None => return err("unknown challenge"),
        };
        if stored.born.elapsed() > TTL {
            let ip = stored.ip.clone();
            store.remove(&sol.challenge_id);
            let mut rate = s.rate.lock().await;
            release_active(&mut rate, &ip);
            return err("challenge expired");
        }
        (
            hex::decode(&stored.seed_hex).unwrap(),
            stored.difficulty,
            stored.ip.clone(),
        )
    };
    if let Err(m) = validate_clicks(&sol.clicks) {
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
        let mut rate = s.rate.lock().await;
        release_active(&mut rate, &ip);
    }
    Json(VerifyResponse {
        ok,
        token: if ok {
            Some(sign_token(&s.token_key, &sol.challenge_id, now_secs() + 300))
        } else {
            None
        },
        message: if ok { "verified".into() } else { "proof invalid".into() },
    })
}