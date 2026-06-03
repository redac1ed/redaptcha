use axum::{
    extract::State,
    http::Method,
    routing::post,
    Json, Router,
};
use core_types::{Challenge, Solution, VerifyResponse};
use num_bigint::BigUint;
use rand::RngCore;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;
use vdf::{VdfParams, VdfProof};

const MODULUS_HEX: &str =
    "800000000000000000000000000000000000000000000000000000000000f182\
     80000000000000000000000000000000000000000000000000000000491e8f5f";
const DIFFICULTY: u64 = 50_000;
const TTL: Duration = Duration::from_secs(120);

struct Stored {
    seed_hex: String,
    difficulty: u64,
    born: Instant,
}

struct AppState {
    store: Mutex<HashMap<String, Stored>>,
    params: VdfParams,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        store: Mutex::new(HashMap::new()),
        params: VdfParams::from_modulus_hex(MODULUS_HEX),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/challenge", post(issue))
        .route("/verify", post(verify))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn issue(State(s): State<Arc<AppState>>) -> Json<Challenge> {
    let id = Uuid::new_v4().to_string();
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let seed_hex = hex::encode(seed);

    s.store.lock().await.insert(id.clone(), Stored {
        seed_hex: seed_hex.clone(),
        difficulty: DIFFICULTY,
        born: Instant::now(),
    });

    Json(Challenge {
        id,
        seed_hex,
        modulus_hex: MODULUS_HEX.to_string(),
        difficulty: DIFFICULTY,
    })
}

async fn verify(
    State(s): State<Arc<AppState>>,
    Json(sol): Json<Solution>,
) -> Json<VerifyResponse> {
    let mut store = s.store.lock().await;

    let stored = match store.get(&sol.challenge_id) {
        Some(c) => c,
        None => return Json(VerifyResponse {
            ok: false, token: None,
            message: "unknown challenge".into(),
        }),
    };

    if stored.born.elapsed() > TTL {
        store.remove(&sol.challenge_id);
        return Json(VerifyResponse {
            ok: false, token: None,
            message: "challenge expired".into(),
        });
    }

    let seed = hex::decode(&stored.seed_hex).unwrap();
    let x = s.params.challenge_element(&seed);
    let difficulty = stored.difficulty;

    let output = match BigUint::parse_bytes(sol.output_hex.as_bytes(), 16) {
        Some(v) => v,
        None => return Json(VerifyResponse {
            ok: false, token: None,
            message: "invalid output hex".into(),
        }),
    };
    let proof_val = match BigUint::parse_bytes(sol.proof_hex.as_bytes(), 16) {
        Some(v) => v,
        None => return Json(VerifyResponse {
            ok: false, token: None,
            message: "invalid proof hex".into(),
        }),
    };

    let vdf_proof = VdfProof { output, proof: proof_val };
    let ok = s.params.verify(&x, difficulty, &vdf_proof);

    if ok {
        store.remove(&sol.challenge_id);
    }

    Json(VerifyResponse {
        ok,
        token: if ok { Some(Uuid::new_v4().to_string()) } else { None },
        message: if ok { "verified".into() } else { "proof invalid".into() },
    })
}