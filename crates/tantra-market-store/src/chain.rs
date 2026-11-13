//! Options-chain enumeration/reader (`tantra-market/docs/gaps/0001-options-chain-reader.md`).
//!
//! [`InstrumentChainReader`] serves the listed option chain for an underlying by joining
//! three ports:
//!
//! 1. **Enumeration** — [`InstrumentMaster::list_chain`] filters the instrument master's
//!    option contracts by underlying (ordered by expiry, then strike, then option type).
//! 2. **Quotes** — [`QuoteReader`] supplies each contract's best bid/ask (last-traded
//!    price is not a top-of-book field; consumers derive a midpoint where useful).
//! 3. **Open interest** — an optional [`OpenInterestStore`] supplies the latest OI per
//!    contract. This is the first cut's OI join (gap 0002's snapshot store); when no OI
//!    store is wired the field stays `None` rather than fabricating a value.
//!
//! Every value that crosses this reader's public boundary is exact `Decimal`
//! (`tantra_core::Price`/`Quantity`) — see [`ChainContract`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use tantra_core::Quantity;

use crate::open_interest::OpenInterestStore;
use crate::ports::{
    ChainContract, ChainReader, InstrumentMaster, PortError, PortResult, QuoteReader,
};

/// An option-chain reader joining instrument enumeration with quotes and OI.
#[derive(Clone)]
pub struct InstrumentChainReader {
    instruments: Arc<dyn InstrumentMaster>,
    quotes: Arc<dyn QuoteReader>,
    oi: Option<Arc<dyn OpenInterestStore>>,
}

impl InstrumentChainReader {
    /// Creates a chain reader over the given instrument master and quote source.
    pub fn new(instruments: Arc<dyn InstrumentMaster>, quotes: Arc<dyn QuoteReader>) -> Self {
        Self {
            instruments,
            quotes,
            oi: None,
        }
    }

    /// Attaches an open-interest store so contracts carry their latest OI.
    pub fn with_oi_store(mut self, oi: Arc<dyn OpenInterestStore>) -> Self {
        self.oi = Some(oi);
        self
    }
}

/// Rounds an `f64` into an exact `Decimal` at the reader's boundary (the only place a
/// float may cross: the legacy `f64` wire types the Parquet engine still uses).
fn to_decimal(value: f64) -> Decimal {
    Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
}

#[async_trait]
impl ChainReader for InstrumentChainReader {
    async fn options_chain(
        &self,
        underlying: &tantra_core::InstrumentId,
        expiry: Option<NaiveDate>,
        limit: Option<usize>,
    ) -> PortResult<Vec<ChainContract>> {
        let instruments = self.instruments.list_chain(underlying, None).await?;
        let mut contracts = Vec::with_capacity(instruments.len());
        for instrument in instruments {
            let (Some(expiry_date), Some(strike), Some(option_type)) = (
                instrument.expiry(),
                instrument.strike(),
                instrument.option_type(),
            ) else {
                continue;
            };
            if expiry.is_some_and(|filter| filter != expiry_date) {
                continue;
            }
            let quote = self
                .quotes
                .read_quote(instrument.id(), None)
                .await
                .map_err(|e| {
                    PortError::Internal(format!("quote lookup for {} failed: {e}", instrument.id()))
                })?;
            let (bid, ask) = match quote {
                Some(tick) => (
                    Some(tantra_core::Price::new(to_decimal(tick.bid_price()))),
                    Some(tantra_core::Price::new(to_decimal(tick.ask_price()))),
                ),
                None => (None, None),
            };
            let open_interest = match &self.oi {
                Some(store) => store
                    .latest(instrument.id(), None)
                    .await
                    .map(|snap| snap.map(|s| s.open_interest))
                    .map_err(|e| {
                        PortError::Internal(format!(
                            "OI lookup for {} failed: {e}",
                            instrument.id()
                        ))
                    })?,
                None => None,
            };
            contracts.push(ChainContract {
                instrument: instrument.id().clone(),
                option_type,
                strike: tantra_core::Price::new(to_decimal(strike)),
                expiry: expiry_date,
                lot_size: Quantity::new(to_decimal(instrument.lot_size())),
                last_price: None,
                bid,
                ask,
                open_interest,
            });
        }
        // `list_chain` already ordered by expiry → strike → option type; the join preserves
        // order, so only the cap remains.
        if let Some(limit) = limit {
            contracts.truncate(limit);
        }
        Ok(contracts)
    }
}
