//! End-to-end reconnect, token rotation, subscription replay, REST snapshot gap-fill,
//! and live health metrics tests for [`KiteFeed`] (`tantra-market/docs/gaps/0010`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use tantra_core::{Exchange, InstrumentId, Tick};
use tantra_market_core::{MarketDataFeed, MdError, Mode, Subscription, TokenProvider, WsConfig};
use tantra_market_zerodha::KiteFeed;
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

fn kite_ltp_frame(token: u32, paise: i32) -> Vec<u8> {
    let mut bytes = vec![1u8, 0u8]; // packet count = 1
    bytes.extend_from_slice(&8i16.to_le_bytes()); // packet len = 8
    bytes.extend_from_slice(&token.to_le_bytes());
    bytes.extend_from_slice(&paise.to_le_bytes());
    bytes
}

#[tokio::test]
#[allow(clippy::result_large_err)]
async fn kite_feed_rotates_token_resubscribes_and_fills_snapshot_gap_on_reconnect() {
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
        current: Mutex::new("KITE_TOK_V1".into()),
    });
    let provider_for_ws = token_provider.clone();

    // Minimal HTTP/1.1 REST snapshot server.
    let rest_task = tokio::spawn(async move {
        while let Ok((mut sock, _)) = rest_listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
            rest_hdr_srv.lock().unwrap().push(req_str);
            let body = r#"{"data":{"NSE:INFY":{"instrument_token":408065,"last_price":1525.75}}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });

    // WS server: accepts connection #1, receives subscribe+mode, sends LTP, rotates token & closes;
    // accepts connection #2, expects replayed subscribe+mode, sends live LTP.
    let ws_task = tokio::spawn(async move {
        let (s1, _) = ws_listener.accept().await.unwrap();
        let u1 = uris_srv.clone();
        let mut ws1 = accept_hdr_async(s1, move |req: &Request, resp: Response| {
            u1.lock().unwrap().push(req.uri().to_string());
            Ok(resp)
        })
        .await
        .unwrap();

        for _ in 0..2 {
            if let Some(Ok(Message::Text(txt))) = ws1.next().await {
                sub_srv.lock().unwrap().push(txt);
            }
        }
        let _ = ws1
            .send(Message::Binary(kite_ltp_frame(408_065, 151_000)))
            .await;

        *provider_for_ws.current.lock().unwrap() = "KITE_TOK_V2".into();
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

        for _ in 0..2 {
            if let Some(Ok(Message::Text(txt))) = ws2.next().await {
                sub_srv.lock().unwrap().push(txt);
            }
        }
        let _ = ws2
            .send(Message::Binary(kite_ltp_frame(408_065, 153_250)))
            .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let feed = KiteFeed::with_config(
        "MY_KITE_KEY",
        format!("ws://{ws_addr}"),
        format!("http://{rest_addr}"),
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

    let inst = InstrumentId::try_new(Exchange::Nse, "INFY")
        .unwrap()
        .with_token("408065");
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

    // Should see live tick 1510, snapshot gap-fill tick 1525.75, and post-reconnect live tick 1532.50.
    assert!(
        prices.contains(&Decimal::new(1510, 0)),
        "missing pre-drop tick: {prices:?}"
    );
    assert!(
        prices.contains(&Decimal::new(152575, 2)),
        "missing snapshot gap-fill tick: {prices:?}"
    );
    assert!(
        prices.contains(&Decimal::new(15325, 1)),
        "missing post-reconnect live tick: {prices:?}"
    );

    let uris_seen = uris.lock().unwrap().clone();
    assert_eq!(uris_seen.len(), 2);
    assert!(uris_seen[0].contains("access_token=KITE_TOK_V1"));
    assert!(uris_seen[1].contains("access_token=KITE_TOK_V2"));

    let subs_seen = sub_msgs.lock().unwrap().clone();
    assert_eq!(
        subs_seen.len(),
        4,
        "expected initial (sub+mode) + replayed (sub+mode)"
    );
    assert!(subs_seen[2].contains("subscribe") && subs_seen[2].contains("408065"));
    assert!(subs_seen[3].contains("mode") && subs_seen[3].contains("408065"));

    let rest_seen = rest_headers.lock().unwrap().clone();
    assert!(
        rest_seen
            .iter()
            .any(|req| req.contains("token MY_KITE_KEY:KITE_TOK_V2")),
        "expected snapshot REST call with rotated token: {rest_seen:?}"
    );

    let health_after = feed.feed_health();
    assert!(health_after.connected);
    assert!(health_after.reconnect_count >= 1);

    feed.disconnect().await;
    ws_task.abort();
    rest_task.abort();
}
