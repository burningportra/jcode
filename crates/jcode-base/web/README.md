# jcode remote browser client (PWA)

A small progressive web app that lets a browser pair with a running jcode
session and attach to it: view the streaming transcript, send messages, and
interrupt a turn. It speaks the same WebSocket protocol as the TUI and iOS
clients (`crates/jcode-protocol/src/wire.rs`), and is served directly by the
gateway, so there is nothing to deploy.

## Using it

1. On the machine running jcode, enable the gateway once and restart:

   ```
   /remote on
   jcode server reload
   ```

2. Hand off the current session (or just pair a device):

   ```
   /remote handoff      # this conversation
   /remote pair         # a fresh session
   ```

   Both print a browser link like `http://your-host:7643/?code=123456&session=...`.

3. Open that link in any browser on the **same trusted network** (Tailscale or
   LAN). The app exchanges the pairing code for a token, strips the code from
   the URL, and connects. On a phone you can "Add to Home Screen" to install it.

## How it works

- The gateway serves the app shell (`index.html`, `app.js`, `wire.js`,
  `app.css`, `manifest.webmanifest`, `service-worker.js`, `icon.svg`) from
  `crates/jcode-base/web/`, embedded into the binary at compile time. No Node or
  bundler is involved in the Rust build; edit the files here directly.
- The app is served over plain HTTP from the same origin as `/ws` and `/pair`,
  which is what lets the browser open a `ws://` connection (an `https` page
  cannot, due to mixed-content rules).
- Auth: the browser cannot set an `Authorization` header on a WebSocket, so the
  token is sent via `Sec-WebSocket-Protocol` (`jcode.bearer.<token>`, offered
  alongside the non-secret `jcode.v1` protocol the server echoes). This keeps the
  token out of the URL, so it does not land in server/proxy logs or browser
  history. The legacy `?token=` query parameter still works for older clients.

## Security

- A paired browser has the **same full control** of jcode as the TUI: it can
  drive an agent that runs commands and edits files. Treat the pairing link and
  token like a shell on the machine.
- Keep the gateway on Tailscale or a LAN. Do **not** expose port 7643 to the
  public internet without putting TLS and additional authentication in front.
- The token is plaintext on the wire (no TLS in v1). The browser sends it via
  `Sec-WebSocket-Protocol` rather than the URL, so it stays out of logs and
  history, but on an untrusted network it is still observable. Revoke a device
  any time with `/remote revoke <name>`.

## Scope (v1)

Included: pairing, live transcript (assistant text, reasoning, tool calls),
sending messages, interrupt, reconnect with backoff, resume-from-background,
installable PWA shell.

Not included yet: TLS/https, cloud relay, model switching / rewind / compact
from the browser, and session-scoped (rather than device-scoped) tokens.
