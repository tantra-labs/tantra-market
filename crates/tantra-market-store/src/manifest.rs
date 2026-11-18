//! Lakehouse v3 git-style CAS manifest resolver, writer, and alias directory
//! (`docs/design/0004-data-catalog-lakehouse-layout.md`, `tantra-market/docs/gaps/0008`).
//!
//! Resolves immutable snapshots (`refs/heads/*`, `refs/tags/*`, or raw 64-hex
//! `commit_sha`) via `{root}/commits/<sha>.json` and `{root}/trees/<tree_hash>.json`
//! into content-addressed Parquet blobs under `{root}/objects/sha256/<ab>/<sha>.parquet`.
//! When reading a pre-existing v3 root whose `trees/<tree_hash>.json` was not exported
//! yet and the caller is resolving `main`, [`ManifestResolver`] safely falls back to
//! scanning `{root}/views/...` for that branch only — pinned tags and historical commits
//! always require an immutable tree snapshot.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use arrow::array::{Array, Date32Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, NaiveDate, SecondsFormat, TimeZone, Utc};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tantra_core::Exchange;
use thiserror::Error;
use uuid::Uuid;

/// Expected `schema_version` of v3 manifest commits and trees.
pub const LAKEHOUSE_SCHEMA_VERSION: u32 = 3;

/// Days from 0001-01-01 to 1970-01-01 for Arrow `Date32` conversion.
const UNIX_EPOCH_DAYS_FROM_CE: i64 = 719_163;

/// Metadata record parsed from `{root}/commits/<sha>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestCommit {
    /// Content SHA-256 hex digest identifying this immutable commit.
    pub commit_sha: String,
    /// Optional parent commit SHA-256 hex digest.
    pub parent_sha: Option<String>,
    /// Canonical tree SHA-256 hex digest (`{root}/trees/<tree_hash>.json`).
    pub tree_hash: String,
    /// UTC creation timestamp serialized in ISO-8601 / RFC 3339 format.
    pub created_at_utc: String,
    /// IST trading date represented by this commit.
    pub as_of_ist: NaiveDate,
    /// Lakehouse manifest schema version (expected [`LAKEHOUSE_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Human-readable commit summary message.
    pub summary: String,
}

/// One partition entry inside a v3 manifest tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Logical domain (`"market"`, `"derivatives"`, `"disclosures"`, `"fundamentals"`, `"reference"`).
    pub domain: String,
    /// Dataset within the domain (`"bars"`, `"ticks"`, `"open_interest"`, `"block_deals"`, `"aliases"`, ...).
    pub dataset: String,
    /// Exchange coordinate when applicable (e.g. `"NSE"`, `"NFO"`).
    pub exchange: Option<String>,
    /// Asset class coordinate when applicable (e.g. `"EQ"`, `"equity"`, `"FO"`).
    pub asset_class: Option<String>,
    /// Bar timeframe coordinate when applicable (e.g. `"1d"`, `"5m"`).
    pub timeframe: Option<String>,
    /// Time bucket partition path segment (e.g. `"year=2024"`, `"year=2026/month=09"`, `"date=2026-10-20"`).
    pub time_bucket: String,
    /// Content SHA-256 hex digest of the CAS Parquet object.
    pub object_sha: String,
    /// Inclusive minimum UTC event timestamp contained in this object.
    pub min_ts: DateTime<Utc>,
    /// Inclusive maximum UTC event timestamp contained in this object.
    pub max_ts: DateTime<Utc>,
    /// Number of rows stored in this CAS Parquet object.
    pub row_count: usize,
    /// Declared sort key expression, e.g. `"(instrument_uid, ts)"`.
    pub sort_key: String,
}

/// Point-in-time ticker alias mapping an `(exchange, symbol)` window to a canonical `instrument_uid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentAlias {
    /// Exchange code (e.g. `"NSE"`).
    pub exchange: String,
    /// Ticker symbol valid during `[valid_from, valid_to]`.
    pub symbol: String,
    /// Optional series filter (e.g. `"EQ"`, `"BE"`).
    pub series: Option<String>,
    /// First inclusive trading date this alias is active (`None` means open start).
    pub valid_from: Option<NaiveDate>,
    /// Last inclusive trading date this alias is active (`None` means still active).
    pub valid_to: Option<NaiveDate>,
    /// Canonical immutable instrument identifier (typically ISIN or `{EX}:{SYMBOL}`).
    pub instrument_uid: String,
}

