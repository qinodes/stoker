export type Route = "overview" | "jobs" | "queue" | "logs" | "configuration";

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
  auth_required?: boolean;
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

export interface JobDetailResponse {
  job: Job;
  working_directory_status?: string;
  display_timezone?: string;
}

export interface LogsResponse {
  stdout: string;
  stderr: string;
  stdout_available: boolean;
  stderr_available: boolean;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
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
  config: UiConfig | null;
  settings: SettingsResponse | null;
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
}

export interface PageInfo<T> {
  page: number;
  totalPages: number;
  totalItems: number;
  start: number;
  end: number;
  items: T[];
}
