# Roadmap

Feel free to submit PRs/issues for anything you want to work on.

## Phase 1 (MVP)

- [x] VDF System (`crates/vdf/`)
  - [x] Wesolowski VDF over RSA group
    - [x] Sequential modular squaring eval (`t` steps)
    - [x] Fiat-Shamir hash-to-prime challenge
    - [x] `floor(2^t / l)` long division without materialising `2^t`
    - [x] Fast verify (constant time, independent of `t`)
  - [x] Unit tests (eval+verify passes, tampered output fails, wrong `t` fails)

- [ ] Core Types (`crates/core/`)
  - [ ] `Challenge { id, seed: [u8;32], difficulty: u64, expires_at }`
  - [ ] `Solution { challenge_id, vdf_output, vdf_proof }`

- [ ] Server (`crates/server/`)
  - [ ] Axum setup
    - [ ] `POST /challenge` — issue challenge (random seed + difficulty)
    - [ ] `POST /verify` — check VDF proof, return signed token
  - [ ] In-memory challenge store with expiry
  - [ ] CORS headers
  - [ ] Per-IP rate limiting (one active challenge at a time)

- [ ] Frontend (`frontend/`)
  - [x] Vite + JS project
  - [x] Moving noise dot field (canvas)
  - [x] Bouncing orange targets — click to hit
  - [x] Countdown timer bar (green ? red)
  - [x] Pass / fail overlay
  - [ ] Targets speed up over time
  - [ ] Mobile touch support
  - [ ] Talk to server — submit solution after puzzle is solved

## Phase 2 (WASM Integration)

- [ ] WASM Bridge (`crates/client-wasm/`)
  - [ ] `wasm-bindgen` + `wasm-pack` setup
  - [ ] Export `eval(seed_hex, difficulty)` ? `{ output_hex, proof_hex }`
  - [ ] Run VDF eval in a Web Worker (UI stays at 60 FPS)
  - [ ] Frontend loads WASM, calls eval, posts result to `/verify`

- [ ] Trajectory Binding
  - [ ] Record click timestamps and coordinates during puzzle
  - [ ] `vdf_seed = SHA256(server_seed || trajectory_hash)`
  - [ ] Coordinates never leave the client

## Phase 3 (Hardening)

- [ ] Security
  - [ ] Replace test RSA modulus with 2048-bit trusted-setup modulus
  - [ ] Migrate `crates/vdf` from `num-bigint` to `crypto-bigint` (constant-time)
  - [ ] HMAC-signed tokens on `/verify` response
  - [ ] HTTPS enforcement

- [ ] Reliability
  - [ ] Persistent challenge store (SQLite or Redis)
  - [ ] Difficulty auto-calibration (server benchmarks client speed on first request)
  - [ ] Structured logging + error codes

- [ ] Distribution
  - [ ] Embeddable `<script>` tag widget
  - [ ] NPM package for the frontend widget
  - [ ] Rust SDK crate for server-side token verification

## Phase 4 (Advanced / v2)

- [ ] Chaotic dot motion seeded from server nonce (Logistic map)
- [ ] Continuous VDF — inject trajectory coordinates into VDF state each frame
- [ ] Zero-Knowledge Proofs — prove trajectory valid without revealing coordinates to server
- [ ] WASM SIMD — `RUSTFLAGS="-C target-feature=+simd128"` for faster proving
