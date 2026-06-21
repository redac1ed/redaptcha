import { createCaptcha, setupTelemetry } from "./main.js";
import "./style.css";

function findScript() {
  if (document.currentScript) return document.currentScript;
  const all = document.querySelectorAll("script[src]");
  for (const s of all) {
    if (/redaptcha(\.min)?\.js(\?|$)/.test(s.src) || s.dataset.endpoint) return s;
  }
  return null;
}

function resolveServer() {
  const script = findScript();
  if (script && script.dataset.endpoint) return script.dataset.endpoint.replace(/\/$/, "");
  try { return new URL(script.src).origin; } catch { return ""; }
}

const SERVER = resolveServer();

function widgetMarkup() {
  return `
    <div class="captcha" data-role="captcha">
      <button class="cb-box" data-role="checkbox" type="button" aria-label="Verify you are human">
        <span class="cb-mark"></span>
        <span class="cb-spinner"></span>
      </button>
      <div class="cb-label"><span data-role="status">Verify you are human</span></div>
      <div class="cb-brand">
        <span class="brand-name">redaptcha</span>
        <span class="brand-sub">Privacy · Terms</span>
      </div>
    </div>
    <div class="puzzle-panel" data-role="panel">
      <div class="puzzle">
        <span data-role="progress" class="puzzle-progress"></span>
        <canvas data-role="canvas" width="320" height="240"></canvas>
        <div class="timer-wrap"><div data-role="timer" class="timer-bar"></div></div>
      </div>
    </div>`;
}

function ensureHiddenInput(el, name) {
  const form = el.closest("form");
  if (!form) return null;
  let input = form.querySelector(`input[name="${name}"]`);
  if (!input) {
    input = document.createElement("input");
    input.type = "hidden";
    input.name = name;
    form.appendChild(input);
  }
  return input;
}

function renderInto(el) {
  if (el.dataset.rdRendered === "1") return;
  el.dataset.rdRendered = "1";
  const siteKey = el.dataset.sitekey || "";
  const kind = el.dataset.kind || "three"; 
  const fieldName = el.dataset.responseField || "redaptcha-response";
  const cbName = el.dataset.callback;
  const card = document.createElement("div");
  card.className = "captcha-card redaptcha-widget";
  card.dataset.kind = kind;
  card.innerHTML = widgetMarkup();
  el.appendChild(card);
  const hidden = ensureHiddenInput(el, fieldName);
  createCaptcha(card, {
    server: SERVER,
    siteKey,
    kind,
    onSolve(token) {
      if (hidden) hidden.value = token;
      if (cbName && typeof window[cbName] === "function") window[cbName](token);
      el.dispatchEvent(new CustomEvent("redaptcha-solved", { detail: { token }, bubbles: true }));
    },
  });
}

function scan(root = document) {
  root.querySelectorAll("[data-sitekey], .redaptcha").forEach(renderInto);
}

window.redaptcha = {
  render(el, opts = {}) {
    if (typeof el === "string") el = document.querySelector(el);
    if (!el) return;
    if (opts.sitekey) el.dataset.sitekey = opts.sitekey;
    if (opts.kind) el.dataset.kind = opts.kind;
    if (opts.callback) el.dataset.callback = opts.callback;
    renderInto(el);
  },
  scan,
};

setupTelemetry();
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", () => scan());
} else {
  scan();
}