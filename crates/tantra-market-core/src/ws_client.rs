//! Shared WebSocket transport for market data feeds.
//!
//! Provides a reusable [`WsClient`] wrapping `tokio-tungstenite` with `rustls`,
//! automated heartbeat ping/pong with RTT tracking, dead-connection timeout
//! detection inside the I/O select loop, bounded exponential backoff reconnection
//! with jitter and max-attempt termination, dynamic [`WsHandshake`] auth refresh,
//! `on_connected` subscription replay/reconciliation hooks, and redacted URL/error
//! logging.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use rand::Rng;
use thiserror::Error;
use tokio::sync::{RwLock, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{WebSocketStream, connect_async_with_config};
use tracing::{info, warn};

/// Capacity of the internal bounded outgoing WebSocket frame channel.
const OUTGOING_CHANNEL_CAPACITY: usize = 1024;

/// Result type for WebSocket operations.
pub type WsResult<T> = Result<T, WsError>;

/// Async callback invoked after every successful WebSocket handshake.
///
/// The boolean parameter is `false` on the initial connection and `true` on any
/// subsequent reconnection, allowing feed adapters to replay active subscriptions
/// and trigger missed-tick REST snapshot reconciliation.
pub type OnConnectedHook = Arc<dyn Fn(bool) -> BoxFuture<'static, WsResult<()>> + Send + Sync>;

/// WebSocket transport errors.
#[derive(Debug, Error)]
pub enum WsError {
    /// WebSocket connection failed.
    #[error("websocket connection failed: {0}")]
    Connection(String),
    /// WebSocket send/recv failed.
    #[error("websocket transport error: {0}")]
    Transport(String),
    /// WebSocket message serialization failed.
    #[error("websocket message error: {0}")]
    Message(String),
    /// WebSocket connection was closed.
    #[error("websocket closed: {0}")]
    Closed(String),
}

/// Configuration for WebSocket connection behavior.
#[derive(Debug, Clone)]
pub struct WsConfig {
    /// Maximum reconnection delay in seconds.
    pub max_reconnect_delay_secs: u64,
    /// Initial reconnection delay in seconds.
    pub initial_reconnect_delay_secs: u64,
    /// Heartbeat ping interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Dead connection detection timeout in seconds (no incoming frame received).
    pub dead_connection_timeout_secs: u64,
    /// Maximum message size in bytes.
    pub max_message_size: usize,
    /// Maximum consecutive failed connect/reconnect attempts before entering
    /// [`WsConnectionState::Failed`]. `None` retries indefinitely.
    pub max_reconnect_attempts: Option<u32>,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            max_reconnect_delay_secs: 60,
            initial_reconnect_delay_secs: 1,
            heartbeat_interval_secs: 30,
            dead_connection_timeout_secs: 90,
            max_message_size: 10 * 1024 * 1024, // 10 MB
            max_reconnect_attempts: Some(10),
        }
    }
}

/// Target URL and upgrade headers produced by [`WsHandshake::prepare`].
#[derive(Clone, Default)]
pub struct WsConnectTarget {
    /// Full WebSocket URL (may contain broker query auth tokens).
    pub url: String,
    /// Optional HTTP headers added to the WebSocket upgrade request.
    pub headers: Vec<(String, String)>,
}

impl WsConnectTarget {
    /// Construct a WebSocket connection target for the given URL with no extra headers.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
        }
    }

    /// Append an HTTP header to the upgrade request.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

impl fmt::Debug for WsConnectTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(k, _)| (k.as_str(), "[REDACTED]"))
            .collect();
        f.debug_struct("WsConnectTarget")
            .field("url", &redact_url(&self.url))
            .field("headers", &redacted_headers)
            .finish()
    }
}

/// Async handshake port invoked on every (re)connection attempt so adapters
/// can refresh expiring access tokens and supply dynamic URLs or headers.
#[async_trait]
pub trait WsHandshake: Send + Sync {
    /// Prepare the WebSocket URL and headers for this connection attempt.
    async fn prepare(&self) -> WsResult<WsConnectTarget>;
}

