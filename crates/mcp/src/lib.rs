//! Model Context Protocol (MCP) server: lets an AI agent watch and drive the simulator.
//!
//! The tools do what the app's panels do, through the same engine API: read the state, steer,
//! change the environment, override values, manage AIS targets, sensors and outputs, load
//! scenarios and read the data being sent.
//!
//! Two transports:
//! * [`McpServer::start_http`]: Streamable HTTP on `http://127.0.0.1:<port>/mcp`, inside the
//!   running app or CLI. Loopback only; requests from web pages (an `Origin` that is not
//!   localhost) are refused.
//! * [`serve_stdio`]: stdin/stdout, for agents that start `marine-sim-cli --mcp-stdio`
//!   themselves.
//!
//! The server never reads or writes files: scenarios that name a polar or soundings file, and
//! file or serial outputs, are refused here (use the app or the CLI for those).

mod tools;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{Context as _, Result};
use engine::Engine;
use engine::scenario::Scenario;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

pub use tools::SimTools;

/// The port the app proposes: near the default NMEA port (10113); 10114 is the one it
/// proposes for WebSocket outputs.
pub const DEFAULT_PORT: u16 = 10115;

/// What the server drives. The app implements it to keep its window in step.
pub trait Host: Send + Sync + 'static {
    fn engine(&self) -> &Engine;

    /// Replaces the whole state with `scenario`.
    fn load_scenario(&self, scenario: Scenario) {
        self.engine().load_scenario(scenario);
    }

    /// The vessel was moved instantly (the app clears its track).
    fn vessel_moved(&self) {}
}

/// A host that is only an engine (the CLI).
pub struct EngineHost(pub Arc<Engine>);

impl Host for EngineHost {
    fn engine(&self) -> &Engine {
        &self.0
    }
}

/// A running Streamable HTTP server; stops when dropped.
pub struct McpServer {
    addr: SocketAddr,
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl McpServer {
    /// Serves `host` on `127.0.0.1:port` (0 = any free port). Fails at once when the port is
    /// taken.
    pub fn start_http(host: Arc<dyn Host>, port: u16) -> Result<McpServer> {
        let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = std::net::TcpListener::bind(bind).with_context(|| format!("binding {bind}"))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;

        let ct = cancel.clone();
        let thread = std::thread::Builder::new().name("mcp".into()).spawn(move || {
            runtime.block_on(async move {
                let config = StreamableHttpServerConfig::default()
                    .with_cancellation_token(ct.clone())
                    // A browser page must not drive the simulator: only local origins pass.
                    // (Agents are not browsers and send no Origin.)
                    .with_allowed_origins(["http://localhost:*", "http://127.0.0.1:*", "http://[::1]:*"]);
                let service: StreamableHttpService<SimTools, LocalSessionManager> =
                    StreamableHttpService::new(move || Ok(SimTools::new(host.clone())), Default::default(), config);
                let router = axum::Router::new().nest_service("/mcp", service);
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => return tracing::error!("MCP server: {e}"),
                };
                tracing::info!("MCP server: http://{addr}/mcp");
                if let Err(e) = axum::serve(listener, router).with_graceful_shutdown(ct.cancelled_owned()).await {
                    tracing::error!("MCP server: {e}");
                }
            });
        })?;
        Ok(McpServer { addr, cancel, thread: Some(thread) })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The address agents connect to.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Wait for the port to be free again, but never hold the caller (the app's window) for
        // a connection that is slow to close: the thread then ends on its own.
        if let Some(t) = self.thread.take() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !t.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}

/// Serves `host` on stdin/stdout until the client closes them. Nothing else may write to stdout.
pub fn serve_stdio(host: Arc<dyn Host>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async move {
        let service = SimTools::new(host).serve(rmcp::transport::stdio()).await.context("starting MCP on stdio")?;
        service.waiting().await.context("MCP on stdio")?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;

    /// One HTTP POST to the server; returns what comes back within a second.
    fn post(port: u16, extra_headers: &str, body: &str) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        write!(
            s,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{extra_headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        // The reply is a stream that stays open: read until it pauses.
        let mut out = vec![];
        let mut buf = [0u8; 4096];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn http_handshake_and_refusals() {
        let mut sc = engine::scenario::default_scenario();
        sc.outputs.clear();
        let host = Arc::new(EngineHost(Arc::new(Engine::start(sc, None).unwrap())));
        let server = McpServer::start_http(host.clone(), 0).unwrap();
        let port = server.port();
        assert_eq!(server.url(), format!("http://127.0.0.1:{port}/mcp"));

        let ok = post(port, "", INIT);
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains(r#""name":"marine-data-simulator""#), "{ok}");

        // A web page (any Origin that is not this computer) may not drive the simulator.
        let page = post(port, "Origin: https://example.com\r\n", INIT);
        assert!(page.starts_with("HTTP/1.1 403"), "{page}");
        let local = post(port, "Origin: http://localhost:3000\r\n", INIT);
        assert!(local.starts_with("HTTP/1.1 200"), "{local}");

        // A port in use is reported at once, and a stopped server frees its port.
        assert!(McpServer::start_http(host.clone(), port).is_err());
        drop(server);
        assert!(McpServer::start_http(host, port).is_ok());
    }
}
