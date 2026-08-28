use super::*;
use tokio_tungstenite::tungstenite::handshake::server::Request;

#[test]
fn test_device_registry_pairing() {
    let mut registry = DeviceRegistry::default();

    // Generate pairing code
    let code = registry.generate_pairing_code();
    assert_eq!(code.len(), 6);
    assert_eq!(registry.pending_codes.len(), 1);

    // Validate correct code
    assert!(registry.validate_code(&code));
    assert_eq!(registry.pending_codes.len(), 0); // consumed

    // Validate again should fail (consumed)
    assert!(!registry.validate_code(&code));
}

#[test]
fn test_device_registry_token_auth() {
    let mut registry = DeviceRegistry::default();

    // Pair a device
    let token = registry.pair_device("test-device-1".to_string(), "Test iPhone".to_string(), None);

    // Validate correct token
    assert!(registry.validate_token(&token).is_some());
    let device = registry.validate_token(&token).unwrap();
    assert_eq!(device.name, "Test iPhone");
    assert_eq!(device.id, "test-device-1");

    // Validate wrong token
    assert!(registry.validate_token("wrong-token").is_none());

    // Token hash should be stored, not raw token
    assert!(registry.devices[0].token_hash.starts_with("sha256:"));
}

#[test]
fn test_device_re_pairing() {
    let mut registry = DeviceRegistry::default();

    // Pair same device twice
    let token1 = registry.pair_device("device-1".to_string(), "iPhone v1".to_string(), None);
    let token2 = registry.pair_device("device-1".to_string(), "iPhone v2".to_string(), None);

    // Only one device entry (old one replaced)
    assert_eq!(registry.devices.len(), 1);
    assert_eq!(registry.devices[0].name, "iPhone v2");

    // Old token should be invalid
    assert!(registry.validate_token(&token1).is_none());
    // New token should be valid
    assert!(registry.validate_token(&token2).is_some());
}

#[test]
fn test_parse_bearer_token() {
    assert_eq!(parse_bearer_token("Bearer abc"), Some("abc"));
    assert_eq!(parse_bearer_token("bearer abc"), Some("abc"));
    assert_eq!(parse_bearer_token("BEARER abc"), Some("abc"));
    assert_eq!(parse_bearer_token("Bearer"), None);
    assert_eq!(parse_bearer_token("Basic abc"), None);
    assert_eq!(parse_bearer_token("Bearer abc def"), None);
}

#[test]
fn test_parse_query_token() {
    assert_eq!(parse_query_token("token=abc"), Some("abc"));
    assert_eq!(parse_query_token("foo=bar&token=abc123"), Some("abc123"));
    assert_eq!(parse_query_token("token="), None);
    assert_eq!(parse_query_token("foo=bar"), None);
}

#[test]
fn test_hex_token_validation() {
    assert!(is_valid_hex_token(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    ));
    assert!(!is_valid_hex_token("abc"));
    assert!(!is_valid_hex_token(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
    ));
}

#[test]
fn test_extract_ws_auth_prefers_header_and_falls_back_to_query() {
    let token_a = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let token_b = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    let header_request = Request::builder()
        .uri("ws://example.com/ws")
        .header("authorization", format!("Bearer {token_a}"))
        .body(())
        .expect("request");
    let header_auth = extract_ws_auth(&header_request).expect("header auth");
    assert_eq!(header_auth.token, token_a);
    assert_eq!(header_auth.source, WsAuthSource::Header);

    let query_request = Request::builder()
        .uri(format!("ws://example.com/ws?token={token_b}"))
        .body(())
        .expect("request");
    let query_auth = extract_ws_auth(&query_request).expect("query auth");
    assert_eq!(query_auth.token, token_b);
    assert_eq!(query_auth.source, WsAuthSource::Query);
}

#[test]
fn test_extract_ws_auth_rejects_conflicting_sources() {
    let token_a = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let token_b = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    let request = Request::builder()
        .uri(format!("ws://example.com/ws?token={token_b}"))
        .header("authorization", format!("Bearer {token_a}"))
        .body(())
        .expect("request");
    assert!(extract_ws_auth(&request).is_err());
}

