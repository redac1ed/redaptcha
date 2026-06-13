# redaptcha
Another CAPTCHA that protects websites from bots and all.

# Types of CAPTCHAs:
- **Click CAPTCHA**, where you have to click on the targets moving at random speeds and directions 
- **Slider CAPTCHA**, where you have to drag a piece of the image (currently using a gradient) to the correct position.

# Features:
- **VDF time lock**: Every solve runs a Wesolowski Verifiable Delay Function (VDF), a sequential PoW over a RSA group in the browser to force a minimum wall-clock cost per attempt.
- **Server-side grading**: Puzzle solves are graded on the server for better security and flexibility.
- **No answer stored at rest**: The answers/solutions are received deterministically from a secret challenge key + challenge ID (which does not persist).
- **Signed challenges**: Every challenge is HMAC signed to prevent a forged challenge ID from being accepted before grading.
- **Three rounds**: Multi-round verification is enforced (per-IP) on the server and the frontend.
- **Anti-Bot scoring**: Validates cursor movements (reaction time, smoothness, teleport detection etc.), click timings, drag trajectory and other client behaviour for bot detection.
- **Single-use tokens**: Every successfully solved challenge gets a single-use HMAC token, backed by a Redis redeem log/in-memory storage to prevent reuse.
- and more!

# How to use:
- Run the server:
```bash
cargo run -p server
```
- Run the frontend: (optional, useful for testing)
```bash
cd frontend
npm install 
npm run dev
```

# Configurations:
- `REDAPTCHA_TOKEN_KEY`: B64 of 32 bytes; HMAC key for unlock tokens (auto-generated if not set).
- `REDAPTCHA_CHALLENGE_KEY`: B64 of 32 bytes; HMAC key for challenge signing/derivation (auto-generated if not set).
- `REDAPTCHA_MODULUS_HEX`: Persisted RSA modulus (auto-generated if not set).
- `REDIS_URL` (optional): Redeem log store. Falls back to in-memory if not set.
- `VITE_SERVER` (optional, frontend): API base URL, defaults to the same origin. 
- `REDAPTCHA_ENV` (optional): Set to `production` to enable prod. hardening (requires all the above secrets to be set).
- `PORT` (optional): Server listening port, defaults to `3000`.

# License:
This project is open source and available under the MIT License.

# Authors:
Created by redac1ed. 