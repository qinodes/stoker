//! Terminal backend and editor orchestration.

use std::io::{self, Write};

use anyhow::Context;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use uuid::Uuid;

use crate::Job;

use super::render::render;
use super::state::{EditorIntent, EditorKey, EditorMode, EditorMoveError, EditorState};

impl From<KeyEvent> for EditorKey {
    fn from(key: KeyEvent) -> Self {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Self::Interrupt;
        }
        match key.code {
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::Enter => Self::Enter,
            KeyCode::Char('q') | KeyCode::Esc => Self::Cancel,
            _ => Self::Other,
        }
    }
}

pub(crate) trait TerminalBackend {
    fn enable_raw_mode(&mut self) -> anyhow::Result<()>;
    fn disable_raw_mode(&mut self) -> anyhow::Result<()>;
    fn enter_alternate_screen(&mut self) -> anyhow::Result<()>;
    fn leave_alternate_screen(&mut self) -> anyhow::Result<()>;
    fn hide_cursor(&mut self) -> anyhow::Result<()>;
    fn show_cursor(&mut self) -> anyhow::Result<()>;
    fn clear(&mut self) -> anyhow::Result<()>;
    fn write(&mut self, output: &str) -> anyhow::Result<()>;
    fn read_key(&mut self) -> anyhow::Result<KeyEvent>;
}

struct CrosstermTerminal {
    stdout: io::Stdout,
}

impl CrosstermTerminal {
    fn new() -> Self {
        Self {
            stdout: io::stdout(),
        }
    }
}

impl TerminalBackend for CrosstermTerminal {
    fn enable_raw_mode(&mut self) -> anyhow::Result<()> {
        terminal::enable_raw_mode().context("enable terminal raw mode")?;
        Ok(())
    }

    fn disable_raw_mode(&mut self) -> anyhow::Result<()> {
        terminal::disable_raw_mode().context("disable terminal raw mode")?;
        Ok(())
    }

    fn enter_alternate_screen(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, EnterAlternateScreen).context("enter alternate screen")?;
        Ok(())
    }

    fn leave_alternate_screen(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, LeaveAlternateScreen).context("leave alternate screen")?;
        Ok(())
    }

    fn hide_cursor(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, Hide).context("hide cursor")?;
        Ok(())
    }

    fn show_cursor(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, Show).context("show cursor")?;
        Ok(())
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        execute!(self.stdout, Clear(ClearType::All), MoveTo(0, 0)).context("clear terminal")?;
        Ok(())
    }

    fn write(&mut self, output: &str) -> anyhow::Result<()> {
        self.stdout
            .write_all(output.as_bytes())
            .context("write terminal")?;
        self.stdout.flush().context("flush terminal")?;
        Ok(())
    }

    fn read_key(&mut self) -> anyhow::Result<KeyEvent> {
        loop {
            if let Event::Key(key) = event::read().context("read terminal input")? {
                return Ok(key);
            }
        }
    }
}

struct Cleanup<'a, T: TerminalBackend> {
    terminal: &'a mut T,
}

impl<'a, T: TerminalBackend> Cleanup<'a, T> {
    fn new(terminal: &'a mut T) -> Self {
        Self { terminal }
    }

    fn terminal(&mut self) -> &mut T {
        self.terminal
    }
}

impl<T: TerminalBackend> Drop for Cleanup<'_, T> {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        let _ = self.terminal.leave_alternate_screen();
        let _ = self.terminal.disable_raw_mode();
    }
}

pub(crate) fn run_queue_editor<F, R, E>(
    initial_jobs: Vec<Job>,
    move_job: F,
    reload_jobs: R,
) -> anyhow::Result<()>
where
    F: FnMut(Uuid, usize) -> Result<Vec<Job>, E>,
    E: Into<EditorMoveError>,
    R: FnMut() -> anyhow::Result<Vec<Job>>,
{
    let mut terminal = CrosstermTerminal::new();
    run_queue_editor_with_terminal(&mut terminal, initial_jobs, move_job, reload_jobs)
}

pub(super) fn run_queue_editor_with_terminal<T, F, R, E>(
    terminal: &mut T,
    initial_jobs: Vec<Job>,
    mut move_job: F,
    mut reload_jobs: R,
) -> anyhow::Result<()>
where
    T: TerminalBackend,
    F: FnMut(Uuid, usize) -> Result<Vec<Job>, E>,
    E: Into<EditorMoveError>,
    R: FnMut() -> anyhow::Result<Vec<Job>>,
{
    terminal.enable_raw_mode()?;
    let mut cleanup = Cleanup::new(terminal);
    cleanup.terminal().enter_alternate_screen()?;
    cleanup.terminal().hide_cursor()?;

    run_editor_loop(
        cleanup.terminal(),
        initial_jobs,
        &mut move_job,
        &mut reload_jobs,
    )
}

fn run_editor_loop<T, F, R, E>(
    terminal: &mut T,
    initial_jobs: Vec<Job>,
    move_job: &mut F,
    reload_jobs: &mut R,
) -> anyhow::Result<()>
where
    T: TerminalBackend,
    F: FnMut(Uuid, usize) -> Result<Vec<Job>, E>,
    E: Into<EditorMoveError>,
    R: FnMut() -> anyhow::Result<Vec<Job>>,
{
    let mut state = EditorState::new(initial_jobs);
    let mut notice = None;
    loop {
        render(terminal, &state, notice)?;
        notice = None;
        if state.jobs.is_empty() {
            return Ok(());
        }
        let key = terminal.read_key()?;
        if key.kind == KeyEventKind::Release {
            continue;
        }
        let intent = state.reduce(key);
        match intent {
            EditorIntent::None => {}
            EditorIntent::Exit => return Ok(()),
            EditorIntent::Move { id, target_order } => {
                let mode_after_reduce = state.mode;
                let result: Result<Vec<Job>, EditorMoveError> =
                    move_job(id, target_order).map_err(Into::into);
                match result {
                    Ok(jobs) => match mode_after_reduce {
                        EditorMode::Move { id, original_order } => {
                            state.replace_jobs_after_move(jobs, id, original_order);
                        }
                        EditorMode::Browse => state.replace_jobs(jobs),
                    },
                    Err(EditorMoveError::Stale) => {
                        notice = Some("Selected job was removed; reloading queued jobs.");
                        state.replace_jobs(reload_jobs()?);
                    }
                    Err(EditorMoveError::Callback(error)) => return Err(error),
                }
            }
        }
    }
}
