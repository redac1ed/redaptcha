use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Challenge {
    pub id: String,
    pub seed_hex: String,
    pub modulus_hex: String,
    pub difficulty: u64,
    pub frames_b64: Vec<String>,
    pub motions: Vec<PanelMotion>,
    pub frame_count: u32,
    pub frame_dt_ms: f64,
    pub puzzle_w: f64,
    pub puzzle_h: f64,
    pub sig: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PanelMotion {
    pub cx: f64,
    pub cy: f64,
    pub amp: f64,
    pub turns: f64,
    pub phase: f64,
    pub dir: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Mover {
    pub x0: f64,
    pub y0: f64,
    pub vx: f64,
    pub vy: f64,
    pub amp: f64,
    pub turns: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Click {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailPoint {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeRequest {
    #[serde(default)]
    pub site_key: String,
    #[serde(default)]
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub challenge_id: String,
    pub output_hex: String,
    pub proof_hex: String,
    pub clicks: Vec<Click>,
    #[serde(default)]
    pub trail: Vec<TrailPoint>,
    #[serde(default)]
    pub sig: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub message: String,
}