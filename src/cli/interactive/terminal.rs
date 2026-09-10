//! Shared terminal cleanup for CLI interactive screens.

use std::io;

use crossterm::cursor::Show;
use crossterm::execute;
use crossterm::terminal::{self, LeaveAlternateScreen};

pub(crate) struct InteractiveTerminalGuard;

impl Drop for InteractiveTerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
