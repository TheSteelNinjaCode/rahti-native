//! The loopback gate: that it lets this launch's WebView in, and everything
//! else out, without costing a Rahti feature.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::gate::{LAUNCH_PARAM, LaunchToken, gate, launch_token};

fn app() -> Router {
    Router::new()
        .route("/", get(|| async { "the page" }))
        .route("/js/main.js", get(|| async { "the runtime" }))
        .route("/todo", post(|| async { "an rpc" }))
        .layer(axum::middleware::from_fn(gate))
}

async fn send(request: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app().oneshot(request).await.expect("a response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).to_string())
}

fn with_cookie(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::COOKIE, format!("{LAUNCH_PARAM}={token}"))
        .body(Body::empty())
        .unwrap()
}

#[test]
fn the_token_is_random_and_stable_within_a_launch() {
    let once = launch_token();
    assert_eq!(once, launch_token(), "the token changed mid-launch");
    assert_eq!(once.len(), 48, "24 random bytes, as hex");
    assert!(once.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_launch_url_carries_the_token_once() {
    let url = LaunchToken::launch_url("http://127.0.0.1:5173");
    assert_eq!(
        url,
        format!("http://127.0.0.1:5173/?{LAUNCH_PARAM}={}", launch_token())
    );
    // A trailing slash on the base must not produce `//`.
    assert_eq!(url, LaunchToken::launch_url("http://127.0.0.1:5173/"));
}

#[tokio::test]
async fn a_request_with_no_token_is_refused() {
    // Every other process on the machine can reach this port.
    let (status, _, body) = send(Request::builder().uri("/").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nothing that tells a port scanner what it found.
    assert!(body.is_empty(), "{body:?}");
}

#[tokio::test]
async fn a_request_with_the_wrong_token_is_refused() {
    let (status, _, _) = send(with_cookie("/", &"0".repeat(48))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_launch_url_sets_the_cookie_and_redirects_to_the_clean_url() {
    let (status, headers, _) = send(
        Request::builder()
            .uri(format!("/?{LAUNCH_PARAM}={}", launch_token()))
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::SEE_OTHER);

    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with(&format!("{LAUNCH_PARAM}={}", launch_token())));
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    // The loopback origin is `http:`, and a `Secure` cookie on it is a cookie
    // the browser will not send.
    assert!(!cookie.contains("Secure"), "{cookie}");

    // The token does not stay in the address bar, in `document.location`, or
    // in a `Referer` header on the next request.
    assert_eq!(headers[header::LOCATION], "/");
}

#[tokio::test]
async fn the_launch_url_keeps_the_querystring_it_was_given() {
    let (_, headers, _) = send(
        Request::builder()
            .uri(format!(
                "/?next=%2Faccount&{LAUNCH_PARAM}={}",
                launch_token()
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(headers[header::LOCATION], "/?next=%2Faccount");
}

#[tokio::test]
async fn the_cookie_admits_everything_a_rahti_page_needs() {
    // The reason the credential is a cookie and not a header: a header can be
    // attached to `fetch` and to nothing else, so a header scheme would allow
    // RPCs and refuse the document, the stylesheet and the runtime bundle.
    let token = launch_token();

    let (status, _, body) = send(with_cookie("/", token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "the page");

    let (status, _, body) = send(with_cookie("/js/main.js", token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "the runtime");

    let rpc = Request::builder()
        .method("POST")
        .uri("/todo")
        .header(header::COOKIE, format!("{LAUNCH_PARAM}={token}"))
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(rpc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "an rpc");
}

#[tokio::test]
async fn the_cookie_is_found_beside_the_others_a_rahti_page_carries() {
    // A real request carries the session cookie and the CSRF cookie too.
    let request = Request::builder()
        .uri("/")
        .header(
            header::COOKIE,
            format!(
                "rahti_session_9f2c=abc; {LAUNCH_PARAM}={}; rahti_csrf_3000=def",
                launch_token()
            ),
        )
        .body(Body::empty())
        .unwrap();

    let (status, _, _) = send(request).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_token_in_the_query_of_a_post_is_not_accepted() {
    // A navigation is a GET. Accepting a token in a POST query string would
    // put it in reach of any page that could get the application to submit a
    // form.
    let request = Request::builder()
        .method("POST")
        .uri(format!("/todo?{LAUNCH_PARAM}={}", launch_token()))
        .body(Body::empty())
        .unwrap();

    let (status, _, _) = send(request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_cookie_header_that_is_not_a_cookie_header_is_refused_quietly() {
    for bad in ["", ";;;", "=", LAUNCH_PARAM, "__rahti_native"] {
        let request = Request::builder()
            .uri("/")
            .header(header::COOKIE, bad)
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = send(request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bad:?} was accepted");
    }
}

#[tokio::test]
async fn a_prefix_of_the_token_is_not_the_token() {
    let short = &launch_token()[..20];
    let (status, _, _) = send(with_cookie("/", short)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
