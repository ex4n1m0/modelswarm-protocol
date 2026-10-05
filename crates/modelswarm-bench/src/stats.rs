//! Deterministic statistics machinery for the harness: xorshift RNG,
//! linear-interpolation percentiles, [`CellSummary`] aggregation, and
//! bootstrap confidence intervals (bench-harness-spec: median + p10/p90 +
//! IQR, comparisons via bootstrap 95% CIs with 10 000 resamples).

use serde::Serialize;

/// Deterministic xorshift64* RNG (no external rand dependency).
#[derive(Debug, Clone)]
pub struct Xorshift {
    state: u64,
}

impl Xorshift {
    /// Seeded RNG; a zero seed is remixed so the stream is never degenerate.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[-amplitude, +amplitude]`.
    pub fn next_symmetric(&mut self, amplitude: f64) -> f64 {
        (self.next_f64() * 2.0 - 1.0) * amplitude
    }
}

/// Linear-interpolation percentile on a sorted copy (`p` in `0..=100`).
pub fn percentile(values: &[f64], p: f64) -> f64 {
    assert!(!values.is_empty(), "percentile of empty sample");
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    percentile_sorted(&sorted, p)
}

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let position = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(sorted.len() - 1);
    let fraction = position - lower as f64;
    sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
}

/// Aggregated view of one cell's completion times (bench-harness-spec:
/// median, p10/p90, IQR computed as p75 − p25).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CellSummary {
    /// Median (p50) in milliseconds.
    pub median: f64,
    /// 10th percentile in milliseconds.
    pub p10: f64,
    /// 90th percentile in milliseconds.
    pub p90: f64,
    /// Interquartile range (p75 − p25) in milliseconds.
    pub iqr: f64,
    /// Sample size.
    pub n: usize,
}

/// Median, p10, p90, and IQR (p75 − p25) of `values`.
pub fn aggregate(values: &[f64]) -> CellSummary {
    assert!(!values.is_empty(), "aggregate of empty sample");
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    CellSummary {
        median: percentile_sorted(&sorted, 50.0),
        p10: percentile_sorted(&sorted, 10.0),
        p90: percentile_sorted(&sorted, 90.0),
        iqr: percentile_sorted(&sorted, 75.0) - percentile_sorted(&sorted, 25.0),
        n: sorted.len(),
    }
}

/// Bootstrap 95% confidence interval of the **median**, resampling
/// `resamples` times with the seeded RNG (deterministic). Returns
/// `(lo, hi)` at the 2.5th/97.5th percentiles of the bootstrap medians.
pub fn bootstrap_ci(values: &[f64], resamples: usize, seed: u64) -> (f64, f64) {
    assert!(!values.is_empty(), "bootstrap of empty sample");
    let mut rng = Xorshift::new(seed);
    let mut medians: Vec<f64> = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut sample: Vec<f64> = Vec::with_capacity(values.len());
        for _ in 0..values.len() {
            let index = (rng.next_u64() % values.len() as u64) as usize;
            sample.push(values[index]);
        }
        sample.sort_by(|a, b| a.total_cmp(b));
        medians.push(percentile_sorted(&sample, 50.0));
    }
    medians.sort_by(|a, b| a.total_cmp(b));
    (
        percentile_sorted(&medians, 2.5),
        percentile_sorted(&medians, 97.5),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_known_array() {
        let summary = aggregate(&[5.0, 1.0, 3.0, 2.0, 4.0]);
        assert_eq!(summary.n, 5);
        assert!((summary.median - 3.0).abs() < 1e-12);
        assert!((summary.p10 - 1.4).abs() < 1e-12);
        assert!((summary.p90 - 4.6).abs() < 1e-12);
    }

    #[test]
    fn even_sample_median_interpolates() {
        let summary = aggregate(&[1.0, 2.0, 3.0, 4.0]);
        assert!((summary.median - 2.5).abs() < 1e-12);
    }

    #[test]
    fn bootstrap_ci_brackets_the_median_and_is_deterministic() {
        let values: Vec<f64> = (1..=21).map(f64::from).collect();
        let (lo_a, hi_a) = bootstrap_ci(&values, 10_000, 42);
        let (lo_b, hi_b) = bootstrap_ci(&values, 10_000, 42);
        assert_eq!((lo_a, hi_a), (lo_b, hi_b), "same seed ⇒ same CI");
        let median = aggregate(&values).median;
        assert!(
            lo_a <= median && median <= hi_a,
            "({lo_a}, {hi_a}) vs {median}"
        );
        assert!(lo_a < hi_a);
    }

    #[test]
    fn bootstrap_ci_of_identical_values_is_degenerate() {
        let (lo, hi) = bootstrap_ci(&[7.0; 30], 1_000, 1);
        assert!((lo - 7.0).abs() < 1e-12 && (hi - 7.0).abs() < 1e-12);
    }

    #[test]
    fn xorshift_streams_differ_by_seed_and_repeat_by_seed() {
        let mut a = Xorshift::new(1);
        let mut b = Xorshift::new(1);
        let mut c = Xorshift::new(2);
        let seq_a: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        let seq_c: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(seq_a, seq_b);
        assert_ne!(seq_a, seq_c);
        assert!((0..1000).all(|_| {
            let v = a.next_f64();
            (0.0..1.0).contains(&v)
        }));
    }
}
