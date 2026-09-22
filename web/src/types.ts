export type WorkspaceMode = "serial" | "scheduled";

export type Route =
  | "overview"
  | "jobs"
  | "queue"
  | "workloads"
  | "runs"
  | "logs"
  | "sources"
  | "configuration"
  | "policy"
  | "mode-change";

export interface WorkspaceResponse {
  mode: WorkspaceMode;
  queue_locked: boolean;
  recovery_fence: boolean;
  scheduler: {
    running: boolean;
    pid: number | null;
    active_job: string | null;
    queued_jobs: number;
  };
  timezone: TimezoneInfo;
  generated_at: string;
}

export interface ModeTransition {
  actualMode: WorkspaceMode;
  deferred: boolean;
}

export type JobState =
  | "DRAFT"
  | "QUEUED"
  | "STARTING"
  | "RUNNING"
  | "CANCELLING"
  | "SUCCEEDED"
  | "FAILED"
  | "CANCELLED"
  | "LOST"
  | string;

export interface Job {
  id: string;
  name: string;
  user: string;
  cwd: string;
  command?: string[];
  command_line?: string | null;
  state: JobState;
  queue_order?: number | null;
  created_at?: string | null;
  committed_at?: string | null;
  started_at?: string | null;
  finished_at?: string | null;
  exit_code?: number | null;
  pid?: number | null;
  failure_detail?: string | null;
  description?: string | null;
  description_revision?: number;
}

export interface UiConfig {
  version?: string;
  max_job_name_length?: number;
  max_job_user_length?: number;
  max_job_description_length?: number;
}

export interface TimezoneInfo {
  name: string;
  source: string;
}

export interface StatusResponse {
  scheduler: {
    running: boolean;
    pid: number | null;
    active_job: Job | null;
    queued_jobs: number;
  };
  counts: {
    total: number;
    draft: number;
    queued: number;
    active: number;
    succeeded: number;
    failed: number;
  };
  queue_locked: boolean;
  disk_pressure?: boolean;
  timezone?: TimezoneInfo | null;
  generated_at?: string;
}

export interface JobsResponse {
  jobs: Job[];
  timezone?: TimezoneInfo | null;
}

export interface QueueResponse {
  jobs: Job[];
  locked: boolean;
}

export interface Snapshot {
  path: string;
  valid: boolean;
  created_at?: string | null;
  reason?: string | null;
  timezone?: string | null;
  error?: string | null;
}

export interface SettingsResponse {
  config: Record<string, unknown>;
  effective_timezone: TimezoneInfo;
  timezones: string[];
  config_path: string;
  snapshot_dir: string;
  snapshots: Snapshot[];
}

export interface PolicyResponse {
  log: {
    max_bytes_per_job: number;
    segment_bytes: number;
    max_bytes_total: number;
    retention_jobs: number;
    disk_reserve_bytes: number;
  };
  runtime: {
    termination_grace_ms: number;
    max_runtime_ms: number | null;
    startup_timeout_ms: number;
  };
  defaults: {
    log: PolicyResponse["log"];
    runtime: PolicyResponse["runtime"];
  };
  units: {
    log: Record<keyof PolicyResponse["log"], string>;
    runtime: Record<keyof PolicyResponse["runtime"], string>;
  };
  queue_locked: boolean;
  can_update: boolean;
  active_jobs: Array<{ id: string; name: string; state: JobState }>;
  blocked_reason: string | null;
}

export interface JobDetailResponse {
  job: Job;
  working_directory_status?: string;
  display_timezone?: string;
}

export type ScheduledSchedule =
  | { kind: "once"; at: string }
  | { kind: "daily"; time: string; timezone: string }
  | { kind: "periodic"; every: string; first_at?: string | null };

export interface ScheduledDependency { task_id: string; state?: string }

export interface ScheduledTask {
  task_id: string;
  name: string;
  cwd: string;
  command: string;
  retry: number;
  dependencies: ScheduledDependency[];
  depend_mode: "all" | "any" | string;
  sequence: number;
}

export interface ScheduledFlow {
  flow_id: string;
  name: string;
  owner: string;
  mode: string;
  schedule: ScheduledSchedule | null;
  tasks: ScheduledTask[];
  committed: boolean;
  frozen: boolean;
  enabled: boolean;
  graph_revision: number;
  schedule_generation: number;
  draft_revision: number;
  has_draft: boolean;
  queue_order?: number | null;
}

