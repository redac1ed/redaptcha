use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const TOKEN_TTL_SECS: u64 = 300;
pub const REDEEM_RETENTION: Duration = Duration::from_secs(600);

pub struct TokenClaims {
    pub challenge_id: String,
    pub site_key: String,
    pub hostname: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn b64url(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    URL_SAFE_NO_PAD.decode(text.as_bytes()).ok()
}

pub fn sign(key: &[u8], claims: &TokenClaims) -> String {
    let payload = format!(
        "{}|{}|{}|{}|{}",
        claims.challenge_id, claims.site_key, claims.hostname, claims.issued_at, claims.expires_at
    );
    let encoded = b64url(payload.as_bytes());
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(encoded.as_bytes());
    let sig = b64url(&mac.finalize().into_bytes());
    format!("{}.{}", encoded, sig)
}

pub fn verify(key: &[u8], token: &str) -> Option<TokenClaims> {
    let (encoded, sig) = token.split_once('.')?;
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(encoded.as_bytes());
    let expected = mac.finalize().into_bytes();
    let got = b64url_decode(sig)?;
    if got.len() != expected.len() {
        return None;
    }
    if got.ct_eq(&expected).unwrap_u8() == 0 {
        return None;
    }
    let raw = b64url_decode(encoded)?;
    let payload = String::from_utf8(raw).ok()?;
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 5 {
        return None;
    }
    let issued_at: u64 = parts[3].parse().ok()?;
    let expires_at: u64 = parts[4].parse().ok()?;
    if now_secs() > expires_at {
        return None;
    }
    Some(TokenClaims {
        challenge_id: parts[0].to_string(),
        site_key: parts[1].to_string(),
        hostname: parts[2].to_string(),
        issued_at,
        expires_at,
    })
}

pub struct RedeemLog {
    seen: HashMap<String, Instant>,
}

impl RedeemLog {
    pub fn new() -> Self {
        RedeemLog {
            seen: HashMap::new(),
        }
    }

    pub fn gc(&mut self) {
        self.seen.retain(|_, t| t.elapsed() < REDEEM_RETENTION);
    }

    pub fn try_consume(&mut self, token: &str) -> bool {
        self.gc();
        if self.seen.contains_key(token) {
            return false;
        }
        self.seen.insert(token.to_string(), Instant::now());
        true
    }
}

impl Default for RedeemLog {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sign_blob(key: &[u8], data: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(data.as_bytes());
    b64url(&mac.finalize().into_bytes())
}

pub fn derive_bytes(key: &[u8], label: &str, out: &mut [u8]) {
    let mut counter: u32 = 0;
    let mut pos = 0;
    while pos < out.len() {
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
        mac.update(label.as_bytes());
        mac.update(&counter.to_be_bytes());
        let block = mac.finalize().into_bytes();
        let take = (out.len() - pos).min(block.len());
        out[pos..pos + take].copy_from_slice(&block[..take]);
        pos += take;
        counter += 1;
    }
}

pub fn verify_blob(key: &[u8], data: &str, sig: &str) -> bool {
    let expected = {
        let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
        mac.update(data.as_bytes());
        mac.finalize().into_bytes()
    };
    let Some(got) = b64url_decode(sig) else {
        return false;
    };
    if got.len() != expected.len() {
        return false;
    }
    got.ct_eq(&expected).unwrap_u8() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> TokenClaims {
        let iat = now_secs();
        TokenClaims {
            challenge_id: "cid-123".into(),
            site_key: "site_abc".into(),
            hostname: "example.com".into(),
            issued_at: iat,
            expires_at: iat + TOKEN_TTL_SECS,
        }
    }
    #[test]
    fn roundtrip_valid() {
        let key = [7u8; 32];
        let t = sign(&key, &claims());
        let parsed = verify(&key, &t).expect("valid token");
        assert_eq!(parsed.site_key, "site_abc");
        assert_eq!(parsed.hostname, "example.com");
    }
    #[test]
    fn wrong_key_rejected() {
        let t = sign(&[1u8; 32], &claims());
        assert!(verify(&[2u8; 32], &t).is_none());
    }
    #[test]
    fn tampered_payload_rejected() {
        let key = [3u8; 32];
        let t = sign(&key, &claims());
        let mut chars: Vec<char> = t.chars().collect();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert!(verify(&key, &tampered).is_none());
    }
    #[test]
    fn expired_rejected() {
        let key = [9u8; 32];
        let iat = now_secs() - 1000;
        let c = TokenClaims {
            challenge_id: "x".into(),
            site_key: "s".into(),
            hostname: "h".into(),
            issued_at: iat,
            expires_at: iat + 10,
        };
        let t = sign(&key, &c);
        assert!(verify(&key, &t).is_none());
    }
    #[test]
    fn single_use_enforced() {
        let mut log = RedeemLog::new();
        assert!(log.try_consume("tok"));
        assert!(!log.try_consume("tok"));
    }
}
