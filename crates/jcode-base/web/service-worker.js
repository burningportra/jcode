// service-worker.js - app-shell cache ONLY.
// Never caches /pair, /health, /ws, or any authenticated response or token.
// Cache version is tied to the app build so a reloaded gateway invalidates it.
const CACHE = "jcode-shell-v1";
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
  // Only serve the static app shell from cache. Everything else (API, WS
  // upgrades, cross-origin) bypasses the SW entirely.
  const isShell =
    e.request.method === "GET" &&
    url.origin === self.location.origin &&
    SHELL.includes(url.pathname);
  if (!isShell) return; // do not intercept /pair, /health, /ws, etc.

  e.respondWith(
    caches.match(e.request).then((hit) => hit || fetch(e.request))
  );
});
