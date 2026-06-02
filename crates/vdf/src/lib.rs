use num_bigint::BigUint;
use num_integer::Integer;
use sha2::{Digest, Sha256};

pub struct VdfParams {
    pub modulus: BigUint,
}

pub struct VdfProof {
    pub output: BigUint,
    pub proof: BigUint,
}

impl VdfParams {
    pub fn from_modulus_hex(hex: &str) -> Self {
        let modulus = BigUint::parse_bytes(hex.as_bytes(), 16)
            .expect("invalid modulus hex");
        Self { modulus }
    }
    pub fn challenge_element(&self, seed: &[u8]) -> BigUint {
        let mut hasher = Sha256::new();
        hasher.update(b"redaptcha-vdf-x");
        hasher.update(seed);
        let h = BigUint::from_bytes_be(&hasher.finalize());
        (h % (&self.modulus - 2u32)) + 2u32
    }
    pub fn eval(&self, x: &BigUint, t: u64) -> VdfProof {
        let mut y = x.clone();
        for _ in 0..t {
            y = (&y * &y) % &self.modulus;
        }
        let l = hash_to_prime(x, &y);
        let q = floor_two_pow_t_div(t, &l);
        let proof = x.modpow(&q, &self.modulus);
        VdfProof { output: y, proof }
    }

    pub fn verify(&self, x: &BigUint, t: u64, proof: &VdfProof) -> bool {
        let l = hash_to_prime(x, &proof.output);
        let r = BigUint::from(2u32).modpow(&BigUint::from(t), &l);
        let lhs = (proof.proof.modpow(&l, &self.modulus)
            * x.modpow(&r, &self.modulus))
            % &self.modulus;
        lhs == proof.output
    }
}

fn floor_two_pow_t_div(t: u64, l: &BigUint) -> BigUint {
    let mut quotient = BigUint::from(0u32);
    let mut remainder = BigUint::from(0u32);
    for i in 0..=t {
        remainder <<= 1;
        if i == 0 {
            remainder += 1u32;
        }
        quotient <<= 1;
        if &remainder >= l {
            remainder -= l;
            quotient += 1u32;
        }
    }
    quotient
}

fn hash_to_prime(x: &BigUint, y: &BigUint) -> BigUint {
    let mut nonce: u64 = 0;
    loop {
        let mut hasher = Sha256::new();
        hasher.update(b"redaptcha-vdf-l");
        hasher.update(x.to_bytes_be());
        hasher.update(y.to_bytes_be());
        hasher.update(nonce.to_le_bytes());
        let mut candidate = BigUint::from_bytes_be(&hasher.finalize());
        candidate |= BigUint::from(1u32);
        if is_probable_prime(&candidate) {
            return candidate;
        }
        nonce += 1;
    }
}

fn is_probable_prime(n: &BigUint) -> bool {
    let two = BigUint::from(2u32);
    if n < &two {
        return false;
    }
    if n == &two {
        return true;
    }
    if n.is_even() {
        return false;
    }
    let n_minus_one = n - 1u32;
    let mut d = n_minus_one.clone();
    let mut r = 0u32;
    while d.is_even() {
        d >>= 1;
        r += 1;
    }
    let witnesses = [2u32, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    'witness: for &a in witnesses.iter() {
        let a = BigUint::from(a);
        if &a >= n {
            continue;
        }
        let mut xx = a.modpow(&d, n);
        if xx == BigUint::from(1u32) || xx == n_minus_one {
            continue;
        }
        for _ in 0..(r - 1) {
            xx = (&xx * &xx) % n;
            if xx == n_minus_one {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests;
