const SERVER = import.meta.env.VITE_SERVER ?? "";
const SITE_KEY = "rk_site_demo";
const CAPTCHA_KIND = import.meta.env.VITE_CAPTCHA_KIND ?? "one";
const canvas = document.getElementById("puzzle");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const checkbox = document.getElementById("checkbox");
const captchaEl = document.getElementById("captcha");
const panel = document.getElementById("puzzle-panel");
const timerBar = document.getElementById("timer-bar");
const progressEl = document.getElementById("puzzle-progress");
const W = canvas.width, H = canvas.height;
const TIME_LIMIT = 15000;
const EXPECTED_CLICKS = 3;
const HIT_R = 22;
const TARGET_R = 16;
const SLIDER_ROUNDS = 3

let state = "idle";
let challenge = null;
let deadline = 0;
let worker = null;
let vdfProgress = 0;
let clicks = [];
let trail = [];
let puzzleStart = 0;
let frameCount = 0;
let frameDt = 50;
let panelStrips = [];
let panelTiles = [];
let panelMotions = [];
let panelIdx = 0;
let flashUntil = 0;
let inputType = "mouse";
let pointerDown = false;
let sliderHint = null;
let pieceX = 0;
let pieceY = 0;
let dragging = false; 
let dragStartT = 0;
let isSlider = false;
let pieceImg = null;
let dragDX = 0;
let dragDY = 0;
let lastDrawnIdx = -1;
let sliderRound = 0;

function setUi() {
  captchaEl.classList.toggle("is-loading", state === "vdf");
  captchaEl.classList.toggle("is-success", state === "success");
  captchaEl.classList.toggle("is-error", state === "failed");
  panel.classList.toggle("is-open", state === "active" || state === "vdf");
  checkbox.disabled = state !== "idle" && state !== "success" && state !== "failed";
}

function tTotal() {
  return frameCount * frameDt;
}

function ballPosAtFrame(idx) {
  const m = panelMotions[panelIdx];
  if (!m) return { cx: W / 2, cy: H / 2 };
  const t = idx * frameDt;
  const w = m.dir * (m.turns * 2 * Math.PI * t) / tTotal();
  const cx = m.cx + m.amp * Math.sin(m.phase + w) + m.amp * 0.4 * Math.sin(m.phase + 2.3 * w);
  const cy = m.cy + m.amp * Math.sin(m.phase * 1.7 + 1.3 * w) + m.amp * 0.4 * Math.cos(m.phase + 1.9 * w);
  return { cx, cy };
}

