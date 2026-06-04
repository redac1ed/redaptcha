use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub id: String,
    pub seed_hex: String,
    pub modulus_hex: String,
    pub difficulty: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Click {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub challenge_id: String,
    pub output_hex: String,
    pub proof_hex: String,
    pub clicks: Vec<Click>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerifyResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub message: String,
}