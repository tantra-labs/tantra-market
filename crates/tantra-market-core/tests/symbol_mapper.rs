//! Symbol mapper port tests (extracted from the inline `#[cfg(test)] mod
//! tests` of the source crate `honba-me-core/src/lib.rs`).

use tantra_market_core::{Exchange, InstrumentId, SymbolMapper};

#[test]
fn test_symbol_mapper_heuristics() {
    let mapper = SymbolMapper::new();

    // Pattern 1
    let inst1 = mapper.to_canonical("RELIANCE$NSE").unwrap();
    assert_eq!(inst1.symbol, "RELIANCE");
    assert_eq!(inst1.exchange, Exchange::Nse);

    // Pattern 2
    let inst2 = mapper.to_canonical("NSE:INFY-EQ").unwrap();
    assert_eq!(inst2.symbol, "INFY");
    assert_eq!(inst2.exchange, Exchange::Nse);

    // Pattern 3
    let inst3 = mapper.to_canonical("BSE_EQ|500325").unwrap();
    assert_eq!(inst3.symbol, "500325");
    assert_eq!(inst3.exchange, Exchange::Bse);
}

#[test]
fn test_symbol_mapper_explicit_registration() {
    let mut mapper = SymbolMapper::new();
    let inst = InstrumentId::new(Exchange::Nse, "TCS").with_token("11536");
    mapper.register("11536", inst.clone());

    assert_eq!(mapper.to_canonical("11536"), Some(inst.clone()));
    assert_eq!(mapper.to_broker_key(&inst), Some("11536"));
}