/// Errors returned while resolving, reading, or committing Lakehouse v3 manifests.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// Filesystem I/O failure.
    #[error("manifest I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Commit or tree JSON decode/encode failure.
    #[error("manifest JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Parquet decode/encode failure.
    #[error("manifest Parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// Arrow record-batch decode/cast failure.
    #[error("manifest Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
    /// Requested git-style ref could not be resolved to any commit.
    #[error("ref not found: {0}")]
    RefNotFound(String),
    /// Ref name failed validation (e.g. path traversal).
    #[error("invalid ref name: {0}")]
    InvalidRef(String),
    /// Commit SHA string is not a 64-character lowercase hex digest.
    #[error("invalid commit SHA {0:?}")]
    InvalidCommitSha(String),
    /// Commit metadata file is missing.
    #[error("commit {0} not found")]
    CommitNotFound(String),
    /// Commit `schema_version` does not match v3.
    #[error("unsupported manifest schema_version {found} (expected {expected})")]
    UnsupportedSchemaVersion {
        /// Version encoded in the commit file.
        found: u32,
        /// Version supported by this reader.
        expected: u32,
    },
    /// Tree snapshot file `{root}/trees/<tree_hash>.json` is missing for a pinned commit.
    #[error("missing tree snapshot {tree_hash} for commit {commit_sha}")]
    MissingTree {
        /// Expected tree SHA-256 hex digest.
        tree_hash: String,
        /// Commit referencing the missing tree.
        commit_sha: String,
    },
    /// Referenced CAS Parquet blob does not exist on disk.
    #[error("missing CAS object {object_sha} at {path}")]
    MissingObject {
        /// Expected CAS object SHA-256 hex digest.
        object_sha: String,
        /// Expected object filesystem path.
        path: PathBuf,
    },
    /// Manifest entries violated monotonic `(min_ts, max_ts)` ordering.
    #[error("non-monotonic manifest entries: {0}")]
    NonMonotonicEntries(String),
    /// Invalid field encoding inside a manifest JSON or alias table.
    #[error("invalid manifest value: {0}")]
    BadValue(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawCommitJson {
    parent_sha: Option<String>,
    tree_hash: String,
    created_at_utc: String,
    as_of_ist: String,
    schema_version: u32,
    summary: String,
    #[serde(default)]
    entries: Option<Vec<RawManifestEntryJson>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawManifestEntryJson {
    asset_class: Option<String>,
    dataset: String,
    domain: String,
    exchange: Option<String>,
    max_ts: String,
    min_ts: String,
    object_sha: String,
    row_count: usize,
    sort_key: String,
    time_bucket: String,
    timeframe: Option<String>,
}

impl ManifestEntry {
    /// Expected CAS Parquet blob path under `root`:
    /// `{root}/objects/sha256/<ab>/<object_sha>.parquet`.
    pub fn object_path(&self, root: &Path) -> PathBuf {
        object_path_for_sha(root, &self.object_sha)
    }

    /// Expected materialized Hive view path under `root`:
    /// `{root}/views/{domain}/{dataset}/.../{object_sha}.parquet`.
    pub fn view_path(&self, root: &Path) -> PathBuf {
        let mut path = root.join("views").join(&self.domain).join(&self.dataset);
        if let Some(ex) = &self.exchange {
            path = path.join(format!("exchange={ex}"));
        }
        if let Some(ac) = &self.asset_class {
            path = path.join(format!("asset_class={ac}"));
        }
        if let Some(tf) = &self.timeframe {
            path = path.join(format!("timeframe={tf}"));
        }
        for part in self.time_bucket.split('/') {
            if !part.is_empty() {
                path = path.join(part);
            }
        }
        path.join(format!("{}.parquet", self.object_sha))
    }

    /// Returns `true` when this entry matches every non-`None` filter coordinate.
    pub fn matches(
        &self,
        domain: &str,
        dataset: &str,
        exchange: Option<&str>,
        asset_class: Option<&str>,
        timeframe: Option<&str>,
        time_bucket: Option<&str>,
    ) -> bool {
        if self.domain != domain || self.dataset != dataset {
            return false;
        }
        if let Some(want) = exchange
            && !self
                .exchange
                .as_deref()
                .is_some_and(|got| got.eq_ignore_ascii_case(want))
        {
            return false;
        }
        if let Some(want) = asset_class
            && !self
                .asset_class
                .as_deref()
                .is_some_and(|got| asset_class_eq(got, want))
        {
            return false;
        }
        if let Some(want) = timeframe
            && !self
                .timeframe
                .as_deref()
                .is_some_and(|got| got.eq_ignore_ascii_case(want))
        {
            return false;
        }
        if let Some(want) = time_bucket
            && self.time_bucket != want
        {
            return false;
        }
        true
    }
}

fn asset_class_eq(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    matches!(
        (
            a.to_ascii_lowercase().as_str(),
            b.to_ascii_lowercase().as_str()
        ),
        ("eq" | "equity", "eq" | "equity")
            | ("fut" | "futures", "fut" | "futures")
            | ("opt" | "options", "opt" | "options")
    )
}

/// Canonical CAS object path for a 64-hex SHA-256 digest under `root`.
pub fn object_path_for_sha(root: &Path, object_sha: &str) -> PathBuf {
    let prefix = object_sha.get(..2).unwrap_or("00");
    root.join("objects")
        .join("sha256")
        .join(prefix)
        .join(format!("{object_sha}.parquet"))
}

/// Validates that `sha` is a 64-character lowercase hexadecimal digest.
pub fn is_valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Lakehouse v3 manifest resolver over `{root}/{refs,commits,trees,objects,views}`.
#[derive(Debug, Clone)]
pub struct ManifestResolver {
    root: PathBuf,
    legacy_fallback: bool,
}

impl ManifestResolver {
    /// Creates a manifest resolver rooted at `root` with `legacy_fallback = true`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            legacy_fallback: true,
        }
    }

    /// Configures whether callers may fall back to legacy v1/v2 directory layouts
    /// when a v3 manifest is not present.
    #[must_use]
    pub fn with_legacy_fallback(mut self, enabled: bool) -> Self {
        self.legacy_fallback = enabled;
        self
    }

    /// Whether legacy v1/v2 fallback reading is enabled.
    pub fn legacy_fallback(&self) -> bool {
        self.legacy_fallback
    }

    /// Lakehouse root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns `true` when `root` carries Lakehouse v3 structures (`refs/`, `commits/`,
    /// `trees/`, `objects/sha256/`, or `views/`).
    pub fn has_v3_layout(&self) -> bool {
        self.root.join("refs").is_dir()
            || self.root.join("commits").is_dir()
            || self.root.join("trees").is_dir()
            || self.root.join("objects").join("sha256").is_dir()
            || self.root.join("views").is_dir()
    }

    /// Resolves `reference` (default `"refs/heads/main"`) to its 64-char commit SHA.
    ///
    /// Accepts a raw 64-char SHA-256, an explicit `"refs/..."` relative path,
    /// or a short branch/tag name (checked under `refs/heads/<name>` then `refs/tags/<name>`).
    pub fn resolve_ref_sha(&self, reference: Option<&str>) -> Result<String, ManifestError> {
        let raw = reference.map(str::trim).filter(|s| !s.is_empty());
        let target = raw.unwrap_or("refs/heads/main");
        if is_valid_sha(target) {
            return Ok(target.to_string());
        }
        validate_ref_name(target)?;
        let candidates = if target.starts_with("refs/") {
            vec![target.to_string()]
        } else {
            vec![
                format!("refs/heads/{target}"),
                format!("refs/tags/{target}"),
            ]
        };
        for candidate in candidates {
            let ref_path = self.root.join(&candidate);
            if ref_path.is_file() {
                let sha = fs::read_to_string(&ref_path)?.trim().to_string();
                if !is_valid_sha(&sha) {
                    return Err(ManifestError::InvalidCommitSha(sha));
                }
                return Ok(sha);
            }
        }
        Err(ManifestError::RefNotFound(target.to_string()))
    }

    /// Loads commit metadata from `{root}/commits/<sha>.json`.
    pub fn load_commit(&self, sha: &str) -> Result<ManifestCommit, ManifestError> {
        if !is_valid_sha(sha) {
            return Err(ManifestError::InvalidCommitSha(sha.to_string()));
        }
        let path = self.root.join("commits").join(format!("{sha}.json"));
        if !path.is_file() {
            return Err(ManifestError::CommitNotFound(sha.to_string()));
        }
        let raw: RawCommitJson = serde_json::from_str(&fs::read_to_string(&path)?)?;
        if raw.schema_version != LAKEHOUSE_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion {
                found: raw.schema_version,
                expected: LAKEHOUSE_SCHEMA_VERSION,
            });
        }
        let as_of_ist = NaiveDate::parse_from_str(&raw.as_of_ist, "%Y-%m-%d").map_err(|e| {
            ManifestError::BadValue(format!("invalid as_of_ist {:?}: {e}", raw.as_of_ist))
        })?;
        Ok(ManifestCommit {
            commit_sha: sha.to_string(),
            parent_sha: raw.parent_sha,
            tree_hash: raw.tree_hash,
            created_at_utc: raw.created_at_utc,
            as_of_ist,
            schema_version: raw.schema_version,
            summary: raw.summary,
        })
    }

    /// Resolves `reference` (default `"refs/heads/main"`) to its [`ManifestCommit`].
    pub fn resolve_ref(&self, reference: Option<&str>) -> Result<ManifestCommit, ManifestError> {
        let sha = self.resolve_ref_sha(reference)?;
        self.load_commit(&sha)
    }

    /// Loads a commit and all of its [`ManifestEntry`] records.
    ///
    /// Reads `{root}/trees/<tree_hash>.json` (or inline `"entries"` inside the commit
    /// JSON). If a pre-existing v3 catalog lacks `trees/<tree_hash>.json` AND `reference`
    /// points to `refs/heads/main` with `{root}/views/` present, scans `{root}/views/`
    /// for `main`; pinned tags and non-`main` commits always require an immutable tree.
    pub fn load_entries(
        &self,
        reference: Option<&str>,
    ) -> Result<(ManifestCommit, Vec<ManifestEntry>), ManifestError> {
        let sha = self.resolve_ref_sha(reference)?;
        let commit_path = self.root.join("commits").join(format!("{sha}.json"));
        if !commit_path.is_file() {
            return Err(ManifestError::CommitNotFound(sha));
        }
        let raw: RawCommitJson = serde_json::from_str(&fs::read_to_string(&commit_path)?)?;
        if raw.schema_version != LAKEHOUSE_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion {
                found: raw.schema_version,
                expected: LAKEHOUSE_SCHEMA_VERSION,
            });
        }
        let as_of_ist = NaiveDate::parse_from_str(&raw.as_of_ist, "%Y-%m-%d").map_err(|e| {
            ManifestError::BadValue(format!("invalid as_of_ist {:?}: {e}", raw.as_of_ist))
        })?;
        let commit = ManifestCommit {
            commit_sha: sha.clone(),
            parent_sha: raw.parent_sha,
            tree_hash: raw.tree_hash.clone(),
            created_at_utc: raw.created_at_utc,
            as_of_ist,
            schema_version: raw.schema_version,
            summary: raw.summary,
        };

        let tree_path = self
            .root
            .join("trees")
            .join(format!("{}.json", commit.tree_hash));
        if tree_path.is_file() {
            let raw_entries: Vec<RawManifestEntryJson> =
                serde_json::from_str(&fs::read_to_string(&tree_path)?)?;
            let entries = raw_entries
                .into_iter()
                .map(parse_raw_entry)
                .collect::<Result<Vec<_>, _>>()?;
            return Ok((commit, entries));
        }
        if let Some(inline) = raw.entries {
            let entries = inline
                .into_iter()
                .map(parse_raw_entry)
                .collect::<Result<Vec<_>, _>>()?;
            return Ok((commit, entries));
        }

        let is_main = matches!(
            reference.map(str::trim),
            None | Some("" | "main" | "refs/heads/main")
        );
        if is_main && self.root.join("views").is_dir() {
            let entries = scan_views_as_entries(&self.root)?;
            return Ok((commit, entries));
        }

        Err(ManifestError::MissingTree {
            tree_hash: commit.tree_hash,
            commit_sha: commit.commit_sha,
        })
    }

    /// Resolves matching manifest entries and their on-disk Parquet file paths for a logical
    /// domain/dataset coordinate at `reference` (default `"refs/heads/main"`).
    ///
    /// Returns `(Some(commit), entries)` when resolved from a commit manifest, or
    /// `(None, entries)` when only an uncommitted `{root}/views/` directory is present,
    /// or `(None, Vec::new())` when no v3 structures exist under `root`.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn resolve_partition_entries(
        &self,
        domain: &str,
        dataset: &str,
        exchange: Option<&str>,
        asset_class: Option<&str>,
        timeframe: Option<&str>,
        time_bucket: Option<&str>,
        reference: Option<&str>,
    ) -> Result<(Option<ManifestCommit>, Vec<(ManifestEntry, PathBuf)>), ManifestError> {
        let has_commits_or_refs =
            self.root.join("refs").is_dir() || self.root.join("commits").is_dir();
        if has_commits_or_refs {
            let (commit, entries) = self.load_entries(reference)?;
            let mut matched = Vec::new();
            for entry in entries {
                if !entry.matches(
                    domain,
                    dataset,
                    exchange,
                    asset_class,
                    timeframe,
                    time_bucket,
                ) {
                    continue;
                }
                let obj = entry.object_path(&self.root);
                if obj.is_file() {
                    matched.push((entry, obj));
                } else {
                    let view = entry.view_path(&self.root);
                    if view.is_file() {
                        matched.push((entry, view));
                    } else {
                        return Err(ManifestError::MissingObject {
                            object_sha: entry.object_sha,
                            path: obj,
                        });
                    }
                }
            }
            return Ok((Some(commit), matched));
        }

        if self.root.join("views").is_dir() {
            let entries = scan_views_as_entries(&self.root)?;
            let mut matched = Vec::new();
            for entry in entries {
                if !entry.matches(
                    domain,
                    dataset,
                    exchange,
                    asset_class,
                    timeframe,
                    time_bucket,
                ) {
                    continue;
                }
                let view = entry.view_path(&self.root);
                let obj = entry.object_path(&self.root);
                let path = if obj.is_file() { obj } else { view };
                matched.push((entry, path));
            }
            return Ok((None, matched));
        }

        Ok((None, Vec::new()))
    }

    /// Resolves matching Parquet file paths for a logical v3 coordinate.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_partition(
        &self,
        domain: &str,
        dataset: &str,
        exchange: Option<&str>,
        asset_class: Option<&str>,
        timeframe: Option<&str>,
        time_bucket: Option<&str>,
        reference: Option<&str>,
    ) -> Result<Vec<PathBuf>, ManifestError> {
        let (_, pairs) = self.resolve_partition_entries(
            domain,
            dataset,
            exchange,
            asset_class,
            timeframe,
            time_bucket,
            reference,
        )?;
        Ok(pairs.into_iter().map(|(_, path)| path).collect())
    }

    /// Loads `reference/aliases` rows from the resolved commit (or `{root}/views/reference/aliases`
    /// / `{root}/reference/aliases` fallback).
    pub fn load_aliases(
        &self,
        reference: Option<&str>,
    ) -> Result<Vec<InstrumentAlias>, ManifestError> {
        let mut files =
            self.resolve_partition("reference", "aliases", None, None, None, None, reference)?;
        if files.is_empty() {
            let direct_dir = self.root.join("reference").join("aliases");
            if direct_dir.is_dir() {
                collect_parquet_files_recursive(&direct_dir, &mut files)?;
            }
        }
        let mut aliases = Vec::new();
        for path in files {
            read_alias_file(&path, &mut aliases)?;
        }
        Ok(aliases)
    }

    /// Resolves `(exchange, symbol)` at optional `as_of` date to a canonical `instrument_uid`.
    ///
    /// Mirrors `ManifestStore.resolve_alias` in `tantra-data`: when `as_of` is `Some(date)`,
    /// only aliases with `[valid_from, valid_to]` covering `date` match; without `as_of`, the
    /// most recent alias window wins.
    pub fn resolve_alias(
        aliases: &[InstrumentAlias],
        exchange: &str,
        symbol: &str,
        as_of: Option<NaiveDate>,
    ) -> Option<String> {
        let mut matches: Vec<&InstrumentAlias> = aliases
            .iter()
            .filter(|a| {
                a.exchange.eq_ignore_ascii_case(exchange)
                    && a.symbol.eq_ignore_ascii_case(symbol)
                    && match as_of {
                        None => true,
                        Some(d) => {
                            a.valid_from.is_none_or(|vf| vf <= d)
                                && a.valid_to.is_none_or(|vt| vt >= d)
                        }
                    }
            })
            .collect();
        matches.sort_by(|a, b| {
            let a_rank = (a.valid_from.is_some(), a.valid_from);
            let b_rank = (b.valid_from.is_some(), b.valid_from);
            b_rank.cmp(&a_rank).then_with(|| {
                let a_eq = a
                    .series
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("EQ"));
                let b_eq = b
                    .series
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("EQ"));
                b_eq.cmp(&a_eq)
            })
        });
        matches.first().map(|a| a.instrument_uid.clone())
    }

    /// Returns all aliases sharing `instrument_uid`.
    pub fn aliases_for_uid<'a>(
        aliases: &'a [InstrumentAlias],
        instrument_uid: &str,
    ) -> Vec<&'a InstrumentAlias> {
        aliases
            .iter()
            .filter(|a| a.instrument_uid == instrument_uid)
            .collect()
    }

    /// Picks the canonical `(Exchange, symbol)` display identifier for `instrument_uid`.
    ///
    /// Prefers an open-ended or newest `"EQ"`-series alias, falling back to any alias for `uid`.
    pub fn canonical_symbol(
        aliases: &[InstrumentAlias],
        instrument_uid: &str,
        as_of: Option<NaiveDate>,
    ) -> Option<(Exchange, String)> {
        let mut candidates: Vec<&InstrumentAlias> = aliases
            .iter()
            .filter(|a| {
                a.instrument_uid == instrument_uid
                    && match as_of {
                        None => true,
                        Some(d) => {
                            a.valid_from.is_none_or(|vf| vf <= d)
                                && a.valid_to.is_none_or(|vt| vt >= d)
                        }
                    }
            })
            .collect();
        if candidates.is_empty() && as_of.is_some() {
            candidates = Self::aliases_for_uid(aliases, instrument_uid);
        }
        candidates.sort_by(|a, b| {
            let a_eq = a
                .series
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("EQ"));
            let b_eq = b
                .series
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("EQ"));
            let a_open = a.valid_to.is_none();
            let b_open = b.valid_to.is_none();
            b_eq.cmp(&a_eq)
                .then_with(|| b_open.cmp(&a_open))
                .then_with(|| b.valid_to.cmp(&a.valid_to))
                .then_with(|| b.valid_from.cmp(&a.valid_from))
        });
        candidates.into_iter().find_map(|a| {
            a.exchange
                .parse::<Exchange>()
                .ok()
                .map(|ex| (ex, a.symbol.clone()))
        })
    }
}

