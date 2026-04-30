use crate::{
    bridge::{self, Bridge, Direction},
    config::Platform,
    event_bus::MjaiBus,
    logger::{BinaryLogger, Session},
};
use hudsucker::{
    Body, HttpContext, HttpHandler, RequestOrResponse, WebSocketContext, WebSocketHandler,
    futures::{Sink, SinkExt, Stream, StreamExt},
    hyper::{Request, Response, StatusCode, Uri},
    tokio_tungstenite::tungstenite::{self, Message},
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

const TAG_CLIENT_TO_SERVER: u8 = 0;
const TAG_SERVER_TO_CLIENT: u8 = 1;

/// Shared, per-WS-upgrade bridge. Both directions of the same WebSocket
/// connection (client→server and server→client) need the same `Bridge`
/// instance because Majsoul's request/response correlation lives in the
/// parser's `pending` map: the Request travels client→server and the
/// matching Response travels server→client.
type SharedBridge = Arc<StdMutex<Box<dyn Bridge>>>;

#[derive(Clone)]
pub struct ProxyHandler {
    session: Arc<Session>,
    binary: Arc<BinaryLogger>,
    platform: Platform,
    bridges: Arc<StdMutex<HashMap<SocketAddr, SharedBridge>>>,
    next_flow_id: Arc<AtomicU64>,
    /// Optional fan-out for parsed mjai events. `None` keeps the proxy
    /// usable in tests and in standalone "log only" mode.
    mjai_tx: Option<MjaiBus>,
    /// Triggered by `stop_capture` to kick all in-flight WS flows. Without
    /// this, hudsucker's `with_graceful_shutdown` only blocks new
    /// connections; existing ones would drain naturally and the game
    /// client would never see a disconnect.
    force_close: Arc<Notify>,
}

impl ProxyHandler {
    pub fn new(
        session: Arc<Session>,
        platform: Platform,
        mjai_tx: Option<MjaiBus>,
        force_close: Arc<Notify>,
    ) -> anyhow::Result<Self> {
        let binary = session.binary_logger("proxy")?;
        Ok(Self {
            session,
            binary,
            platform,
            bridges: Arc::new(StdMutex::new(HashMap::new())),
            next_flow_id: Arc::new(AtomicU64::new(1)),
            mjai_tx,
            force_close,
        })
    }

    fn acquire_bridge(&self, client: SocketAddr, uri: &Uri) -> SharedBridge {
        let mut map = self.bridges.lock().expect("bridges mutex poisoned");
        map.entry(client)
            .or_insert_with(|| {
                let flow_id = self.next_flow_id.fetch_add(1, Ordering::Relaxed);
                let path = uri_path_slug(uri);
                let file_name = format!("{flow_id:06}-{path}.log");
                let label = format!("{} {} {}", self.platform.subdir(), client, uri);
                let flow_log =
                    match self.session.flow_logger(self.platform.subdir(), &file_name, label) {
                        Ok(log) => Some(log),
                        Err(e) => {
                            warn!("failed to open flow log for {client}: {e:#}");
                            None
                        }
                    };
                Arc::new(StdMutex::new(bridge::for_platform(
                    self.platform,
                    flow_log,
                    Some(self.session.clone()),
                )))
            })
            .clone()
    }

    /// Drop our reference; if no other direction still holds the bridge,
    /// remove it from the map so per-connection state doesn't leak.
    fn release_bridge(&self, client: SocketAddr, bridge: SharedBridge) {
        drop(bridge);
        let mut map = self.bridges.lock().expect("bridges mutex poisoned");
        if let Some(existing) = map.get(&client) {
            // Only the map's own Arc remains → connection fully closed.
            if Arc::strong_count(existing) == 1 {
                map.remove(&client);
            }
        }
    }
}

impl HttpHandler for ProxyHandler {
    async fn handle_request(
        &mut self,
        _ctx: &HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        if req.uri().path() == "/ping" {
            return Response::builder()
                .status(StatusCode::OK)
                .body(Body::from("pong"))
                .expect("Failed to build ping response")
                .into();
        }
        req.into()
    }
}

impl WebSocketHandler for ProxyHandler {
    async fn handle_websocket(
        mut self,
        ctx: WebSocketContext,
        mut stream: impl Stream<Item = Result<Message, tungstenite::Error>> + Unpin + Send + 'static,
        mut sink: impl Sink<Message, Error = tungstenite::Error> + Unpin + Send + 'static,
    ) {
        let client = client_addr(&ctx);
        let server_uri = server_uri(&ctx);
        let bridge = self.acquire_bridge(client, &server_uri);
        let force_close = self.force_close.clone();

        loop {
            tokio::select! {
                biased;
                _ = force_close.notified() => {
                    info!("force-closing WS flow for {client}");
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
                next = stream.next() => {
                    let Some(message) = next else { break };
                    match message {
                        Ok(message) => {
                            let Some(out) = self.handle_message(&ctx, message, &bridge).await else {
                                continue;
                            };
                            match sink.send(out).await {
                                Ok(()) => (),
                                // Peer already gone — normal at end of game / lobby.
                                Err(tungstenite::Error::ConnectionClosed)
                                | Err(tungstenite::Error::AlreadyClosed) => break,
                                Err(e) => {
                                    error!("WebSocket send error: {e}");
                                    break;
                                }
                            }
                        }
                        Err(tungstenite::Error::ConnectionClosed)
                        | Err(tungstenite::Error::AlreadyClosed) => break,
                        Err(e) => {
                            error!("WebSocket recv error: {e}");
                            match sink.send(Message::Close(None)).await {
                                Ok(())
                                | Err(tungstenite::Error::ConnectionClosed)
                                | Err(tungstenite::Error::AlreadyClosed) => (),
                                Err(e) => error!("WebSocket close error: {e}"),
                            }
                            break;
                        }
                    }
                }
            }
        }

        self.release_bridge(client, bridge);
    }
}

impl ProxyHandler {
    async fn handle_message(
        &mut self,
        ctx: &WebSocketContext,
        msg: Message,
        bridge: &SharedBridge,
    ) -> Option<Message> {
        let (tag, dir, dir_arrow, uri) = match ctx {
            WebSocketContext::ServerToClient { src, .. } => {
                (TAG_SERVER_TO_CLIENT, Direction::Down, '\u{2193}', src.to_string())
            }
            WebSocketContext::ClientToServer { dst, .. } => {
                (TAG_CLIENT_TO_SERVER, Direction::Up, '\u{2191}', dst.to_string())
            }
        };

        match &msg {
            Message::Binary(buf) => {
                debug!("{dir_arrow} {uri} binary len={}", buf.len());
                self.binary.write(tag, buf);
                let events = {
                    let mut b = bridge.lock().expect("bridge mutex poisoned");
                    b.parse(dir, buf)
                };
                if !events.is_empty() {
                    debug!("{dir_arrow} {uri} bridge emitted {} event(s)", events.len());
                    if let Some(tx) = &self.mjai_tx {
                        for ev in events {
                            // No subscribers is fine — broadcast just drops.
                            let _ = tx.send(ev);
                        }
                    }
                }
            }
            Message::Text(t) => {
                debug!("{dir_arrow} {uri} text len={}", t.len());
            }
            Message::Close(_) => debug!("{dir_arrow} {uri} close"),
            _ => {}
        }

        if let Message::Frame(_) = &msg {
            warn!("unexpected raw frame at {uri}");
        }

        Some(msg)
    }
}

fn client_addr(ctx: &WebSocketContext) -> SocketAddr {
    match ctx {
        WebSocketContext::ClientToServer { src, .. } => *src,
        WebSocketContext::ServerToClient { dst, .. } => *dst,
    }
}

fn server_uri(ctx: &WebSocketContext) -> Uri {
    match ctx {
        WebSocketContext::ClientToServer { dst, .. } => dst.clone(),
        WebSocketContext::ServerToClient { src, .. } => src.clone(),
    }
}

/// Sanitize the URI path into a filename-safe slug. `/game-gateway` →
/// `game-gateway`, `/` → `root`, anything outside `[A-Za-z0-9_-]` becomes
/// `_`.
fn uri_path_slug(uri: &Uri) -> String {
    let raw = uri.path().trim_matches('/');
    if raw.is_empty() {
        return "root".into();
    }
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