/// Static URL [`WsHandshake`] implementation used by [`WsClient::new`] and [`WsClient::with_config`].
pub struct StaticHandshake {
    url: String,
}

impl StaticHandshake {
    /// Wrap a fixed WebSocket URL in a [`WsHandshake`].
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl fmt::Debug for StaticHandshake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticHandshake")
            .field("url", &redact_url(&self.url))
            .finish()
    }
}

#[async_trait]
impl WsHandshake for StaticHandshake {
    async fn prepare(&self) -> WsResult<WsConnectTarget> {
        Ok(WsConnectTarget::new(self.url.clone()))
    }
}

/// Strip query parameters, fragments, and `userinfo` credentials from a URL before logging.
pub fn redact_url(raw: &str) -> String {
    let without_qf = raw.split(&['?', '#'][..]).next().unwrap_or(raw);
    if let Some(scheme_end) = without_qf.find("://") {
        let after_scheme = &without_qf[scheme_end + 3..];
        let authority_end = after_scheme.find('/').unwrap_or(after_scheme.len());
        let authority = &after_scheme[..authority_end];
        if let Some(at_pos) = authority.rfind('@') {
            let prefix = &without_qf[..scheme_end + 3];
            let host_and_rest = &after_scheme[at_pos + 1..];
            return format!("{prefix}[REDACTED]@{host_and_rest}");
        }
    }
    without_qf.to_string()
}

/// Scrub full URLs, query-string parameter values, and explicit secrets from an error string.
pub fn sanitize_error_text(err_msg: &str, raw_url: &str, extra_secrets: &[&str]) -> String {
    let mut sanitized = err_msg.replace(raw_url, &redact_url(raw_url));
    if let Some(q_idx) = raw_url.find('?') {
        let query_and_frag = &raw_url[q_idx..];
        if !query_and_frag.is_empty() {
            sanitized = sanitized.replace(query_and_frag, "?[REDACTED]");
        }
        let query_only = query_and_frag
            .split('#')
            .next()
            .unwrap_or(query_and_frag)
            .trim_start_matches('?');
        for pair in query_only.split('&') {
            if let Some((_, val)) = pair.split_once('=')
                && val.len() >= 4
            {
                sanitized = sanitized.replace(val, "[REDACTED]");
            }
        }
        if !query_only.is_empty() {
            sanitized = sanitized.replace(query_only, "[REDACTED]");
        }
    }
    for secret in extra_secrets {
        if !secret.is_empty() {
            sanitized = sanitized.replace(secret, "[REDACTED]");
        }
    }
    sanitized
}

/// Compute bounded exponential backoff duration for a 1-based reconnect `attempt`.
///
/// Uses pure integer millisecond arithmetic with jitter (`jitter_bps` in `0..=10_000`
/// mapping linearly to `[0.50, 1.00]` of the exponential cap). Always returns at
/// least `1s` (`Duration::from_secs(1)`) and at most `max_delay_secs.max(1)`.
pub fn compute_backoff_duration(
    attempt: u32,
    initial_delay_secs: u64,
    max_delay_secs: u64,
    jitter_bps: u16,
) -> Duration {
    let init_ms = initial_delay_secs.max(1).saturating_mul(1000);
    let max_ms = max_delay_secs.max(1).saturating_mul(1000);
    let shift = attempt.saturating_sub(1).min(30);
    let factor = 1u64 << shift;
    let exp_ms = init_ms.saturating_mul(factor).min(max_ms);
    let scale_bps = 5000u64 + u64::from(jitter_bps.min(10_000) / 2);
    let jittered_ms = exp_ms.saturating_mul(scale_bps) / 10_000;
    Duration::from_millis(jittered_ms.clamp(1000, max_ms))
}

/// Lock-free live health snapshot tracked by [`WsClient`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WsHealthSnapshot {
    /// Whether the WebSocket is currently connected.
    pub connected: bool,
    /// Timestamp of the last received heartbeat or initial connection.
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// Measured ping/pong round-trip latency in milliseconds, if observed.
    pub rtt_ms: Option<u64>,
    /// Number of incoming messages dropped due to bounded downstream saturation.
    pub dropped_count: u64,
    /// Total reconnection attempts started since client creation/connect.
    pub reconnect_count: u64,
}

