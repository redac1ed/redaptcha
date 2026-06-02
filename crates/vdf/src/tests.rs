use super::*;

const TEST_MODULUS_HEX: &str =
    "800000000000000000000000000000000000000000000000000000000000f18280000000000000000000000000000000000000000000000000000000491e8f5f";

#[test]
fn eval_then_verify_succeeds() {
    let params = VdfParams::from_modulus_hex(TEST_MODULUS_HEX);
    let x = params.challenge_element(b"hello");
    let t = 1000;
    let proof = params.eval(&x, t);
    assert!(params.verify(&x, t, &proof));
}

#[test]
fn tampered_output_fails() {
    let params = VdfParams::from_modulus_hex(TEST_MODULUS_HEX);
    let x = params.challenge_element(b"hello");
    let t = 1000;
    let mut proof = params.eval(&x, t);
    proof.output += 1u32;
    assert!(!params.verify(&x, t, &proof));
}

#[test]
fn wrong_t_fails() {
    let params = VdfParams::from_modulus_hex(TEST_MODULUS_HEX);
    let x = params.challenge_element(b"hello");
    let proof = params.eval(&x, 1000);
    assert!(!params.verify(&x, 999, &proof));
}
