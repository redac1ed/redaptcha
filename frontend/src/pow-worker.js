import { argon2id } from "hash-wasm";

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return out;
}

function leadingZeroBits(bytes) {
  let count = 0;
  for (const b of bytes) {
    if (b === 0) {
      count += 8;
      continue;
    }
    let x = b;
    while ((x & 0x80) === 0) {
      count += 1;
      x = (x << 1) & 0xff;
    }
    break;
  }
  return count;
}

self.onmessage = async (e) => {
  const { saltHex, mCost, tCost, pCost, bits } = e.data;
  try {
    const salt = hexToBytes(saltHex);
    let nonce = 0;
    for (;;) {
      const hashHex = await argon2id({
        password: String(nonce),
        salt,
        parallelism: pCost,
        iterations: tCost,
        memorySize: mCost,
        hashLength: 32,
        outputType: "hex",
      });
      if (leadingZeroBits(hexToBytes(hashHex)) >= bits) {
        self.postMessage({ type: "done", nonce, hashHex });
        return;
      }
      nonce += 1;
      if (nonce % 8 === 0) {
        self.postMessage({ type: "progress", nonce });
      }
    }
  } catch (err) {
    self.postMessage({ type: "error", message: err.message });
  }
};