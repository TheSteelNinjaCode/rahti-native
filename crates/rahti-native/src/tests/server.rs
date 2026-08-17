//! The embedded server: where it binds, that it is listening before anything
//! is told to go there, and that it stops.

use std::time::Duration;

use axum::Router;
use axum::routing::get;

use crate::server::EmbeddedServer;

fn hello() -> Router {
    Router::new().route("/", get(|| async { "hello" }))
}

/// A tiny HTTP/1.1 request over a real socket.
///
/// A real socket rather than `tower::ServiceExt::oneshot`, because half of
/// what these tests assert is about the socket: which address it is on, that
/// it accepts, and that it stops accepting.
async fn fetch(addr: std::net::SocketAddr, path: &str) -> std::io::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}

#[tokio::test]
async fn the_listener_is_loopback_only() {
    // The rule a packaged application must not break: a server on `0.0.0.0`
    // is a signed-in session offered to the user's network.
    let server = EmbeddedServer::bind().await.expect("a loopback listener");
    let addr = server.addr();

    assert!(addr.ip().is_loopback(), "bound {addr}");
    assert!(!addr.ip().is_unspecified(), "bound {addr}");
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}

#[tokio::test]
async fn the_port_is_assigned_by_the_operating_system() {
    let one = EmbeddedServer::bind().await.expect("a listener");
    let two = EmbeddedServer::bind().await.expect("a second listener");

    assert_ne!(one.port(), 0, "port 0 is a request, not an assignment");
    // Two copies of the application can run at once, which a fixed port would
    // not allow.
    assert_ne!(one.port(), two.port());
}

#[tokio::test]
async fn the_base_url_names_the_port_that_actually_bound() {
    let server = EmbeddedServer::bind().await.expect("a listener");
    assert_eq!(
        server.base_url(),
        format!("http://127.0.0.1:{}", server.addr().port())
    );
    // Not `localhost`: the name resolves to both stacks, and the origin a
    // CSRF cookie and a socket handshake are checked against must not be
    // decided by a resolver.
    assert!(!server.base_url().contains("localhost"));
}

#[tokio::test]
async fn the_port_accepts_before_anything_is_told_to_go_there() {
    // The startup race, and why `bind` is the only constructor: a URL from
    // this module names a socket that is already queueing connections.
    let server = EmbeddedServer::bind().await.expect("a listener");
    let addr = server.addr();

    assert!(
        tokio::net::TcpStream::connect(addr).await.is_ok(),
        "the bound port refused a connection before `serve` was called"
    );

    let running = server.serve(hello());
    running.wait_until_ready().await.expect("a ready server");

    let response = fetch(addr, "/").await.expect("a response");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("hello"), "{response}");

    running
        .shutdown(Duration::from_secs(2))
        .await
        .expect("a clean stop");
}

#[tokio::test]
async fn a_shutdown_stops_the_server_accepting() {
    let server = EmbeddedServer::bind().await.expect("a listener");
    let addr = server.addr();
    let running = server.serve(hello());
    running.wait_until_ready().await.expect("a ready server");

    fetch(addr, "/").await.expect("a response while running");

    running
        .shutdown(Duration::from_secs(5))
        .await
        .expect("a clean stop");

    // The listener is closed, so either the connection is refused or it is
    // accepted by nothing and answers nothing. Both are "stopped"; a full
    // HTTP response is not.
    let after = fetch(addr, "/").await;
    let stopped = match after {
        Err(_) => true,
        Ok(body) => !body.starts_with("HTTP/1.1 200"),
    };
    assert!(stopped, "the server was still answering after shutdown");
}

#[tokio::test]
async fn dropping_the_handle_stops_the_server_too() {
    // Android can destroy the process without giving anything the chance to
    // run a shutdown. A serve task that outlived its handle would hold the
    // socket into the next launch.
    let server = EmbeddedServer::bind().await.expect("a listener");
    let addr = server.addr();

    {
        let running = server.serve(hello());
        running.wait_until_ready().await.expect("a ready server");
        fetch(addr, "/").await.expect("a response while running");
    }

    // The abort is asynchronous; give the runtime a moment to run it.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let after = fetch(addr, "/").await;
    let stopped = match after {
        Err(_) => true,
        Ok(body) => !body.starts_with("HTTP/1.1 200"),
    };
    assert!(stopped, "the server outlived its handle");
}

#[tokio::test]
async fn a_bounded_shutdown_gives_up_rather_than_hanging() {
    // A graceful shutdown waits for every open connection. A WebSocket the
    // page left open has no reason to close, and a window the user closed must
    // not leave a process behind.
    let router = Router::new().route(
        "/forever",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            "never"
        }),
    );

    let server = EmbeddedServer::bind().await.expect("a listener");
    let addr = server.addr();
    let running = server.serve(router);
    running.wait_until_ready().await.expect("a ready server");

    // Hold a request open, then stop with a grace period shorter than it.
    let held = tokio::spawn(async move { fetch(addr, "/forever").await });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let started = std::time::Instant::now();
    let result = running.shutdown(Duration::from_millis(300)).await;
    let waited = started.elapsed();

    assert!(result.is_err(), "an abandoned connection is reported");
    assert!(
        waited < Duration::from_secs(5),
        "shutdown waited {waited:?} for a connection that was never going to end"
    );
    held.abort();
}
