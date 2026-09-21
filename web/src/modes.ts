import { ApiError } from "./api.ts";
import type { Route, WorkspaceMode, WorkspaceState } from "./types.ts";
import { createScheduledState } from "./scheduled/state.ts";

export const SERIAL_ROUTES = ["overview", "jobs", "queue", "logs", "configuration", "policy"] as const;
export const SCHEDULED_ROUTES = ["overview", "workloads", "runs", "logs", "sources", "configuration", "policy"] as const;

export function routesForMode(mode: WorkspaceMode) {
  return mode === "scheduled" ? SCHEDULED_ROUTES : SERIAL_ROUTES;
}

export function isRouteAllowed(mode: WorkspaceMode, route: Route): boolean {
  return (routesForMode(mode) as readonly Route[]).includes(route);
}

export function modeFromApiError(error: unknown): WorkspaceMode | null {
  if (!(error instanceof ApiError) || error.status !== 409 || error.code !== "mode_changed") return null;
  const mode = typeof error.details === "object" && error.details !== null
    ? (error.details as { mode?: unknown }).mode
    : null;
  return mode === "serial" || mode === "scheduled" ? mode : null;
}

export function beginModeTransition(state: WorkspaceState, actualMode: WorkspaceMode): WorkspaceState {
  return {
    ...state,
    mode: null,
    modeTransition: { actualMode, deferred: false },
    workspace: null,
    config: null,
    settings: null,
    policy: null,
    status: null,
    timezone: null,
    jobs: [],
    queue: { jobs: [], locked: false },
    logs: { jobId: "", stream: "stdout", data: null, error: null, search: "" },
    route: "mode-change",
    filters: { search: "", user: "", state: "" },
    pagination: { jobs: 1, snapshots: 1 },
    loaded: false,
    loading: false,
    error: null,
    jobDraft: { user: "", name: "", cwd: "", command: "", description: "" },
    filesystem: { roots: null, current: null, inputPath: "", loading: false, error: "" },
    selectedJob: null,
    selectedJobDetail: null,
    configurationDraft: null,
    scheduled: createScheduledState(),
  };
}

export function deferModeTransition(state: WorkspaceState): WorkspaceState {
  if (!state.modeTransition) return state;
  return { ...state, route: "mode-change", modeTransition: { ...state.modeTransition, deferred: true } };
}
