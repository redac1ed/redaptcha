use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Challenge {
    pub id: String,
    pub kind: String,
    pub seed_hex: String,
    pub modulus_hex: String,
    pub difficulty: u64,
    pub frames_b64: Vec<String>,
    #[serde(default)]
    pub slider: Option<SliderHint>,
    pub frame_count: u32,
    pub frame_dt_ms: f64,
    pub puzzle_w: f64,
    pub puzzle_h: f64,
    pub sig: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SliderHint {
    pub piece_w: f64,
    pub piece_h: f64,
    pub start_x: f64,
    pub start_y: f64,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionTelemetry {
    #[serde(default)]
    pub page_load_to_first_move_ms: Option<f64>,
    #[serde(default)]
    pub focus_events: u32,
    #[serde(default)]
    pub blur_events: u32,
    #[serde(default)]
    pub scroll_events: u32,
    #[serde(default)]
    pub key_events: u32,
    #[serde(default)]
    pub move_events: u32,
    #[serde(default)]
    pub has_touch: bool,
    #[serde(default)]
    pub max_pressure: f64,
    #[serde(default)]
    pub pointer_kinds: Vec<String>,
    #[serde(default)]
    pub screen_w: Option<u32>,
    #[serde(default)]
    pub screen_h: Option<u32>,
    #[serde(default)]
    pub viewport_w: Option<u32>,
    #[serde(default)]
    pub viewport_h: Option<u32>,
    #[serde(default = "default_dpr")]
    pub device_pixel_ratio: f64,
    #[serde(default)]
    pub webdriver: bool,
    #[serde(default)]
    pub hidden_time_ms: f64,
}

fn default_dpr() -> f64 {
    1.0
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
    #[serde(default)]
    pub input_type: String,
    #[serde(default)]
    pub telemetry: SessionTelemetry,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}