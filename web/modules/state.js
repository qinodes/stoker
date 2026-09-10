export const JOBS_PAGE_SIZE = 6;
export const SNAPSHOTS_PAGE_SIZE = 5;
export const DIRECTORY_CACHE_TTL_MS = 20_000;
export const DIRECTORY_CACHE_LIMIT = 64;

export function createState(route = "overview") {
  return {
    config: null,
    settings: null,
    status: null,
    timezone: null,
    jobs: [],
    queue: { jobs: [], locked: false },
    logs: { jobId: "", stream: "stdout", data: null, error: null, search: "" },
    route,
    filters: { search: "", user: "", state: "" },
    pagination: { jobs: 1, snapshots: 1 },
    loaded: false,
    loading: false,
    error: null,
    requestSequence: 0,
    appliedSequence: 0,
    jobDraft: { user: "", name: "", cwd: "", command: "", description: "" },
    filesystem: { roots: null, current: null, inputPath: "", loading: false, error: "", requestId: 0 },
    selectedJob: null,
    selectedJobDetail: null,
    configurationDraft: null,
    detailRequestId: 0,
    directoryCache: new Map(),
  };
}

export function beginLoad(state) {
  state.loading = true;
  state.error = null;
  state.requestSequence += 1;
  return state.requestSequence;
}

export function applyLoad(state, sequence, payload) {
  if (sequence < state.requestSequence) return false;
  state.appliedSequence = sequence;
  state.config = payload.config;
  state.status = payload.status;
  state.jobs = payload.jobs.jobs || [];
  state.timezone = payload.jobs.timezone || payload.status.timezone || null;
  state.queue = payload.queue;
  state.settings = payload.settings;
  state.loaded = true;
  state.loading = false;
  return true;
}

export function failLoad(state, sequence, error) {
  if (sequence < state.requestSequence) return false;
  state.error = error.message;
  state.loading = false;
  return true;
}

export function pageInfo(items, requestedPage, pageSize) {
  const totalPages = Math.max(1, Math.ceil(items.length / pageSize));
  const page = Math.min(Math.max(Number(requestedPage) || 1, 1), totalPages);
  const start = (page - 1) * pageSize;
  return {
    page,
    totalPages,
    totalItems: items.length,
    start,
    end: Math.min(start + pageSize, items.length),
    items: items.slice(start, start + pageSize),
  };
}

export function cachedDirectory(state, path, now = Date.now()) {
  const entry = state.directoryCache.get(path);
  if (!entry || now - entry.storedAt >= DIRECTORY_CACHE_TTL_MS) return null;
  state.directoryCache.delete(path);
  state.directoryCache.set(path, entry);
  return entry.value;
}

export function cacheDirectory(state, path, value, now = Date.now()) {
  state.directoryCache.delete(path);
  state.directoryCache.set(path, { value, storedAt: now });
  while (state.directoryCache.size > DIRECTORY_CACHE_LIMIT) {
    state.directoryCache.delete(state.directoryCache.keys().next().value);
  }
}
