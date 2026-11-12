# `vyapaar-market-finance` — Market Contracts & India Pack Reference

Documents `crates/vyapaar-market-finance/src/`: generic market contracts plus
the pluggable `india` market pack and the `null` offline test pack.

## Module Map

| Module | Purpose |
|--------|---------|
| `calendar.rs` | Generic trading-session/calendar contracts: `Session`, `MarketCalendar`, `TradingCalendar`, `HolidaySource` |
| `costs.rs` | `Charge`, `FeeBreakdown`, `CostSchedule`, `DatedCostScheduleSource`, `MarketSegment`, `edge_after_costs` |
| `expiry.rs` | `ExpiryRules` trait + `LastThursdayExpiry` (standard monthly rule) |
| `rules.rs` | `PriceBand`, `InstrumentRules`, `InstrumentRulesProvider`, `SymbolGrammar`, `PriceViolation`/`QuantityViolation` |
| `settlement.rs` | `MarginModel`, `MarginRequirement`, `PercentageMarginModel`, `SettlementRules`, `StandardRollingSettlement` (T+0/T+1/T+2), `SettlementSchedule` (dated phases) |
| `profile.rs` | `MarketProfile` trait (bundle of calendar+rules+costs+margin+settlement) + `MarketRegistry` |
| `universes.rs` | `Universe`, `UniverseSource`, `StaticUniverse` |
| `aggregator.rs` | `BarAggregation`, `BarSpecification`, `BarType`, `SessionBarAggregator` (zero-lookahead, session-aligned bar resampling) |
| `null.rs` | Offline test pack: `NullCalendar`, `NullCostSchedule`, `NullInstrumentRulesProvider`, `NullSymbolGrammar`, `NullExpiryRules`, `NullMarginModel`, `NullMarketProfile` |
| `india/` | `NseCalendar`, `SttRates`, `CostModel`/`CostBreakdown`/`Segment`, `NseCashEquitySchedule`, `FnoContract` grammar, `Nifty50`, `IndiaExchange`, `IndiaMarketProfile` |

## India F&O Grammar (`india/fno`)

`parse_fno_symbol` / `format_fno_symbol` implement the NSE F&O symbol grammar:

| Contract | Format | Example |
|----------|--------|---------|
| Monthly future | `<UNDERLYING><YY><MMM>FUT` | `NIFTY24OCTFUT` |
| Weekly future | `<UNDERLYING><YY><M><DD>FUT` | `NIFTY24O17FUT` |
| Monthly option | `<UNDERLYING><YY><MMM><STRIKE><CE\|PE>` | `NIFTY24OCT24000CE` |
| Weekly option | `<UNDERLYING><YY><M><DD><STRIKE><CE\|PE>` | `NIFTY24O1724000CE` |

Types: `FnoContract::Future`/`Option`, `FnoExpiry::Monthly`/`Weekly`,
`OptionType::Call`/`Put`. Weekly month codes are `1..9` (Jan–Sep), `O` (Oct),
`N` (Nov), `D` (Dec). `default_fno_lot_size` returns the standard index lots
(NIFTY 25, BANKNIFTY 15, FINNIFTY 25, MIDCPNIFTY 50). `PriceParams::from_fno_contract`
in `vyapaar-market-pricing` consumes this grammar.

## Costs

* `SttRates` — Securities Transaction Tax rates per segment (delivery,
  intraday, futures, options).
* `CostModel` / `CostBreakdown` — itemized `stt`, `exchange_fee`, `gst`,
  `stamp_duty`, `sebi_fee`, `brokerage`.
* `NseCashEquitySchedule` — delivery/intraday NSE schedule (STT sell 0.10%,
  stamp buy 0.015%, exchange 0.00297%, SEBI 0.0001%, IPFT 0.0001%, brokerage
  0.03% capped at ₹20, GST 18%), effective `2025-04-01`.

## Calendar

`NseCalendar` is built from an explicit holiday set (via `HolidaySource` or a
literal `HashSet<NaiveDate>`), session 09:15–15:30 IST. No holiday data is
hardcoded in the crate — every value is injected.

## Zero Floats Policy

`rules.rs` and everything crossing to domain crates uses exact
`vyapaar_core::Price`/`Quantity`. The `india/costs` charge amounts are `f64`
fractions of notional (statutory rate tables), matching the Python reference
implementation; they are always rounded to exact `Decimal` before being
booked by a consumer.

## Related

* `code-pricing.md` — `PriceParams::from_fno_contract` turns a parsed `FnoContract` into pricing input.
* `vyapaar-core` — `Price`, `Quantity`, `InstrumentId`.