/// Resolves Parquet paths for flat projection readers (`block_deals`, `filings`, etc.):
/// checks v3 manifest / `views/` first, then `{root}/{domain}/{dataset}`, and finally
/// `{root}/catalog/{legacy_subdir}` only when `legacy_fallback` is `true`.
pub(crate) fn resolve_projection_files(
    root: &Path,
    domain: &str,
    dataset: &str,
    legacy_subdir: &str,
    reference: Option<&str>,
    legacy_fallback: bool,
) -> Result<Vec<PathBuf>, ManifestError> {
    let resolver = ManifestResolver::new(root).with_legacy_fallback(legacy_fallback);
    if resolver.has_v3_layout() {
        let files =
            resolver.resolve_partition(domain, dataset, None, None, None, None, reference)?;
        if !files.is_empty() {
            return Ok(files);
        }
    }

    let v3_dir = root.join(domain).join(dataset);
    if v3_dir.is_dir() {
        let mut files = Vec::new();
        collect_parquet_files_recursive(&v3_dir, &mut files)?;
        files.sort();
        if !files.is_empty() {
            return Ok(files);
        }
    }

    if legacy_fallback {
        let legacy_dir = root.join("catalog").join(legacy_subdir);
        if legacy_dir.is_dir() {
            let mut files = Vec::new();
            collect_parquet_files_recursive(&legacy_dir, &mut files)?;
            files.sort();
            return Ok(files);
        }
    }

    Ok(Vec::new())
}

