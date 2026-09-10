//! Interactive configuration snapshot selection and rendering.

use std::io::{self, Write};

use anyhow::Context;
use chrono::Utc;
use crossterm::cursor::{Hide, MoveTo};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen};

use crate::StokerPaths;
use crate::application::{self, SnapshotReason};
use crate::config::{
    ConfigSnapshot, ConfigSnapshotEntry, ConfigSnapshotFile, ConfigSnapshotReason,
    ResolvedTimezone, StokerConfig, resolve_timezone,
};

use super::super::{application_cli_error, print_info, print_success, print_warning};
use super::terminal::InteractiveTerminalGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotView {
    List,
    Detail,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotSelectorState {
    pub(crate) selected: usize,
    pub(crate) view: SnapshotView,
}

impl SnapshotSelectorState {
    pub(crate) fn new() -> Self {
        Self {
            selected: 0,
            view: SnapshotView::List,
        }
    }

    pub(crate) fn reduce(
        &mut self,
        key: KeyEvent,
        entries: &[ConfigSnapshotEntry],
    ) -> SnapshotSelectorAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SnapshotSelectorAction::Exit;
        }

        match self.view {
            SnapshotView::List => match key.code {
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    SnapshotSelectorAction::None
                }
                KeyCode::Down => {
                    if !entries.is_empty() {
                        self.selected = (self.selected + 1).min(entries.len() - 1);
                    }
                    SnapshotSelectorAction::None
                }
                KeyCode::Enter if !entries.is_empty() => {
                    self.view = SnapshotView::Detail;
                    SnapshotSelectorAction::None
                }
                KeyCode::Char('q') | KeyCode::Esc => SnapshotSelectorAction::Exit,
                _ => SnapshotSelectorAction::None,
            },
            SnapshotView::Detail => match key.code {
                KeyCode::Enter
                    if matches!(
                        entries.get(self.selected),
                        Some(ConfigSnapshotEntry::Valid(_))
                    ) =>
                {
                    self.view = SnapshotView::Confirm;
                    SnapshotSelectorAction::None
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    self.view = SnapshotView::List;
                    SnapshotSelectorAction::None
                }
                _ => SnapshotSelectorAction::None,
            },
            SnapshotView::Confirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    SnapshotSelectorAction::Restore(self.selected)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.view = SnapshotView::List;
                    SnapshotSelectorAction::None
                }
                _ => SnapshotSelectorAction::None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotSelectorAction {
    None,
    Exit,
    Restore(usize),
}

