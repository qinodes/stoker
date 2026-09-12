use std::fmt::Display;
use std::io::{self, IsTerminal};

use crossterm::style::{Color, Stylize};

use crate::domain::JobState;

/// Returns whether colors should be emitted for a stream with the supplied
/// terminal properties.
///
/// Keeping this decision separate from the actual styling makes the behavior
/// deterministic in tests and prevents ANSI escape sequences from leaking into
/// redirected output or scripts.
fn colors_enabled(is_terminal: bool, no_color: bool, dumb_terminal: bool) -> bool {
    is_terminal && !no_color && !dumb_terminal
}

pub fn stdout_color_enabled() -> bool {
    colors_enabled(
        io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        dumb_terminal(),
    )
}

pub fn stderr_color_enabled() -> bool {
    colors_enabled(
        io::stderr().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
        dumb_terminal(),
    )
}

fn dumb_terminal() -> bool {
    std::env::var("TERM")
        .map(|value| value.eq_ignore_ascii_case("dumb"))
        .unwrap_or(false)
}

pub fn paint<T: Display>(value: T, color: Color, enabled: bool) -> String {
    let value = value.to_string();
    if enabled {
        value.with(color).to_string()
    } else {
        value
    }
}

pub fn paint_bold<T: Display>(value: T, color: Color, enabled: bool) -> String {
    let value = value.to_string();
    if enabled {
        value.with(color).bold().to_string()
    } else {
        value
    }
}

pub fn state_color(state: JobState) -> Color {
    match state {
        JobState::Draft => Color::DarkGrey,
        JobState::Queued => Color::Cyan,
        JobState::Starting | JobState::Running => Color::Yellow,
        JobState::Cancelling => Color::Magenta,
        JobState::Succeeded => Color::Green,
        JobState::Failed | JobState::Lost => Color::Red,
        JobState::Cancelled => Color::Magenta,
    }
}

pub fn paint_state(state: JobState, enabled: bool) -> String {
    paint(state, state_color(state), enabled)
}

#[cfg(test)]
mod tests {
    use super::{
        colors_enabled, paint, paint_bold, paint_state, state_color, stderr_color_enabled,
    };
    use crate::JobState;
    use crossterm::style::Color;

    #[test]
    fn colors_require_a_real_terminal_and_supported_terminal_mode() {
        assert!(colors_enabled(true, false, false));
        assert!(!colors_enabled(false, false, false));
        assert!(!colors_enabled(true, true, false));
        assert!(!colors_enabled(true, false, true));
    }

    #[test]
    fn disabled_colors_preserve_plain_text() {
        assert_eq!(paint("ok", Color::Green, false), "ok");
        assert_eq!(paint_bold("title", Color::Cyan, false), "title");
        assert_eq!(paint_state(JobState::Failed, false), "FAILED");
    }

    #[test]
    fn enabled_colors_emit_style_sequences() {
        assert!(paint("ok", Color::Green, true).contains("\u{1b}["));
        assert!(paint_bold("title", Color::Cyan, true).contains("\u{1b}["));
        assert!(paint_state(JobState::Succeeded, true).contains("SUCCEEDED"));
    }

    #[test]
    fn state_colors_distinguish_outcomes_and_activity() {
        assert_eq!(state_color(JobState::Succeeded), Color::Green);
        assert_eq!(state_color(JobState::Failed), Color::Red);
        assert_eq!(state_color(JobState::Running), Color::Yellow);
        assert_eq!(state_color(JobState::Starting), Color::Yellow);
        assert_eq!(state_color(JobState::Cancelling), Color::Magenta);
        assert_eq!(state_color(JobState::Cancelled), Color::Magenta);
    }

    #[test]
    fn stderr_color_detection_is_safe_when_output_is_redirected() {
        let _ = stderr_color_enabled();
        assert_eq!(paint_state(JobState::Cancelling, false), "CANCELLING");
    }
}
