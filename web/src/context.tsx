import { uiMessage, type UiMessage } from "./i18n/messages.ts";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createApiClient, type ApiClient } from "./api";
import { beginModeTransition, deferModeTransition, isRouteAllowed, modeFromApiError } from "./modes.ts";
import { useScheduledActions, type ConfirmationRequest } from "./scheduled/actions.ts";
import { scheduledApi } from "./scheduled/api.ts";
import { reduceScheduled } from "./scheduled/state.ts";
import { cacheDirectory, cachedDirectory, createRequestSequence, createState, JOBS_PAGE_SIZE, SNAPSHOTS_PAGE_SIZE, pageInfo, type DirectoryCacheEntry } from "./state";
import type { Job, JobDetailResponse, JobDraft, JobsResponse, LogsResponse, PolicyResponse, Route, ScheduledFlow, ScheduledRun, ScheduledStandaloneJob, SettingsResponse, StatusResponse, UiConfig, WorkspaceMode, WorkspaceState } from "./types";

export type Confirmation = ConfirmationRequest;

export interface WorkspaceActions {
  loadData: () => Promise<void>;
  acceptModeTransition: () => Promise<void>;
  deferModeTransition: () => void;
  checkModeTransition: () => Promise<void>;
  navigate: (route: Route) => void;
  setFilter: (key: "search" | "user" | "state", value: string) => void;
  setLogSearch: (value: string) => void;
  selectLogJob: (jobId: string) => Promise<void>;
  setLogStream: (stream: "stdout" | "stderr") => void;
  setPage: (kind: "jobs" | "snapshots", page: number) => void;
  openJobForm: () => Promise<void>;
  closeJobForm: () => void;
  openDirectoryBrowser: (path: string, onChoose?: (path: string) => void) => Promise<void>;
  closeDirectoryBrowser: () => void;
  updateDraft: (patch: Partial<JobDraft>) => void;
  loadDirectory: (path: string) => Promise<void>;
  setFilesystemInputPath: (path: string) => void;
  chooseDirectory: () => void;
  saveJob: () => Promise<void>;
  openJobDetail: (id: string) => Promise<void>;
  closeJobDetail: () => void;
  toggleDescriptionEdit: () => void;
  saveDescription: (description: string) => Promise<void>;
  copyJobId: (id: string) => Promise<void>;
  requestConfirmation: (confirmation: Confirmation) => Promise<boolean>;
  resolveConfirmation: (value: boolean) => void;
  cleanJobs: () => Promise<void>;
  queueLock: (locked: boolean) => Promise<void>;
  setWorkspaceMode: (mode: WorkspaceMode) => Promise<void>;
  moveQueueJob: (id: string, targetOrder: number) => Promise<void>;
  commitJob: (id: string) => Promise<void>;
  cancelJob: (id: string) => Promise<void>;
  viewJobLogs: (id: string) => Promise<void>;
  setConfigurationDraft: (value: string | null) => void;
  saveTimezone: (value: string) => Promise<void>;
  unsetTimezone: () => Promise<void>;
  createSnapshot: () => Promise<void>;
  restoreSnapshot: (path: string) => Promise<void>;
  savePolicy: (key: string, value: number) => Promise<boolean>;
  unsetPolicy: (key: string) => Promise<boolean>;
  selectScheduledTab: (tab: "flows" | "jobs") => void;
  createScheduledFlow: (flow: Pick<ScheduledFlow, "flow_id" | "name" | "owner" | "schedule">) => Promise<boolean>;
  openFlow: (flowId: string) => Promise<void>;
  closeScheduledFlow: () => void;
  deleteScheduledDraftFlow: (flow: ScheduledFlow) => Promise<boolean>;
  openScheduledJob: (jobId: string) => Promise<void>;
  scheduledFlowMutation: (flow: ScheduledFlow, path: string, method: string, body?: Record<string, unknown>) => Promise<boolean>;
  scheduledJobMutation: (job: ScheduledStandaloneJob, path: string, method: string, body?: Record<string, unknown>) => Promise<boolean>;
  loadScheduledRuns: () => Promise<void>;
  openScheduledRun: (runId: string) => Promise<void>;
  cancelScheduledRun: (run: ScheduledRun) => Promise<void>;
  cancelScheduledRunTask: (run: ScheduledRun, taskId: string) => Promise<void>;
  selectScheduledLogTarget: (runId: string, taskId: string, attempt: number | null) => Promise<void>;
  setScheduledLogStream: (stream: "stdout" | "stderr") => void;
  loadScheduledSourceFile: (file: File) => Promise<void>;
  previewScheduledSource: () => Promise<void>;
  syncScheduledSource: () => Promise<void>;
  setScheduledSourceMode: (mode: "manual" | "sync") => Promise<void>;
  exportScheduledSource: () => Promise<void>;
  snapshotScheduledSource: () => Promise<void>;
  saveScheduledConcurrency: (value: number) => Promise<boolean>;
  reconcileScheduledRecovery: (runId: string) => Promise<void>;
}

