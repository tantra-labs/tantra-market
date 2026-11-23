# 0014 — Lakehouse v3 has two independent writers with no lock, non-CAS ref update and TOCTOU part naming

**Status**: open
**Filed**: 2026-11-23 — workspace design review (root plan 0021) · **Kind**: design-flaw · **Severity**: high
**Plan**: [0006](../plans/0006-design-review-2026-11.md)

## What's stubbed or hardcoded

- `crates/tantra-market-store/src/manifest.rs:875` `write_atomic(&root.join(&ref_rel), commit_sha)` overwrites `refs/heads/main` unconditionally. Two writers (Rust `compact_v3` / recorder and Python `ManifestStore.commit`, `tantra-data/tantra_data/adapters/manifest_store.py:184`) each read the parent, then both write; last writer wins and the other commit becomes unreachable. No lock file exists on either side.
- `crates/tantra-market-store/src/lakehouse.rs:886-896` `next_part_path`: `exists()` check then `rename`; `crates/tantra-market-store/src/lakehouse.rs:906-913` uses the fixed temp name `<file>.parquet.tmp`, so concurrent recorders collide on it and silently overwrite a part.
- `crates/tantra-market-store/src/manifest.rs:~852-857` hand-rolls canonical JSON "to match Python `json.dumps(sort_keys=True, separators)`" (comment at `manifest.rs:~842`); float/escape differences silently change the hash.
- Schemas are declared twice: Rust `crates/tantra-market-store/src/lakehouse.rs:153-157` (`Field::new("kind", ...)`) vs Python `tantra-data/tantra_data/adapters/lakehouse_v3.py:16` (`BARS_SCHEMA`, `ALIASES_SCHEMA`) and `tantra-data/tantra_data/adapters/parquet_catalog.py:36` (`_ARROW_SCHEMA`). There is no shared schema file or generated conformance test.

## Why

The stated deployment is a recorder daemon next to a cron ingest. Concurrent writers can lose a commit or corrupt a partition; schema drift breaks the other language's reader at runtime only.

## How to close it

1. Designate `tantra-data` as catalog schema owner: publish `schemas/lakehouse_v3/*.json` (Arrow field lists plus canonicalisation golden vectors) consumed by both sides.
2. Add `RefStore::compare_and_swap` (O_EXCL lock file, or object-store conditional put) used by both writers.
3. Use unique temp names (uuid) and `O_EXCL`/link for part files.
4. Add a cross-language golden test in CI: Rust writes, Python reads, and the reverse.

**Verification**: concurrent-writer test in `crates/tantra-market-store/tests/` (two committers, both commits reachable); canonical-JSON golden vectors; cross-language fixture round-trip. Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `make check`, targeted `cargo test -p tantra-market-store --profile prod`, `make prod`.

## Related

- Root gap 0018 (manifest/ref contract); `tantra-data/docs/gaps/0027-bridge-export-drops-datasets-from-main.md`, `tantra-data/docs/gaps/0028-ref-cas-and-catalog-locking.md`.
- Closed gap 0008 (resolver/compaction) did not cover write concurrency.
- [0013](0013-store-memory-bounds-and-object-store.md) (object-store conditional put).
