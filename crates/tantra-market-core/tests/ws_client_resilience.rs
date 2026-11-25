//! TDD tests for [`WsClient`] resilience, URL redaction, backoff bounds,
//! dead-connection detection, dynamic handshakes, and `on_connected` hooks
//! (closed `tantra-market` gap `0010`; `docs/gaps/README.md#closed-gaps`).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use proptest::prelude::*;
use tantra_market_core::{
    WsClient, WsConfig, WsConnectTarget, WsConnectionState, WsHandshake, WsResult,
    compute_backoff_duration, redact_url, sanitize_error_text,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// Process-wide in-memory tracing subscriber for asserting zero credential leakage in logs.
#[derive(Clone, Default)]
struct LogCapture {
    entries: Arc<Mutex<Vec<String>>>,
}

impl LogCapture {
    fn snapshot(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }
}

fn global_log_capture() -> &'static LogCapture {
    static CAPTURE: OnceLock<LogCapture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let cap = LogCapture::default();
        let _ = tracing::subscriber::set_global_default(cap.clone());
        cap
    })
}

struct FieldVisitor<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

impl Subscriber for LogCapture {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldVisitor(&mut fields));
        let line = fields
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        self.entries.lock().unwrap().push(line);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

proptest! {
    #[test]
    fn backoff_always_at_least_one_second_and_never_exceeds_max(
        attempt in 0u32..=32,
        initial_secs in 0u64..=30,
        max_secs in 0u64..=300,
        jitter_bps in 0u16..=10_000,
    ) {
        let dur = compute_backoff_duration(attempt, initial_secs, max_secs, jitter_bps);
        prop_assert!(
            dur >= Duration::from_secs(1),
            "backoff dropped below 1s: {:?} (attempt={}, init={}, max={}, jitter={})",
            dur, attempt, initial_secs, max_secs, jitter_bps
        );
        prop_assert!(
            dur <= Duration::from_secs(max_secs.max(1)),
            "backoff exceeded max: {:?} > {}s",
            dur, max_secs.max(1)
        );
    }
}

#[test]
fn url_and_debug_and_error_redaction_strips_tokens() {
    let dhan_url =
        "wss://api-feed.dhan.co/v2?version=2&token=DHAN_SUPER_SECRET_123&clientId=C99#frag";
    assert_eq!(redact_url(dhan_url), "wss://api-feed.dhan.co/v2");

    let userinfo_url = "wss://myuser:mysecretpass@ws.example.com:8443/stream?api_key=KITE_SECRET";
    assert_eq!(
        redact_url(userinfo_url),
        "wss://[REDACTED]@ws.example.com:8443/stream"
    );

    let target = WsConnectTarget::new(dhan_url).with_header("access-token", "HEADER_SECRET_ABC");
    let debug_out = format!("{target:?}");
    assert!(!debug_out.contains("DHAN_SUPER_SECRET_123"));
    assert!(!debug_out.contains("HEADER_SECRET_ABC"));
    assert!(debug_out.contains("wss://api-feed.dhan.co/v2"));
    assert!(debug_out.contains("[REDACTED]"));

    let raw_err = format!(
        "handshake rejected for {dhan_url} with token HEADER_SECRET_ABC and query token=DHAN_SUPER_SECRET_123"
    );
    let sanitized = sanitize_error_text(&raw_err, dhan_url, &["HEADER_SECRET_ABC"]);
    assert!(!sanitized.contains("DHAN_SUPER_SECRET_123"));
    assert!(!sanitized.contains("HEADER_SECRET_ABC"));
}

