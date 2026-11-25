//! End-to-end reconnect, token rotation, subscription replay, REST snapshot gap-fill,
//! and live health metrics tests for [`DhanFeed`] (`tantra-market/docs/gaps/0010`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Tick};
use tantra_market_core::{MarketDataFeed, MdError, Mode, Subscription, TokenProvider, WsConfig};
use tantra_market_dhan::DhanFeed;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::Message;

struct DynamicToken {
    current: Mutex<String>,
}

#[async_trait]
impl TokenProvider for DynamicToken {
    async fn access_token(&self) -> Result<String, MdError> {
        Ok(self.current.lock().unwrap().clone())
    }
}

fn dhan_ticker_packet(security_id: u32, ltp: f32, ltt: i32) -> Vec<u8> {
    let mut bytes = vec![2u8]; // Ticker response code
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.push(1u8); // NSE_EQ segment code
    bytes.extend_from_slice(&security_id.to_le_bytes());
    bytes.extend_from_slice(&ltp.to_le_bytes());
    bytes.extend_from_slice(&ltt.to_le_bytes());
    bytes
}

#[tokio::test]
#[allow(clippy::result_large_err)]
async fn dhan_feed_rotates_token_resubscribes_and_fills_snapshot_gap_on_reconnect() {
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_addr = ws_listener.local_addr().unwrap();

    let rest_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_addr = rest_listener.local_addr().unwrap();

    let uris = Arc::new(Mutex::new(Vec::<String>::new()));
    let uris_srv = uris.clone();
    let sub_msgs = Arc::new(Mutex::new(Vec::<String>::new()));
    let sub_srv = sub_msgs.clone();
    let rest_headers = Arc::new(Mutex::new(Vec::<String>::new()));
    let rest_hdr_srv = rest_headers.clone();

    let token_provider = Arc::new(DynamicToken {
        current: Mutex::new("DHAN_TOK_V1".into()),
    });
    let provider_for_ws = token_provider.clone();

    // Minimal HTTP/1.1 REST snapshot server.
    let rest_task = tokio::spawn(async move {
        while let Ok((mut sock, _)) = rest_listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
            rest_hdr_srv.lock().unwrap().push(req_str);
            let body = r#"{"data":{"NSE_EQ":{"2885":{"last_price":2950.5}}}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });

    // WS server: accepts connection #1, receives initial subscribe, rotates token & drops;
    // accepts connection #2, expects replayed subscribe JSON, sends live ticker packet.
    let ws_task = tokio::spawn(async move {
        let (s1, _) = ws_listener.accept().await.unwrap();
        let u1 = uris_srv.clone();
        let mut ws1 = accept_hdr_async(s1, move |req: &Request, resp: Response| {
            u1.lock().unwrap().push(req.uri().to_string());
            Ok(resp)
        })
        .await
        .unwrap();

        if let Some(Ok(Message::Text(txt))) = ws1.next().await {
            sub_srv.lock().unwrap().push(txt);
        }
        let _ = ws1
            .send(Message::Binary(dhan_ticker_packet(
                2885,
                2940.0,
                1_700_000_000,
            )))
            .await;

        // Rotate token before dropping connection 1 so reconnect picks up DHAN_TOK_V2.
        *provider_for_ws.current.lock().unwrap() = "DHAN_TOK_V2".into();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = ws1.close(None).await;

        let (s2, _) = ws_listener.accept().await.unwrap();
        let u2 = uris_srv.clone();
        let mut ws2 = accept_hdr_async(s2, move |req: &Request, resp: Response| {
            u2.lock().unwrap().push(req.uri().to_string());
            Ok(resp)
        })
        .await
        .unwrap();

        if let Some(Ok(Message::Text(txt))) = ws2.next().await {
            sub_srv.lock().unwrap().push(txt);
        }
        let _ = ws2
            .send(Message::Binary(dhan_ticker_packet(
                2885,
                2965.25,
                1_700_000_010,
            )))
            .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let feed = DhanFeed::with_config(
        "C123",
        format!("ws://{ws_addr}"),
        format!("http://{rest_addr}/v2/marketfeed/ltp"),
        token_provider,
        WsConfig {
            initial_reconnect_delay_secs: 1,
            max_reconnect_delay_secs: 1,
            max_reconnect_attempts: Some(3),
            ..Default::default()
        },
    );

    feed.connect().await.unwrap();
    let health_initial = feed.feed_health();
    assert!(health_initial.connected);
    assert!(health_initial.last_heartbeat.is_some());
    assert_eq!(health_initial.reconnect_count, 0);

    let inst = InstrumentId::try_new(Exchange::Nse, "RELIANCE")
        .unwrap()
        .with_token("2885");
    let mut stream = feed
        .subscribe(vec![Subscription::new(inst, Mode::Ltp)])
        .await
        .unwrap();

    let mut prices = Vec::new();
    for _ in 0..3 {
        let tick = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("timed out waiting for tick")
            .expect("stream ended");
        if let Tick::Ltp { price, .. } = tick {
            prices.push(price.value());
        }
    }

    // Should see live tick 2940, snapshot gap-fill tick 2950.5, and post-reconnect live tick 2965.25.
    assert!(
        prices.contains(&Decimal::new(2940, 0)),
        "missing pre-drop tick: {prices:?}"
    );
    assert!(
        prices.contains(&Decimal::new(29505, 1)),
        "missing snapshot gap-fill tick: {prices:?}"
    );
    assert!(
        prices.contains(&Decimal::new(296525, 2)),
        "missing post-reconnect live tick: {prices:?}"
    );

    let uris_seen = uris.lock().unwrap().clone();
    assert_eq!(uris_seen.len(), 2);
    assert!(uris_seen[0].contains("token=DHAN_TOK_V1"));
    assert!(uris_seen[1].contains("token=DHAN_TOK_V2"));

    let subs_seen = sub_msgs.lock().unwrap().clone();
    assert_eq!(subs_seen.len(), 2, "expected initial + replayed subscribe");
    assert!(subs_seen[1].contains("\"SecurityId\":\"2885\""));

    let rest_seen = rest_headers.lock().unwrap().clone();
    assert!(
        rest_seen.iter().any(|req| req
            .to_ascii_lowercase()
            .contains("access-token: dhan_tok_v2")),
        "expected snapshot REST call with rotated token: {rest_seen:?}"
    );

    let health_after = feed.feed_health();
    assert!(health_after.connected);
    assert!(health_after.reconnect_count >= 1);

    feed.disconnect().await;
    ws_task.abort();
    rest_task.abort();
}
