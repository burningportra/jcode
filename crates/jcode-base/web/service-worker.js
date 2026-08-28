// service-worker.js - app-shell cache ONLY.
// Never caches /pair, /health, /ws, or any authenticated response or token.
//
// The cache name embeds the server build version (the gateway rewrites the
// __JCODE_VERSION__ token when it serves this file), so a new server build
// creates a new cache and the old one is deleted on activate. The fetch handler
// is NETWORK-FIRST for the shell, so a fix in app.js/wire.js lands on the next
// load instead of being pinned by a stale cache-first entry.
const VERSION = "__JCODE_VERSION__";
const CACHE = "jcode-shell-" + VERSION;
const SHELL = [
  "/",
  "/index.html",
  "/app.js",
  "/app.css",
  "/wire.js",
  "/manifest.webmanifest",
  "/icon.svg",
];

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
  self.skipWaiting();
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    )
  );
  self.clients.claim();
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  // Only handle the static app shell. Everything else (API, WS upgrades,
  // cross-origin) bypasses the SW entirely.
  const isShell =
    e.request.method === "GET" &&
    url.origin === self.location.origin &&
    SHELL.includes(url.pathname);
  if (!isShell) return; // do not intercept /pair, /health, /ws, etc.

  // Network-first: always try to fetch the latest shell so a new build lands
  // immediately; fall back to cache only when offline. Refresh the cache on
  // every successful fetch.
  e.respondWith(
    fetch(e.request)
      .then((resp) => {
        const copy = resp.clone();
        caches.open(CACHE).then((c) => c.put(e.request, copy)).catch(() => {});
        return resp;
      })
      .catch(() => caches.match(e.request))
  );
});