export interface ScheduledStandaloneJob {
  job: Job;
  definition: { mode: string; schedule: ScheduledSchedule | null; retry: number; enabled: boolean; generation: number };
  flow_id: string;
}

export interface ScheduledOverviewResponse {
  capacity: { max_concurrency: number; active_attempts: number };
  active_runs: ScheduledRunSummary[];
  next_occurrences: ScheduledOccurrence[];
  recent_failures: ScheduledRunSummary[];
}

export interface ScheduledRunSummary { run_id: string; flow_id: string; state: string; started_at?: string | null; finished_at?: string | null }
export interface ScheduledAttempt { attempt_id: string; number: number; state: string; exit_code?: number | null; failure_kind?: string | null; failure_detail?: string | null; started_at?: string | null; finished_at?: string | null }
export interface ScheduledTaskRun { task_id: string; state: string; attempt_count: number; next_attempt_at?: string | null; cancel_requested: boolean; attempts: ScheduledAttempt[] }
export interface ScheduledRun extends ScheduledRunSummary { generation: number; source: string; occurrence_id?: string | null; tasks: ScheduledTaskRun[] }
export interface ScheduledOccurrence { occurrence_id: string; flow_id: string; generation: number; due_at: string; state?: string; reason?: string | null; local_date?: string | null }
export interface ScheduledSourceState { mode: "manual" | "sync" | string; revision: number; hash: string }
export interface ScheduledSyncPreview { hash: string; changed: boolean; revision?: number; diff: { added: number; updated: number; removed: number; unchanged: number } }
export interface ScheduledSourceDocument { text: string; hash: string; value: unknown }
export interface ScheduledLogsState { runId: string; taskId: string; attempt: number | null; stream: "stdout" | "stderr"; data: LogsResponse | null; error: string | null }

export interface ScheduledWorkspaceState {
  overview: ScheduledOverviewResponse | null;
  flows: ScheduledFlow[];
  jobs: ScheduledStandaloneJob[];
  source: ScheduledSourceState | null;
  selectedFlow: ScheduledFlow | null;
  selectedJob: ScheduledStandaloneJob | null;
  activeTab: "flows" | "jobs";
  revisionConflict: boolean;
  runs: ScheduledRun[];
  selectedRun: ScheduledRun | null;
  logs: ScheduledLogsState;
  sourceDocument: ScheduledSourceDocument | null;
  syncPreview: ScheduledSyncPreview | null;
  concurrencyUnsafe: boolean;
}

export interface LogsResponse {
  stdout: string;
  stderr: string;
  stdout_available: boolean;
  stderr_available: boolean;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
  stdout_capture_error?: string | null;
  stderr_capture_error?: string | null;
  message?: string | null;
}

export interface DirectoryEntry {
  name: string;
  path: string;
  is_symlink?: boolean;
}

export interface DirectoryResponse {
  path: string;
  parent?: string | null;
  directories: DirectoryEntry[];
  truncated?: boolean;
  skipped_entries?: number;
}

export interface RootsResponse {
  default_path: string;
  locations?: DirectoryEntry[];
}

export interface JobDraft {
  user: string;
  name: string;
  cwd: string;
  command: string;
  description: string;
}

export interface LogsState {
  jobId: string;
  stream: "stdout" | "stderr";
  data: LogsResponse | null;
  error: string | null;
  search: string;
}

export interface WorkspaceState {
  mode: WorkspaceMode | null;
  modeTransition: ModeTransition | null;
  workspace: WorkspaceResponse | null;
  config: UiConfig | null;
  settings: SettingsResponse | null;
  policy: PolicyResponse | null;
  status: StatusResponse | null;
  timezone: TimezoneInfo | null;
  jobs: Job[];
  queue: QueueResponse;
  logs: LogsState;
  route: Route;
  filters: { search: string; user: string; state: string };
  pagination: { jobs: number; snapshots: number };
  loaded: boolean;
  loading: boolean;
  error: string | null;
  jobDraft: JobDraft;
  filesystem: {
    roots: RootsResponse | null;
    current: DirectoryResponse | null;
    inputPath: string;
    loading: boolean;
    error: string;
  };
  selectedJob: Job | null;
  selectedJobDetail: JobDetailResponse | null;
  configurationDraft: string | null;
  scheduled: ScheduledWorkspaceState;
}

export interface PageInfo<T> {
  page: number;
  totalPages: number;
  totalItems: number;
  start: number;
  end: number;
  items: T[];
}
