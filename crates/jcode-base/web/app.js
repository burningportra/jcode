// app.js - jcode remote PWA controller.
// State machine: unpaired -> pairing -> paired -> connecting -> subscribed -> live.
// See initiative remote-web-pwa-handoff for the design.
import { Req, decodeEvent } from "/wire.js";

const LS_TOKEN = "jcode.token";
const LS_DEVICE = "jcode.device_id";

const els = {
  dot: document.getElementById("dot"),
  status: document.getElementById("status"),
  session: document.getElementById("session"),
  transcript: document.getElementById("transcript"),
  form: document.getElementById("composer"),
  input: document.getElementById("input"),
  send: document.getElementById("send"),
  interrupt: document.getElementById("interrupt"),
};

const state = {
  ws: null,
  token: localStorage.getItem(LS_TOKEN) || null,
  sessionId: null,        // target session from URL/handoff
  connSessionId: null,    // session id reported by server
  backoff: 500,
  reconnectTimer: null,
  live: false,
  processing: false,
  // in-flight streaming buffers, cleared on (re)subscribe to avoid double-render
  liveEl: null,
  liveText: "",
  reasoningEl: null,
  toolEls: new Map(),
  intentionalClose: false,
};

// ---- small DOM helpers ----
function setStatus(text, cls) {
  els.status.textContent = text;
  els.dot.className = "dot" + (cls ? " " + cls : "");
}
function notice(text, isError) {
  const d = document.createElement("div");
  d.className = "notice" + (isError ? " error" : "");
  d.textContent = text;
  els.transcript.appendChild(d);
  scroll();
}
function scroll() {
  els.transcript.scrollTop = els.transcript.scrollHeight;
}
function addMessage(role, text) {
  const wrap = document.createElement("div");
  wrap.className = "msg " + role;
  const r = document.createElement("div");
  r.className = "role";
  r.textContent = role;
  const b = document.createElement("div");
  b.className = "body";
  b.textContent = text || "";
  wrap.appendChild(r);
  wrap.appendChild(b);
  els.transcript.appendChild(wrap);
  scroll();
  return b; // the body element, for streaming appends
}

// ---- pairing ----
function deviceId() {
  let id = localStorage.getItem(LS_DEVICE);
  if (!id) {
    id = (crypto.randomUUID && crypto.randomUUID()) ||
      "web-" + Math.random().toString(16).slice(2);
    localStorage.setItem(LS_DEVICE, id);
  }
  return id;
}
function deviceName() {
  const ua = navigator.userAgent;
  const plat =
    /iPhone|iPad/.test(ua) ? "iOS" :
    /Android/.test(ua) ? "Android" :
    /Mac/.test(ua) ? "Mac" :
    /Windows/.test(ua) ? "Windows" : "Web";
  return plat + " browser";
}
async function pair(code) {
  setStatus("pairing", "connecting");
  const res = await fetch("/pair", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      code,
      device_id: deviceId(),
      device_name: deviceName(),
    }),
  });
  if (!res.ok) {
    let msg = "Pairing failed (" + res.status + ")";
    try {
      const j = await res.json();
      if (j && j.error) msg = j.error;
    } catch {}
    throw new Error(msg);
  }
  const j = await res.json();
  if (!j.token) throw new Error("Pairing response missing token");
  state.token = j.token;
  localStorage.setItem(LS_TOKEN, j.token);
}

// ---- websocket lifecycle ----
function wsUrl() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  // Browsers cannot set Authorization on WebSocket; token rides the query param.
  return `${proto}//${location.host}/ws?token=${encodeURIComponent(state.token)}`;
}
function clearLiveBuffers() {
  state.liveEl = null;
  state.liveText = "";
  state.reasoningEl = null;
  state.toolEls.clear();
}
function connect() {
  if (!state.token) return;
  clearTimeout(state.reconnectTimer);
  setStatus("connecting", "connecting");
  state.intentionalClose = false;
  let ws;
  try {
    ws = new WebSocket(wsUrl());
  } catch (e) {
    scheduleReconnect();
    return;
  }
  state.ws = ws;

  ws.onopen = () => {
    state.backoff = 500;
    // (Re)subscribe then re-fetch history as source of truth; clear any
    // in-flight delta buffer so a mid-turn reconnect never double-renders.
    clearLiveBuffers();
    els.transcript.replaceChildren();
    ws.send(Req.subscribe(state.sessionId));
    ws.send(Req.getHistory());
    setStatus("live", "live");
    state.live = true;
  };
  ws.onmessage = (ev) => {
    // Server may batch multiple NDJSON lines in one frame.
    for (const line of String(ev.data).split("\n")) {
      const s = line.trim();
      if (s) handleEvent(decodeEvent(s));
    }
  };
  ws.onclose = () => {
    state.live = false;
    if (state.intentionalClose) return;
    // The browser cannot read the handshake 401 body, so an immediate close
    // after a stored token is most likely auth failure (revoked/re-paired).
    setStatus("disconnected", "error");
    scheduleReconnect();
  };
  ws.onerror = () => { /* onclose handles reconnect */ };
}
function scheduleReconnect() {
  if (document.hidden) return; // resume on visibility instead of hammering
  const jitter = Math.random() * 250;
  const delay = Math.min(state.backoff, 15000) + jitter;
  state.backoff = Math.min(state.backoff * 2, 15000);
  setStatus("reconnecting", "connecting");
  state.reconnectTimer = setTimeout(connect, delay);
}

