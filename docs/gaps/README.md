# Gaps index

Every time something in this repo is stubbed, mocked, or hardcoded because the real
dependency (a data source, a pipeline, a design decision) isn't wired up yet, it gets a
numbered entry here instead of being left silent. Per root `.agents/rules/gap-tracking.md`
§3, this index also carries gaps whose symptom shows up in a *different* repo but whose root
cause lives here.

## Entry template

```markdown
# XXXX — Short title

**Status**: open | closed (link the closing commit/PR)

## What's stubbed or hardcoded
...

## Why
...

## How to close it
...
```

## Open gaps

| # | Title | Area |
|---|-------|------|
| [0001](0001-options-chain-reader.md) | Options-chain enumeration/reader — contract-reference + Greeks building blocks exist, but nothing enumerates/serves a chain (blocks chat `options_chain` tool) | `vyapaar-market-store` / `vyapaar-market-pricing` |
| [0002](0002-open-interest-history-and-buildup-tracking.md) | Open-interest time-series history & buildup classification — no persisted OI history for options or futures (blocks chat `oi_buildup` tool) | `vyapaar-market-store` |
