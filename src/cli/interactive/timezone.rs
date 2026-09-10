//! Interactive timezone selection state, rendering, and orchestration.

use std::io::{self, Write};

use anyhow::Context;
use crossterm::cursor::{Hide, MoveTo};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen};

use super::terminal::InteractiveTerminalGuard;

#[derive(Debug, Clone)]
pub(crate) struct TimezoneSelectorState {
    pub(crate) timezones: Vec<String>,
    pub(crate) query: String,
    pub(crate) selected: usize,
}

impl TimezoneSelectorState {
    pub(crate) fn new(mut timezones: Vec<String>, current: Option<&str>) -> Self {
        timezones.sort();
        timezones.dedup();
        let selected = current
            .and_then(|value| timezones.iter().position(|timezone| timezone == value))
            .unwrap_or(0);
        Self {
            timezones,
            query: String::new(),
            selected,
        }
    }

    pub(crate) fn matches(&self) -> Vec<usize> {
        let query = self.query.to_ascii_lowercase();
        self.timezones
            .iter()
            .enumerate()
            .filter_map(|(index, timezone)| {
                if timezone.to_ascii_lowercase().contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn selected_timezone(&self) -> Option<&str> {
        self.matches()
            .get(self.selected)
            .and_then(|index| self.timezones.get(*index))
            .map(String::as_str)
    }

    pub(crate) fn reduce(&mut self, key: KeyEvent) -> TimezoneSelectorAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return TimezoneSelectorAction::Exit;
        }

        let matches = self.matches();
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                TimezoneSelectorAction::None
            }
            KeyCode::Down => {
                if !matches.is_empty() {
                    self.selected = (self.selected + 1).min(matches.len() - 1);
                }
                TimezoneSelectorAction::None
            }
            KeyCode::Enter => self
                .selected_timezone()
                .map(|timezone| TimezoneSelectorAction::Select(timezone.to_owned()))
                .unwrap_or(TimezoneSelectorAction::None),
            KeyCode::Esc => TimezoneSelectorAction::Exit,
            KeyCode::Char('q') => TimezoneSelectorAction::Exit,
            KeyCode::Char(character) => {
                self.query.push(character);
                self.selected = 0;
                TimezoneSelectorAction::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
                TimezoneSelectorAction::None
            }
            _ => TimezoneSelectorAction::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TimezoneSelectorAction {
    None,
    Exit,
    Select(String),
}

pub(crate) fn select_timezone(current: Option<&str>) -> anyhow::Result<Option<String>> {
    let timezones = chrono_tz::TZ_VARIANTS
        .iter()
        .map(ToString::to_string)
        .collect();
    terminal::enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = InteractiveTerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide).context("enter timezone selector")?;
    let mut state = TimezoneSelectorState::new(timezones, current);

    loop {
        render_timezone_selector(&mut stdout, &state)?;
        let key = loop {
            if let Event::Key(key) = event::read().context("read timezone selector input")?
                && key.kind != KeyEventKind::Release
            {
                break key;
            }
        };
        match state.reduce(key) {
            TimezoneSelectorAction::None => {}
            TimezoneSelectorAction::Exit => return Ok(None),
            TimezoneSelectorAction::Select(timezone) => return Ok(Some(timezone)),
        }
    }
}

pub(crate) fn render_timezone_selector(
    stdout: &mut impl Write,
    state: &TimezoneSelectorState,
) -> anyhow::Result<()> {
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0)).context("clear timezone selector")?;
    let matches = state.matches();
    let mut output = String::new();
    output.push_str("Select timezone\n");
    output.push_str(&format!(
        "Search: {}  ({} matches)\n",
        state.query,
        matches.len()
    ));
    output.push_str(
        "↑/↓ select || type to search || Backspace delete || Enter save || q/Esc cancel\n\n",
    );

    if matches.is_empty() {
        output.push_str("No matching IANA timezones.\n");
    } else {
        const VISIBLE_ROWS: usize = 12;
        let start = state
            .selected
            .saturating_sub(VISIBLE_ROWS / 2)
            .min(matches.len().saturating_sub(VISIBLE_ROWS));
        let end = (start + VISIBLE_ROWS).min(matches.len());
        for (visible_index, match_index) in matches[start..end].iter().enumerate() {
            let index = start + visible_index;
            let marker = if index == state.selected { '>' } else { ' ' };
            output.push_str(&format!("{marker} {}\n", state.timezones[*match_index]));
        }
    }

    stdout
        .write_all(output.as_bytes())
        .context("write timezone selector")?;
    stdout.flush().context("flush timezone selector")?;
    Ok(())
}
