//! Primitive 2: Zone extractors (`ElephantZonesExtractor`, `Orb15Extractor`).

use rust_decimal::Decimal;

use super::types::{
    ExtractedLevelLine, ExtractedZoneBand, IndicatorSet, SessionSlice, SessionZoneMap, StudyBar,
    ZoneExtractionOutput, ZoneExtractor, ZonePolarity,
};

/// Nearest-rank empirical percentile (`sorted_asc` non-empty, `pct` in `0..=100`).
pub(crate) fn nearest_rank_percentile(sorted_asc: &[Decimal], pct: u32) -> Decimal {
    if sorted_asc.is_empty() {
        return Decimal::ZERO;
    }
    let n = sorted_asc.len();
    let p = pct.min(100) as usize;
    let rank = (p * n).div_ceil(100).clamp(1, n);
    sorted_asc[rank - 1]
}

/// Elephant Edge empirical percentile + golden-ratio fallback zone extractor.
///
/// Matches the benchmark logic in `elephant-edge-overlay.ts` (`S1/S2/R1/R2`, dotted
/// equilibrium pivot blend, automatic polarity flip across session open, and outer-edge
/// `+k * ATR` protective stop buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElephantZonesExtractor {
    /// Additional stop buffer multiplier beyond outer band edge (default `0.25 ATR`).
    pub sl_buffer_atr: Decimal,
    /// Inner support quantile (default `25`).
    pub s1_pct: u32,
    /// Outer support quantile (default `35`).
    pub s2_pct: u32,
    /// Inner resistance quantile (default `63`).
    pub r1_pct: u32,
    /// Outer resistance quantile (default `74`).
    pub r2_pct: u32,
}

impl Default for ElephantZonesExtractor {
    fn default() -> Self {
        Self {
            sl_buffer_atr: Decimal::new(25, 2),
            s1_pct: 25,
            s2_pct: 35,
            r1_pct: 63,
            r2_pct: 74,
        }
    }
}

impl ElephantZonesExtractor {
    /// Creates an extractor overridden with a custom outer SL ATR buffer.
    pub fn with_sl_buffer_atr(mut self, sl_buffer_atr: Decimal) -> Self {
        self.sl_buffer_atr = sl_buffer_atr.max(Decimal::ZERO);
        self
    }
}

