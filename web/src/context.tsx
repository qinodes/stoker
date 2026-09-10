import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createApiClient, type ApiClient } from "./api";
import { cacheDirectory, cachedDirectory, createState, JOBS_PAGE_SIZE, SNAPSHOTS_PAGE_SIZE, pageInfo, type DirectoryCacheEntry } from "./state";
import type { Job, JobDetailResponse, JobDraft, JobsResponse, LogsResponse, Route, SettingsResponse, StatusResponse, UiConfig, WorkspaceState } from "./types";

const ROUTES: Route[] = ["overview", "jobs", "queue", "logs", "configuration"];

export interface Confirmation {
  kicker: string;
  title: string;
  message: string;
  acceptLabel: string;
  destructive: boolean;
}

export interface WorkspaceActions {
  loadData: () => Promise<void>;
  navigate: (route: Route) => void;
  setFilter: (key: "search" | "user" | "state", value: string) => void;
  setLogSearch: (value: string) => void;
  selectLogJob: (jobId: string) => Promise<void>;
  setLogStream: (stream: "stdout" | "stderr") => void;
  setPage: (kind: "jobs" | "snapshots", page: number) => void;
  openJobForm: () => Promise<void>;
  closeJobForm: () => void;
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
  moveQueueJob: (id: string, targetOrder: number) => Promise<void>;
  commitJob: (id: string) => Promise<void>;
  cancelJob: (id: string) => Promise<void>;
  viewJobLogs: (id: string) => Promise<void>;
  setConfigurationDraft: (value: string | null) => void;
  saveTimezone: (value: string) => Promise<void>;
  unsetTimezone: () => Promise<void>;
  createSnapshot: () => Promise<void>;
  restoreSnapshot: (path: string) => Promise<void>;
  connectToken: (token: string) => Promise<void>;
  dismissToken: () => void;
}

export interface WorkspaceContextValue {
  state: WorkspaceState;
  actions: WorkspaceActions;
  jobFormOpen: boolean;
  jobDetailOpen: boolean;
  detailEditing: boolean;
  tokenOpen: boolean;
  confirmation: Confirmation | null;
  toasts: Array<{ id: number; message: string; error: boolean }>;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

export function useWorkspace(): WorkspaceContextValue {
  const value = useContext(WorkspaceContext);
  if (!value) throw new Error("useWorkspace must be used inside WorkspaceProvider");
  return value;
}

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const initialRoute = (location.hash.slice(1) as Route) || "overview";
  const [state, setState] = useState(() => createState(ROUTES.includes(initialRoute) ? initialRoute : "overview"));
  const stateRef = useRef(state);
  const sequence = useRef(0);
  const detailRequest = useRef(0);
  const directoryCache = useRef(new Map<string, DirectoryCacheEntry>());
  const [jobFormOpen, setJobFormOpen] = useState(false);
  const [jobDetailOpen, setJobDetailOpen] = useState(false);
  const [detailEditing, setDetailEditing] = useState(false);
  const [tokenOpen, setTokenOpen] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const resolver = useRef<((value: boolean) => void) | null>(null);
  const [toasts, setToasts] = useState<Array<{ id: number; message: string; error: boolean }>>([]);
  const toastSequence = useRef(0);
  const apiRef = useRef<ApiClient | null>(null);

  useEffect(() => { stateRef.current = state; }, [state]);

  const showToast = useCallback((message: string, error = false) => {
    const id = ++toastSequence.current;
    setToasts((current) => [...current, { id, message, error }]);
    window.setTimeout(() => setToasts((current) => current.filter((toast) => toast.id !== id)), 4200);
  }, []);

  if (!apiRef.current) apiRef.current = createApiClient({ onUnauthorized: () => setTokenOpen(true) });
  const api = apiRef.current;

