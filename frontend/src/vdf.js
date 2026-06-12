async function sha256(data) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", data));
}
function bigIntToBytesBE(n) {
  if (n === 0n) return new Uint8Array(1);
  let hex = n.toString(16);
  if (hex.length % 2) hex = "0" + hex;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++)
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}
function hexToBytes(hex) {
  if (hex.length % 2) hex = "0" + hex;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++)
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}
function bytesToHex(b) {
  return Array.from(b)
    .map((x) => x.toString(16).padStart(2, "0"))
    .join("");
}

function concat(...parts) {
  const out = new Uint8Array(parts.reduce((s, p) => s + p.length, 0));
  let off = 0;
  for (const p of parts) { out.set(p, off); off += p.length; }
  return out;
}

function modPow(base, exp, mod) {
  let r = 1n;
  base %= mod;
  while (exp > 0n) {
    if (exp & 1n) r = (r * base) % mod;
    exp >>= 1n;
    base = (base * base) % mod;
  }
  return r;
}

function isProbablePrime(n) {
  if (n < 2n) return false;
  if (n === 2n) return true;
  if (!(n & 1n)) return false;
  let d = n - 1n, r = 0;
  while (!(d & 1n)) { d >>= 1n; r++; }
  outer: for (const a of [2n,3n,5n,7n,11n,13n,17n,19n,23n,29n,31n,37n]) {
    if (a >= n) continue;
    let x = modPow(a, d, n);
    if (x === 1n || x === n - 1n) continue;
    for (let i = 0; i < r - 1; i++) {
      x = (x * x) % n;
      if (x === n - 1n) continue outer;
    }
    return false;
  }
  return true;
}

async function hashToPrime(xBytes, yBytes) {
  const prefix = new TextEncoder().encode("redaptcha-vdf-l");
  const nonceBuf = new Uint8Array(8);
  const view = new DataView(nonceBuf.buffer);
  for (let nonce = 0; ; nonce++) {
    view.setBigUint64(0, BigInt(nonce), true);
    const hash = await sha256(concat(prefix, xBytes, yBytes, new Uint8Array(nonceBuf)));
    let c = BigInt("0x" + bytesToHex(hash));
    c |= 1n;
    if (isProbablePrime(c)) return c;
  }
}

function floorTwoPowTDiv(t, l) {
  return (1n << BigInt(t)) / l;
}

export function trajectoryBytes(clicks) {
  const s = clicks.map((c) => `${c.x},${c.y},${c.t};`).join("");
  return new TextEncoder().encode(s);
}

export async function evalVdf(seedHex, modulusHex, t, clicks, onProgress) {
  const N = BigInt("0x" + modulusHex);
  const prefix = new TextEncoder().encode("redaptcha-vdf-x");
  const xHash = await sha256(
    concat(prefix, hexToBytes(seedHex), trajectoryBytes(clicks))
  );
  const x = (BigInt("0x" + bytesToHex(xHash)) % (N - 2n)) + 2n;
  let y = x;
  const BATCH = 5000;
  for (let i = 0; i < t; i += BATCH) {
    const end = Math.min(i + BATCH, t);
    for (let j = i; j < end; j++) y = (y * y) % N;
    if (onProgress) onProgress(end / t);
  }
  const xBytes = bigIntToBytesBE(x);
  const yBytes = bigIntToBytesBE(y);
  const l = await hashToPrime(xBytes, yBytes);
  const q = floorTwoPowTDiv(t, l);
  return {
    outputHex: y.toString(16),
    proofHex: modPow(x, q, N).toString(16),
  };
}