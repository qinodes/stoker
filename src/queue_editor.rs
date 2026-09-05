use std::io::{self, Write};

use anyhow::Context;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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

#[derive(Debug)]
pub(crate) enum EditorMoveError {
    Stale,
    Callback(anyhow::Error),
}

impl From<anyhow::Error> for EditorMoveError {
    fn from(error: anyhow::Error) -> Self {
        Self::Callback(error)
    }
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

    #[cfg(test)]
    pub(crate) fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    #[cfg(test)]
    pub(crate) fn selected_index(&self) -> usize {
        self.selected
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> EditorMode {
        self.mode
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

    pub(crate) fn replace_jobs_after_move(
        &mut self,
        mut jobs: Vec<Job>,
        id: Uuid,
        original_order: usize,
    ) {
        jobs.sort_by_key(|job| job.queue_order.unwrap_or(i64::MAX));
        self.jobs = jobs;
        self.selected = self
            .jobs
            .iter()
            .position(|job| job.id == id)
            .unwrap_or_else(|| self.selected.min(self.jobs.len().saturating_sub(1)));
        if self.jobs.is_empty() {
            self.selected = 0;
        }
        self.mode = EditorMode::Move { id, original_order };
    }

    pub(crate) fn reduce(&mut self, key: KeyEvent) -> EditorIntent {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return EditorIntent::Exit;
        }
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

fn run_queue_editor_with_terminal<T, F, R, E>(
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

fn render<T: TerminalBackend>(
    terminal: &mut T,
    state: &EditorState,
    notice: Option<&str>,
) -> anyhow::Result<()> {
    terminal.clear()?;
    let mut output = String::new();
    if let Some(notice) = notice {
        output.push_str(notice);
        output.push('\n');
        output.push('\n');
    }
    match state.mode {
        EditorMode::Browse => {
            output.push_str(&format!(
                "Queue locked || {} jobs waiting\n",
                state.jobs.len()
            ));
            output.push_str("↑/↓ select a job || Enter move selected job || q leave editor\n\n");
        }
        EditorMode::Move { id, .. } => {
            let name = state
                .jobs
                .iter()
                .find(|job| job.id == id)
                .map(|job| job.name.as_str())
                .unwrap_or("(removed)");
            output.push_str(&format!("Moving: {name}\n"));
            output.push_str("↑/↓ adjust position || Enter keep move || q undo this move\n\n");
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
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::path::PathBuf;
    use std::rc::Rc;

    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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

    fn control_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    #[test]
    fn browse_down_selects_the_next_row() {
        let jobs = vec![job("first", 1), job("second", 2)];
        let mut state = EditorState::new(jobs);

        assert_eq!(state.reduce(key(KeyCode::Down)), EditorIntent::None);
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn key_release_does_not_move_selection_again() {
        let mut terminal = RecordingTerminal::with_events([
            key(KeyCode::Down),
            key_with_kind(KeyCode::Down, KeyEventKind::Release),
            key(KeyCode::Char('q')),
        ]);

        run_queue_editor_with_input(
            &mut terminal,
            vec![job("first", 1), job("second", 2), job("third", 3)],
            |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
            || -> anyhow::Result<Vec<Job>> { unreachable!() },
        )
        .unwrap();

        let rendered_rows = terminal
            .output
            .lines()
            .filter(|line| line.starts_with("> "))
            .collect::<Vec<_>>();
        assert!(rendered_rows.iter().any(|line| line.contains("second")));
        assert!(!rendered_rows.iter().any(|line| line.contains("third")));
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
        let first_id = first.id;
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
            vec![second_id, first_id]
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
    fn successful_move_keeps_move_mode_for_continued_editing() {
        let first = job("first", 1);
        let second = job("second", 2);
        let second_id = second.id;
        let mut state = EditorState::new(vec![first.clone(), second.clone()]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Enter));
        assert_eq!(
            state.reduce(key(KeyCode::Up)),
            EditorIntent::Move {
                id: second_id,
                target_order: 1,
            }
        );

        let mut moved_jobs = vec![second, first];
        for (index, job) in moved_jobs.iter_mut().enumerate() {
            job.queue_order = Some((index + 1) as i64);
        }
        state.replace_jobs_after_move(moved_jobs, second_id, 2);

        assert_eq!(
            state.mode(),
            EditorMode::Move {
                id: second_id,
                original_order: 2,
            }
        );
        assert_eq!(
            state.reduce(key(KeyCode::Down)),
            EditorIntent::Move {
                id: second_id,
                target_order: 2,
            }
        );
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
    fn ctrl_c_in_any_mode_exits() {
        let mut state = EditorState::new(vec![job("first", 1)]);

        assert_eq!(
            state.reduce(control_key(KeyCode::Char('c'))),
            EditorIntent::Exit
        );
        state.reduce(key(KeyCode::Enter));
        assert_eq!(
            state.reduce(control_key(KeyCode::Char('c'))),
            EditorIntent::Exit
        );
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
        output: String,
    }

    impl RecordingTerminal {
        fn with_events(events: impl IntoIterator<Item = KeyEvent>) -> Self {
            Self {
                events: events.into_iter().collect(),
                calls: Vec::new(),
                output: String::new(),
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
            self.output.push_str(_output);
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
            |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
            || -> anyhow::Result<Vec<Job>> { unreachable!() },
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
            |_id, _target| -> Result<Vec<Job>, anyhow::Error> {
                Err(anyhow::anyhow!("simulated move failure"))
            },
            || -> anyhow::Result<Vec<Job>> { unreachable!() },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "simulated move failure");
        terminal.assert_cleaned_up();
    }

    #[test]
    fn ctrl_c_exit_restores_terminal_state() {
        let mut terminal = RecordingTerminal::with_events([control_key(KeyCode::Char('c'))]);

        run_queue_editor_with_input(
            &mut terminal,
            vec![job("first", 1)],
            |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
            || -> anyhow::Result<Vec<Job>> { unreachable!() },
        )
        .unwrap();

        terminal.assert_cleaned_up();
    }

    #[test]
    fn stale_move_reloads_jobs_and_exits_cleanly_when_queue_is_empty() {
        let mut terminal =
            RecordingTerminal::with_events([key(KeyCode::Enter), key(KeyCode::Down)]);
        let mut reload_count = 0;

        run_queue_editor_with_input(
            &mut terminal,
            vec![job("selected", 1), job("other", 2)],
            |_id, _target| Err(EditorMoveError::Stale),
            || {
                reload_count += 1;
                Ok(Vec::new())
            },
        )
        .unwrap();

        assert_eq!(reload_count, 1);
        assert!(terminal.output.contains("removed"));
        terminal.assert_cleaned_up();
    }

    #[test]
    fn stale_move_reloads_current_jobs_and_continues_editing() {
        let mut terminal = RecordingTerminal::with_events([
            key(KeyCode::Enter),
            key(KeyCode::Down),
            key(KeyCode::Char('q')),
        ]);
        let mut reload_count = 0;

        run_queue_editor_with_input(
            &mut terminal,
            vec![job("selected", 1), job("other", 2)],
            |_id, _target| Err(EditorMoveError::Stale),
            || {
                reload_count += 1;
                Ok(vec![job("remaining", 1)])
            },
        )
        .unwrap();

        assert_eq!(reload_count, 1);
        assert!(terminal.output.contains("removed"));
        terminal.assert_cleaned_up();
    }

    #[test]
    fn non_empty_editor_can_move_keep_undo_and_exit_without_unlocking() {
        let first = job("first", 1);
        let second = job("second", 2);
        let third = job("third", 3);
        let second_id = second.id;
        let third_id = third.id;
        let initial_ids = vec![first.id, second_id, third_id];
        let persisted = Rc::new(RefCell::new(vec![first, second, third]));
        let mut moves = Vec::new();
        let mut terminal = RecordingTerminal::with_events([
            key(KeyCode::Down),
            key(KeyCode::Enter),
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::Enter),
            key(KeyCode::Down),
            key(KeyCode::Enter),
            key(KeyCode::Up),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('q')),
        ]);
        let move_state = Rc::clone(&persisted);
        let reload_state = Rc::clone(&persisted);

        let initial_jobs = persisted.borrow().clone();
        run_queue_editor_with_input(
            &mut terminal,
            initial_jobs,
            |id, target_order| {
                moves.push((id, target_order));
                let mut persisted = move_state.borrow_mut();
                let selected = persisted
                    .iter()
                    .position(|job| job.id == id)
                    .expect("selected job remains queued");
                let job = persisted.remove(selected);
                persisted.insert(target_order - 1, job);
                for (index, job) in persisted.iter_mut().enumerate() {
                    job.queue_order = Some((index + 1) as i64);
                }
                Ok::<_, anyhow::Error>(persisted.clone())
            },
            move || Ok::<_, anyhow::Error>(reload_state.borrow().clone()),
        )
        .unwrap();

        assert_eq!(
            moves,
            vec![(second_id, 1), (second_id, 2), (third_id, 2), (third_id, 3),]
        );
        assert_eq!(
            persisted
                .borrow()
                .iter()
                .map(|job| job.id)
                .collect::<Vec<_>>(),
            initial_ids
        );
        terminal.assert_cleaned_up();
    }

    fn run_queue_editor_with_input<T, F, R, E>(
        terminal: &mut T,
        jobs: Vec<Job>,
        move_job: F,
        reload_jobs: R,
    ) -> anyhow::Result<()>
    where
        T: TerminalBackend,
        F: FnMut(Uuid, usize) -> Result<Vec<Job>, E>,
        E: Into<EditorMoveError>,
        R: FnMut() -> anyhow::Result<Vec<Job>>,
    {
        run_queue_editor_with_terminal(terminal, jobs, move_job, reload_jobs)
    }

    #[allow(dead_code)]
    fn _io_error(error: io::Error) -> anyhow::Error {
        error.into()
    }
}