/// Decodes a `Utf8` or `LargeUtf8` Arrow column into an owned [`StringArray`].
pub fn arrow_utf8_array(column: &dyn Array) -> Result<StringArray, arrow::error::ArrowError> {
    match column.data_type() {
        DataType::Utf8 => Ok(column
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("Utf8 array downcasts to StringArray")
            .clone()),
        DataType::LargeUtf8 => {
            let casted = cast(column, &DataType::Utf8)?;
            Ok(casted
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("cast to Utf8 yields StringArray")
                .clone())
        }
        other => Err(arrow::error::ArrowError::InvalidArgumentError(format!(
            "expected Utf8 or LargeUtf8, got {other:?}"
        ))),
    }
}

/// Writes `batch` to `{root}/objects/sha256/<ab>/<sha256>.parquet` atomically with `compression`
/// and returns its 64-character lowercase SHA-256 hex digest.
pub fn write_cas_batch(
    root: &Path,
    batch: &RecordBatch,
    compression: Compression,
) -> Result<String, ManifestError> {
    let tmp_dir = root.join("objects").join("sha256").join(".tmp");
    fs::create_dir_all(&tmp_dir)?;
    let tmp_path = tmp_dir.join(format!("{}.parquet.tmp", Uuid::new_v4().simple()));

    {
        let file = File::create(&tmp_path)?;
        let props = WriterProperties::builder()
            .set_compression(compression)
            .build();
        let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))?;
        writer.write(batch)?;
        writer.close()?;
    }

    let mut hasher = Sha256::new();
    let mut reader = File::open(&tmp_path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    drop(reader);
    let sha = hex::encode(hasher.finalize());
    let dest = object_path_for_sha(root, &sha);
    if dest.exists() {
        let _ = fs::remove_file(&tmp_path);
        return Ok(sha);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&tmp_path, &dest)?;
    Ok(sha)
}