struct WsHealthCounters {
    connected: AtomicBool,
    last_heartbeat_ms: AtomicI64,
    rtt_ms: AtomicI64,
    dropped_count: AtomicU64,
    reconnect_count: AtomicU64,
}

impl Default for WsHealthCounters {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            last_heartbeat_ms: AtomicI64::new(-1),
            rtt_ms: AtomicI64::new(-1),
            dropped_count: AtomicU64::new(0),
            reconnect_count: AtomicU64::new(0),
        }
    }
}

impl WsHealthCounters {
    fn snapshot(&self) -> WsHealthSnapshot {
        let hb_ms = self.last_heartbeat_ms.load(Ordering::Acquire);
        let rtt = self.rtt_ms.load(Ordering::Acquire);
        WsHealthSnapshot {
            connected: self.connected.load(Ordering::Acquire),
            last_heartbeat: (hb_ms >= 0)
                .then(|| DateTime::from_timestamp_millis(hb_ms))
                .flatten(),
            rtt_ms: (rtt >= 0).then_some(rtt as u64),
            dropped_count: self.dropped_count.load(Ordering::Acquire),
            reconnect_count: self.reconnect_count.load(Ordering::Acquire),
        }
    }

    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }

    fn record_heartbeat(&self, now: DateTime<Utc>, rtt_ms: Option<u64>) {
        self.last_heartbeat_ms
            .store(now.timestamp_millis(), Ordering::Release);
        if let Some(rtt) = rtt_ms {
            let clamped = i64::try_from(rtt).unwrap_or(i64::MAX);
            self.rtt_ms.store(clamped, Ordering::Release);
        }
    }

    fn inc_dropped(&self) {
        self.dropped_count.fetch_add(1, Ordering::Relaxed);
    }

    fn inc_reconnect(&self) {
        self.reconnect_count.fetch_add(1, Ordering::Relaxed);
    }

    fn reset(&self) {
        self.connected.store(false, Ordering::Release);
        self.last_heartbeat_ms.store(-1, Ordering::Release);
        self.rtt_ms.store(-1, Ordering::Release);
        self.dropped_count.store(0, Ordering::Release);
        self.reconnect_count.store(0, Ordering::Release);
    }
}

/// Incoming WebSocket message.
#[derive(Debug, Clone)]
pub struct WsMessage {
    /// Raw binary payload.
    pub payload: Vec<u8>,
    /// Whether the message is binary (vs text).
    pub is_binary: bool,
    /// Timestamp when the message was received.
    pub received_at: DateTime<Utc>,
}

/// Outgoing WebSocket message.
#[derive(Debug, Clone)]
pub struct WsOutMessage {
    /// Raw binary payload.
    pub payload: Vec<u8>,
    /// Whether the message is binary (vs text).
    pub is_binary: bool,
}

/// Connection state for WebSocket transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsConnectionState {
    /// Not yet connected.
    Disconnected,
    /// Connecting in progress.
    Connecting,
    /// Connected and ready.
    Connected,
    /// Connection lost, attempting reconnect.
    Reconnecting { attempt: u32 },
    /// Connection permanently failed after exhausting retries.
    Failed,
}

/// Sink abstraction supporting both existing [`WsClient::connect`] unbounded
/// callers and bounded [`WsClient::connect_bounded`] adapters.
#[derive(Clone)]
enum WsMessageSink {
    Unbounded(mpsc::UnboundedSender<WsMessage>),
    Bounded(mpsc::Sender<WsMessage>),
}

impl WsMessageSink {
    fn send_incoming(&self, msg: WsMessage, health: &WsHealthCounters) {
        match self {
            Self::Unbounded(tx) => {
                if tx.send(msg).is_err() {
                    health.inc_dropped();
                }
            }
            Self::Bounded(tx) => {
                if tx.try_send(msg).is_err() {
                    health.inc_dropped();
                }
            }
        }
    }
}

