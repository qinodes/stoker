//! Queue editor rendering.

use super::state::{EditorMode, EditorState};
use super::terminal::TerminalBackend;

pub(super) fn render<T: TerminalBackend>(
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
            output
                .push_str("↑/↓ select a job || Enter move selected job || q/Esc leave editor\n\n");
        }
        EditorMode::Move { id, .. } => {
            let name = state
                .jobs
                .iter()
                .find(|job| job.id == id)
                .map(|job| job.name.as_str())
                .unwrap_or("(removed)");
            output.push_str(&format!("Moving: {name}\n"));
            output.push_str("↑/↓ adjust position || Enter keep move || q/Esc undo this move\n\n");
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
