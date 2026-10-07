//! Settings over the store's saved break length for human time. Whole
//! minutes cross IPC as a plain integer; the store owns the bounds, the
//! default and the committed read-back. The Dashboard reads the same value.
use crate::state::{AppState, StateError};
use xt_store::human_break::HumanBreak;

/// The break length the Dashboard's human time read.
pub(crate) fn break_length(saved: HumanBreak) -> Result<xt_metrics::BreakLength, StateError> {
    Ok(xt_metrics::BreakLength::new(saved.minutes())?)
}

impl AppState {
    /// The saved break length in minutes; 60 when nothing was saved.
    pub fn human_break(&self) -> Result<u32, StateError> {
        self.with_store(|store| Ok(store.human_break()?.minutes()))
    }

    /// Validates before any write and returns the length read back after the
    /// commit. A refused or failed write leaves the saved length unchanged.
    pub fn set_human_break(&self, minutes: u32) -> Result<u32, StateError> {
        let length = HumanBreak::new(minutes).map_err(|_| StateError::InvalidHumanBreak)?;
        self.with_store(|store| Ok(store.set_human_break(length)?.minutes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_minutes_are_the_metric_break_length() {
        for minutes in [5, 60, 240] {
            let length = break_length(HumanBreak::new(minutes).unwrap()).unwrap();
            assert_eq!(length.minutes(), minutes);
        }
        assert_eq!(
            break_length(HumanBreak::default()).unwrap(),
            xt_metrics::BreakLength::default(),
            "the store's default is the metric's default"
        );
    }
}