/// Shared WebSocket client with dynamic handshake, heartbeat RTT, dead-connection
/// timeout, jittered backoff, and `on_connected` subscription replay hook.
pub struct WsClient {
    config: WsConfig,
    handshake: Arc<dyn WsHandshake>,
    state: Arc<RwLock<WsConnectionState>>,
    state_watch_tx: watch::Sender<WsConnectionState>,
    tx: Arc<tokio::sync::Mutex<Option<mpsc::Sender<WsOutMessage>>>>,
    msg_sink: Arc<tokio::sync::Mutex<Option<WsMessageSink>>>,
    last_message: Arc<RwLock<Option<Instant>>>,
    on_connected: Arc<std::sync::Mutex<Option<OnConnectedHook>>>,
    health: Arc<WsHealthCounters>,
    reconnect_task: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl Drop for WsClient {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.reconnect_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }
}

impl WsClient {
    /// Create a new WebSocket client for the given static URL.
    pub fn new(ws_url: impl Into<String>) -> Self {
        Self::with_config(ws_url, WsConfig::default())
    }

    /// Create a new WebSocket client with a static URL and custom configuration.
    pub fn with_config(ws_url: impl Into<String>, config: WsConfig) -> Self {
        Self::from_handshake(Arc::new(StaticHandshake::new(ws_url)), config)
    }

