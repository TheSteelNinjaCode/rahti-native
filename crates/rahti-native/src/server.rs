//! The embedded loopback server.
//!
//! A packaged Rahti application serves itself. The generated Axum router — the
//! same one a deployment runs, with the same auth guard, the same CSRF layer
//! and the same static fallback — binds a socket inside the installed process,
//! and the WebView is pointed at it.
//!
//! Serving HTTP to a WebView on the same machine looks like a detour, and the
//! alternative is worse. Rahti's RPCs are Axum requests: they carry cookies,
//! a CSRF header, multipart bodies and streaming responses, and its sockets
//! are real HTTP upgrades. A Tauri IPC transport would have to reimplement
//! every one of those, and would be a different protocol wearing the same
//! names. Keeping HTTP keeps the application identical on both sides.
//!
//! ## Two rules, both about the address
//!
//! **Loopback only.** [`EmbeddedServer::bind`] binds `127.0.0.1` and offers no
//! way to bind anything else. A packaged application that bound the configured
//! `0.0.0.0:3000` would be a web server on the user's network, serving their
//! signed-in session to it — which is what a deployment wants and is a
//! vulnerability in a program somebody installed.
//!
//! **A port the operating system picks.** Port `0` asks for a free one. A
//! fixed port collides with whatever else holds it, and two copies of the
//! application could not run at once.
//!
//! ## Bind, then serve, then navigate
//!
//! The order is the whole of the startup race, and it is enforced by the
//! types: [`EmbeddedServer::bind`] is the only constructor, it is `async`, and
//! it returns a value that already holds a bound listener. There is no way to
//! obtain a URL from this module that nothing is listening on.
//!
//! That the socket is *bound* is the part that matters, not that
//! [`RunningServer`] has started polling it. A bound TCP listener queues
//! connections in the kernel from the moment it exists, so a WebView that
//! races ahead and connects is not refused — it waits, and is answered when
//! the accept loop reaches it. [`RunningServer::wait_until_ready`] is
//! available for a host that would rather prove it than reason about it.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use axum::Router;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::NativeError;

/// How long [`RunningServer::wait_until_ready`] tries before giving up.
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a shutdown waits for open connections before it stops waiting.
///
/// Bounded because a graceful shutdown waits for *every* open connection, and
/// a WebSocket that a page left open has no reason to close on its own. Rahti's
/// shutdown broadcast tells those to end, and this is the answer to the ones
/// that do not.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// A bound loopback listener, not yet serving anything.
pub struct EmbeddedServer {
    listener: TcpListener,
    addr: SocketAddr,
}

impl EmbeddedServer {
    /// Bind `127.0.0.1` on a port the operating system chooses.
    ///
    /// There is deliberately no `bind_to(host, port)`. Every reason to want
    /// one — a fixed port for a bookmark, a LAN address for a second device —
    /// is a reason a packaged application should not have.
    pub async fn bind() -> Result<Self, NativeError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| {
                NativeError::new(
                    "listener",
                    format!("cannot bind a loopback port for the embedded server: {e}"),
                )
            })?;

        let addr = listener.local_addr().map_err(|e| {
            NativeError::new("listener", format!("the listener has no address: {e}"))
        })?;

        Ok(EmbeddedServer { listener, addr })
    }

    /// The address that actually bound, port included.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The port the operating system assigned.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Where to point the WebView.
    ///
    /// `127.0.0.1` rather than `localhost`: the name resolves to both stacks
    /// and a WebView that tried `::1` first would spend a timeout on every
    /// launch reaching a server that is not there. It is also the origin the
    /// document will be on, and an origin that is decided by a resolver is one
    /// that CSRF and the socket handshake's origin check cannot rely on.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Start serving `router`, on a task.
    ///
    /// Returns immediately. The listener was bound by [`bind`](Self::bind), so
    /// the port in [`base_url`](Self::base_url) is already accepting.
    pub fn serve(self, router: Router) -> RunningServer {
        let addr = self.addr;
        let (stop, stopped) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            axum::serve(self.listener, router)
                .with_graceful_shutdown(async {
                    // A closed sender counts as a stop: it means the
                    // `RunningServer` was dropped without a shutdown, which is
                    // the host going away.
                    let _ = stopped.await;
                })
                .await
        });

        RunningServer {
            addr,
            stop: Some(stop),
            task: Some(task),
        }
    }
}

/// A server that is serving.
pub struct RunningServer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl RunningServer {
    /// The address it is serving on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Where to point the WebView.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Prove the port answers before anything is told to go there.
    ///
    /// Not strictly required — see the module note on binding before serving —
    /// but a host that would rather check than reason gets a check, and one
    /// that finds this failing has a real problem to report rather than a
    /// blank window.
    pub async fn wait_until_ready(&self) -> Result<(), NativeError> {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;

        loop {
            if TcpStream::connect(self.addr).await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(NativeError::new(
                    "server",
                    format!(
                        "the embedded server did not answer on {} within {} seconds",
                        self.addr,
                        READY_TIMEOUT.as_secs()
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Stop accepting, let open work finish, and end.
    ///
    /// The host must first signal its application's framework shutdown
    /// broadcast, which closes long-lived responses such as the dev event
    /// stream and WebSockets. This method then tells Axum to stop accepting and
    /// waits for what is left. Keeping the framework signal in the host makes
    /// it use the application's Rahti version rather than a second copy linked
    /// by this platform-neutral crate.
    ///
    /// `grace` bounds that wait. Whatever is still open when it expires is
    /// abandoned, because a window the user closed must not leave a process
    /// behind.
    pub async fn shutdown(mut self, grace: Duration) -> Result<(), NativeError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }

        let Some(task) = self.task.take() else {
            return Ok(());
        };

        match tokio::time::timeout(grace, task).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(NativeError::new(
                "server",
                format!("the embedded server stopped with an error: {e}"),
            )),
            Ok(Err(e)) => Err(NativeError::new(
                "server",
                format!("the embedded server task did not finish: {e}"),
            )),
            Err(_) => Err(NativeError::new(
                "server",
                format!(
                    "the embedded server still had work open after {} seconds and was abandoned",
                    grace.as_secs()
                ),
            )),
        }
    }
}

impl Drop for RunningServer {
    /// A host that drops this without calling
    /// [`shutdown`](RunningServer::shutdown) still stops the server.
    ///
    /// Which is the Android case: the operating system can destroy the process
    /// without giving anything a chance to run a shutdown, and a serve task
    /// that outlived its handle would keep a socket open into the next launch.
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