/// Atomically writes `{root}/trees/<tree_hash>.json`, `{root}/commits/<commit_sha>.json`,
/// and updates `refs/heads/<branch>` (default `"main"`).
pub fn commit_entries(
    root: &Path,
    summary: &str,
    as_of_ist: NaiveDate,
    mut entries: Vec<ManifestEntry>,
    parent: Option<&ManifestCommit>,
    branch: Option<&str>,
) -> Result<ManifestCommit, ManifestError> {
    entries.sort_by_key(|a| (a.min_ts, a.max_ts));
    for pair in entries.windows(2) {
        let (prev, curr) = (&pair[0], &pair[1]);
        if (curr.min_ts, curr.max_ts) < (prev.min_ts, prev.max_ts) {
            return Err(ManifestError::NonMonotonicEntries(format!(
                "{} before {}",
                prev.object_sha, curr.object_sha
            )));
        }
    }

    let raw_entries: Vec<RawManifestEntryJson> = entries
        .iter()
        .map(|e| RawManifestEntryJson {
            asset_class: e.asset_class.clone(),
            dataset: e.dataset.clone(),
            domain: e.domain.clone(),
            exchange: e.exchange.clone(),
            max_ts: format_iso_utc(e.max_ts),
            min_ts: format_iso_utc(e.min_ts),
            object_sha: e.object_sha.clone(),
            row_count: e.row_count,
            sort_key: e.sort_key.clone(),
            time_bucket: e.time_bucket.clone(),
            timeframe: e.timeframe.clone(),
        })
        .collect();
    // Struct fields are sorted alphabetically to match Python `json.dumps(..., sort_keys=True, separators=(",", ":"))`.
    let tree_canonical = serde_json::to_string(&raw_entries)?;
    let tree_hash = sha256_hex(tree_canonical.as_bytes());

    let tree_path = root.join("trees").join(format!("{tree_hash}.json"));
    if !tree_path.exists() {
        write_atomic(&tree_path, &format!("{tree_canonical}\n"))?;
    }

    let created_at_utc = format_iso_utc(Utc::now());
    let parent_sha = parent.map(|p| p.commit_sha.clone());
    let commit_canonical_map = serde_json::json!({
        "as_of_ist": as_of_ist.format("%Y-%m-%d").to_string(),
        "created_at_utc": created_at_utc,
        "parent_sha": parent_sha,
        "schema_version": LAKEHOUSE_SCHEMA_VERSION,
        "summary": summary,
        "tree_hash": tree_hash,
    });
    let commit_canonical = serde_json::to_string(&commit_canonical_map)?;
    let commit_sha = sha256_hex(commit_canonical.as_bytes());

    let commit_path = root.join("commits").join(format!("{commit_sha}.json"));
    write_atomic(&commit_path, &format!("{commit_canonical}\n"))?;

    let branch_name = branch.unwrap_or("main").trim();
    let ref_rel = if branch_name.starts_with("refs/") {
        branch_name.to_string()
    } else {
        format!("refs/heads/{branch_name}")
    };
    validate_ref_name(&ref_rel)?;
    write_atomic(&root.join(&ref_rel), &format!("{commit_sha}\n"))?;

    Ok(ManifestCommit {
        commit_sha,
        parent_sha,
        tree_hash,
        created_at_utc,
        as_of_ist,
        schema_version: LAKEHOUSE_SCHEMA_VERSION,
        summary: summary.to_string(),
    })
}