    /// Create a new WebSocket client with a dynamic [`WsHandshake`] provider.
    pub fn from_handshake(handshake: Arc<dyn WsHandshake>, config: WsConfig) -> Self {
        let (state_watch_tx, _) = watch::channel(WsConnectionState::Disconnected);
        Self {
            config,
            handshake,
            state: Arc::new(RwLock::new(WsConnectionState::Disconnected)),
            state_watch_tx,
            tx: Arc::new(tokio::sync::Mutex::new(None)),
            msg_sink: Arc::new(tokio::sync::Mutex::new(None)),
            last_message: Arc::new(RwLock::new(None)),
            on_connected: Arc::new(std::sync::Mutex::new(None)),
            health: Arc::new(WsHealthCounters::default()),
            reconnect_task: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Register a callback executed after every successful connection or reconnection.
    pub fn set_on_connected(&self, hook: OnConnectedHook) {
        if let Ok(mut guard) = self.on_connected.lock() {
            *guard = Some(hook);
        }
    }

    /// Subscribe to connection state transitions via a `tokio::sync::watch` receiver.
    pub fn subscribe_state(&self) -> watch::Receiver<WsConnectionState> {
        self.state_watch_tx.subscribe()
    }

    /// Lock-free snapshot of connection health, heartbeat timestamp, RTT, and counters.
    pub fn health_snapshot(&self) -> WsHealthSnapshot {
        self.health.snapshot()
    }

    /// Get the current connection state.
    pub async fn state(&self) -> WsConnectionState {
        self.state.read().await.clone()
    }

    /// Check if the client is connected.
    pub async fn is_connected(&self) -> bool {
        matches!(*self.state.read().await, WsConnectionState::Connected)
    }

    /// Send a message over the WebSocket connection.
    pub async fn send(&self, msg: WsOutMessage) -> WsResult<()> {
        let sender = {
            let tx = self.tx.lock().await;
            tx.clone()
        };
        if let Some(sender) = sender {
            sender
                .send(msg)
                .await
                .map_err(|e| WsError::Transport(e.to_string()))
        } else {
            Err(WsError::Transport("not connected".into()))
        }
    }

    /// Send a text message.
    pub async fn send_text(&self, text: impl Into<String>) -> WsResult<()> {
        self.send(WsOutMessage {
            payload: text.into().into_bytes(),
            is_binary: false,
        })
        .await
    }

    /// Send a binary message.
    pub async fn send_binary(&self, payload: Vec<u8>) -> WsResult<()> {
        self.send(WsOutMessage {
            payload,
            is_binary: true,
        })
        .await
    }

    /// Wait until the initial connection attempt becomes `Connected`, or return an
    /// error if it fails or starts reconnecting before establishing a session.
    pub async fn wait_initial_connection(&self) -> WsResult<()> {
        let mut rx = self.subscribe_state();
        loop {
            match rx.borrow().clone() {
                WsConnectionState::Connected => return Ok(()),
                WsConnectionState::Reconnecting { attempt } => {
                    return Err(WsError::Connection(format!(
                        "initial connection failed (reconnecting attempt {attempt})"
                    )));
                }
                WsConnectionState::Failed => {
                    return Err(WsError::Connection("connection failed".into()));
                }
                WsConnectionState::Disconnected | WsConnectionState::Connecting => {}
            }
            if rx.changed().await.is_err() {
                return Err(WsError::Closed("connection state channel closed".into()));
            }
        }
    }

    /// Connect to the WebSocket server using an unbounded incoming message channel.
    ///
    /// Spawns a background task handling connection, heartbeat, dead-connection detection,
    /// dynamic handshake auth refresh, `on_connected` hooks, and exponential backoff
    /// reconnection. Returns immediately; use [`Self::state`] or [`Self::subscribe_state`]
    /// to observe state changes.
    pub async fn connect(&self, msg_tx: mpsc::UnboundedSender<WsMessage>) -> WsResult<()> {
        self.start_loop(WsMessageSink::Unbounded(msg_tx)).await
    }

    /// Connect to the WebSocket server using a bounded incoming message channel.
    ///
    /// When the downstream receiver falls behind and the channel is full, incoming
    /// frames increment [`WsHealthSnapshot::dropped_count`] rather than stalling the
    /// WebSocket I/O loop.
    pub async fn connect_bounded(&self, msg_tx: mpsc::Sender<WsMessage>) -> WsResult<()> {
        self.start_loop(WsMessageSink::Bounded(msg_tx)).await
    }

    async fn start_loop(&self, sink: WsMessageSink) -> WsResult<()> {
        self.abort_reconnect_task();
        self.health.reset();

        {
            let mut ms = self.msg_sink.lock().await;
            *ms = Some(sink);
        }

        let handshake = self.handshake.clone();
        let config = self.config.clone();
        let state = self.state.clone();
        let state_watch_tx = self.state_watch_tx.clone();
        let tx = self.tx.clone();
        let msg_sink = self.msg_sink.clone();
        let last_message = self.last_message.clone();
        let on_connected = self.on_connected.clone();
        let health = self.health.clone();

        let task = tokio::spawn(async move {
            let mut consecutive_failures: u32 = 0;
            let mut has_connected_once = false;

            loop {
                set_state(&state, &state_watch_tx, WsConnectionState::Connecting).await;

                let prepared = handshake.prepare().await;
                let connect_attempt = match prepared {
                    Ok(target) => {
                        let safe_url = redact_url(&target.url);
                        let secrets: Vec<&str> =
                            target.headers.iter().map(|(_, v)| v.as_str()).collect();
                        match build_client_request(&target) {
                            Ok(req) => {
                                let ws_cfg = build_ws_protocol_config(&config);
                                connect_async_with_config(req, Some(ws_cfg), false)
                                    .await
                                    .map(|(stream, _)| (stream, safe_url))
                                    .map_err(|e| {
                                        sanitize_error_text(&e.to_string(), &target.url, &secrets)
                                    })
                            }
                            Err(e) => {
                                Err(sanitize_error_text(&e.to_string(), &target.url, &secrets))
                            }
                        }
                    }
                    Err(e) => Err(e.to_string()),
                };

                match connect_attempt {
                    Ok((ws_stream, safe_url)) => {
                        info!("WebSocket connected: {}", safe_url);
                        let is_reconnect = has_connected_once;
                        has_connected_once = true;
                        consecutive_failures = 0;

                        let current_sink = {
                            let ms = msg_sink.lock().await;
                            ms.clone()
                        };
                        let Some(sink) = current_sink else {
                            warn!("No message receiver set, closing connection");
                            break;
                        };

                        let (out_tx, out_rx) =
                            mpsc::channel::<WsOutMessage>(OUTGOING_CHANNEL_CAPACITY);
                        {
                            let mut t = tx.lock().await;
                            *t = Some(out_tx);
                        }
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(Instant::now());
                        }
                        health.set_connected(true);
                        health.record_heartbeat(Utc::now(), None);
                        set_state(&state, &state_watch_tx, WsConnectionState::Connected).await;

                        let hook = on_connected.lock().ok().and_then(|g| g.clone());
                        let hook_ok = if let Some(cb) = hook {
                            if let Err(e) = cb(is_reconnect).await {
                                warn!("WebSocket on_connected hook failed: {}", e);
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        };

                        if hook_ok
                            && let Err(e) = run_ws_loop(
                                ws_stream,
                                out_rx,
                                config.clone(),
                                sink,
                                last_message.clone(),
                                health.clone(),
                            )
                            .await
                        {
                            warn!("WebSocket loop error: {}", e);
                        }

                        {
                            let mut t = tx.lock().await;
                            *t = None;
                        }
                        health.set_connected(false);
                    }
                    Err(safe_err) => {
                        warn!("WebSocket connect failed: {}", safe_err);
                        health.set_connected(false);
                    }
                }

                consecutive_failures = consecutive_failures.saturating_add(1);
                if let Some(max) = config.max_reconnect_attempts
                    && consecutive_failures > max
                {
                    warn!(
                        "WebSocket exhausted max_reconnect_attempts ({}); entering Failed state",
                        max
                    );
                    health.set_connected(false);
                    set_state(&state, &state_watch_tx, WsConnectionState::Failed).await;
                    break;
                }

                health.inc_reconnect();
                set_state(
                    &state,
                    &state_watch_tx,
                    WsConnectionState::Reconnecting {
                        attempt: consecutive_failures,
                    },
                )
                .await;

                let jitter_bps = rand::thread_rng().gen_range(0..=10_000);
                let delay = compute_backoff_duration(
                    consecutive_failures,
                    config.initial_reconnect_delay_secs,
                    config.max_reconnect_delay_secs,
                    jitter_bps,
                );
                tokio::time::sleep(delay).await;
            }
        });

        if let Ok(mut guard) = self.reconnect_task.lock() {
            *guard = Some(task);
        }
        Ok(())
    }

    fn abort_reconnect_task(&self) {
        if let Ok(mut guard) = self.reconnect_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
    }

    /// Disconnect and stop the reconnect task.
    pub async fn disconnect(&self) {
        self.abort_reconnect_task();
        self.health.set_connected(false);
        set_state(
            &self.state,
            &self.state_watch_tx,
            WsConnectionState::Disconnected,
        )
        .await;
        {
            let mut tx = self.tx.lock().await;
            *tx = None;
        }
    }

    /// Get the elapsed time since the last incoming frame was received.
    pub async fn last_message_elapsed(&self) -> Option<Duration> {
        self.last_message.read().await.map(|lm| lm.elapsed())
    }

    /// Check if the connection appears dead (no incoming frames within timeout).
    pub async fn is_dead(&self) -> bool {
        match self.last_message_elapsed().await {
            Some(elapsed) => {
                elapsed > Duration::from_secs(self.config.dead_connection_timeout_secs)
            }
            None => !self.is_connected().await,
        }
    }
}

async fn set_state(
    state: &Arc<RwLock<WsConnectionState>>,
    watch_tx: &watch::Sender<WsConnectionState>,
    next: WsConnectionState,
) {
    {
        let mut s = state.write().await;
        *s = next.clone();
    }
    let _ = watch_tx.send(next);
}

fn normalize_ws_url_for_http(raw: &str) -> String {
    if let Some(scheme_end) = raw.find("://") {
        let after_scheme = &raw[scheme_end + 3..];
        if let Some(pos) = after_scheme.find(&['/', '?', '#'][..])
            && after_scheme.as_bytes()[pos] == b'?'
        {
            let split_at = scheme_end + 3 + pos;
            return format!("{}/{}", &raw[..split_at], &raw[split_at..]);
        }
    }
    raw.to_string()
}

fn build_client_request(
    target: &WsConnectTarget,
) -> WsResult<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let normalized = normalize_ws_url_for_http(&target.url);
    let mut req = normalized
        .into_client_request()
        .map_err(|e| WsError::Connection(format!("invalid websocket request: {e}")))?;
    for (name, value) in &target.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| WsError::Connection(format!("invalid header name '{name}': {e}")))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_| WsError::Connection(format!("invalid header value for '{name}'")))?;
        req.headers_mut().insert(header_name, header_value);
    }
    Ok(req)
}

