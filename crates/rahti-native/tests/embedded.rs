//! A Rahti application over the embedded loopback server, on a real socket.
//!
//! The unit tests inside the crate cover the pieces — the listener, the gate,
//! the headers, the paths, the key. This covers them wired together the way a
//! generated native shell wires them, against a router built by hand the way
//! `src/routes.rs` builds one, with a client that is not axum.
//!
//! That distinction is the point. Every native-packaging risk lives in the gap
//! between "the router answers" and "a WebView on a loopback origin can use
//! it": a launch cookie a `fetch` does not carry, a CSRF token scoped to a port
//! that changes per launch, a streaming response a layer buffers, a multipart
//! body a middleware consumes, a socket upgrade something swallows. None of
//! those can fail in an in-process test, and all of them would ship.
//!
//! The client is hand-written over `TcpStream`. A real HTTP client would be a
//! dependency that knows how to do all of this correctly, which is exactly why
//! it is the wrong tool here — the bytes on the wire are the thing under test.

// `html!` expands to `crate::rahti::` paths, so an app has to make the runtime
// reachable there. A test crate is an app like any other.
pub use rahti;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use rahti::{Html, RpcFile, RpcStream, html, rpc};
use rahti_native::{EmbeddedServer, LAUNCH_PARAM, LaunchToken, RunningServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ------------------------------------------------------------- the app

/// A page, as `app/page.rs` writes one.
async fn page() -> Html {
    html! {
        <html lang="en">
            <body>
                <h1>"Embedded"</h1>
                <script type="module" src="/js/main.js"></script>
            </body>
        </html>
    }
}

/// An ordinary rpc — the CSRF path.
#[rpc]
async fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

/// A streaming rpc — the chunked path.
#[rpc]
async fn count(to: usize) -> rahti::Result<RpcStream> {
    let (send, stream) = RpcStream::channel(8);
    tokio::spawn(async move {
        for at in 1..=to {
            if !send.send(&at).await {
                break;
            }
        }
    });
    Ok(stream)
}

/// An upload — the multipart path.
#[rpc]
async fn receive(note: String, upload: RpcFile) -> String {
    format!("{note}:{}", upload.safe_name().unwrap_or_default())
}

/// A socket — the upgrade path.
#[rahti::socket]
async fn echo(greeting: String, socket: rahti::ws::Socket) {
    let _ = socket.send(&format!("server: {greeting}")).await;
}

/// The rpc dispatch a generated `routes.rs` writes for one page URL.
async fn rpcs(request: axum::extract::Request) -> axum::response::Response {
    let name = match rahti::rpc_name(request.headers()) {
        Ok(name) => name,
        Err(response) => return response,
    };
    match name.as_str() {
        "greet" => __rahti_rpc_greet(request).await,
        "count" => __rahti_rpc_count(request).await,
        "receive" => __rahti_rpc_receive(request).await,
        other => rahti::rpc_unknown(other),
    }
}

/// The router, with the layers codegen emits and the two a native shell adds.
fn router(public: &std::path::Path) -> Router {
    let app = Router::new()
        .route("/", get(page))
        .route("/", post(rpcs))
        .merge(rahti::ws::routes())
        .fallback_service(tower_http::services::ServeDir::new(public))
        .layer(axum::middleware::from_fn(rahti::csrf));

    // Exactly what the generated shell does: the gate outermost, the security
    // headers under it.
    rahti_native::secure(app, true)
}

// ------------------------------------------------------------ the client

/// Serializes the tests in this binary.
///
/// The framework and native runtime intentionally keep process-wide state such
/// as auth configuration, launch tokens and environment overrides. A packaged
/// process has one application; this test binary starts several, so they run
/// one at a time.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Packaged {
    server: RunningServer,
    addr: SocketAddr,
    cookies: HashMap<String, String>,
    _assets: TempDir,
    /// Held for the life of the test — see [`ONE_AT_A_TIME`].
    _exclusive: tokio::sync::MutexGuard<'static, ()>,
}

impl Packaged {
    async fn start() -> Self {
        let exclusive = ONE_AT_A_TIME.lock().await;

        // A packaged application serves assets staged out of its binary; this
        // stands in for that, so the static path is exercised as an absolute
        // one an installed program would have.
        let assets = TempDir::new("assets");
        std::fs::create_dir_all(assets.path().join("js")).unwrap();
        std::fs::write(assets.path().join("js/main.js"), "the runtime").unwrap();

        let server = EmbeddedServer::bind().await.expect("a loopback listener");
        let addr = server.addr();
        let server = server.serve(router(assets.path()));
        server.wait_until_ready().await.expect("a ready server");

        let mut packaged = Packaged {
            server,
            addr,
            cookies: HashMap::new(),
            _assets: assets,
            _exclusive: exclusive,
        };
        packaged.launch().await;
        packaged
    }

