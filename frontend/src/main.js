const SERVER = import.meta.env.VITE_SERVER ?? "";
const SITE_KEY = "rk_site_demo";
const W = 320, H = 240;
const TIME_LIMIT = 15000;
const EXPECTED_CLICKS = 3;
const HIT_R = 22;
const SLIDER_ROUNDS = 3;
const PURSUIT_MS = 4200;

const session = {
  pageLoad: Date.now(),
  firstMove: null,
  focus: 0,
  blur: 0,
  scroll: 0,
  key: 0,
  move: 0,
  hasTouch: false,
  maxPressure: 0,
  pointerKinds: new Set(),
  hiddenStart: null,
  hiddenMs: 0,
  moveTrail: [],
};

export function setupTelemetry() {
  window.addEventListener("focus", () => { session.focus += 1; });
  window.addEventListener("blur", () => { session.blur += 1; });
  window.addEventListener("scroll", () => { session.scroll += 1; }, { passive: true });
  window.addEventListener("keydown", () => { session.key += 1; });
  window.addEventListener("pointermove", (e) => {
    session.move += 1;
    if (session.firstMove === null) session.firstMove = Date.now() - session.pageLoad;
    if (e.pointerType) session.pointerKinds.add(e.pointerType);
    if (e.pointerType === "touch" || e.pointerType === "pen") session.hasTouch = true;
    if (typeof e.pressure === "number" && e.pressure > session.maxPressure) {
      session.maxPressure = e.pressure;
    }
    if (session.move % 3 === 0) {
      session.moveTrail.push({
        x: Math.round(e.clientX),
        y: Math.round(e.clientY),
        t: Math.round(Date.now() - session.pageLoad),
      });
      if (session.moveTrail.length > 64) session.moveTrail.shift();
    }
  }, { passive: true });
  if (navigator.maxTouchPoints > 0 || "ontouchstart" in window) session.hasTouch = true;
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      session.hiddenStart = Date.now();
    } else if (session.hiddenStart !== null) {
      session.hiddenMs += Date.now() - session.hiddenStart;
      session.hiddenStart = null;
    }
  });
}

function canvasHash() {
  try {
    const c = document.createElement("canvas");
    const ctx = c.getContext("2d");
    ctx.textBaseline = "top";
    ctx.font = "14px Arial";
    ctx.fillStyle = "#069";
    ctx.fillText("redaptcha-\u{1F512}", 2, 2);
    return c.toDataURL().slice(-64);
  } catch {
    return null;
  }
}

function webglInfo() {
  try {
    const gl =
      document.createElement("canvas").getContext("webgl") ||
      document.createElement("canvas").getContext("experimental-webgl");
    if (!gl) return { hash: null, vendor: null };
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    return {
      hash: String(gl.getParameter(gl.VERSION) || "").slice(-32),
      vendor: dbg ? gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) : null,
    };
  } catch {
    return { hash: null, vendor: null };
  }
}

function perfJitter(n = 16) {
  const out = [];
  let prev = performance.now();
  for (let i = 0; i < n; i++) {
    const now = performance.now();
    out.push(now - prev);
    prev = now;
  }
  return out;
}

function rotl32(v, by) {
  const b = by & 31;
  if (b === 0) return v >>> 0;
  return ((v << b) | (v >>> (32 - b))) >>> 0;
}