  const loadLogs = useCallback(async (jobId: string) => {
    if (!jobId) {
      setState((current) => ({ ...current, logs: { ...current.logs, data: null } }));
      return;
    }
    try {
      const data = await api.get<LogsResponse>(`/api/v1/jobs/${jobId}/logs`);
      setState((current) => current.logs.jobId === jobId ? { ...current, logs: { ...current.logs, data, error: null } } : current);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setState((current) => current.logs.jobId === jobId ? { ...current, logs: { ...current.logs, data: null, error: message } } : current);
    }
  }, [api]);

  const loadData = useCallback(async () => {
    const request = ++sequence.current;
    const current = stateRef.current;
    setState((value) => ({ ...value, loading: true, error: null }));
    try {
      const configPromise = current.config ? Promise.resolve(current.config) : api.get<UiConfig>("/api/v1/ui/config");
      const [config, status, jobs, queue, settings] = await Promise.all([
        configPromise,
        api.get<StatusResponse>("/api/v1/status"),
        api.get<JobsResponse>("/api/v1/jobs"),
        api.get<WorkspaceState["queue"]>("/api/v1/queue"),
        api.get<SettingsResponse>("/api/v1/config"),
      ]);
      if (request !== sequence.current) return;
      setState((value) => {
        const selectedJob = value.selectedJob ? jobs.jobs.find((job) => job.id === value.selectedJob?.id) || value.selectedJob : null;
        return {
          ...value,
          config,
          status,
          jobs: jobs.jobs || [],
          timezone: jobs.timezone || status?.timezone || null,
          queue,
          settings,
          selectedJob,
          loaded: true,
          loading: false,
        };
      });
      const logJobId = stateRef.current.logs.jobId;
      if (logJobId) await loadLogs(logJobId);
    } catch (error) {
      if (request !== sequence.current) return;
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, loading: false, error: message }));
      if (current.loaded) showToast(message, true);
    }
  }, [api, loadLogs, showToast]);

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
      setState((value) => ({ ...value, route: ROUTES.includes(next) ? next : "overview" }));
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const navigate = useCallback((route: Route) => { location.hash = route; }, []);
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
    try {
      const roots = current.filesystem.roots || await api.get<WorkspaceState["filesystem"]["roots"]>("/api/v1/fs/roots");
      setState((value) => ({ ...value, filesystem: { ...value.filesystem, roots, inputPath: value.jobDraft.cwd || roots?.default_path || "" }, jobDraft: { ...value.jobDraft, cwd: value.jobDraft.cwd || roots?.default_path || "" } }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setState((value) => ({ ...value, filesystem: { ...value.filesystem, error: message } }));
    }
    setJobFormOpen(true);
  }, [api]);
  const closeJobForm = useCallback(() => setJobFormOpen(false), []);
  const updateDraft = useCallback((patch: Partial<JobDraft>) => setState((current) => ({ ...current, jobDraft: { ...current.jobDraft, ...patch } })), []);

  const loadDirectory = useCallback(async (path: string) => {
    if (!path) return;
    const cached = cachedDirectory(directoryCache.current, path);
    if (cached) {
      setState((current) => ({ ...current, filesystem: { ...current.filesystem, current: cached, inputPath: cached?.path || path, loading: false, error: "" } }));
      return;
    }
    setState((current) => ({ ...current, filesystem: { ...current.filesystem, loading: true, error: "", inputPath: path } }));
    try {
      const value = await api.get<NonNullable<WorkspaceState["filesystem"]["current"]>>(`/api/v1/fs/directories?path=${encodeURIComponent(path)}`);
      cacheDirectory(directoryCache.current, value.path, value);
      setState((current) => ({ ...current, filesystem: { ...current.filesystem, current: value, inputPath: value.path, loading: false } }));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setState((current) => ({ ...current, filesystem: { ...current.filesystem, loading: false, error: message } }));
    }
  }, [api]);
  const chooseDirectory = useCallback(() => setState((current) => current.filesystem.current ? { ...current, jobDraft: { ...current.jobDraft, cwd: current.filesystem.current.path }, filesystem: { ...current.filesystem, roots: null, current: null } } : current), []);
  const setFilesystemInputPath = useCallback((path: string) => setState((current) => ({ ...current, filesystem: { ...current.filesystem, inputPath: path } })), []);

  const mutate = useCallback(async <T,>(path: string, method: string, body: unknown = null, message = "Change saved to the server."): Promise<T | null> => {
    try {
      const result = await api.send<T>(path, method, body);
      showToast(message);
      await loadData();
      return result;
    } catch (error) {
      showToast(error instanceof Error ? error.message : String(error), true);
      await loadData();
      return null;
    }
  }, [api, loadData, showToast]);

  const saveJob = useCallback(async () => {
    const current = stateRef.current;
    const created = await mutate<{ job: Job }>("/api/v1/jobs", "POST", current.jobDraft, "Job created as DRAFT.");
    if (created) {
      setState((value) => ({ ...value, jobDraft: { user: value.jobDraft.user, name: "", cwd: value.jobDraft.cwd, command: "", description: "" } }));
      setJobFormOpen(false);
    }
  }, [mutate]);

  const openJobDetail = useCallback(async (id: string) => {
    const request = ++detailRequest.current;
    setJobDetailOpen(true);
    setDetailEditing(false);
    setState((current) => ({ ...current, selectedJob: current.jobs.find((job) => job.id === id) || current.selectedJob, selectedJobDetail: null }));
    try {
      const detail = await api.get<JobDetailResponse>(`/api/v1/jobs/${id}`);
      if (request !== detailRequest.current) return;
      setState((current) => ({ ...current, selectedJob: detail.job, selectedJobDetail: detail }));
    } catch (error) {
      showToast(error instanceof Error ? error.message : String(error), true);
    }
  }, [api, showToast]);
  const closeJobDetail = useCallback(() => { setJobDetailOpen(false); setDetailEditing(false); }, []);
  const toggleDescriptionEdit = useCallback(() => setDetailEditing((value) => !value), []);
  const saveDescription = useCallback(async (description: string) => {
    const job = stateRef.current.selectedJob;
    if (!job) return;
    const result = await mutate<JobDetailResponse>(`/api/v1/jobs/${job.id}/description`, "PATCH", { description, expected_revision: job.description_revision }, "Description updated.");
    if (result) {
      setState((current) => ({ ...current, selectedJob: result.job, selectedJobDetail: current.selectedJobDetail ? { ...current.selectedJobDetail, job: result.job } : current.selectedJobDetail }));
      setDetailEditing(false);
    }
  }, [mutate]);
  const copyJobId = useCallback(async (id: string) => { await navigator.clipboard.writeText(id); showToast("Job ID copied."); }, [showToast]);

  const requestConfirmation = useCallback((value: Confirmation) => new Promise<boolean>((resolve) => { resolver.current = resolve; setConfirmation(value); }), []);
  const resolveConfirmation = useCallback((value: boolean) => { const current = resolver.current; resolver.current = null; setConfirmation(null); current?.(value); }, []);
  const cleanJobs = useCallback(async () => { if (await requestConfirmation({ kicker: "Maintenance", title: "Clean terminal jobs?", message: "This removes terminal job history and its run artifacts.", acceptLabel: "Clean jobs", destructive: true })) await mutate("/api/v1/clean", "POST", null, "Terminal job history cleaned."); }, [mutate, requestConfirmation]);
  const queueLock = useCallback(async (locked: boolean) => { await mutate(`/api/v1/queue/${locked ? "lock" : "unlock"}`, "POST"); }, [mutate]);
  const moveQueueJob = useCallback(async (id: string, targetOrder: number) => { await mutate(`/api/v1/queue/${id}/move`, "POST", { target_order: targetOrder }); }, [mutate]);
  const commitJob = useCallback(async (id: string) => { if (await requestConfirmation({ kicker: "Job action", title: "Commit this job?", message: "The scheduler will apply this state transition.", acceptLabel: "Commit job", destructive: false })) { await mutate(`/api/v1/jobs/${id}/commit`, "POST"); setJobDetailOpen(false); } }, [mutate, requestConfirmation]);
  const cancelJob = useCallback(async (id: string) => { if (await requestConfirmation({ kicker: "Job action", title: "Cancel this job?", message: "The scheduler will apply this state transition.", acceptLabel: "Cancel job", destructive: true })) { await mutate(`/api/v1/jobs/${id}/cancel`, "POST"); setJobDetailOpen(false); } }, [mutate, requestConfirmation]);
  const viewJobLogs = useCallback(async (id: string) => { setState((current) => ({ ...current, logs: { ...current.logs, jobId: id } })); setJobDetailOpen(false); navigate("logs"); await loadLogs(id); }, [loadLogs, navigate]);
  const setConfigurationDraft = useCallback((value: string | null) => setState((current) => ({ ...current, configurationDraft: value })), []);
  const saveTimezone = useCallback(async (value: string) => { const result = await mutate<SettingsResponse>("/api/v1/config/timezone", "PUT", { value }, "Timezone updated."); if (result) setState((current) => ({ ...current, configurationDraft: null })); }, [mutate]);
  const unsetTimezone = useCallback(async () => { const result = await mutate<SettingsResponse>("/api/v1/config/timezone", "DELETE", null, "System timezone enabled."); if (result) setState((current) => ({ ...current, configurationDraft: null })); }, [mutate]);
  const createSnapshot = useCallback(async () => { await mutate("/api/v1/config/snapshot", "POST", null, "Configuration snapshot created."); }, [mutate]);
  const restoreSnapshot = useCallback(async (path: string) => { if (await requestConfirmation({ kicker: "Configuration snapshot", title: "Restore this snapshot?", message: "The current configuration will be preserved before restore.", acceptLabel: "Restore snapshot", destructive: false })) await mutate("/api/v1/config/restore", "POST", { path }, "Configuration restored."); }, [mutate, requestConfirmation]);
  const connectToken = useCallback(async (token: string) => { if (!token.trim()) return; sessionStorage.setItem("stoker-ui-token", token.trim()); setTokenOpen(false); await loadData(); }, [loadData]);
  const dismissToken = useCallback(() => setTokenOpen(false), []);

  const actions = useMemo<WorkspaceActions>(() => ({ loadData, navigate, setFilter, setLogSearch, selectLogJob, setLogStream, setPage, openJobForm, closeJobForm, updateDraft, loadDirectory, setFilesystemInputPath, chooseDirectory, saveJob, openJobDetail, closeJobDetail, toggleDescriptionEdit, saveDescription, copyJobId, requestConfirmation, resolveConfirmation, cleanJobs, queueLock, moveQueueJob, commitJob, cancelJob, viewJobLogs, setConfigurationDraft, saveTimezone, unsetTimezone, createSnapshot, restoreSnapshot, connectToken, dismissToken }), [loadData, navigate, setFilter, setLogSearch, selectLogJob, setLogStream, setPage, openJobForm, closeJobForm, updateDraft, loadDirectory, setFilesystemInputPath, chooseDirectory, saveJob, openJobDetail, closeJobDetail, toggleDescriptionEdit, saveDescription, copyJobId, requestConfirmation, resolveConfirmation, cleanJobs, queueLock, moveQueueJob, commitJob, cancelJob, viewJobLogs, setConfigurationDraft, saveTimezone, unsetTimezone, createSnapshot, restoreSnapshot, connectToken, dismissToken]);
  return <WorkspaceContext.Provider value={{ state, actions, jobFormOpen, jobDetailOpen, detailEditing, tokenOpen, confirmation, toasts }}>{children}</WorkspaceContext.Provider>;
}

export { JOBS_PAGE_SIZE, SNAPSHOTS_PAGE_SIZE, pageInfo };
