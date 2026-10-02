/// Median and the probe's floor(0.9*n), capped at n-1, over measured durations.
/// Sorts the caller's bounded sample in place; no interpolation for p90.
pub(crate) fn median_p90(values: &mut [u64]) -> Option<(f64, u64)> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    let median = if values.len().is_multiple_of(2) {
        values[middle - 1] as f64 / 2.0 + values[middle] as f64 / 2.0
    } else {
        values[middle] as f64
    };
    // n - ceil(n/10) equals floor(9*n/10), without multiplying n.
    let p90 = values[values.len() - values.len().div_ceil(10)];
    Some((median, p90))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_median_and_probe_p90_boundaries() {
        assert_eq!(median_p90(&mut []), None);
        assert_eq!(median_p90(&mut [7]), Some((7.0, 7)));
        assert_eq!(median_p90(&mut [9, 1]), Some((5.0, 9)));
        assert_eq!(median_p90(&mut [9, 1, 4]), Some((4.0, 9)));
        assert_eq!(median_p90(&mut [4, 1, 3, 2]), Some((2.5, 4)));
        assert_eq!(
            median_p90(&mut (0..10).rev().collect::<Vec<_>>()),
            Some((4.5, 9))
        );
        assert_eq!(
            median_p90(&mut (0..11).rev().collect::<Vec<_>>()),
            Some((5.0, 9))
        );
        assert_eq!(
            median_p90(&mut [u64::MAX, u64::MAX]),
            Some((u64::MAX as f64, u64::MAX))
        );
    }
}
