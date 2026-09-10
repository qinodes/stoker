//! Interactive queue editor compatibility façade.

mod render;
mod state;
mod terminal;

pub(crate) use state::EditorMoveError;
#[cfg(test)]
use state::{EditorIntent, EditorMode, EditorState};
pub(crate) use terminal::run_queue_editor;
#[cfg(test)]
use terminal::{TerminalBackend, run_queue_editor_with_terminal};

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
            description: None,
            description_revision: 0,
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
    fn escape_in_browse_mode_exits() {
        let mut state = EditorState::new(vec![job("first", 1)]);

        assert_eq!(state.reduce(key(KeyCode::Esc)), EditorIntent::Exit);
    }

    #[test]
    fn escape_in_move_mode_undoes_the_current_move() {
        let first = job("first", 1);
        let second = job("second", 2);
        let first_id = first.id;
        let second_id = second.id;
        let mut state = EditorState::new(vec![first, second]);
        state.reduce(key(KeyCode::Down));
        state.reduce(key(KeyCode::Enter));
        state.reduce(key(KeyCode::Up));

        assert_eq!(
            state.reduce(key(KeyCode::Esc)),
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
        fail_on: Option<&'static str>,
    }

    impl RecordingTerminal {
        fn with_events(events: impl IntoIterator<Item = KeyEvent>) -> Self {
            Self {
                events: events.into_iter().collect(),
                calls: Vec::new(),
                output: String::new(),
                fail_on: None,
            }
        }

        fn failing(stage: &'static str) -> Self {
            let mut terminal = Self::with_events([key(KeyCode::Char('q'))]);
            terminal.fail_on = Some(stage);
            terminal
        }

        fn record(&mut self, stage: &'static str) -> anyhow::Result<()> {
            self.calls.push(stage);
            if self.fail_on == Some(stage) {
                anyhow::bail!("simulated {stage} failure");
            }
            Ok(())
        }

        fn assert_cleaned_up(&self) {
            assert!(self.calls.contains(&"leave_alternate_screen"));
            assert!(self.calls.contains(&"show_cursor"));
            assert!(self.calls.contains(&"disable_raw_mode"));
        }
    }

    impl TerminalBackend for RecordingTerminal {
        fn enable_raw_mode(&mut self) -> anyhow::Result<()> {
            self.record("enable_raw_mode")
        }

        fn disable_raw_mode(&mut self) -> anyhow::Result<()> {
            self.record("disable_raw_mode")
        }

        fn enter_alternate_screen(&mut self) -> anyhow::Result<()> {
            self.record("enter_alternate_screen")
        }

        fn leave_alternate_screen(&mut self) -> anyhow::Result<()> {
            self.record("leave_alternate_screen")
        }

        fn hide_cursor(&mut self) -> anyhow::Result<()> {
            self.record("hide_cursor")
        }

        fn show_cursor(&mut self) -> anyhow::Result<()> {
            self.record("show_cursor")
        }

        fn clear(&mut self) -> anyhow::Result<()> {
            self.record("clear")
        }

        fn write(&mut self, _output: &str) -> anyhow::Result<()> {
            self.record("write")?;
            self.output.push_str(_output);
            Ok(())
        }

        fn read_key(&mut self) -> anyhow::Result<KeyEvent> {
            self.record("read_key")?;
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
    fn setup_and_loop_failures_restore_terminal_after_raw_mode_is_enabled() {
        for stage in [
            "enter_alternate_screen",
            "hide_cursor",
            "clear",
            "write",
            "read_key",
        ] {
            let mut terminal = RecordingTerminal::failing(stage);

            let error = run_queue_editor_with_input(
                &mut terminal,
                vec![job("first", 1)],
                |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
                || -> anyhow::Result<Vec<Job>> { unreachable!() },
            )
            .unwrap_err();

            assert!(error.to_string().contains(stage));
            terminal.assert_cleaned_up();
        }
    }

    #[test]
    fn cleanup_attempts_every_restore_step_even_if_one_fails() {
        for stage in ["show_cursor", "leave_alternate_screen", "disable_raw_mode"] {
            let mut terminal = RecordingTerminal::failing(stage);

            run_queue_editor_with_input(
                &mut terminal,
                vec![job("first", 1)],
                |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
                || -> anyhow::Result<Vec<Job>> { unreachable!() },
            )
            .unwrap();

            terminal.assert_cleaned_up();
        }
    }

    #[test]
    fn raw_mode_failure_does_not_run_cleanup_for_state_that_was_not_entered() {
        let mut terminal = RecordingTerminal::failing("enable_raw_mode");

        let error = run_queue_editor_with_input(
            &mut terminal,
            vec![job("first", 1)],
            |_id, _target| -> Result<Vec<Job>, anyhow::Error> { unreachable!() },
            || -> anyhow::Result<Vec<Job>> { unreachable!() },
        )
        .unwrap_err();

        assert!(error.to_string().contains("enable_raw_mode"));
        assert_eq!(terminal.calls, vec!["enable_raw_mode"]);
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
