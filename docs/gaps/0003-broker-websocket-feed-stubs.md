# 0003 — Real broker WebSocket feed implementations

**Status**: open — **feature tracker, 2 of 5 shipped 2026-11-13 (T3.3)**.

Cross-reference: root gap `docs/gaps/0001-market-broker-feed-stubs.md`
(provider repo `tantra-market`).

## What's real

Two broker adapters are now real `MarketDataFeed` implementations built on
`tantra-market-core`'s `WsClient`/`SymbolMapper`/`ReconnectionManager`, each with
golden-byte-tested wire decoders:

| Broker | Crate | Wire protocol | Commit |
|---|---|---|---|
| Dhan | `tantra-market-dhan` | DhanHQ v2 Live Market Feed (JSON requests, little-endian binary ticker/quote/OI/prev-close/full/disconnect packets) | `fb82571` |
| Zerodha | `tantra-market-zerodha` | Kite Connect 3 ticker (JSON requests, binary frames: ltp 8 / instrument quote 44 / full 184, index quote 28 / full 32) | `ad79205` |

Both normalize every price/quantity to exact `Decimal` before it enters a
canonical `Tick` (the adapter-boundary float exception, documented in each
crate). Live verification against real credentials is a manual follow-on for
each (the Dhan credentials are available; the decoders are unit-verified
against the documented wire format).

## What remains

The empty stub crates for **Upstox**, **Fyers** and **Angel One** were removed
2026-11-13 (root plan 0003 Phase 1). Per gap-tracking §4, they are re-added as
real crates **only** when their WS wire protocols are actually implemented,
mirroring `tantra-market-dhan`/`tantra-market-zerodha` (protocol module +
`MarketDataFeed` impl + golden-byte tests + REST snapshot path):

1. **Upstox** — binary WebSocket feed (ticker/quote/full modes); protocol
   documented at upstox.com developer docs; implement the decoder + crate.
2. **Fyers** — v3 Data Socket is JSON-text (`sf`/`if` equity/option/index
   messages are fully documented); the **raw subscribe request** is only
   exposed through the official `fyers_apiv3` SDK, so the wire request shape
   must be captured from that SDK before the crate can be correct.
3. **Angel One** — SmartAPI 2.0 WebSocket (compressed/JSON tick payloads);
   protocol documented at smartapi.angelbroking.com.

## Verification

`cargo check --workspace --profile prod` passes with the two real crates and
without the remaining three; `make test` green on the market workspace.