/// Pins `refs/tags/<tag>` to `commit_sha`.
pub fn create_tag(root: &Path, tag: &str, commit_sha: &str) -> Result<(), ManifestError> {
    if !is_valid_sha(commit_sha) {
        return Err(ManifestError::InvalidCommitSha(commit_sha.to_string()));
    }
    let rel = if tag.starts_with("refs/tags/") {
        tag.to_string()
    } else {
        format!("refs/tags/{tag}")
    };
    validate_ref_name(&rel)?;
    write_atomic(&root.join(rel), &format!("{commit_sha}\n"))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn format_iso_utc(dt: DateTime<Utc>) -> String {
    let micros = dt.timestamp_subsec_micros();
    let base = if micros == 0 {
        dt.to_rfc3339_opts(SecondsFormat::Secs, false)
    } else {
        dt.to_rfc3339_opts(SecondsFormat::Micros, false)
    };
    base.replace('Z', "+00:00")
}

fn write_atomic(path: &Path, content: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(".{}.tmp", Uuid::new_v4().simple()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn validate_ref_name(name: &str) -> Result<(), ManifestError> {
    if name.is_empty() || name.starts_with('/') {
        return Err(ManifestError::InvalidRef(name.to_string()));
    }
    for comp in Path::new(name).components() {
        match comp {
            Component::Normal(_) => {}
            _ => return Err(ManifestError::InvalidRef(name.to_string())),
        }
    }
    Ok(())
}

fn parse_raw_entry(raw: RawManifestEntryJson) -> Result<ManifestEntry, ManifestError> {
    let min_ts = DateTime::parse_from_rfc3339(&raw.min_ts)
        .map_err(|e| ManifestError::BadValue(format!("invalid min_ts {:?}: {e}", raw.min_ts)))?
        .with_timezone(&Utc);
    let max_ts = DateTime::parse_from_rfc3339(&raw.max_ts)
        .map_err(|e| ManifestError::BadValue(format!("invalid max_ts {:?}: {e}", raw.max_ts)))?
        .with_timezone(&Utc);
    Ok(ManifestEntry {
        domain: raw.domain,
        dataset: raw.dataset,
        exchange: raw.exchange,
        asset_class: raw.asset_class,
        timeframe: raw.timeframe,
        time_bucket: raw.time_bucket,
        object_sha: raw.object_sha,
        min_ts,
        max_ts,
        row_count: raw.row_count,
        sort_key: raw.sort_key,
    })
}

pub(crate) fn collect_parquet_files_recursive(
    dir: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), std::io::Error> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_parquet_files_recursive(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "parquet") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan_views_as_entries(root: &Path) -> Result<Vec<ManifestEntry>, ManifestError> {
    let views_dir = root.join("views");
    let mut files = Vec::new();
    collect_parquet_files_recursive(&views_dir, &mut files)?;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let Ok(rel) = file.strip_prefix(&views_dir) else {
            continue;
        };
        let parts: Vec<String> = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
            .collect();
        if parts.len() < 3 {
            continue;
        }
        let domain = parts[0].clone();
        let dataset = parts[1].clone();
        let file_stem = Path::new(&parts[parts.len() - 1])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let mut exchange = None;
        let mut asset_class = None;
        let mut timeframe = None;
        let mut bucket_parts = Vec::new();
        for seg in &parts[2..parts.len() - 1] {
            if let Some((k, v)) = seg.split_once('=') {
                match k {
                    "exchange" => exchange = Some(v.to_string()),
                    "asset_class" => asset_class = Some(v.to_string()),
                    "timeframe" => timeframe = Some(v.to_string()),
                    _ => bucket_parts.push(seg.clone()),
                }
            } else {
                bucket_parts.push(seg.clone());
            }
        }
        let time_bucket = bucket_parts.join("/");
        let (min_ts, max_ts) = bounds_from_time_bucket(&time_bucket);
        entries.push(ManifestEntry {
            domain,
            dataset,
            exchange,
            asset_class,
            timeframe,
            time_bucket,
            object_sha: file_stem,
            min_ts,
            max_ts,
            row_count: 0,
            sort_key: "(instrument_uid, ts)".to_string(),
        });
    }
    entries.sort_by_key(|a| (a.min_ts, a.max_ts));
    Ok(entries)
}

fn bounds_from_time_bucket(time_bucket: &str) -> (DateTime<Utc>, DateTime<Utc>) {
    let mut year: Option<i32> = None;
    let mut month: Option<u32> = None;
    let mut date: Option<NaiveDate> = None;
    for part in time_bucket.split('/') {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "year" => year = v.parse().ok(),
                "month" => month = v.parse().ok(),
                "date" => date = NaiveDate::parse_from_str(v, "%Y-%m-%d").ok(),
                _ => {}
            }
        }
    }
    if let Some(d) = date {
        let start = d.and_hms_opt(0, 0, 0).expect("valid start").and_utc();
        let end = d.and_hms_opt(23, 59, 59).expect("valid end").and_utc();
        return (start, end);
    }
    if let (Some(y), Some(m)) = (year, month)
        && let Some(first) = NaiveDate::from_ymd_opt(y, m, 1)
    {
        let next_month = if m == 12 {
            NaiveDate::from_ymd_opt(y + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(y, m + 1, 1)
        };
        let last = next_month.and_then(|n| n.pred_opt()).unwrap_or(first);
        return (
            first.and_hms_opt(0, 0, 0).expect("valid").and_utc(),
            last.and_hms_opt(23, 59, 59).expect("valid").and_utc(),
        );
    }
    if let Some(y) = year
        && let (Some(first), Some(last)) = (
            NaiveDate::from_ymd_opt(y, 1, 1),
            NaiveDate::from_ymd_opt(y, 12, 31),
        )
    {
        return (
            first.and_hms_opt(0, 0, 0).expect("valid").and_utc(),
            last.and_hms_opt(23, 59, 59).expect("valid").and_utc(),
        );
    }
    (
        Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2099, 12, 31, 23, 59, 59).unwrap(),
    )
}

