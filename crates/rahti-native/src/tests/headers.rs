//! The security headers a packaged application serves.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use tower::ServiceExt;

use crate::headers::{install_csp, security_headers};

const POLICY: &str = "default-src 'self'; script-src 'self'";

fn app() -> Router {
    // Installed once for the whole test binary; `install_csp` ignores a
    // second call by design, and every test here asserts the same policy.
    install_csp(POLICY);
    Router::new()
        .route("/", get(|| async { "the page" }))
        .route(
            "/strict",
            get(|| async {
                (
                    [(header::CONTENT_SECURITY_POLICY, "default-src 'none'")],
                    "a page with its own policy",
                )
            }),
        )
        .layer(axum::middleware::from_fn(security_headers))
}

async fn headers_of(path: &str) -> axum::http::HeaderMap {
    let response = app()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .expect("a response");
    assert_eq!(response.status(), StatusCode::OK);
    response.headers().clone()
}

#[tokio::test]
async fn every_response_carries_the_policy() {
    // The page comes from the embedded HTTP server, so the only policy a
    // browser applies to it is one in its own response headers — not the one
    // in tauri.conf.json.
    let headers = headers_of("/").await;
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], POLICY);
}

#[tokio::test]
async fn a_page_that_set_its_own_policy_keeps_it() {
    // A deliberate narrowing must not be quietly widened by a layer above it.
    let headers = headers_of("/strict").await;
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'"
    );
}

#[tokio::test]
async fn sniffing_and_referrers_are_turned_off() {
    let headers = headers_of("/").await;
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    // The loopback URL carries the port, and for one request the launch token.
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
}

#[test]
fn the_default_policy_covers_what_a_rahti_page_actually_loads() {
    let csp = crate::config::NativeConfig::new("A", "com.example.a", "1.0.0", &["windows"])
        .security
        .csp;

    // Every script is same-origin, so no external source is permitted.
    assert!(csp.contains("script-src 'self'"), "{csp}");
    // Nothing evaluated and nothing inline: PulsePoint runs only the client
    // registry the application compiled, and only its Trusted Types policy
    // may write markup.
    assert!(!csp.contains("unsafe-eval"), "{csp}");
    assert!(csp.contains("require-trusted-types-for 'script'"), "{csp}");
    assert!(csp.contains("trusted-types pulsepoint"), "{csp}");
    // `pp.rpc`, a streaming response, and a `#[socket]` connection.
    assert!(csp.contains("connect-src"), "{csp}");
    assert!(csp.contains("ws:"), "{csp}");
    // Nothing may frame a privileged WebView, and nothing may re-point a
    // relative URL out of it.
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(csp.contains("base-uri 'self'"), "{csp}");
    assert!(csp.contains("object-src 'none'"), "{csp}");
}