#[test]
fn test_find_header_end() {
    assert_eq!(
        super::find_header_end(b"POST /pair HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}"),
        Some(38)
    );
    assert_eq!(
        super::find_header_end(b"POST /pair HTTP/1.1\r\nContent-"),
        None
    );
    assert_eq!(super::find_header_end(b""), None);
}

#[test]
fn test_authorize_ws_device_valid_token() {
    let mut registry = DeviceRegistry::default();
    let token = registry.pair_device("dev-1".to_string(), "iPhone".to_string(), None);

    let device = auth::authorize_ws_device(&registry, &token).expect("valid token authorizes");
    assert_eq!(device.name, "iPhone");
    assert_eq!(device.id, "dev-1");
}

#[test]
fn test_authorize_ws_device_rejects_unknown_and_revoked_with_401() {
    let mut registry = DeviceRegistry::default();
    let token = registry.pair_device("dev-1".to_string(), "iPhone".to_string(), None);

    // Unknown token -> 401 at handshake time.
    let unknown = "a".repeat(64);
    let err =
        auth::authorize_ws_device(&registry, &unknown).expect_err("unknown token must be rejected");
    assert_eq!(err.status(), 401);
    assert!(
        err.body()
            .as_deref()
            .unwrap_or_default()
            .contains("re-pair"),
        "401 body should tell the client to re-pair"
    );

    // Revoked device -> same 401 path.
    registry.devices.retain(|d| d.id != "dev-1");
    let err =
        auth::authorize_ws_device(&registry, &token).expect_err("revoked token must be rejected");
    assert_eq!(err.status(), 401);
}

/// The PWA asset server must never capture the API/WS paths, or serving the web
/// app would silently break pairing, health, and the WebSocket upgrade. This is
/// the load-bearing non-regression guarantee for M2.
#[test]
fn test_asset_server_never_shadows_api_paths() {
    assert!(super::web_assets::serve_asset("/health").is_none());
    assert!(super::web_assets::serve_asset("/pair").is_none());
    assert!(super::web_assets::serve_asset("/ws").is_none());
    // But it does serve the app shell and its assets.
    assert!(super::web_assets::serve_asset("/").is_some());
    assert!(super::web_assets::serve_asset("/app.js").is_some());
    assert!(super::web_assets::serve_asset("/service-worker.js").is_some());
    assert!(super::web_assets::serve_asset("/manifest.webmanifest").is_some());
}

/// End-to-end HTTP: bind an ephemeral port, run the real connection router, and
/// make real TCP requests. Proves the gateway actually serves the PWA over the
/// wire AND that adding asset routes did not regress /health or the 404 path.
#[tokio::test]
async fn test_gateway_http_serves_pwa_and_preserves_api() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (client_tx, _client_rx) = tokio::sync::mpsc::unbounded_channel::<super::GatewayClient>();
    let registry = std::sync::Arc::new(tokio::sync::RwLock::new(DeviceRegistry::default()));

    // Accept loop: one connection per request (each request uses Connection: close).
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                break;
            };
            let registry = std::sync::Arc::clone(&registry);
            let client_tx = client_tx.clone();
            tokio::spawn(async move {
                let _ = super::handle_connection(stream, peer, registry, client_tx).await;
            });
        }
    });

    async fn request(addr: std::net::SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.write_all(raw.as_bytes()).await.expect("write");
        stream.flush().await.expect("flush");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).into_owned()
    }

    // GET / serves the PWA shell as HTML.
    let root = request(addr, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(root.starts_with("HTTP/1.1 200 OK"), "root status: {root:.40}");
    assert!(root.contains("Content-Type: text/html"));
    assert!(root.contains("<!DOCTYPE html>"));

    // GET /app.js serves JavaScript, not JSON, in the header.
    let js = request(addr, "GET /app.js HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    let js_header = js.split("\r\n\r\n").next().unwrap_or_default();
    assert!(js_header.contains("Content-Type: text/javascript"));

    // GET /health is UNCHANGED: still JSON with status ok.
    let health =
        request(addr, "GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(health.starts_with("HTTP/1.1 200 OK"));
    assert!(health.contains("Content-Type: application/json"));
    assert!(health.contains("\"status\":\"ok\""));

    // An unknown path still 404s.
    let missing =
        request(addr, "GET /nope HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(missing.starts_with("HTTP/1.1 404"));
}
