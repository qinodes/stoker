import { useCallback, useMemo, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { ApiError, type ApiClient } from "../api";
import { uiMessage, type UiMessage } from "../i18n/messages.ts";
import { modeFromApiError } from "../modes.ts";
import { scheduledApi } from "./api.ts";
import { flowMutationRequest, reduceScheduled } from "./state.ts";
import type { ScheduledFlow, ScheduledRun, ScheduledSourceDocument, ScheduledStandaloneJob, WorkspaceMode, WorkspaceState } from "../types.ts";

export interface ConfirmationRequest {
  kicker: UiMessage;
  title: UiMessage;
  message: UiMessage;
  acceptLabel: UiMessage;
  destructive: boolean;
}

interface ScheduledActionDependencies {
  api: ApiClient;
  stateRef: MutableRefObject<WorkspaceState>;
  setState: Dispatch<SetStateAction<WorkspaceState>>;
  showToast: (message: UiMessage, error?: boolean) => void;
  loadData: () => Promise<void>;
  enterModeTransition: (mode: WorkspaceMode) => void;
  requestConfirmation: (confirmation: ConfirmationRequest) => Promise<boolean>;
}

type SaveFileWriter = { write: (data: string) => Promise<void>; close: () => Promise<void> };
type SaveFileHandle = { createWritable: () => Promise<SaveFileWriter> };
type SaveFilePicker = (options: { suggestedName: string; types: Array<{ description: string; accept: Record<string, string[]> }> }) => Promise<SaveFileHandle>;
type SaveFileWindow = Window & { showSaveFilePicker?: SaveFilePicker };

async function saveSourceCopy(text: string, suggestedName: string): Promise<"saved" | "downloaded" | "cancelled"> {
  const picker = (window as SaveFileWindow).showSaveFilePicker;
  if (picker) {
    try {
      const handle = await picker({ suggestedName, types: [{ description: "JSON source", accept: { "application/json": [".json"] } }] });
      const writable = await handle.createWritable();
      await writable.write(text);
      await writable.close();
      return "saved";
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") return "cancelled";
      if (!(error instanceof DOMException && (error.name === "NotAllowedError" || error.name === "SecurityError"))) throw error;
    }
  }
  const url = URL.createObjectURL(new Blob([text], { type: "application/json;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = suggestedName;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
  return "downloaded";
}

function snapshotName(path: string): string {
  return path.split(/[\\/]/).pop() || "stoker-flow-snapshot.json";
}

/** Scheduled-only mutations and reads, kept apart from the serial workspace controller. */
export function useScheduledActions({ api, stateRef, setState, showToast, loadData, enterModeTransition, requestConfirmation }: ScheduledActionDependencies) {
  const scheduled = useMemo(() => scheduledApi(api), [api]);
  const selectScheduledTab = useCallback((tab: "flows" | "jobs") => setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "tabSelected", tab }) } : current), [setState]);
  const createScheduledFlow = useCallback(async (flow: Pick<ScheduledFlow, "flow_id" | "name" | "owner" | "schedule">) => {
    try {
      const { flow: created } = await scheduled.createFlow(flow);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "flowLoaded", flow: created }) } : current);
      showToast(uiMessage("toast.saved"));
      return true;
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); return false; }
  }, [scheduled, setState, showToast]);
  const openFlow = useCallback(async (flowId: string) => {
    if (stateRef.current.mode !== "scheduled") return;
    try {
      const { flow } = await scheduled.flow(flowId);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "flowLoaded", flow }) } : current);
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [scheduled, setState, showToast, stateRef]);
  const closeScheduledFlow = useCallback(() => setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "flowClosed" }) } : current), [setState]);
  const deleteScheduledDraftFlow = useCallback(async (flow: ScheduledFlow) => {
    if (!await requestConfirmation({ kicker: uiMessage("confirm.scheduledFlow"), title: uiMessage("confirm.deleteFlowTitle"), message: uiMessage("confirm.deleteFlowMessage"), acceptLabel: uiMessage("scheduled.flow.delete"), destructive: true })) return false;
    try {
      await scheduled.deleteDraftFlow(flow.flow_id);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: { ...current.scheduled, flows: current.scheduled.flows.filter((item) => item.flow_id !== flow.flow_id), selectedFlow: null } } : current);
      showToast(uiMessage("toast.flowDeleted"));
      return true;
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); return false; }
  }, [requestConfirmation, scheduled, setState, showToast]);
  const openScheduledJob = useCallback(async (jobId: string) => {
    if (stateRef.current.mode !== "scheduled") return;
    try {
      const { job } = await scheduled.job(jobId);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "jobLoaded", job }) } : current);
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [scheduled, setState, showToast, stateRef]);
  const scheduledFlowMutation = useCallback(async (flow: ScheduledFlow, path: string, method: string, body: Record<string, unknown> = {}) => {
    try {
      const { flow: next } = await scheduled.mutateFlow(flow.flow_id, path, method, flowMutationRequest(flow, body));
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "flowLoaded", flow: next }) } : current);
      showToast(uiMessage("toast.saved"));
      return true;
    } catch (error) {
      const isConflict = error instanceof ApiError && error.status === 409;
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "revisionConflict", value: isConflict }) } : current);
      showToast(error instanceof Error ? error.message : String(error), true);
      return false;
    }
  }, [scheduled, setState, showToast]);
  const scheduledJobMutation = useCallback(async (job: ScheduledStandaloneJob, path: string, method: string, body: Record<string, unknown> = {}) => {
    try {
      const { job: next } = await scheduled.mutateJob(job.job.id, path, method, body);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "jobLoaded", job: next }) } : current);
      showToast(uiMessage("toast.saved"));
      return true;
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); return false; }
  }, [scheduled, setState, showToast]);
  const loadScheduledRuns = useCallback(async () => {
    const current = stateRef.current;
    if (current.mode !== "scheduled" || current.modeTransition) return;
    try {
      const histories = await Promise.all([...current.scheduled.flows.map((flow) => scheduled.flowRuns(flow.flow_id)), ...current.scheduled.jobs.map((job) => scheduled.jobRuns(job.job.id))]);
      const summaries = histories.flatMap((history) => history.runs || []);
      const runs = (await Promise.all(summaries.map((run) => scheduled.run(run.run_id)))).map((item) => item.run);
      setState((value) => value.mode === "scheduled" && !value.modeTransition ? { ...value, scheduled: reduceScheduled(value.scheduled, { type: "runsLoaded", runs }) } : value);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) { enterModeTransition(actualMode); return; }
      showToast(error instanceof Error ? error.message : String(error), true);
    }
  }, [enterModeTransition, scheduled, setState, showToast, stateRef]);
  const openScheduledRun = useCallback(async (runId: string) => {
    if (stateRef.current.mode !== "scheduled") return;
    try {
      const { run } = await scheduled.run(runId);
      setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "runLoaded", run }) } : current);
    } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [scheduled, setState, showToast, stateRef]);
  const cancelScheduledRun = useCallback(async (run: ScheduledRun) => {
    if (!await requestConfirmation({ kicker: uiMessage("confirm.scheduledRun"), title: uiMessage("confirm.cancelRunTitle"), message: uiMessage("confirm.transition"), acceptLabel: uiMessage("scheduled.runs.cancel"), destructive: true })) return;
    try { const { run: next } = await scheduled.cancelRun(run.run_id); setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "runLoaded", run: next }) } : current); showToast(uiMessage("toast.saved")); }
    catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [requestConfirmation, scheduled, setState, showToast]);
  const cancelScheduledRunTask = useCallback(async (run: ScheduledRun, taskId: string) => {
    if (!await requestConfirmation({ kicker: uiMessage("confirm.scheduledRun"), title: uiMessage("confirm.cancelRunTaskTitle"), message: uiMessage("confirm.transition"), acceptLabel: uiMessage("scheduled.runs.cancelTask"), destructive: true })) return;
    try { const { run: next } = await scheduled.cancelRunTask(run.run_id, taskId); setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "runLoaded", run: next }) } : current); showToast(uiMessage("toast.saved")); }
    catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [requestConfirmation, scheduled, setState, showToast]);
  const selectScheduledLogTarget = useCallback(async (runId: string, taskId: string, attempt: number | null) => {
    setState((current) => current.mode === "scheduled" ? { ...current, scheduled: { ...current.scheduled, logs: { ...current.scheduled.logs, runId, taskId, attempt, data: null, error: null } } } : current);
    if (!runId || !taskId || attempt === null) return;
    try { const data = await scheduled.attemptLogs(runId, taskId, attempt); setState((current) => current.mode === "scheduled" && current.scheduled.logs.runId === runId && current.scheduled.logs.taskId === taskId && current.scheduled.logs.attempt === attempt ? { ...current, scheduled: { ...current.scheduled, logs: { ...current.scheduled.logs, data, error: null } } } : current); }
    catch (error) { const message = error instanceof Error ? error.message : String(error); setState((current) => current.mode === "scheduled" ? { ...current, scheduled: { ...current.scheduled, logs: { ...current.scheduled.logs, error: message } } } : current); }
  }, [scheduled, setState]);
  const setScheduledLogStream = useCallback((stream: "stdout" | "stderr") => setState((current) => current.mode === "scheduled" ? { ...current, scheduled: { ...current.scheduled, logs: { ...current.scheduled.logs, stream } } } : current), [setState]);
  const hashSourceText = useCallback(async (text: string) => {
    const bytes = new TextEncoder().encode(text);
    const digest = await crypto.subtle.digest("SHA-256", bytes);
    return `sha256:${Array.from(new Uint8Array(digest)).map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }, []);
  const loadScheduledSourceFile = useCallback(async (file: File) => {
    try { const text = await file.text(); const value = JSON.parse(text) as unknown; const document: ScheduledSourceDocument = { text, hash: await hashSourceText(text), value }; setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "sourceDocumentLoaded", document }) } : current); }
    catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [hashSourceText, setState, showToast]);
  const previewScheduledSource = useCallback(async () => {
    const document = stateRef.current.scheduled.sourceDocument;
    if (!document) return;
    try { const preview = await scheduled.dryRunSource(document.value); setState((current) => current.mode === "scheduled" && current.scheduled.sourceDocument?.hash === document.hash ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "syncPreviewLoaded", preview }) } : current); }
    catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [scheduled, setState, showToast, stateRef]);
  const syncScheduledSource = useCallback(async () => {
    const { sourceDocument, syncPreview } = stateRef.current.scheduled;
    if (!sourceDocument || !syncPreview) return;
    try { await scheduled.syncSource(sourceDocument.value, syncPreview.hash); setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "sourceImportCleared" }) } : current); showToast(uiMessage("toast.scheduledSync")); await loadData(); }
    catch (error) { setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "syncPreviewLoaded", preview: null }) } : current); showToast(error instanceof Error ? error.message : String(error), true); }
  }, [loadData, scheduled, setState, showToast, stateRef]);
  const setScheduledSourceMode = useCallback(async (mode: "manual" | "sync") => { try { await scheduled.setSourceMode(mode); await loadData(); } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); } }, [loadData, scheduled, showToast]);
  const saveSourceSnapshot = useCallback(async () => {
    const { document: value, path } = await scheduled.snapshotSource();
    const text = `${JSON.stringify(value, null, 2)}\n`;
    showToast(uiMessage("toast.sourceSnapshotSaved"));
    const result = await saveSourceCopy(text, snapshotName(path));
    showToast(uiMessage(result === "saved" ? "toast.sourceCopySaved" : result === "downloaded" ? "toast.sourceCopyDownloaded" : "toast.sourceCopyCancelled"));
  }, [scheduled, showToast]);
  const exportScheduledSource = useCallback(async () => { try { await saveSourceSnapshot(); } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); } }, [saveSourceSnapshot, showToast]);
  const snapshotScheduledSource = useCallback(async () => { try { await saveSourceSnapshot(); } catch (error) { showToast(error instanceof Error ? error.message : String(error), true); } }, [saveSourceSnapshot, showToast]);
  const saveScheduledConcurrency = useCallback(async (value: number) => {
    if (!Number.isSafeInteger(value) || value < 1) return false;
    try { await scheduled.setMaxConcurrency(value); showToast(uiMessage("toast.saved")); await loadData(); return true; }
    catch (error) { const unsafe = error instanceof ApiError && error.status === 409; setState((current) => current.mode === "scheduled" ? { ...current, scheduled: reduceScheduled(current.scheduled, { type: "concurrencyUnsafe", value: unsafe }) } : current); showToast(error instanceof Error ? error.message : String(error), true); return false; }
  }, [loadData, scheduled, setState, showToast]);
  const reconcileScheduledRecovery = useCallback(async (runId: string) => {
    if (!await requestConfirmation({ kicker: uiMessage("confirm.recovery"), title: uiMessage("confirm.recoveryTitle"), message: uiMessage("confirm.recoveryMessage"), acceptLabel: uiMessage("scheduled.recovery.reconcile"), destructive: false })) return;
    try { await scheduled.reconcileRecovery(runId); showToast(uiMessage("toast.saved")); await loadData(); }
    catch (error) { showToast(error instanceof Error ? error.message : String(error), true); }
  }, [loadData, requestConfirmation, scheduled, showToast]);
  return { selectScheduledTab, createScheduledFlow, openFlow, closeScheduledFlow, deleteScheduledDraftFlow, openScheduledJob, scheduledFlowMutation, scheduledJobMutation, loadScheduledRuns, openScheduledRun, cancelScheduledRun, cancelScheduledRunTask, selectScheduledLogTarget, setScheduledLogStream, loadScheduledSourceFile, previewScheduledSource, syncScheduledSource, setScheduledSourceMode, exportScheduledSource, snapshotScheduledSource, saveScheduledConcurrency, reconcileScheduledRecovery };
}
