import type { ScheduledFlow, ScheduledRun, ScheduledSourceDocument, ScheduledStandaloneJob, ScheduledSyncPreview, ScheduledWorkspaceState } from "../types.ts";

export type ScheduledAction =
  | { type: "loaded"; value: Pick<ScheduledWorkspaceState, "overview" | "flows" | "jobs" | "source"> }
  | { type: "flowLoaded"; flow: ScheduledFlow }
  | { type: "jobLoaded"; job: ScheduledStandaloneJob }
  | { type: "tabSelected"; tab: ScheduledWorkspaceState["activeTab"] }
  | { type: "revisionConflict"; value: boolean }
  | { type: "runsLoaded"; runs: ScheduledRun[] }
  | { type: "runLoaded"; run: ScheduledRun }
  | { type: "sourceDocumentLoaded"; document: ScheduledSourceDocument }
  | { type: "syncPreviewLoaded"; preview: ScheduledSyncPreview | null }
  | { type: "concurrencyUnsafe"; value: boolean };

export function createScheduledState(): ScheduledWorkspaceState {
  return { overview: null, flows: [], jobs: [], source: null, selectedFlow: null, selectedJob: null, activeTab: "flows", revisionConflict: false, runs: [], selectedRun: null, logs: { runId: "", taskId: "", attempt: null, stream: "stdout", data: null, error: null }, sourceDocument: null, syncPreview: null, concurrencyUnsafe: false };
}

export function reduceScheduled(state: ScheduledWorkspaceState, action: ScheduledAction): ScheduledWorkspaceState {
  switch (action.type) {
    case "loaded": return { ...state, ...action.value };
    case "flowLoaded": return { ...state, selectedFlow: action.flow, flows: replace(state.flows, action.flow, "flow_id") };
    case "jobLoaded": return { ...state, selectedJob: action.job, jobs: replace(state.jobs, action.job, "flow_id") };
    case "tabSelected": return { ...state, activeTab: action.tab };
    case "revisionConflict": return { ...state, revisionConflict: action.value };
    case "runsLoaded": return { ...state, runs: action.runs, selectedRun: state.selectedRun ? action.runs.find((run) => run.run_id === state.selectedRun?.run_id) || state.selectedRun : null };
    case "runLoaded": return { ...state, selectedRun: action.run, runs: replace(state.runs, action.run, "run_id") };
    case "sourceDocumentLoaded": return { ...state, sourceDocument: action.document, syncPreview: null };
    case "syncPreviewLoaded": return { ...state, syncPreview: action.preview };
    case "concurrencyUnsafe": return { ...state, concurrencyUnsafe: action.value };
  }
}

function replace<T extends Record<K, string>, K extends string>(items: T[], value: T, key: K) {
  return items.some((item) => item[key] === value[key]) ? items.map((item) => item[key] === value[key] ? value : item) : [...items, value];
}

export function flowMutationRequest(flow: ScheduledFlow, body: Record<string, unknown> = {}) {
  return { ...body, expected_draft_revision: flow.draft_revision };
}

export function withSyncPreview(state: ScheduledWorkspaceState, preview: ScheduledSyncPreview): ScheduledWorkspaceState {
  return reduceScheduled(state, { type: "syncPreviewLoaded", preview });
}

export function canConfirmSync(state: ScheduledWorkspaceState, confirmedHash: string): boolean {
  return state.syncPreview?.hash === confirmedHash;
}

export const scheduledSelectors = {
  isSyncSource: (state: ScheduledWorkspaceState) => state.source?.mode === "sync",
  canEditFlow: (state: ScheduledWorkspaceState, flow: ScheduledFlow) => !scheduledSelectors.isSyncSource(state) && flow.frozen,
  flowActionState: (flow: ScheduledFlow) => !flow.committed ? "draft" : flow.frozen ? "frozen" : "committed",
};