pub(crate) fn restore_config(
    paths: &StokerPaths,
    cli_timezone: Option<&str>,
) -> anyhow::Result<()> {
    let entries = application::configuration::list_snapshots(paths)
        .map_err(application_cli_error)?
        .into_iter()
        .map(|snapshot| {
            if snapshot.valid {
                ConfigSnapshotEntry::Valid(ConfigSnapshotFile {
                    path: snapshot.path,
                    snapshot: ConfigSnapshot {
                        snapshot_version: 1,
                        created_at: snapshot.created_at.unwrap_or_else(Utc::now),
                        reason: match snapshot.reason.unwrap_or(SnapshotReason::Manual) {
                            SnapshotReason::Initial => ConfigSnapshotReason::Initial,
                            SnapshotReason::BeforeConfigUpdate => {
                                ConfigSnapshotReason::BeforeConfigUpdate
                            }
                            SnapshotReason::BeforeRestore => ConfigSnapshotReason::BeforeRestore,
                            SnapshotReason::Manual => ConfigSnapshotReason::Manual,
                        },
                        config: StokerConfig {
                            timezone: snapshot.timezone,
                        },
                    },
                })
            } else {
                ConfigSnapshotEntry::Invalid {
                    path: snapshot.path,
                    error: snapshot
                        .error
                        .unwrap_or_else(|| "invalid snapshot".to_owned()),
                }
            }
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        print_warning(format!(
            "No configuration snapshots found in {}.",
            paths.snapshot_dir().display()
        ));
        return Ok(());
    }

    let timezone = resolve_timezone(paths, cli_timezone)?;
    let current =
        application::configuration::configuration(paths).map_err(application_cli_error)?;
    let current = StokerConfig {
        timezone: current.timezone,
    };
    let Some(selected) = select_config_snapshot(&entries, &current, &timezone)? else {
        return Ok(());
    };

    let restored = application::configuration::restore_snapshot(paths, &selected.path)
        .map_err(application_cli_error)?;
    let changed = restored.timezone != current.timezone;
    if changed {
        print_success(format!(
            "Restored configuration from {}.",
            selected.path.display()
        ));
    } else {
        print_info("Configuration already matches that snapshot.");
    }
    Ok(())
}

pub(crate) fn select_config_snapshot(
    entries: &[ConfigSnapshotEntry],
    current: &StokerConfig,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<Option<ConfigSnapshotFile>> {
    terminal::enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = InteractiveTerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide).context("enter snapshot selector")?;
    let mut state = SnapshotSelectorState::new();

    loop {
        render_snapshot_selector(&mut stdout, &state, entries, current, timezone)?;
        let key = loop {
            if let Event::Key(key) = event::read().context("read snapshot selector input")?
                && key.kind != KeyEventKind::Release
            {
                break key;
            }
        };
        match state.reduce(key, entries) {
            SnapshotSelectorAction::None => {}
            SnapshotSelectorAction::Exit => return Ok(None),
            SnapshotSelectorAction::Restore(index) => {
                let Some(ConfigSnapshotEntry::Valid(snapshot)) = entries.get(index) else {
                    continue;
                };
                return Ok(Some(snapshot.clone()));
            }
        }
    }
}

pub(crate) fn render_snapshot_selector(
    stdout: &mut impl Write,
    state: &SnapshotSelectorState,
    entries: &[ConfigSnapshotEntry],
    current: &StokerConfig,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<()> {
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0)).context("clear snapshot selector")?;
    let mut output = String::new();
    match state.view {
        SnapshotView::List => {
            output.push_str("Stoker configuration snapshots\n");
            output.push_str("↑/↓ select || Enter details || q/Esc quit\n\n");
            let reason_width = snapshot_reason_width(entries);
            output.push_str(&snapshot_list_header(reason_width));
            for (index, entry) in entries.iter().enumerate() {
                let marker = if index == state.selected { '>' } else { ' ' };
                match entry {
                    ConfigSnapshotEntry::Valid(snapshot) => {
                        output.push_str(&format_snapshot_list_row(
                            marker,
                            timezone.format(snapshot.snapshot.created_at),
                            snapshot.snapshot.reason,
                            snapshot_summary(current, &snapshot.snapshot),
                            reason_width,
                        ));
                    }
                    ConfigSnapshotEntry::Invalid { path, .. } => {
                        output.push_str(&format_snapshot_list_row(
                            marker,
                            "-",
                            "invalid snapshot",
                            format!(
                                "unavailable: {}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("<unknown file>")
                            ),
                            reason_width,
                        ));
                    }
                }
            }
        }
        SnapshotView::Detail => {
            let entry = &entries[state.selected];
            output.push_str("Snapshot details (read-only)\n");
            output.push_str("q/Esc back to snapshot list\n\n");
            match entry {
                ConfigSnapshotEntry::Valid(snapshot) => {
                    output.push_str(&format!("File: {}\n", snapshot.path.display()));
                    output.push_str("Changes from current configuration:\n");
                    output.push_str(&format!(
                        "{}\n\n",
                        detailed_snapshot_summary(current, &snapshot.snapshot)
                    ));
                    output.push_str("Snapshot JSON:\n");
                    output.push_str(&snapshot_json_for_display(&snapshot.snapshot, timezone)?);
                    output.push_str("\n\nEnter restore this snapshot || q/Esc back");
                }
                ConfigSnapshotEntry::Invalid { path, error } => {
                    output.push_str(&format!("File: {}\n\n", path.display()));
                    output.push_str("This snapshot is unavailable and cannot be restored.\n");
                    output.push_str(&format!("Reason: {error}\n\nq/Esc back"));
                }
            }
        }
        SnapshotView::Confirm => {
            let snapshot = match &entries[state.selected] {
                ConfigSnapshotEntry::Valid(snapshot) => snapshot,
                ConfigSnapshotEntry::Invalid { .. } => {
                    return Ok(());
                }
            };
            output.push_str("Restore configuration snapshot?\n\n");
            output.push_str(&format!("File: {}\n", snapshot.path.display()));
            output.push_str(&format!(
                "Created time: {}\n",
                timezone.format(snapshot.snapshot.created_at)
            ));
            output.push_str("\nThe current configuration will be saved as a new snapshot first.\n");
            output.push_str("Continue? [y/N, q/Esc cancel]");
        }
    }
    stdout
        .write_all(output.as_bytes())
        .context("write snapshot selector")?;
    stdout.flush().context("flush snapshot selector")?;
    Ok(())
}

