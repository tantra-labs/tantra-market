# Gaps Index — `vyapaar-market`

Every time something in this repo is stubbed, mocked, or hardcoded because the
real dependency (a data source, a pipeline, a design decision) isn't wired up
yet, it gets a numbered entry here instead of being left silent. Per root
`.agents/rules/gap-tracking.md` §5, this index also carries gaps whose symptom
shows up in a *different* repo but whose root cause lives here — and it is the
**implementation tracker** for provider-side gaps that are additionally filed
at the workspace root for cross-repo visibility.

## Open gaps

| # | Title | Area |
|---|-------|------|
| [0001](0001-options-chain-reader.md) | Options-chain enumeration/reader — contract-reference + Greeks building blocks exist, but nothing enumerates/serves a chain (blocks chat `options_chain` tool) | `vyapaar-market-store` / `vyapaar-market-pricing` |
| [0002](0002-open-interest-history-and-buildup-tracking.md) | Open-interest time-series history & buildup classification — no persisted OI history for options or futures (blocks chat `oi_buildup` tool) | `vyapaar-market-store` |
| [0003](0003-broker-websocket-feed-stubs.md) | Broker WS feed crates (`zerodha`, `upstox`, `fyers`, `angelone`) are empty stubs; `dhan` is a skeleton returning an empty stream (root gap 0001) | 5 broker crates |
| [0004](0004-feed-loaders-stubs.md) | `vyapaar-market-feed` and `vyapaar-market-loaders` are empty stubs; feed traits live in `core`, loaders live in `store` (root gap 0002) | `vyapaar-market-feed` / `vyapaar-market-loaders` |

## Closed gaps

| # | Title | Closed In |
|---|-------|-----------|
| — | *(none)* | — |

## Related

- Root index: `../../../docs/gaps/README.md` (gaps 0001 `market-broker-feed-stubs`,
  0002 `market-feed-loaders-stubs`).
- [`docs/design/architecture.md`](../design/architecture.md) — crate status table.