    /// What the shell does first: open the launch URL, take the cookie, follow
    /// the redirect to the clean one.
    async fn launch(&mut self) {
        let url = LaunchToken::launch_url(&format!("http://127.0.0.1:{}", self.addr.port()));
        let path = url.split_once("/?").map(|(_, q)| format!("/?{q}")).unwrap();

        let response = self.send(Request::get(&path)).await;
        assert_eq!(response.status, 303, "the launch URL did not admit us");
        assert_eq!(response.header("location").as_deref(), Some("/"));
        assert!(self.cookies.contains_key(LAUNCH_PARAM));
    }

    async fn send(&mut self, mut request: Request) -> Response {
        if !self.cookies.is_empty() {
            let jar: Vec<String> = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            request.headers.push(("Cookie".into(), jar.join("; ")));
        }
        let response = request.send(self.addr).await;
        for (name, value) in &response.set_cookies {
            self.cookies.insert(name.clone(), value.clone());
        }
        response
    }

    async fn rpc(&mut self, name: &str, body: &str) -> Response {
        let token = self.csrf();
        let mut request = Request::post("/", "application/json", body.as_bytes().to_vec());
        request.headers.push(("X-PP-Function".into(), name.into()));
        if let Some(token) = token {
            request.headers.push(("X-CSRF-Token".into(), token));
        }
        self.send(request).await
    }

    /// Port-scoped, which is the detail worth a test: the port is assigned per
    /// launch, so the cookie's name is not knowable until the server has bound.
    fn csrf(&self) -> Option<String> {
        self.cookies
            .get(&format!("pp_csrf_{}", self.addr.port()))
            .or_else(|| self.cookies.get("pp_csrf"))
            .cloned()
    }
}

struct Request {
    method: &'static str,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn get(path: &str) -> Self {
        Request {
            method: "GET",
            path: path.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn post(path: &str, content_type: &str, body: Vec<u8>) -> Self {
        Request {
            method: "POST",
            path: path.to_string(),
            headers: vec![("Content-Type".into(), content_type.into())],
            body,
        }
    }

    async fn send(self, addr: SocketAddr) -> Response {
        let mut stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("a connection");

        let mut head = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        head.push_str("Host: 127.0.0.1\r\nConnection: close\r\n");
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));

        stream.write_all(head.as_bytes()).await.expect("a request");
        stream.write_all(&self.body).await.expect("a body");

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.expect("a response");
        Response::parse(&raw)
    }
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    set_cookies: Vec<(String, String)>,
    body: String,
}

impl Response {
    fn parse(raw: &[u8]) -> Self {
        let text = String::from_utf8_lossy(raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));

        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);

        let mut headers = Vec::new();
        let mut set_cookies = Vec::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
            if name == "set-cookie" {
                let pair = value.split(';').next().unwrap_or_default();
                if let Some((cookie, cookie_value)) = pair.split_once('=') {
                    set_cookies.push((cookie.trim().to_string(), cookie_value.trim().to_string()));
                }
            }
            headers.push((name, value));
        }

        Response {
            status,
            body: dechunk(body, &headers),
            headers,
            set_cookies,
        }
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.clone())
    }
}

/// Unwrap `Transfer-Encoding: chunked`, which axum uses for a response whose
/// length it does not know in advance.
fn dechunk(body: &str, headers: &[(String, String)]) -> String {
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
    if !chunked {
        return body.to_string();
    }

    let mut out = String::new();
    let mut rest = body;
    while let Some((size, remainder)) = rest.split_once("\r\n") {
        let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if size == 0 || remainder.len() < size {
            break;
        }
        out.push_str(&remainder[..size]);
        rest = &remainder[size + 2..];
    }
    out
}

