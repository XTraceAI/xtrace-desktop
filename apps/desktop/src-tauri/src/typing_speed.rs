//! Settings over the store's saved typing speed. Whole words per minute cross
//! IPC as a plain integer; the store owns the bounds, the default and the
//! committed read-back. The Dashboard reads the same saved value.
use crate::state::{AppState, StateError};
use xt_store::typing_speed::TypingSpeed;

/// The official Monkeytype test. A compiled constant: no URL crosses IPC.
pub const TYPING_TEST_URL: &str = "https://monkeytype.com/";

/// Hands the fixed test page to the system browser. Nothing is fetched,
/// measured or read back: the user types their own number into Settings.
pub fn open_typing_test() -> Result<(), StateError> {
    let status = typing_test_command()?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| StateError::TypingTestUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(StateError::TypingTestUnavailable)
    }
}

/// An absolute launcher path, so the search path cannot substitute another.
#[cfg(target_os = "macos")]
fn typing_test_command() -> Result<std::process::Command, StateError> {
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg(TYPING_TEST_URL);
    Ok(command)
}
#[cfg(not(target_os = "macos"))]
fn typing_test_command() -> Result<std::process::Command, StateError> {
    Err(StateError::TypingTestUnavailable)
}

/// The typing rate the M-15 estimate and the M-07 Human estimate read, at five
/// characters per word.
pub(crate) fn typing_rate(speed: TypingSpeed) -> Result<xt_metrics::TypingRate, StateError> {
    Ok(xt_metrics::TypingRate::new(speed.characters_per_minute())?)
}

impl AppState {
    /// The saved speed in words per minute; 40 when nothing was saved.
    pub fn typing_speed(&self) -> Result<u32, StateError> {
        self.with_store(|store| Ok(store.typing_speed()?.wpm()))
    }

    /// Validates before any write and returns the speed read back after the
    /// commit. A refused or failed write leaves the saved speed unchanged.
    pub fn set_typing_speed(&self, wpm: u32) -> Result<u32, StateError> {
        let speed = TypingSpeed::new(wpm).map_err(|_| StateError::InvalidTypingSpeed)?;
        self.with_store(|store| Ok(store.set_typing_speed(speed)?.wpm()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_convert_at_five_characters_each() {
        for (wpm, cpm) in [(1, 5), (40, 200), (80, 400), (300, 1500)] {
            let rate = typing_rate(TypingSpeed::new(wpm).unwrap()).unwrap();
            assert_eq!(rate.characters_per_minute(), cpm);
        }
        assert_eq!(
            typing_rate(TypingSpeed::default()).unwrap(),
            xt_metrics::TypingRate::default(),
            "the default speed is the existing default rate"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_test_opens_only_the_fixed_official_page() {
        let command = typing_test_command().unwrap();
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["https://monkeytype.com/"]);
    }
}