function draw(vp) {
  if (state === "active" && isSlider) {
    const bg = panelStrips[0];
    if (bg) ctx.drawImage(bg, 0, 0, W, H);
    if (pieceImg && sliderHint) {
      const flashing = Date.now() < flashUntil;
      ctx.globalAlpha = flashing ? 1 : 0.95;
      ctx.drawImage(pieceImg, pieceX, pieceY, sliderHint.piece_w, sliderHint.piece_h);
      ctx.globalAlpha = 1;
    }
    return;
  }
  lastDrawnIdx = -1;
  ctx.fillStyle = "#00103d";
  ctx.fillRect(0, 0, W, H);
  if (state === "vdf") {
    ctx.fillStyle = "rgba(11,15,26,0.7)";
    ctx.fillRect(0, 0, W, H);
    const barW = Math.round((vp ?? 0) * (W - 40));
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

function buildTilesFor(idx) {
  panelTiles = [];
  const img = panelStrips[idx];
  if (!img) return;
  for (let i = 0; i < frameCount; i++) {
    const tile = document.createElement("canvas");
    tile.width = W;
    tile.height = H;
    tile.getContext("2d").drawImage(img, i * W, 0, W, H, 0, 0, W, H);
    panelTiles.push(tile);
  }
  lastDrawnIdx = -1;
}

function setProgress() {
  if (!progressEl) return;
  if (isSlider) {
    progressEl.textContent = `Round ${sliderRound + 1} / ${SLIDER_ROUNDS}`;
  } else {
    progressEl.textContent = `${panelIdx} / ${EXPECTED_CLICKS} done`;
  }
}

async function startChallenge() {
  statusEl.textContent = "Fetching challenge…";
  setUi();
  try {
    const res = await fetch(`${SERVER}/challenge/${CAPTCHA_KIND}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ site_key: SITE_KEY, hostname: location.hostname }),
    });
    if (!res.ok) {
      statusEl.textContent = `Server error ${res.status}`;
      state = "idle"; setUi(); return;
    }
    challenge = await res.json();
    if (
      !challenge ||
      !challenge.modulus_hex ||
      !challenge.seed_hex ||
      !Array.isArray(challenge.frames_b64) ||
      challenge.frames_b64.length === 0
    ) {
      statusEl.textContent = "Bad challenge from server";
      state = "idle"; setUi(); return;
    }
  } catch {
    statusEl.textContent = "Could not reach server";
    state = "idle"; setUi(); return;
  }
  frameCount = challenge.frame_count || 0;
  frameDt = challenge.frame_dt_ms || 50;
  panelMotions = Array.isArray(challenge.motions) ? challenge.motions : [];
  isSlider = challenge.kind === "two";
  sliderHint = challenge.slider || null;
  panelStrips = [];
  for (const b64 of challenge.frames_b64) {
    const img = new Image();
    const ok = await new Promise((resolve) => {
      img.onload = () => resolve(true);
      img.onerror = () => resolve(false);
      img.src = `data:image/png;base64,${b64}`;
    });
    if (!ok || !(img.complete && img.naturalWidth > 0)) {
      statusEl.textContent = "Could not load puzzle";
      state = "idle"; setUi(); return;
    }
    panelStrips.push(img);
  }
  state = "active";
  clicks = [];
  trail = [];
  panelIdx = 0;
  flashUntil = 0;
  puzzleStart = Date.now();
  if (isSlider && sliderHint) {
    pieceImg = panelStrips[1] || null;
    pieceX = sliderHint.start_x;
    pieceY = sliderHint.start_y;
    dragging = false;
    statusEl.textContent = "Drag the piece into the gap";
  } else {
    statusEl.textContent = "Click the moving ball";
  }
  deadline = Date.now() + TIME_LIMIT;
  buildTilesFor(0);
  setProgress();
  timerBar.style.opacity = "1";
  timerBar.style.transform = "scaleX(1)";
  setUi();
}

function advancePanel() {
  panelIdx += 1;
  setProgress();
  if (panelIdx >= EXPECTED_CLICKS) {
    puzzleSolved();
    return;
  }
  buildTilesFor(panelIdx);
  statusEl.textContent = "Click the moving ball";
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
        input_type: inputType,
      }),
    });
    const result = await res.json();
    if (result.ok && result.token) {
      if (isSlider && sliderRound < SLIDER_ROUNDS - 1) {
        sliderRound += 1;
        if (worker) { worker.terminate(); worker = null; }
        statusEl.textContent = `Round ${sliderRound + 1} of ${SLIDER_ROUNDS}…`;
        await startChallenge();
        return;
      }
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
  trail = [];
  panelStrips = [];
  panelTiles = [];
  panelMotions = [];
  panelIdx = 0;
  flashUntil = 0;
  inputType = "mouse";
  pointerDown = false;
  sliderHint = null;
  pieceX = 0;
  pieceY = 0;
  dragging = false;
  isSlider = false;
  dragDX = 0;
  dragDY = 0;
  pieceImg = null;
  sliderRound = 0
  if (progressEl) progressEl.textContent = "";
  timerBar.style.opacity = "0";
  statusEl.textContent = "Verify you are human";
  setUi();
}

checkbox.addEventListener("click", () => {
  if (state === "idle" || state === "failed") {
    sliderRound = 0;
    startChallenge();
  } else if (state === "success") {
    reset();
  }
});

canvas.addEventListener("click", (e) => {
  if (state !== "active") return;
  if (isSlider) return;
  if (Date.now() < flashUntil) return;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  const t = Date.now() - puzzleStart;
  const seenIdx = Math.floor(t / frameDt) % frameCount;
  const { cx: bx, cy: by } = ballPosAtFrame(seenIdx);
  const hit = Math.hypot(cx - bx, cy - by) <= HIT_R;
  if (!hit) {
    statusEl.textContent = "Missed — click the ball";
    return;
  }
  clicks.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
  flashUntil = Date.now() + 350;
  statusEl.textContent = "Nice!";
  setTimeout(advancePanel, 360);
});

canvas.addEventListener("pointerdown", (e) => {
  if (state !== "active") return;
  if (isSlider) return;
  inputType = e.pointerType || "mouse";
  pointerDown = true;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(Date.now() - puzzleStart) });
  if (trail.length > 400) trail.shift();
});

canvas.addEventListener("pointermove", (e) => {
  if (state !== "active") return;
  if (isSlider) return;
  if (e.pointerType === "touch" && !pointerDown) return;
  inputType = e.pointerType || inputType;
  const rect = canvas.getBoundingClientRect();
  const cx = (e.clientX - rect.left) * (W / rect.width);
  const cy = (e.clientY - rect.top) * (H / rect.height);
  trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(Date.now() - puzzleStart) });
  if (trail.length > 400) trail.shift();
});

canvas.addEventListener("pointerup", () => { pointerDown = false; });
canvas.addEventListener("pointercancel", () => { pointerDown = false; });

canvas.addEventListener("pointerdown", (e) => {
  if (state !== "active" || !isSlider || !sliderHint) return;
  const rect = canvas.getBoundingClientRect();
  const x = (e.clientX - rect.left) * (W / rect.width);
  const y = (e.clientY - rect.top) * (H / rect.height);
  if (
    x >= pieceX && x <= pieceX + sliderHint.piece_w &&
    y >= pieceY && y <= pieceY + sliderHint.piece_h
  ) {
    dragging = true;
    inputType = e.pointerType || "mouse";
    dragDX = x - pieceX;
    dragDY = y - pieceY;
    dragStartT = Date.now() - puzzleStart;
    clicks = [{ x: Math.round(pieceX), y: Math.round(pieceY), t: Math.round(dragStartT) }];
    try { canvas.setPointerCapture(e.pointerId); } catch {}
  }
});

canvas.addEventListener("pointermove", (e) => {
  if (state !== "active" || !isSlider || !dragging || !sliderHint) return;
  const rect = canvas.getBoundingClientRect();
  const x = (e.clientX - rect.left) * (W / rect.width);
  const y = (e.clientY - rect.top) * (H / rect.height);
  pieceX = Math.max(0, Math.min(W - sliderHint.piece_w, x - dragDX));
  pieceY = Math.max(0, Math.min(H - sliderHint.piece_h, y - dragDY));
  trail.push({ x: Math.round(x), y: Math.round(y), t: Math.round(Date.now() - puzzleStart) });
  if (trail.length > 400) trail.shift();
});

canvas.addEventListener("pointerup", () => {
  if (state !== "active" || !isSlider || !dragging || !sliderHint) return;
  dragging = false;
  const t = Date.now() - puzzleStart;
  clicks.push({ x: Math.round(pieceX), y: Math.round(pieceY), t: Math.round(t) });
  flashUntil = Date.now() + 350;
  statusEl.textContent = "Checking…";
  setTimeout(puzzleSolved, 360);
});

reset();
loop();