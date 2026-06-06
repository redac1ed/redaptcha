const SERVER = import.meta.env.VITE_SERVER ?? "";
const SITE_KEY = "rk_site_demo";
const canvas = document.getElementById("puzzle");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const checkbox = document.getElementById("checkbox");
const captchaEl = document.getElementById("captcha");
const panel = document.getElementById("puzzle-panel");
const timerBar = document.getElementById("timer-bar");
const W = canvas.width, H = canvas.height;
const TIME_LIMIT = 15000;
const puzzleImg = new Image();
const EXPECTED_CLICKS = 3;
let imgLoaded = false;

let state = "idle";
let challenge = null;
let deadline = 0;
let worker = null;
let vdfProgress = 0;
let clicks = [];
let marks = [];
let puzzleStart = 0;
let trail = [];
let framesImg = new Image();
let framesLoaded = false;
let frameCount = 0;
let frameDt = 90;
let frameTiles = [];

function setUi() {
  captchaEl.classList.toggle("is-loading", state === "vdf");
  captchaEl.classList.toggle("is-success", state === "success");
  captchaEl.classList.toggle("is-error", state === "failed");
  panel.classList.toggle("is-open", state === "active" || state === "vdf");
  checkbox.disabled = state !== "idle" && state !== "success" && state !== "failed";
}

let lastDrawnIdx = -1;
function draw(vdfProgress) {
  if (state === "active" && framesLoaded && frameTiles.length > 0) {
    const t = Date.now() - puzzleStart;
    const idx = Math.floor(t / frameDt) % frameTiles.length;
    if (idx !== lastDrawnIdx || marks.length > 0) {
      ctx.drawImage(frameTiles[idx], 0, 0);
      for (const m of marks) {
        ctx.beginPath();
        ctx.arc(m.x, m.y, 20, 0, Math.PI * 2);
        ctx.fillStyle = "#00103d";
        ctx.fill();
        ctx.strokeStyle = "#1697f9";
        ctx.lineWidth = 2;
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(m.x - 7, m.y - 7);
        ctx.lineTo(m.x + 7, m.y + 7);
        ctx.moveTo(m.x + 7, m.y - 7);
        ctx.lineTo(m.x - 7, m.y + 7);
        ctx.strokeStyle = "#edfffd";
        ctx.stroke();
      }
      lastDrawnIdx = idx;
    }
    return;
  }
  lastDrawnIdx = -1;
  ctx.fillStyle = "#00103d";
  ctx.fillRect(0, 0, W, H);
  if (state === "vdf") {
    ctx.fillStyle = "rgba(11,15,26,0.7)";
    ctx.fillRect(0, 0, W, H);
    const barW = Math.round((vdfProgress ?? 0) * (W - 40));
    ctx.fillStyle = "#1697f9";
    ctx.fillRect(20, H / 2 - 8, barW, 16);
    ctx.strokeStyle = "#edfffd";
    ctx.strokeRect(20, H / 2 - 8, W - 40, 16);
  }
}

function loop() {
  if (state === "active") {
    const ratio = Math.max(0, (deadline - Date.now()) / TIME_LIMIT);
    timerBar.style.transform = `scaleX(${ratio})`;
    timerBar.style.backgroundColor = `hsl(${Math.round(ratio * 120)}, 80%, 55%)`;
    if (Date.now() >= deadline) fail();
  }
  draw(vdfProgress);
  requestAnimationFrame(loop);
}

async function startChallenge() {
  statusEl.textContent = "Fetching challenge…";
  setUi();
  try {
    const res = await fetch(`${SERVER}/challenge`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ site_key: SITE_KEY, hostname: location.hostname }),
    });
    if (!res.ok) {
      statusEl.textContent = `Server error ${res.status}`;
      state = "idle"; setUi(); return;
    }
    challenge = await res.json();
    if (!challenge || !challenge.modulus_hex || !challenge.seed_hex || !challenge.frames_b64) {
      statusEl.textContent = "Bad challenge from server";
      state = "idle"; setUi(); return;
    }
  } catch {
    statusEl.textContent = "Could not reach server";
    state = "idle"; setUi(); return;
  }
  frameCount = challenge.frame_count || 0;
  frameDt = challenge.frame_dt_ms || 90;
  framesLoaded = false;
  const img = new Image();
  await new Promise((resolve) => {
    img.onload = () => resolve(true);
    img.onerror = () => resolve(false);
    img.src = `data:image/png;base64,${challenge.frames_b64}`;
  });
  framesImg = img;
  framesLoaded = img.complete && img.naturalWidth > 0;
  if (!framesLoaded) {
    statusEl.textContent = "Could not load puzzle";
    state = "idle"; setUi(); return;
  }
  frameTiles = [];
  for (let i = 0; i < frameCount; i++) {
    const tile = document.createElement("canvas");
    tile.width = W;
    tile.height = H;
    tile.getContext("2d").drawImage(img, i * W, 0, W, H, 0, 0, W, H);
    frameTiles.push(tile);
  }
  state = "active";
  clicks = [];
  marks = [];
  trail = [];
  puzzleStart = Date.now();
  deadline = Date.now() + TIME_LIMIT;
  timerBar.style.opacity = "1";
  timerBar.style.transform = "scaleX(1)";
  statusEl.textContent = `Click all ${EXPECTED_CLICKS} targets`;
  setUi();
}

