// wire.js - browser port of ios/Sources/JCodeKit/Wire.swift (v1 subset).
// Encodes client Requests and decodes server ServerEvents for jcode's
// newline-delimited JSON protocol (crates/jcode-protocol/src/wire.rs).

let nextId = 1;
export function newId() {
  return nextId++;
}

// --- Requests (client -> server). Each returns a single JSON line string. ---
export const Req = {
  subscribe(targetSessionId, workingDir) {
    const o = { id: newId(), type: "subscribe" };
    if (targetSessionId) o.target_session_id = targetSessionId;
    // The server rejects a subscribe without an absolute working_dir. The
    // browser learns the server home from /health and passes it here; the
    // server's home-dir guard ignores that value when the attached session
    // already has its own cwd, so this never re-pins a live session. "/" is a
    // safe absolute fallback that also keeps a brand-new session valid.
    o.working_dir = workingDir && workingDir.startsWith("/") ? workingDir : "/";
    return JSON.stringify(o);
  },
  getHistory() {
    return JSON.stringify({ id: newId(), type: "get_history" });
  },
  message(content) {
    return JSON.stringify({ id: newId(), type: "message", content });
  },
  cancel() {
    return JSON.stringify({ id: newId(), type: "cancel" });
  },
  ping() {
    return JSON.stringify({ id: newId(), type: "ping" });
  },
  prepareDisconnect() {
    return JSON.stringify({ id: newId(), type: "prepare_disconnect" });
  },
};

// --- Events (server -> client). Decodes one NDJSON line into {type, ...}. ---
// Unknown types return {type:"unknown", raw} so newer servers never break us.
export function decodeEvent(line) {
  let obj;
  try {
    obj = JSON.parse(line);
  } catch {
    return { type: "unknown", raw: line };
  }
  if (!obj || typeof obj.type !== "string") {
    return { type: "unknown", raw: line };
  }
  return obj;
}
