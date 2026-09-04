use std::io::{self, Write};

use anyhow::Context;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use uuid::Uuid;

use crate::Job;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorIntent {
    None,
    Exit,
    Move { id: Uuid, target_order: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorMode {
    Browse,
    Move { id: Uuid, original_order: usize },
}

#[derive(Debug, Clone)]
pub(crate) struct EditorState {
    jobs: Vec<Job>,
    selected: usize,
    mode: EditorMode,
}

impl EditorState {
    pub(crate) fn new(mut jobs: Vec<Job>) -> Self {
        jobs.sort_by_key(|job| job.queue_order.unwrap_or(i64::MAX));
        Self {
            jobs,
            selected: 0,
            mode: EditorMode::Browse,
        }
    }

    pub(crate) fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    pub(crate) fn selected_index(&self) -> usize {
        self.selected
    }

    pub(crate) fn mode(&self) -> EditorMode {
        self.mode
    }

    pub(crate) fn mode_id(&self) -> Option<Uuid> {
        match self.mode {
            EditorMode::Browse => None,
            EditorMode::Move { id, .. } => Some(id),
        }
    }

    pub(crate) fn replace_jobs(&mut self, mut jobs: Vec<Job>) {
        let selected_id = self.jobs.get(self.selected).map(|job| job.id);
        jobs.sort_by_key(|job| job.queue_order.unwrap_or(i64::MAX));
        self.jobs = jobs;
        self.selected = selected_id
            .and_then(|id| self.jobs.iter().position(|job| job.id == id))
            .unwrap_or_else(|| self.selected.min(self.jobs.len().saturating_sub(1)));
        if self.jobs.is_empty() {
            self.selected = 0;
        }
        self.mode = EditorMode::Browse;
    }

    pub(crate) fn reduce(&mut self, key: KeyEvent) -> EditorIntent {
        match self.mode {
            EditorMode::Browse => self.reduce_browse(key),
            EditorMode::Move { id, original_order } => self.reduce_move(key, id, original_order),
        }
    }

    fn reduce_browse(&mut self, key: KeyEvent) -> EditorIntent {
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                EditorIntent::None
            }
            KeyCode::Down => {
                if !self.jobs.is_empty() {
                    self.selected = (self.selected + 1).min(self.jobs.len() - 1);
                }
                EditorIntent::None
            }
            KeyCode::Enter if !self.jobs.is_empty() => {
                let id = self.jobs[self.selected].id;
                self.mode = EditorMode::Move {
                    id,
                    original_order: self.selected + 1,
                };
                EditorIntent::None
            }
            KeyCode::Char('q') => EditorIntent::Exit,
            _ => EditorIntent::None,
        }
    }

    fn reduce_move(&mut self, key: KeyEvent, id: Uuid, original_order: usize) -> EditorIntent {
        let Some(current) = self.jobs.iter().position(|job| job.id == id) else {
            self.mode = EditorMode::Browse;
            return EditorIntent::None;
        };
        match key.code {
            KeyCode::Up if current > 0 => {
                self.jobs.swap(current, current - 1);
                self.selected = current - 1;
                self.normalize_orders();
                EditorIntent::Move {
                    id,
                    target_order: self.selected + 1,
                }
            }
            KeyCode::Down if current + 1 < self.jobs.len() => {
                self.jobs.swap(current, current + 1);
                self.selected = current + 1;
                self.normalize_orders();
                EditorIntent::Move {
                    id,
                    target_order: self.selected + 1,
                }
            }
            KeyCode::Enter => {
                self.selected = current;
                self.mode = EditorMode::Browse;
                EditorIntent::None
            }
            KeyCode::Char('q') => {
                let target = original_order
                    .saturating_sub(1)
                    .min(self.jobs.len().saturating_sub(1));
                let job = self.jobs.remove(current);
                self.jobs.insert(target, job);
                self.selected = target;
                self.normalize_orders();
                self.mode = EditorMode::Browse;
                EditorIntent::Move {
                    id,
                    target_order: target + 1,
                }
            }
            _ => EditorIntent::None,
        }
    }

    fn normalize_orders(&mut self) {
        for (index, job) in self.jobs.iter_mut().enumerate() {
            job.queue_order = Some((index + 1) as i64);
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

pub(crate) fn run_queue_editor<F>(initial_jobs: Vec<Job>, move_job: F) -> anyhow::Result<()>
where
    F: FnMut(Uuid, usize) -> anyhow::Result<Vec<Job>>,
{
    let mut terminal = CrosstermTerminal::new();
    run_queue_editor_with_terminal(&mut terminal, initial_jobs, move_job)
}

fn run_queue_editor_with_terminal<T, F>(
    terminal: &mut T,
    initial_jobs: Vec<Job>,
    mut move_job: F,
) -> anyhow::Result<()>
where
    T: TerminalBackend,
    F: FnMut(Uuid, usize) -> anyhow::Result<Vec<Job>>,
{
    terminal.enable_raw_mode()?;
    let mut cleanup = Cleanup::new(terminal);
    cleanup.terminal().enter_alternate_screen()?;
    cleanup.terminal().hide_cursor()?;

    let result = run_editor_loop(cleanup.terminal(), initial_jobs, &mut move_job);
    result
}

fn run_editor_loop<T, F>(
    terminal: &mut T,
    initial_jobs: Vec<Job>,
    move_job: &mut F,
) -> anyhow::Result<()>
where
    T: TerminalBackend,
    F: FnMut(Uuid, usize) -> anyhow::Result<Vec<Job>>,
{
    let mut state = EditorState::new(initial_jobs);
    loop {
        render(terminal, &state)?;
        if state.jobs.is_empty() {
            return Ok(());
        }
        let intent = state.reduce(terminal.read_key()?);
        match intent {
            EditorIntent::None => {}
            EditorIntent::Exit => return Ok(()),
            EditorIntent::Move { id, target_order } => {
                let jobs = move_job(id, target_order)?;
                state.replace_jobs(jobs);
            }
        }
    }
}

fn render<T: TerminalBackend>(terminal: &mut T, state: &EditorState) -> anyhow::Result<()> {
    terminal.clear()?;
    let mut output = String::new();
    match state.mode {
        EditorMode::Browse => {
            output.push_str(&format!(
                "Queue locked · {} jobs waiting\n",
                state.jobs.len()
            ));
            output.push_str("↑/↓ select a job · Enter move selected job · q leave editor\n\n");
        }
        EditorMode::Move { id, .. } => {
            let name = state
                .jobs
                .iter()
                .find(|job| job.id == id)
                .map(|job| job.name.as_str())
                .unwrap_or("(removed)");
            output.push_str(&format!("Moving: {name}\n"));
            output.push_str("↑/↓ adjust position · Enter keep move · q undo this move\n\n");
        }
    }
    for (index, job) in state.jobs.iter().enumerate() {
        let marker = if index == state.selected { '>' } else { ' ' };
        let short_id = job.id.to_string();
        let short_id = &short_id[..8];
        output.push_str(&format!(
            "{marker} {:>2}. {:<24} {:<16} {short_id}\n",
            index + 1,
            job.name,
            job.user
        ));
    }
    terminal.write(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;
    use std::path::PathBuf;

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use uuid::Uuid;

    use crate::{Job, JobState};

    fn job(name: &str, order: usize) -> Job {
        Job {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            user: "alice".to_owned(),
            cwd: PathBuf::from("/tmp"),
            command: vec!["echo".to_owned(), name.to_owned()],
            command_line: None,
            state: JobState::Queued,
            queue_order: Some(order as i64),
            created_at: Utc::now(),
            committed_at: Some(Utc::now()),
            started_at: None,
            finished_at: None,
            exit_code: None,
            pid: None,
            failure_detail: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn browse_down_selects_the_next_row() {
        let jobs = vec![job("first", 1), job("second", 2)];
        let mut state = EditorState::new(jobs);

        assert_eq!(state.reduce(key(KeyCode::Down)), EditorIntent::None);
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn enter_records_original_order_and_enters_move_mode() {
        let jobs = vec![job("first", 1), job("second", 2)];
        let mut state = EditorState::new(jobs);
        state.reduce(key(KeyCode::Down));

        assert_eq!(state.reduce(key(KeyCode::Enter)), EditorIntent::None);
        assert_eq!(
            state.mode(),
            EditorMode::Move {
                id: state.jobs()[1].id,
                original_order: 2,
            }
        );
    }

    #[test]
    fn up_returns_move_intent_for_the_new_position() {
        let first = job("first", 1);
        let second = job("second", 2);
        let second_id = second.id;
        let mut state = EditorState::new(vec![first, second]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Enter));

        assert_eq!(
            state.reduce(key(KeyCode::Up)),
            EditorIntent::Move {
                id: second_id,
                target_order: 1,
            }
        );
        assert_eq!(
            state.jobs().iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![second_id, first.id]
        );
    }

    #[test]
    fn enter_returns_to_browse_while_retaining_the_moved_display_order() {
        let first = job("first", 1);
        let second = job("second", 2);
        let second_id = second.id;
        let mut state = EditorState::new(vec![first, second]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Enter));
        state.reduce(key(KeyCode::Up));

        assert_eq!(state.reduce(key(KeyCode::Enter)), EditorIntent::None);
        assert_eq!(state.mode(), EditorMode::Browse);
        assert_eq!(state.jobs()[0].id, second_id);
    }

    #[test]
    fn q_in_move_mode_undoes_only_the_current_job_and_returns_to_browse() {
        let first = job("first", 1);
        let second = job("second", 2);
        let first_id = first.id;
        let second_id = second.id;
        let mut state = EditorState::new(vec![first, second]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Enter));
        state.reduce(key(KeyCode::Up));

        assert_eq!(
            state.reduce(key(KeyCode::Char('q'))),
            EditorIntent::Move {
                id: second_id,
                target_order: 2,
            }
        );
        assert_eq!(state.mode(), EditorMode::Browse);
        assert_eq!(state.jobs()[0].id, first_id);
        assert_eq!(state.jobs()[1].id, second_id);
    }

    #[test]
    fn q_in_browse_mode_exits() {
        let mut state = EditorState::new(vec![job("first", 1)]);

        assert_eq!(state.reduce(key(KeyCode::Char('q'))), EditorIntent::Exit);
    }

    #[test]
    fn reload_after_removed_selected_job_clamps_selection() {
        let first = job("first", 1);
        let second = job("second", 2);
        let third = job("third", 3);
        let mut state = EditorState::new(vec![first, second, third]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Down));

        state.replace_jobs(vec![job("first", 1), job("second", 2)]);

        assert_eq!(state.selected_index(), 1);
        assert_eq!(state.mode(), EditorMode::Browse);
    }

    struct RecordingTerminal {
        events: VecDeque<KeyEvent>,
        calls: Vec<&'static str>,
    }

    impl RecordingTerminal {
        fn with_events(events: impl IntoIterator<Item = KeyEvent>) -> Self {
            Self {
                events: events.into_iter().collect(),
                calls: Vec::new(),
            }
        }

        fn assert_cleaned_up(&self) {
            assert!(self.calls.contains(&"leave_alternate_screen"));
            assert!(self.calls.contains(&"show_cursor"));
            assert!(self.calls.contains(&"disable_raw_mode"));
        }
    }

    impl TerminalBackend for RecordingTerminal {
        fn enable_raw_mode(&mut self) -> anyhow::Result<()> {
            self.calls.push("enable_raw_mode");
            Ok(())
        }

        fn disable_raw_mode(&mut self) -> anyhow::Result<()> {
            self.calls.push("disable_raw_mode");
            Ok(())
        }

        fn enter_alternate_screen(&mut self) -> anyhow::Result<()> {
            self.calls.push("enter_alternate_screen");
            Ok(())
        }

        fn leave_alternate_screen(&mut self) -> anyhow::Result<()> {
            self.calls.push("leave_alternate_screen");
            Ok(())
        }

        fn hide_cursor(&mut self) -> anyhow::Result<()> {
            self.calls.push("hide_cursor");
            Ok(())
        }

        fn show_cursor(&mut self) -> anyhow::Result<()> {
            self.calls.push("show_cursor");
            Ok(())
        }

        fn clear(&mut self) -> anyhow::Result<()> {
            self.calls.push("clear");
            Ok(())
        }

        fn write(&mut self, _output: &str) -> anyhow::Result<()> {
            self.calls.push("write");
            Ok(())
        }

        fn read_key(&mut self) -> anyhow::Result<KeyEvent> {
            self.events
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no event"))
        }
    }

    #[test]
    fn successful_editor_exit_restores_terminal_state() {
        let mut terminal = RecordingTerminal::with_events([key(KeyCode::Char('q'))]);

        run_queue_editor_with_input(
            &mut terminal,
            vec![job("first", 1)],
            |_id, _target| unreachable!(),
        )
        .unwrap();

        terminal.assert_cleaned_up();
    }

    #[test]
    fn move_callback_error_restores_terminal_state() {
        let mut terminal =
            RecordingTerminal::with_events([key(KeyCode::Enter), key(KeyCode::Down)]);

        let error = run_queue_editor_with_input(
            &mut terminal,
            vec![job("first", 1), job("second", 2)],
            |_id, _target| Err(anyhow::anyhow!("simulated move failure")),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "simulated move failure");
        terminal.assert_cleaned_up();
    }

    fn run_queue_editor_with_input<T, F>(
        terminal: &mut T,
        jobs: Vec<Job>,
        move_job: F,
    ) -> anyhow::Result<()>
    where
        T: TerminalBackend,
        F: FnMut(Uuid, usize) -> anyhow::Result<Vec<Job>>,
    {
        run_queue_editor_with_terminal(terminal, jobs, move_job)
    }

    #[allow(dead_code)]
    fn _io_error(error: io::Error) -> anyhow::Error {
        error.into()
    }
}