function puzzleSolved() {
  state = "vdf";
  vdfProgress = 0;
  timerBar.style.opacity = "0";
  statusEl.textContent = "Verifying… 0%";
  setUi();
  worker = new Worker(new URL("./vdf-worker.js", import.meta.url), { type: "module" });
  worker.onmessage = async (e) => {
    if (e.data.type === "progress") {
      vdfProgress = e.data.progress;
      statusEl.textContent = `Verifying… ${Math.round(e.data.progress * 100)}%`;
    } else if (e.data.type === "done") {
      await submitSolution(e.data.outputHex, e.data.proofHex);
    } else if (e.data.type === "error") {
      statusEl.textContent = `Error: ${e.data.message}`;
      state = "failed";
      setUi();
    }
  };
  worker.postMessage({
    seedHex: challenge.seed_hex,
    modulusHex: challenge.modulus_hex,
    difficulty: challenge.difficulty,
    clicks,
  });
}
async function submitSolution(outputHex, proofHex) {
  statusEl.textContent = "Submitting…";
  try {
    const res = await fetch(`${SERVER}/verify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        challenge_id: challenge.id,
        output_hex: outputHex,
        proof_hex: proofHex,
        clicks,
        trail,
        sig: challenge.sig,
      }),
    });
    const result = await res.json();
    if (result.ok && result.token) {
      state = "success";
      statusEl.textContent = "Success!";
      setUi();
      unlockContent(result.token);   
      return;
    } else {
      state = "failed";
      statusEl.textContent = `Failed: ${result.message}`;
    }
  } catch {
    state = "failed";
    statusEl.textContent = "Network error";
  }
  setUi();
}

async function unlockContent(token) {
  const gate = document.getElementById("restricted");
  const hint = gate.querySelector(".lock-hint");
  try {
    const res = await fetch(`${SERVER}/content`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token }),
    });
    if (!res.ok) {
      hint.textContent = `Server error ${res.status}`;
      return;
    }
    const data = await res.json();
    if (data.ok) {
      document.getElementById("restricted-body").textContent = data.content;
      gate.classList.add("is-unlocked");
    } else {
      hint.textContent = "Token rejected.";
    }
  } catch {
    hint.textContent = "Could not load content.";
  }
}

function fail() {
  state = "failed";
  timerBar.style.opacity = "0";
  statusEl.textContent = "Expired — click to retry";
  setUi();
}
function reset() {
  if (worker) { worker.terminate(); worker = null; }
  state = "idle";
  clicks = [];
  marks = [];
  trail = [];
  frameTiles = [];
  framesLoaded = false;
  timerBar.style.opacity = "0";
  statusEl.textContent = "Verify you are human";
  setUi();
}
checkbox.addEventListener("click", () => {
  if (state === "idle" || state === "failed") {
    startChallenge();
  } else if (state === "success") {
    reset();
  }
});
canvas.addEventListener("click", (e) => {
  if (state !== "active") return;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  const t = Date.now() - puzzleStart;
  clicks.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
  marks.push({ x: cx, y: cy });
  const left = EXPECTED_CLICKS - clicks.length;
  statusEl.textContent = left > 0 ? `${left} left` : "";
  if (clicks.length >= EXPECTED_CLICKS) puzzleSolved();
});
canvas.addEventListener("pointermove", (e) => {
  if (state !== "active") return;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(Date.now() - puzzleStart) });
  if (trail.length > 400) trail.shift();
});

reset();
loop();