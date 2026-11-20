//! Primitive 4: Entry signal detection filters (`ZoneTouchEntryFilter`, `RejectionWickFilter`, `OrbBreakoutEntryFilter`).

use rust_decimal::Decimal;

use super::types::{
    EntrySignal, EntrySignalPrimitive, IndicatorSet, SignalDirection, StudyBar,
    ZoneExtractionOutput, ZonePolarity,
};

fn effective_atr(
    indicators: &IndicatorSet,
    idx: usize,
    close: Decimal,
    fallback: Decimal,
) -> Decimal {
    indicators
        .atr
        .get(idx)
        .copied()
        .unwrap_or(fallback)
        .max(close * Decimal::new(1, 3))
        .max(Decimal::new(1, 2))
}

/// Raw zone-touch entry filter (exact parity with `stop-hunt-buffer-study.ts` zone touch detection).
#[derive(Clone, Copy, Debug, Default)]
pub struct ZoneTouchEntryFilter;

impl EntrySignalPrimitive for ZoneTouchEntryFilter {
    fn id(&self) -> &'static str {
        "zone_touch_close"
    }

    fn detect_signals(
        &self,
        bars: &[StudyBar],
        zones: &ZoneExtractionOutput,
        indicators: &IndicatorSet,
    ) -> Vec<EntrySignal> {
        let mut signals = Vec::new();
        let ten_pct = Decimal::new(1, 1); // 0.1
        let fifteen_pct = Decimal::new(15, 2); // 0.15
        let wick_ratio_req = Decimal::new(85, 2); // 0.85

        for sess in &zones.sessions {
            let max_i = sess.slice.end_index.min(bars.len().saturating_sub(3));
            if sess.slice.start_index + 1 > max_i {
                continue;
            }
            for (i, b) in bars
                .iter()
                .enumerate()
                .take(max_i + 1)
                .skip(sess.slice.start_index + 1)
            {
                let rng = b.high - b.low;
                if rng <= Decimal::ZERO {
                    continue;
                }
                let atr = effective_atr(indicators, i, b.close, sess.atr_at_open);
                let body = (b.close - b.open).abs();
                let lower_wick = b.open.min(b.close) - b.low;
                let upper_wick = b.high - b.open.max(b.close);

                for band in &sess.bands {
                    match band.polarity {
                        ZonePolarity::Support => {
                            if b.low <= band.high
                                && b.close >= band.low - ten_pct * atr
                                && b.close >= b.open - fifteen_pct * rng
                            {
                                let is_sweep_reclaim = b.low < band.low
                                    && b.close >= band.low
                                    && lower_wick >= body * wick_ratio_req;
                                signals.push(EntrySignal {
                                    bar_index: i,
                                    direction: SignalDirection::Long,
                                    entry_price: b.close,
                                    band_label: band.label.clone(),
                                    band_center: band.center,
                                    band_low: band.low,
                                    band_high: band.high,
                                    atr,
                                    is_sweep_reclaim,
                                    hint: format!(
                                        "Support reclaim at {} [{:.2}-{:.2}]",
                                        band.label, band.low, band.high
                                    ),
                                });
                                break;
                            }
                        }
                        ZonePolarity::Resistance => {
                            if b.high >= band.low
                                && b.close <= band.high + ten_pct * atr
                                && b.close <= b.open + fifteen_pct * rng
                            {
                                let is_sweep_reclaim = b.high > band.high
                                    && b.close <= band.high
                                    && upper_wick >= body * wick_ratio_req;
                                signals.push(EntrySignal {
                                    bar_index: i,
                                    direction: SignalDirection::Short,
                                    entry_price: b.close,
                                    band_label: band.label.clone(),
                                    band_center: band.center,
                                    band_low: band.low,
                                    band_high: band.high,
                                    atr,
                                    is_sweep_reclaim,
                                    hint: format!(
                                        "Resistance rejection at {} [{:.2}-{:.2}]",
                                        band.label, band.low, band.high
                                    ),
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
        signals
    }
}

/// Selective rejection-wick entry filter requiring strong wick-to-body reclamation inside or
/// sweeping past the zone band (strict subset of `ZoneTouchEntryFilter`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RejectionWickFilter {
    /// Minimum `wick / body` multiple required to trigger entry (default `0.85`).
    pub min_wick_to_body: Decimal,
}

impl Default for RejectionWickFilter {
    fn default() -> Self {
        Self {
            min_wick_to_body: Decimal::new(85, 2),
        }
    }
}

impl EntrySignalPrimitive for RejectionWickFilter {
    fn id(&self) -> &'static str {
        "rejection_wick"
    }

    fn detect_signals(
        &self,
        bars: &[StudyBar],
        zones: &ZoneExtractionOutput,
        indicators: &IndicatorSet,
    ) -> Vec<EntrySignal> {
        let raw_touches = ZoneTouchEntryFilter.detect_signals(bars, zones, indicators);
        raw_touches
            .into_iter()
            .filter(|sig| {
                let b = &bars[sig.bar_index];
                let body = (b.close - b.open).abs();
                match sig.direction {
                    SignalDirection::Long => {
                        let lower_wick = b.open.min(b.close) - b.low;
                        b.close >= sig.band_low && lower_wick >= body * self.min_wick_to_body
                    }
                    SignalDirection::Short => {
                        let upper_wick = b.high - b.open.max(b.close);
                        b.close <= sig.band_high && upper_wick >= body * self.min_wick_to_body
                    }
                }
            })
            .collect()
    }
}

/// Opening-Range Breakout (ORB-15) signal filter compatible with `Orb15Extractor` bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrbBreakoutEntryFilter;

impl EntrySignalPrimitive for OrbBreakoutEntryFilter {
    fn id(&self) -> &'static str {
        "orb_breakout"
    }

    fn detect_signals(
        &self,
        bars: &[StudyBar],
        zones: &ZoneExtractionOutput,
        indicators: &IndicatorSet,
    ) -> Vec<EntrySignal> {
        let mut signals = Vec::new();
        for sess in &zones.sessions {
            let max_i = sess.slice.end_index.min(bars.len().saturating_sub(2));
            if sess.slice.start_index + 1 > max_i {
                continue;
            }
            let or_high = sess
                .bands
                .iter()
                .find(|b| b.label == "OR_HIGH")
                .or_else(|| {
                    sess.bands
                        .iter()
                        .find(|b| b.polarity == ZonePolarity::Resistance)
                });
            let or_low = sess.bands.iter().find(|b| b.label == "OR_LOW").or_else(|| {
                sess.bands
                    .iter()
                    .find(|b| b.polarity == ZonePolarity::Support)
            });

            let (Some(hi_band), Some(lo_band)) = (or_high, or_low) else {
                continue;
            };

            let mut flips = 0usize;
            let mut active_dir: Option<SignalDirection> = None;

            for (i, b) in bars
                .iter()
                .enumerate()
                .take(max_i + 1)
                .skip(sess.slice.start_index + 1)
            {
                if flips >= 3 {
                    break;
                }
                let atr = effective_atr(indicators, i, b.close, sess.atr_at_open);
                if b.close > hi_band.center && active_dir != Some(SignalDirection::Long) {
                    active_dir = Some(SignalDirection::Long);
                    flips += 1;
                    signals.push(EntrySignal {
                        bar_index: i,
                        direction: SignalDirection::Long,
                        entry_price: b.close,
                        band_label: hi_band.label.clone(),
                        band_center: hi_band.center,
                        band_low: lo_band.low,
                        band_high: hi_band.high,
                        atr,
                        is_sweep_reclaim: false,
                        hint: format!("ORB upside breakout above {:.2}", hi_band.center),
                    });
                } else if b.close < lo_band.center && active_dir != Some(SignalDirection::Short) {
                    active_dir = Some(SignalDirection::Short);
                    flips += 1;
                    signals.push(EntrySignal {
                        bar_index: i,
                        direction: SignalDirection::Short,
                        entry_price: b.close,
                        band_label: lo_band.label.clone(),
                        band_center: lo_band.center,
                        band_low: lo_band.low,
                        band_high: hi_band.high,
                        atr,
                        is_sweep_reclaim: false,
                        hint: format!("ORB downside breakdown below {:.2}", lo_band.center),
                    });
                }
            }
        }
        signals
    }
}
