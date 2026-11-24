//! Unit tests for `crate::cpr`.

mod common;
use common::{assert_dec_close, hlc};
use rust_decimal_macros::dec;
use tantra_market_indicators::{Cpr, Indicator};

#[test]
fn computes_exact_decimal_pivot_bc_tc_width() {
    let mut cpr = Cpr::new();
    // H=120, L=90, C=105 -> Pivot = 315/3 = 105, BC = 210/2 = 105, TC = 2*105 - 105 = 105
    let p = cpr.update(&hlc(120.0, 90.0, 105.0)).unwrap();
    assert_eq!(p, dec!(105));
    assert_eq!(cpr.pivot(), Some(dec!(105)));
    assert_eq!(cpr.bc(), Some(dec!(105)));
    assert_eq!(cpr.tc(), Some(dec!(105)));
    assert_eq!(cpr.width(), Some(dec!(0)));

    // H=130, L=100, C=120 -> Pivot = 350/3, BC = 115, TC = 700/3 - 115 = 355/3, width = 10/3
    let p2 = cpr.update(&hlc(130.0, 100.0, 120.0)).unwrap();
    assert_dec_close(p2, dec!(350) / dec!(3));
    assert_eq!(cpr.bc(), Some(dec!(115)));
    assert_dec_close(cpr.tc().unwrap(), dec!(355) / dec!(3));
    assert_dec_close(cpr.width().unwrap(), dec!(10) / dec!(3));
}

#[test]
fn reset_clears_all_cpr_levels() {
    let mut cpr = Cpr::new();
    cpr.update(&hlc(110.0, 90.0, 100.0));
    assert!(cpr.is_ready());
    cpr.reset();
    assert_eq!(cpr.value(), None);
    assert_eq!(cpr.pivot(), None);
    assert_eq!(cpr.bc(), None);
    assert_eq!(cpr.tc(), None);
    assert_eq!(cpr.width(), None);
}
