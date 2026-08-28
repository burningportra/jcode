// wire.test.mjs - runnable check that the client wire protocol matches the
// server contract (crates/jcode-protocol/src/wire.rs). Run with: node wire.test.mjs
// No test framework or npm install needed; exits non-zero on failure.
import { Req, decodeEvent, newId } from "./wire.js";

let fail = 0;
function ok(cond, msg) {
  if (!cond) {
    console.log("FAIL:", msg);
    fail++;
  } else {
    console.log("ok:", msg);
  }
}

// Requests encode to a single JSON line with the tags the server expects.
const sub = JSON.parse(Req.subscribe("sess_abc"));
ok(
  sub.type === "subscribe" &&
    sub.target_session_id === "sess_abc" &&
    typeof sub.id === "number",
  "subscribe carries target_session_id"
);
const subNull = JSON.parse(Req.subscribe(null));
ok(
  subNull.type === "subscribe" && !("target_session_id" in subNull),
  "subscribe omits a null session"
);
ok(JSON.parse(Req.message("hi")).content === "hi", "message content");
ok(JSON.parse(Req.cancel()).type === "cancel", "cancel");
ok(JSON.parse(Req.ping()).type === "ping", "ping");
ok(JSON.parse(Req.getHistory()).type === "get_history", "get_history");
ok(
  JSON.parse(Req.prepareDisconnect()).type === "prepare_disconnect",
  "prepare_disconnect"
);
ok(newId() < newId(), "ids increment");
ok(!Req.subscribe("s").includes("\n"), "encoded request has no newline");

// decodeEvent parses events and never throws on unexpected input.
ok(
  decodeEvent('{"type":"text_delta","text":"hi"}').type === "text_delta",
  "decode text_delta"
);
ok(decodeEvent("not json").type === "unknown", "bad json -> unknown");
ok(decodeEvent('{"no":"type"}').type === "unknown", "missing type -> unknown");
ok(
  decodeEvent('{"type":"future_event"}').type === "future_event",
  "unknown event type passes through (forward compat)"
);

if (fail === 0) {
  console.log("ALL WIRE TESTS PASS");
  process.exit(0);
} else {
  console.log("WIRE FAILURES:", fail);
  process.exit(1);
}
