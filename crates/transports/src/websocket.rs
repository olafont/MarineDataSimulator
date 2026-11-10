//! WebSocket server: every connected client receives each line as one text frame (without the
//! line ending). Messages from clients are ignored.
//!
//! A Signal K output also behaves like a minimal Signal K server: it sends the hello message
//! first, and answers plain HTTP `GET /signalk` with the discovery document, so clients that
//! discover the stream endpoint find `ws://<host>/signalk/v1/stream` (any path is accepted).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::{Message, OutputConfig, Stats, Status, bind_listener, hello_line, lag_status};

pub(crate) async fn run(
    cfg: &OutputConfig,
    bind: &str,
    rx: broadcast::Receiver<Message>,
    stats: &Arc<Stats>,
    hello: Option<Arc<Mutex<String>>>,
) -> Result<()> {
    let listener = bind_listener(bind).await?;
    stats.set_status(Status::Running);
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut stream, peer) = match accepted {
                    Ok(a) => a,
                    Err(e) => {
                        stats.error(1);
                        tracing::debug!("accept: {e}");
                        continue;
                    }
                };
                let _ = stream.set_nodelay(true);
                let mut rx = rx.resubscribe();
                let stats = stats.clone();
                let cfg = cfg.clone();
                let hello = hello.clone();
                clients.spawn(async move {
                    if hello.is_some() && answer_discovery(&mut stream).await {
                        return;
                    }
                    let ws = match tokio_tungstenite::accept_async(stream).await {
                        Ok(ws) => ws,
                        Err(e) => {
                            stats.error(1);
                            tracing::info!(output = %cfg.name, "websocket handshake with {peer}: {e}");
                            return;
                        }
                    };
                    tracing::info!(output = %cfg.name, "websocket client connected: {peer}");
                    stats.clients.fetch_add(1, Ordering::Relaxed);
                    let (mut sink, mut source) = ws.split();
                    let greeting = hello_line(hello.as_ref());
                    let mut ok = match greeting {
                        Some(g) => sink.send(WsMessage::text(g.trim_end())).await.is_ok(),
                        None => true,
                    };
                    while ok {
                        tokio::select! {
                            msg = rx.recv() => match msg {
                                Ok(m) if cfg.takes(&m) => {
                                    let text = m.data.trim_end();
                                    if sink.send(WsMessage::text(text)).await.is_err() {
                                        ok = false;
                                    }
                                    stats.count(text.len());
                                }
                                Ok(_) => {}
                                Err(broadcast::error::RecvError::Lagged(n)) => lag_status(&stats, n, "the client"),
                                Err(broadcast::error::RecvError::Closed) => ok = false,
                            },
                            incoming = source.next() => match incoming {
                                None | Some(Err(_)) | Some(Ok(WsMessage::Close(_))) => ok = false,
                                Some(Ok(_)) => {}
                            },
                        }
                    }
                    stats.clients.fetch_sub(1, Ordering::Relaxed);
                    tracing::info!(output = %cfg.name, "websocket client disconnected: {peer}");
                });
            }
            Some(_) = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

/// Reads the request head without consuming it. If it is a plain HTTP request (not a WebSocket
/// upgrade), answers it: the discovery document for `/signalk`, 404 otherwise. Returns whether
/// the connection was answered (and should be closed).
async fn answer_discovery(stream: &mut TcpStream) -> bool {
    let mut buf = [0u8; 4096];
    let mut head = String::new();
    for _ in 0..40 {
        match stream.peek(&mut buf).await {
            Ok(0) | Err(_) => return true,
            Ok(n) => {
                head = String::from_utf8_lossy(&buf[..n]).into_owned();
                if head.contains("\r\n\r\n") || n == buf.len() {
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let lower = head.to_ascii_lowercase();
    if lower.contains("upgrade: websocket") {
        return false;
    }
    let path = head.split_whitespace().nth(1).unwrap_or("/");
    let host = head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.trim().to_string()))
        .or_else(|| stream.local_addr().ok().map(|a| a.to_string()))
        .unwrap_or_default();
    let (status, body) = if path.trim_end_matches('/') == "/signalk" {
        ("200 OK", discovery(&host))
    } else {
        ("404 Not Found", "{\"message\":\"use /signalk for discovery, or connect a WebSocket\"}".to_string())
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // Consume the request so the peer sees a clean close.
    let n = head.len().min(buf.len());
    let _ = tokio::io::AsyncReadExt::read(stream, &mut buf[..n]).await;
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    true
}

/// The Signal K discovery document.
fn discovery(host: &str) -> String {
    format!(
        "{{\"endpoints\":{{\"v1\":{{\"version\":\"1.7.0\",\"signalk-ws\":\"ws://{host}/signalk/v1/stream\"}}}},\
         \"server\":{{\"id\":\"marine-data-simulator\",\"version\":\"{}\"}}}}",
        env!("CARGO_PKG_VERSION")
    )
}