pub(crate) fn snapshot_reason_width(entries: &[ConfigSnapshotEntry]) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            ConfigSnapshotEntry::Valid(snapshot) => snapshot.snapshot.reason.to_string().len(),
            ConfigSnapshotEntry::Invalid { .. } => "invalid snapshot".len(),
        })
        .max()
        .unwrap_or(0)
        .max("Reason".len())
}

pub(crate) fn snapshot_list_header(reason_width: usize) -> String {
    format!(
        "  {:<29}  {:<reason_width$} {}\n",
        "Created time",
        "Reason",
        "Summary",
        reason_width = reason_width
    )
}

pub(crate) fn format_snapshot_list_row(
    marker: char,
    created_time: impl std::fmt::Display,
    reason: impl std::fmt::Display,
    summary: impl std::fmt::Display,
    reason_width: usize,
) -> String {
    let created_time = created_time.to_string();
    let reason = reason.to_string();
    let summary = summary.to_string();
    format!(
        "{marker} {created_time:<29}  {reason:<reason_width$} {summary}\n",
        reason_width = reason_width
    )
}

pub(crate) fn snapshot_summary(current: &StokerConfig, snapshot: &ConfigSnapshot) -> String {
    let keys = changed_config_keys(current, &snapshot.config);
    if keys.is_empty() {
        "same as current".to_owned()
    } else {
        format!("{} changed: {}", keys.len(), summarize_keys(&keys))
    }
}

pub(crate) fn detailed_snapshot_summary(
    current: &StokerConfig,
    snapshot: &ConfigSnapshot,
) -> String {
    let keys = changed_config_keys(current, &snapshot.config);
    if keys.is_empty() {
        "No differences.".to_owned()
    } else {
        keys.into_iter()
            .map(|key| format!("  {key}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub(crate) fn snapshot_json_for_display(
    snapshot: &ConfigSnapshot,
    timezone: &ResolvedTimezone,
) -> anyhow::Result<String> {
    let created_at = snapshot.created_at;
    let mut snapshot_json =
        serde_json::to_value(snapshot).context("format snapshot configuration")?;
    if let Some(object) = snapshot_json.as_object_mut() {
        object.insert(
            "created_at".to_owned(),
            serde_json::Value::String(timezone.format(created_at)),
        );
    }
    serde_json::to_string_pretty(&snapshot_json).context("format snapshot configuration")
}

pub(crate) fn changed_config_keys(current: &StokerConfig, snapshot: &StokerConfig) -> Vec<String> {
    let current = serde_json::to_value(current).expect("StokerConfig serializes");
    let snapshot = serde_json::to_value(snapshot).expect("StokerConfig serializes");
    let (Some(current), Some(snapshot)) = (current.as_object(), snapshot.as_object()) else {
        return Vec::new();
    };
    let mut keys = current
        .keys()
        .chain(snapshot.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    keys.retain(|key| current.get(key) != snapshot.get(key));
    keys.into_iter().collect()
}

pub(crate) fn summarize_keys(keys: &[String]) -> String {
    let value = keys.join(", ");
    if value.chars().count() <= 32 {
        value
    } else {
        let prefix = value.chars().take(29).collect::<String>();
        format!("{prefix}...")
    }
}