fn read_alias_file(path: &Path, out: &mut Vec<InstrumentAlias>) -> Result<(), ManifestError> {
    let file = File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    for batch_res in reader {
        let batch = batch_res?;
        let schema = batch.schema();
        let idx = |name: &'static str| -> Result<usize, ManifestError> {
            schema
                .index_of(name)
                .map_err(|_| ManifestError::BadValue(format!("missing column {name} in {path:?}")))
        };
        let exchanges = arrow_utf8_array(batch.column(idx("exchange")?).as_ref())?;
        let symbols = arrow_utf8_array(batch.column(idx("symbol")?).as_ref())?;
        let uids = arrow_utf8_array(batch.column(idx("instrument_uid")?).as_ref())?;
        let series = match schema.index_of("series") {
            Ok(i) => Some(arrow_utf8_array(batch.column(i).as_ref())?),
            Err(_) => None,
        };
        let valid_from = match schema.index_of("valid_from") {
            Ok(i) => Some(extract_opt_dates(batch.column(i).as_ref())?),
            Err(_) => None,
        };
        let valid_to = match schema.index_of("valid_to") {
            Ok(i) => Some(extract_opt_dates(batch.column(i).as_ref())?),
            Err(_) => None,
        };
        for row in 0..batch.num_rows() {
            if exchanges.is_null(row) || symbols.is_null(row) || uids.is_null(row) {
                continue;
            }
            out.push(InstrumentAlias {
                exchange: exchanges.value(row).to_string(),
                symbol: symbols.value(row).to_string(),
                series: series
                    .as_ref()
                    .filter(|s| !s.is_null(row))
                    .map(|s| s.value(row).to_string()),
                valid_from: valid_from.as_ref().and_then(|v| v[row]),
                valid_to: valid_to.as_ref().and_then(|v| v[row]),
                instrument_uid: uids.value(row).to_string(),
            });
        }
    }
    Ok(())
}