// ---- event reducer ----
function handleEvent(e) {
  switch (e.type) {
    case "history": {
      renderHistory(e);
      break;
    }
    case "session": {
      state.connSessionId = e.session_id;
      els.session.textContent = e.session_id ? "· " + e.session_id : "";
      break;
    }
    case "state": {
      state.processing = !!e.is_processing;
      updateInterrupt();
      break;
    }
    case "text_delta": {
      appendAssistant(e.text || "");
      break;
    }
    case "text_replace": {
      state.liveText = e.text || "";
      if (!state.liveEl) state.liveEl = addMessage("assistant", "");
      state.liveEl.textContent = state.liveText;
      scroll();
      break;
    }
    case "reasoning_delta": {
      if (!state.reasoningEl) state.reasoningEl = addMessage("reasoning", "");
      state.reasoningEl.textContent += e.text || "";
      scroll();
      break;
    }
    case "reasoning_done": {
      state.reasoningEl = null;
      break;
    }
    case "tool_start":
    case "tool_exec": {
      ensureTool(e.id, e.name);
      break;
    }
    case "tool_done": {
      finishTool(e.id, e.name, e.output || "", e.error);
      break;
    }
    case "message_end": {
      // finalize the current assistant bubble
      state.liveEl = null;
      state.liveText = "";
      break;
    }
    case "interrupted": {
      notice("interrupted");
      state.processing = false;
      state.liveEl = null;
      updateInterrupt();
      break;
    }
    case "done": {
      state.processing = false;
      updateInterrupt();
      break;
    }
    case "error": {
      notice(e.message || "error", true);
      state.processing = false;
      updateInterrupt();
      break;
    }
    case "reloading": {
      notice("server reloading, reconnecting...");
      break;
    }
    case "session_close_requested": {
      notice("session closed: " + (e.reason || ""));
      break;
    }
    // ack, tokens, status_detail, connection_phase, pong, etc: no-op for v1
    default:
      break;
  }
}

function appendAssistant(text) {
  if (!state.liveEl) {
    state.liveEl = addMessage("assistant", "");
    state.liveText = "";
  }
  state.liveText += text;
  state.liveEl.textContent = state.liveText;
  scroll();
  state.processing = true;
  updateInterrupt();
}
function ensureTool(id, name) {
  if (state.toolEls.has(id)) return state.toolEls.get(id);
  const el = document.createElement("div");
  el.className = "tool";
  el.innerHTML = `<span class="name"></span> <span class="args"></span><div class="out"></div>`;
  el.querySelector(".name").textContent = name || "tool";
  els.transcript.appendChild(el);
  state.toolEls.set(id, el);
  scroll();
  return el;
}
function finishTool(id, name, output, error) {
  const el = ensureTool(id, name);
  if (error) el.classList.add("err");
  const out = el.querySelector(".out");
  out.textContent = error ? String(error) : String(output);
  scroll();
}

function renderHistory(e) {
  els.transcript.replaceChildren();
  clearLiveBuffers();
  const msgs = Array.isArray(e.messages) ? e.messages : [];
  for (const m of msgs) {
    const role = m.role || "assistant";
    if (m.tool_data) {
      const td = m.tool_data;
      const el = ensureTool(td.id || Math.random().toString(36), td.name);
      finishTool(td.id, td.name, td.output || "", td.error);
    } else if (role === "user" || role === "assistant") {
      if ((m.content || "").trim()) addMessage(role, m.content);
    }
  }
  if (e.session_id) {
    state.connSessionId = e.session_id;
    els.session.textContent = "· " + e.session_id;
  }
  scroll();
}

// ---- composer ----
function updateInterrupt() {
  els.interrupt.hidden = !state.processing;
}
els.form.addEventListener("submit", (ev) => {
  ev.preventDefault();
  const text = els.input.value.trim();
  if (!text || !state.ws || state.ws.readyState !== WebSocket.OPEN) return;
  addMessage("user", text);
  state.ws.send(Req.message(text));
  els.input.value = "";
  els.input.style.height = "auto";
  state.processing = true;
  updateInterrupt();
});
els.interrupt.addEventListener("click", () => {
  if (state.ws && state.ws.readyState === WebSocket.OPEN) {
    state.ws.send(Req.cancel());
  }
});
els.input.addEventListener("input", () => {
  els.input.style.height = "auto";
  els.input.style.height = Math.min(els.input.scrollHeight, window.innerHeight * 0.4) + "px";
});
els.input.addEventListener("keydown", (ev) => {
  if (ev.key === "Enter" && !ev.shiftKey && !ev.isComposing) {
    ev.preventDefault();
    els.form.requestSubmit();
  }
});

// ---- lifecycle: resume from background, graceful detach ----
document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;
  if (!state.live && state.token) {
    state.backoff = 500;
    connect();
  }
});
window.addEventListener("pagehide", () => {
  state.intentionalClose = true;
  try {
    if (state.ws && state.ws.readyState === WebSocket.OPEN) {
      state.ws.send(Req.prepareDisconnect());
      state.ws.close();
    }
  } catch {}
});

// ---- boot ----
async function boot() {
  const params = new URLSearchParams(location.search);
  const code = params.get("code");
  state.sessionId = params.get("session") || null;

  // Strip credentials from the URL bar/history immediately.
  if (code || params.has("session")) {
    history.replaceState(null, "", location.pathname);
  }

  if (code) {
    try {
      await pair(code);
    } catch (e) {
      setStatus("pairing failed", "error");
      notice(e.message || "pairing failed", true);
      return;
    }
  }

  if (!state.token) {
    setStatus("not paired", "error");
    notice("No pairing code. Open the handoff link from `/remote handoff` on your desktop.", true);
    return;
  }
  connect();
}

if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("/service-worker.js").catch(() => {});
}
boot();
