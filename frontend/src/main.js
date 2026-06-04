const SERVER = "http://localhost:3000";
const canvas = document.getElementById("puzzle");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const checkbox = document.getElementById("checkbox");
const captchaEl = document.getElementById("captcha");
const panel = document.getElementById("puzzle-panel");
const timerBar = document.getElementById("timer-bar");
const W = canvas.width, H = canvas.height;
const NOISE_COUNT = 100, TARGET_COUNT = 3, TARGET_R = 14, TIME_LIMIT = 15000;
const noise = Array.from({ length: NOISE_COUNT }, () => ({
  x: Math.random() * W, y: Math.random() * H,
  vx: (Math.random() - 0.5) * 1.2, vy: (Math.random() - 0.5) * 1.2,
}));

let state = "idle";
let challenge = null;
let targets = [], remaining = 0, deadline = 0;
let worker = null;
let vdfProgress = 0;
let clicks = [];
let puzzleStart = 0;

function setUi() {
  captchaEl.classList.toggle("is-loading", state === "vdf");
  captchaEl.classList.toggle("is-success", state === "success");
  captchaEl.classList.toggle("is-error", state === "failed");
  panel.classList.toggle("is-open", state === "active" || state === "vdf");
  checkbox.disabled = state !== "idle" && state !== "success" && state !== "failed";
}

function moveNoise() {
  for (const d of noise) {
    d.x += d.vx; d.y += d.vy;
    if (d.x < 0 || d.x > W) d.vx *= -1;
    if (d.y < 0 || d.y > H) d.vy *= -1;
  }
}
function spawnTargets() {
  targets = Array.from({ length: TARGET_COUNT }, () => ({
    x: TARGET_R + Math.random() * (W - TARGET_R * 2),
    y: TARGET_R + Math.random() * (H - TARGET_R * 2),
    vx: (Math.random() - 0.5) * 2,
    vy: (Math.random() - 0.5) * 2,
    hit: false,
  }));
}
function moveTargets() {
  for (const t of targets) {
    if (t.hit) continue;
    t.x += t.vx; t.y += t.vy;
    if (t.x < TARGET_R || t.x > W - TARGET_R) t.vx *= -1;
    if (t.y < TARGET_R || t.y > H - TARGET_R) t.vy *= -1;
  }
}

function draw(vdfProgress) {
  ctx.fillStyle = "#00103d";
  ctx.fillRect(0, 0, W, H);
  ctx.fillStyle = "#ff5e8c";
  for (const d of noise) {
    ctx.beginPath(); ctx.arc(d.x, d.y, 2, 0, Math.PI * 2); ctx.fill();
  }
  if (state === "active") {
    for (const t of targets) {
      if (t.hit) continue;
      ctx.beginPath(); ctx.arc(t.x, t.y, TARGET_R, 0, Math.PI * 2);
      ctx.fillStyle = "#1697f9"; ctx.fill();
      ctx.strokeStyle = "#edfffd"; ctx.lineWidth = 1.5; ctx.stroke();
    }
  }
  if (state === "vdf") {
    ctx.fillStyle = "rgba(11,15,26,0.7)";
    ctx.fillRect(0, 0, W, H);
    const barW = Math.round((vdfProgress ?? 0) * (W - 40));
    ctx.fillStyle = "#2563eb";
    ctx.fillRect(20, H / 2 - 6, barW, 12);
    ctx.strokeStyle = "#3b82f6";
    ctx.strokeRect(20, H / 2 - 6, W - 40, 12);
  }
}

function loop() {
  moveNoise();
  if (state === "active") {
    moveTargets();
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
    const res = await fetch(`${SERVER}/challenge`, { method: "POST" });
    challenge = await res.json();
  } catch {
    statusEl.textContent = "Could not reach server";
    state = "idle";
    setUi();
    return;
  }
  state = "active";
  clicks = [];
  puzzleStart = Date.now();
  spawnTargets();
  remaining = TARGET_COUNT;
  deadline = Date.now() + TIME_LIMIT;
  timerBar.style.opacity = "1";
  timerBar.style.transform = "scaleX(1)";
  statusEl.textContent = `Click all ${TARGET_COUNT} targets`;
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
      }),
    });
    const result = await res.json();
    state = result.ok ? "success" : "failed";
    statusEl.textContent = result.ok ? "Success!" : `Failed: ${result.message}`;
  } catch {
    state = "failed";
    statusEl.textContent = "Network error";
  }
  setUi();
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
  timerBar.style.opacity = "0";
  statusEl.textContent = "Verify you are human";
  setUi();
}
canvas.addEventListener("click", (e) => {
  if (state !== "active") return;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  for (const t of targets) {
    if (t.hit) continue;
    const dx = cx - t.x, dy = cy - t.y;
    if (Math.sqrt(dx * dx + dy * dy) <= TARGET_R + 4) {
      t.hit = true;
      remaining--;
      clicks.push({
        x: Math.round(cx),
        y: Math.round(cy),
        t: Math.round(Date.now() - puzzleStart),
      });
      statusEl.textContent = remaining > 0 ? `${remaining} left` : "";
      if (remaining === 0) puzzleSolved();
      break;
    }
  }
});

checkbox.addEventListener("click", () => {
  if (state === "success") return;
  if (state === "failed") { reset(); startChallenge(); return; }
  if (state === "idle") startChallenge();
});

reset();
loop();