fn build_ws_protocol_config(config: &WsConfig) -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(config.max_message_size),
        max_frame_size: Some(config.max_message_size),
        ..Default::default()
    }
}

/// Run the WebSocket message loop with heartbeat ping/pong RTT tracking and
/// active dead-connection timeout enforcement.
async fn run_ws_loop(
    ws_stream: WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    mut out_rx: mpsc::Receiver<WsOutMessage>,
    config: WsConfig,
    msg_sink: WsMessageSink,
    last_message: Arc<RwLock<Option<Instant>>>,
    health: Arc<WsHealthCounters>,
) -> WsResult<()> {
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let hb_dur = Duration::from_secs(config.heartbeat_interval_secs.max(1));
    let mut heartbeat_interval = tokio::time::interval_at(Instant::now() + hb_dur, hb_dur);
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let dead_dur = Duration::from_secs(config.dead_connection_timeout_secs.max(1));
    let dead_sleep = tokio::time::sleep(dead_dur);
    tokio::pin!(dead_sleep);

    let mut last_ping_sent: Option<Instant> = None;

    loop {
        tokio::select! {
            // Active dead-connection deadline (only reset on incoming server frames)
            _ = &mut dead_sleep => {
                return Err(WsError::Transport(format!(
                    "dead connection timeout after {}s",
                    config.dead_connection_timeout_secs
                )));
            }
            // Incoming WebSocket message
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let now = Instant::now();
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(now);
                        }
                        dead_sleep.as_mut().reset(now + dead_dur);
                        msg_sink.send_incoming(
                            WsMessage {
                                payload: data,
                                is_binary: true,
                                received_at: Utc::now(),
                            },
                            &health,
                        );
                    }
                    Some(Ok(Message::Text(text))) => {
                        let now = Instant::now();
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(now);
                        }
                        dead_sleep.as_mut().reset(now + dead_dur);
                        msg_sink.send_incoming(
                            WsMessage {
                                payload: text.into_bytes(),
                                is_binary: false,
                                received_at: Utc::now(),
                            },
                            &health,
                        );
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let now = Instant::now();
                        let _ = ws_tx.send(Message::Pong(data)).await;
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(now);
                        }
                        dead_sleep.as_mut().reset(now + dead_dur);
                        health.record_heartbeat(Utc::now(), None);
                    }
                    Some(Ok(Message::Pong(_))) => {
                        let now = Instant::now();
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(now);
                        }
                        dead_sleep.as_mut().reset(now + dead_dur);
                        let rtt = last_ping_sent.take().map(|sent| {
                            u64::try_from(now.saturating_duration_since(sent).as_millis())
                                .unwrap_or(u64::MAX)
                        });
                        health.record_heartbeat(Utc::now(), rtt);
                    }
                    Some(Ok(Message::Frame(_))) => {
                        let now = Instant::now();
                        {
                            let mut lm = last_message.write().await;
                            *lm = Some(now);
                        }
                        dead_sleep.as_mut().reset(now + dead_dur);
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("WebSocket close received");
                        return Err(WsError::Closed("server initiated close".into()));
                    }
                    Some(Err(e)) => {
                        return Err(WsError::Transport(e.to_string()));
                    }
                    None => {
                        return Err(WsError::Closed("stream ended".into()));
                    }
                }
            }
            // Outgoing message
            Some(msg) = out_rx.recv() => {
                let ws_msg = if msg.is_binary {
                    Message::Binary(msg.payload)
                } else {
                    Message::Text(String::from_utf8_lossy(&msg.payload).to_string())
                };
                if let Err(e) = ws_tx.send(ws_msg).await {
                    return Err(WsError::Transport(e.to_string()));
                }
            }
            // Heartbeat ping
            _ = heartbeat_interval.tick() => {
                last_ping_sent = Some(Instant::now());
                if let Err(e) = ws_tx.send(Message::Ping(vec![])).await {
                    return Err(WsError::Transport(e.to_string()));
                }
            }
        }
    }
}