impl ZoneExtractor for ElephantZonesExtractor {
    fn id(&self) -> &'static str {
        "elephant_zones"
    }

    fn extract(
        &self,
        bars: &[StudyBar],
        sessions: &[SessionSlice],
        indicators: &IndicatorSet,
    ) -> ZoneExtractionOutput {
        if bars.is_empty() || sessions.is_empty() {
            return ZoneExtractionOutput::default();
        }

        let phi1 = Decimal::new(1618, 3);
        let phi2 = Decimal::new(2618, 3);
        let half = Decimal::new(5, 1);
        let three = Decimal::from(3);
        let min_floor = Decimal::new(5, 2); // 0.05
        let eps = Decimal::new(1, 2); // 0.01

        let mut completed_downs: Vec<Decimal> = Vec::new();
        let mut completed_ups: Vec<Decimal> = Vec::new();
        let mut prev_typical_pivot: Option<Decimal> = None;

        let mut out_sessions = Vec::with_capacity(sessions.len());
        let mut out_bands = Vec::with_capacity(sessions.len() * 4);
        let mut out_levels = Vec::with_capacity(sessions.len());

        for sess in sessions {
            if sess.start_index >= bars.len() || sess.end_index >= bars.len() {
                continue;
            }
            let first = &bars[sess.start_index];
            let session_open = first.open;
            let anchor_close = first.close;
            let first_range = (first.high - first.low).max(Decimal::new(1, 4));
            let atr_raw = indicators
                .atr
                .get(sess.start_index)
                .copied()
                .unwrap_or(first_range);
            let atr_at_open = atr_raw
                .max(session_open * Decimal::new(1, 3))
                .max(eps)
                .round_dp(2);

            let equilibrium = match prev_typical_pivot {
                Some(prev) => (half * anchor_close + half * prev).round_dp(2),
                None => anchor_close.round_dp(2),
            };

            let half_width = (Decimal::new(35, 2) * atr_at_open)
                .max(Decimal::new(35, 2) * first_range)
                .max(session_open * Decimal::new(12, 4))
                .max(eps)
                .round_dp(2);

            let sl_buffer = (self.sl_buffer_atr * atr_at_open)
                .max(session_open * Decimal::new(5, 4))
                .max(eps)
                .round_dp(2);

            let (s1_center, s2_center, r1_center, r2_center) =
                if completed_downs.len() >= 2 && completed_ups.len() >= 2 {
                    let mut sorted_downs = completed_downs.clone();
                    let mut sorted_ups = completed_ups.clone();
                    sorted_downs.sort();
                    sorted_ups.sort();

                    let d1 = nearest_rank_percentile(&sorted_downs, self.s1_pct);
                    let d2 = nearest_rank_percentile(&sorted_downs, self.s2_pct);
                    let u1 = nearest_rank_percentile(&sorted_ups, self.r1_pct);
                    let u2 = nearest_rank_percentile(&sorted_ups, self.r2_pct);

                    let min_step =
                        (half_width * Decimal::new(135, 2)).max(atr_at_open * Decimal::new(65, 2));
                    let s1 = equilibrium - d1.max(atr_at_open * half);
                    let s2 = (equilibrium - d2.max(atr_at_open)).min(s1 - min_step);
                    let r1 = equilibrium + u1.max(atr_at_open * half);
                    let r2 = (equilibrium + u2.max(atr_at_open)).max(r1 + min_step);
                    (s1, s2, r1, r2)
                } else {
                    let eff_range = first_range.max(atr_at_open);
                    (
                        equilibrium - phi1 * eff_range,
                        equilibrium - phi2 * eff_range,
                        equilibrium + phi1 * eff_range,
                        equilibrium + phi2 * eff_range,
                    )
                };

            let raw_defs = [
                ("R2", r2_center, ZonePolarity::Resistance),
                ("R1", r1_center, ZonePolarity::Resistance),
                ("S1", s1_center, ZonePolarity::Support),
                ("S2", s2_center, ZonePolarity::Support),
            ];

            let mut bands = Vec::with_capacity(4);
            for (label, center, base_pol) in raw_defs {
                let low = (center - half_width).max(min_floor).round_dp(2);
                let high = (center + half_width).max(low + eps).round_dp(2);
                let polarity = if high <= session_open {
                    ZonePolarity::Support
                } else if low >= session_open {
                    ZonePolarity::Resistance
                } else {
                    base_pol
                };
                let outer_sl = match polarity {
                    ZonePolarity::Support => {
                        let candidate = (low - sl_buffer).round_dp(2);
                        if low > min_floor {
                            candidate.max(min_floor).min(low - eps)
                        } else {
                            (low - eps).max(Decimal::new(1, 2))
                        }
                    }
                    ZonePolarity::Resistance => (high + sl_buffer).round_dp(2),
                };

                let band = ExtractedZoneBand {
                    id: format!("{}:{}", sess.session_key, label),
                    label: label.to_string(),
                    polarity,
                    center: center.round_dp(2),
                    low,
                    high,
                    outer_sl,
                    start_index: sess.start_index,
                    end_index: sess.end_index,
                };
                out_bands.push(band.clone());
                bands.push(band);
            }

            out_levels.push(ExtractedLevelLine {
                session_key: sess.session_key.clone(),
                label: "Equilibrium".to_string(),
                price: equilibrium,
                start_index: sess.start_index,
                end_index: sess.end_index,
            });

            out_sessions.push(SessionZoneMap {
                slice: sess.clone(),
                equilibrium,
                atr_at_open,
                sl_buffer,
                bands,
            });

            // Roll completed session excursions into history for next session
            let mut sess_high = bars[sess.start_index].high;
            let mut sess_low = bars[sess.start_index].low;
            for b in bars.iter().take(sess.end_index + 1).skip(sess.start_index) {
                if b.high > sess_high {
                    sess_high = b.high;
                }
                if b.low < sess_low {
                    sess_low = b.low;
                }
            }
            let sess_close = bars[sess.end_index].close;
            completed_downs.push((session_open - sess_low).max(Decimal::ZERO));
            completed_ups.push((sess_high - session_open).max(Decimal::ZERO));
            prev_typical_pivot = Some((sess_high + sess_low + sess_close) / three);
        }

        ZoneExtractionOutput {
            sessions: out_sessions,
            bands: out_bands,
            levels: out_levels,
        }
    }
}

