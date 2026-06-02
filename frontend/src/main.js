const canvas = document.getElementById("puzzle");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const verifyBtn = document.getElementById("verify");

const DOT_COUNT = 120;
const dots = [];
for (let i = 0; i < DOT_COUNT; i++) {
  dots.push({
    x: Math.random() * canvas.width,
    y: Math.random() * canvas.height,
    vx: (Math.random() - 0.5) * 1.5,
    vy: (Math.random() - 0.5) * 1.5,
  });
}

function tick() {
  ctx.fillStyle = "#0b0f1a";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#7dd3fc";
  for (const d of dots) {
    d.x += d.vx;
    d.y += d.vy;
    if (d.x < 0 || d.x > canvas.width) d.vx *= -1;
    if (d.y < 0 || d.y > canvas.height) d.vy *= -1;
    ctx.beginPath();
    ctx.arc(d.x, d.y, 2, 0, Math.PI * 2);
    ctx.fill();
  }
  requestAnimationFrame(tick);
}
tick();

verifyBtn.addEventListener("click", () => {
  statusEl.textContent = "Verifying… (VDF runs here later)";
  setTimeout(() => {
    statusEl.textContent = "Placeholder: verified ✓";
  }, 800);
});