function runInstrProgram(program) {
  let acc = program.seed >>> 0;
  for (const step of program.ops) {
    const k = step.k >>> 0;
    switch (step.op) {
      case "xor":
        acc = (acc ^ k) >>> 0;
        break;
      case "add":
        acc = (acc + k) >>> 0;
        break;
      case "mul":
        acc = Math.imul(acc, k | 1) >>> 0;
        break;
      case "and": {
        const r = (acc & k) >>> 0;
        acc = r === 0 ? (acc + k) >>> 0 : r;
        break;
      }
      case "or":
        acc = (acc | k) >>> 0;
        break;
      case "rotl":
        acc = rotl32(acc, k);
        break;
      case "dom": {
        const n = (k % 24) + 8;
        const host = document.createElement("div");
        host.style.display = "none";
        document.body.appendChild(host);
        let v = acc >>> 0;
        for (let i = 0; i < n; i++) {
          const node = document.createElement("span");
          node.setAttribute("data-v", String(i));
          host.appendChild(node);
          const read = parseInt(node.getAttribute("data-v"), 10) >>> 0;
          v = (v + Math.imul(read, 2654435761)) >>> 0;
          v = (v ^ rotl32(v, 7)) >>> 0;
        }
        host.remove();
        v = (v ^ n) >>> 0;
        acc = v >>> 0;
        break;
      }
      default:
        break;
    }
  }
  return acc >>> 0;
}

function telemetrySnapshot(nonce) {
  const dpr = window.devicePixelRatio || 1;
  const wg = webglInfo();
  let tzName = null;
  try {
    tzName = Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    tzName = null;
  }
  return {
    page_load_to_first_move_ms: session.firstMove,
    focus_events: session.focus,
    blur_events: session.blur,
    scroll_events: session.scroll,
    key_events: session.key,
    move_events: session.move,
    has_touch: session.hasTouch,
    max_pressure: session.maxPressure,
    pointer_kinds: Array.from(session.pointerKinds),
    screen_w: screen.width,
    screen_h: screen.height,
    viewport_w: window.innerWidth,
    viewport_h: window.innerHeight,
    device_pixel_ratio: dpr,
    webdriver: navigator.webdriver === true,
    hidden_time_ms: session.hiddenMs,
    canvas_hash: canvasHash(),
    webgl_hash: wg.hash,
    webgl_vendor: wg.vendor,
    timezone_offset_min: -new Date().getTimezoneOffset(),
    timezone_name: tzName,
    language: navigator.language || null,
    platform: navigator.platform || null,
    user_agent: navigator.userAgent || null,
    hardware_concurrency: navigator.hardwareConcurrency || 0,
    device_memory: navigator.deviceMemory || null,
    perf_jitter: perfJitter(),
    nonce_echo: nonce || null,
  };
}

function setupCarousel() {
  const scroller = document.getElementById("captcha-scroller");
  const prevBtn = document.getElementById("carousel-prev");
  const nextBtn = document.getElementById("carousel-next");
  if (!scroller || !prevBtn || !nextBtn) return;
  const cards = Array.from(scroller.querySelectorAll(".captcha-card"));
  let index = 0;
  function update() {
    scroller.style.transform = `translateX(-${index * 100}%)`;
    prevBtn.disabled = index === 0;
    nextBtn.disabled = index === cards.length - 1;
  }
  prevBtn.addEventListener("click", () => {
    if (index > 0) { index -= 1; update(); }
  });
  nextBtn.addEventListener("click", () => {
    if (index < cards.length - 1) { index += 1; update(); }
  });
  update();
};

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

