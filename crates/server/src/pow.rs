use argon2::{Algorithm, Argon2, Params, Version};
use core_types::PowChallenge;

pub const POW_M_COST: u32 = 32_768;
pub const POW_T_COST: u32 = 1;
pub const POW_P_COST: u32 = 1;
pub const POW_HASH_LEN: usize = 32;
pub const POW_MIN_BITS: u32 = 3;
pub const POW_MAX_BITS: u32 = 6;

pub fn params_for(suspicion: f64, fail_ratio: f64) -> u32 {
    let base = POW_MIN_BITS as f64 + suspicion * 3.0 + fail_ratio * 3.0;
    (base.round() as u32).clamp(POW_MIN_BITS, POW_MAX_BITS)
}

pub fn challenge(salt_hex: &str, bits: u32) -> PowChallenge {
    PowChallenge {
        salt_hex: salt_hex.to_string(),
        m_cost: POW_M_COST,
        t_cost: POW_T_COST,
        p_cost: POW_P_COST,
        bits,
    }
}

fn hash(salt: &[u8], nonce: u64) -> Option<[u8; POW_HASH_LEN]> {
    let params = Params::new(POW_M_COST, POW_T_COST, POW_P_COST, Some(POW_HASH_LEN)).ok()?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let pwd = nonce.to_string();
    let mut out = [0u8; POW_HASH_LEN];
    argon.hash_password_into(pwd.as_bytes(), salt, &mut out).ok()?;
    Some(out)
}

fn leading_zero_bits(h: &[u8]) -> u32 {
    let mut count = 0;
    for &b in h {
        if b == 0 {
            count += 8;
        } else {
            count += b.leading_zeros();
            break;
        }
    }
    count
}

pub fn verify(salt: &[u8], bits: u32, nonce: u64, claimed_hex: &str) -> bool {
    let h = match hash(salt, nonce) {
        Some(h) => h,
        None => return false,
    };
    if leading_zero_bits(&h) < bits {
        return false;
    }
    hex::encode(h) == claimed_hex.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn params_clamped() {
        assert_eq!(params_for(0.0, 0.0), POW_MIN_BITS);
        assert_eq!(params_for(5.0, 5.0), POW_MAX_BITS);
    }
    #[test]
    fn rejects_wrong_hash() {
        assert!(!verify(b"0123456789abcdef", 3, 0, "00"));
    }
    #[test]
    fn accepts_valid_solution() {
        let salt = b"0123456789abcdef0123456789abcdef";
        let bits = 3;
        let mut nonce = 0u64;
        loop {
            let h = hash(salt, nonce).unwrap();
            if leading_zero_bits(&h) >= bits {
                assert!(verify(salt, bits, nonce, &hex::encode(h)));
                break;
            }
            nonce += 1;
        }
    }
    #[test]
    fn rejects_insufficient_bits() {
        let salt = b"0123456789abcdef0123456789abcdef";
        let mut nonce = 0u64;
        loop {
            let h = hash(salt, nonce).unwrap();
            if leading_zero_bits(&h) == 0 {
                assert!(!verify(salt, 4, nonce, &hex::encode(h)));
                break;
            }
            nonce += 1;
        }
    }
}