export interface WorkspaceContextValue {
  state: WorkspaceState;
  actions: WorkspaceActions;
  jobFormOpen: boolean;
  directoryBrowserOpen: boolean;
  jobDetailOpen: boolean;
  detailEditing: boolean;
  confirmation: Confirmation | null;
  toasts: Array<{ id: number; message: UiMessage; error: boolean }>;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function useWorkspace(): WorkspaceContextValue {
  const value = useContext(WorkspaceContext);
  if (!value) throw new Error("useWorkspace must be used inside WorkspaceProvider");
  return value;
}

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const initialRoute = (location.hash.slice(1) as Route) || "overview";
  const [state, setState] = useState(() => createState(initialRoute === "mode-change" ? "overview" : initialRoute));
  const stateRef = useRef(state);
  const sequence = useRef(createRequestSequence());
  const detailRequest = useRef(0);
  const directoryCache = useRef(new Map<string, DirectoryCacheEntry>());
  const directoryBrowserSelection = useRef<((path: string) => void) | null>(null);
  const [jobFormOpen, setJobFormOpen] = useState(false);
  const [directoryBrowserOpen, setDirectoryBrowserOpen] = useState(false);
  const [jobDetailOpen, setJobDetailOpen] = useState(false);
  const [detailEditing, setDetailEditing] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const resolver = useRef<((value: boolean) => void) | null>(null);
  const [toasts, setToasts] = useState<Array<{ id: number; message: UiMessage; error: boolean }>>([]);
  const toastSequence = useRef(0);
  const apiRef = useRef<ApiClient | null>(null);

  useEffect(() => { stateRef.current = state; }, [state]);

  const showToast = useCallback((message: UiMessage, error = false) => {
    const id = ++toastSequence.current;
    setToasts((current) => [...current, { id, message, error }]);
    window.setTimeout(() => setToasts((current) => current.filter((toast) => toast.id !== id)), 4200);
  }, []);

  if (!apiRef.current) apiRef.current = createApiClient();
  const api = apiRef.current;
  const scheduled = useMemo(() => scheduledApi(api), [api]);

  const enterModeTransition = useCallback((actualMode: WorkspaceMode) => {
    sequence.current.next();
    detailRequest.current += 1;
    directoryCache.current.clear();
    directoryBrowserSelection.current = null;
    setJobFormOpen(false);
    setDirectoryBrowserOpen(false);
    setJobDetailOpen(false);
    setDetailEditing(false);
    const pendingConfirmation = resolver.current;
    resolver.current = null;
    setConfirmation(null);
    pendingConfirmation?.(false);
    setState((current) => beginModeTransition(current, actualMode));
  }, []);

  const loadLogs = useCallback(async (jobId: string) => {
    if (!jobId) {
      setState((current) => ({ ...current, logs: { ...current.logs, data: null } }));
      return;
    }
    const mode = stateRef.current.mode;
    try {
      const data = await api.get<LogsResponse>(`/api/v1/jobs/${jobId}/logs`);
      setState((current) => current.mode === mode && current.logs.jobId === jobId ? { ...current, logs: { ...current.logs, data, error: null } } : current);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setState((current) => current.mode === mode && current.logs.jobId === jobId ? { ...current, logs: { ...current.logs, data: null, error: message } } : current);
    }
  }, [api, enterModeTransition]);

