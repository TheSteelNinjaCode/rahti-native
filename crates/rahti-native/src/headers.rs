//! The response headers a packaged application serves.
//!
//! ## Why the policy is not in `tauri.conf.json`
//!
//! Tauri's `app.security.csp` applies to the documents Tauri itself serves
//! through its asset protocol. A packaged Rahti application does not use it:
//! every document comes from the embedded HTTP server, on a `http://127.0.0.1`
//! origin, and the only Content-Security-Policy a browser will apply to those
//! is one that arrives in *their* response headers.
//!
//! So it is set here, by the server that serves them. The value in
//! `rahti.native.json` is the one that reaches the page; the value in the
//! generated `tauri.conf.json` covers the shell's own placeholder document and
//! is the same string so that neither can be mistaken for the other.
//!
//! ## Why it matters more here than on the web
//!
//! A cross-site scripting bug in a web page steals a session. The same bug in
//! a native shell reaches the native command bridge as well. The policy is the
//! layer that stops injected markup from loading an attacker's script at all,
//! and it is worth being strict about in exactly the place where being wrong
//! costs the most.

use std::sync::OnceLock;

use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

static POLICY: OnceLock<String> = OnceLock::new();

/// Install the policy the [`security_headers`] layer will send.
///
/// Called once by the native host, before the router is built. A second call
/// is ignored: the policy belongs to the launch, and a layer whose policy
/// could change mid-run is a layer whose behaviour cannot be reasoned about.
pub fn install_csp(policy: &str) {
    let _ = POLICY.set(policy.to_string());
}

/// The installed policy, if there is one.
pub fn csp() -> Option<&'static str> {
    POLICY.get().map(String::as_str)
}

/// Add the security headers to every response.
///
/// Wired by the native host, outside the application's own layers:
///
/// ```ignore
/// router.layer(axum::middleware::from_fn(rahti_native::security_headers))
/// ```
///
/// Existing values are left alone. A page that set its own policy meant it,
/// and a layer that overwrote it would be a layer that quietly widened
/// somebody's deliberate narrowing.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    if let Some(policy) = csp() {
        if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
            if let Ok(value) = HeaderValue::from_str(policy) {
                headers.insert(header::CONTENT_SECURITY_POLICY, value);
            }
        }
    }

    // A stylesheet that a browser decided was HTML is a stylesheet that can
    // carry a script. The server sends a type for everything it serves, so
    // sniffing can only ever disagree with it.
    headers
        .entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));

    // The loopback URL carries the port, and for one request it carries the
    // launch token. Neither belongs in a `Referer` sent anywhere else.
    headers
        .entry(header::REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("no-referrer"));

    response
}

/// Put a Rahti router behind the native security layers.
///
/// One call rather than two `.layer(...)` lines in every generated shell, for
/// two reasons. The shell then needs no `axum` dependency of its own — it
/// never names a type from it — and the *order* of the two layers is decided
/// here, where it can be tested, rather than in generated code where getting
/// it backwards would be invisible.
///
/// The order: `axum` applies the last `.layer` outermost, so the gate is added
/// second and runs first. A request that did not come from this launch is
/// refused before it reaches the auth guard, the CSRF layer, a handler, or the
/// static file service.
///
/// `loopback_token` is `security.loopbackToken`. When it is off the layer is
/// not in the router at all, rather than present and deciding it does not
/// apply.
pub fn secure(router: axum::Router, loopback_token: bool) -> axum::Router {
    let router = router.layer(axum::middleware::from_fn(security_headers));
    if loopback_token {
        return router.layer(axum::middleware::from_fn(crate::gate::gate));
    }
    router
}