#[tokio::test]
async fn ws_client_never_logs_query_token_on_connect_or_failure() {
    let capture = global_log_capture();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = ws.send(Message::Text("hello".into())).await;
            let _ = ws.close(None).await;
        }
    });

    // Reserve an unbound port to force a deterministic connect error.
    let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed_listener.local_addr().unwrap();
    drop(closed_listener);

    let secret_1 = "LIVE_DHAN_TOKEN_987654321";
    let secret_2 = "LIVE_KITE_TOKEN_112233445";

    let client = WsClient::with_config(
        format!("ws://{addr}/feed?version=2&token={secret_1}&clientId=C1"),
        WsConfig {
            max_reconnect_attempts: Some(0),
            ..Default::default()
        },
    );
    let mut state_rx = client.subscribe_state();
    let (tx, mut rx) = mpsc::unbounded_channel();
    client.connect(tx).await.unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("expected initial ws text")
        .expect("channel open");
    assert_eq!(msg.payload, b"hello");

    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while !matches!(*state_rx.borrow(), WsConnectionState::Failed) {
            if state_rx.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    client.disconnect().await;
    let _ = server_task.await;

    let fail_client = WsClient::with_config(
        format!("ws://{closed_addr}/ticker?api_key=MYKEY&access_token={secret_2}"),
        WsConfig {
            max_reconnect_attempts: Some(0),
            ..Default::default()
        },
    );
    let mut fail_rx = fail_client.subscribe_state();
    let (tx2, _rx2) = mpsc::unbounded_channel();
    fail_client.connect(tx2).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !matches!(*fail_rx.borrow(), WsConnectionState::Failed) {
            fail_rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    fail_client.disconnect().await;

    let logs = capture.snapshot().join("\n");
    assert!(
        logs.contains(&format!("ws://{addr}/feed")),
        "expected redacted connect log, got:\n{logs}"
    );
    assert!(
        !logs.contains(secret_1),
        "token secret_1 leaked into logs:\n{logs}"
    );
    assert!(
        !logs.contains(secret_2),
        "token secret_2 leaked into logs:\n{logs}"
    );
    assert!(
        !logs.contains("?version=") && !logs.contains("?api_key="),
        "query string leaked into logs:\n{logs}"
    );
}

#[tokio::test(start_paused = true)]
async fn half_open_dead_connection_timeout_triggers_reconnect_and_reaches_failed() {
    let _capture = global_log_capture();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Accept one silent WebSocket connection (never responds to Ping, never sends Close),
    // then drop the listener so reconnect attempts 1 and 2 fail.
    let silence_hold = Arc::new(AtomicBool::new(true));
    let silence_hold_srv = silence_hold.clone();
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        drop(listener);
        while silence_hold_srv.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let config = WsConfig {
        initial_reconnect_delay_secs: 1,
        max_reconnect_delay_secs: 2,
        heartbeat_interval_secs: 5,
        dead_connection_timeout_secs: 15,
        max_message_size: 1024 * 1024,
        max_reconnect_attempts: Some(2),
    };
    let client = WsClient::with_config(format!("ws://{addr}/ws"), config);
    let mut state_rx = client.subscribe_state();
    let (tx, _rx) = mpsc::unbounded_channel();
    client.connect(tx).await.unwrap();

    // Let real TCP loopback accept+handshake finish while tokio clock is paused.
    for _ in 0..40 {
        tokio::task::yield_now().await;
        if matches!(*state_rx.borrow(), WsConnectionState::Connected) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(client.state().await, WsConnectionState::Connected);
    assert!(client.health_snapshot().connected);

    let mut observed = Vec::new();
    observed.push(state_rx.borrow().clone());

    // Advance past dead_connection_timeout_secs (15s) -> should enter Reconnecting { attempt: 1 }.
    tokio::time::advance(Duration::from_secs(16)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }

    // Step through reconnect attempts until Failed.
    for _ in 0..15 {
        while state_rx.has_changed().unwrap_or(false) {
            observed.push(state_rx.borrow_and_update().clone());
        }
        if matches!(*state_rx.borrow(), WsConnectionState::Failed) {
            break;
        }
        tokio::time::advance(Duration::from_secs(2)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    while state_rx.has_changed().unwrap_or(false) {
        observed.push(state_rx.borrow_and_update().clone());
    }

    silence_hold.store(false, Ordering::SeqCst);
    let _ = server_task.await;

    assert!(
        observed.contains(&WsConnectionState::Reconnecting { attempt: 1 }),
        "expected Reconnecting(1), observed: {observed:?}"
    );
    assert!(
        observed.contains(&WsConnectionState::Reconnecting { attempt: 2 }),
        "expected Reconnecting(2), observed: {observed:?}"
    );
    assert_eq!(client.state().await, WsConnectionState::Failed);
    let health = client.health_snapshot();
    assert!(!health.connected);
    assert!(health.reconnect_count >= 2);
}

#[tokio::test(start_paused = true)]
async fn pong_resets_dead_connection_deadline_and_populates_heartbeat_health() {
    let _capture = global_log_capture();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(frame)) = ws.next().await {
            if let Message::Ping(payload) = frame
                && ws.send(Message::Pong(payload)).await.is_err()
            {
                break;
            }
        }
    });

    let config = WsConfig {
        initial_reconnect_delay_secs: 1,
        max_reconnect_delay_secs: 4,
        heartbeat_interval_secs: 5,
        dead_connection_timeout_secs: 15,
        max_message_size: 1024 * 1024,
        max_reconnect_attempts: Some(2),
    };
    let client = WsClient::with_config(format!("ws://{addr}/ws"), config);
    let state_rx = client.subscribe_state();
    let (tx, _rx) = mpsc::unbounded_channel();
    client.connect(tx).await.unwrap();

    for _ in 0..40 {
        tokio::task::yield_now().await;
        if matches!(*state_rx.borrow(), WsConnectionState::Connected) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(client.state().await, WsConnectionState::Connected);

    // Advance 30s in 5s steps; Ping/Pong keeps connection healthy beyond the 15s deadline.
    for _ in 0..6 {
        tokio::time::advance(Duration::from_secs(5)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    assert_eq!(client.state().await, WsConnectionState::Connected);
    let health = client.health_snapshot();
    assert!(health.connected);
    assert!(health.last_heartbeat.is_some());
    assert!(health.rtt_ms.is_some());
    assert_eq!(health.reconnect_count, 0);

    client.disconnect().await;
    server_task.abort();
}

struct RotatingHandshake {
    base_url: String,
    counter: AtomicU32,
}

#[async_trait]
impl WsHandshake for RotatingHandshake {
    async fn prepare(&self) -> WsResult<WsConnectTarget> {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(
            WsConnectTarget::new(format!("{}?token=tok_v{seq}", self.base_url))
                .with_header("x-session-seq", format!("hdr_{seq}")),
        )
    }
}

#[tokio::test]
#[allow(clippy::result_large_err)]
async fn dynamic_handshake_rotates_token_and_fires_on_connected_hook_on_reconnect() {
    let _capture = global_log_capture();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let captured_requests = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
    let captured_srv = captured_requests.clone();
    let server_received_resub = Arc::new(Mutex::new(Vec::<String>::new()));
    let resub_srv = server_received_resub.clone();

    let server_task = tokio::spawn(async move {
        // Accept connection #1, check headers/uri, send 1 message, close immediately.
        let (stream1, _) = listener.accept().await.unwrap();
        let cap1 = captured_srv.clone();
        let mut ws1 = accept_hdr_async(stream1, move |req: &Request, resp: Response| {
            let hdr = req
                .headers()
                .get("x-session-seq")
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);
            cap1.lock().unwrap().push((req.uri().to_string(), hdr));
            Ok(resp)
        })
        .await
        .unwrap();
        let _ = ws1.send(Message::Text("msg1".into())).await;
        let _ = ws1.close(None).await;

        // Accept connection #2, expect rotated token + RESUBSCRIBE frame from hook.
        let (stream2, _) = listener.accept().await.unwrap();
        let cap2 = captured_srv.clone();
        let mut ws2 = accept_hdr_async(stream2, move |req: &Request, resp: Response| {
            let hdr = req
                .headers()
                .get("x-session-seq")
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);
            cap2.lock().unwrap().push((req.uri().to_string(), hdr));
            Ok(resp)
        })
        .await
        .unwrap();
        if let Some(Ok(Message::Text(txt))) = ws2.next().await {
            resub_srv.lock().unwrap().push(txt);
        }
        let _ = ws2.send(Message::Text("msg2".into())).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let handshake = Arc::new(RotatingHandshake {
        base_url: format!("ws://{addr}/stream"),
        counter: AtomicU32::new(0),
    });
    let client = Arc::new(WsClient::from_handshake(
        handshake,
        WsConfig {
            initial_reconnect_delay_secs: 1,
            max_reconnect_delay_secs: 1,
            max_reconnect_attempts: Some(3),
            ..Default::default()
        },
    ));

    let hook_flags = Arc::new(Mutex::new(Vec::<bool>::new()));
    let hook_flags_cb = hook_flags.clone();
    let weak_client = Arc::downgrade(&client);
    client.set_on_connected(Arc::new(move |is_reconnect| {
        let hook_flags_cb = hook_flags_cb.clone();
        let weak_client = weak_client.clone();
        Box::pin(async move {
            hook_flags_cb.lock().unwrap().push(is_reconnect);
            if is_reconnect && let Some(c) = weak_client.upgrade() {
                c.send_text("RESUBSCRIBE").await?;
            }
            Ok(())
        })
    }));

    let (tx, mut rx) = mpsc::unbounded_channel();
    client.connect(tx).await.unwrap();

    let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.payload, b"msg1");

    let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.payload, b"msg2");

    let reqs = captured_requests.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].0, "/stream?token=tok_v1");
    assert_eq!(reqs[0].1.as_deref(), Some("hdr_1"));
    assert_eq!(reqs[1].0, "/stream?token=tok_v2");
    assert_eq!(reqs[1].1.as_deref(), Some("hdr_2"));

    assert_eq!(*hook_flags.lock().unwrap(), vec![false, true]);
    assert_eq!(
        *server_received_resub.lock().unwrap(),
        vec!["RESUBSCRIBE".to_string()]
    );
    assert!(client.health_snapshot().reconnect_count >= 1);

    client.disconnect().await;
    server_task.abort();
}