/// A temporary directory that deletes itself.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rahti-native-embedded-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a test directory");
        TempDir(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// -------------------------------------------------------------- the tests

#[tokio::test]
async fn a_page_renders_through_the_embedded_server() {
    let mut app = Packaged::start().await;
    let response = app.send(Request::get("/")).await;

    assert_eq!(response.status, 200);
    assert!(
        response
            .body
            .to_ascii_lowercase()
            .contains("<h1>embedded</h1>"),
        "{}",
        response.body
    );
    // And the security headers a native shell adds.
    assert_eq!(
        response.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn a_static_asset_is_served_from_an_absolute_packaged_path() {
    // `ServeDir` resolves a non-absolute path against the working directory,
    // and an installed application does not control that — so a packaged
    // application serves from an absolute one under application storage.
    let mut app = Packaged::start().await;
    let response = app.send(Request::get("/js/main.js")).await;

    assert_eq!(response.status, 200, "the packaged asset path 404'd");
    assert!(response.body.contains("the runtime"), "{}", response.body);

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn an_rpc_answers_over_the_socket_with_its_csrf_token() {
    let mut app = Packaged::start().await;
    app.send(Request::get("/")).await;
    assert!(app.csrf().is_some(), "no CSRF cookie was issued");

    let response = app.rpc("greet", r#"{"name":"Ada"}"#).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains("Hello, Ada!"), "{}", response.body);

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn csrf_is_still_enforced_behind_the_launch_gate() {
    // The gate proves the request came from this application. It does not
    // prove the call came from a page of ours, which is a different question
    // and still CSRF's to answer.
    let mut app = Packaged::start().await;
    app.send(Request::get("/")).await;

    let mut request = Request::post("/", "application/json", br#"{"name":"Ada"}"#.to_vec());
    request
        .headers
        .push(("X-PP-Function".into(), "greet".into()));
    let response = app.send(request).await;

    assert_eq!(response.status, 403, "an rpc without a token was answered");

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn a_streaming_rpc_arrives_in_pieces_rather_than_at_the_end() {
    // The risk a gate or a header layer introduces is buffering: a streaming
    // response collected and sent whole still passes a "is the body right"
    // test and has stopped being a stream.
    let mut app = Packaged::start().await;
    app.send(Request::get("/")).await;

    let response = app.rpc("count", r#"{"to":3}"#).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        response
            .header("transfer-encoding")
            .is_some_and(|value| value.contains("chunked")),
        "the streaming rpc was not chunked: {:?}",
        response.headers
    );
    assert!(response.body.contains('1'), "{}", response.body);
    assert!(response.body.contains('3'), "{}", response.body);

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn a_multipart_upload_reaches_its_handler() {
    // A multipart body is the one a middleware is most likely to consume: it
    // is large, it is streamed, and a layer that reads it to inspect it leaves
    // the handler nothing.
    let mut app = Packaged::start().await;
    app.send(Request::get("/")).await;

    const BOUNDARY: &str = "----rahtinativetest";
    let mut body = Vec::new();
    for (name, filename, value) in [
        ("note", None, "from the embedded server"),
        ("upload", Some("hello.txt"), "the file contents"),
    ] {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(filename) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n\
                     Content-Type: text/plain\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());

    let token = app.csrf().expect("a csrf token");
    let mut request = Request::post(
        "/",
        &format!("multipart/form-data; boundary={BOUNDARY}"),
        body,
    );
    request
        .headers
        .push(("X-PP-Function".into(), "receive".into()));
    request.headers.push(("X-CSRF-Token".into(), token));

    let response = app.send(request).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains("hello.txt"), "{}", response.body);

    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn a_socket_handshake_upgrades_and_carries_a_message() {
    use futures_util::{SinkExt, StreamExt};

    let app = Packaged::start().await;
    let origin = format!("http://127.0.0.1:{}", app.addr.port());

    // Cookies on a handshake are what the gate needs and what a header scheme
    // could not have supplied — this is the case that decided the design.
    let jar: Vec<String> = app
        .cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();

    let request = http::Request::builder()
        .uri(format!(
            "ws://127.0.0.1:{}/__pulsepoint/ws?name=echo",
            app.addr.port()
        ))
        .header("Host", format!("127.0.0.1:{}", app.addr.port()))
        // Same-origin: `rahti::ws` refuses a handshake from anywhere else, and
        // a loopback origin has to satisfy that like any other.
        .header("Origin", &origin)
        .header("Cookie", jar.join("; "))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .expect("a handshake request");

    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("the socket upgraded");
    assert_eq!(response.status().as_u16(), 101);

    // The first frame is the arguments, which is how `#[socket]` reads them.
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"greeting":"hello"}"#.into(),
        ))
        .await
        .expect("the arguments");

    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("a reply within five seconds")
        .expect("a frame")
        .expect("a message");
    assert!(
        reply
            .to_text()
            .unwrap_or_default()
            .contains("server: hello")
    );

    let _ = socket.close(None).await;
    app.server.shutdown(Duration::from_secs(5)).await.unwrap();
}

#[tokio::test]
async fn the_host_stops_the_server_when_the_window_closes() {
    // A packaged GUI never receives Ctrl+C and Android's lifecycle has nothing
    // like it, so the host says when. What must not happen is a process left
    // behind holding a port.
    let mut app = Packaged::start().await;
    let addr = app.addr;
    assert_eq!(app.send(Request::get("/")).await.status, 200);

    app.server
        .shutdown(Duration::from_secs(5))
        .await
        .expect("a clean stop");

    let after = tokio::net::TcpStream::connect(addr).await;
    let stopped = match after {
        Err(_) => true,
        Ok(mut stream) => {
            let _ = stream
                .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .await;
            let mut buffer = Vec::new();
            let read =
                tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buffer)).await;
            !matches!(read, Ok(Ok(n)) if n > 0 && buffer.starts_with(b"HTTP/1.1 200"))
        }
    };
    assert!(stopped, "the server was still answering after shutdown");
}
