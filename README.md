# redaptcha
Another CAPTCHA that protects websites from bots and all.

# Types of CAPTCHAs:
- **Click CAPTCHA**, where you have to click on the targets moving at random speeds and directions 
- **Puzzle CAPTCHA**, where you have to click a hole that fits the piece in an image (currently using a gradient). 

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
## Development:
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

## Production:
- Add the enviromental variables (see below) in `.env.local`.
- Build and run the server:
```bash
docker compose up -d
```

# Configurations:
- `REDAPTCHA_TOKEN_KEY`: B64 of 32 bytes; HMAC key for unlock tokens (auto-generated if not set).
- `REDAPTCHA_CHALLENGE_KEY`: B64 of 32 bytes; HMAC key for challenge signing/derivation (auto-generated if not set).
- `REDAPTCHA_MODULUS_HEX`: Persisted RSA modulus (auto-generated if not set).
- `REDIS_URL` (optional): Redeem log store. Falls back to in-memory if not set.
- `VITE_SERVER` (optional, frontend): API base URL, defaults to the same origin. 
- `REDAPTCHA_ENV` (optional): Set to `production` to enable prod. hardening (requires all the above secrets to be set).
- `PORT` (optional): Server listening port, defaults to `3000`.
- `REDAPTCHA_VPN_BLOCK` (optional): Set to `true` to block known VPN/proxy IPs. Off by default. When enabled, a public VPN IP list is fetched at startup and refreshed every 6h; blocking fails open (keeps the last good list, or none, if the fetch fails).
- `REDAPTCHA_VPN_BLOCK_UNKNOWN` (optional): When VPN blocking is on, also block IPs that can't be classified (e.g. unparseable or IPv6 without a v4 mapping). Stricter; may reject legitimate users.
- `REDAPTCHA_BLOCK_CIDRS` (optional): Path to a file of extra CIDR ranges (one per line, `#` comments allowed) to block in addition to the fetched list.

# Usage:
To use this in your website/app, add this script to your HTML page:
```html
<form action="/submit" method="POST">
  <div class="redaptcha" data-sitekey="rk_live_demo"></div>
  <button type="submit">Submit</button>
</form>
<script type="module"
        src="https://your-host/redaptcha.js"
        data-endpoint="https://your-host"></script>
```

Note: replace `https://your-host` with the actual URL and `rk_live_demo` with an actual site key. This is meant to be an example script that you can use, but I recommend using this as per your needs.

# License:
This project is open source and available under the MIT License.

# Authors:
Created by redac1ed. 