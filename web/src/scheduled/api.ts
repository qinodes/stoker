import type { ApiClient } from "../api.ts";
import type { LogsResponse, ScheduledFlow, ScheduledOverviewResponse, ScheduledRun, ScheduledRunSummary, ScheduledSourceState, ScheduledStandaloneJob, ScheduledSyncPreview } from "../types.ts";

export const scheduledApi = (api: ApiClient) => ({
  overview: () => api.get<ScheduledOverviewResponse>("/api/v1/scheduled/overview"),
  flows: () => api.get<{ flows: ScheduledFlow[] }>("/api/v1/scheduled/flows"),
  createFlow: (flow: Pick<ScheduledFlow, "flow_id" | "name" | "owner" | "schedule">) => api.send<{ flow: ScheduledFlow }>("/api/v1/scheduled/flows", "POST", flow),
  jobs: () => api.get<{ jobs: ScheduledStandaloneJob[] }>("/api/v1/scheduled/jobs"),
  source: () => api.get<ScheduledSourceState>("/api/v1/scheduled/sources"),
  flow: (id: string) => api.get<{ flow: ScheduledFlow }>(`/api/v1/scheduled/flows/${encodeURIComponent(id)}`),
  deleteDraftFlow: (id: string) => api.send<null>(`/api/v1/scheduled/flows/${encodeURIComponent(id)}`, "DELETE"),
  job: (id: string) => api.get<{ job: ScheduledStandaloneJob }>(`/api/v1/scheduled/jobs/${encodeURIComponent(id)}`),
  mutateFlow: (id: string, path: string, method: string, body?: unknown) => api.send<{ flow: ScheduledFlow }>(`/api/v1/scheduled/flows/${encodeURIComponent(id)}${path}`, method, body),
  mutateJob: (id: string, path: string, method: string, body?: unknown) => api.send<{ job: ScheduledStandaloneJob }>(`/api/v1/scheduled/jobs/${encodeURIComponent(id)}${path}`, method, body),
  flowRuns: (id: string) => api.get<{ runs: ScheduledRunSummary[] }>(`/api/v1/scheduled/flows/${encodeURIComponent(id)}/runs`),
  jobRuns: (id: string) => api.get<{ runs: ScheduledRunSummary[] }>(`/api/v1/scheduled/jobs/${encodeURIComponent(id)}/runs`),
  run: (id: string) => api.get<{ run: ScheduledRun }>(`/api/v1/scheduled/runs/${encodeURIComponent(id)}`),
  cancelRun: (id: string) => api.send<{ run: ScheduledRun }>(`/api/v1/scheduled/runs/${encodeURIComponent(id)}/cancel`, "POST"),
  cancelRunTask: (runId: string, taskId: string) => api.send<{ run: ScheduledRun }>(`/api/v1/scheduled/runs/${encodeURIComponent(runId)}/tasks/${encodeURIComponent(taskId)}/cancel`, "POST"),
  attemptLogs: (runId: string, taskId: string, attempt: number) => api.get<LogsResponse>(`/api/v1/scheduled/runs/${encodeURIComponent(runId)}/tasks/${encodeURIComponent(taskId)}/attempts/${attempt}/logs`),
  setMaxConcurrency: (value: number) => api.send<{ max_concurrency: number }>("/api/v1/scheduled/settings/max-concurrency", "PUT", { value }),
  reconcileRecovery: (runId: string) => api.send<{ run: ScheduledRun }>(`/api/v1/scheduled/recoveries/${encodeURIComponent(runId)}/reconcile`, "POST"),
  exportSource: () => api.get<{ document: unknown }>("/api/v1/scheduled/sources/export"),
  setSourceMode: (mode: "manual" | "sync") => api.send<ScheduledSourceState>("/api/v1/scheduled/sources/mode", "POST", { mode }),
  dryRunSource: (document: unknown) => api.send<ScheduledSyncPreview>("/api/v1/scheduled/sources/dry-run", "POST", { document }),
  syncSource: (document: unknown, confirmedHash: string) => api.send<ScheduledSyncPreview>("/api/v1/scheduled/sources/sync", "POST", { document, confirmed_hash: confirmedHash }),
  snapshotSource: () => api.send<{ document: unknown; path: string }>("/api/v1/scheduled/sources/snapshot", "POST"),
});