fn extract_opt_dates(col: &dyn Array) -> Result<Vec<Option<NaiveDate>>, ManifestError> {
    let mut out = Vec::with_capacity(col.len());
    match col.data_type() {
        DataType::Date32 => {
            let arr = col
                .as_any()
                .downcast_ref::<Date32Array>()
                .expect("Date32 downcasts");
            for row in 0..arr.len() {
                if arr.is_null(row) {
                    out.push(None);
                } else {
                    let days_ce = i64::from(arr.value(row)) + UNIX_EPOCH_DAYS_FROM_CE;
                    let d = i32::try_from(days_ce)
                        .ok()
                        .and_then(NaiveDate::from_num_days_from_ce_opt);
                    out.push(d);
                }
            }
        }
        DataType::Utf8 | DataType::LargeUtf8 => {
            let arr = arrow_utf8_array(col)?;
            for row in 0..arr.len() {
                if arr.is_null(row) || arr.value(row).is_empty() {
                    out.push(None);
                } else {
                    out.push(NaiveDate::parse_from_str(arr.value(row), "%Y-%m-%d").ok());
                }
            }
        }
        DataType::Null => {
            out.resize(col.len(), None);
        }
        other => {
            return Err(ManifestError::BadValue(format!(
                "unsupported alias date column type {other:?}"
            )));
        }
    }
    Ok(out)
}
