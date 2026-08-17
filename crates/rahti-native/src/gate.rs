//! Keeping the loopback server to the application that started it.
//!
//! ## The threat
//!
//! `127.0.0.1` is not a private address. It is unreachable from the network
//! and reachable by *every process on the machine* — every other application
//! the user installed, every browser tab that can be persuaded to fetch a
//! local URL, every script they were talked into running. A packaged Rahti
//! application listening on a loopback port is a signed-in session, an upload
//! endpoint and a database, exposed to all of them.
//!
//! The port is assigned by the operating system, which helps and does not
//! solve it: 65535 ports is a few seconds of scanning, and Rahti's own pages
//! identify themselves.
//!
//! ## The answer, and why it is this one
//!
//! A random token is minted per launch. The host opens
//! `http://127.0.0.1:<port>/?__rahti_native=<token>`; this layer recognizes
//! it, sets it as a cookie, and redirects to the clean URL. Every later
//! request — pages, assets, `pp.rpc`, a streaming response, a multipart
//! upload, a WebSocket handshake — carries the cookie because the browser
//! carries cookies, and is allowed. A request without it gets 403 and nothing
//! else.
//!
//! A cookie rather than a header is the whole design. A header can be attached
//! to `fetch` and to nothing else: a header scheme would allow RPCs and refuse
//! the document, the stylesheet, the PulsePoint bundle, an `<img>`, a file
//! download and the WebSocket upgrade. The cookie is the one credential the
//! browser attaches to *every* kind of request a Rahti page makes, which is
//! why this scheme costs no feature.
//!
//! ## What it is not
//!
//! It is not authentication, and it does not replace CSRF. It answers "is this
//! request from the WebView this application opened", and Rahti's own layers
//! still answer "is there a session" and "did this call come from a page of
//! ours". A local attacker who can read the application's own cookie jar has
//! already won and this does not pretend otherwise.

use std::sync::OnceLock;

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The query parameter the launch URL carries, and the cookie it becomes.
pub const LAUNCH_PARAM: &str = "__rahti_native";

/// This launch's token.
///
/// Generated once per process, on first use. A new one every launch is the
/// point: a token that survived a restart would be a token that could be read
/// off disk.
///
/// If the operating system will not provide randomness — which on the
/// platforms this runs on means something is very wrong — the token becomes a
/// value nothing can present, so the gate refuses everything rather than
/// accepting anything. A closed door is the right way to fail.
pub fn launch_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut bytes = [0u8; 24];
        match getrandom::fill(&mut bytes) {
            Ok(()) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
            Err(_) => String::new(),
        }
    })
}

/// The URL the host points the WebView at.
///
/// `base` is [`crate::EmbeddedServer::base_url`].
pub struct LaunchToken;

impl LaunchToken {
    /// The one URL that will be let in without a cookie.
    pub fn launch_url(base: &str) -> String {
        format!(
            "{}/?{LAUNCH_PARAM}={}",
            base.trim_end_matches('/'),
            launch_token()
        )
    }
}

/// Refuse anything that did not come from this launch.
///
/// Wired by the native host as the outermost layer:
///
/// ```ignore
/// router.layer(axum::middleware::from_fn(rahti_native::gate))
/// ```
///
/// Wired *only* when `security.loopbackToken` is on, which is why there is no
/// runtime switch here — a project that turned it off does not have this layer
/// in its router at all, rather than having a layer that decides it does not
/// apply.
pub async fn gate(request: Request, next: Next) -> Response {
    let token = launch_token();

    // An empty token means the launch had no randomness. Nothing can match it,
    // which is deliberate.
    if token.is_empty() {
        return refuse();
    }

    if cookie_matches(&request, token) {
        return next.run(request).await;
    }

    if let Some(clean) = query_matches(&request, token) {
        return admit(&clean, token);
    }

    refuse()
}

/// Whether the request carries the launch cookie.
fn cookie_matches(request: &Request, token: &str) -> bool {
    let Some(header) = request.headers().get(header::COOKIE) else {
        return false;
    };
    let Ok(header) = header.to_str() else {
        return false;
    };

    header
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .any(|(name, value)| name.trim() == LAUNCH_PARAM && constant_time_eq(value.trim(), token))
}

/// Whether this is the launch URL, and what to redirect to if it is.
///
/// Only a `GET` qualifies. The launch URL is a navigation, and accepting a
/// token in the query string of a `POST` would put it in reach of any page
/// that could get the application to submit a form.
fn query_matches(request: &Request, token: &str) -> Option<String> {
    if request.method() != axum::http::Method::GET {
        return None;
    }

    let uri = request.uri();
    let query = uri.query()?;

    let mut carried = false;
    let mut rest: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some((LAUNCH_PARAM, value)) => carried = constant_time_eq(value, token),
            _ => rest.push(pair),
        }
    }
    if !carried {
        return None;
    }

    // Back to the URL without the token, so it does not sit in the address bar,
    // in `document.location`, or in a `Referer` header on the next request.
    let path = uri.path();
    Some(if rest.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", rest.join("&"))
    })
}

/// Set the cookie and send the WebView to the clean URL.
fn admit(clean: &str, token: &str) -> Response {
    // `HttpOnly`, because no page has any reason to read it and an XSS that
    // could would have stolen the one credential this layer rests on.
    // `SameSite=Strict`, because every request the application makes is to its
    // own origin, so nothing legitimate is lost and a cross-site navigation
    // into the port arrives without it.
    // No `Secure`: the loopback origin is `http:`, and a `Secure` cookie on it
    // is a cookie the browser will not send.
    let cookie = format!("{LAUNCH_PARAM}={token}; Path=/; HttpOnly; SameSite=Strict");

    let mut response = (StatusCode::SEE_OTHER, "").into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, value);
    }
    if let Ok(value) = HeaderValue::from_str(clean) {
        headers.insert(header::LOCATION, value);
    }
    response
}

/// 403 and nothing informative.
///
/// No body describing the scheme, no header naming the application: a process
/// scanning loopback ports learns that something refused it.
fn refuse() -> Response {
    StatusCode::FORBIDDEN.into_response()
}

/// Compare without leaking where two values first differ.
///
/// The token is a secret being compared against attacker-supplied input, which
/// is the case `==` on a `str` is wrong for.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        .eq(&0)
}