export function createCaptcha(card, opts = {}) {
  const SERVER_URL = opts.server ?? SERVER;
  const SITE_KEY_W = opts.siteKey ?? SITE_KEY;
  const canvas = card.querySelector('[data-role="canvas"]');
  const ctx = canvas.getContext("2d");
  const statusEl = card.querySelector('[data-role="status"]');
  const checkbox = card.querySelector('[data-role="checkbox"]');
  const captchaEl = card.querySelector('[data-role="captcha"]');
  const panel = card.querySelector('[data-role="panel"]');
  const timerBar = card.querySelector('[data-role="timer"]');
  const progressEl = card.querySelector('[data-role="progress"]');
  const CORE_R = 7;
  const CORE_LUM_THRESHOLD = 175;
  let CAPTCHA_KIND = card.dataset.kind;
  let fallbackKind = null;
  let state = "idle";
  let challenge = null;
  let passiveIssuedAt = 0;
  let instrResult = null;
  let deadline = 0;
  let worker = null;
  let vdfProgress = 0;
  let clicks = [];
  let trail = [];
  let puzzleStart = 0;
  let frameCount = 0;
  let frameDt = 50;
  let panelStrips = [];
  let panelMotions = [];
  let panelIdx = 0;
  let flashUntil = 0;
  let inputType = "mouse";
  let pointerDown = false;
  let touchClickPending = false;
  let sliderHint = null;
  let pieceX = 0;
  let pieceY = 0;
  let dragging = false;
  let dragStartT = 0;
  let isSlider = false;
  let pieceImg = null;
  let dragDX = 0;
  let dragDY = 0;
  let sliderRound = 0;
  let sliderRounds = SLIDER_ROUNDS;
  let sliderDropped = false;
  let selX = -1;
  let selY = -1;
  let selT = 0;
  let confirmReady = false;
  let rafID = 0;
  let powNonce = null;
  let powHashHex = null;
  let isPursuit = false;
  let pursuitTimer = 0;

  function setUi() {
    captchaEl.classList.toggle("is-loading", state === "vdf");
    captchaEl.classList.toggle("is-success", state === "success");
    captchaEl.classList.toggle("is-error", state === "failed");
    card.classList.toggle("is-stepup", CAPTCHA_KIND !== card.dataset.kind);
    panel.classList.toggle("is-open", state === "active" || state === "vdf");
    checkbox.disabled = state !== "idle" && state !== "success" && state !== "failed";
  }

  function tTotal() {
    return frameCount * frameDt;
  }

  function confirmBtn() {
    const w = 90, h = 28;
    return { x: W - w - 8, y: H - h - 8, w, h };
  }

  function isBallPixel(cx, cy) {
    const scan = CORE_R + (inputType === "touch" ? 5 : 0);
    const x0 = Math.max(0, Math.floor(cx - scan));
    const y0 = Math.max(0, Math.floor(cy - scan));
    const x1 = Math.min(W, Math.ceil(cx + scan));
    const y1 = Math.min(H, Math.ceil(cy + scan));
    const w = x1 - x0;
    const h = y1 - y0;
    if (w <= 0 || h <= 0) return false;
    let img;
    try {
      img = ctx.getImageData(x0, y0, w, h).data;
    } catch {
      return true;
    }
    const r2 = scan * scan;
    for (let y = 0; y < h; y += 1) {
      for (let x = 0; x < w; x += 1) {
        const dx = x0 + x + 0.5 - cx;
        const dy = y0 + y + 0.5 - cy;
        if (dx * dx + dy * dy > r2) continue;
        const i = (y * w + x) * 4;
        const lum = img[i] * 0.299 + img[i + 1] * 0.587 + img[i + 2] * 0.114;
        if (lum > CORE_LUM_THRESHOLD) return true;
      }
    }
    return false;
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

  function draw(now, vp) {
    if (state === "active" && isSlider) {
      const bg = panelStrips[0];
      if (bg) ctx.drawImage(bg, 0, 0, W, H);
      if (pieceImg && sliderHint) {
        ctx.save();
        ctx.shadowColor = "rgba(0,0,0,0.55)";
        ctx.shadowBlur = 10;
        ctx.drawImage(pieceImg, pieceX, pieceY, sliderHint.piece_w, sliderHint.piece_h);
        ctx.restore();
        ctx.strokeStyle = "rgba(237,255,253,0.9)";
        ctx.lineWidth = 2;
        ctx.strokeRect(pieceX - 1, pieceY - 1, sliderHint.piece_w + 2, sliderHint.piece_h + 2);
      }
      if (selX >= 0) {
        ctx.strokeStyle = now < flashUntil ? "#7CFFB2" : "#22d3ee";
        ctx.lineWidth = 3;
        ctx.beginPath();
        ctx.arc(selX, selY, 30, 0, Math.PI * 2);
        ctx.stroke();
      }
      if (confirmReady) {
        const b = confirmBtn();
        ctx.fillStyle = "#1697f9";
        ctx.fillRect(b.x, b.y, b.w, b.h);
        ctx.strokeStyle = "#edfffd";
        ctx.lineWidth = 1.5;
        ctx.strokeRect(b.x, b.y, b.w, b.h);
        ctx.fillStyle = "#ffffff";
        ctx.font = "bold 14px system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText("Confirm", b.x + b.w / 2, b.y + b.h / 2);
        ctx.textAlign = "start";
        ctx.textBaseline = "alphabetic";
      }
      return;
    }
    ctx.fillStyle = "#00103d";
    ctx.fillRect(0, 0, W, H);
    if (state === "active" && !isSlider && panelStrips.length) {
      const t = now - puzzleStart;
      const idx = Math.floor(t / frameDt) % frameCount;
      ctx.drawImage(panelStrips[panelIdx], idx * W, 0, W, H, 0, 0, W, H);
      if (now < flashUntil) {
        ctx.fillStyle = "rgba(237,255,253,0.25)";
        ctx.fillRect(0, 0, W, H);
      }
      return;
    }
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

  function needsAnimation() {
    return state === "active" || state === "vdf";
  }

  function loop() {
    const now = Date.now();
    if (state === "active") {
      const ratio = Math.max(0, (deadline - now) / TIME_LIMIT);
      timerBar.style.transform = `scaleX(${ratio})`;
      timerBar.style.backgroundColor = `hsl(${Math.round(ratio * 120)}, 80%, 55%)`;
      if (now >= deadline) { fail(); }
    }
    draw(now, vdfProgress);
    if (needsAnimation()) {
      rafID = requestAnimationFrame(loop);
    } else {
      rafID = 0;
    }
  }

  function startLoop() {
    if (!rafID && needsAnimation()) {
      rafID = requestAnimationFrame(loop);
    }
  }

  function setProgress() {
    if (!progressEl) return;
    if (isSlider) {
      progressEl.textContent = `Round ${Math.min(sliderRound + 1, sliderRounds)} / ${sliderRounds}`;
    } else {
      progressEl.textContent = `${panelIdx} / ${EXPECTED_CLICKS} done`;
    }
  }

  async function startPassive() {
    statusEl.textContent = "Verifying…";
    state = "vdf";
    setUi();
    try {
      const res = await fetch(`${SERVER_URL}/challenge/${CAPTCHA_KIND}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ site_key: SITE_KEY_W, hostname: location.hostname }),
      });
      if (!res.ok) {
        statusEl.textContent = `Server error ${res.status}`;
        state = "idle"; setUi(); return;
      }
      challenge = await res.json();
      passiveIssuedAt = Date.now();
      if (!challenge || !challenge.modulus_hex || !challenge.seed_hex) {
        statusEl.textContent = "Bad challenge from server";
        state = "idle"; setUi(); return;
      }
    } catch {
      statusEl.textContent = "Could not reach server";
      state = "idle"; setUi(); return;
    }
    clicks = [];
    trail = session.moveTrail.slice();
    inputType = "mouse";
    instrResult = null;
    if (challenge.instr) {
      try {
        instrResult = runInstrProgram(JSON.parse(challenge.instr));
      } catch {
        instrResult = null;
      }
    }
    powNonce = null;
    powHashHex = null;
    if (challenge.pow) {
      statusEl.textContent = "Verifying…";
      try {
        const r = await solvePow(challenge.pow);
        powNonce = r.nonce;
        powHashHex = r.hashHex;
      } catch {
        powNonce = null;
        powHashHex = null;
      }
    }
    puzzleSolved();
  }

  function solvePow(pow) {
    return new Promise((resolve, reject) => {
      const w = new Worker(new URL("./pow-worker.js", import.meta.url), { type: "module" });
      w.onmessage = (e) => {
        if (e.data.type === "done") {
          w.terminate();
          resolve({ nonce: e.data.nonce, hashHex: e.data.hashHex });
        } else if (e.data.type === "error") {
          w.terminate();
          reject(new Error(e.data.message));
        }
      };
      w.postMessage({
        saltHex: pow.salt_hex,
        mCost: pow.m_cost,
        tCost: pow.t_cost,
        pCost: pow.p_cost,
        bits: pow.bits,
      });
    });
  }

  async function startChallenge() {
    if (fallbackKind) {
      CAPTCHA_KIND = fallbackKind;
      fallbackKind = null;
    }
    if (CAPTCHA_KIND === "three") {
      await startPassive();
      return;
    }
    statusEl.textContent = "Fetching challenge…";
    setUi();
    try {
      const res = await fetch(`${SERVER_URL}/challenge/${CAPTCHA_KIND}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ site_key: SITE_KEY_W, hostname: location.hostname }),
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
    isPursuit = challenge.kind === "four";
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
    touchClickPending = false;
    puzzleStart = Date.now();
    if (isSlider && sliderHint) {
      pieceImg = panelStrips[1] || null;
      pieceX = sliderHint.start_x;
      pieceY = sliderHint.start_y;
      dragging = false;
      sliderDropped = false;
      selX = -1;
      selY = -1;
      confirmReady = false;
      statusEl.textContent = "Click the hole that fits the piece";
    } else if (isPursuit) {
      statusEl.textContent = "Follow the moving target with your cursor";
    } else {
      statusEl.textContent = "Click a ball with the color shown";
    }
    deadline = Date.now() + TIME_LIMIT;
    setProgress();
    timerBar.style.opacity = "1";
    timerBar.style.transform = "scaleX(1)";
    setUi();
    startLoop();
    if (isPursuit) {
      if (progressEl) progressEl.textContent = "Tracking…";
      pursuitTimer = setTimeout(() => {
        if (state === "active") puzzleSolved();
      }, PURSUIT_MS);
    }
  }

  function advancePanel() {
    panelIdx += 1;
    setProgress();
    if (panelIdx >= EXPECTED_CLICKS) {
      puzzleSolved();
      return;
    }
    statusEl.textContent = "Click a ball with the color shown";
  }

  function puzzleSolved() {
    state = "vdf";
    vdfProgress = 0;
    timerBar.style.opacity = "0";
    statusEl.textContent = "Verifying… 0%";
    setUi();
    startLoop();
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
      powNonce, 
      powHashHex,
    });
  }

  async function submitSolution(outputHex, proofHex) {
    if (challenge && challenge.kind === "three" && passiveIssuedAt) {
      const elapsed = Date.now() - passiveIssuedAt;
      const remaining = 9000 - elapsed;
      if (remaining > 0) {
        statusEl.textContent = "Finishing up…";
        await new Promise((r) => setTimeout(r, remaining));
      }
    }
    statusEl.textContent = "Submitting…";
    try {
      const res = await fetch(`${SERVER_URL}/verify`, {
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
          instr_result: instrResult,
          telemetry: telemetrySnapshot(challenge.nonce),
          pow_nonce: powNonce,
          pow_hash_hex: powHashHex,
        }),
      });
      const result = await res.json();
      if (result.ok && result.token) {
        state = "success";
        statusEl.textContent = "Success!";
        setUi();
        if (opts.onSolve) {
          opts.onSolve(result.token);  
        } else {
          unlockContent(result.token); 
        }
        return;
      } else if (result.ok && !result.token) {
        if (worker) { worker.terminate(); worker = null; }
        if (result.need_challenge) {
          fallbackKind = result.need_challenge;
          statusEl.textContent = "One more step…";
          setUi();
          await startChallenge();
          return;
        }
        const m = /round\s+(\d+)\s+of\s+(\d+)/i.exec(result.message || "");
        if (m) {
          sliderRound = Number(m[1]);
          sliderRounds = Number(m[2]);
        }
        statusEl.textContent = result.message || "Next round…";
        await startChallenge();
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

  function fail() {
    if (pursuitTimer) { clearTimeout(pursuitTimer); pursuitTimer = 0; }
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
    panelMotions = [];
    panelIdx = 0;
    flashUntil = 0;
    touchClickPending = false;
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
    sliderRound = 0;
    sliderRounds = SLIDER_ROUNDS;
    sliderDropped = false; 
    selX = -1;
    selY = -1;
    confirmReady = false;
    powNonce = null;
    powHashHex = null;
    timerBar.style.opacity = "0";
    statusEl.textContent = "Verify you are human";
    isPursuit = false;
    if (pursuitTimer) { clearTimeout(pursuitTimer); pursuitTimer = 0;}
    if (progressEl) progressEl.textContent = "";
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
    if (isPursuit) return;
    if (Date.now() < flashUntil) return;
    if (touchClickPending) { touchClickPending = false; return; }
    const rect = canvas.getBoundingClientRect();
    const cx = (e.clientX - rect.left) * (W / rect.width);
    const cy = (e.clientY - rect.top) * (H / rect.height);
    if (!isBallPixel(cx, cy)) {
      statusEl.textContent = "Missed - click a ball with the color shown";
      return;
    }
    const t = Date.now() - puzzleStart;
    trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
    if (trail.length > 400) trail.shift();
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
    const t = Date.now() - puzzleStart;
    trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
    if (trail.length > 400) trail.shift();
    if (e.pointerType === "touch" && Date.now() >= flashUntil && !isPursuit) {
      if (!isBallPixel(cx, cy)) {
        statusEl.textContent = "Missed - tap a ball with the color shown";
        return;
      }
      touchClickPending = true;
      trail.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
      if (trail.length > 400) trail.shift();
      clicks.push({ x: Math.round(cx), y: Math.round(cy), t: Math.round(t) });
      flashUntil = Date.now() + 350;
      statusEl.textContent = "Nice!";
      setTimeout(advancePanel, 360);
    }
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
  canvas.addEventListener("pointercancel", () => { pointerDown = false; dragging = false; });
  canvas.addEventListener("pointermove", (e) => {
    if (state !== "active" || !isSlider) return;
    const rect = canvas.getBoundingClientRect();
    const x = (e.clientX - rect.left) * (W / rect.width);
    const y = (e.clientY - rect.top) * (H / rect.height);
    inputType = e.pointerType || inputType;
    trail.push({ x: Math.round(x), y: Math.round(y), t: Math.round(Date.now() - puzzleStart) });
    if (trail.length > 400) trail.shift();
  });
  canvas.addEventListener("pointerdown", (e) => {
    if (state !== "active" || !isSlider || !sliderHint) return;
    const rect = canvas.getBoundingClientRect();
    const x = (e.clientX - rect.left) * (W / rect.width);
    const y = (e.clientY - rect.top) * (H / rect.height);
    inputType = e.pointerType || "mouse";
    const t = Date.now() - puzzleStart;
    trail.push({ x: Math.round(x), y: Math.round(y), t: Math.round(t) });
    if (trail.length > 400) trail.shift();
    if (confirmReady) {
      const b = confirmBtn();
      if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) {
        clicks = [
          { x: Math.round(selX), y: Math.round(selY), t: Math.round(selT) },
          { x: Math.round(x), y: Math.round(y), t: Math.round(t) },
        ];
        flashUntil = Date.now() + 350;
        statusEl.textContent = "Checking…";
        setTimeout(puzzleSolved, 360);
        return;
      }
    }
    if (y > H - 44 && x > W - 100) return;
    selX = x;
    selY = y;
    selT = t;
    confirmReady = true;
    flashUntil = Date.now() + 250;
    statusEl.textContent = "Click Confirm to submit";
  });
  reset();
  draw(Date.now(), 0);
}

document.querySelectorAll(".captcha-card").forEach((c) => createCaptcha(c));
setupCarousel();
setupTelemetry();