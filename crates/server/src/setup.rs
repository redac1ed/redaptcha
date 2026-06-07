use num_bigint::{BigUint, RandBigInt};
use num_integer::Integer;
use rand::rngs::OsRng;
use zeroize::Zeroize; 

const SMALL_PRIMES: [u32; 25] = [
    2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
];

pub struct TrustedSetup {
    pub modulus_hex: String,
    pub bits: u64,
}

pub fn generate(bits: u64) -> TrustedSetup {
    let half = bits / 2;
    let p = random_prime(half);
    let q = random_prime(bits - half);
    let n = &p * &q;
    let setup = TrustedSetup {
        modulus_hex: n.to_str_radix(16),
        bits: n.bits(),
    };
    drop_secret(p);
    drop_secret(q);
    setup
}

fn drop_secret(mut value: BigUint) {
    let mut bytes = value.to_bytes_le();
    bytes.zeroize();
    value.assign_from_slice(&[]);
    drop(value);
}

fn random_prime(bits: u64) -> BigUint {
    let mut rng = OsRng;
    loop {
        let mut candidate = rng.gen_biguint(bits);
        let top = BigUint::from(1u32) << (bits - 1);
        candidate |= &top;
        candidate |= BigUint::from(1u32);
        if is_probable_prime(&candidate, 40) {
            return candidate;
        }
        let mut bytes = candidate.to_bytes_le();
        bytes.zeroize();
    }
}

pub fn is_probable_prime(n: &BigUint, rounds: u32) -> bool {
    let two = BigUint::from(2u32);
    let three = BigUint::from(3u32);
    if n < &two {
        return false;
    }
    if n == &two || n == &three {
        return true;
    }
    if n.is_even() {
        return false;
    }
    for &small in SMALL_PRIMES.iter() {
        let sp = BigUint::from(small);
        if n == &sp {
            return true;
        }
        if (n % &sp) == BigUint::from(0u32) {
            return false;
        }
    }
    let n_minus_one = n - 1u32;
    let mut d = n_minus_one.clone();
    let mut r = 0u32;
    while d.is_even() {
        d >>= 1;
        r += 1;
    }
    let mut rng = OsRng;
    'witness: for _ in 0..rounds {
        let a = rng.gen_biguint_range(&two, &n_minus_one);
        let mut x = a.modpow(&d, n);
        if x == BigUint::from(1u32) || x == n_minus_one {
            continue;
        }
        for _ in 0..(r.saturating_sub(1)) {
            x = (&x * &x) % n;
            if x == n_minus_one {
                continue 'witness;
            }
            if x == BigUint::from(1u32) {
                return false;
            }
        }
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_known_primes() {
        assert!(is_probable_prime(&BigUint::from(2u32), 8));
        assert!(is_probable_prime(&BigUint::from(3u32), 8));
        assert!(is_probable_prime(&BigUint::from(97u32), 8));
        assert!(is_probable_prime(&BigUint::from(7919u32), 8));
    }
    #[test]
    fn small_known_composites() {
        assert!(!is_probable_prime(&BigUint::from(1u32), 8));
        assert!(!is_probable_prime(&BigUint::from(4u32), 8));
        assert!(!is_probable_prime(&BigUint::from(100u32), 8));
        assert!(!is_probable_prime(&BigUint::from(7917u32), 8));
    }
    #[test]
    fn generated_modulus_has_expected_size() {
        let setup = generate(512);
        assert!(setup.bits >= 500 && setup.bits <= 512);
    }
}
