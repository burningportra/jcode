//! Static asset serving for the browser/PWA remote client.
//!
//! The gateway serves a small progressive web app so a browser can pair with a
//! running session and attach to it, exactly like the TUI/iOS clients. Assets
//! are embedded at compile time from `crates/jcode-base/web/`, so `cargo build`
//! needs no Node/bundler and the binary is self-contained.
//!
//! Served over plain HTTP from the same origin as `/ws` and `/pair`, which is
//! what lets the browser open `ws://` without tripping mixed-content rules. See
//! the `remote-web-pwa-handoff` initiative for the full design.

/// One embedded asset: URL path, bytes, and content type.
struct Asset {
    path: &'static str,
    body: &'static [u8],
    content_type: &'static str,
}

const WEB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web");

macro_rules! asset {
    ($path:expr, $file:expr, $ct:expr) => {
        Asset {
            path: $path,
            body: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/", $file)),
            content_type: $ct,
        }
    };
}

/// All embedded PWA assets. `/` is served from index.html.
static ASSETS: &[Asset] = &[
    asset!("/", "index.html", "text/html; charset=utf-8"),
    asset!("/index.html", "index.html", "text/html; charset=utf-8"),
    asset!("/app.js", "app.js", "text/javascript; charset=utf-8"),
    asset!("/wire.js", "wire.js", "text/javascript; charset=utf-8"),
    asset!("/app.css", "app.css", "text/css; charset=utf-8"),
    asset!(
        "/service-worker.js",
        "service-worker.js",
        "text/javascript; charset=utf-8"
    ),
    asset!(
        "/manifest.webmanifest",
        "manifest.webmanifest",
        "application/manifest+json; charset=utf-8"
    ),
    asset!("/icon.svg", "icon.svg", "image/svg+xml"),
];

/// Reference the WEB dir so the constant is not dead code if ASSETS shrinks.
#[allow(dead_code)]
fn web_dir() -> &'static str {
    WEB
}

/// A ready-to-write HTTP response for a static asset.
pub(super) struct AssetResponse {
    pub(super) bytes: Vec<u8>,
}

/// Look up an embedded asset by request path and build its HTTP/1.1 response.
///
/// Returns `None` when no asset matches, so the caller falls through to its
/// existing 404 handling. The service worker needs a permissive scope, so its
/// response carries `Service-Worker-Allowed: /`.
pub(super) fn serve_asset(path_base: &str) -> Option<AssetResponse> {
    let asset = ASSETS.iter().find(|a| a.path == path_base)?;
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-cache\r\n",
        asset.content_type,
        asset.body.len()
    );
    if asset.path == "/service-worker.js" {
        head.push_str("Service-Worker-Allowed: /\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(asset.body);
    Some(AssetResponse { bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_index_at_root() {
        let r = serve_asset("/").expect("index at /");
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("Content-Type: text/html"));
        assert!(text.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn serves_js_with_javascript_mime_not_json() {
        let r = serve_asset("/app.js").expect("app.js");
        let text = String::from_utf8_lossy(&r.bytes);
        // Check the Content-Type header line specifically; the JS body itself
        // legitimately contains the string "application/json" (the /pair fetch).
        let header = text.split("\r\n\r\n").next().unwrap_or_default();
        assert!(header.contains("Content-Type: text/javascript"));
        assert!(!header.contains("application/json"));
    }

    #[test]
    fn service_worker_has_permissive_scope_header() {
        let r = serve_asset("/service-worker.js").expect("sw");
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(text.contains("Service-Worker-Allowed: /"));
    }

    #[test]
    fn manifest_has_manifest_mime() {
        let r = serve_asset("/manifest.webmanifest").expect("manifest");
        let text = String::from_utf8_lossy(&r.bytes);
        assert!(text.contains("application/manifest+json"));
    }

    #[test]
    fn unknown_path_is_none() {
        assert!(serve_asset("/nope").is_none());
        // Critically, API paths must NOT be captured by the asset server.
        assert!(serve_asset("/health").is_none());
        assert!(serve_asset("/pair").is_none());
        assert!(serve_asset("/ws").is_none());
    }

    #[test]
    fn content_length_matches_body() {
        for a in ASSETS {
            let r = serve_asset(a.path).expect(a.path);
            let text = String::from_utf8_lossy(&r.bytes);
            let needle = format!("Content-Length: {}\r\n", a.body.len());
            assert!(
                text.contains(&needle),
                "content-length mismatch for {}",
                a.path
            );
        }
    }
}
