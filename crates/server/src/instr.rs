use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrOp {
    pub op: String,
    pub k: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrProgram {
    pub seed: u32,
    pub ops: Vec<InstrOp>,
}

const OP_KINDS: [&str; 7] = ["xor", "add", "mul", "and", "or", "rotl", "dom"];

fn rotl32(v: u32, by: u32) -> u32 {
    let b = by & 31;
    if b == 0 {
        v
    } else {
        (v << b) | (v >> (32 - b))
    }
}

fn apply(acc: u32, op: &InstrOp) -> u32 {
    match op.op.as_str() {
        "xor" => acc ^ op.k,
        "add" => acc.wrapping_add(op.k),
        "mul" => acc.wrapping_mul(op.k | 1),
        "and" => {
            let r = acc & op.k;
            if r == 0 {
                acc.wrapping_add(op.k)
            } else {
                r
            }
        }
        "or" => acc | op.k,
        "rotl" => rotl32(acc, op.k),
        "dom" => {
            let n = (op.k % 24) + 8;
            let mut v = acc;
            for i in 0..n {
                v = v.wrapping_add(i.wrapping_mul(2654435761));
                v ^= rotl32(v, 7);
            }
            v ^= n;
            v
        }
        _ => acc,
    }
}

pub fn generate(challenge_key: &[u8], challenge_id: &str, nonce: &str) -> InstrProgram {
    let mut hasher = Sha256::new();
    hasher.update(b"redaptcha-instr-gen");
    hasher.update(challenge_key);
    hasher.update(challenge_id.as_bytes());
    hasher.update(nonce.as_bytes());
    let digest = hasher.finalize();
    let mut stream: Vec<u32> = Vec::with_capacity(16);
    let mut counter: u32 = 0;
    let mut pull = |stream: &mut Vec<u32>| -> u32 {
        if stream.is_empty() {
            let mut h = Sha256::new();
            h.update(b"redaptcha-instr-stream");
            h.update(digest);
            h.update(counter.to_le_bytes());
            counter = counter.wrapping_add(1);
            let d = h.finalize();
            for chunk in d.chunks_exact(4) {
                stream.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
        }
        stream.pop().unwrap()
    };
    let seed = pull(&mut stream);
    let op_count = 18 + (pull(&mut stream) % 11) as usize;
    let mut ops = Vec::with_capacity(op_count);
    let mut dom_used = false;
    for i in 0..op_count {
        let mut idx = (pull(&mut stream) as usize) % OP_KINDS.len();
        if OP_KINDS[idx] == "dom" {
            dom_used = true;
        }
        if i == op_count - 1 && !dom_used {
            idx = OP_KINDS.iter().position(|k| *k == "dom").unwrap();
        }
        let k = pull(&mut stream);
        ops.push(InstrOp {
            op: OP_KINDS[idx].to_string(),
            k,
        });
    }
    InstrProgram { seed, ops }
}

pub fn expected(program: &InstrProgram) -> u32 {
    let mut acc = program.seed;
    for op in &program.ops {
        acc = apply(acc, op);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_for_same_inputs() {
        let a = generate(b"key", "id-1", "nonce-1");
        let b = generate(b"key", "id-1", "nonce-1");
        assert_eq!(expected(&a), expected(&b));
        assert_eq!(a.ops.len(), b.ops.len());
    }
    #[test]
    fn differs_across_nonce() {
        let a = generate(b"key", "id-1", "nonce-1");
        let b = generate(b"key", "id-1", "nonce-2");
        assert!(a.seed != b.seed || expected(&a) != expected(&b));
    }
    #[test]
    fn always_includes_dom_op() {
        for i in 0..50 {
            let id = format!("id-{}", i);
            let p = generate(b"key", &id, "n");
            assert!(p.ops.iter().any(|o| o.op == "dom"));
        }
    }
    #[test]
    fn op_count_in_range() {
        for i in 0..50 {
            let id = format!("id-{}", i);
            let p = generate(b"key", &id, "n");
            assert!(p.ops.len() >= 18 && p.ops.len() <= 28);
        }
    }
}
