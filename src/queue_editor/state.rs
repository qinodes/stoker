//! Pure queue editor state transitions.

use uuid::Uuid;

use crate::Job;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorIntent {
    None,
    Exit,
    Move { id: Uuid, target_order: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorKey {
    Up,
    Down,
    Enter,
    Cancel,
    Interrupt,
    Other,
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
    pub(super) jobs: Vec<Job>,
    pub(super) selected: usize,
    pub(super) mode: EditorMode,
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

    pub(crate) fn reduce(&mut self, key: impl Into<EditorKey>) -> EditorIntent {
        let key = key.into();
        if key == EditorKey::Interrupt {
            return EditorIntent::Exit;
        }
        match self.mode {
            EditorMode::Browse => self.reduce_browse(key),
            EditorMode::Move { id, original_order } => self.reduce_move(key, id, original_order),
        }
    }

    fn reduce_browse(&mut self, key: EditorKey) -> EditorIntent {
        match key {
            EditorKey::Up => {
                self.selected = self.selected.saturating_sub(1);
                EditorIntent::None
            }
            EditorKey::Down => {
                if !self.jobs.is_empty() {
                    self.selected = (self.selected + 1).min(self.jobs.len() - 1);
                }
                EditorIntent::None
            }
            EditorKey::Enter if !self.jobs.is_empty() => {
                let id = self.jobs[self.selected].id;
                self.mode = EditorMode::Move {
                    id,
                    original_order: self.selected + 1,
                };
                EditorIntent::None
            }
            EditorKey::Cancel => EditorIntent::Exit,
            _ => EditorIntent::None,
        }
    }

    fn reduce_move(&mut self, key: EditorKey, id: Uuid, original_order: usize) -> EditorIntent {
        let Some(current) = self.jobs.iter().position(|job| job.id == id) else {
            self.mode = EditorMode::Browse;
            return EditorIntent::None;
        };
        match key {
            EditorKey::Up if current > 0 => {
                self.jobs.swap(current, current - 1);
                self.selected = current - 1;
                self.normalize_orders();
                EditorIntent::Move {
                    id,
                    target_order: self.selected + 1,
                }
            }
            EditorKey::Down if current + 1 < self.jobs.len() => {
                self.jobs.swap(current, current + 1);
                self.selected = current + 1;
                self.normalize_orders();
                EditorIntent::Move {
                    id,
                    target_order: self.selected + 1,
                }
            }
            EditorKey::Enter => {
                self.selected = current;
                self.mode = EditorMode::Browse;
                EditorIntent::None
            }
            EditorKey::Cancel => {
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