/// 15-minute Opening Range Breakout zone extractor (`OR_HIGH`, `OR_LOW`, & OR midpoint).
///
/// Proves multi-strategy reusability over the exact same `ZoneExtractor` / `StrategyLoopKernel`
/// / `StopExitPolicy` pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orb15Extractor {
    /// Opening range window length in minutes (default `15`).
    pub opening_range_minutes: u16,
    /// Outer stop buffer multiplier in ATR (default `0.25`).
    pub sl_buffer_atr: Decimal,
}

impl Default for Orb15Extractor {
    fn default() -> Self {
        Self {
            opening_range_minutes: 15,
            sl_buffer_atr: Decimal::new(25, 2),
        }
    }
}

impl ZoneExtractor for Orb15Extractor {
    fn id(&self) -> &'static str {
        "orb_15"
    }

    fn extract(
        &self,
        bars: &[StudyBar],
        sessions: &[SessionSlice],
        indicators: &IndicatorSet,
    ) -> ZoneExtractionOutput {
        if bars.is_empty() || sessions.is_empty() {
            return ZoneExtractionOutput::default();
        }

        let two = Decimal::from(2);
        let min_floor = Decimal::new(5, 2);
        let eps = Decimal::new(1, 2);

        let mut out_sessions = Vec::with_capacity(sessions.len());
        let mut out_bands = Vec::with_capacity(sessions.len() * 2);
        let mut out_levels = Vec::with_capacity(sessions.len());

        for sess in sessions {
            if sess.start_index >= bars.len() || sess.end_index >= bars.len() {
                continue;
            }
            let start_bar = &bars[sess.start_index];
            let start_min = start_bar.minute_of_day.unwrap_or(555);
            let cutoff_min = start_min.saturating_add(self.opening_range_minutes);

            let mut or_high = start_bar.high;
            let mut or_low = start_bar.low;
            for b in bars.iter().take(sess.end_index + 1).skip(sess.start_index) {
                if let Some(m) = b.minute_of_day {
                    if m > cutoff_min {
                        break;
                    }
                } else {
                    break;
                }
                if b.high > or_high {
                    or_high = b.high;
                }
                if b.low < or_low {
                    or_low = b.low;
                }
                if b.minute_of_day.is_some() && b.minute_of_day == Some(start_min) {
                    continue;
                }
            }

            let atr_at_open = indicators
                .atr
                .get(sess.start_index)
                .copied()
                .unwrap_or(or_high - or_low)
                .max(start_bar.open * Decimal::new(1, 3))
                .max(eps)
                .round_dp(2);

            let half_width = (atr_at_open * Decimal::new(15, 2)).max(eps).round_dp(2);
            let sl_buffer = (self.sl_buffer_atr * atr_at_open).max(eps).round_dp(2);
            let eq = ((or_high + or_low) / two).round_dp(2);

            let or_high_band = ExtractedZoneBand {
                id: format!("{}:OR_HIGH", sess.session_key),
                label: "OR_HIGH".to_string(),
                polarity: ZonePolarity::Resistance,
                center: or_high.round_dp(2),
                low: (or_high - half_width).max(min_floor).round_dp(2),
                high: (or_high + half_width).max(or_high).round_dp(2),
                outer_sl: (or_high + half_width + sl_buffer).round_dp(2),
                start_index: sess.start_index,
                end_index: sess.end_index,
            };

            let or_low_low = (or_low - half_width).max(min_floor).round_dp(2);
            let or_low_band = ExtractedZoneBand {
                id: format!("{}:OR_LOW", sess.session_key),
                label: "OR_LOW".to_string(),
                polarity: ZonePolarity::Support,
                center: or_low.round_dp(2),
                low: or_low_low,
                high: (or_low + half_width).max(or_low_low + eps).round_dp(2),
                outer_sl: (or_low_low - sl_buffer)
                    .max(min_floor)
                    .min((or_low_low - eps).max(eps)),
                start_index: sess.start_index,
                end_index: sess.end_index,
            };

            out_bands.push(or_high_band.clone());
            out_bands.push(or_low_band.clone());

            out_levels.push(ExtractedLevelLine {
                session_key: sess.session_key.clone(),
                label: "OR Mid".to_string(),
                price: eq,
                start_index: sess.start_index,
                end_index: sess.end_index,
            });

            out_sessions.push(SessionZoneMap {
                slice: sess.clone(),
                equilibrium: eq,
                atr_at_open,
                sl_buffer,
                bands: vec![or_high_band, or_low_band],
            });
        }

        ZoneExtractionOutput {
            sessions: out_sessions,
            bands: out_bands,
            levels: out_levels,
        }
    }
}