  const loadSerialData = useCallback(async (request: number, current: WorkspaceState) => {
    try {
      const configPromise = current.config ? Promise.resolve(current.config) : api.get<UiConfig>("/api/v1/ui/config");
      const [config, status, jobs, queue, settings, policy] = await Promise.all([
        configPromise,
        api.get<StatusResponse>("/api/v1/status"),
        api.get<JobsResponse>("/api/v1/jobs"),
        api.get<WorkspaceState["queue"]>("/api/v1/queue"),
        api.get<SettingsResponse>("/api/v1/config"),
        api.get<PolicyResponse>("/api/v1/policy"),
      ]);
      if (!sequence.current.isCurrent(request)) return;
      setState((value) => {
        if (value.mode !== "serial" || value.modeTransition) return value;
        const selectedJob = value.selectedJob ? jobs.jobs.find((job) => job.id === value.selectedJob?.id) || value.selectedJob : null;
        return {
          ...value,
          config,
          status,
          jobs: jobs.jobs || [],
          timezone: jobs.timezone || status?.timezone || value.workspace?.timezone || null,
          queue,
          settings,
          policy,
          selectedJob,
          loaded: true,
          loading: false,
        };
      });
      const logJobId = stateRef.current.logs.jobId;
      if (logJobId) await loadLogs(logJobId);
    } catch (error) {
      if (!sequence.current.isCurrent(request)) return;
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, loading: false, error: message }));
      if (current.loaded) showToast(message, true);
    }
  }, [api, enterModeTransition, loadLogs, showToast]);

  const loadScheduledData = useCallback(async (request: number, current: WorkspaceState) => {
    try {
      const configPromise = current.config ? Promise.resolve(current.config) : api.get<UiConfig>("/api/v1/ui/config");
      const [config, overview, flows, jobs, source, settings, policy] = await Promise.all([
        configPromise, scheduled.overview(), scheduled.flows(), scheduled.jobs(), scheduled.source(),
        api.get<SettingsResponse>("/api/v1/config"), api.get<PolicyResponse>("/api/v1/policy"),
      ]);
      if (!sequence.current.isCurrent(request)) return;
      setState((value) => value.mode !== "scheduled" || value.modeTransition ? value : {
        ...value, config, settings, policy, loaded: true, loading: false,
        scheduled: reduceScheduled(value.scheduled, { type: "loaded", value: { overview, flows: flows.flows || [], jobs: jobs.jobs || [], source } }),
      });
    } catch (error) {
      if (!sequence.current.isCurrent(request)) return;
      const actualMode = modeFromApiError(error);
      if (actualMode) { enterModeTransition(actualMode); return; }
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, loading: false, error: message }));
      if (current.loaded) showToast(message, true);
    }
  }, [api, enterModeTransition, scheduled, showToast]);

  const loadData = useCallback(async (acceptModeTransition = false) => {
    const request = sequence.current.next();
    const prior = stateRef.current;
    setState((value) => ({ ...value, loading: true, error: null }));
    try {
      const workspace = await api.workspace();
      if (!sequence.current.isCurrent(request)) return;
      const current = stateRef.current;
      const modeChanged = current.mode !== null && current.mode !== workspace.mode;
      if (modeChanged && !acceptModeTransition) {
        enterModeTransition(workspace.mode);
        return;
      }
      if (current.modeTransition && !acceptModeTransition) {
        setState((value) => ({ ...value, workspace, modeTransition: { actualMode: workspace.mode, deferred: value.modeTransition?.deferred ?? false }, loading: false, error: null }));
        return;
      }
      if (current.mode === null || modeChanged) {
        setState((value) => ({
          ...value,
          mode: workspace.mode,
          modeTransition: null,
          workspace,
          timezone: workspace.timezone,
          route: isRouteAllowed(workspace.mode, current.route) ? current.route : "overview",
          loaded: value.loaded,
          loading: true,
          error: null,
        }));
      } else {
        setState((value) => ({
          ...value,
          workspace,
          timezone: workspace.timezone,
          route: isRouteAllowed(workspace.mode, value.route) ? value.route : "overview",
          loaded: value.loaded,
          loading: true,
          error: null,
        }));
      }
      if (workspace.mode === "serial") await loadSerialData(request, current);
      else await loadScheduledData(request, current);
    } catch (error) {
      if (!sequence.current.isCurrent(request)) return;
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, loading: false, error: message }));
      if (prior.loaded) showToast(message, true);
    }
  }, [api, enterModeTransition, loadScheduledData, loadSerialData, showToast]);

  useEffect(() => {
    void loadData();
    const interval = window.setInterval(() => {
      if (document.visibilityState === "visible") void loadData();
    }, 2000);
    return () => window.clearInterval(interval);
  }, [loadData]);

  useEffect(() => {
      const onHashChange = () => {
      const next = location.hash.slice(1) as Route;
      setState((value) => ({
        ...value,
        route: value.modeTransition ? "mode-change" : value.mode && isRouteAllowed(value.mode, next) ? next : "overview",
      }));
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const navigate = useCallback((route: Route) => {
    const mode = stateRef.current.mode;
    if (mode && isRouteAllowed(mode, route)) location.hash = route;
  }, []);
  const setFilter = useCallback((key: "search" | "user" | "state", value: string) => setState((current) => ({ ...current, filters: { ...current.filters, [key]: value }, pagination: { ...current.pagination, jobs: 1 } })), []);
  const setLogSearch = useCallback((value: string) => setState((current) => ({ ...current, logs: { ...current.logs, search: value } })), []);
  const selectLogJob = useCallback(async (jobId: string) => {
    setState((current) => ({ ...current, logs: { ...current.logs, jobId, data: null, error: null } }));
    await loadLogs(jobId);
  }, [loadLogs]);
  const setLogStream = useCallback((stream: "stdout" | "stderr") => setState((current) => ({ ...current, logs: { ...current.logs, stream } })), []);
  const setPage = useCallback((kind: "jobs" | "snapshots", page: number) => setState((current) => ({ ...current, pagination: { ...current.pagination, [kind]: page } })), []);

  const openJobForm = useCallback(async () => {
    const current = stateRef.current;
    if (current.mode !== "serial") return;
    try {
      const roots = current.filesystem.roots || await api.get<WorkspaceState["filesystem"]["roots"]>("/api/v1/fs/roots");
      setState((value) => value.mode === "serial" ? { ...value, filesystem: { ...value.filesystem, roots, inputPath: value.jobDraft.cwd || roots?.default_path || "" }, jobDraft: { ...value.jobDraft, cwd: value.jobDraft.cwd || roots?.default_path || "" } } : value);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, filesystem: { ...value.filesystem, error: message } }));
    }
    setJobFormOpen(true);
  }, [api, enterModeTransition]);
  const closeJobForm = useCallback(() => { setJobFormOpen(false); directoryBrowserSelection.current = null; setDirectoryBrowserOpen(false); }, []);
  const updateDraft = useCallback((patch: Partial<JobDraft>) => setState((current) => ({ ...current, jobDraft: { ...current.jobDraft, ...patch } })), []);

  const loadDirectory = useCallback(async (path: string) => {
    if (!path) return;
    const mode = stateRef.current.mode;
    if (!mode) return;
    const cached = cachedDirectory(directoryCache.current, path);
    if (cached) {
      setState((current) => current.mode === mode ? { ...current, filesystem: { ...current.filesystem, current: cached, inputPath: cached?.path || path, loading: false, error: "" } } : current);
      return;
    }
    setState((current) => ({ ...current, filesystem: { ...current.filesystem, loading: true, error: "", inputPath: path } }));
    try {
      const value = await api.get<NonNullable<WorkspaceState["filesystem"]["current"]>>(`/api/v1/fs/directories?path=${encodeURIComponent(path)}`);
      cacheDirectory(directoryCache.current, value.path, value);
      setState((current) => current.mode === mode ? { ...current, filesystem: { ...current.filesystem, current: value, inputPath: value.path, loading: false } } : current);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setState((current) => current.mode === mode ? { ...current, filesystem: { ...current.filesystem, loading: false, error: message } } : current);
    }
  }, [api, enterModeTransition]);
  const openDirectoryBrowser = useCallback(async (path: string, onChoose?: (path: string) => void) => {
    const mode = stateRef.current.mode;
    if (!mode) return;
    directoryBrowserSelection.current = onChoose || null;
    let initialPath = path;
    if (!initialPath) {
      try {
        const current = stateRef.current;
        const roots = current.filesystem.roots || await api.get<WorkspaceState["filesystem"]["roots"]>("/api/v1/fs/roots");
        initialPath = roots?.default_path || "";
        setState((value) => value.mode === mode ? { ...value, filesystem: { ...value.filesystem, roots, inputPath: initialPath } } : value);
      } catch (error) {
        const actualMode = modeFromApiError(error);
        if (actualMode) {
          enterModeTransition(actualMode);
          return;
        }
        const message = error instanceof Error ? error.message : String(error);
        setState((value) => value.mode === mode ? { ...value, filesystem: { ...value.filesystem, error: message } } : value);
      }
    }
    setDirectoryBrowserOpen(true);
    await loadDirectory(initialPath);
  }, [api, enterModeTransition, loadDirectory]);
  const closeDirectoryBrowser = useCallback(() => { directoryBrowserSelection.current = null; setDirectoryBrowserOpen(false); }, []);
  const chooseDirectory = useCallback(() => {
    const path = stateRef.current.filesystem.current?.path;
    const onChoose = directoryBrowserSelection.current;
    directoryBrowserSelection.current = null;
    if (!path) {
      setDirectoryBrowserOpen(false);
      return;
    }
    onChoose?.(path);
    setState((current) => current.filesystem.current?.path === path ? { ...current, ...(onChoose ? {} : { jobDraft: { ...current.jobDraft, cwd: path } }), filesystem: { ...current.filesystem, roots: null, current: null } } : current);
    setDirectoryBrowserOpen(false);
  }, []);
  const setFilesystemInputPath = useCallback((path: string) => setState((current) => ({ ...current, filesystem: { ...current.filesystem, inputPath: path } })), []);

  const mutate = useCallback(async <T,>(path: string, method: string, body: unknown = null, message: UiMessage = uiMessage("toast.saved")): Promise<T | null> => {
    try {
      const result = await api.send<T>(path, method, body);
      showToast(message);
      await loadData();
      return result;
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return null;
      }
      showToast(error instanceof Error ? error.message : String(error), true);
      await loadData();
      return null;
    }
  }, [api, enterModeTransition, loadData, showToast]);

  const saveJob = useCallback(async () => {
    const current = stateRef.current;
    const created = await mutate<{ job: Job }>("/api/v1/jobs", "POST", current.jobDraft, uiMessage("toast.created"));
    if (created) {
      setState((value) => ({ ...value, jobDraft: { user: value.jobDraft.user, name: "", cwd: value.jobDraft.cwd, command: "", description: "" } }));
      setJobFormOpen(false);
    }
  }, [mutate]);

  const openJobDetail = useCallback(async (id: string) => {
    const mode = stateRef.current.mode;
    if (mode !== "serial") return;
    const request = ++detailRequest.current;
    setJobDetailOpen(true);
    setDetailEditing(false);
    setState((current) => ({ ...current, selectedJob: current.jobs.find((job) => job.id === id) || current.selectedJob, selectedJobDetail: null }));
    try {
      const detail = await api.get<JobDetailResponse>(`/api/v1/jobs/${id}`);
      if (request !== detailRequest.current) return;
      setState((current) => current.mode === mode ? { ...current, selectedJob: detail.job, selectedJobDetail: detail } : current);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) {
        enterModeTransition(actualMode);
        return;
      }
      showToast(error instanceof Error ? error.message : String(error), true);
    }
  }, [api, enterModeTransition, showToast]);
  const closeJobDetail = useCallback(() => { setJobDetailOpen(false); setDetailEditing(false); }, []);
  const toggleDescriptionEdit = useCallback(() => setDetailEditing((value) => !value), []);
  const saveDescription = useCallback(async (description: string) => {
    const job = stateRef.current.selectedJob;
    if (!job) return;
    const result = await mutate<JobDetailResponse>(`/api/v1/jobs/${job.id}/description`, "PATCH", { description, expected_revision: job.description_revision }, uiMessage("toast.description"));
    if (result) {
      setState((current) => ({ ...current, selectedJob: result.job, selectedJobDetail: current.selectedJobDetail ? { ...current.selectedJobDetail, job: result.job } : current.selectedJobDetail }));
      setDetailEditing(false);
    }
  }, [mutate]);
  const copyJobId = useCallback(async (id: string) => { await navigator.clipboard.writeText(id); showToast(uiMessage("toast.copied")); }, [showToast]);

  const requestConfirmation = useCallback((value: Confirmation) => new Promise<boolean>((resolve) => { resolver.current = resolve; setConfirmation(value); }), []);
  const resolveConfirmation = useCallback((value: boolean) => { const current = resolver.current; resolver.current = null; setConfirmation(null); current?.(value); }, []);
  const cleanJobs = useCallback(async () => { if (await requestConfirmation({ kicker: uiMessage("confirm.maintenance"), title: uiMessage("confirm.cleanTitle"), message: uiMessage("confirm.cleanMessage"), acceptLabel: uiMessage("confirm.cleanAccept"), destructive: true })) await mutate("/api/v1/clean", "POST", null, uiMessage("toast.cleaned")); }, [mutate, requestConfirmation]);
  const queueLock = useCallback(async (locked: boolean) => {
    if (!locked && !await requestConfirmation({ kicker: uiMessage("confirm.maintenance"), title: uiMessage("queue.unlock"), message: uiMessage("queue.unlockHelp"), acceptLabel: uiMessage("queue.unlock"), destructive: false })) return;
    await mutate(`/api/v1/queue/${locked ? "lock" : "unlock"}`, "POST");
  }, [mutate, requestConfirmation]);
  const setWorkspaceMode = useCallback(async (mode: WorkspaceMode) => {
    const current = stateRef.current;
    if (!current.mode || current.modeTransition || current.mode === mode) return;
    try {
      const result = await api.send<{ mode: WorkspaceMode }>("/api/v1/workspace/mode", "POST", { mode });
      if (result.mode !== mode) return;
      await loadData(true);
    } catch (error) {
      const actualMode = modeFromApiError(error);
      if (actualMode) { enterModeTransition(actualMode); return; }
      showToast(error instanceof Error ? error.message : String(error), true);
    }
  }, [api, enterModeTransition, loadData, showToast]);
  const moveQueueJob = useCallback(async (id: string, targetOrder: number) => { await mutate(`/api/v1/queue/${id}/move`, "POST", { target_order: targetOrder }); }, [mutate]);
  const commitJob = useCallback(async (id: string) => { if (await requestConfirmation({ kicker: uiMessage("confirm.jobAction"), title: uiMessage("confirm.commitTitle"), message: uiMessage("confirm.transition"), acceptLabel: uiMessage("jobs.commit"), destructive: false })) { await mutate(`/api/v1/jobs/${id}/commit`, "POST"); setJobDetailOpen(false); } }, [mutate, requestConfirmation]);
  const cancelJob = useCallback(async (id: string) => { if (await requestConfirmation({ kicker: uiMessage("confirm.jobAction"), title: uiMessage("confirm.cancelTitle"), message: uiMessage("confirm.transition"), acceptLabel: uiMessage("jobs.cancel"), destructive: true })) { await mutate(`/api/v1/jobs/${id}/cancel`, "POST"); setJobDetailOpen(false); } }, [mutate, requestConfirmation]);
  const viewJobLogs = useCallback(async (id: string) => { setState((current) => ({ ...current, logs: { ...current.logs, jobId: id } })); setJobDetailOpen(false); navigate("logs"); await loadLogs(id); }, [loadLogs, navigate]);
  const setConfigurationDraft = useCallback((value: string | null) => setState((current) => ({ ...current, configurationDraft: value })), []);
  const saveTimezone = useCallback(async (value: string) => { const result = await mutate<SettingsResponse>("/api/v1/config/timezone", "PUT", { value }, uiMessage("toast.timezone")); if (result) setState((current) => ({ ...current, configurationDraft: null })); }, [mutate]);
  const unsetTimezone = useCallback(async () => { const result = await mutate<SettingsResponse>("/api/v1/config/timezone", "DELETE", null, uiMessage("toast.systemTimezone")); if (result) setState((current) => ({ ...current, configurationDraft: null })); }, [mutate]);
  const createSnapshot = useCallback(async () => { await mutate("/api/v1/config/snapshot", "POST", null, uiMessage("toast.snapshot")); }, [mutate]);
  const restoreSnapshot = useCallback(async (path: string) => { if (await requestConfirmation({ kicker: uiMessage("confirm.snapshot"), title: uiMessage("confirm.restoreTitle"), message: uiMessage("confirm.restoreMessage"), acceptLabel: uiMessage("confirm.restoreAccept"), destructive: false })) await mutate("/api/v1/config/restore", "POST", { path }, uiMessage("toast.restored")); }, [mutate, requestConfirmation]);
  const savePolicy = useCallback(async (key: string, value: number) => Boolean(await mutate<PolicyResponse>(`/api/v1/policy/${key}`, "PUT", { value }, uiMessage("toast.policy"))), [mutate]);
  const unsetPolicy = useCallback(async (key: string) => Boolean(await mutate<PolicyResponse>(`/api/v1/policy/${key}`, "DELETE", null, uiMessage("toast.policyReset"))), [mutate]);
  const scheduledActions = useScheduledActions({ api, stateRef, setState, showToast, loadData, enterModeTransition, requestConfirmation });
  const acceptModeTransition = useCallback(() => loadData(true), [loadData]);
  const deferModeTransitionAction = useCallback(() => setState((current) => deferModeTransition(current)), []);
  const checkModeTransition = useCallback(() => loadData(false), [loadData]);
  const actions = useMemo<WorkspaceActions>(() => ({ loadData, acceptModeTransition, deferModeTransition: deferModeTransitionAction, checkModeTransition, navigate, setFilter, setLogSearch, selectLogJob, setLogStream, setPage, openJobForm, closeJobForm, openDirectoryBrowser, closeDirectoryBrowser, updateDraft, loadDirectory, setFilesystemInputPath, chooseDirectory, saveJob, openJobDetail, closeJobDetail, toggleDescriptionEdit, saveDescription, copyJobId, requestConfirmation, resolveConfirmation, cleanJobs, queueLock, setWorkspaceMode, moveQueueJob, commitJob, cancelJob, viewJobLogs, setConfigurationDraft, saveTimezone, unsetTimezone, createSnapshot, restoreSnapshot, savePolicy, unsetPolicy, ...scheduledActions }), [loadData, acceptModeTransition, deferModeTransitionAction, checkModeTransition, navigate, setFilter, setLogSearch, selectLogJob, setLogStream, setPage, openJobForm, closeJobForm, openDirectoryBrowser, closeDirectoryBrowser, updateDraft, loadDirectory, setFilesystemInputPath, chooseDirectory, saveJob, openJobDetail, closeJobDetail, toggleDescriptionEdit, saveDescription, copyJobId, requestConfirmation, resolveConfirmation, cleanJobs, queueLock, setWorkspaceMode, moveQueueJob, commitJob, cancelJob, viewJobLogs, setConfigurationDraft, saveTimezone, unsetTimezone, createSnapshot, restoreSnapshot, savePolicy, unsetPolicy, scheduledActions]);
  return <WorkspaceContext.Provider value={{ state, actions, jobFormOpen, directoryBrowserOpen, jobDetailOpen, detailEditing, confirmation, toasts }}>{children}</WorkspaceContext.Provider>;
}

export { JOBS_PAGE_SIZE, SNAPSHOTS_PAGE_SIZE, pageInfo };
