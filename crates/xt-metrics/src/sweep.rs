use crate::{ActiveSpan, Error, MetricsDb, Result, Window};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Concurrency {
    pub max: Option<u32>,
    pub mean: Option<f64>,
    pub wall_active_ms: u64,
}

impl MetricsDb {
    /// Sweep the existing in-window spans on their POSIX millisecond axis.
    /// Mean concurrency is weighted by occupied wall time, excluding idle gaps.
    /// Without positive-duration spans, both max and mean are unknown.
    pub fn concurrency(&self, window: Window) -> Result<Concurrency> {
        sweep(&self.active_spans(window)?.spans)
    }
}

fn sweep(spans: &[ActiveSpan]) -> Result<Concurrency> {
    let mut points = Vec::new();
    for span in spans.iter().filter(|s| s.start_ms < s.end_ms) {
        points.push((span.start_ms, 1_i8));
        points.push((span.end_ms, -1_i8));
    }
    // End before start: touching half-open spans never inflate the peak.
    points.sort_unstable();
    let mut previous = points.first().map_or(0, |p| p.0);
    let mut occupancy = 0_u32;
    let mut peak = 0;
    let mut wall_active_ms = 0_u64;
    let mut weighted_ms = 0_u64;
    for (time, delta) in points {
        if occupancy > 0 {
            let elapsed = time.checked_sub(previous).ok_or(Error::CounterOverflow)? as u64;
            wall_active_ms = wall_active_ms
                .checked_add(elapsed)
                .ok_or(Error::CounterOverflow)?;
            weighted_ms = weighted_ms
                .checked_add(
                    elapsed
                        .checked_mul(u64::from(occupancy))
                        .ok_or(Error::CounterOverflow)?,
                )
                .ok_or(Error::CounterOverflow)?;
        }
        occupancy = if delta < 0 {
            occupancy.checked_sub(1)
        } else {
            occupancy.checked_add(1)
        }
        .ok_or(Error::CounterOverflow)?;
        peak = peak.max(occupancy);
        previous = time;
    }
    Ok(Concurrency {
        max: (wall_active_ms > 0).then_some(peak),
        mean: (wall_active_ms > 0).then(|| weighted_ms as f64 / wall_active_ms as f64),
        wall_active_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(start_ms: i64, end_ms: i64) -> ActiveSpan {
        ActiveSpan {
            session_id: "test".into(),
            host: "claude".into(),
            start_ms,
            end_ms,
        }
    }
    #[test]
    fn sweep_touching_empty_and_zero_spans() {
        for spans in [vec![], vec![span(1, 1), span(2, 2)]] {
            assert_eq!(
                sweep(&spans).unwrap(),
                Concurrency {
                    max: None,
                    mean: None,
                    wall_active_ms: 0
                }
            );
        }
        let mut spans = vec![
            span(0, 10),
            span(10, 20),
            span(5, 5),
            span(10, 10),
            span(40, 50),
        ];
        for _ in 0..2 {
            assert_eq!(
                sweep(&spans).unwrap(),
                Concurrency {
                    max: Some(1),
                    mean: Some(1.0),
                    wall_active_ms: 30
                }
            );
            spans.reverse();
        }
    }
    #[test]
    fn sweep_checked_duration_overflow() {
        assert!(matches!(
            sweep(&[span(i64::MIN, i64::MAX)]),
            Err(Error::CounterOverflow)
        ));
        assert!(matches!(
            sweep(&[span(0, i64::MAX), span(0, i64::MAX), span(0, i64::MAX)]),
            Err(Error::CounterOverflow)
        ));
    }
}
